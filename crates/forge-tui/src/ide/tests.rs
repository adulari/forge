//! Screens at three widths, the queue as the app sees it, hash invalidation and the key flows.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::backend::TestBackend;
use ratatui::Terminal;

use super::wire::*;
use super::*;

const NOW: i64 = 1_000_000;

fn file(path: &str, hash: &str) -> WireChangedFile {
    WireChangedFile {
        path: path.into(),
        kind: "modified".into(),
        adds: 2,
        dels: 1,
        content_hash: hash.into(),
        ..Default::default()
    }
}

fn session(id: &str, waiting: bool, files: Vec<WireChangedFile>) -> WireSessionChanges {
    WireSessionChanges {
        session_id: id.into(),
        title: format!("agent {id}"),
        branch: "main".into(),
        waiting,
        files,
        ..Default::default()
    }
}

fn changes() -> ProjectChanges {
    ProjectChanges {
        cwd: "/repo".into(),
        sessions: vec![
            session(
                "a",
                false,
                vec![file("src/a.rs", "h1"), file("src/b.rs", "h2")],
            ),
            session("b", true, vec![file("lib.rs", "h3")]),
        ],
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
                lines: vec![" ctx".into(), "-old".into(), "+new".into()],
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    }
}

fn app() -> IdeApp {
    let mut app = IdeApp::new("/repo", NOW);
    app.apply(IdeEvent::Reviewed(Default::default()));
    app.apply(IdeEvent::Changes(Ok(changes())));
    app
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn press(app: &mut IdeApp, c: char) -> Vec<IdeAction> {
    handle_key(app, key(KeyCode::Char(c)))
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

fn open_first(app: &mut IdeApp) -> String {
    let actions = press(app, 'j');
    let fetch = actions.iter().find_map(|a| match a {
        IdeAction::FetchDiff {
            path, session_id, ..
        } => Some((path.clone(), session_id.clone())),
        _ => None,
    });
    let (path, session_id) = fetch.expect("opening a file asks for its diff");
    app.apply(IdeEvent::Diff(DiffLoaded {
        session_id,
        path: path.clone(),
        bucket: hunk::Bucket::Unstaged,
        file: Ok(Some(diff_file(&path, 3))),
    }));
    path
}

#[test]
fn the_waiting_agents_files_come_first_in_the_queue() {
    let app = app();
    assert_eq!(app.rows[0].path, "lib.rs");
    assert_eq!(app.agents[0].id, "b", "needs-you agents sort first");
}

#[test]
fn layouts_follow_the_width() {
    assert_eq!(mode_for(200), Mode::Wide);
    assert_eq!(mode_for(140), Mode::Mid);
    assert_eq!(mode_for(100), Mode::Narrow);
}

#[test]
fn wide_screen_shows_rail_editor_and_agent_panel() {
    let mut app = app();
    open_first(&mut app);
    let s = screen(&mut app, 200, 40);
    assert!(s.contains("Agents"), "{s}");
    assert!(s.contains("lib.rs"), "{s}");
    assert!(s.contains("hunk 1/3"), "{s}");
    assert!(s.contains("Activity"), "{s}");
    assert!(s.contains("needs you"), "{s}");
}

#[test]
fn mid_screen_hides_the_agent_panel_until_tab() {
    let mut app = app();
    open_first(&mut app);
    let s = screen(&mut app, 140, 40);
    assert!(!s.contains("Activity"), "{s}");
    press_tab(&mut app);
    press_tab(&mut app);
    let s = screen(&mut app, 140, 40);
    assert!(s.contains("Activity"), "{s}");
}

fn press_tab(app: &mut IdeApp) {
    handle_key(app, key(KeyCode::Tab));
}

#[test]
fn narrow_screen_is_one_pane_with_a_tab_bar() {
    let mut app = app();
    let s = screen(&mut app, 100, 30);
    assert!(
        s.contains("Changes") && s.contains("Editor") && s.contains("Bottom"),
        "{s}"
    );
    assert!(s.contains("lib.rs"));
    app.focus = Pane::Editor;
    let s = screen(&mut app, 100, 30);
    assert!(!s.contains("src/a.rs"), "the rail is not drawn: {s}");
}

#[test]
fn clicking_a_queue_row_opens_it_and_a_tab_switches_panes() {
    let mut app = app();
    screen(&mut app, 100, 30);
    let (rect, _) = app
        .hits
        .iter()
        .find(|(_, h)| matches!(h, Hit::Queue(k) if k.ends_with("src/a.rs")))
        .cloned()
        .unwrap();
    let actions = handle_mouse(
        &mut app,
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: rect.x + 3,
            row: rect.y,
            modifiers: KeyModifiers::NONE,
        },
    );
    assert!(actions
        .iter()
        .any(|a| matches!(a, IdeAction::FetchDiff { path, .. } if path == "src/a.rs")));
    assert_eq!(app.focus, Pane::Editor);
}

#[test]
fn a_changed_hash_drops_the_reviewed_mark_and_refetches_the_open_file() {
    let mut calm = changes();
    calm.sessions[1].waiting = false;
    let mut app = IdeApp::new("/repo", NOW);
    app.apply(IdeEvent::Reviewed(Default::default()));
    app.apply(IdeEvent::Changes(Ok(calm.clone())));
    let path = open_first(&mut app);
    assert_eq!(path, "src/a.rs");
    let actions = press(&mut app, 'r');
    assert!(actions
        .iter()
        .any(|a| matches!(a, IdeAction::SaveReviewed(_))));
    assert!(app
        .rows
        .iter()
        .any(|r| r.path == "src/a.rs" && r.tier == model::Tier::Reviewed));
    // Marking moved to the next file; open the reviewed one again, then the agent edits it.
    app.open_file("a".into(), "src/a.rs".into());
    let mut edited = calm;
    edited.sessions[0].files[0].content_hash = "h1-new".into();
    let actions = app.apply(IdeEvent::Changes(Ok(edited)));
    assert!(app
        .rows
        .iter()
        .any(|r| r.path == "src/a.rs" && r.tier == model::Tier::Unreviewed));
    assert!(
        actions
            .iter()
            .any(|a| matches!(a, IdeAction::SaveReviewed(_))),
        "stale mark pruned"
    );
    assert!(
        actions
            .iter()
            .any(|a| matches!(a, IdeAction::FetchDiff { .. })),
        "diff refetched"
    );
}

#[test]
fn accept_revert_and_undo_build_the_right_requests() {
    let mut app = app();
    open_first(&mut app);
    press(&mut app, 'n');
    let accept = press(&mut app, 'a');
    match &accept[0] {
        IdeAction::Hunk {
            action,
            header,
            patch,
            ..
        } => {
            assert_eq!(*action, hunk::HunkAction::Stage);
            assert_eq!(header, "@@ -11,2 +11,2 @@");
            assert!(patch.starts_with("--- a/lib.rs\n+++ b/lib.rs\n@@ -11,2 +11,2 @@\n"));
        }
        other => panic!("{other:?}"),
    }
    assert!(press(&mut app, 'x').is_empty(), "revert asks first");
    assert!(app.confirm.is_some());
    let revert = press(&mut app, 'y');
    let undo = match &revert[0] {
        IdeAction::Hunk { action, undo, .. } => {
            assert_eq!(*action, hunk::HunkAction::Revert);
            undo.clone().expect("a revert is undoable")
        }
        other => panic!("{other:?}"),
    };
    app.apply(IdeEvent::Hunk(HunkOutcome {
        session_id: "b".into(),
        path: "lib.rs".into(),
        action: hunk::HunkAction::Revert,
        result: Ok(()),
        undo: Some(undo),
        is_undo: false,
    }));
    let again = press(&mut app, 'u');
    match &again[0] {
        IdeAction::Hunk { patch, is_undo, .. } => {
            assert!(*is_undo);
            assert!(patch.contains("\n+old\n-new\n"), "{patch}");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_stale_hunk_refetches_instead_of_retrying() {
    let mut app = app();
    open_first(&mut app);
    let actions = app.apply(IdeEvent::Hunk(HunkOutcome {
        session_id: "b".into(),
        path: "lib.rs".into(),
        action: hunk::HunkAction::Stage,
        result: Err(HunkFailure::Stale(
            "The file changed since this diff loaded. Refreshed.".into(),
        )),
        undo: None,
        is_undo: false,
    }));
    assert!(actions
        .iter()
        .any(|a| matches!(a, IdeAction::FetchDiff { .. })));
    assert!(actions
        .iter()
        .any(|a| matches!(a, IdeAction::RefreshChanges)));
}

#[test]
fn approve_and_deny_echo_the_prompt_seq() {
    let mut app = app();
    app.apply(IdeEvent::Snapshot(
        "b".into(),
        crate::board::LiveSnapshot {
            session_id: "b".into(),
            permission_prompt: Some("run write_file?".into()),
            prompt_seq: 7,
            ..Default::default()
        },
    ));
    let s = screen(&mut app, 200, 40);
    assert!(s.contains("needs your permission"), "{s}");
    let allow = press(&mut app, 'Y');
    assert_eq!(
        allow,
        vec![IdeAction::Input(
            "b".into(),
            serde_json::json!({"kind":"allow","yes":true,"seq":7})
        )]
    );
    let deny = press(&mut app, 'N');
    assert!(matches!(&deny[0], IdeAction::Input(_, v) if v["yes"] == false && v["seq"] == 7));
}

#[test]
fn the_composer_sends_a_prompt() {
    let mut app = app();
    press(&mut app, 'm');
    for c in "hi".chars() {
        press(&mut app, c);
    }
    let sent = handle_key(&mut app, key(KeyCode::Enter));
    assert!(matches!(&sent[0], IdeAction::Input(id, v) if id == "b" && v["text"] == "hi"));
}

#[test]
fn f2_and_esc_close() {
    let mut app = app();
    assert_eq!(
        handle_key(&mut app, key(KeyCode::F(2))),
        vec![IdeAction::Close]
    );
    assert_eq!(
        handle_key(&mut app, key(KeyCode::Esc)),
        vec![IdeAction::Close]
    );
}

#[test]
fn selecting_an_agent_scopes_the_queue_and_watches_only_it() {
    let mut app = app();
    let actions = app.toggle_scope("a");
    assert_eq!(app.rows.len(), 2);
    assert!(actions.contains(&IdeAction::Watch(Some("a".into()))));
    app.clear_scope();
    assert_eq!(app.rows.len(), 3);
}

#[test]
fn a_five_thousand_line_diff_draws_quickly() {
    let mut app = app();
    open_first(&mut app);
    let big = WireDiffFile {
        path: "lib.rs".into(),
        kind: "modified".into(),
        hunks: (0..100)
            .map(|i| WireHunk {
                header: format!("@@ -{0},50 +{0},50 @@", i * 60 + 1),
                lines: (0..50)
                    .map(|n| {
                        if n % 2 == 0 {
                            format!("+line {n}")
                        } else {
                            format!(" ctx {n}")
                        }
                    })
                    .collect(),
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    };
    app.apply(IdeEvent::Diff(DiffLoaded {
        session_id: "b".into(),
        path: "lib.rs".into(),
        bucket: hunk::Bucket::Unstaged,
        file: Ok(Some(big)),
    }));
    screen(&mut app, 200, 50);
    let mut term = Terminal::new(TestBackend::new(200, 50)).unwrap();
    let started = std::time::Instant::now();
    for _ in 0..20 {
        press(&mut app, 'n');
        term.draw(|f| draw(&mut app, f)).unwrap();
    }
    let per = started.elapsed() / 20;
    eprintln!("ide frame with a 5,000-line diff: {per:?}");
    assert!(
        per < std::time::Duration::from_millis(16) * if cfg!(debug_assertions) { 6 } else { 1 }
    );
}
