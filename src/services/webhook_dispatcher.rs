/// Webhook Dispatcher Service
/// Processes webhook events and sends them to registered webhooks with retry logic
use anyhow::Result;
use reqwest::Client;
use sqlx::SqlitePool;
use std::time::Duration;

use crate::webhooks::{
    WebhookEventEnvelope, WebhookService, WebhookSignature, MAX_WEBHOOK_ATTEMPTS,
};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Webhook dispatcher - sends events to webhooks asynchronously
pub struct WebhookDispatcher {
    db: SqlitePool,
    http_client: Client,
}

impl WebhookDispatcher {
    /// Create new webhook dispatcher
    #[must_use]
    pub fn new(db: SqlitePool) -> Self {
        let http_client = Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap_or_else(|_| Client::new());

        Self { db, http_client }
    }

    /// Run dispatcher loop - processes pending webhook events
    pub async fn run(&self) -> Result<()> {
        tracing::info!("Starting webhook dispatcher");

        let mut interval = tokio::time::interval(Duration::from_secs(5));

        loop {
            interval.tick().await;

            if let Err(e) = self.process_pending_events().await {
                tracing::error!("Error processing webhook events: {}", e);
            }
        }
    }

    /// Process all pending webhook events
    async fn process_pending_events(&self) -> Result<()> {
        let service = WebhookService::new(self.db.clone());

        // Fetch pending events (max 10 per run)
        let events = service.get_pending_deliveries(10).await?;

        for event in events {
            let Some(token) = service.reserve_event(&event).await? else {
                continue;
            };

            // Get webhook details
            let webhook = if let Some(w) = service.get_webhook(&event.webhook_id).await? {
                w
            } else {
                // Webhook was deleted, mark event as failed
                service
                    .finish_reserved_event(
                        &event.id,
                        &token,
                        "failed",
                        Some("webhook_deleted"),
                        event.retries,
                    )
                    .await?;
                continue;
            };

            if !webhook.is_active {
                service
                    .finish_reserved_event(
                        &event.id,
                        &token,
                        "failed",
                        Some("webhook_inactive"),
                        event.retries,
                    )
                    .await?;
                continue;
            }

            let body = match event.delivery_body() {
                Ok(body) => body,
                Err(error) => {
                    service
                        .finish_reserved_event(
                            &event.id,
                            &token,
                            "failed",
                            Some(&format!("invalid_event_payload: {error}")),
                            event.retries,
                        )
                        .await?;
                    continue;
                }
            };
            let body = service.persist_delivery_payload(&event.id, &body).await?;
            if !service.owns_reservation(&event.id, &token).await? {
                continue;
            }

            // Attempt delivery
            match self
                .deliver_webhook(&webhook.url, &body, &webhook.secret)
                .await
            {
                Ok(()) => {
                    // Success
                    service
                        .finish_reserved_event(&event.id, &token, "delivered", None, event.retries)
                        .await?;

                    tracing::info!(
                        "Webhook delivered successfully: webhook_id={}, event={}",
                        event.webhook_id,
                        event.event_type
                    );
                }
                Err(e) => {
                    let failures = event.retries + 1;
                    let status = if failures < MAX_WEBHOOK_ATTEMPTS {
                        "pending"
                    } else {
                        "failed"
                    };
                    service
                        .finish_reserved_event(
                            &event.id,
                            &token,
                            status,
                            Some(&e.to_string()),
                            failures,
                        )
                        .await?;

                    if status == "pending" {
                        tracing::warn!(
                            "Webhook delivery failed (will retry): webhook_id={}, error={}, retries={}",
                            event.webhook_id,
                            e,
                            failures
                        );
                    } else {
                        tracing::error!(
                            "Webhook delivery failed (max retries): webhook_id={}, error={}",
                            event.webhook_id,
                            e
                        );
                    }
                }
            }
        }

        Ok(())
    }

    /// Deliver webhook to URL
    async fn deliver_webhook(&self, url: &str, body: &str, secret: &str) -> Result<()> {
        // Headers and signature describe the exact envelope frozen in the DB.
        let envelope: WebhookEventEnvelope = serde_json::from_str(body)?;
        let signature = WebhookSignature::sign(body, secret);

        tracing::debug!(
            "Sending webhook to {}: delivery_id={}, signature={}...",
            url,
            envelope.id,
            &signature[..20]
        );

        let response = self
            .http_client
            .post(url)
            .timeout(REQUEST_TIMEOUT)
            .header("X-Zapier-Event", envelope.event)
            .header("X-Zapier-Signature", signature)
            .header("X-Zapier-Timestamp", envelope.timestamp.to_string())
            .header("X-Zapier-Delivery-ID", envelope.id)
            .header("Content-Type", "application/json")
            .body(body.to_owned())
            .send()
            .await?;

        if response.status().is_success() {
            Ok(())
        } else {
            anyhow::bail!("Webhook failed with status {}", response.status())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Context;
    use axum::{
        extract::State,
        http::{HeaderMap, StatusCode},
        routing::post,
        Router,
    };
    use serde_json::json;
    use sqlx::sqlite::SqlitePoolOptions;
    use std::sync::Arc;
    use tokio::{net::TcpListener, sync::Mutex, task::JoinHandle};

    use crate::webhooks::WebhookEventStatus;

    const WEBHOOK_ID: &str = "webhook-1";
    const SUBSCRIBER_SECRET: &str = "subscriber-secret";

    #[derive(Clone)]
    struct CapturedRequest {
        headers: HeaderMap,
        body: String,
    }

    struct ReceiverState {
        statuses: Vec<StatusCode>,
        requests: Mutex<Vec<CapturedRequest>>,
        redirect_to: Option<String>,
    }

    struct TestReceiver {
        url: String,
        state: Arc<ReceiverState>,
        task: JoinHandle<std::io::Result<()>>,
    }

    impl TestReceiver {
        async fn start(statuses: Vec<StatusCode>) -> Result<Self> {
            Self::start_with_redirect(statuses, None).await
        }

        async fn start_with_redirect(
            statuses: Vec<StatusCode>,
            redirect_to: Option<String>,
        ) -> Result<Self> {
            let listener = TcpListener::bind("127.0.0.1:0").await?;
            let address = listener.local_addr()?;
            let state = Arc::new(ReceiverState {
                statuses,
                requests: Mutex::new(Vec::new()),
                redirect_to,
            });
            let app = Router::new()
                .route("/webhook", post(capture_request))
                .with_state(state.clone());
            let task = tokio::spawn(async move { axum::serve(listener, app).await });

            Ok(Self {
                url: format!("http://{address}/webhook"),
                state,
                task,
            })
        }

        async fn requests(&self) -> Vec<CapturedRequest> {
            self.state.requests.lock().await.clone()
        }
    }

    impl Drop for TestReceiver {
        fn drop(&mut self) {
            self.task.abort();
        }
    }

    async fn capture_request(
        State(state): State<Arc<ReceiverState>>,
        headers: HeaderMap,
        body: String,
    ) -> (StatusCode, HeaderMap, &'static str) {
        let mut requests = state.requests.lock().await;
        let status = state
            .statuses
            .get(requests.len())
            .copied()
            .unwrap_or(StatusCode::SERVICE_UNAVAILABLE);
        requests.push(CapturedRequest { headers, body });
        let mut headers = HeaderMap::new();
        if let Some(location) = &state.redirect_to {
            headers.insert(axum::http::header::LOCATION, location.parse().unwrap());
        }
        (status, headers, "receiver response")
    }

    async fn dispatcher_db(url: &str, active: bool) -> Result<SqlitePool> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await?;
        sqlx::query("CREATE TABLE users (id TEXT PRIMARY KEY)")
            .execute(&pool)
            .await?;
        sqlx::raw_sql(include_str!("../../migrations/019_oauth_webhooks.sql"))
            .execute(&pool)
            .await?;
        sqlx::raw_sql(include_str!(
            "../../migrations/035_webhook_delivery_retries.sql"
        ))
        .execute(&pool)
        .await?;
        sqlx::query("INSERT INTO users (id) VALUES ('owner')")
            .execute(&pool)
            .await?;
        sqlx::query(
            "INSERT INTO webhooks (id, user_id, url, event_types, secret, is_active)
             VALUES (?, 'owner', ?, 'test', ?, ?)",
        )
        .bind(WEBHOOK_ID)
        .bind(url)
        .bind(SUBSCRIBER_SECRET)
        .bind(active)
        .execute(&pool)
        .await?;

        Ok(pool)
    }

    async fn queue_event(pool: &SqlitePool) -> Result<String> {
        WebhookService::new(pool.clone())
            .create_webhook_event(WEBHOOK_ID, "test", json!({"amount": 42}))
            .await
    }

    async fn event_state(pool: &SqlitePool, event_id: &str) -> Result<WebhookEventStatus> {
        Ok(sqlx::query_as::<_, WebhookEventStatus>(
            "SELECT id, webhook_id, event_type, status, retries, last_error, created_at,
                    next_attempt_at, last_attempt_at, delivered_at
             FROM webhook_events WHERE id = ?",
        )
        .bind(event_id)
        .fetch_one(pool)
        .await?)
    }

    async fn make_due(pool: &SqlitePool, event_id: &str) -> Result<()> {
        sqlx::query("UPDATE webhook_events SET next_attempt_at = 0 WHERE id = ?")
            .bind(event_id)
            .execute(pool)
            .await?;
        Ok(())
    }

    async fn last_fired_at(pool: &SqlitePool) -> Result<Option<String>> {
        Ok(
            sqlx::query_scalar("SELECT last_fired_at FROM webhooks WHERE id = ?")
                .bind(WEBHOOK_ID)
                .fetch_one(pool)
                .await?,
        )
    }

    #[tokio::test]
    async fn retries_survive_dispatcher_restart_with_identical_signed_delivery() -> Result<()> {
        let receiver =
            TestReceiver::start(vec![StatusCode::INTERNAL_SERVER_ERROR, StatusCode::OK]).await?;
        let pool = dispatcher_db(&receiver.url, true).await?;
        let event_id = queue_event(&pool).await?;
        let first_dispatcher = WebhookDispatcher::new(pool.clone());
        let before = chrono::Utc::now().timestamp();

        first_dispatcher.process_pending_events().await?;
        let pending = event_state(&pool, &event_id).await?;
        assert_eq!(pending.status, "pending");
        assert_eq!(pending.retries, 1);
        assert!(pending
            .last_error
            .as_deref()
            .is_some_and(|e| e.contains("500")));
        assert!(
            pending
                .next_attempt_at
                .context("retry must have a due time")?
                >= before + 5
        );
        assert!(pending.last_attempt_at.is_some());
        assert_eq!(pending.delivered_at, None);
        assert_eq!(last_fired_at(&pool).await?, None);
        drop(first_dispatcher);

        let restarted_dispatcher = WebhookDispatcher::new(pool.clone());
        restarted_dispatcher.process_pending_events().await?;
        assert_eq!(receiver.requests().await.len(), 1);
        assert_eq!(event_state(&pool, &event_id).await?.retries, 1);

        make_due(&pool, &event_id).await?;
        restarted_dispatcher.process_pending_events().await?;

        let delivered = event_state(&pool, &event_id).await?;
        assert_eq!(delivered.status, "delivered");
        assert_eq!(delivered.retries, 1);
        assert_eq!(delivered.last_error, pending.last_error);
        assert_eq!(delivered.next_attempt_at, None);
        assert!(delivered.delivered_at.is_some());
        assert!(last_fired_at(&pool).await?.is_some());

        let requests = receiver.requests().await;
        assert_eq!(requests.len(), 2);
        let first = &requests[0];
        let second = &requests[1];
        assert_eq!(first.body, second.body);
        for name in [
            "X-Zapier-Event",
            "X-Zapier-Signature",
            "X-Zapier-Timestamp",
            "X-Zapier-Delivery-ID",
            "Content-Type",
        ] {
            assert_eq!(first.headers.get(name), second.headers.get(name));
            assert!(first.headers.contains_key(name));
        }

        let envelope: WebhookEventEnvelope = serde_json::from_str(&first.body)?;
        assert_eq!(envelope.id, event_id);
        assert_eq!(envelope.event, "test");
        assert_eq!(envelope.data, json!({"amount": 42}));
        assert_eq!(
            first
                .headers
                .get("X-Zapier-Delivery-ID")
                .context("delivery ID header")?
                .to_str()?,
            event_id
        );
        assert_eq!(
            first
                .headers
                .get("X-Zapier-Timestamp")
                .context("timestamp header")?
                .to_str()?,
            envelope.timestamp.to_string()
        );
        let signature = first
            .headers
            .get("X-Zapier-Signature")
            .context("signature header")?
            .to_str()?;
        assert!(WebhookSignature::verify(
            &first.body,
            SUBSCRIBER_SECRET,
            signature
        ));
        assert!(!WebhookSignature::verify(
            &format!("{} ", first.body),
            SUBSCRIBER_SECRET,
            signature
        ));
        assert!(!WebhookSignature::verify(
            &first.body,
            "different-subscriber-secret",
            signature
        ));
        Ok(())
    }

    #[tokio::test]
    async fn three_failed_attempts_become_terminal_without_a_fourth_request() -> Result<()> {
        let receiver = TestReceiver::start(vec![StatusCode::SERVICE_UNAVAILABLE]).await?;
        let pool = dispatcher_db(&receiver.url, true).await?;
        let event_id = queue_event(&pool).await?;

        for attempt in 1..=3 {
            let dispatcher = WebhookDispatcher::new(pool.clone());
            let before = chrono::Utc::now().timestamp();
            dispatcher.process_pending_events().await?;

            let state = event_state(&pool, &event_id).await?;
            assert_eq!(state.retries, attempt);
            assert!(state
                .last_error
                .as_deref()
                .is_some_and(|e| e.contains("503")));
            assert_eq!(state.delivered_at, None);
            assert_eq!(receiver.requests().await.len(), attempt as usize);

            if attempt < 3 {
                assert_eq!(state.status, "pending");
                let delay = if attempt == 1 { 5 } else { 10 };
                assert!(state.next_attempt_at.context("retry due time")? >= before + delay);
                dispatcher.process_pending_events().await?;
                assert_eq!(receiver.requests().await.len(), attempt as usize);
                make_due(&pool, &event_id).await?;
            } else {
                assert_eq!(state.status, "failed");
                assert_eq!(state.next_attempt_at, None);
            }
        }

        // Even an explicitly due timestamp cannot revive a terminal delivery.
        make_due(&pool, &event_id).await?;
        WebhookDispatcher::new(pool.clone())
            .process_pending_events()
            .await?;
        assert_eq!(receiver.requests().await.len(), 3);
        assert_eq!(event_state(&pool, &event_id).await?.status, "failed");
        assert_eq!(last_fired_at(&pool).await?, None);
        Ok(())
    }

    #[tokio::test]
    async fn redirects_do_not_contact_another_receiver_or_store_response_bodies() -> Result<()> {
        let destination = TestReceiver::start(vec![StatusCode::OK]).await?;
        let redirect = TestReceiver::start_with_redirect(
            vec![StatusCode::FOUND],
            Some(destination.url.clone()),
        )
        .await?;
        let pool = dispatcher_db(&redirect.url, true).await?;
        let event_id = queue_event(&pool).await?;
        WebhookDispatcher::new(pool.clone())
            .process_pending_events()
            .await?;
        let state = event_state(&pool, &event_id).await?;
        assert_eq!(state.status, "pending");
        assert_eq!(state.retries, 1);
        let error = state
            .last_error
            .context("redirect failure must be recorded")?;
        assert!(error.contains("302"));
        assert!(!error.contains("receiver response"));
        assert_eq!(redirect.requests().await.len(), 1);
        assert!(destination.requests().await.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn persistence_errors_propagate_and_roll_back_delivery_state() -> Result<()> {
        for (trigger, expected_error) in [
            (
                "CREATE TRIGGER reject_delivered BEFORE UPDATE OF status ON webhook_events
                 WHEN NEW.status = 'delivered'
                 BEGIN SELECT RAISE(ABORT, 'reject delivered'); END;",
                "reject delivered",
            ),
            (
                "CREATE TRIGGER reject_last_fired BEFORE UPDATE OF last_fired_at ON webhooks
                 BEGIN SELECT RAISE(ABORT, 'reject last_fired_at'); END;",
                "reject last_fired_at",
            ),
        ] {
            let receiver = TestReceiver::start(vec![StatusCode::OK]).await?;
            let pool = dispatcher_db(&receiver.url, true).await?;
            let event_id = queue_event(&pool).await?;
            sqlx::raw_sql(trigger).execute(&pool).await?;
            let before = chrono::Utc::now().timestamp();

            let error = WebhookDispatcher::new(pool.clone())
                .process_pending_events()
                .await
                .err()
                .context("injected persistence failure must propagate")?;
            assert!(error.to_string().contains(expected_error));

            let state = event_state(&pool, &event_id).await?;
            assert_eq!(state.status, "pending");
            assert_eq!(state.retries, 0);
            assert_eq!(state.last_error, None);
            assert_eq!(state.delivered_at, None);
            assert!(state.last_attempt_at.is_some());
            assert!(state.next_attempt_at.context("reservation must remain")? >= before + 30);
            assert_eq!(last_fired_at(&pool).await?, None);
            assert_eq!(receiver.requests().await.len(), 1);

            WebhookDispatcher::new(pool.clone())
                .process_pending_events()
                .await?;
            assert_eq!(receiver.requests().await.len(), 1);
        }
        Ok(())
    }

    #[tokio::test]
    async fn inactive_webhook_does_not_send_or_reset_failure_count() -> Result<()> {
        let receiver = TestReceiver::start(vec![StatusCode::OK]).await?;
        let pool = dispatcher_db(&receiver.url, false).await?;
        let event_id = queue_event(&pool).await?;
        sqlx::query(
            "UPDATE webhook_events SET retries = 2, last_error = 'previous failure' WHERE id = ?",
        )
        .bind(&event_id)
        .execute(&pool)
        .await?;

        WebhookDispatcher::new(pool.clone())
            .process_pending_events()
            .await?;

        let state = event_state(&pool, &event_id).await?;
        assert_eq!(state.status, "failed");
        assert_eq!(state.retries, 2);
        assert_eq!(state.last_error.as_deref(), Some("webhook_inactive"));
        assert_eq!(state.next_attempt_at, None);
        assert_eq!(state.delivered_at, None);
        assert_eq!(last_fired_at(&pool).await?, None);
        assert!(receiver.requests().await.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn legacy_sqlite_timestamp_is_frozen_before_retry() -> Result<()> {
        let receiver =
            TestReceiver::start(vec![StatusCode::INTERNAL_SERVER_ERROR, StatusCode::OK]).await?;
        let pool = dispatcher_db(&receiver.url, true).await?;
        let event_id = "legacy-event";
        sqlx::query(
            "INSERT INTO webhook_events
                 (id, webhook_id, event_type, payload, status, retries, created_at)
             VALUES (?, ?, 'test', '{\"legacy\":true}', 'pending', 0, '2023-11-14 22:13:20')",
        )
        .bind(event_id)
        .bind(WEBHOOK_ID)
        .execute(&pool)
        .await?;

        WebhookDispatcher::new(pool.clone())
            .process_pending_events()
            .await?;
        let first = receiver.requests().await;
        assert_eq!(first.len(), 1);
        let envelope: WebhookEventEnvelope = serde_json::from_str(&first[0].body)?;
        assert_eq!(envelope.id, event_id);
        assert_eq!(envelope.timestamp, 1_700_000_000);
        assert_eq!(envelope.data, json!({"legacy": true}));
        let frozen_body: Option<String> =
            sqlx::query_scalar("SELECT delivery_payload FROM webhook_events WHERE id = ?")
                .bind(event_id)
                .fetch_one(&pool)
                .await?;
        assert_eq!(frozen_body.as_deref(), Some(first[0].body.as_str()));

        // Retries use the frozen body even if the original row is later edited.
        sqlx::query(
            "UPDATE webhook_events SET payload = '{\"legacy\":false}',
                 created_at = '2030-01-01 00:00:00', next_attempt_at = 0 WHERE id = ?",
        )
        .bind(event_id)
        .execute(&pool)
        .await?;
        WebhookDispatcher::new(pool.clone())
            .process_pending_events()
            .await?;

        let requests = receiver.requests().await;
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].body, requests[1].body);
        assert_eq!(event_state(&pool, event_id).await?.status, "delivered");
        Ok(())
    }
}
