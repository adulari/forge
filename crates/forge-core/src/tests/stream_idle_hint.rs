//! The outer stream-idle watchdog must not undercut a CLI bridge's own (larger) inner window: a
//! silent claude thinking/compacting for >180s was killed by the outer guard, taking a running
//! tool's process group with it.

use super::*;
use std::time::Duration;

struct HintedProvider(Option<Duration>);

#[async_trait::async_trait]
impl Provider for HintedProvider {
    async fn complete(
        &self,
        _model: &str,
        _messages: &[Message],
        _tools: &[ToolSpec],
        _on_event: &mut forge_provider::EventSink<'_>,
    ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
        unreachable!("only the idle hint is consulted")
    }

    fn stream_idle_hint(&self, _model: &str) -> Option<Duration> {
        self.0
    }
}

#[test]
fn outer_idle_is_raised_to_the_providers_hint_but_never_lowered_or_enabled() {
    let configured = Duration::from_secs(180);
    let hinted = HintedProvider(Some(Duration::from_secs(360)));
    assert_eq!(
        crate::effective_stream_idle(&hinted, "claude-cli::opus", configured),
        Duration::from_secs(360)
    );
    let small = HintedProvider(Some(Duration::from_secs(10)));
    assert_eq!(
        crate::effective_stream_idle(&small, "m", configured),
        configured
    );
    let none = HintedProvider(None);
    assert_eq!(
        crate::effective_stream_idle(&none, "m", configured),
        configured
    );
    assert_eq!(
        crate::effective_stream_idle(&hinted, "m", Duration::ZERO),
        Duration::ZERO,
        "a disabled watchdog stays disabled"
    );
}
