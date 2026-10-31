//! `/host`: list or forget machines added with `forge host add`. Adding needs a terminal for the
//! safety-code approval, so the TUI points to the command instead.

use forge_tui::App;

pub(super) fn show(app: &mut App, arg: &str) {
    match crate::hosts::remove_argument(arg) {
        Some(name) => match crate::hosts::remove(name) {
            Ok(true) => app.note(&format!("forgot {name}")),
            Ok(false) => app.note(&format!("no machine named {name}")),
            Err(error) => app.note(&format!("could not update the host list: {error}")),
        },
        _ => match crate::hosts::list() {
            Ok(list) if list.is_empty() => {
                app.note("no machines added yet — run `forge host add user@host` in a terminal")
            }
            Ok(list) => {
                app.note(&format!(
                    "{} machines (add more: `forge host add user@host`):",
                    list.len()
                ));
                for host in list {
                    let state = if host.paired {
                        "paired"
                    } else {
                        "service only"
                    };
                    app.note(&format!("  {}  {}  {}", host.name, host.target, state));
                }
            }
            Err(error) => app.note(&format!("could not read the host list: {error}")),
        },
    }
}
