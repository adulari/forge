use super::*;
use crate::app::render::render_transcript_area;
use crate::PresenterEvent;
use ratatui::backend::TestBackend;
use ratatui::Terminal;

/// A full-screen app with folding on and an open assistant turn.
fn fold_app() -> App {
    let mut app = App {
        fullscreen: true,
        fold_tool_groups: true,
        transcript_follow: true,
        ..App::default()
    };
    app.submit_user("tour the project");
    app.ensure_assistant_label();
    let _ = app.drain_flush();
    app
}

fn start(app: &mut App, name: &str, args: &str) {
    app.apply(PresenterEvent::ToolStart {
        name: name.into(),
        args: args.into(),
    });
    let _ = app.drain_flush();
}

fn finish(app: &mut App, name: &str, ok: bool, detail: Option<&str>) {
    app.apply(PresenterEvent::ToolResult {
        name: name.into(),
        ok,
        summary: format!("{name}: exit {} in 12ms", if ok { 0 } else { 3 }),
        detail: detail.map(str::to_string),
    });
    let _ = app.drain_flush();
}

fn tool(app: &mut App, name: &str, args: &str) {
    start(app, name, args);
    finish(app, name, true, Some("out"));
}

fn prose(app: &mut App, text: &str) {
    app.apply(PresenterEvent::AssistantText(text.into()));
    let _ = app.drain_flush();
}

/// Draw the transcript into a `w`×`h` buffer and return its rows.
fn rows(app: &App, w: u16, h: u16) -> Vec<String> {
    let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
    terminal
        .draw(|f| render_transcript_area(f, f.area(), app))
        .unwrap();
    let buffer = terminal.backend().buffer();
    (0..h)
        .map(|y| (0..w).map(|x| buffer[(x, y)].symbol()).collect())
        .collect()
}

fn row_of(rows: &[String], needle: &str) -> Option<u16> {
    rows.iter()
        .position(|r| r.contains(needle))
        .map(|y| y as u16)
}

fn explore(app: &mut App) {
    tool(app, "list_dir", r#"{"path":"src"}"#);
    tool(app, "shell", r#"{"command":"cargo build"}"#);
    tool(app, "shell", r#"{"command":"cargo test"}"#);
}

#[test]
fn consecutive_calls_fold_into_one_dim_summary_row() {
    let mut app = fold_app();
    prose(&mut app, "Looking around first.");
    explore(&mut app);
    prose(&mut app, "All green.");
    let screen = rows(&app, 100, 30);
    let at = row_of(&screen, "Listed src, ran 2 shell commands")
        .or_else(|| row_of(&screen, "Listed 1 directory, ran 2 shell commands"))
        .unwrap_or_else(|| panic!("summary row: {screen:#?}"));
    assert!(screen[at as usize].contains('▸'), "{screen:#?}");
    assert!(
        !screen.iter().any(|r| r.contains("cargo build")),
        "the calls are folded away: {screen:#?}"
    );
    assert!(screen.iter().any(|r| r.contains("Looking around first.")));
    assert!(screen.iter().any(|r| r.contains("All green.")));
}

#[test]
fn the_summary_says_listed_1_directory_when_the_group_mixes_kinds() {
    let mut app = fold_app();
    explore(&mut app);
    let screen = rows(&app, 100, 30);
    assert!(
        screen
            .iter()
            .any(|r| r.contains("Listed 1 directory, ran 2 shell commands")),
        "{screen:#?}"
    );
}

#[test]
fn prose_between_calls_starts_a_new_group() {
    let mut app = fold_app();
    tool(&mut app, "read_file", r#"{"path":"a.rs"}"#);
    prose(&mut app, "Now the tests.");
    tool(&mut app, "shell", r#"{"command":"cargo test"}"#);
    let screen = rows(&app, 100, 30);
    let read = row_of(&screen, "Read a.rs").expect("first group");
    let ran = row_of(&screen, "Ran cargo test").expect("second group");
    let between = row_of(&screen, "Now the tests.").expect("prose");
    assert!(read < between && between < ran, "{screen:#?}");
}

#[test]
fn clicking_the_summary_opens_the_group_and_clicking_again_folds_it() {
    let mut app = fold_app();
    explore(&mut app);
    let screen = rows(&app, 100, 30);
    let y = row_of(&screen, "Listed 1 directory").unwrap();
    assert!(app.toggle_tool_card_at(5, y), "the summary is a control");
    let open = rows(&app, 100, 30);
    let header = row_of(&open, "▾ Listed 1 directory").expect("header stays");
    assert_eq!(header, y, "the header does not move under the pointer");
    assert!(
        open.iter().any(|r| r.contains("cargo build")) && open.iter().any(|r| r.contains("Run")),
        "the cards are back, as they always render: {open:#?}"
    );
    let card = row_of(&open, "cargo build").unwrap();
    assert!(
        open[card as usize].starts_with(" ▏   ✓ Run"),
        "the cards hang under the header: {open:#?}"
    );
    assert!(app.toggle_tool_card_at(5, header));
    let closed = rows(&app, 100, 30);
    assert!(
        !closed.iter().any(|r| r.contains("cargo build")),
        "{closed:#?}"
    );
}

#[test]
fn clicking_a_card_inside_an_open_group_toggles_that_card() {
    let mut app = fold_app();
    explore(&mut app);
    let screen = rows(&app, 100, 30);
    let y = row_of(&screen, "Listed 1 directory").unwrap();
    app.toggle_tool_card_at(5, y);
    let open = rows(&app, 100, 30);
    let card = row_of(&open, "cargo test").unwrap();
    assert!(app.toggle_tool_card_at(5, card));
    let after = rows(&app, 100, 30);
    assert!(
        after
            .iter()
            .any(|r| r.contains("click or Ctrl+T to collapse")),
        "the card opened inside the group: {after:#?}"
    );
    assert!(after.iter().any(|r| r.contains("▾ Listed")), "{after:#?}");
}

#[test]
fn expansion_survives_resizing_and_scrolling() {
    let mut app = fold_app();
    explore(&mut app);
    for i in 0..30 {
        prose(&mut app, &format!("line {i}"));
    }
    let screen = rows(&app, 100, 80);
    let y = row_of(&screen, "Listed 1 directory").unwrap();
    app.toggle_tool_card_at(5, y);
    for (w, h) in [(60, 20), (120, 40), (100, 80)] {
        app.transcript_scroll_up(5);
        let screen = rows(&app, w, h);
        app.transcript_to_top();
        let top = rows(&app, w, h);
        assert!(
            top.iter().any(|r| r.contains("▾ Listed")),
            "still open at {w}x{h}: {top:#?} {screen:#?}"
        );
    }
}

#[test]
fn a_running_group_shows_a_spinner_and_the_call_in_flight() {
    let mut app = fold_app();
    tool(&mut app, "read_file", r#"{"path":"a.rs"}"#);
    start(&mut app, "shell", r#"{"command":"cargo test --workspace"}"#);
    let screen = rows(&app, 100, 30);
    let summary = row_of(&screen, "Read a.rs").expect("finished calls are summarised");
    let running = row_of(&screen, "cargo test --workspace").expect("the running call shows");
    assert_eq!(running, summary + 1, "{screen:#?}");
    assert!(
        screen[running as usize].starts_with(" ▏   "),
        "the running call hangs under the summary: {screen:#?}"
    );
    let spinner = crate::theme::glyphs::g().spin(app.tick);
    assert!(
        screen[summary as usize].contains(spinner),
        "the summary spins while the group runs: {screen:#?}"
    );
    finish(&mut app, "shell", true, None);
    let done = rows(&app, 100, 30);
    assert!(
        done.iter()
            .any(|r| r.contains("Read 1 file, ran 1 shell command")),
        "{done:#?}"
    );
    assert!(!done.iter().any(|r| r.contains("cargo test --workspace")));
}

#[test]
fn the_first_call_of_a_group_shows_alone_while_it_runs() {
    let mut app = fold_app();
    start(&mut app, "shell", r#"{"command":"make"}"#);
    let screen = rows(&app, 100, 30);
    let at = row_of(&screen, "make").expect("running call");
    assert!(screen[at as usize].contains("Run"), "{screen:#?}");
    assert!(!screen.iter().any(|r| r.contains("Working")), "{screen:#?}");
}

#[test]
fn a_failed_call_stays_on_screen_under_the_summary() {
    let mut app = fold_app();
    tool(&mut app, "shell", r#"{"command":"ls"}"#);
    start(&mut app, "shell", r#"{"command":"cargo test -p upload"}"#);
    finish(&mut app, "shell", false, Some("test failed"));
    tool(&mut app, "shell", r#"{"command":"git status"}"#);
    let screen = rows(&app, 100, 30);
    let summary = row_of(&screen, "Ran 3 shell commands").expect("summary");
    assert!(screen[summary as usize].contains("1 failed"), "{screen:#?}");
    let failed = row_of(&screen, "cargo test -p upload").expect("the failure is named");
    assert_eq!(failed, summary + 1);
    assert!(screen[failed as usize].contains('✗'), "{screen:#?}");
    assert!(
        !screen[failed as usize].trim_end().ends_with('▸'),
        "a click there opens the group, so the card's triangle is gone: {screen:#?}"
    );
    assert!(!screen.iter().any(|r| r.contains("git status")));
}

#[test]
fn a_call_waiting_for_permission_shows_its_diff() {
    let mut app = fold_app();
    tool(&mut app, "read_file", r#"{"path":"a.rs"}"#);
    start(
        &mut app,
        "write_file",
        r#"{"path":"notes.md","content":"hello"}"#,
    );
    app.apply(PresenterEvent::Diff(forge_types::FileDiff {
        path: "notes.md".into(),
        kind: forge_types::DiffKind::Created,
        old: None,
        new: Some("hello world\n".into()),
        lang: None,
        binary: false,
    }));
    let _ = app.drain_flush();
    app.prompt = Some("Allow write_file?".into());
    let screen = rows(&app, 100, 30);
    let waiting = row_of(&screen, "awaiting approval").expect("the pending call");
    assert!(
        screen[waiting as usize + 1].contains("hello world"),
        "the diff the decision is about: {screen:#?}"
    );
    app.prompt = None;
    let after = rows(&app, 100, 30);
    assert!(
        !after.iter().any(|r| r.contains("hello world")),
        "{after:#?}"
    );
}

#[test]
fn turning_folding_off_restores_one_card_per_call() {
    let mut app = fold_app();
    explore(&mut app);
    app.fold_tool_groups = false;
    let screen = rows(&app, 100, 30);
    assert!(
        screen.iter().any(|r| r.contains("cargo build")),
        "{screen:#?}"
    );
    assert!(!screen.iter().any(|r| r.contains("ran 2 shell commands")));
    assert!(!app.toggle_group_at(5, 2));
}

#[test]
fn alt_o_opens_every_group_and_again_folds_them_all() {
    let mut app = fold_app();
    tool(&mut app, "read_file", r#"{"path":"a.rs"}"#);
    prose(&mut app, "Then:");
    tool(&mut app, "shell", r#"{"command":"cargo test"}"#);
    let _ = rows(&app, 100, 30);
    assert!(app.tool_group_key(KeyKind::ToggleToolGroups));
    let open = rows(&app, 100, 30);
    assert_eq!(
        open.iter().filter(|r| r.contains('▾')).count(),
        2,
        "{open:#?}"
    );
    // A group that arrives while everything is open opens too.
    prose(&mut app, "More:");
    tool(&mut app, "shell", r#"{"command":"git log"}"#);
    let more = rows(&app, 100, 30);
    assert!(more.iter().any(|r| r.contains("git log")), "{more:#?}");
    assert!(app.tool_group_key(KeyKind::ToggleToolGroups));
    let folded = rows(&app, 100, 30);
    assert!(!folded.iter().any(|r| r.contains('▾')), "{folded:#?}");
}

#[test]
fn keyboard_focus_moves_between_groups_and_enter_toggles() {
    let mut app = fold_app();
    tool(&mut app, "read_file", r#"{"path":"a.rs"}"#);
    prose(&mut app, "Then:");
    tool(&mut app, "shell", r#"{"command":"cargo test"}"#);
    let _ = rows(&app, 100, 30);
    // Nothing is captured until a group has focus.
    assert!(!app.tool_group_key(KeyKind::Enter));
    assert!(app.tool_group_key(KeyKind::PrevToolGroup));
    let screen = rows(&app, 100, 30);
    let focused = row_of(&screen, "Enter expand").expect("the focused row says what Enter does");
    assert!(
        screen[focused as usize].contains("Ran cargo test"),
        "{screen:#?}"
    );
    assert!(app.tool_group_key(KeyKind::Up));
    let screen = rows(&app, 100, 30);
    let focused = row_of(&screen, "Enter expand").unwrap();
    assert!(
        screen[focused as usize].contains("Read a.rs"),
        "{screen:#?}"
    );
    assert!(app.tool_group_key(KeyKind::Char(' ')));
    let screen = rows(&app, 100, 30);
    assert!(
        screen.iter().any(|r| r.contains("Enter collapse")),
        "{screen:#?}"
    );
    assert!(app.tool_group_key(KeyKind::Esc));
    assert_eq!(app.fold_ui.focus, None);
    // Typing lets go of the focus and is not swallowed.
    assert!(app.tool_group_key(KeyKind::NextToolGroup));
    assert!(!app.tool_group_key(KeyKind::Char('h')));
    assert_eq!(app.fold_ui.focus, None);
}

#[test]
fn a_pending_decision_keeps_its_keys() {
    let mut app = fold_app();
    tool(&mut app, "shell", r#"{"command":"ls"}"#);
    let _ = rows(&app, 100, 30);
    app.tool_group_key(KeyKind::PrevToolGroup);
    app.prompt = Some("Allow?".into());
    assert!(
        !app.tool_group_key(KeyKind::Enter),
        "Enter answers the prompt"
    );
    assert_eq!(app.fold_ui.focus, None);
}

#[test]
fn ctrl_t_opens_the_group_then_its_newest_call_then_folds_it() {
    let mut app = fold_app();
    explore(&mut app);
    let _ = rows(&app, 100, 30);
    assert!(app.toggle_last_tool_card());
    let group = app.tool_cards.last().unwrap().batch.unwrap();
    assert!(app.group_expanded(group));
    assert!(!app.tool_cards.last().unwrap().expanded);
    assert!(app.toggle_last_tool_card());
    assert!(app.tool_cards.last().unwrap().expanded);
    assert!(app.toggle_last_tool_card());
    assert!(!app.group_expanded(group) && !app.tool_cards.last().unwrap().expanded);
}

#[test]
fn a_new_turn_starts_a_new_group() {
    let mut app = fold_app();
    tool(&mut app, "shell", r#"{"command":"ls"}"#);
    app.apply(PresenterEvent::Done {
        final_text: String::new(),
        stop_reason: forge_types::StopReason::FinalAnswer,
    });
    let _ = app.drain_flush();
    app.submit_user("again");
    let _ = app.drain_flush();
    tool(&mut app, "shell", r#"{"command":"pwd"}"#);
    let batches: Vec<u64> = app.tool_cards.iter().filter_map(|c| c.batch).collect();
    assert_eq!(batches.len(), 2);
    assert_ne!(batches[0], batches[1]);
}

#[test]
fn clearing_the_transcript_forgets_cards_that_pointed_into_it() {
    let mut app = fold_app();
    explore(&mut app);
    app.clear_transcript();
    assert!(app.tool_cards.is_empty());
    assert!(app.group_spans().is_empty());
}

#[test]
fn a_running_call_inside_an_open_group_keeps_its_status_on_screen() {
    let mut app = fold_app();
    tool(&mut app, "read_file", r#"{"path":"a.rs"}"#);
    start(&mut app, "shell", r#"{"command":"cargo test"}"#);
    let _ = rows(&app, 100, 30);
    let group = app.tool_cards[0].batch.unwrap();
    app.set_group_anchored(group, true);
    let screen = rows(&app, 100, 30);
    let at = row_of(&screen, "cargo test").unwrap() as usize;
    assert!(screen[at].starts_with(" ▏   "), "indented: {screen:#?}");
    assert!(screen[at].trim_end().ends_with("running"), "{screen:#?}");
}
