//! The tracing filter used when `RUST_LOG` is unset.

/// `warn` overall, with two sources silenced:
/// - genai's adapters log full raw HTTP failure bodies at ERROR on every retried request (a
///   multi-line 413/429 dump); forge's own error classifier prints a clean one-liner instead.
/// - rmcp's worker logs `worker quit with fatal` at ERROR when an MCP connect fails. The same
///   failure is reported once, concisely, by `forge_mcp` and retried; at login, before DNS works,
///   that duplicate ERROR appeared on every start.
///
/// `RUST_LOG` opts back in to all of it.
pub(crate) const DEFAULT_LOG_FILTER: &str = "warn,genai=off,rmcp::transport::worker=off";

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use tracing_subscriber::layer::SubscriberExt;

    struct Count(Arc<AtomicUsize>);

    impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for Count {
        fn on_event(&self, _: &tracing::Event<'_>, _: tracing_subscriber::layer::Context<'_, S>) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn rmcp_worker_errors_are_silenced_but_other_errors_and_warnings_are_not() {
        let seen = Arc::new(AtomicUsize::new(0));
        let subscriber = tracing_subscriber::registry()
            .with(tracing_subscriber::EnvFilter::new(DEFAULT_LOG_FILTER))
            .with(Count(Arc::clone(&seen)));
        tracing::subscriber::with_default(subscriber, || {
            tracing::error!(target: "rmcp::transport::worker", "worker quit with fatal");
            assert_eq!(seen.load(Ordering::SeqCst), 0);
            tracing::warn!(target: "forge_mcp", "failed to connect");
            tracing::error!(target: "forge_provider", "real failure");
            tracing::info!(target: "forge_mcp", "chatter");
            assert_eq!(seen.load(Ordering::SeqCst), 2);
        });
    }
}
