use super::*;
use crate::theme::color;
use ratatui::backend::TestBackend;
use ratatui::Terminal;

fn cells(s: &str) -> Vec<(String, bool)> {
    s.chars().map(|c| (c.to_string(), false)).collect()
}

#[test]
fn border_glyphs_are_stripped_at_row_edges_without_a_clip() {
    let row = cells("│  hello world      │");
    let end = row.len();
    assert_eq!(snapshot_row_text(&row, 0, end, true), "hello world");
}

#[test]
fn border_only_rows_yield_nothing() {
    let row = cells("╭──────────╮");
    assert_eq!(snapshot_row_text(&row, 0, row.len(), true), "");
}

#[test]
fn a_clipped_row_keeps_glyphs_that_are_content() {
    let row = cells("│ a │ b │");
    assert_eq!(snapshot_row_text(&row, 1, 8, false), " a │ b");
}

#[test]
fn decoration_cells_are_dropped() {
    let mut row = cells("┆ code");
    row[0].1 = true;
    assert_eq!(snapshot_row_text(&row, 0, row.len(), false), " code");
}

#[test]
fn wide_glyphs_are_not_doubled() {
    let row = vec![
        ("世".to_string(), false),
        (" ".to_string(), false),
        ("x".to_string(), false),
    ];
    assert_eq!(snapshot_row_text(&row, 0, 3, false), "世x");
}

#[test]
fn rows_join_with_newlines_and_trim_blank_edges() {
    let rows = vec![
        String::new(),
        "a".to_string(),
        String::new(),
        "b".to_string(),
        String::new(),
    ];
    assert_eq!(join_rows(rows), "a\n\nb");
}

fn btw_app() -> App {
    let mut app = App {
        fullscreen: true,
        ..App::default()
    };
    app.open_btw("What is two plus two?");
    app.apply_btw_answer(
        "What is two plus two?".into(),
        "It is four.\nSecond line here".into(),
        "model".into(),
        None,
    );
    app
}

fn draw(app: &App, w: u16, h: u16) -> Terminal<TestBackend> {
    let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
    t.draw(|f| render_live(f, app)).unwrap();
    t
}

fn find(t: &Terminal<TestBackend>, needle: &str) -> (u16, u16) {
    let buf = t.backend().buffer();
    for y in 0..buf.area.height {
        let line: String = (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect();
        if let Some(i) = line.find(needle) {
            return (line[..i].chars().count() as u16, y);
        }
    }
    panic!("{needle:?} not on screen");
}

#[test]
fn dragging_across_a_btw_card_copies_its_text_and_nothing_around_it() {
    let mut app = btw_app();
    let t = draw(&app, 100, 30);
    let (x0, y0) = find(&t, "It is four.");
    let (_, y1) = find(&t, "Second line here");
    assert!(app.screen_select_wanted(x0, y0));
    app.screen_select_begin(x0, y0);
    app.screen_select_extend(90, y1);
    draw(&app, 100, 30);
    let text = app.screen_select_end().expect("selection text");
    assert_eq!(text, "It is four.\nSecond line here");
}

#[test]
fn reversed_drag_copies_the_same_text() {
    let mut app = btw_app();
    let t = draw(&app, 100, 30);
    let (x0, y0) = find(&t, "It is four.");
    let (_, y1) = find(&t, "Second line here");
    app.screen_select_begin(90, y1);
    app.screen_select_extend(x0, y0);
    draw(&app, 100, 30);
    assert_eq!(
        app.screen_select_end().as_deref(),
        Some("It is four.\nSecond line here")
    );
}

#[test]
fn the_selection_is_highlighted() {
    let mut app = btw_app();
    let t = draw(&app, 100, 30);
    let (x0, y0) = find(&t, "It is four.");
    app.screen_select_begin(x0, y0);
    app.screen_select_extend(x0 + 4, y0);
    let t = draw(&app, 100, 30);
    assert_eq!(t.backend().buffer()[(x0, y0)].bg, color::selection());
    assert_ne!(t.backend().buffer()[(x0 + 8, y0)].bg, color::selection());
}

#[test]
fn a_plain_click_copies_nothing_and_leaves_no_highlight() {
    let mut app = btw_app();
    let t = draw(&app, 100, 30);
    let (x0, y0) = find(&t, "It is four.");
    app.screen_select_begin(x0, y0);
    draw(&app, 100, 30);
    assert_eq!(app.screen_select_end(), None);
    assert!(!app.screen_selection_active());
}

#[test]
fn closing_the_overlay_drops_the_selection() {
    let mut app = btw_app();
    let t = draw(&app, 100, 30);
    let (x0, y0) = find(&t, "It is four.");
    app.screen_select_begin(x0, y0);
    app.screen_select_extend(x0 + 3, y0);
    draw(&app, 100, 30);
    assert!(app.screen_selection_active());
    app.btw = None;
    draw(&app, 100, 30);
    assert!(!app.screen_selection_active());
}

#[test]
fn a_plain_transcript_press_stays_a_transcript_selection() {
    let mut app = App {
        fullscreen: true,
        ..App::default()
    };
    app.push_scrollback(vec![TextLine::from("alpha")]);
    draw(&app, 100, 30);
    assert!(!app.screen_select_wanted(2, 2));
    assert!(app.screen_select_wanted(2, 29));
    app.clear_screen_selection();
}
