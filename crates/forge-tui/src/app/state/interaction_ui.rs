//! UI state and entry points owned by the composer, popups and decision blocks.

use crate::app::KeyKind;
use crate::ConfirmOutcome;

use crate::app::App;

/// State only the interaction surfaces read or write. Starts empty; the interaction work package
/// adds fields here and nowhere else.
#[derive(Debug, Clone, Default)]
pub(crate) struct InteractionUi {}

impl App {
    /// Map a key pressed while a tool permission prompt is open to its outcome. `None` means the
    /// key is not a choice and the prompt stays open. Today every key decides: `a` always allows,
    /// `y` and Enter allow, anything else denies.
    pub fn permission_key(&mut self, key: KeyKind) -> Option<ConfirmOutcome> {
        Some(match key {
            KeyKind::Char('a') | KeyKind::Char('A') => ConfirmOutcome::AlwaysAllow,
            KeyKind::Char('y') | KeyKind::Char('Y') | KeyKind::Enter => ConfirmOutcome::Allow,
            _ => ConfirmOutcome::Deny,
        })
    }

    /// Open the permission prompt for `tool`; `side_effect` is its `SideEffect` debug name.
    pub fn open_permission(&mut self, tool: &str, side_effect: &str) {
        self.prompt = Some(format!("allow {tool} ({side_effect}) [y/n/a=always]"));
    }
}
