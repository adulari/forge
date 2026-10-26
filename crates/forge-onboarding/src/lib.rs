//! Forge onboarding, independent of any screen.
//!
//! One engine serves both front ends (the terminal flow in `forge-tui`, the desktop app through
//! the daemon). It owns:
//!
//! * [`state`]: the `onboarding.json` progress file both surfaces share, and the required-step
//!   gate that decides whether onboarding reopens.
//! * [`detect`] and [`machine`]: what is already on the machine, found in parallel inside a
//!   1.5 s budget, as plain serializable data.
//! * [`step`]: the ordered step catalog, with which steps apply to which [`Surface`].
//! * [`flow`]: [`flow::start`] decides, from the state file and detection, whether to open,
//!   resume, migrate an existing install or show only what is new.
//! * [`action`] and [`job`]: every change a step can make as an [`Action`], run as a [`Jobs`]
//!   job that streams [`JobEvent`]s. All of it derives serde, so a daemon can expose the same
//!   types over HTTP.
//!
//! Secrets travel as [`Secret`]: masked in `Debug`, written as `***` by serde, scrubbed from job
//! output, and stored only through `forge_config`'s secret store.

pub mod action;
pub mod detect;
pub mod flow;
mod flows;
pub mod host;
pub mod job;
mod local_actions;
pub mod machine;
mod proc;
pub mod providers;
mod pty;
pub mod recommend;
mod reporting;
pub mod state;
pub mod step;
pub mod verify;

pub use action::{Action, Cmd, Exec, Exposure, Secret, SecretSlot};
pub use detect::{claude_reconnect_needed, detect, detect_with};
pub use flow::{
    apply_result, interactive_terminal, start, start_with, Session, Start, DEFERRED_HINT,
    MAX_DEFERRALS, OPT_OUT_ENV,
};
pub use forge_config::CLAUDE_OAUTH_TOKEN_SLOT;
pub use host::{Host, NoHost};
pub use job::{
    ActionResult, JobEvent, JobHandle, JobId, JobPoll, JobState, Jobs, PromptKind, Success,
};
pub use machine::{Login, Machine, ModelSource, SourceKind, SourceState};
pub use recommend::{recommended, Recommended, DEFAULT_ONLY_STEPS};
pub use state::{
    gate, load, save, source_counts, state_path, Gate, OnboardingState, StepStatus, Verification,
    VerifyStatus,
};
pub use step::{
    catalog, plan, satisfied, start_index, PlanMode, PlanStep, StepDef, StepId, Surface,
    FLOW_VERSION,
};

#[cfg(test)]
pub(crate) mod test_support {
    use std::ffi::{OsStr, OsString};
    use std::sync::{Mutex, MutexGuard};

    /// Serializes tests that change process-wide environment variables.
    pub fn env_lock() -> MutexGuard<'static, ()> {
        static LOCK: Mutex<()> = Mutex::new(());
        LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Sets (or removes) variables and puts every one back on drop.
    pub struct ScopedEnv(Vec<(String, Option<OsString>)>);

    impl ScopedEnv {
        pub fn set(vars: &[(&str, Option<&OsStr>)]) -> Self {
            let saved = vars
                .iter()
                .map(|(k, _)| (k.to_string(), std::env::var_os(k)))
                .collect();
            for (k, v) in vars {
                match v {
                    Some(v) => std::env::set_var(k, v),
                    None => std::env::remove_var(k),
                }
            }
            Self(saved)
        }
    }

    impl Drop for ScopedEnv {
        fn drop(&mut self) {
            for (k, v) in &self.0 {
                match v {
                    Some(v) => std::env::set_var(k, v),
                    None => std::env::remove_var(k),
                }
            }
        }
    }
}
