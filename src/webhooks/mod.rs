/// Webhooks module for Zapier integration
/// Manages webhook registrations, event definitions, and dispatching
pub mod channel;
pub mod events;

use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use sqlx::SqlitePool;
use uuid::Uuid;

type HmacSha256 = Hmac<Sha256>;

pub use channel::{WebhookChannel, WebhookEndpoint};

/// Maximum failed delivery attempts, including the initial attempt.
pub const MAX_WEBHOOK_ATTEMPTS: i32 = 3;
const RETRY_BASE_SECONDS: i64 = 5;
const RETRY_MAX_SECONDS: i64 = 300;
// Longer than the dispatcher's per-request timeout. A failed status write or
// worker restart must not immediately send the same event again.
const DELIVERY_RESERVATION_SECONDS: i64 = 30;

fn retry_delay_seconds(failures: i32) -> i64 {
    RETRY_BASE_SECONDS
        .saturating_mul(2_i64.saturating_pow(failures.saturating_sub(1).max(0) as u32))
        .min(RETRY_MAX_SECONDS)
}

/// Webhook signature - for verifying webhook requests
pub struct WebhookSignature;

impl WebhookSignature {
    /// Generate HMAC-SHA256 signature for webhook payload
    #[must_use]
    pub fn sign(payload: &str, secret: &str) -> String {
        // SAFETY: HmacSha256::new_from_slice accepts any key length by design;
        // the only error case (invalid length) cannot occur for HMAC-SHA256.
        #[allow(clippy::expect_used)]
        let mut mac =
            HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC can take key of any size");
        Mac::update(&mut mac, payload.as_bytes());
        format!("sha256={}", hex::encode(Mac::finalize(mac).into_bytes()))
    }

    /// Verify webhook signature
    #[must_use]
    pub fn verify(payload: &str, secret: &str, signature: &str) -> bool {
        let expected = Self::sign(payload, secret);
        signature == expected
    }
}

/// Webhook Configuration
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Webhook {
    pub id: String,
    pub user_id: String,
    pub url: String,
    pub event_types: String,     // comma-separated
    pub filters: Option<String>, // JSON
    pub secret: String,
    pub is_active: bool,
    pub created_at: String,
    pub last_fired_at: Option<String>,
}

/// Webhook creation request
#[derive(Debug, Deserialize)]
pub struct CreateWebhookRequest {
    pub url: String,
    pub event_types: Vec<String>,
    pub filters: Option<serde_json::Value>,
}

/// Webhook creation response
#[derive(Debug, Serialize)]
pub struct WebhookResponse {
    pub id: String,
    pub url: String,
    pub event_types: Vec<String>,
    pub filters: Option<serde_json::Value>,
    pub is_active: bool,
    pub created_at: String,
}

/// Returned once at registration so the subscriber can verify deliveries.
#[derive(Debug, Serialize)]
pub struct WebhookRegistrationResponse {
    #[serde(flatten)]
    pub webhook: WebhookResponse,
    pub secret: String,
}

/// Webhook event envelope
#[derive(Debug, Serialize, Deserialize)]
pub struct WebhookEventEnvelope {
    pub id: String, // Delivery ID for idempotency
    pub event: String,
    pub timestamp: i64,
    pub data: serde_json::Value,
}

/// Persisted delivery state; subscriber secrets and event data are not exposed.
#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct WebhookEventStatus {
    pub id: String,
    pub webhook_id: String,
    pub event_type: String,
    pub status: String,
    /// Number of failed attempts, retained after eventual success.
    pub retries: i32,
    pub last_error: Option<String>,
    pub created_at: String,
    pub next_attempt_at: Option<i64>,
    pub last_attempt_at: Option<i64>,
    pub delivered_at: Option<i64>,
}

#[derive(Debug, sqlx::FromRow)]
pub(crate) struct PendingWebhookEvent {
    pub id: String,
    pub webhook_id: String,
    pub event_type: String,
    pub payload: String,
    pub retries: i32,
    pub created_at: String,
    pub delivery_payload: Option<String>,
}

impl PendingWebhookEvent {
    /// Older events have no stored envelope yet. Derive it from immutable
    /// database fields, accepting both service and SQLite timestamp formats.
    pub(crate) fn delivery_body(&self) -> anyhow::Result<String> {
        if let Some(body) = &self.delivery_payload {
            return Ok(body.clone());
        }

        let timestamp = chrono::DateTime::parse_from_rfc3339(&self.created_at)
            .map(|time| time.timestamp())
            .or_else(|_| {
                chrono::NaiveDateTime::parse_from_str(&self.created_at, "%Y-%m-%d %H:%M:%S%.f")
                    .map(|time| time.and_utc().timestamp())
            })?;

        Ok(serde_json::to_string(&WebhookEventEnvelope {
            id: self.id.clone(),
            event: self.event_type.clone(),
            timestamp,
            data: serde_json::from_str(&self.payload)?,
        })?)
    }
}

/// Event types that can trigger webhooks
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum WebhookEventType {
    CorridorHealthDegraded,
    AnchorStatusChanged,
    PaymentCreated,
    CorridorLiquidityDropped,
}

impl WebhookEventType {
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::CorridorHealthDegraded => "corridor.health_degraded",
            Self::AnchorStatusChanged => "anchor.status_changed",
            Self::PaymentCreated => "payment.created",
            Self::CorridorLiquidityDropped => "corridor.liquidity_dropped",
        }
    }

    #[must_use]
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "corridor.health_degraded" => Some(Self::CorridorHealthDegraded),
            "anchor.status_changed" => Some(Self::AnchorStatusChanged),
            "payment.created" => Some(Self::PaymentCreated),
            "corridor.liquidity_dropped" => Some(Self::CorridorLiquidityDropped),
            _ => None,
        }
    }
}

/// Webhook service - manages webhook operations
pub struct WebhookService {
    pub db: SqlitePool,
    encryption_key: String,
}

impl WebhookService {
    #[must_use]
    pub fn new(db: SqlitePool) -> Self {
        let encryption_key = std::env::var("ENCRYPTION_KEY").unwrap_or_else(|_| {
            "0000000000000000000000000000000000000000000000000000000000000000".to_string()
        });
        Self { db, encryption_key }
    }

    /// Register a new webhook
    pub async fn register_webhook(
        &self,
        user_id: &str,
        request: CreateWebhookRequest,
    ) -> anyhow::Result<WebhookResponse> {
        Ok(self
            .register_webhook_with_secret(user_id, request)
            .await?
            .webhook)
    }

    /// Register a webhook and return its signing secret to its creator once.
    pub async fn register_webhook_with_secret(
        &self,
        user_id: &str,
        request: CreateWebhookRequest,
    ) -> anyhow::Result<WebhookRegistrationResponse> {
        let id = Uuid::new_v4().to_string();
        let secret = Uuid::new_v4().to_string();
        let event_types_str = request.event_types.join(",");
        let filters_str = request
            .filters
            .as_ref()
            .map(std::string::ToString::to_string);
        let now = chrono::Utc::now().to_rfc3339();

        let encrypted_secret = crate::crypto::encrypt_data(&secret, &self.encryption_key)
            .unwrap_or_else(|_| secret.clone());

        sqlx::query(
            r"
            INSERT INTO webhooks (id, user_id, url, event_types, filters, secret, is_active, created_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            ",
        )
        .bind(&id)
        .bind(user_id)
        .bind(&request.url)
        .bind(&event_types_str)
        .bind(filters_str.as_deref())
        .bind(&encrypted_secret)
        .bind(true)
        .bind(&now)
        .execute(&self.db)
        .await?;

        Ok(WebhookRegistrationResponse {
            webhook: WebhookResponse {
                id,
                url: request.url,
                event_types: request.event_types,
                filters: request.filters,
                is_active: true,
                created_at: now,
            },
            secret,
        })
    }

    /// Get webhook by ID
    pub async fn get_webhook(&self, webhook_id: &str) -> anyhow::Result<Option<Webhook>> {
        let mut webhook = sqlx::query_as::<_, Webhook>(
            "SELECT id, user_id, url, event_types, filters, secret, is_active, created_at, last_fired_at FROM webhooks WHERE id = ?"
        )
        .bind(webhook_id)
        .fetch_optional(&self.db)
        .await?;

        if let Some(ref mut w) = webhook {
            w.secret = crate::crypto::decrypt_data(&w.secret, &self.encryption_key)
                .unwrap_or_else(|_| w.secret.clone());
        }

        Ok(webhook)
    }

    /// List webhooks for a user
    pub async fn list_webhooks(&self, user_id: &str) -> anyhow::Result<Vec<Webhook>> {
        let mut webhooks = sqlx::query_as::<_, Webhook>(
            "SELECT id, user_id, url, event_types, filters, secret, is_active, created_at, last_fired_at FROM webhooks WHERE user_id = ? AND is_active = 1 ORDER BY created_at DESC"
        )
        .bind(user_id)
        .fetch_all(&self.db)
        .await?;

        for w in &mut webhooks {
            w.secret = crate::crypto::decrypt_data(&w.secret, &self.encryption_key)
                .unwrap_or_else(|_| w.secret.clone());
        }

        Ok(webhooks)
    }

    /// Delete/deactivate webhook
    pub async fn delete_webhook(&self, webhook_id: &str, user_id: &str) -> anyhow::Result<bool> {
        let result = sqlx::query("UPDATE webhooks SET is_active = 0 WHERE id = ? AND user_id = ?")
            .bind(webhook_id)
            .bind(user_id)
            .execute(&self.db)
            .await?;

        Ok(result.rows_affected() > 0)
    }

    /// Record webhook event for delivery
    pub async fn create_webhook_event(
        &self,
        webhook_id: &str,
        event_type: &str,
        payload: serde_json::Value,
    ) -> anyhow::Result<String> {
        let id = Uuid::new_v4().to_string();
        let payload_str = payload.to_string();
        let now = chrono::Utc::now();
        let delivery_payload = serde_json::to_string(&WebhookEventEnvelope {
            id: id.clone(),
            event: event_type.to_string(),
            timestamp: now.timestamp(),
            data: payload,
        })?;

        sqlx::query(
            "INSERT INTO webhook_events (id, webhook_id, event_type, payload, status, retries, created_at, delivery_payload)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)"
        )
        .bind(id.clone())
        .bind(webhook_id)
        .bind(event_type)
        .bind(payload_str)
        .bind("pending")
        .bind(0)
        .bind(now.to_rfc3339())
        .bind(delivery_payload)
        .execute(&self.db)
        .await?;

        Ok(id)
    }

    /// Get pending events whose persisted retry time has arrived.
    pub async fn get_pending_events(
        &self,
        limit: usize,
    ) -> anyhow::Result<Vec<(String, String, String, String)>> {
        Ok(self
            .get_pending_deliveries(limit)
            .await?
            .into_iter()
            .map(|event| (event.id, event.webhook_id, event.event_type, event.payload))
            .collect())
    }

    pub(crate) async fn get_pending_deliveries(
        &self,
        limit: usize,
    ) -> anyhow::Result<Vec<PendingWebhookEvent>> {
        let query_limit = i64::try_from(limit)?;

        Ok(sqlx::query_as::<_, PendingWebhookEvent>(
            "SELECT we.id, we.webhook_id, we.event_type, we.payload,
                    we.retries, we.created_at, we.delivery_payload
             FROM webhook_events we
             WHERE we.status = 'pending' AND we.retries < ?
               AND (we.next_attempt_at IS NULL OR we.next_attempt_at <= ?)
             ORDER BY we.created_at ASC, we.id ASC
             LIMIT ?",
        )
        .bind(MAX_WEBHOOK_ATTEMPTS)
        .bind(chrono::Utc::now().timestamp())
        .bind(query_limit)
        .fetch_all(&self.db)
        .await?)
    }

    /// Atomically reserve a due event before making an HTTP request. Competing
    /// dispatchers that selected the same row cannot both start delivery.
    pub(crate) async fn reserve_event(
        &self,
        event: &PendingWebhookEvent,
    ) -> anyhow::Result<Option<String>> {
        let now = chrono::Utc::now().timestamp();
        let token = Uuid::new_v4().to_string();
        let result = sqlx::query(
            "UPDATE webhook_events SET next_attempt_at = ?, last_attempt_at = ?, delivery_token = ?
             WHERE id = ? AND status = 'pending' AND retries = ? AND retries < ?
               AND (next_attempt_at IS NULL OR next_attempt_at <= ?)",
        )
        .bind(now + DELIVERY_RESERVATION_SECONDS)
        .bind(now)
        .bind(&token)
        .bind(&event.id)
        .bind(event.retries)
        .bind(MAX_WEBHOOK_ATTEMPTS)
        .bind(now)
        .execute(&self.db)
        .await?;

        Ok((result.rows_affected() == 1).then_some(token))
    }

    /// Check ownership again after preparing the request, before contacting
    /// the subscriber. A paused worker must not send under an expired claim.
    pub(crate) async fn owns_reservation(
        &self,
        event_id: &str,
        token: &str,
    ) -> anyhow::Result<bool> {
        Ok(sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM webhook_events
             WHERE id = ? AND status = 'pending' AND delivery_token = ? AND next_attempt_at > ?)",
        )
        .bind(event_id)
        .bind(token)
        .bind(chrono::Utc::now().timestamp())
        .fetch_one(&self.db)
        .await?)
    }

    /// Freeze legacy envelopes before sending; never overwrite a stored body.
    pub(crate) async fn persist_delivery_payload(
        &self,
        event_id: &str,
        body: &str,
    ) -> anyhow::Result<String> {
        Ok(sqlx::query_scalar(
            "UPDATE webhook_events SET delivery_payload = COALESCE(delivery_payload, ?)
             WHERE id = ? RETURNING delivery_payload",
        )
        .bind(body)
        .bind(event_id)
        .fetch_one(&self.db)
        .await?)
    }

    /// Query a delivery only when its webhook belongs to the authenticated user.
    pub async fn get_event_status(
        &self,
        webhook_id: &str,
        event_id: &str,
        user_id: &str,
    ) -> anyhow::Result<Option<WebhookEventStatus>> {
        Ok(sqlx::query_as::<_, WebhookEventStatus>(
            "SELECT we.id, we.webhook_id, we.event_type, we.status, we.retries,
                    we.last_error, we.created_at, we.next_attempt_at,
                    we.last_attempt_at, we.delivered_at
             FROM webhook_events we JOIN webhooks w ON w.id = we.webhook_id
             WHERE we.id = ? AND we.webhook_id = ? AND w.user_id = ?",
        )
        .bind(event_id)
        .bind(webhook_id)
        .bind(user_id)
        .fetch_optional(&self.db)
        .await?)
    }

    /// Persist the outcome and next due time. A success retains earlier failure
    /// information and updates last_fired_at in the same transaction.
    pub async fn update_event_status(
        &self,
        event_id: &str,
        status: &str,
        error: Option<&str>,
        retries: i32,
    ) -> anyhow::Result<()> {
        self.set_event_status(event_id, status, error, retries, None)
            .await
    }

    pub(crate) async fn finish_reserved_event(
        &self,
        event_id: &str,
        token: &str,
        status: &str,
        error: Option<&str>,
        retries: i32,
    ) -> anyhow::Result<()> {
        self.set_event_status(event_id, status, error, retries, Some(token))
            .await
    }

    async fn set_event_status(
        &self,
        event_id: &str,
        status: &str,
        error: Option<&str>,
        retries: i32,
        token: Option<&str>,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(retries >= 0, "Invalid webhook retry count");
        anyhow::ensure!(
            matches!(status, "pending" | "delivered" | "failed"),
            "Invalid webhook event status"
        );
        let status = if status == "pending" && retries >= MAX_WEBHOOK_ATTEMPTS {
            "failed"
        } else {
            status
        };
        let now = chrono::Utc::now();
        let next_attempt_at =
            (status == "pending").then(|| now.timestamp() + retry_delay_seconds(retries));
        let delivered_at = (status == "delivered").then(|| now.timestamp());
        let mut transaction = self.db.begin().await?;
        let webhook_id: String = sqlx::query_scalar(
            "UPDATE webhook_events
             SET status = ?, last_error = COALESCE(?, last_error), retries = ?,
                 next_attempt_at = ?, delivered_at = ?, delivery_token = NULL
             WHERE id = ? AND status = 'pending'
               AND (delivery_token = ? OR (? IS NULL AND delivery_token IS NULL))
             RETURNING webhook_id",
        )
        .bind(status)
        .bind(error)
        .bind(retries)
        .bind(next_attempt_at)
        .bind(delivered_at)
        .bind(event_id)
        .bind(token)
        .bind(token)
        .fetch_one(&mut *transaction)
        .await?;

        if status == "delivered" {
            let result = sqlx::query("UPDATE webhooks SET last_fired_at = ? WHERE id = ?")
                .bind(now.to_rfc3339())
                .bind(webhook_id)
                .execute(&mut *transaction)
                .await?;
            anyhow::ensure!(result.rows_affected() == 1, "Webhook no longer exists");
        }

        transaction.commit().await?;
        Ok(())
    }

    /// Update webhook's `last_fired_at` timestamp
    pub async fn update_last_fired(&self, webhook_id: &str) -> anyhow::Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query("UPDATE webhooks SET last_fired_at = ? WHERE id = ?")
            .bind(now)
            .bind(webhook_id)
            .execute(&self.db)
            .await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn legacy_database() -> SqlitePool {
        let db = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::raw_sql("CREATE TABLE users (id TEXT PRIMARY KEY);")
            .execute(&db)
            .await
            .unwrap();
        sqlx::raw_sql(include_str!("../../migrations/019_oauth_webhooks.sql"))
            .execute(&db)
            .await
            .unwrap();
        sqlx::raw_sql(
            "INSERT INTO users (id) VALUES ('owner');
             INSERT INTO webhooks (id, user_id, url, event_types, secret)
             VALUES ('webhook', 'owner', 'http://localhost/', 'test', 'subscriber-secret');",
        )
        .execute(&db)
        .await
        .unwrap();
        db
    }

    #[test]
    fn retry_delay_is_exponential_and_saturates() {
        assert_eq!(
            (1..=8).map(retry_delay_seconds).collect::<Vec<_>>(),
            vec![5, 10, 20, 40, 80, 160, 300, 300]
        );
        assert_eq!(retry_delay_seconds(i32::MAX), 300);
    }

    #[tokio::test]
    async fn migration_closes_exhausted_events_without_losing_errors() {
        let db = legacy_database().await;
        sqlx::raw_sql(
            "INSERT INTO webhook_events (id, webhook_id, event_type, payload, status, retries, last_error)
             VALUES ('exhausted', 'webhook', 'test', '{}', 'pending', 3, 'original failure'),
                    ('delivered', 'webhook', 'test', '{}', 'delivered', 2, 'prior failure'),
                    ('ready', 'webhook', 'test', '{}', 'pending', 1, 'retryable failure');",
        )
        .execute(&db)
        .await
        .unwrap();
        sqlx::raw_sql(include_str!(
            "../../migrations/035_webhook_delivery_retries.sql"
        ))
        .execute(&db)
        .await
        .unwrap();

        let service = WebhookService::new(db);
        let exhausted = service
            .get_event_status("webhook", "exhausted", "owner")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(exhausted.status, "failed");
        assert_eq!(exhausted.retries, 3);
        assert_eq!(exhausted.last_error.as_deref(), Some("original failure"));
        let delivered = service
            .get_event_status("webhook", "delivered", "owner")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(delivered.status, "delivered");
        assert_eq!(delivered.retries, 2);
        assert_eq!(delivered.last_error.as_deref(), Some("prior failure"));
        let pending = service.get_pending_events(10).await.unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].0, "ready");
    }

    #[tokio::test]
    async fn stale_workers_cannot_reserve_the_same_due_event() {
        let db = legacy_database().await;
        sqlx::raw_sql(include_str!(
            "../../migrations/035_webhook_delivery_retries.sql"
        ))
        .execute(&db)
        .await
        .unwrap();
        let service = WebhookService::new(db.clone());
        let event_id = service
            .create_webhook_event("webhook", "test", serde_json::json!({"value": 1}))
            .await
            .unwrap();
        let first_view = service.get_pending_deliveries(10).await.unwrap();
        let second_view = service.get_pending_deliveries(10).await.unwrap();
        let (first, second) = tokio::join!(
            service.reserve_event(&first_view[0]),
            service.reserve_event(&second_view[0])
        );
        assert_ne!(first.unwrap().is_some(), second.unwrap().is_some());
        assert!(service.get_pending_events(10).await.unwrap().is_empty());

        // A crashed worker's reservation expires without discarding the event.
        sqlx::query("UPDATE webhook_events SET next_attempt_at = 0 WHERE id = ?")
            .bind(&event_id)
            .execute(&db)
            .await
            .unwrap();
        let recovered = service.get_pending_deliveries(10).await.unwrap();
        assert_eq!(recovered[0].id, event_id);
        assert_eq!(
            recovered[0].delivery_payload,
            first_view[0].delivery_payload
        );
        assert!(service
            .reserve_event(&recovered[0])
            .await
            .unwrap()
            .is_some());
    }

    #[tokio::test]
    async fn expired_workers_cannot_overwrite_a_new_delivery_outcome() {
        let db = legacy_database().await;
        sqlx::raw_sql(include_str!(
            "../../migrations/035_webhook_delivery_retries.sql"
        ))
        .execute(&db)
        .await
        .unwrap();
        let service = WebhookService::new(db.clone());
        let event_id = service
            .create_webhook_event("webhook", "test", serde_json::json!({}))
            .await
            .unwrap();
        let event = service.get_pending_deliveries(1).await.unwrap();
        let old_token = service.reserve_event(&event[0]).await.unwrap().unwrap();
        sqlx::query("UPDATE webhook_events SET next_attempt_at = 0 WHERE id = ?")
            .bind(&event_id)
            .execute(&db)
            .await
            .unwrap();
        assert!(!service
            .owns_reservation(&event_id, &old_token)
            .await
            .unwrap());
        let new_token = service.reserve_event(&event[0]).await.unwrap().unwrap();
        assert_ne!(old_token, new_token);
        service
            .finish_reserved_event(&event_id, &new_token, "pending", Some("new failure"), 1)
            .await
            .unwrap();
        let current = service
            .get_event_status("webhook", &event_id, "owner")
            .await
            .unwrap()
            .unwrap();
        assert!(service
            .finish_reserved_event(&event_id, &old_token, "pending", Some("stale failure"), 1)
            .await
            .is_err());
        let after = service
            .get_event_status("webhook", &event_id, "owner")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(after.last_error, current.last_error);
        assert_eq!(after.retries, current.retries);
        assert_eq!(after.next_attempt_at, current.next_attempt_at);
    }

    #[tokio::test]
    async fn terminal_events_are_not_resurrected_by_status_updates() {
        let db = legacy_database().await;
        sqlx::raw_sql(include_str!(
            "../../migrations/035_webhook_delivery_retries.sql"
        ))
        .execute(&db)
        .await
        .unwrap();
        let service = WebhookService::new(db);
        let event_id = service
            .create_webhook_event("webhook", "test", serde_json::json!({}))
            .await
            .unwrap();
        service
            .update_event_status(&event_id, "pending", Some("last failure"), 3)
            .await
            .unwrap();
        assert!(service
            .update_event_status(&event_id, "pending", None, 0)
            .await
            .is_err());
        assert!(service
            .update_event_status("missing", "delivered", None, 0)
            .await
            .is_err());
        let state = service
            .get_event_status("webhook", &event_id, "owner")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(state.status, "failed");
        assert_eq!(state.retries, 3);
        assert_eq!(state.last_error.as_deref(), Some("last failure"));
        assert!(state.next_attempt_at.is_none());
    }

    #[test]
    fn test_webhook_signature() {
        let payload = r#"{"event":"test"}"#;
        let secret = "my-secret";

        let signature = WebhookSignature::sign(payload, secret);
        assert!(WebhookSignature::verify(payload, secret, &signature));
    }

    #[test]
    fn webhook_signature_matches_hmac_sha256_test_vector() {
        // RFC 4231, test case 1.
        assert_eq!(
            WebhookSignature::sign("Hi There", &"\x0b".repeat(20)),
            "sha256=b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
    }

    #[test]
    fn test_event_type_conversion() {
        let event = WebhookEventType::CorridorHealthDegraded;
        assert_eq!(event.as_str(), "corridor.health_degraded");
        assert_eq!(
            WebhookEventType::from_str("corridor.health_degraded"),
            Some(WebhookEventType::CorridorHealthDegraded)
        );
    }
}
