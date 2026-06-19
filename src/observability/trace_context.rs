use axum::http::HeaderMap;
use opentelemetry::global;
use opentelemetry::trace::TraceContextExt as _;
use tracing::Span;

/// Extract W3C TraceContext (traceparent header) from request headers.
///
/// Returns the trace ID if a valid traceparent header exists, otherwise None.
/// Format: `00-trace_id-parent_span_id-flags`
pub fn extract_trace_id_from_headers(headers: &HeaderMap) -> Option<String> {
    headers
        .get("traceparent")
        .and_then(|h| h.to_str().ok())
        .and_then(|traceparent| {
            let parts: Vec<&str> = traceparent.split('-').collect();
            if parts.len() >= 2 {
                Some(parts[1].to_string())
            } else {
                None
            }
        })
}

/// Inject the current OpenTelemetry trace context into outbound request headers.
///
/// Uses the globally registered propagator (W3C TraceContext by default) to ensure
/// `traceparent` and `tracestate` headers are properly formatted and forwarded to
/// downstream services.
///
/// # Example
/// ```rust
/// let mut headers = HeaderMap::new();
/// inject_trace_context_to_headers(&mut headers);
/// ```
pub fn inject_trace_context_to_headers(headers: &mut HeaderMap) {
    let mut carrier = std::collections::HashMap::new();
    let cx = opentelemetry::Context::current();

    global::get_text_map_propagator(|propagator| {
        propagator.inject_context(&cx, &mut carrier);
    });

    for (key, value) in carrier {
        if let Ok(header_value) = axum::http::HeaderValue::from_str(&value) {
            headers.insert(
                axum::http::HeaderName::from_bytes(key.as_bytes()).unwrap_or_else(|_| {
                    axum::http::header::HeaderName::from_static("traceparent")
                }),
                header_value,
            );
        }
    }
}

/// Get the current trace ID from the active OpenTelemetry span context.
///
/// Returns a valid trace ID string if a span is active, otherwise an empty string.
pub fn get_current_trace_id() -> String {
    let cx = opentelemetry::Context::current();
    let span = cx.span();
    let span_context = span.span_context();

    if span_context.is_valid() {
        span_context.trace_id().to_string()
    } else {
        String::new()
    }
}

/// Get the current span ID from the active OpenTelemetry span context.
///
/// Returns a valid span ID string if a span is active, otherwise an empty string.
pub fn get_current_span_id() -> String {
    let cx = opentelemetry::Context::current();
    let span = cx.span();
    let span_context = span.span_context();

    if span_context.is_valid() {
        span_context.span_id().to_string()
    } else {
        String::new()
    }
}

/// Create a child span with the given name, inheriting the current trace context.
///
/// This is useful for background jobs, RPC calls, and other operations that need
/// their own span but should be linked to the parent request trace.
///
/// # Example
/// ```rust
/// let job_span = create_child_span("process_backfill_job");
/// let _guard = job_span.enter();
/// // ... perform work with trace context ...
/// ```
pub fn create_child_span(_name: &str) -> Span {
    tracing::info_span!(
        "child_operation",
        trace_id = %get_current_trace_id(),
        span_id = %get_current_span_id(),
    )
}

/// Extract W3C TraceContext from headers into a carrier map for propagator use.
///
/// Returns a HashMap suitable for use with `propagator.extract()`.
pub fn extract_trace_context_carrier(headers: &HeaderMap) -> std::collections::HashMap<String, String> {
    headers
        .iter()
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|v| (name.as_str().to_owned(), v.to_owned()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_trace_id_from_valid_traceparent() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "traceparent",
            "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01"
                .parse()
                .unwrap(),
        );

        let trace_id = extract_trace_id_from_headers(&headers);
        assert_eq!(trace_id, Some("4bf92f3577b34da6a3ce929d0e0e4736".to_string()));
    }

    #[test]
    fn test_extract_trace_id_from_invalid_traceparent() {
        let mut headers = HeaderMap::new();
        headers.insert("traceparent", "invalid-format".parse().unwrap());

        let trace_id = extract_trace_id_from_headers(&headers);
        assert_eq!(trace_id, None);
    }

    #[test]
    fn test_extract_trace_id_missing_header() {
        let headers = HeaderMap::new();
        let trace_id = extract_trace_id_from_headers(&headers);
        assert_eq!(trace_id, None);
    }

    #[test]
    fn test_extract_trace_context_carrier() {
        let mut headers = HeaderMap::new();
        headers.insert("traceparent", "00-trace-span-01".parse().unwrap());
        headers.insert("tracestate", "vendor=value".parse().unwrap());

        let carrier = extract_trace_context_carrier(&headers);
        assert_eq!(carrier.get("traceparent"), Some(&"00-trace-span-01".to_string()));
        assert_eq!(carrier.get("tracestate"), Some(&"vendor=value".to_string()));
    }

    #[test]
    fn test_get_current_trace_id_without_span() {
        let trace_id = get_current_trace_id();
        assert_eq!(trace_id, String::new());
    }

    #[test]
    fn test_create_child_span() {
        let span = create_child_span("test_job");
        assert_eq!(span.name(), "test_job");
    }
}
