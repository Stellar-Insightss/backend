/// Instrumented health check endpoints with span-based tracing.
///
/// This module provides Axum handlers for comprehensive health checks that are
/// properly instrumented with OpenTelemetry spans for distributed tracing.

use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::time::Instant;
use tracing::{instrument, Span};

use crate::cache::CacheManager;
use crate::database::Database;
use crate::rpc::StellarRpcClient;

/// Component-level health check result.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComponentHealth {
    pub healthy: bool,
    pub response_time_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl ComponentHealth {
    pub fn healthy(response_time_ms: u64) -> Self {
        Self {
            healthy: true,
            response_time_ms,
            message: None,
        }
    }

    pub fn unhealthy(response_time_ms: u64, message: String) -> Self {
        Self {
            healthy: false,
            response_time_ms,
            message: Some(message),
        }
    }

    pub fn degraded(response_time_ms: u64, message: String) -> Self {
        Self {
            healthy: true,
            response_time_ms,
            message: Some(message),
        }
    }
}

/// Comprehensive health check response with dependency status.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthCheckResponse {
    pub status: String,
    pub timestamp: DateTime<Utc>,
    pub version: String,
    pub uptime_seconds: u64,
    pub components: HealthComponents,
    pub overall_response_time_ms: u64,
}

/// Individual component health statuses.
#[derive(Debug, Serialize, Deserialize)]
pub struct HealthComponents {
    pub database: ComponentHealth,
    pub cache: ComponentHealth,
    pub rpc: ComponentHealth,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub websocket: Option<ComponentHealth>,
}

/// Check database connectivity and basic health.
#[instrument(skip(db), fields(component = "database"), err)]
async fn check_database_health(db: &Database) -> ComponentHealth {
    let start = Instant::now();

    match db.health_check().await {
        Ok(_) => {
            let elapsed = start.elapsed().as_millis() as u64;
            tracing::info!(response_time_ms = elapsed, "Database health check passed");
            ComponentHealth::healthy(elapsed)
        }
        Err(e) => {
            let elapsed = start.elapsed().as_millis() as u64;
            let message = format!("Database connection failed: {}", e);
            tracing::warn!(response_time_ms = elapsed, error = %e, "Database health check failed");
            ComponentHealth::unhealthy(elapsed, message)
        }
    }
}

/// Check cache connectivity and basic health.
#[instrument(skip(cache), fields(component = "cache"), err)]
async fn check_cache_health(cache: &CacheManager) -> ComponentHealth {
    let start = Instant::now();

    match cache.health_check().await {
        Ok(_) => {
            let elapsed = start.elapsed().as_millis() as u64;
            tracing::info!(response_time_ms = elapsed, "Cache health check passed");
            ComponentHealth::healthy(elapsed)
        }
        Err(e) => {
            let elapsed = start.elapsed().as_millis() as u64;
            let message = format!("Cache connection failed: {}", e);
            tracing::warn!(response_time_ms = elapsed, error = %e, "Cache health check failed");
            ComponentHealth::unhealthy(elapsed, message)
        }
    }
}

/// Check Stellar RPC connectivity and basic health.
#[instrument(skip(rpc), fields(component = "rpc"), err)]
async fn check_rpc_health(rpc: &StellarRpcClient) -> ComponentHealth {
    let start = Instant::now();

    match rpc.health_check().await {
        Ok(_) => {
            let elapsed = start.elapsed().as_millis() as u64;
            tracing::info!(response_time_ms = elapsed, "RPC health check passed");
            ComponentHealth::healthy(elapsed)
        }
        Err(e) => {
            let elapsed = start.elapsed().as_millis() as u64;
            let message = format!("RPC connection failed: {}", e);
            tracing::warn!(response_time_ms = elapsed, error = %e, "RPC health check failed");
            ComponentHealth::unhealthy(elapsed, message)
        }
    }
}

/// Determine overall health status based on component health.
fn determine_overall_status(components: &HealthComponents) -> String {
    match (
        components.database.healthy,
        components.cache.healthy,
        components.rpc.healthy,
    ) {
        (true, true, true) => "healthy".to_string(),
        (true, true, false) | (true, false, true) => "degraded".to_string(),
        _ => "unhealthy".to_string(),
    }
}

/// Calculate total response time across all health checks.
fn total_response_time(components: &HealthComponents) -> u64 {
    let mut total = components.database.response_time_ms
        + components.cache.response_time_ms
        + components.rpc.response_time_ms;

    if let Some(ws) = &components.websocket {
        total += ws.response_time_ms;
    }

    total
}

/// Main health check endpoint handler.
///
/// This handler runs all component health checks in parallel and returns
/// a comprehensive status. It is instrumented with OpenTelemetry tracing.
#[instrument(skip(db, cache, rpc), fields(endpoint = "/health"), err)]
pub async fn health_check_handler(
    State(db): State<Database>,
    State(cache): State<CacheManager>,
    State(rpc): State<StellarRpcClient>,
) -> impl IntoResponse {
    let start = Instant::now();
    tracing::info!("Starting health check");

    // Run all health checks in parallel
    let (db_health, cache_health, rpc_health) =
        tokio::join!(
            check_database_health(&db),
            check_cache_health(&cache),
            check_rpc_health(&rpc)
        );

    let components = HealthComponents {
        database: db_health,
        cache: cache_health,
        rpc: rpc_health,
        websocket: None,
    };

    let overall_status = determine_overall_status(&components);
    let total_time = total_response_time(&components);
    let uptime = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());

    let response = HealthCheckResponse {
        status: overall_status.clone(),
        timestamp: Utc::now(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        uptime_seconds: uptime,
        components,
        overall_response_time_ms: total_time,
    };

    let status_code = match overall_status.as_str() {
        "healthy" => StatusCode::OK,
        "degraded" => StatusCode::OK,
        _ => StatusCode::SERVICE_UNAVAILABLE,
    };

    tracing::info!(
        status = %overall_status,
        response_time_ms = total_time,
        "Health check completed"
    );

    (status_code, Json(response))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_component_health_creation() {
        let healthy = ComponentHealth::healthy(50);
        assert!(healthy.healthy);
        assert_eq!(healthy.response_time_ms, 50);
        assert_eq!(healthy.message, None);

        let unhealthy = ComponentHealth::unhealthy(100, "Connection failed".to_string());
        assert!(!unhealthy.healthy);
        assert_eq!(unhealthy.message, Some("Connection failed".to_string()));
    }

    #[test]
    fn test_determine_overall_status_all_healthy() {
        let components = HealthComponents {
            database: ComponentHealth::healthy(10),
            cache: ComponentHealth::healthy(5),
            rpc: ComponentHealth::healthy(20),
            websocket: None,
        };

        let status = determine_overall_status(&components);
        assert_eq!(status, "healthy");
    }

    #[test]
    fn test_determine_overall_status_one_failed() {
        let components = HealthComponents {
            database: ComponentHealth::healthy(10),
            cache: ComponentHealth::healthy(5),
            rpc: ComponentHealth::unhealthy(0, "Failed".to_string()),
            websocket: None,
        };

        let status = determine_overall_status(&components);
        assert_eq!(status, "degraded");
    }

    #[test]
    fn test_determine_overall_status_multiple_failed() {
        let components = HealthComponents {
            database: ComponentHealth::unhealthy(0, "Failed".to_string()),
            cache: ComponentHealth::unhealthy(0, "Failed".to_string()),
            rpc: ComponentHealth::healthy(20),
            websocket: None,
        };

        let status = determine_overall_status(&components);
        assert_eq!(status, "unhealthy");
    }

    #[test]
    fn test_total_response_time_without_websocket() {
        let components = HealthComponents {
            database: ComponentHealth::healthy(10),
            cache: ComponentHealth::healthy(5),
            rpc: ComponentHealth::healthy(20),
            websocket: None,
        };

        let total = total_response_time(&components);
        assert_eq!(total, 35);
    }

    #[test]
    fn test_total_response_time_with_websocket() {
        let components = HealthComponents {
            database: ComponentHealth::healthy(10),
            cache: ComponentHealth::healthy(5),
            rpc: ComponentHealth::healthy(20),
            websocket: Some(ComponentHealth::healthy(15)),
        };

        let total = total_response_time(&components);
        assert_eq!(total, 50);
    }
}
