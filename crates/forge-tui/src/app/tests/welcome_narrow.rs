//! The welcome wraps or shortens cleanly at any width from 40 up; it never clips a word.

use std::collections::BTreeSet;

use super::*;
use crate::app::render::welcome::{welcome_lines, TRY_EXAMPLES};
use unicode_width::UnicodeWidthStr;

fn app(onboarding: bool) -> App {
    let mut app = App {
        fullscreen: true,
        ..Default::default()
    };
    app.set_project_initialized(!onboarding);
    app.set_recent_sessions(vec![("fix the login redirect".into(), "3m ago".into())]);
    app
}

fn rows(app: &App, width: u16, height: u16) -> Vec<String> {
    welcome_lines(app, width, height)
        .iter()
        .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
        .collect()
}

fn words(rows: &[String]) -> BTreeSet<String> {
    rows.iter()
        .flat_map(|r| r.split_whitespace())
        .map(str::to_string)
        .collect()
}

#[test]
fn no_welcome_row_is_wider_than_the_terminal_from_40_columns_up() {
    for onboarding in [false, true] {
        for height in [24u16, 40] {
            for width in 40..=140u16 {
                for row in rows(&app(onboarding), width, height) {
                    assert!(
                        row.width() <= width as usize,
                        "{width}x{height} onboarding={onboarding}: {row:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn narrowing_only_removes_whole_entries_or_moves_words_to_the_next_row() {
    for onboarding in [false, true] {
        for width in 40..=140u16 {
            // The widest rendering of the same wording is the reference: a clipped word would be
            // one it does not contain.
            let reference = if width < 100 { 99 } else { 200 };
            let wide = words(&rows(&app(onboarding), reference, 40));
            for word in words(&rows(&app(onboarding), width, 40)) {
                assert!(
                    wide.contains(&word),
                    "{width} onboarding={onboarding}: {word:?} was cut"
                );
            }
        }
    }
}

#[test]
fn the_tagline_the_examples_and_the_nudge_survive_whole_at_every_width() {
    for width in 40..=140u16 {
        let all = words(&rows(&app(true), width, 40));
        for text in TRY_EXAMPLES
            .iter()
            .copied()
            .chain([crate::app::lines::banner::TAGLINE])
        {
            for word in text.split_whitespace() {
                assert!(
                    all.contains(word),
                    "{width}: {word:?} of {text:?} is missing"
                );
            }
        }
        for word in ["/init", "guidance", "agents"] {
            assert!(all.contains(word), "{width}: {word}");
        }
    }
}

#[test]
fn a_wrapped_example_hangs_under_its_own_text() {
    let rows = rows(&app(false), 40, 40);
    let at = rows
        .iter()
        .position(|r| r.contains("fix the failing test in this"))
        .expect("first example");
    assert!(rows[at].starts_with("   Try    "), "{:?}", rows[at]);
    assert!(
        rows[at + 1].starts_with("          project"),
        "continuation lines up with the text, not the label: {:?}",
        rows[at + 1]
    );
}

#[test]
fn the_nudge_keeps_the_key_and_what_it_does_together() {
    let narrow = rows(&app(true), 40, 40);
    let key = narrow.iter().find(|r| r.contains("/init")).unwrap();
    assert!(key.contains("/init writes guidance + agents"), "{key:?}");
    let wide = rows(&app(true), 120, 40);
    let one = wide.iter().find(|r| r.contains("/init")).unwrap();
    assert!(one.contains("isn't set up for Forge yet"), "{one:?}");
}

#[test]
fn the_composer_hint_gives_way_instead_of_clipping() {
    let app = App {
        fullscreen: true,
        ..Default::default()
    };
    for width in 40..=60u16 {
        let text = screen_rows(&app, width, 30).join("\n");
        assert!(text.contains("Message Forge…"), "{width}: {text}");
        let clipped = text
            .lines()
            .any(|l| l.contains("! runs") && !l.contains("! runs a shell command"));
        assert!(!clipped, "{width}: a half-shown shell hint: {text}");
    }
    let wide = screen_rows(&app, 80, 30).join("\n");
    assert!(wide.contains("! runs a shell command"), "{wide}");
}
