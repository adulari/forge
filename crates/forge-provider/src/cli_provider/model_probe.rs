//! How long a bridge CLI gets to enumerate its models, and when a recent answer is reused.
//!
//! `agy models` is not a local lookup: it fetches the list over the network and takes 1.3-7 s
//! on a quiet machine (measured), longer while the daemon is starting under load or before DNS
//! works. A flat 5 s budget turned that into a failure on every daemon start.

use std::time::Duration;

use super::{recall_bridge_models_at, BridgeModelSource, BridgeModels, CliKind};

/// The local CLIs answer from their own files; agy waits on the network.
const LOCAL_PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const AGY_PROBE_TIMEOUT: Duration = Duration::from_secs(30);

/// A successful `agy models` this recent is reused instead of re-run on every daemon restart.
const AGY_LIST_TTL_SECS: u64 = 60 * 60;

impl CliKind {
    pub(super) fn probe_timeout(self) -> Duration {
        match self {
            CliKind::Antigravity => AGY_PROBE_TIMEOUT,
            CliKind::ClaudeCode | CliKind::Codex => LOCAL_PROBE_TIMEOUT,
        }
    }

    /// The list this bridge last advertised, if it is recent enough that asking again is wasted
    /// work. Only the networked probe has a TTL; the local ones are cheap to run every time.
    fn list_within_ttl(self, cached: Option<(Vec<String>, u64)>) -> Option<Vec<String>> {
        let ttl = match self {
            CliKind::Antigravity => AGY_LIST_TTL_SECS,
            CliKind::ClaudeCode | CliKind::Codex => return None,
        };
        cached
            .filter(|(_, age)| *age < ttl)
            .map(|(models, _)| models)
    }

    /// [`Self::bridge_models_detailed`] for discovery: reuses a recent successful list rather than
    /// paying the CLI's startup (and, for agy, a network round trip) on every daemon restart.
    /// `forge doctor` keeps probing live.
    pub async fn bridge_models_for_discovery(self) -> BridgeModels {
        let cached = super::bridge_model_cache_path()
            .and_then(|path| recall_bridge_models_at(&path, self.prefix(), super::now_unix()));
        match self.list_within_ttl(cached) {
            Some(models) => BridgeModels {
                models,
                source: BridgeModelSource::Live,
                probe_error: None,
            },
            None => self.bridge_models_detailed().await,
        }
    }
}

/// The one-line warning for a probe that failed, saying what the mesh uses instead.
pub(super) fn failure_warning(prefix: &str, error: &str, cached_age_secs: Option<u64>) -> String {
    match cached_age_secs {
        Some(age) => format!(
            "{prefix} model discovery failed: {error} — using the list it advertised {}m ago",
            age / 60
        ),
        None => format!(
            "{prefix} model discovery failed: {error} — the mesh will use an unverified model \
             list for this bridge"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cached(age: u64) -> Option<(Vec<String>, u64)> {
        Some((vec!["gemini-3.7-flash-low".to_string()], age))
    }

    #[test]
    fn agy_gets_a_budget_that_fits_its_measured_worst_case() {
        assert!(CliKind::Antigravity.probe_timeout() >= Duration::from_secs(15));
        assert_eq!(CliKind::Codex.probe_timeout(), Duration::from_secs(5));
    }

    #[test]
    fn a_recent_agy_list_is_reused_and_an_old_one_is_not() {
        assert!(CliKind::Antigravity.list_within_ttl(cached(60)).is_some());
        assert!(CliKind::Antigravity
            .list_within_ttl(cached(AGY_LIST_TTL_SECS))
            .is_none());
        assert!(CliKind::Antigravity.list_within_ttl(None).is_none());
    }

    #[test]
    fn local_bridges_are_always_probed_live() {
        assert!(CliKind::Codex.list_within_ttl(cached(1)).is_none());
        assert!(CliKind::ClaudeCode.list_within_ttl(cached(1)).is_none());
    }

    #[test]
    fn the_warning_says_whether_a_cached_list_stands_in() {
        assert!(failure_warning("agy-cli", "timed out", Some(600)).contains("10m ago"));
        assert!(failure_warning("agy-cli", "timed out", None).contains("unverified"));
    }
}
