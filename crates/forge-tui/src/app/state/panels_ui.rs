//! UI state owned by the panels, welcome state and overlays.

use std::cell::{Cell, RefCell};
use std::time::{Duration, Instant};

use crate::app::App;

/// How often the welcome re-checks whether `/init` has written project guidance.
const INIT_PROBE_EVERY: Duration = Duration::from_secs(2);

/// State only the panel surfaces read or write; the panels work package adds fields here and
/// nowhere else.
#[derive(Debug, Clone, Default)]
pub(crate) struct PanelsUi {
    /// Set once the project's initialisation state is known; `None` before that.
    pub project_initialized: Option<bool>,
    /// Prior sessions in this workspace as `(title, age)`, newest first. The welcome shows two.
    pub recent_sessions: Vec<(String, String)>,
    /// The welcome example row holding keyboard focus, if any.
    pub try_focus: Option<usize>,
    /// The last on-disk initialisation probe, so the welcome does not stat files every frame.
    init_probe: Cell<Option<(Instant, bool)>>,
    /// Observed start and finish times for tasks and agents (see `render::panels::common`).
    pub(crate) clock: RefCell<crate::app::render::panels::Clock>,
}

impl PanelsUi {
    /// Whether the welcome should still nudge the user towards `/init`: the launcher found no
    /// project guidance and none has appeared since (`/init` writes it while the welcome is up).
    pub(crate) fn onboarding_pending(&self, workspace: Option<&std::path::Path>) -> bool {
        if self.project_initialized != Some(false) {
            return false;
        }
        let Some(workspace) = workspace else {
            return true;
        };
        let now = Instant::now();
        if let Some((at, initialized)) = self.init_probe.get() {
            if now.duration_since(at) < INIT_PROBE_EVERY {
                return !initialized;
            }
        }
        let initialized = forge_config::project_initialization(workspace).initialized;
        self.init_probe.set(Some((now, initialized)));
        !initialized
    }
}

impl App {
    /// Record whether the project has Forge guidance. Fullscreen shows the nudge as a line of the
    /// welcome (state, cleared once `/init` succeeds); inline has no welcome state, so an
    /// uninitialised project gets the same one-line warning in scrollback.
    pub fn set_project_initialized(&mut self, initialized: bool) {
        self.panels_ui.project_initialized = Some(initialized);
        if !initialized && !self.fullscreen {
            self.apply(crate::PresenterEvent::Warning(
                "This project isn't set up for Forge yet. Run /init to set it up.".to_string(),
            ));
        }
    }

    /// Prior sessions to offer on the welcome, newest first, as `(title, age)`.
    pub fn set_recent_sessions(&mut self, sessions: Vec<(String, String)>) {
        self.panels_ui.recent_sessions = sessions;
    }
}

impl App {
    /// Welcome "Try" focus. `Down` from an empty composer focuses the examples, `Up`/`Down` move,
    /// `Enter` copies the focused example into the composer (never sends), `Esc` leaves. Any other
    /// key drops the focus and is left to the normal handler. Returns whether the key was consumed.
    pub fn welcome_key(&mut self, key: &crate::KeyKind) -> bool {
        use crate::app::render::welcome::{transcript_is_empty, TRY_EXAMPLES};
        use crate::KeyKind;
        if !self.fullscreen || !transcript_is_empty(self, self.last_width.get().max(1)) {
            self.panels_ui.try_focus = None;
            return false;
        }
        let last = TRY_EXAMPLES.len() - 1;
        match (self.panels_ui.try_focus, key) {
            (None, KeyKind::Down) if self.input.is_empty() => {
                self.panels_ui.try_focus = Some(0);
                true
            }
            (None, _) => false,
            (Some(i), KeyKind::Down) => {
                self.panels_ui.try_focus = Some((i + 1).min(last));
                true
            }
            (Some(0), KeyKind::Up) => {
                self.panels_ui.try_focus = None;
                true
            }
            (Some(i), KeyKind::Up) => {
                self.panels_ui.try_focus = Some(i - 1);
                true
            }
            (Some(i), KeyKind::Enter) => {
                self.input = TRY_EXAMPLES[i].to_string();
                self.input_cursor = self.input.len();
                self.panels_ui.try_focus = None;
                true
            }
            (Some(_), KeyKind::Esc) => {
                self.panels_ui.try_focus = None;
                true
            }
            (Some(_), _) => {
                self.panels_ui.try_focus = None;
                false
            }
        }
    }
}
