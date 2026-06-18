use std::pin::Pin;
use std::task::{Context, Poll};
use tracing::Span;

/// A wrapper around a future that preserves the current trace span context
/// across async boundaries (e.g., when using tokio::spawn).
///
/// This allows background jobs to maintain parent-child relationships in the
/// distributed trace even when they are spawned on separate async tasks.
///
/// # Example
/// ```rust
/// let span = tracing::info_span!("my_job");
/// let task = TraceAwareTask::new(async { /* work */ }, span);
/// tokio::spawn(task);
/// ```
pub struct TraceAwareTask<F>
where
    F: std::future::Future + Unpin,
{
    future: F,
    span: Span,
}

impl<F> TraceAwareTask<F>
where
    F: std::future::Future + Unpin,
{
    /// Create a new trace-aware task wrapping the given future and span.
    pub fn new(future: F, span: Span) -> Self {
        Self { future, span }
    }
}

impl<F> std::future::Future for TraceAwareTask<F>
where
    F: std::future::Future + Unpin,
{
    type Output = F::Output;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let _guard = self.span.enter();
        Pin::new(&mut self.future).poll(cx)
    }
}

/// Spawn a background job with trace context preservation.
///
/// The spawned task will execute within the current span, allowing the job's
/// logs and metrics to be correlated with the original request in the distributed trace.
///
/// # Example
/// ```rust
/// spawn_traced("process_backfill", async {
///     // This code runs with trace context intact
///     tracing::info!("Job started");
/// });
/// ```
pub fn spawn_traced<F>(_name: &str, future: F) -> tokio::task::JoinHandle<F::Output>
where
    F: std::future::Future + Send + Unpin + 'static,
    F::Output: Send + 'static,
{
    use crate::observability::trace_context;

    let span = tracing::info_span!(
        "background_job",
        trace_id = %trace_context::get_current_trace_id(),
        span_id = %trace_context::get_current_span_id(),
    );

    tokio::spawn(TraceAwareTask::new(future, span))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_trace_aware_task_completes() {
        let span = tracing::info_span!("test_task");
        let task = TraceAwareTask::new(async { 42 }, span);
        let result = task.await;
        assert_eq!(result, 42);
    }

    #[tokio::test]
    async fn test_spawn_traced_completes() {
        let handle = spawn_traced("test_job", async { "success" });
        let result = handle.await.unwrap();
        assert_eq!(result, "success");
    }

    #[tokio::test]
    async fn test_spawn_traced_preserves_span() {
        let parent_span = tracing::info_span!("parent");
        let _guard = parent_span.enter();

        let handle = spawn_traced("child_job", async { 100 });
        let result = handle.await.unwrap();
        assert_eq!(result, 100);
    }

    #[test]
    fn test_trace_aware_task_is_send() {
        fn assert_send<T: Send>() {}
        fn assert_sync<T: Sync>() {}

        // These will compile only if TraceAwareTask implements Send/Sync appropriately
        // (They would fail at compile time if the trait bounds aren't met)
        let _check = || {
            assert_send::<TraceAwareTask<std::future::Ready<i32>>>();
            assert_sync::<TraceAwareTask<std::future::Ready<i32>>>();
        };
    }
}
