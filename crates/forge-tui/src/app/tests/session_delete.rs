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
            "its rows, scrubbed where SQLite allows; `forge sessions --delete <id> --compact-db` (Forge closed) removes every trace".into(),
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
fn a_narrow_terminal_still_shows_the_id_prompt() {
    let app = App {
        session_delete: Some(dialog()),
        ..App::default()
    };
    let text = joined(&screen_rows(&app, 60, 24));
    assert!(text.contains("Type 0198abcd to confirm"), "{text}");
}

/// With a transcript behind it the overlay must not let the gutter or line ends show beside the
/// frame, at a small and a large size.
#[test]
fn the_overlay_is_opaque_across_its_rows_over_a_busy_transcript() {
    for (w, h) in [(80u16, 24u16), (160, 48)] {
        let mut app = App {
            fullscreen: true,
            ..App::default()
        };
        for i in 0..12 {
            app.submit_user(&format!(
                "transcript line {i} with some padding text to reach the edge"
            ));
            app.apply(crate::PresenterEvent::AssistantText(
                "an answer that is long enough to run to the right hand side of the screen d"
                    .into(),
            ));
            let _ = app.drain_flush();
        }
        app.transcript_to_bottom();
        let plain = screen_rows(&app, w, h).join("\n");
        assert!(
            plain.contains("transcript line"),
            "{w}x{h}: transcript not drawn:\n{plain}"
        );
        app.session_delete = Some(dialog());
        let rows = screen_rows(&app, w, h);
        let top = rows
            .iter()
            .position(|r| r.contains("Delete this session forever?"))
            .expect("dialog drawn");
        let bottom = rows.iter().rposition(|r| r.contains("Esc cancel")).unwrap();
        for row in &rows[top..=bottom] {
            let chars: Vec<char> = row.chars().collect();
            let first = chars
                .iter()
                .position(|c| *c == '╭' || *c == '│' || *c == '╰')
                .unwrap();
            let last = chars
                .iter()
                .rposition(|c| *c == '╮' || *c == '│' || *c == '╯')
                .unwrap();
            assert!(
                chars[..first].iter().all(|c| *c == ' '),
                "{w}x{h} left of frame: {row:?}"
            );
            assert!(
                chars[last + 1..].iter().all(|c| *c == ' '),
                "{w}x{h} right of frame: {row:?}"
            );
        }
        let _ = std::fs::write(
            format!("/tmp/claude-1000/sd/busy-{w}x{h}.txt"),
            rows.join("\n"),
        );
    }
}
