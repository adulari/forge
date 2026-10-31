//! Factory Droid loop controls that need more than a flag read, each behind its own `[harness]`
//! flag (default off) so a benchmark round can A/B them: `stream_silence_retry`,
//! `append_only_prefix` and `compaction_summary_cap`.

use forge_provider::ProviderError;

/// Droid aborts a model stream after four silent minutes.
pub(crate) const STREAM_SILENCE_SECS: u64 = 240;

/// Same-model retries of a stalled stream before the normal failover path takes over.
pub(crate) const STALL_RETRY_MAX: u32 = 2;

/// Droid's compaction summary budget.
pub(crate) const SUMMARY_CAP_TOKENS: usize = 16_000;

/// Opens the transcript note that carries the task list under `append_only_prefix`.
pub(crate) const TASK_SNAPSHOT_PREFIX: &str = "[task list snapshot]\n";

const STALL_MARK: &str = " — no data for ";

#[cfg(test)]
pub(crate) static SILENCE_FLOOR_OVERRIDE: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);

fn silence_floor() -> std::time::Duration {
    #[cfg(test)]
    {
        let secs = SILENCE_FLOOR_OVERRIDE.load(std::sync::atomic::Ordering::Relaxed);
        if secs > 0 {
            return std::time::Duration::from_secs(secs);
        }
    }
    std::time::Duration::from_secs(STREAM_SILENCE_SECS)
}

/// The stream idle window under `stream_silence_retry`: at least four minutes. A disabled
/// watchdog (zero) stays disabled.
pub(crate) fn silence_window(configured: std::time::Duration, on: bool) -> std::time::Duration {
    if !on || configured.is_zero() {
        return configured;
    }
    configured.max(silence_floor())
}

/// `[harness] wait_job`: show `shell_job` its `wait` action. The tool always runs `wait`; only the
/// advertised spec changes, so a session with the flag off keeps its tool list byte-identical.
pub(crate) fn advertise_job_wait(spec: &mut forge_provider::ToolSpec) {
    if spec.name != "shell_job" {
        return;
    }
    spec.description = format!(
        "Wait for a background job to exit, or inspect and control jobs. Action wait blocks up to \
         timeout_secs and returns the exit code and log tail; use it instead of polling log. {}",
        spec.description
    );
    if let Some(action) = spec.schema.pointer_mut("/properties/action") {
        if let Some(values) = action.get_mut("enum").and_then(|e| e.as_array_mut()) {
            values.push("wait".into());
        }
        action["description"] = "list, log, status, stop, expose, or wait.".into();
    }
    if let Some(props) = spec
        .schema
        .get_mut("properties")
        .and_then(|p| p.as_object_mut())
    {
        props.insert(
            "timeout_secs".to_string(),
            serde_json::json!({
                "type": "integer", "minimum": 1, "maximum": 600,
                "description": "For wait: the longest to block for the job to exit (default 60)."
            }),
        );
    }
}

/// [`silence_window`] for a session's config.
pub(crate) fn silence_window_in(
    config: &forge_config::Config,
    configured: std::time::Duration,
) -> std::time::Duration {
    silence_window(configured, config.harness.stream_silence_retry)
}

impl crate::Session {
    /// The summary ceiling `compaction_summary_cap` imposes, when on.
    pub(crate) fn summary_cap(&self) -> Option<usize> {
        self.config
            .harness
            .compaction_summary_cap
            .then_some(SUMMARY_CAP_TOKENS)
    }

    /// Whether the failed request is retried on the same model: a transient error within the
    /// usual budget, or a silent stream under `stream_silence_retry` (announced, `retries`
    /// already spent this turn).
    pub(crate) fn same_model_retry_due(
        &mut self,
        model: &str,
        error: &ProviderError,
        retries: u32,
    ) -> bool {
        let stall = self.config.harness.stream_silence_retry
            && is_stream_stall(error)
            && retries < STALL_RETRY_MAX;
        if stall {
            self.presenter
                .emit(forge_types::PresenterEvent::Warning(format!(
                    "{error} — retrying {model} ({}/{STALL_RETRY_MAX})",
                    retries + 1
                )));
        }
        stall
            || (retries < crate::MAX_TRANSIENT_RETRIES
                && crate::should_retry_same_model_transient(model, error))
    }
}

/// The error the idle watchdog raises for a silent stream. `what` says whether the stream ever
/// started.
pub(crate) fn stall_error(what: &str, secs: u64) -> ProviderError {
    ProviderError::Unavailable(format!("{what}{STALL_MARK}{secs}s"))
}

/// Whether `error` is the idle watchdog's, not a provider outage.
pub(crate) fn is_stream_stall(error: &ProviderError) -> bool {
    matches!(error, ProviderError::Unavailable(m) if m.contains(STALL_MARK))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn the_window_is_raised_only_when_on_and_never_enabled_from_zero() {
        assert_eq!(
            silence_window(Duration::from_secs(180), true),
            Duration::from_secs(240)
        );
        assert_eq!(
            silence_window(Duration::from_secs(600), true),
            Duration::from_secs(600)
        );
        assert_eq!(
            silence_window(Duration::from_secs(180), false),
            Duration::from_secs(180)
        );
        assert_eq!(silence_window(Duration::ZERO, true), Duration::ZERO);
    }

    #[test]
    fn a_watchdog_error_is_a_stall_and_an_outage_is_not() {
        assert!(is_stream_stall(&stall_error("stream stalled", 240)));
        assert!(is_stream_stall(&stall_error(
            "no response from the model",
            120
        )));
        assert!(!is_stream_stall(&ProviderError::Unavailable(
            "503 service unavailable".into()
        )));
    }
}
