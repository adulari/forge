//! UI state owned by the panels, welcome state and overlays.

use crate::app::App;

/// State only the panel surfaces read or write. Starts with the one fact the welcome state needs;
/// the panels work package adds fields here and nowhere else.
#[derive(Debug, Clone, Default)]
pub(crate) struct PanelsUi {
    /// Set once the project's initialisation state is known; `None` before that.
    pub project_initialized: Option<bool>,
}

impl App {
    /// Record whether the project has Forge guidance. An uninitialised project gets the same
    /// one-line warning the launcher used to push directly.
    pub fn set_project_initialized(&mut self, initialized: bool) {
        self.panels_ui.project_initialized = Some(initialized);
        if !initialized {
            self.apply(crate::PresenterEvent::Warning(
                "This project isn't set up for Forge — no project guidance or custom agents. Run /init to set it up for me.".to_string(),
            ));
        }
    }
}
