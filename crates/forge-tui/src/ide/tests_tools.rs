//! The review tools: the editor, why-trace, the review draft, checks, shells, the terminal, the
//! Files tab, the ship row and the coach line, driven through keys and drawn at three widths.

use std::time::Instant;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::backend::TestBackend;
use ratatui::Terminal;

use super::checks::{WireAction, WireActions};
use super::wire::*;
use super::*;

const NOW: i64 = 1_000_000;

fn changes() -> ProjectChanges {
    ProjectChanges {
        cwd: "/repo".into(),
        sessions: vec![WireSessionChanges {
            session_id: "a".into(),
            title: "agent a".into(),
            branch: "main".into(),
            files: vec![
                WireChangedFile {
                    path: "src/a.rs".into(),
                    kind: "modified".into(),
                    adds: 2,
                    dels: 1,
                    content_hash: "h1".into(),
                },
                WireChangedFile {
                    path: "src/b.rs".into(),
                    kind: "modified".into(),
                    adds: 1,
                    dels: 1,
                    content_hash: "h2".into(),
                },
            ],
            ..Default::default()
        }],
    }
}

fn diff_file(path: &str, hunks: usize) -> WireDiffFile {
    WireDiffFile {
        path: path.into(),
        kind: "modified".into(),
        adds: hunks,
        dels: hunks,
        hunks: (0..hunks)
            .map(|i| WireHunk {
                header: format!("@@ -{0},2 +{0},2 @@", i * 10 + 1),
                lines: vec![" ctx".into(), "-old".into(), format!("+new{i}")],
            })
            .collect(),
        ..Default::default()
    }
}

fn app() -> IdeApp {
    let mut app = IdeApp::new("/repo", NOW);
    app.apply(IdeEvent::Reviewed(Default::default()));
    app.apply(IdeEvent::Prefs { coach_seen: true });
    app.apply(IdeEvent::Changes(Ok(changes())));
    app
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

fn press(app: &mut IdeApp, c: char) -> Vec<IdeAction> {
    handle_key(app, key(KeyCode::Char(c)))
}

fn type_text(app: &mut IdeApp, text: &str) {
    for c in text.chars() {
        press(app, c);
    }
}

fn screen(app: &mut IdeApp, w: u16, h: u16) -> String {
    let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
    term.draw(|f| draw(app, f)).unwrap();
    let buf = term.backend().buffer().clone();
    (0..h)
        .map(|y| {
            (0..w)
                .map(|x| buf[(x, y)].symbol().to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn open_diff(app: &mut IdeApp) -> String {
    let actions = press(app, 'j');
    let (path, session_id) = actions
        .iter()
        .find_map(|a| match a {
            IdeAction::FetchDiff {
                path, session_id, ..
            } => Some((path.clone(), session_id.clone())),
            _ => None,
        })
        .expect("opening a file asks for its diff");
    app.apply(IdeEvent::Diff(DiffLoaded {
        session_id,
        path: path.clone(),
        bucket: hunk::Bucket::Unstaged,
        file: Ok(Some(diff_file(&path, 3))),
    }));
    path
}

fn open_editor(app: &mut IdeApp, content: &str) -> String {
    let path = open_diff(app);
    let actions = press(app, 'e');
    assert!(
        actions
            .iter()
            .any(|a| matches!(a, IdeAction::FetchFile { path: p, .. } if *p == path)),
        "{actions:?}"
    );
    app.apply(IdeEvent::File {
        session_id: "a".into(),
        path: path.clone(),
        result: Ok(WireFile {
            content: content.into(),
            hash: "disk1".into(),
            read_only: false,
        }),
    });
    path
}

#[test]
fn e_opens_the_editor_and_typing_edits_the_buffer() {
    let mut app = app();
    let path = open_editor(&mut app, "fn main() {\n}\n");
    assert!(app.editing());
    type_text(&mut app, "let x = 1;");
    handle_key(&mut app, key(KeyCode::Enter));
    handle_key(&mut app, key(KeyCode::Tab));
    assert!(app.edit_dirty());
    for width in [100, 140, 200] {
        let s = screen(&mut app, width, 40);
        assert!(s.contains("edit"), "{width}: {s}");
        assert!(s.contains(&path), "{width}: {s}");
        assert!(
            s.contains("let x = 1;") && s.contains("fn main()"),
            "{width}: {s}"
        );
        assert!(s.contains("modified"), "{width}: {s}");
    }
}

#[test]
fn ctrl_s_saves_with_the_loaded_hash_and_clears_dirty() {
    let mut app = app();
    let path = open_editor(&mut app, "one\n");
    press(&mut app, 'x');
    let actions = handle_key(&mut app, ctrl('s'));
    let IdeAction::SaveFile {
        content,
        expected_hash,
        path: saved,
        ..
    } = &actions[0]
    else {
        panic!("{actions:?}");
    };
    assert_eq!(content, "xone\n");
    assert_eq!(expected_hash.as_deref(), Some("disk1"));
    assert_eq!(saved, &path);
    let after = app.apply(IdeEvent::Saved {
        session_id: "a".into(),
        path,
        result: Ok("disk2".into()),
    });
    assert!(after.contains(&IdeAction::RefreshChanges));
    assert!(!app.edit_dirty());
}

#[test]
fn a_save_conflict_offers_reload_or_overwrite() {
    let mut app = app();
    let path = open_editor(&mut app, "one\n");
    press(&mut app, 'x');
    handle_key(&mut app, ctrl('s'));
    app.apply(IdeEvent::Saved {
        session_id: "a".into(),
        path: path.clone(),
        result: Err(SaveFailure::Conflict),
    });
    let s = screen(&mut app, 140, 30);
    assert!(s.contains("agent changed this file"), "{s}");
    assert!(s.contains("reload"), "{s}");
    assert!(s.contains("overwrite"), "{s}");
    // Overwrite saves again without a stale hash.
    let actions = press(&mut app, 'o');
    assert!(matches!(
        &actions[0],
        IdeAction::SaveFile {
            expected_hash: None,
            ..
        }
    ));
    app.apply(IdeEvent::Saved {
        session_id: "a".into(),
        path,
        result: Err(SaveFailure::Conflict),
    });
    let actions = press(&mut app, 'r');
    assert!(matches!(&actions[0], IdeAction::FetchFile { .. }));
}

#[test]
fn esc_with_unsaved_edits_asks_even_when_the_editor_is_not_focused() {
    let mut app = app();
    open_editor(&mut app, "one\n");
    press(&mut app, 'x');
    assert!(app.edit_dirty());
    app.focus = Pane::Rail;
    assert!(handle_key(&mut app, key(KeyCode::Esc)).is_empty());
    assert!(app.confirm.is_some(), "Esc must not drop the edit");
    assert!(app.edit_dirty());
    assert_eq!(press(&mut app, 'y'), vec![IdeAction::Close]);
}

#[test]
fn closing_with_unsaved_edits_asks_first() {
    let mut app = app();
    open_editor(&mut app, "one\n");
    press(&mut app, 'x');
    assert!(handle_key(&mut app, key(KeyCode::Esc)).is_empty());
    assert!(app.confirm.is_some());
    press(&mut app, 'n');
    assert!(app.editing() && app.edit_dirty(), "n keeps the edits");
    let actions = handle_key(&mut app, key(KeyCode::F(2)));
    assert!(actions.is_empty() && app.confirm.is_some());
    assert_eq!(press(&mut app, 'y'), vec![IdeAction::Close]);
}

#[test]
fn other_files_do_not_open_over_unsaved_edits() {
    let mut app = app();
    open_editor(&mut app, "one\n");
    press(&mut app, 'x');
    let actions = app.open_file("a".into(), "src/b.rs".into());
    assert!(actions.is_empty());
    assert_eq!(app.open.as_ref().unwrap().path, "src/a.rs");
}

#[test]
fn undo_and_redo_run_in_the_editor() {
    let mut app = app();
    open_editor(&mut app, "one\n");
    type_text(&mut app, "ab");
    handle_key(&mut app, ctrl('z'));
    assert!(!app.edit_dirty());
    handle_key(&mut app, ctrl('y'));
    assert!(app.edit_dirty());
}

#[test]
fn the_agent_editing_a_clean_buffer_reloads_it() {
    let mut app = app();
    open_editor(&mut app, "one\n");
    let mut moved = changes();
    moved.sessions[0].files[0].content_hash = "h9".into();
    let actions = app.apply(IdeEvent::Changes(Ok(moved)));
    assert!(
        actions
            .iter()
            .any(|a| matches!(a, IdeAction::FetchFile { .. })),
        "{actions:?}"
    );
}

#[test]
fn comments_batch_into_one_review() {
    let mut app = app();
    open_diff(&mut app);
    press(&mut app, 'c');
    type_text(&mut app, "rename this");
    handle_key(&mut app, key(KeyCode::Enter));
    press(&mut app, 'n');
    press(&mut app, 'c');
    type_text(&mut app, "and this");
    handle_key(&mut app, key(KeyCode::Enter));
    assert_eq!(app.draft_count(), 2);
    let s = screen(&mut app, 200, 40);
    assert!(s.contains("2 comments"), "{s}");
    let actions = press(&mut app, 'S');
    let IdeAction::Input(id, json) = &actions[0] else {
        panic!("{actions:?}");
    };
    assert_eq!(id, "a");
    let text = json["text"].as_str().unwrap();
    assert!(text.starts_with("Please address this review."));
    assert!(text.contains("1. `src/a.rs`") && text.contains("2. `src/a.rs`"));
    assert!(text.contains("rename this") && text.contains("and this"));
    assert_eq!(app.draft_count(), 0);
}

#[test]
fn a_single_comment_can_be_sent_right_away() {
    let mut app = app();
    open_diff(&mut app);
    press(&mut app, 'c');
    type_text(&mut app, "just this");
    let actions = handle_key(&mut app, KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT));
    let IdeAction::Input(_, json) = &actions[0] else {
        panic!("{actions:?}");
    };
    assert!(json["text"]
        .as_str()
        .unwrap()
        .starts_with("Please address this review comment."));
    assert_eq!(app.draft_count(), 0);
}

#[test]
fn explain_asks_a_btw_side_question() {
    let mut app = app();
    open_diff(&mut app);
    let actions = press(&mut app, 'b');
    let IdeAction::Input(_, json) = &actions[0] else {
        panic!("{actions:?}");
    };
    assert!(json["text"]
        .as_str()
        .unwrap()
        .starts_with("/btw Explain this change in src/a.rs"));
}

#[test]
fn the_why_gutter_marks_lines_and_the_popup_jumps_to_the_turn() {
    let mut app = app();
    open_diff(&mut app);
    let actions = press(&mut app, 'w');
    assert!(actions
        .iter()
        .any(|a| matches!(a, IdeAction::FetchBlame { .. })));
    app.apply(IdeEvent::Blame {
        session_id: "a".into(),
        path: "src/a.rs".into(),
        result: Ok(vec![wire::BlameLine {
            line: 1,
            session_id: "a".into(),
            turn: 7,
            model: Some("m::n".into()),
            tool_call_id: Some("call-1".into()),
            prompt_summary: Some("make it faster".into()),
            created_at: None,
        }]),
    });
    let s = screen(&mut app, 200, 40);
    assert!(s.contains('┃'), "{s}");
    press(&mut app, 'W');
    let s = screen(&mut app, 200, 40);
    assert!(s.contains("make it faster"), "{s}");
    assert!(s.contains("turn"), "{s}");
    handle_key(&mut app, key(KeyCode::Enter));
    assert_eq!(app.focus, Pane::Agent);
}

fn action(id: &str, running: bool, exit: Option<i32>) -> WireAction {
    WireAction {
        id: id.into(),
        name: format!("{id} suite"),
        command: format!("run {id}"),
        terminal_id: Some(format!("act-{id}")),
        running,
        exit_code: exit,
    }
}

#[test]
fn checks_run_show_output_and_jump_to_file_line() {
    let mut app = app();
    open_diff(&mut app);
    let actions = press(&mut app, 't');
    assert!(actions
        .iter()
        .any(|a| matches!(a, IdeAction::FetchChecks(_))));
    app.apply(IdeEvent::Checks(
        "a".into(),
        Ok(WireActions {
            actions: vec![action("test", false, Some(1)), action("lint", false, None)],
            untrusted: false,
        }),
    ));
    let s = screen(&mut app, 200, 44);
    assert!(s.contains("test suite") && s.contains("failed (1)"), "{s}");
    app.apply(IdeEvent::CheckOutput {
        session_id: "a".into(),
        action: "test".into(),
        text: "error: boom\n --> src/a.rs:3:5\n".into(),
        reset: true,
    });
    assert_eq!(
        app.rows
            .iter()
            .find(|r| r.path == "src/a.rs")
            .map(|r| r.tier),
        Some(model::Tier::Failing),
        "a failing check marks the file it names"
    );
    let actions = press(&mut app, 'f');
    let IdeAction::Input(_, json) = &actions[0] else {
        panic!("{actions:?}");
    };
    assert!(json["text"].as_str().unwrap().contains("check failed"));
    press(&mut app, 'o');
    let s = screen(&mut app, 200, 44);
    assert!(s.contains("src/a.rs:3:5"), "{s}");
    let actions = handle_key(&mut app, key(KeyCode::Enter));
    assert!(
        actions
            .iter()
            .any(|a| matches!(a, IdeAction::FetchFile { path, .. } if path == "src/a.rs")),
        "{actions:?}"
    );
    assert!(app.editing());
}

#[test]
fn running_a_check_asks_the_host() {
    let mut app = app();
    open_diff(&mut app);
    press(&mut app, 't');
    app.apply(IdeEvent::Checks(
        "a".into(),
        Ok(WireActions {
            actions: vec![action("test", false, None)],
            untrusted: false,
        }),
    ));
    let actions = handle_key(&mut app, key(KeyCode::Enter));
    assert!(matches!(&actions[0], IdeAction::RunCheck { action, .. } if action == "test"));
}

#[test]
fn an_empty_checks_tab_says_what_to_do() {
    let mut app = app();
    open_diff(&mut app);
    press(&mut app, 't');
    app.apply(IdeEvent::Checks("a".into(), Ok(WireActions::default())));
    let s = screen(&mut app, 140, 40);
    assert!(s.contains("[[actions]]"), "{s}");
}

#[test]
fn shells_list_and_follow_a_log() {
    let mut app = app();
    open_diff(&mut app);
    press(&mut app, 't');
    let actions = press(&mut app, ']');
    assert!(actions.iter().any(|a| matches!(a, IdeAction::FetchJobs(_))));
    app.apply(IdeEvent::Jobs(
        "a".into(),
        Ok(vec![WireJob {
            id: 4,
            command: "npm run dev".into(),
            running: true,
            status: "running".into(),
            started_at: NOW - 90,
        }]),
    ));
    let s = screen(&mut app, 140, 40);
    assert!(s.contains("npm run dev"), "{s}");
    let actions = handle_key(&mut app, key(KeyCode::Enter));
    assert!(matches!(
        &actions[0],
        IdeAction::FetchJobTail { job: 4, .. }
    ));
    app.apply(IdeEvent::JobTail {
        session_id: "a".into(),
        job: 4,
        result: Ok(WireTail {
            running: true,
            status: "running".into(),
            text: "ready on :3000\n".into(),
        }),
    });
    let s = screen(&mut app, 140, 40);
    assert!(s.contains("ready on :3000"), "{s}");
}

#[test]
fn the_terminal_opens_echoes_and_esc_esc_leaves() {
    let mut app = app();
    open_diff(&mut app);
    press(&mut app, 't');
    screen(&mut app, 140, 40);
    press(&mut app, ']');
    press(&mut app, ']');
    screen(&mut app, 140, 40);
    let actions = app.apply(IdeEvent::Tick);
    let open = actions
        .iter()
        .find(|a| matches!(a, IdeAction::TermOpen { .. }))
        .expect("the terminal tab opens a shell");
    let IdeAction::TermOpen { cols, rows, .. } = open else {
        unreachable!()
    };
    assert!(*cols >= 20 && *rows >= 4);
    app.apply(IdeEvent::TermBytes(
        b"user@host$ echo hi\r\nhi\r\n".to_vec(),
    ));
    let s = screen(&mut app, 140, 40);
    assert!(s.contains("echo hi"), "{s}");
    assert!(s.contains("\nhi") || s.contains(" hi"), "{s}");
    // Keys reach the shell, `t` included.
    let actions = press(&mut app, 't');
    assert_eq!(actions, vec![IdeAction::TermInput(b"t".to_vec())]);
    let actions = handle_key(&mut app, key(KeyCode::Enter));
    assert_eq!(actions, vec![IdeAction::TermInput(b"\r".to_vec())]);
    let actions = handle_key(&mut app, key(KeyCode::Esc));
    assert_eq!(actions, vec![IdeAction::TermInput(vec![0x1b])]);
    assert!(handle_key(&mut app, key(KeyCode::Esc)).is_empty());
    assert_eq!(app.focus, Pane::Editor);
}

#[test]
fn the_files_tab_lists_the_tree_and_marks_changed_files() {
    let mut app = app();
    let actions = press(&mut app, '3');
    assert!(matches!(
        &actions[0],
        IdeAction::FetchEntries { dir, .. } if dir.is_empty()
    ));
    app.apply(IdeEvent::Entries {
        session_id: "a".into(),
        dir: String::new(),
        result: Ok(WireEntries {
            entries: vec![
                WireEntry {
                    name: "src".into(),
                    path: "src".into(),
                    kind: "directory".into(),
                },
                WireEntry {
                    name: "README.md".into(),
                    path: "README.md".into(),
                    kind: "file".into(),
                },
            ],
            truncated: 0,
        }),
    });
    let s = screen(&mut app, 140, 40);
    assert!(s.contains("src/") && s.contains("README.md"), "{s}");
    let actions = handle_key(&mut app, key(KeyCode::Enter));
    assert!(matches!(&actions[0], IdeAction::FetchEntries { dir, .. } if dir == "src"));
    app.apply(IdeEvent::Entries {
        session_id: "a".into(),
        dir: "src".into(),
        result: Ok(WireEntries {
            entries: vec![WireEntry {
                name: "a.rs".into(),
                path: "src/a.rs".into(),
                kind: "file".into(),
            }],
            truncated: 0,
        }),
    });
    let s = screen(&mut app, 140, 40);
    assert!(s.contains("a.rs"), "{s}");
    press(&mut app, 'j');
    let actions = handle_key(&mut app, key(KeyCode::Enter));
    assert!(
        actions
            .iter()
            .any(|a| matches!(a, IdeAction::FetchDiff { path, .. } if path == "src/a.rs")),
        "a changed file opens its diff: {actions:?}"
    );
}

fn git_status(staged: usize, unstaged: usize) -> GitStatus {
    let row = |p: &str| GitRow {
        path: p.into(),
        status: "M".into(),
        ..Default::default()
    };
    GitStatus {
        branch: "main".into(),
        has_remote: true,
        staged: (0..staged).map(|i| row(&format!("s{i}.rs"))).collect(),
        unstaged: (0..unstaged).map(|i| row(&format!("u{i}.rs"))).collect(),
        ..Default::default()
    }
}

#[test]
fn the_ship_row_says_what_it_will_do_and_runs_it() {
    let mut app = app();
    press(&mut app, '4');
    app.apply(IdeEvent::GitStatus("a".into(), Ok(git_status(0, 2))));
    assert_eq!(
        app.ship_hint(),
        "Will stage, write a message, commit and push"
    );
    let s = screen(&mut app, 140, 40);
    assert!(s.contains("Ship it"), "{s}");
    let actions = press(&mut app, 'Z');
    let IdeAction::Ship {
        stage,
        message,
        push,
        ..
    } = &actions[0]
    else {
        panic!("{actions:?}");
    };
    assert_eq!(stage.len(), 2);
    assert!(message.is_none() && *push);
    app.apply(IdeEvent::GitStatus("a".into(), Ok(git_status(1, 0))));
    app.apply(IdeEvent::Suggested(
        "a".into(),
        Ok("fix(core): handle empty input\n\nThe parser panicked.".into()),
    ));
    assert_eq!(app.ship_hint(), "Will commit and push");
    let actions = press(&mut app, 'Z');
    let IdeAction::Ship { message, .. } = &actions[0] else {
        panic!("{actions:?}");
    };
    assert_eq!(
        message.as_deref(),
        Some("fix(core): handle empty input\n\nThe parser panicked.")
    );
}

#[test]
fn the_coach_line_shows_once_advances_and_is_remembered() {
    let mut app = IdeApp::new("/repo", NOW);
    app.apply(IdeEvent::Reviewed(Default::default()));
    app.apply(IdeEvent::Changes(Ok(changes())));
    app.apply(IdeEvent::Prefs { coach_seen: false });
    let s = screen(&mut app, 140, 30);
    assert!(s.contains("1/3"), "{s}");
    open_diff(&mut app);
    let s = screen(&mut app, 140, 30);
    assert!(s.contains("2/3"), "{s}");
    press(&mut app, 'r');
    let s = screen(&mut app, 140, 30);
    assert!(s.contains("3/3") || s.contains("2/3"), "{s}");
    let actions = press(&mut app, 'H');
    assert!(matches!(&actions[0], IdeAction::SavePrefs(json) if json.contains("coach_seen")));
    let s = screen(&mut app, 140, 30);
    assert!(!s.contains("H hide"), "{s}");
    let mut seen = IdeApp::new("/repo", NOW);
    seen.apply(IdeEvent::Prefs { coach_seen: true });
    assert!(seen.coach_line().is_none());
}

#[test]
fn help_lists_the_new_keys() {
    let keys: String = HELP
        .iter()
        .map(|(k, what)| format!("{k} {what}"))
        .collect::<Vec<_>>()
        .join(" ");
    for want in [
        "Ctrl+S",
        "W",
        "S / Ctrl+Enter",
        "Checks tab",
        "Terminal tab",
        "C",
    ] {
        assert!(keys.contains(want), "{want} missing from {keys}");
    }
}

fn big_file(lines: usize) -> String {
    (0..lines)
        .map(|i| format!("    let value_{i} = compute(\"{i}\", {i} * 2); // line {i}\n"))
        .collect()
}

fn big_diff(lines: usize) -> WireDiffFile {
    let hunks = lines / 25;
    WireDiffFile {
        path: "src/a.rs".into(),
        kind: "modified".into(),
        adds: lines,
        dels: lines,
        hunks: (0..hunks)
            .map(|h| WireHunk {
                header: format!("@@ -{0},25 +{0},25 @@", h * 25 + 1),
                lines: (0..25)
                    .map(|l| match l % 3 {
                        0 => format!(" let ctx_{h}_{l} = 1;"),
                        1 => format!("-let old_{h}_{l} = 2;"),
                        _ => format!("+let new_{h}_{l} = 3;"),
                    })
                    .collect(),
            })
            .collect(),
        ..Default::default()
    }
}

fn average_frame_ms(app: &mut IdeApp, width: u16, height: u16, frames: u32) -> f64 {
    let mut term = Terminal::new(TestBackend::new(width, height)).unwrap();
    term.draw(|f| draw(app, f)).unwrap();
    let start = Instant::now();
    for _ in 0..frames {
        term.draw(|f| draw(app, f)).unwrap();
    }
    start.elapsed().as_secs_f64() * 1000.0 / f64::from(frames)
}

/// A frame must stay under 16 ms with a 5,000-line file open (the bound is generous in a debug
/// build; release numbers are printed by `cargo test --release -- --nocapture`).
#[test]
fn a_five_thousand_line_file_draws_inside_a_frame() {
    let budget = if cfg!(debug_assertions) { 120.0 } else { 16.0 };

    let mut app = app();
    open_editor(&mut app, &big_file(5000));
    handle_key(&mut app, key(KeyCode::PageDown));
    for _ in 0..40 {
        handle_key(&mut app, key(KeyCode::PageDown));
    }
    let edit = average_frame_ms(&mut app, 200, 50, 30);
    println!("editor, 5,000 lines, 200x50: {edit:.2} ms/frame");

    let mut app = app_with_big_diff();
    let diff = average_frame_ms(&mut app, 200, 50, 30);
    println!("diff, 5,000 lines, 200x50: {diff:.2} ms/frame");
    for _ in 0..30 {
        handle_key(&mut app, key(KeyCode::PageDown));
    }
    let scrolled = average_frame_ms(&mut app, 200, 50, 30);
    println!("diff scrolled, 5,000 lines, 200x50: {scrolled:.2} ms/frame");

    assert!(edit < budget, "editor frame {edit:.2} ms");
    assert!(diff < budget, "diff frame {diff:.2} ms");
    assert!(scrolled < budget, "scrolled diff frame {scrolled:.2} ms");
}

fn app_with_big_diff() -> IdeApp {
    let mut app = app();
    let actions = press(&mut app, 'j');
    let (path, session_id) = actions
        .iter()
        .find_map(|a| match a {
            IdeAction::FetchDiff {
                path, session_id, ..
            } => Some((path.clone(), session_id.clone())),
            _ => None,
        })
        .unwrap();
    app.apply(IdeEvent::Diff(DiffLoaded {
        session_id,
        path,
        bucket: hunk::Bucket::Unstaged,
        file: Ok(Some(big_diff(5000))),
    }));
    app
}

#[test]
fn opening_a_timeline_edit_lands_on_the_hunk_that_holds_its_line() {
    use crate::board::HistoryRow;
    let mut app = app();
    app.apply(IdeEvent::History(
        "a".into(),
        vec![HistoryRow {
            seq: 4,
            role: "tool".into(),
            kind: "tool".into(),
            tool: Some("edit_file".into()),
            tool_phase: Some("call".into()),
            content: r#"{"path":"/repo/src/a.rs","old_string":"x","new_string":"let target = 5;"}"#
                .into(),
            ..Default::default()
        }],
    ));
    let mut file = diff_file("src/a.rs", 3);
    file.hunks[2].lines.push("+let target = 5;".into());
    app.focus = Pane::Agent;
    let actions = app.open_timeline(0);
    let (path, session_id) = actions
        .iter()
        .find_map(|a| match a {
            IdeAction::FetchDiff {
                path, session_id, ..
            } => Some((path.clone(), session_id.clone())),
            _ => None,
        })
        .expect("the edit's file is fetched");
    app.apply(IdeEvent::Diff(DiffLoaded {
        session_id,
        path,
        bucket: hunk::Bucket::Unstaged,
        file: Ok(Some(file)),
    }));
    assert_eq!(app.open.as_ref().unwrap().hunk, 2);
}

#[test]
fn esc_closes_a_check_output_before_it_leaves_ide_mode() {
    let mut app = app();
    open_diff(&mut app);
    press(&mut app, 't');
    app.apply(IdeEvent::Checks(
        "a".into(),
        Ok(WireActions {
            actions: vec![action("test", false, Some(1))],
            untrusted: false,
        }),
    ));
    press(&mut app, 'o');
    assert!(app.tools.checks.show_output);
    assert!(handle_key(&mut app, key(KeyCode::Esc)).is_empty());
    assert!(!app.tools.checks.show_output);
    assert_eq!(
        handle_key(&mut app, key(KeyCode::Esc)),
        vec![IdeAction::Close]
    );
}

#[test]
fn the_claude_hand_off_opens_the_terminal_at_its_real_size_and_types_the_command() {
    let mut app = app();
    open_diff(&mut app);
    let actions = app.apply(IdeEvent::Handoff(Ok((
        "/my repo".into(),
        "claude --resume abc".into(),
    ))));
    assert!(
        !actions
            .iter()
            .any(|a| matches!(a, IdeAction::TermOpen { .. })),
        "nothing opens before the pane has a size: {actions:?}"
    );
    screen(&mut app, 200, 50);
    let actions = app.apply(IdeEvent::Tick);
    let open = actions
        .iter()
        .position(|a| matches!(a, IdeAction::TermOpen { cols, .. } if *cols > 60))
        .expect("opens at the drawn size");
    assert_eq!(
        actions[open + 1],
        IdeAction::TermInput(b"cd '/my repo' && claude --resume abc\n".to_vec())
    );
}
