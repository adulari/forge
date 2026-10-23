//! The one line a permission prompt adds when the project not being trusted is the only reason a
//! read-only command asks (S4/S5 of the TUI benchmark: git prompts in an untrusted project, and
//! nothing said why). The decision is core's; this only gathers what it needs from the surface.

use std::path::Path;

use forge_types::{PermissionMode, PermissionRule};

/// `Some("git")` (or `Some("commands")`) when the prompt for `tool` is open in the Ask temper and
/// `forge trust` would let the call run without one. Never changes what the call may do.
pub(crate) fn trust_hint_for(
    app: &forge_tui::App,
    tool: &str,
    rules: &[PermissionRule],
    workspace: &Path,
) -> Option<&'static str> {
    if app.temper != PermissionMode::Default.label() {
        return None;
    }
    let args: serde_json::Value = serde_json::from_str(&app.pending_call_args(tool)).ok()?;
    forge_core::permission::shell_trust_hint(
        PermissionMode::Default,
        tool,
        &args,
        rules,
        Some(workspace),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app_asking(temper: &str, tool: &str, args: &str) -> forge_tui::App {
        let mut app = forge_tui::App::default();
        app.set_temper(temper);
        app.apply(forge_tui::PresenterEvent::ToolStart {
            name: tool.into(),
            args: args.into(),
        });
        app.open_permission(tool, "Shell");
        app
    }

    #[test]
    fn only_the_ask_temper_gets_a_hint() {
        let dir = std::env::temp_dir();
        let args = r#"{"command":"git status"}"#;
        for temper in ["Auto-edit", "Full", "Auto", "Read-only"] {
            let app = app_asking(temper, "shell", args);
            assert_eq!(trust_hint_for(&app, "shell", &[], &dir), None, "{temper}");
        }
    }

    #[test]
    fn calls_that_trust_could_not_change_get_no_hint() {
        let dir = std::env::temp_dir();
        let ask = PermissionMode::Default.label();
        for (tool, args) in [
            ("shell", r#"{"command":"touch ccbench-marker.txt"}"#),
            ("shell", r#"{"command":"git push"}"#),
            ("shell", r#"{"command":"ls"}"#),
            ("shell", "not json"),
            ("write_file", r#"{"path":"a.txt","content":"x"}"#),
        ] {
            let app = app_asking(ask, tool, args);
            assert_eq!(trust_hint_for(&app, tool, &[], &dir), None, "{tool} {args}");
        }
    }
}
