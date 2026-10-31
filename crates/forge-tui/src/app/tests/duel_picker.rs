//! The `/duel` winner picker at 80x24 and wide: ranked rows, the crown, the judge's line, and the
//! merge / open-PR / discard hints.

use super::*;
use crate::{KeyKind, PickerKind, PickerRow};

fn judged_rows() -> Vec<PickerRow> {
    vec![
        PickerRow {
            id: "forge/c".into(),
            title: "★ ✓ deepseek-v4.1-flash".into(),
            subtitle: "judge #1 9.0/10: tidy and small · 2 files +5 -1 · tests ✓ · 2.5s · $0.0123 · did it"
                .into(),
        },
        PickerRow {
            id: "forge/a".into(),
            title: "✓ kimi-k3".into(),
            subtitle: "judge #2 6.5/10: misses an edge case · 3 files +20 -4 · tests ✓ · 9.1s · $0.0400 · did it"
                .into(),
        },
        PickerRow {
            id: "forge/b".into(),
            title: "✗ glm-5".into(),
            subtitle: "judge #3 2.0/10: tests fail · 1 files +2 -0 · tests ✗ · 1.0s · $0.0010 · oops"
                .into(),
        },
    ]
}

fn open() -> App {
    let mut app = App::default();
    app.picker.open_with(
        PickerKind::Duel,
        "⚔ duel — pick the winner (3 candidates) · judge m::x: c is tidy",
        judged_rows(),
    );
    app
}

#[test]
fn the_ranked_picker_fits_80x24_with_the_crown_and_the_hints() {
    let rows = screen_rows(&open(), 80, 24).join("\n");
    assert!(
        rows.contains("duel — pick the winner (3 candidates)"),
        "{rows}"
    );
    assert!(
        !rows.contains("judge m::x"),
        "the verdict stays out of the title: {rows}"
    );
    assert!(rows.contains("★ ✓ deepseek-"), "{rows}");
    assert!(rows.contains("✗ glm-5"), "{rows}");
    assert!(
        rows.contains("judge #1 9.0/10: tidy and small"),
        "the detail line leads with the judge: {rows}"
    );
    assert!(
        rows.contains("merge") && rows.contains("open PR") && rows.contains("discard"),
        "{rows}"
    );
}

#[test]
fn the_detail_line_follows_the_cursor() {
    let mut app = open();
    app.picker.move_down();
    let rows = screen_rows(&app, 140, 30).join("\n");
    assert!(
        rows.contains("judge #2 6.5/10: misses an edge case"),
        "{rows}"
    );
}

#[test]
fn p_turns_into_enter_with_the_alternate_action_marked() {
    let mut app = open();
    assert_eq!(
        app.picker.digit_shortcut(KeyKind::Char('x')),
        KeyKind::Char('x')
    );
    assert!(!app.picker.alt);
    assert_eq!(
        app.picker.digit_shortcut(KeyKind::Char('p')),
        KeyKind::Enter
    );
    assert!(app.picker.alt);
    app.picker.close();
    assert!(app.picker.last_alt && !app.picker.alt);
    app.picker.open_with(PickerKind::Duel, "x", judged_rows());
    app.picker.query.push('z');
    assert_eq!(
        app.picker.digit_shortcut(KeyKind::Char('p')),
        KeyKind::Char('p')
    );
}

#[test]
fn p_means_nothing_in_other_pickers() {
    let mut app = App::default();
    app.picker
        .open_with(PickerKind::Sessions, "x", judged_rows());
    assert_eq!(
        app.picker.digit_shortcut(KeyKind::Char('p')),
        KeyKind::Char('p')
    );
    assert!(!app.picker.alt);
}
