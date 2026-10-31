//! P37: wordmark variants, rotating composer examples, cursor shapes, the tokens-per-second
//! toggle and the verbosity cycle's effect on notices.

use super::*;
use crate::app::render::welcome::welcome_lines;
use crate::app::{CursorShape, Verbosity};

fn app() -> App {
    App {
        fullscreen: true,
        ..Default::default()
    }
}

fn welcome(app: &App, w: u16, h: u16) -> Vec<String> {
    welcome_lines(app, w, h)
        .iter()
        .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
        .collect()
}

fn widest_block_row(rows: &[String]) -> usize {
    rows.iter()
        .filter(|r| r.contains('█'))
        .map(|r| r.chars().count())
        .max()
        .unwrap_or(0)
}

#[test]
fn the_wordmark_has_wide_normal_and_one_row_variants() {
    let app = app();
    let wide = widest_block_row(&welcome(&app, 100, 40));
    let normal = widest_block_row(&welcome(&app, 60, 40));
    assert!(wide > normal && normal > 0, "wide {wide} normal {normal}");
    let narrow = welcome(&app, 24, 40);
    assert_eq!(widest_block_row(&narrow), 0, "{narrow:?}");
    assert!(narrow.iter().any(|r| r.contains("forge")));
}

#[test]
fn the_empty_composer_suggests_an_example_that_rotates() {
    let mut app = app();
    let first = screen_rows(&app, 120, 30).join("\n");
    assert!(first.contains("Message Forge… e.g. \""), "{first}");
    assert!(first.contains("! runs a shell command"), "{first}");
    let before = app.prompt_example();
    app.main_log.push(Default::default());
    assert_ne!(app.prompt_example(), before);
}

#[test]
fn a_bare_bang_suggests_a_shell_command() {
    let mut app = app();
    app.input = "!".to_string();
    app.input_cursor = 1;
    let text = screen_rows(&app, 100, 30).join("\n");
    assert!(text.contains("e.g. "), "{text}");
    assert!(text.contains(app.shell_example()), "{text}");
}

#[test]
fn the_bar_cursor_is_a_thin_glyph_past_the_text() {
    let mut app = app();
    app.input = "hi".to_string();
    app.input_cursor = 2;
    let block = screen_rows(&app, 80, 30).join("\n");
    assert!(!block.contains('\u{258F}'), "{block}");
    app.polish.cursor = CursorShape::Bar;
    let bar = screen_rows(&app, 80, 30).join("\n");
    assert!(bar.contains('\u{258F}'), "{bar}");
}

#[test]
fn quiet_drops_info_notices_but_not_warnings_and_default_restores_them() {
    let mut app = app();
    app.set_verbosity(Verbosity::Quiet);
    app.drain_flush();
    app.note("a routine note");
    app.note("⚠ something is wrong");
    let text = flush_text(&mut app);
    assert!(!text.contains("a routine note"), "{text}");
    assert!(text.contains("something is wrong"), "{text}");
    app.set_verbosity(Verbosity::Default);
    app.note("a routine note");
    assert!(flush_text(&mut app).contains("a routine note"));
}

#[test]
fn show_tps_hides_the_rate_from_the_status_line() {
    let mut app = app();
    assert!(app.polish.show_tps);
    app.polish.show_tps = false;
    let text = screen_rows(&app, 120, 30).join("\n");
    assert!(!text.contains("tok/s"), "{text}");
}

#[test]
fn tui_prefs_clamp_the_wheel_speed() {
    let mut app = app();
    let tui = forge_config::TuiConfig {
        scroll_speed: 0,
        ..Default::default()
    };
    app.apply_tui_prefs(&tui);
    assert_eq!(app.scroll_step(), 1);
    let tui = forge_config::TuiConfig {
        scroll_speed: 200,
        cursor: "underline".into(),
        ..Default::default()
    };
    app.apply_tui_prefs(&tui);
    assert_eq!(app.scroll_step(), 20);
    assert_eq!(app.polish.cursor, CursorShape::Underline);
}
