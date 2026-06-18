/// WebSocket trace context integration.
///
/// This module provides utilities for maintaining distributed trace context
/// across WebSocket connections and message handlers.

use axum::http::HeaderMap;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::Span;
use uuid::Uuid;

use crate::observability::trace_context;

/// Query parameters for WebSocket connection, including optional trace context.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WsTraceContext {
    /// Trace ID from parent HTTP request, if available.
    pub trace_id: Option<String>,
    /// Span ID from parent HTTP request, if available.
    pub span_id: Option<String>,
    /// Connection-specific ID for correlation.
    pub connection_id: String,
}

impl WsTraceContext {
    /// Create trace context from WebSocket headers (converted from HTTP upgrade).
    pub fn from_headers(headers: &HeaderMap) -> Self {
        let trace_id = trace_context::extract_trace_id_from_headers(headers);
        let span_id = None; // Span ID can be extracted similarly if needed

        Self {
            trace_id,
            span_id,
            connection_id: Uuid::new_v4().to_string(),
        }
    }

    /// Create a new trace context with a fresh connection ID.
    pub fn new() -> Self {
        Self {
            trace_id: None,
            span_id: None,
            connection_id: Uuid::new_v4().to_string(),
        }
    }

    /// Get a description of this trace context for logging.
    pub fn as_debug_string(&self) -> String {
        match &self.trace_id {
            Some(trace_id) => format!(
                "trace_id={} connection_id={}",
                trace_id, self.connection_id
            ),
            None => format!("connection_id={}", self.connection_id),
        }
    }
}

impl Default for WsTraceContext {
    fn default() -> Self {
        Self::new()
    }
}

/// A WebSocket message handler wrapper that preserves trace context.
///
/// This is used to ensure that all logs emitted during message handling
/// carry the correct trace ID and connection ID.
pub struct TraceAwareMessageHandler {
    trace_context: Arc<WsTraceContext>,
    message_span: Span,
}

impl TraceAwareMessageHandler {
    /// Create a new handler with the given trace context.
    pub fn new(trace_context: Arc<WsTraceContext>, message_type: &str) -> Self {
        let message_span = tracing::info_span!(
            "ws_message_handler",
            message_type = message_type,
            connection_id = %trace_context.connection_id,
            trace_id = trace_context.trace_id.as_deref().unwrap_or(""),
        );

        Self {
            trace_context,
            message_span,
        }
    }

    /// Execute a closure with the trace context active.
    ///
    /// All logs emitted within the closure will carry the correct trace context.
    pub async fn execute<F, Fut, T>(&self, f: F) -> T
    where
        F: FnOnce(Arc<WsTraceContext>) -> Fut,
        Fut: std::future::Future<Output = T>,
    {
        let _guard = self.message_span.enter();
        f(Arc::clone(&self.trace_context)).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ws_trace_context_creation() {
        let ctx = WsTraceContext::new();
        assert!(ctx.trace_id.is_none());
        assert!(ctx.span_id.is_none());
        assert!(!ctx.connection_id.is_empty());
    }

    #[test]
    fn test_ws_trace_context_debug_string_with_trace() {
        let ctx = WsTraceContext {
            trace_id: Some("abc123".to_string()),
            span_id: None,
            connection_id: "conn-456".to_string(),
        };
        let debug = ctx.as_debug_string();
        assert!(debug.contains("trace_id=abc123"));
        assert!(debug.contains("connection_id=conn-456"));
    }

    #[test]
    fn test_ws_trace_context_debug_string_without_trace() {
        let ctx = WsTraceContext::new();
        let debug = ctx.as_debug_string();
        assert!(debug.contains("connection_id="));
        assert!(!debug.contains("trace_id="));
    }

    #[tokio::test]
    async fn test_trace_aware_message_handler_execution() {
        let trace_context = Arc::new(WsTraceContext::new());
        let handler = TraceAwareMessageHandler::new(trace_context.clone(), "test_message");

        let result = handler
            .execute(|ctx| async move {
                format!("Connection: {}", ctx.connection_id)
            })
            .await;

        assert!(result.contains("Connection:"));
    }
}
