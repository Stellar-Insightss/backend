/// Webhook API endpoints
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde_json::json;
use sqlx::SqlitePool;

use crate::auth_middleware::AuthUser;
use crate::webhooks::{CreateWebhookRequest, WebhookResponse, WebhookService};

/// POST /api/webhooks - Register a new webhook
#[utoipa::path(
    post,
    path = "/api/webhooks",
    request_body = CreateWebhookRequest,
    responses(
        (status = 201, description = "Webhook registered successfully"),
        (status = 400, description = "Invalid webhook URL or missing event types"),
        (status = 401, description = "Unauthorized"),
        (status = 500, description = "Internal server error")
    ),
    tag = "Webhooks"
)]
pub async fn register_webhook(
    State(db): State<SqlitePool>,
    auth_user: AuthUser,
    Json(request): Json<CreateWebhookRequest>,
) -> Result<Response, WebhookApiError> {
    // Validate webhook URL using centralized validation
    crate::validation::validate_webhook_url(&request.url)
        .map_err(|e| WebhookApiError::BadRequest(e.to_string()))?;

    // Validate event types
    if request.event_types.is_empty() {
        return Err(WebhookApiError::BadRequest(
            "At least one event type is required".to_string(),
        ));
    }

    let service = WebhookService::new(db);
    let response = service
        .register_webhook_with_secret(&auth_user.user_id, request)
        .await
        .map_err(|e| WebhookApiError::ServerError(e.to_string()))?;

    Ok((
        StatusCode::CREATED,
        [("Cache-Control", "no-store")],
        Json(response),
    )
        .into_response())
}

/// GET /api/webhooks - List webhooks for authenticated user
#[utoipa::path(
    get,
    path = "/api/webhooks",
    responses(
        (status = 200, description = "List of webhooks"),
        (status = 401, description = "Unauthorized"),
        (status = 500, description = "Internal server error")
    ),
    tag = "Webhooks"
)]
pub async fn list_webhooks(
    State(db): State<SqlitePool>,
    auth_user: AuthUser,
) -> Result<Response, WebhookApiError> {
    let service = WebhookService::new(db);
    let webhooks = service
        .list_webhooks(&auth_user.user_id)
        .await
        .map_err(|e| WebhookApiError::ServerError(e.to_string()))?;

    let response: Vec<WebhookResponse> = webhooks
        .into_iter()
        .map(|w| WebhookResponse {
            id: w.id,
            url: w.url,
            event_types: w
                .event_types
                .split(',')
                .map(std::string::ToString::to_string)
                .collect(),
            filters: w
                .filters
                .as_ref()
                .and_then(|f| serde_json::from_str(f).ok()),
            is_active: w.is_active,
            created_at: w.created_at,
        })
        .collect();

    Ok((StatusCode::OK, Json(json!({"webhooks": response}))).into_response())
}

/// DELETE /api/webhooks/:id - Delete/deactivate webhook
#[utoipa::path(
    delete,
    path = "/api/webhooks/{id}",
    params(
        ("id" = String, Path, description = "Webhook ID")
    ),
    responses(
        (status = 200, description = "Webhook deleted successfully"),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Webhook not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "Webhooks"
)]
pub async fn delete_webhook(
    State(db): State<SqlitePool>,
    auth_user: AuthUser,
    Path(webhook_id): Path<String>,
) -> Result<Response, WebhookApiError> {
    let service = WebhookService::new(db);
    let deleted = service
        .delete_webhook(&webhook_id, &auth_user.user_id)
        .await
        .map_err(|e| WebhookApiError::ServerError(e.to_string()))?;

    if !deleted {
        return Err(WebhookApiError::NotFound("Webhook not found".to_string()));
    }

    Ok((
        StatusCode::OK,
        Json(json!({"message": "Webhook deleted successfully"})),
    )
        .into_response())
}

/// GET /api/webhooks/:id - Get a single webhook by ID
#[utoipa::path(
    get,
    path = "/api/webhooks/{id}",
    params(
        ("id" = String, Path, description = "Webhook ID")
    ),
    responses(
        (status = 200, description = "Webhook details", body = WebhookResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden - not owner"),
        (status = 404, description = "Webhook not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "Webhooks"
)]
pub async fn get_webhook(
    State(db): State<SqlitePool>,
    auth_user: AuthUser,
    Path(webhook_id): Path<String>,
) -> Result<Response, WebhookApiError> {
    let service = WebhookService::new(db);
    let webhook = service
        .get_webhook(&webhook_id)
        .await
        .map_err(|e| WebhookApiError::ServerError(e.to_string()))?
        .ok_or_else(|| WebhookApiError::NotFound("Webhook not found".to_string()))?;

    if webhook.user_id != auth_user.user_id {
        return Err(WebhookApiError::Forbidden);
    }

    let response = WebhookResponse {
        id: webhook.id,
        url: webhook.url,
        event_types: webhook
            .event_types
            .split(',')
            .map(std::string::ToString::to_string)
            .collect(),
        filters: webhook
            .filters
            .as_ref()
            .and_then(|f| serde_json::from_str(f).ok()),
        is_active: webhook.is_active,
        created_at: webhook.created_at,
    };

    Ok((StatusCode::OK, Json(response)).into_response())
}

/// GET /api/webhooks/:id/events/:event_id - Get delivery status for an owned webhook
#[utoipa::path(
    get,
    path = "/api/webhooks/{id}/events/{event_id}",
    params(
        ("id" = String, Path, description = "Webhook ID"),
        ("event_id" = String, Path, description = "Webhook event ID")
    ),
    responses(
        (status = 200, description = "Webhook event delivery status"),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Webhook event not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "Webhooks"
)]
pub async fn get_webhook_event_status(
    State(db): State<SqlitePool>,
    auth_user: AuthUser,
    Path((webhook_id, event_id)): Path<(String, String)>,
) -> Result<Response, WebhookApiError> {
    let service = WebhookService::new(db);
    let event = service
        .get_event_status(&webhook_id, &event_id, &auth_user.user_id)
        .await
        .map_err(|e| WebhookApiError::ServerError(e.to_string()))?
        .ok_or_else(|| WebhookApiError::NotFound("Webhook event not found".to_string()))?;

    Ok((StatusCode::OK, Json(event)).into_response())
}

/// POST /api/webhooks/:id/test - Queue a test event for delivery
#[utoipa::path(
    post,
    path = "/api/webhooks/{id}/test",
    params(
        ("id" = String, Path, description = "Webhook ID")
    ),
    responses(
        (status = 200, description = "Test event queued"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden - not owner"),
        (status = 404, description = "Webhook not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "Webhooks"
)]
pub async fn test_webhook(
    State(db): State<SqlitePool>,
    auth_user: AuthUser,
    Path(webhook_id): Path<String>,
) -> Result<Response, WebhookApiError> {
    let service = WebhookService::new(db);

    // Get webhook and verify ownership
    let webhook = service
        .get_webhook(&webhook_id)
        .await
        .map_err(|e| WebhookApiError::ServerError(e.to_string()))?
        .ok_or_else(|| WebhookApiError::NotFound("Webhook not found".to_string()))?;

    if webhook.user_id != auth_user.user_id {
        return Err(WebhookApiError::Forbidden);
    }

    let test_payload = json!({
        "message": "This is a test webhook delivery",
        "webhook_id": webhook_id,
    });

    let event_id = service
        .create_webhook_event(&webhook_id, "test", test_payload.clone())
        .await
        .map_err(|e| WebhookApiError::ServerError(e.to_string()))?;

    Ok((
        StatusCode::OK,
        Json(json!({
            "message": "Test event queued for delivery",
            "event_id": event_id,
            "payload": test_payload
        })),
    )
        .into_response())
}

/// Webhook API Error types
#[derive(Debug)]
pub enum WebhookApiError {
    NotFound(String),
    BadRequest(String),
    Forbidden,
    ServerError(String),
}

impl IntoResponse for WebhookApiError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            Self::NotFound(msg) => (StatusCode::NOT_FOUND, msg),
            Self::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg),
            Self::Forbidden => (
                StatusCode::FORBIDDEN,
                "You don't have permission to access this webhook".to_string(),
            ),
            Self::ServerError(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg),
        };

        (status, Json(json!({"error": message}))).into_response()
    }
}

/// Create webhook routes
pub fn routes(db: SqlitePool) -> Router {
    Router::new().nest("/api/webhooks", resource_routes(db))
}

/// Resource-relative routes for mounting under the versioned API.
pub(crate) fn resource_routes(db: SqlitePool) -> Router {
    Router::new()
        .route("/", post(register_webhook).get(list_webhooks))
        .route("/:id", get(get_webhook).delete(delete_webhook))
        .route("/:id/events/:event_id", get(get_webhook_event_status))
        .route("/:id/test", post(test_webhook))
        .with_state(db)
}

/// Protected routes mounted by the v1 API, with the existing nested paths
/// retained as aliases for clients that already use them.
pub(crate) fn versioned_routes(db: SqlitePool) -> Router {
    Router::new()
        .nest("/webhooks", resource_routes(db.clone()).merge(routes(db)))
        .layer(axum::middleware::from_fn(
            crate::auth_middleware::auth_middleware,
        ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use sqlx::sqlite::SqlitePoolOptions;
    use tower::ServiceExt;

    async fn event_status_db() -> anyhow::Result<SqlitePool> {
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
        sqlx::raw_sql(
            "INSERT INTO users (id) VALUES ('owner'), ('other');
             INSERT INTO webhooks (id, user_id, url, event_types, secret)
             VALUES ('webhook-1', 'owner', 'https://example.com/webhook', 'test', 'private-secret'),
                    ('webhook-other', 'other', 'https://example.com/other', 'test', 'other-secret');
             INSERT INTO webhook_events
                 (id, webhook_id, event_type, payload, status, retries, last_error,
                  created_at, next_attempt_at, last_attempt_at, delivered_at)
             VALUES
                 ('event-1', 'webhook-1', 'test', '{\"private\":\"payload\"}', 'pending', 2,
                  'Webhook failed with status 503', '2023-11-14T22:13:20Z',
                  1700000090, 1700000030, NULL);",
        )
        .execute(&pool)
        .await?;

        Ok(pool)
    }

    fn event_status_request(
        webhook_id: &str,
        event_id: &str,
        user_id: Option<&str>,
    ) -> Result<Request<Body>, axum::http::Error> {
        let mut request = Request::builder()
            .uri(format!("/api/webhooks/{webhook_id}/events/{event_id}"))
            .body(Body::empty())?;

        if let Some(user_id) = user_id {
            // Auth middleware adds this extension only after validating the access token.
            request.extensions_mut().insert(AuthUser {
                user_id: user_id.to_string(),
                username: user_id.to_string(),
            });
        }

        Ok(request)
    }

    #[tokio::test]
    async fn event_status_requires_authentication() -> anyhow::Result<()> {
        let app = routes(event_status_db().await?);
        let response = app
            .oneshot(event_status_request("webhook-1", "event-1", None)?)
            .await?;

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        Ok(())
    }

    #[tokio::test]
    async fn event_status_returns_delivery_metadata_for_owner() -> anyhow::Result<()> {
        let app = routes(event_status_db().await?);
        let response = app
            .oneshot(event_status_request("webhook-1", "event-1", Some("owner"))?)
            .await?;

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 4096).await?;
        let body: serde_json::Value = serde_json::from_slice(&body)?;
        assert_eq!(
            body,
            json!({
                "id": "event-1",
                "webhook_id": "webhook-1",
                "event_type": "test",
                "status": "pending",
                "retries": 2,
                "last_error": "Webhook failed with status 503",
                "created_at": "2023-11-14T22:13:20Z",
                "next_attempt_at": 1700000090_i64,
                "last_attempt_at": 1700000030_i64,
                "delivered_at": null
            })
        );
        Ok(())
    }

    #[tokio::test]
    async fn event_status_hides_nonowned_missing_and_mismatched_events() -> anyhow::Result<()> {
        let app = routes(event_status_db().await?);

        for (webhook_id, event_id, user_id) in [
            ("webhook-1", "event-1", "other"),
            ("webhook-other", "event-1", "other"),
            ("missing-webhook", "event-1", "owner"),
            ("webhook-1", "missing-event", "owner"),
        ] {
            let response = app
                .clone()
                .oneshot(event_status_request(webhook_id, event_id, Some(user_id))?)
                .await?;

            assert_eq!(response.status(), StatusCode::NOT_FOUND);
            let body = axum::body::to_bytes(response.into_body(), 4096).await?;
            let body: serde_json::Value = serde_json::from_slice(&body)?;
            assert_eq!(body, json!({"error": "Webhook event not found"}));
        }

        Ok(())
    }

    #[tokio::test]
    async fn versioned_event_status_validates_jwt_at_the_mounted_path() -> anyhow::Result<()> {
        use crate::auth::Claims;
        use crate::auth_middleware::JwtSecret;
        use jsonwebtoken::{encode, EncodingKey, Header};
        use std::sync::Arc;

        let secret = "webhook-status-test-key-at-least-32-bytes";
        let app = Router::new()
            .nest("/api/v1", versioned_routes(event_status_db().await?))
            .layer(axum::Extension(JwtSecret(Arc::from(secret))));
        let token = |user_id: &str, token_type: &str, key: &str| {
            let now = chrono::Utc::now().timestamp();
            encode(
                &Header::default(),
                &Claims {
                    sub: user_id.to_string(),
                    username: user_id.to_string(),
                    iat: now,
                    exp: now + 3600,
                    token_type: token_type.to_string(),
                },
                &EncodingKey::from_secret(key.as_bytes()),
            )
        };

        let owner_token = token("owner", "access", secret)?;
        for (uri, bearer, expected) in [
            (
                "/api/v1/webhooks/webhook-1/events/event-1",
                None,
                StatusCode::UNAUTHORIZED,
            ),
            (
                "/api/v1/webhooks/webhook-1/events/event-1",
                Some("not-a-token".to_string()),
                StatusCode::UNAUTHORIZED,
            ),
            (
                "/api/v1/webhooks/webhook-1/events/event-1",
                Some(token("owner", "access", "wrong-key")?),
                StatusCode::UNAUTHORIZED,
            ),
            (
                "/api/v1/webhooks/webhook-1/events/event-1",
                Some(token("owner", "refresh", secret)?),
                StatusCode::UNAUTHORIZED,
            ),
            (
                "/api/v1/webhooks/webhook-1/events/event-1",
                Some(token("other", "access", secret)?),
                StatusCode::NOT_FOUND,
            ),
            (
                "/api/v1/webhooks/webhook-1/events/event-1",
                Some(owner_token.clone()),
                StatusCode::OK,
            ),
            (
                "/api/v1/webhooks/api/webhooks/webhook-1/events/event-1",
                Some(owner_token),
                StatusCode::OK,
            ),
        ] {
            let mut request = Request::builder().uri(uri);
            if let Some(bearer) = bearer {
                request = request.header("Authorization", format!("Bearer {bearer}"));
            }
            let response = app.clone().oneshot(request.body(Body::empty())?).await?;
            assert_eq!(response.status(), expected, "route {uri}");
        }
        Ok(())
    }

    #[tokio::test]
    async fn registration_returns_signing_secret_once_to_authenticated_owner() -> anyhow::Result<()>
    {
        use crate::auth::Claims;
        use crate::auth_middleware::JwtSecret;
        use crate::webhooks::WebhookSignature;
        use jsonwebtoken::{encode, EncodingKey, Header};
        use std::sync::Arc;

        let pool = event_status_db().await?;
        let jwt_secret = "webhook-registration-test-key-at-least-32-bytes";
        let app = Router::new()
            .nest("/api/v1", versioned_routes(pool.clone()))
            .layer(axum::Extension(JwtSecret(Arc::from(jwt_secret))));
        let now = chrono::Utc::now().timestamp();
        let token = encode(
            &Header::default(),
            &Claims {
                sub: "owner".to_string(),
                username: "owner".to_string(),
                iat: now,
                exp: now + 3600,
                token_type: "access".to_string(),
            },
            &EncodingKey::from_secret(jwt_secret.as_bytes()),
        )?;
        let payload = json!({
            "url": "https://example.com/receiver",
            "event_types": ["test"]
        });
        let registration = |authorization: Option<&str>| {
            let mut request = Request::builder()
                .method("POST")
                .uri("/api/v1/webhooks")
                .header("Content-Type", "application/json");
            if let Some(authorization) = authorization {
                request = request.header("Authorization", format!("Bearer {authorization}"));
            }
            request.body(Body::from(payload.to_string()))
        };

        let denied = app.clone().oneshot(registration(None)?).await?;
        assert_eq!(denied.status(), StatusCode::UNAUTHORIZED);
        let response = app.clone().oneshot(registration(Some(&token))?).await?;
        assert_eq!(response.status(), StatusCode::CREATED);
        assert_eq!(response.headers().get("Cache-Control").unwrap(), "no-store");
        let body = axum::body::to_bytes(response.into_body(), 4096).await?;
        let body: serde_json::Value = serde_json::from_slice(&body)?;
        let webhook_id = body["id"].as_str().unwrap();
        let secret = body["secret"].as_str().unwrap();
        assert!(!secret.is_empty());

        let service = WebhookService::new(pool);
        let stored = service.get_webhook(webhook_id).await?.unwrap();
        assert_eq!(secret, stored.secret);
        let sample_body = r#"{"event":"test","data":{"value":42}}"#;
        let signature = WebhookSignature::sign(sample_body, &stored.secret);
        assert!(WebhookSignature::verify(sample_body, secret, &signature));
        let event_id = service
            .create_webhook_event(webhook_id, "test", json!({"value": 42}))
            .await?;

        for path in [
            "/api/v1/webhooks".to_string(),
            format!("/api/v1/webhooks/{webhook_id}"),
            format!("/api/v1/webhooks/{webhook_id}/events/{event_id}"),
        ] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(&path)
                        .header("Authorization", format!("Bearer {token}"))
                        .body(Body::empty())?,
                )
                .await?;
            assert_eq!(response.status(), StatusCode::OK, "route {path}");
            let body = axum::body::to_bytes(response.into_body(), 8192).await?;
            let body: serde_json::Value = serde_json::from_slice(&body)?;
            assert!(body.get("secret").is_none());
            if let Some(webhooks) = body["webhooks"].as_array() {
                assert!(webhooks
                    .iter()
                    .all(|webhook| webhook.get("secret").is_none()));
            }
            assert!(!body.to_string().contains(secret));
        }
        Ok(())
    }
}
