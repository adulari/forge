//! Deciding whether onboarding opens, and how. The trigger is state-driven: a state file whose
//! required steps pass keeps every later launch silent and free (no detection at all); anything
//! else detects the machine and either migrates an install that already works, resumes, or shows
//! what a newer flow added.

use std::sync::Arc;

use crate::detect::detect;
use crate::host::Host;
use crate::job::ActionResult;
use crate::machine::Machine;
use crate::state::{self, Gate, OnboardingState};
use crate::step::{plan, start_index, PlanMode, PlanStep, Surface, FLOW_VERSION};

/// Environment variable that stops onboarding from opening by itself (CI, scripted installs).
/// `forge setup` still opens it.
pub const OPT_OUT_ENV: &str = "FORGE_NO_SETUP";

/// A run in progress: everything a front end renders and mutates.
#[derive(Debug, Clone)]
pub struct Session {
    pub surface: Surface,
    pub mode: PlanMode,
    pub state: OnboardingState,
    pub machine: Machine,
    pub plan: Vec<PlanStep>,
}

#[derive(Debug, Clone)]
pub enum Start {
    /// Nothing to do: the flow is current and required steps pass.
    Quiet,
    /// Not opened, but worth one line: setup was deferred twice, so Forge stops asking.
    Hint(String),
    Run(Box<Session>),
}

/// After this many "skip for now" answers on a machine that already works, Forge stops opening
/// onboarding by itself and only mentions `forge setup`.
pub const MAX_DEFERRALS: u32 = 2;

pub const DEFERRED_HINT: &str = "Run `forge setup` to finish setting up Forge.";

/// Whether this process is attached to a person at a terminal that can draw the TUI.
pub fn interactive_terminal() -> bool {
    use std::io::IsTerminal;
    std::io::stdin().is_terminal()
        && std::io::stdout().is_terminal()
        && std::env::var("TERM").map_or(true, |t| t != "dumb")
        && !ci_environment()
}

fn ci_environment() -> bool {
    [
        "CI",
        "GITHUB_ACTIONS",
        "BUILDKITE",
        "GITLAB_CI",
        "FORGE_HEADLESS",
    ]
    .iter()
    .any(|v| std::env::var_os(v).is_some_and(|s| !s.is_empty() && s != "0" && s != "false"))
}

/// Decide what a launch does. `force` is `forge setup`: always the full flow.
pub fn start(host: Arc<dyn Host>, surface: Surface, force: bool) -> Start {
    start_with(host, surface, force, FLOW_VERSION, interactive_terminal())
}

/// [`start`] with the flow version and interactivity explicit, for tests and for hosts that
/// know better than the process's stdio (the desktop app is interactive without a tty).
///
/// Everyone gets onboarding once per flow version: a missing state file, or one written under an
/// older flow, opens it. Detection pre-fills what the machine already has. Required steps that
/// do not pass reopen it every launch; when they already pass, "skip for now" defers it at most
/// [`MAX_DEFERRALS`] times.
pub fn start_with(
    host: Arc<dyn Host>,
    surface: Surface,
    force: bool,
    current_flow: u32,
    interactive: bool,
) -> Start {
    if !interactive && !force {
        return Start::Quiet;
    }
    let loaded = state::load();
    if !force {
        if std::env::var_os(OPT_OUT_ENV).is_some_and(|v| !v.is_empty()) {
            return Start::Quiet;
        }
        if loaded
            .as_ref()
            .is_some_and(|s| s.required_complete(surface) && s.flow_version >= current_flow)
        {
            return Start::Quiet;
        }
    }

    let machine = detect(host);
    let had_state = loaded.is_some();
    let mut state = loaded.unwrap_or_default();
    let gate = state.refresh_required(&machine, surface);

    let mode = if force {
        PlanMode::Full
    } else if state.flow_version >= current_flow {
        // The flow is current, so only a missing required step brings it back.
        if gate.complete {
            let _ = state::save(&state);
            return Start::Quiet;
        }
        PlanMode::Resume
    } else if gate.complete && state.deferred_count >= MAX_DEFERRALS {
        let _ = state::save(&state);
        return Start::Hint(DEFERRED_HINT.to_string());
    } else if had_state
        && state.flow_version > 0
        && plan(&state, &machine, surface, PlanMode::WhatsNew).is_empty()
    {
        // A newer flow that added nothing this surface shows: record it and stay quiet.
        state.flow_version = current_flow;
        let _ = state::save(&state);
        return Start::Quiet;
    } else if state.flow_version > 0 && gate.complete {
        PlanMode::WhatsNew
    } else {
        PlanMode::Resume
    };
    let _ = state::save(&state);
    let steps = plan(&state, &machine, surface, mode);
    Start::Run(Box::new(Session {
        surface,
        mode,
        state,
        machine,
        plan: steps,
    }))
}

impl Session {
    /// Where the run opens, as an index into `plan`.
    pub fn start_index(&self) -> usize {
        start_index(&self.plan, self.mode)
    }

    pub fn gate(&self) -> Gate {
        state::gate(&self.state, &self.machine, self.surface)
    }

    /// Fold a finished job's structured result into the state.
    pub fn apply(&mut self, result: &ActionResult) {
        apply_result(&mut self.state, result);
    }

    /// Read the machine again after something changed, and rebuild the plan around it.
    pub fn redetect(&mut self, host: Arc<dyn Host>) {
        self.machine = detect(host);
        self.state.refresh_required(&self.machine, self.surface);
        self.plan = plan(&self.state, &self.machine, self.surface, self.mode);
    }

    pub fn save(&self) {
        let _ = state::save(&self.state);
    }

    /// "Skip for now": leave the flow open. When everything required already passes this counts
    /// toward the deferral cap; otherwise onboarding simply reopens.
    pub fn postpone(&mut self) {
        if self.gate().complete {
            self.state.deferred_count += 1;
        }
        self.state.refresh_required(&self.machine, self.surface);
    }

    /// [`Session::postpone`], written to disk.
    pub fn defer(&mut self) {
        self.postpone();
        self.save();
    }

    /// The run is over: the flow is current, the required flag is fresh, the file is written.
    pub fn complete(&mut self) {
        self.state.flow_version = self.state.flow_version.max(FLOW_VERSION);
        self.state.deferred_count = 0;
        self.state.refresh_required(&self.machine, self.surface);
    }

    /// [`Session::complete`], written to disk.
    pub fn finish(&mut self) {
        self.complete();
        self.save();
    }
}

pub fn apply_result(state: &mut OnboardingState, result: &ActionResult) {
    if let ActionResult::Verified {
        source,
        status,
        detail,
    } = result
    {
        state.record_verification(source, *status, detail.clone());
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::host::NoHost;
    use crate::state::{StepStatus, VerifyStatus};
    use crate::step::StepId;
    use crate::test_support::{env_lock, ScopedEnv};
    use std::ffi::OsStr;

    struct Env {
        _guard: std::sync::MutexGuard<'static, ()>,
        _env: ScopedEnv,
        _home: tempfile::TempDir,
    }

    fn env() -> Env {
        let guard = env_lock();
        let home = tempfile::tempdir().unwrap();
        let bin = home.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let config = home.path().join("config");
        let data = home.path().join("data");
        let vars: Vec<(&str, Option<&OsStr>)> = vec![
            ("HOME", Some(home.path().as_os_str())),
            ("XDG_CONFIG_HOME", Some(config.as_os_str())),
            ("XDG_DATA_HOME", Some(data.as_os_str())),
            ("PATH", Some(bin.as_os_str())),
            ("FORGE_DB", None),
            (OPT_OUT_ENV, None),
            ("CLAUDE_CONFIG_DIR", None),
            ("CODEX_HOME", None),
            ("ANTHROPIC_API_KEY", None),
            ("CLAUDE_CODE_OAUTH_TOKEN", None),
            ("OPENAI_API_KEY", None),
            ("GROQ_API_KEY", None),
        ];
        let scoped = ScopedEnv::set(&vars);
        Env {
            _guard: guard,
            _env: scoped,
            _home: home,
        }
    }

    fn host() -> Arc<dyn Host> {
        Arc::new(NoHost)
    }

    fn write_config() {
        let dir = forge_config::config_dir().unwrap();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("config.toml"), "").unwrap();
    }

    fn open(surface: Surface, force: bool) -> Start {
        start_with(host(), surface, force, FLOW_VERSION, true)
    }

    fn run(start: Start) -> Session {
        match start {
            Start::Run(s) => *s,
            other => panic!("expected onboarding to open, got {other:?}"),
        }
    }

    fn finish_with_groq(session: &mut Session) {
        session.apply(&ActionResult::Verified {
            source: "groq".into(),
            status: VerifyStatus::Verified,
            detail: None,
        });
        session.finish();
    }

    #[test]
    fn a_fresh_machine_opens_onboarding_on_the_model_step() {
        let _e = env();
        let session = run(open(Surface::Tui, false));
        assert_eq!(session.mode, PlanMode::Resume);
        assert_eq!(session.plan[session.start_index()].id, StepId::Models);
        assert!(
            state::load().is_some(),
            "the first run writes the state file"
        );
    }

    #[test]
    fn an_existing_user_gets_onboarding_once_with_what_they_have_pre_marked() {
        let _e = env();
        write_config();
        std::env::set_var("GROQ_API_KEY", "gsk-test");
        let mut session = run(open(Surface::Tui, false));
        let models = session
            .plan
            .iter()
            .find(|s| s.id == StepId::Models)
            .unwrap();
        assert!(
            models.satisfied,
            "their working model shows as already set up"
        );
        assert_ne!(
            session.plan[session.start_index()].id,
            StepId::Models,
            "the run lands on the first step that still has something for them"
        );
        assert!(
            session.gate().complete,
            "they are never blocked from working"
        );

        session.finish();
        assert!(matches!(open(Surface::Tui, false), Start::Quiet), "once");
    }

    #[test]
    fn skipping_for_now_reopens_a_machine_with_no_model() {
        let _e = env();
        let mut session = run(open(Surface::Tui, false));
        session.state.mark_skipped(StepId::Models);
        session.defer();
        assert_eq!(
            session.state.deferred_count, 0,
            "nothing worked, so no cap applies"
        );
        for _ in 0..3 {
            assert!(matches!(open(Surface::Tui, false), Start::Run(_)));
        }
    }

    #[test]
    fn a_working_machine_can_defer_twice_then_only_gets_a_hint() {
        let _e = env();
        std::env::set_var("GROQ_API_KEY", "gsk-test");
        let mut first = run(open(Surface::Tui, false));
        first.defer();
        let mut second = run(open(Surface::Tui, false));
        second.defer();
        assert_eq!(second.state.deferred_count, 2);
        match open(Surface::Tui, false) {
            Start::Hint(text) => assert!(text.contains("forge setup")),
            other => panic!("expected the one-line hint, got {other:?}"),
        }
        assert!(
            matches!(open(Surface::Tui, true), Start::Run(_)),
            "forge setup still opens it"
        );
    }

    #[test]
    fn finishing_clears_the_deferrals() {
        let _e = env();
        std::env::set_var("GROQ_API_KEY", "gsk-test");
        let mut session = run(open(Surface::Tui, false));
        session.defer();
        session.finish();
        assert_eq!(session.state.deferred_count, 0);
    }

    #[test]
    fn non_interactive_launches_never_open_it() {
        let _e = env();
        assert!(matches!(
            start_with(host(), Surface::Tui, false, FLOW_VERSION, false),
            Start::Quiet
        ));
        assert!(state::load().is_none(), "and they leave no trace");
        std::env::set_var("CI", "true");
        assert!(!interactive_terminal(), "CI is never interactive");
        std::env::remove_var("CI");
    }

    #[test]
    fn once_required_steps_pass_and_the_flow_is_current_later_launches_are_quiet() {
        let _e = env();
        std::env::set_var("GROQ_API_KEY", "gsk-test");
        let mut session = run(open(Surface::Tui, false));
        finish_with_groq(&mut session);
        std::env::remove_var("GROQ_API_KEY");
        assert!(
            matches!(open(Surface::Tui, false), Start::Quiet),
            "the fast path trusts the state file and never looks at the machine"
        );
    }

    #[test]
    fn forge_setup_always_opens_the_full_flow() {
        let _e = env();
        std::env::set_var("GROQ_API_KEY", "gsk-test");
        let mut session = run(open(Surface::Tui, false));
        finish_with_groq(&mut session);
        let full = run(open(Surface::Tui, true));
        assert_eq!(full.mode, PlanMode::Full);
        assert_eq!(full.plan[full.start_index()].id, StepId::Welcome);
    }

    #[test]
    fn the_opt_out_variable_keeps_it_closed_but_not_forge_setup() {
        let _e = env();
        std::env::set_var(OPT_OUT_ENV, "1");
        assert!(matches!(open(Surface::Tui, false), Start::Quiet));
        assert!(matches!(open(Surface::Tui, true), Start::Run(_)));
    }

    #[test]
    fn a_state_from_before_the_first_flow_is_treated_like_a_first_run() {
        let _e = env();
        std::env::set_var("GROQ_API_KEY", "gsk-test");
        let mut session = run(open(Surface::Tui, false));
        finish_with_groq(&mut session);
        session.state.flow_version = 0;
        session.save();
        assert_eq!(run(open(Surface::Tui, false)).mode, PlanMode::Resume);
    }

    #[test]
    fn a_flow_bump_that_adds_no_steps_is_recorded_silently() {
        let _e = env();
        std::env::set_var("GROQ_API_KEY", "gsk-test");
        let mut session = run(open(Surface::Tui, false));
        finish_with_groq(&mut session);
        assert!(matches!(
            start_with(host(), Surface::Tui, false, 7, true),
            Start::Quiet
        ));
        assert_eq!(state::load().unwrap().flow_version, 7);
    }

    #[test]
    fn the_desktop_and_the_terminal_share_one_state_file() {
        let _e = env();
        std::env::set_var("GROQ_API_KEY", "gsk-test");
        let mut session = run(open(Surface::Tui, false));
        session.state.mark_done(StepId::Behavior);
        finish_with_groq(&mut session);
        let desktop = run(open(Surface::Desktop, false));
        assert_eq!(desktop.state.status(StepId::Behavior), StepStatus::Done);
        assert!(desktop.state.verification("groq").is_some());
        assert!(!desktop.state.required_complete(Surface::Desktop));
    }
}
