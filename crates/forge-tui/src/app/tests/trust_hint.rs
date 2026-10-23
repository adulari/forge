//! The permission panel says why a read-only command asks in an untrusted project.

use super::*;

fn shell_prompt(command: &str) -> App {
    let mut app = App::default();
    app.apply(crate::PresenterEvent::ToolStart {
        name: "shell".into(),
        args: serde_json::json!({ "command": command }).to_string(),
    });
    app.open_permission("shell", "Shell");
    app
}

const HINT: &str = "Trust this project to skip prompts for read-only git: forge trust";

#[test]
fn a_hinted_prompt_carries_one_line_about_forge_trust() {
    let mut app = shell_prompt("git status");
    app.set_permission_trust_hint(Some("git"));
    for (w, h) in [(80u16, 24u16), (120, 40), (100, LIVE_H)] {
        let rows = screen_rows(&app, w, h);
        let text = rows.join("\n");
        assert!(text.contains(HINT), "{w}x{h}: {text}");
        let hint = rows.iter().position(|r| r.contains(HINT)).unwrap();
        let first_choice = rows
            .iter()
            .position(|r| r.contains("1  Allow once"))
            .unwrap();
        assert!(hint < first_choice, "{w}x{h}: above the choices: {text}");
        assert!(text.contains("Allow Shell to run this command?"), "{text}");
        assert!(text.contains("3  Deny"), "choices untouched: {text}");
    }
}

#[test]
fn a_prompt_without_a_hint_is_exactly_what_it_was() {
    let app = shell_prompt("git status");
    let text = screen_rows(&app, 100, 24).join("\n");
    assert!(!text.contains("forge trust"), "{text}");
    let mut hinted = shell_prompt("git status");
    hinted.set_permission_trust_hint(Some("git"));
    hinted.set_permission_trust_hint(None);
    assert_eq!(
        screen_rows(&hinted, 100, 24),
        screen_rows(&app, 100, 24),
        "clearing the hint restores the panel"
    );
}

#[test]
fn the_wording_follows_what_trust_would_cover() {
    let mut app = shell_prompt("file calc.py");
    app.set_permission_trust_hint(Some("commands"));
    let text = screen_rows(&app, 100, 24).join("\n");
    assert!(
        text.contains("skip prompts for read-only commands: forge trust"),
        "{text}"
    );
}

#[test]
fn the_line_shortens_in_steps_and_never_clips_a_word() {
    let mut app = shell_prompt("git status");
    app.set_permission_trust_hint(Some("git"));
    for width in 30..=100u16 {
        let rows = screen_rows(&app, width, 30);
        let Some(row) = rows.iter().find(|r| r.contains("forge trust")) else {
            panic!("{width}: the command must always be named: {rows:#?}");
        };
        let row = row.trim_end();
        assert!(row.chars().count() <= width as usize, "{width}: {row}");
        assert!(
            row.trim_end_matches([' ', '│', '┃'])
                .ends_with("forge trust"),
            "{width}: {row}"
        );
        let full = HINT.split_whitespace().collect::<Vec<_>>();
        let shorter = [
            "Trust", "this", "project", "to", "skip", "these", "prompts:", "forge", "trust",
            "Skip", "these", "prompts:", "prompts:",
        ];
        for word in row.split_whitespace().filter(|w| !w.contains('┃')) {
            assert!(
                full.contains(&word) || shorter.contains(&word),
                "{width}: {word:?} is clipped in {row:?}"
            );
        }
    }
}

#[test]
fn a_hint_for_a_prompt_that_is_not_open_is_ignored() {
    let mut app = App::default();
    app.set_permission_trust_hint(Some("git"));
    assert!(!screen(&app).contains("forge trust"));
}
