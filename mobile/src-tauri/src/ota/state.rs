//! Which UI bundle is live, and the crash-safety state machine around it.
//!
//! Every launch that serves a downloaded bundle records it as `booting`; the web UI clears that
//! with `ota_mark_healthy` once the app shell has rendered. A launch that finds `booting` still
//! set knows the previous boot never got there.

use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::Path;

/// A bundle that has never been healthy is abandoned after one bad boot. One that has run fine
/// before gets a second chance, because quitting during startup looks the same as a crash.
const FAILED_BOOTS_PROVEN: u32 = 2;
const FAILED_BOOTS_UNPROVEN: u32 = 1;
const BLACKLIST_LIMIT: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BundleRef {
    pub id: String,
    pub created_at: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    /// The bundle served at this moment; `None` is the UI embedded in the binary.
    pub active: Option<BundleRef>,
    /// The last bundle that proved healthy: where a rollback goes (`None` is the embedded UI).
    pub last_healthy: Option<BundleRef>,
    pub pending: Option<BundleRef>,
    /// Set from launch until the UI reports healthy.
    pub booting: bool,
    pub failed_boots: u32,
    pub blacklist: Vec<String>,
    pub last_check: Option<u64>,
    pub last_error: Option<String>,
    pub last_rollback: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BootNote {
    /// A bundle was abandoned and the previous one (or the embedded UI) restored.
    RolledBack {
        bad: String,
    },
    Promoted {
        id: String,
    },
}

impl State {
    pub fn load(root: &Path) -> State {
        fs::read(root.join("state.json"))
            .ok()
            .and_then(|raw| serde_json::from_slice(&raw).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, root: &Path) -> io::Result<()> {
        fs::create_dir_all(root)?;
        let temp = root.join("state.json.tmp");
        fs::write(
            &temp,
            serde_json::to_vec_pretty(self).map_err(io::Error::other)?,
        )?;
        fs::rename(temp, root.join("state.json"))
    }

    /// Runs once per launch, before anything is served. Returns what happened, for logging.
    pub fn begin_boot(&mut self) -> Vec<BootNote> {
        let mut notes = Vec::new();
        if self.booting {
            if let Some(active) = self.active.clone() {
                self.failed_boots += 1;
                let proven = self.last_healthy.as_ref() == Some(&active);
                let allowed = if proven {
                    FAILED_BOOTS_PROVEN
                } else {
                    FAILED_BOOTS_UNPROVEN
                };
                if self.failed_boots >= allowed {
                    self.abandon(&active.id);
                    notes.push(BootNote::RolledBack { bad: active.id });
                }
            }
            self.booting = false;
        }
        if let Some(note) = self.promote_pending() {
            notes.push(note);
        }
        self.booting = self.active.is_some();
        notes
    }

    /// Makes the pending bundle the active one. Used at launch and by "Reload to update".
    pub fn promote_pending(&mut self) -> Option<BootNote> {
        let pending = self.pending.take()?;
        if self.is_blacklisted(&pending.id)
            || self
                .active
                .as_ref()
                .is_some_and(|active| active.created_at >= pending.created_at)
        {
            return None;
        }
        self.active = Some(pending.clone());
        self.failed_boots = 0;
        self.booting = true;
        Some(BootNote::Promoted { id: pending.id })
    }

    pub fn mark_healthy(&mut self) {
        if !self.booting {
            return;
        }
        self.booting = false;
        self.failed_boots = 0;
        if let Some(active) = &self.active {
            self.last_healthy = Some(active.clone());
        }
    }

    /// Gives up on the active bundle right now (the watchdog, or a bundle whose files are gone).
    pub fn abandon_active(&mut self) -> Option<String> {
        let active = self.active.clone()?;
        self.abandon(&active.id);
        self.booting = false;
        Some(active.id)
    }

    fn abandon(&mut self, id: &str) {
        self.blacklist_id(id);
        self.active = self.last_healthy.clone().filter(|healthy| healthy.id != id);
        if self
            .pending
            .as_ref()
            .is_some_and(|pending| pending.id == id)
        {
            self.pending = None;
        }
        self.failed_boots = 0;
        self.last_rollback = Some(id.to_string());
    }

    /// Drops every downloaded bundle and returns to the embedded UI. What was live and what was
    /// waiting stay blacklisted, so the next check does not simply fetch them again.
    pub fn reset_to_builtin(&mut self) {
        for bundle in [self.active.take(), self.pending.take()]
            .into_iter()
            .flatten()
        {
            self.blacklist_id(&bundle.id);
        }
        self.last_healthy = None;
        self.booting = false;
        self.failed_boots = 0;
    }

    pub fn stage(&mut self, bundle: BundleRef) {
        self.pending = Some(bundle);
    }

    pub fn is_blacklisted(&self, id: &str) -> bool {
        self.blacklist.iter().any(|entry| entry == id)
    }

    fn blacklist_id(&mut self, id: &str) {
        if !self.is_blacklisted(id) {
            self.blacklist.push(id.to_string());
        }
        if self.blacklist.len() > BLACKLIST_LIMIT {
            self.blacklist.remove(0);
        }
    }

    /// The newest `created_at` this install must not go below.
    pub fn floor(&self, embedded: u64) -> u64 {
        [
            Some(embedded),
            self.active.as_ref().map(|bundle| bundle.created_at),
            self.pending.as_ref().map(|bundle| bundle.created_at),
        ]
        .into_iter()
        .flatten()
        .max()
        .unwrap_or(embedded)
    }

    /// Ids whose files must be kept on disk.
    pub fn referenced(&self) -> Vec<&str> {
        [&self.active, &self.pending, &self.last_healthy]
            .into_iter()
            .flatten()
            .map(|bundle| bundle.id.as_str())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bundle(id: &str, created_at: u64) -> BundleRef {
        BundleRef {
            id: id.to_string(),
            created_at,
        }
    }

    fn healthy_active(id: &str, created_at: u64) -> State {
        let mut state = State::default();
        state.stage(bundle(id, created_at));
        state.begin_boot();
        state.mark_healthy();
        state
    }

    #[test]
    fn embedded_ui_has_nothing_to_prove() {
        let mut state = State::default();
        assert!(state.begin_boot().is_empty());
        assert!(!state.booting);
        assert_eq!(state.active, None);
    }

    #[test]
    fn pending_bundle_is_promoted_on_next_launch_and_must_prove_itself() {
        let mut state = State::default();
        state.stage(bundle("aaaaaaa1", 10));
        let notes = state.begin_boot();
        assert_eq!(
            notes,
            vec![BootNote::Promoted {
                id: "aaaaaaa1".into()
            }]
        );
        assert_eq!(state.active, Some(bundle("aaaaaaa1", 10)));
        assert!(state.booting);
        assert_eq!(state.pending, None);
        state.mark_healthy();
        assert!(!state.booting);
        assert_eq!(state.last_healthy, Some(bundle("aaaaaaa1", 10)));
    }

    #[test]
    fn unproven_bundle_that_never_reports_healthy_is_rolled_back_and_blacklisted() {
        let mut state = State::default();
        state.stage(bundle("badbad01", 10));
        state.begin_boot();
        let notes = state.begin_boot();
        assert_eq!(
            notes,
            vec![BootNote::RolledBack {
                bad: "badbad01".into()
            }]
        );
        assert_eq!(state.active, None);
        assert!(state.is_blacklisted("badbad01"));
        assert!(!state.booting);
        assert_eq!(state.last_rollback.as_deref(), Some("badbad01"));
    }

    #[test]
    fn broken_update_rolls_back_to_the_previous_healthy_bundle() {
        let mut state = healthy_active("good0001", 10);
        state.stage(bundle("bad00002", 20));
        state.begin_boot();
        assert_eq!(state.active, Some(bundle("bad00002", 20)));
        state.begin_boot();
        assert_eq!(state.active, Some(bundle("good0001", 10)));
        assert!(state.is_blacklisted("bad00002"));
        assert!(state.booting, "the restored bundle is still watched");
    }

    #[test]
    fn proven_bundle_survives_one_failed_boot_but_not_two() {
        let mut state = healthy_active("good0001", 10);
        state.begin_boot();
        state.begin_boot();
        assert_eq!(
            state.active,
            Some(bundle("good0001", 10)),
            "one failure is tolerated"
        );
        state.begin_boot();
        assert_eq!(state.active, None);
        assert!(state.is_blacklisted("good0001"));
    }

    #[test]
    fn a_healthy_boot_clears_the_failure_count() {
        let mut state = healthy_active("good0001", 10);
        state.begin_boot();
        state.begin_boot();
        state.mark_healthy();
        state.begin_boot();
        state.begin_boot();
        assert_eq!(state.active, Some(bundle("good0001", 10)));
    }

    #[test]
    fn blacklisted_or_older_pending_bundles_are_not_promoted() {
        let mut state = healthy_active("good0001", 10);
        state.blacklist.push("black001".into());
        state.stage(bundle("black001", 99));
        assert!(state.begin_boot().is_empty());
        assert_eq!(state.active, Some(bundle("good0001", 10)));
        state.mark_healthy();
        state.stage(bundle("older001", 5));
        assert!(state.begin_boot().is_empty());
        assert_eq!(state.active, Some(bundle("good0001", 10)));
    }

    #[test]
    fn floor_is_the_newest_of_embedded_active_and_pending() {
        let mut state = healthy_active("good0001", 10);
        assert_eq!(state.floor(0), 10);
        assert_eq!(state.floor(50), 50);
        state.stage(bundle("next0001", 70));
        assert_eq!(state.floor(50), 70);
    }

    #[test]
    fn abandoning_the_active_bundle_restores_the_embedded_ui() {
        let mut state = healthy_active("good0001", 10);
        assert_eq!(state.abandon_active().as_deref(), Some("good0001"));
        assert_eq!(state.active, None);
        assert!(state.is_blacklisted("good0001"));
    }

    #[test]
    fn reset_returns_to_embedded_and_keeps_old_bundles_out() {
        let mut state = healthy_active("good0001", 10);
        state.stage(bundle("next0001", 20));
        state.reset_to_builtin();
        assert_eq!(state.active, None);
        assert_eq!(state.pending, None);
        assert_eq!(state.last_healthy, None);
        assert!(state.is_blacklisted("good0001") && state.is_blacklisted("next0001"));
    }

    #[test]
    fn state_round_trips_and_survives_corruption() {
        let dir = std::env::temp_dir().join(format!("forge-ota-state-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let state = healthy_active("good0001", 10);
        state.save(&dir).unwrap();
        assert_eq!(State::load(&dir), state);
        fs::write(dir.join("state.json"), b"{not json").unwrap();
        assert_eq!(State::load(&dir), State::default());
        fs::remove_dir_all(dir).unwrap();
    }
}
