//! What Forge remembers about updates, in the data dir (never config): when it last asked, the
//! newest release it found, and the version the user chose to skip.
//!
//! "Remind me later" is deliberately not stored: it lasts for one process, so the next launch
//! offers again from the cached release without waiting on the network.

use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::update_check::is_newer;
use crate::update_feed::Release;

const FILE: &str = "update-state.json";
/// The network is asked at most this often, whatever the number of launches.
pub(crate) const CHECK_INTERVAL_SECS: u64 = 6 * 3600;

/// `FORGE_UPDATE_CHECK_INTERVAL_SECS` overrides the interval (tests); anything else is 6 h.
pub(crate) fn check_interval() -> u64 {
    std::env::var("FORGE_UPDATE_CHECK_INTERVAL_SECS")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(CHECK_INTERVAL_SECS)
}

pub(crate) fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// The version a client should be told about from what is already saved (no network), if newer
/// than this build and not skipped.
pub(crate) fn cached_offer() -> Option<String> {
    UpdateState::load()
        .offer(env!("CARGO_PKG_VERSION"))
        .map(|r| r.version.clone())
}

/// Updates are off by `[update] check = false` or `FORGE_NO_UPDATE_CHECK`.
pub(crate) fn disabled(config: &forge_config::Config) -> bool {
    !config.update.check || std::env::var_os("FORGE_NO_UPDATE_CHECK").is_some()
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateState {
    /// Unix seconds of the last network attempt (made or failed).
    #[serde(default)]
    pub last_check: u64,
    /// The newest release seen so far.
    #[serde(default)]
    pub latest: Option<Release>,
    /// A version the user chose never to be offered again. A newer one is offered anyway.
    #[serde(default)]
    pub skipped_version: Option<String>,
}

impl UpdateState {
    fn path() -> Option<std::path::PathBuf> {
        forge_config::data_dir().map(|d| d.join(FILE))
    }

    /// The saved state, or an empty one when there is none or it cannot be read.
    pub fn load() -> Self {
        Self::path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// Best-effort, atomic: a half-written file would read as "no state", so write aside and rename.
    pub fn save(&self) {
        let Some(path) = Self::path() else {
            return;
        };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let Ok(json) = serde_json::to_string_pretty(self) else {
            return;
        };
        let tmp = path.with_extension(format!("tmp{}", std::process::id()));
        if std::fs::write(&tmp, json).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    }

    /// Whether the network should be asked again.
    pub fn due(&self, now: u64, interval: u64) -> bool {
        now.saturating_sub(self.last_check) >= interval
    }

    /// Record an attempt before it is made, so a hang or an offline run does not retry every launch.
    pub fn record_attempt(&mut self, now: u64) {
        self.last_check = now;
    }

    /// Record what the network said. A failed lookup (`None`) keeps what was known.
    pub fn record_release(&mut self, release: Option<Release>) {
        if let Some(release) = release {
            self.latest = Some(release);
        }
    }

    /// The release to offer: newer than `current` and not skipped.
    pub fn offer(&self, current: &str) -> Option<&Release> {
        let latest = self.latest.as_ref()?;
        if !is_newer(&latest.version, current) {
            return None;
        }
        (self.skipped_version.as_deref() != Some(latest.version.as_str())).then_some(latest)
    }

    pub fn skip(&mut self, version: &str) {
        self.skipped_version = Some(version.trim_start_matches('v').to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(v: &str) -> Release {
        Release {
            version: v.into(),
            notes: String::new(),
            url: String::new(),
        }
    }

    #[test]
    fn the_network_is_asked_once_per_interval() {
        let mut s = UpdateState::default();
        let start = 10 * CHECK_INTERVAL_SECS;
        assert!(s.due(start, CHECK_INTERVAL_SECS), "never asked: due");
        s.record_attempt(start);
        assert!(!s.due(start + CHECK_INTERVAL_SECS - 1, CHECK_INTERVAL_SECS));
        assert!(s.due(start + CHECK_INTERVAL_SECS, CHECK_INTERVAL_SECS));
    }

    #[test]
    fn only_a_newer_release_is_offered() {
        let mut s = UpdateState::default();
        assert!(s.offer("2.23.6").is_none(), "nothing known yet");
        s.record_release(Some(release("2.23.6")));
        assert!(s.offer("2.23.6").is_none(), "same version");
        s.record_release(Some(release("2.24.0")));
        assert_eq!(s.offer("2.23.6").unwrap().version, "2.24.0");
        assert!(s.offer("2.24.0").is_none(), "already running it");
        assert!(
            s.offer("3.0.0").is_none(),
            "a dev build ahead of the release"
        );
    }

    #[test]
    fn skipping_hides_that_version_but_not_the_next_one() {
        let mut s = UpdateState::default();
        s.record_release(Some(release("2.24.0")));
        s.skip("v2.24.0");
        assert!(s.offer("2.23.6").is_none());
        s.record_release(Some(release("2.24.1")));
        assert_eq!(s.offer("2.23.6").unwrap().version, "2.24.1");
    }

    #[test]
    fn a_failed_lookup_keeps_the_known_release() {
        let mut s = UpdateState::default();
        s.record_release(Some(release("2.24.0")));
        s.record_release(None);
        assert_eq!(s.offer("2.23.6").unwrap().version, "2.24.0");
    }

    #[test]
    fn state_round_trips_through_json_and_tolerates_missing_fields() {
        let mut s = UpdateState::default();
        s.record_attempt(42);
        s.record_release(Some(release("2.24.0")));
        s.skip("2.23.9");
        let back: UpdateState = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back, s);
        let partial: UpdateState = serde_json::from_str("{\"last_check\": 7}").unwrap();
        assert_eq!(partial.last_check, 7);
        assert!(partial.latest.is_none() && partial.skipped_version.is_none());
    }
}
