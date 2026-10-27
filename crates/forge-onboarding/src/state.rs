//! The onboarding state file, shared by every surface: whichever opens first records its
//! progress here and the other resumes from it.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::machine::{Machine, SourceState};
use crate::step::{StepId, Surface};

pub const SCHEMA: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    #[default]
    Pending,
    Done,
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct StepRecord {
    pub status: StepStatus,
    /// Unix seconds of the last change.
    pub at: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerifyStatus {
    /// The source answered a live check.
    Verified,
    /// Present, but the live check proved nothing either way (no listing endpoint, offline).
    Unchecked,
    /// The source refused its credential.
    Rejected,
}

/// The outcome of the last live check of one model source. Never carries a credential.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Verification {
    pub status: VerifyStatus,
    pub at: u64,
    pub detail: Option<String>,
}

/// Per-surface "required steps pass" flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct RequiredComplete {
    pub tui: bool,
    pub desktop: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OnboardingState {
    pub schema: u32,
    pub flow_version: u32,
    /// Times the person chose "skip for now" while everything required already worked.
    pub deferred_count: u32,
    /// Keyed by [`StepId::as_str`] so a file written by a newer Forge still loads.
    pub steps: BTreeMap<String, StepRecord>,
    /// Keyed by source id (`claude-cli`, `groq`, `ollama`).
    pub verified: BTreeMap<String, Verification>,
    pub required_complete: RequiredComplete,
    pub completed_at: Option<u64>,
    pub updated_at: u64,
    pub last_surface: Option<Surface>,
}

impl Default for OnboardingState {
    fn default() -> Self {
        Self {
            schema: SCHEMA,
            flow_version: 0,
            deferred_count: 0,
            steps: BTreeMap::new(),
            verified: BTreeMap::new(),
            required_complete: RequiredComplete::default(),
            completed_at: None,
            updated_at: 0,
            last_surface: None,
        }
    }
}

pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

impl OnboardingState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn status(&self, step: StepId) -> StepStatus {
        self.steps
            .get(step.as_str())
            .map(|r| r.status)
            .unwrap_or_default()
    }

    fn set(&mut self, step: StepId, status: StepStatus) {
        let at = now();
        self.steps.insert(
            step.as_str().to_string(),
            StepRecord {
                status,
                at: Some(at),
            },
        );
        self.updated_at = at;
    }

    pub fn mark_done(&mut self, step: StepId) {
        self.set(step, StepStatus::Done);
    }

    pub fn mark_skipped(&mut self, step: StepId) {
        self.set(step, StepStatus::Skipped);
    }

    pub fn reset(&mut self, step: StepId) {
        self.set(step, StepStatus::Pending);
    }

    pub fn record_verification(
        &mut self,
        source: &str,
        status: VerifyStatus,
        detail: Option<String>,
    ) {
        let at = now();
        self.verified
            .insert(source.to_string(), Verification { status, at, detail });
        self.updated_at = at;
    }

    pub fn verification(&self, source: &str) -> Option<&Verification> {
        self.verified.get(source)
    }

    pub fn required_complete(&self, surface: Surface) -> bool {
        match surface {
            Surface::Tui => self.required_complete.tui,
            Surface::Desktop => self.required_complete.desktop,
        }
    }

    /// Recompute the stored "required steps pass" flag for `surface` from `machine`.
    pub fn refresh_required(&mut self, machine: &Machine, surface: Surface) -> Gate {
        let gate = gate(self, machine, surface);
        match surface {
            Surface::Tui => self.required_complete.tui = gate.complete,
            Surface::Desktop => self.required_complete.desktop = gate.complete,
        }
        if gate.complete && self.completed_at.is_none() {
            self.completed_at = Some(now());
        }
        self.last_surface = Some(surface);
        gate
    }
}

/// What still blocks completion on one surface.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Gate {
    pub complete: bool,
    /// Required steps that do not pass yet.
    pub missing: Vec<StepId>,
}

/// Whether a ready source counts toward the model requirement: it only fails when a live check
/// found its credential refused. An unchecked source is trusted until a check says otherwise, so
/// an install that already works is never blocked.
pub fn source_counts(state: &OnboardingState, source: &str) -> bool {
    state.verification(source).map(|v| v.status) != Some(VerifyStatus::Rejected)
}

/// The required-step gate. The model requirement is judged on what detection found plus what
/// the live checks said, not on whether the user pressed Continue, so a step cannot be skipped
/// into a state where Forge cannot run a turn.
pub fn gate(state: &OnboardingState, machine: &Machine, surface: Surface) -> Gate {
    let mut missing = Vec::new();
    for def in crate::step::catalog() {
        if !def.required(surface) || !def.applies(machine, surface) {
            continue;
        }
        let pass = match def.id {
            StepId::Models => machine
                .model_sources()
                .iter()
                .any(|s| s.state == SourceState::Ready && source_counts(state, &s.id)),
            StepId::Service => machine.service.installed && machine.service.running,
            StepId::InstallCli => machine.forge.installed,
            _ => state.status(def.id) == StepStatus::Done,
        };
        if !pass {
            missing.push(def.id);
        }
    }
    Gate {
        complete: missing.is_empty(),
        missing,
    }
}

/// Where the state file lives. An isolated run (`FORGE_DB` set: tests, benchmarks) keeps it
/// beside its scratch store, so it never reads or writes the real installation's progress.
pub fn state_path() -> Option<PathBuf> {
    if let Some(db) = std::env::var_os("FORGE_DB").filter(|v| !v.is_empty()) {
        let mut beside = db;
        beside.push(".onboarding.json");
        return Some(PathBuf::from(beside));
    }
    forge_config::data_dir().map(|d| d.join("onboarding.json"))
}

/// Read the state file. A missing, unreadable or unparsable file is `None` (treated as a fresh
/// install); the damaged file is left in place for whoever wants to look at it.
pub fn load() -> Option<OnboardingState> {
    load_from(&state_path()?)
}

pub fn load_from(path: &std::path::Path) -> Option<OnboardingState> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

pub fn save(state: &OnboardingState) -> std::io::Result<()> {
    let path = state_path().ok_or_else(|| std::io::Error::other("no data directory"))?;
    save_to(&path, state)
}

pub fn save_to(path: &std::path::Path, state: &OnboardingState) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(state).map_err(std::io::Error::other)?;
    forge_config::write_atomic(path, json)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::{KeyFacts, ServiceFacts};

    fn machine_with_key() -> Machine {
        Machine {
            keys: vec![KeyFacts {
                id: "groq".into(),
                name: "Groq".into(),
                configured: true,
                featured: true,
                ..KeyFacts::default()
            }],
            forge: crate::machine::ForgeInstall {
                installed: true,
                ..Default::default()
            },
            ..Machine::default()
        }
    }

    #[test]
    fn steps_move_between_pending_done_and_skipped() {
        let mut state = OnboardingState::new();
        assert_eq!(state.status(StepId::WebSearch), StepStatus::Pending);
        state.mark_done(StepId::WebSearch);
        assert_eq!(state.status(StepId::WebSearch), StepStatus::Done);
        state.mark_skipped(StepId::WebSearch);
        assert_eq!(state.status(StepId::WebSearch), StepStatus::Skipped);
        state.reset(StepId::WebSearch);
        assert_eq!(state.status(StepId::WebSearch), StepStatus::Pending);
        assert!(state.steps["web_search"].at.is_some());
    }

    #[test]
    fn a_detected_source_counts_until_a_check_refuses_it() {
        let machine = machine_with_key();
        let mut state = OnboardingState::new();
        assert!(gate(&state, &machine, Surface::Tui).complete);
        assert!(!gate(&state, &Machine::default(), Surface::Tui).complete);
        state.record_verification("groq", VerifyStatus::Verified, None);
        assert!(gate(&state, &machine, Surface::Tui).complete);
    }

    #[test]
    fn an_unchecked_source_counts_and_a_rejected_one_does_not() {
        let machine = machine_with_key();
        let mut state = OnboardingState::new();
        state.record_verification("groq", VerifyStatus::Unchecked, Some("offline".into()));
        assert!(gate(&state, &machine, Surface::Tui).complete);
        state.record_verification("groq", VerifyStatus::Rejected, Some("401".into()));
        assert!(!gate(&state, &machine, Surface::Tui).complete);
    }

    #[test]
    fn skipping_the_model_step_does_not_pass_the_gate() {
        let mut state = OnboardingState::new();
        state.mark_skipped(StepId::Models);
        let machine = Machine::default();
        assert!(!state.refresh_required(&machine, Surface::Tui).complete);
        assert!(!state.required_complete(Surface::Tui));
    }

    #[test]
    fn the_desktop_also_needs_the_service_running() {
        let mut machine = machine_with_key();
        let mut state = OnboardingState::new();
        state.record_verification("groq", VerifyStatus::Verified, None);
        assert!(gate(&state, &machine, Surface::Tui).complete);
        let desktop = gate(&state, &machine, Surface::Desktop);
        assert_eq!(desktop.missing, [StepId::Service]);
        machine.service = ServiceFacts {
            supported: true,
            installed: true,
            running: true,
        };
        assert!(gate(&state, &machine, Surface::Desktop).complete);
    }

    #[test]
    fn the_desktop_gate_asks_for_the_cli_first() {
        let mut machine = machine_with_key();
        machine.forge.installed = false;
        let mut state = OnboardingState::new();
        state.record_verification("groq", VerifyStatus::Verified, None);
        assert_eq!(
            gate(&state, &machine, Surface::Desktop).missing,
            [StepId::InstallCli, StepId::Service]
        );
    }

    #[test]
    fn the_state_file_round_trips_and_survives_unknown_steps() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("onboarding.json");
        let mut state = OnboardingState::new();
        state.mark_done(StepId::Models);
        state.record_verification("groq", VerifyStatus::Verified, None);
        save_to(&path, &state).unwrap();
        assert_eq!(load_from(&path).unwrap(), state);

        let future = r#"{"schema":9,"steps":{"quantum_step":{"status":"done"},"models":{"status":"done"}},"surprise":1}"#;
        std::fs::write(&path, future).unwrap();
        let loaded = load_from(&path).unwrap();
        assert_eq!(loaded.status(StepId::Models), StepStatus::Done);
    }

    #[test]
    fn a_missing_or_corrupt_file_reads_as_a_fresh_install_and_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("onboarding.json");
        assert!(load_from(&path).is_none());
        std::fs::write(&path, "{ not json").unwrap();
        assert!(load_from(&path).is_none());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ not json");
    }

    #[test]
    fn an_isolated_run_keeps_its_state_beside_its_scratch_store() {
        let _guard = crate::test_support::env_lock();
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("forge.db");
        std::env::set_var("FORGE_DB", &db);
        let path = state_path().unwrap();
        std::env::remove_var("FORGE_DB");
        assert_eq!(path, dir.path().join("forge.db.onboarding.json"));
    }
}
