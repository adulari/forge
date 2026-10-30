//! The permanent-delete confirmation at 80x24: what it lists and how a refusal reads.

use super::*;
use crate::SessionDeleteDialog;

fn dialog() -> SessionDeleteDialog {
    SessionDeleteDialog {
        id: "0198abcdef0123456789".into(),
        title: "refactor the parser".into(),
        current: false,
        facts: vec![
            "9 turn(s), 31 message(s) and 14 tool record(s)".into(),
            "2 subagent session(s)".into(),
            "its usage, routing, task and rewind-snapshot records".into(),
            "its copy on your other synced devices".into(),
            "the database copy is zeroed, not just unlinked".into(),
        ],
        kept: vec!["1 saved memory item(s) from it (/memory removes them)".into()],
        refusal: None,
        typed: "0198".into(),
    }
}

fn rows(app: &App) -> Vec<String> {
    screen_rows(app, 80, 24)
}

fn joined(rows: &[String]) -> String {
    rows.join("\n")
}

#[test]
fn confirmation_names_the_session_and_says_it_is_permanent() {
    let app = App {
        session_delete: Some(dialog()),
        ..App::default()
    };
    let rows = rows(&app);
    let text = joined(&rows);
    for needle in [
        "Delete this session forever?",
        "refactor the parser",
        "permanently removes",
        "2 subagent session(s)",
        "other synced devices",
        "cannot be undone",
        "Type 0198abcd to confirm: 0198",
        "Esc cancel",
    ] {
        assert!(text.contains(needle), "{needle} missing from:\n{text}");
    }
    let _ = std::fs::create_dir_all("/tmp/claude-1000/sd");
    let _ = std::fs::write("/tmp/claude-1000/sd/confirm-80x24.txt", &text);
}

#[test]
fn refusal_shows_the_reason_and_the_next_step_inline() {
    let mut app = App::default();
    let mut d = dialog();
    d.refusal = Some((
        "this session is open in a forge chat terminal on ssu-dev".into(),
        "run /delete in that terminal, or close the chat and delete it here".into(),
    ));
    app.session_delete = Some(d);
    let text = joined(&rows(&app));
    for needle in [
        "Cannot delete this session",
        "open in a forge chat terminal on ssu-dev",
        "Instead: run /delete in that terminal",
        "Esc close",
    ] {
        assert!(text.contains(needle), "{needle} missing from:\n{text}");
    }
    assert!(
        !text.contains("Type 0198"),
        "a refusal must not ask for confirmation"
    );
    let _ = std::fs::write("/tmp/claude-1000/sd/refusal-80x24.txt", &text);
}

#[test]
fn a_small_terminal_still_shows_the_id_prompt() {
    let app = App {
        session_delete: Some(dialog()),
        ..App::default()
    };
    let text = joined(&screen_rows(&app, 60, 16));
    assert!(text.contains("Type 0198abcd to confirm"), "{text}");
}
