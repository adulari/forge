use super::*;

fn parent_with_child(running: bool) -> App {
    let mut app = App {
        fullscreen: true,
        ..App::default()
    };
    for i in 0..60 {
        app.submit_user(&format!("parent question {i}"));
        app.push_turn_lines(crate::render::markdown_to_lines(&format!(
            "parent answer {i}"
        )));
    }
    app.drain_flush();
    app.transcript_to_bottom();
    app.apply(PresenterEvent::SubagentStart {
        id: "child-1".into(),
        agent: "explore".into(),
        task: "map the repo".into(),
        model: Some("opencode::deepseek-v4".into()),
        phase: None,
    });
    if !running {
        app.apply(PresenterEvent::SubagentResult {
            id: "child-1".into(),
            agent: "explore".into(),
            ok: true,
            summary: "mapped".into(),
            cost_usd: 0.0,
        });
    }
    app
}

fn child_items(extra: usize) -> Vec<ReplayItem> {
    let mut items = vec![
        ReplayItem::User("map the repo".into()),
        ReplayItem::Tool {
            name: "read_file".into(),
            args: r#"{"path":"src/main.rs"}"#.into(),
        },
        ReplayItem::ToolResult {
            name: "read_file".into(),
            ok: true,
            summary: "fn main() {}".into(),
        },
        ReplayItem::Assistant("The **entry point** is src/main.rs".into()),
    ];
    for i in 0..extra {
        items.push(ReplayItem::Assistant(format!("extra finding {i}")));
    }
    items
}

fn text(rows: &[String]) -> String {
    rows.join("\n")
}

#[test]
fn opening_a_subagent_shows_its_transcript_with_a_breadcrumb() {
    for (w, h) in [(80, 24), (120, 40)] {
        let mut app = parent_with_child(true);
        assert_eq!(app.subagent_session_at(1).as_deref(), Some("child-1"));
        assert!(app.open_child_view("child-1", &child_items(0)));
        let rows = screen_rows(&app, w, h);
        let screen = text(&rows);
        assert!(screen.contains("main chat"), "{w}x{h} breadcrumb\n{screen}");
        assert!(screen.contains("explore"), "{w}x{h}\n{screen}");
        assert!(screen.contains("deepseek-v4"), "{w}x{h}\n{screen}");
        assert!(screen.contains("running"), "{w}x{h}\n{screen}");
        assert!(screen.contains("map the repo"), "{w}x{h}\n{screen}");
        assert!(screen.contains("entry point"), "{w}x{h}\n{screen}");
        assert!(
            screen.contains("read_file") || screen.contains("Read"),
            "{screen}"
        );
        assert!(!screen.contains("parent answer"), "{w}x{h}\n{screen}");
        assert!(screen.contains("Esc"), "{w}x{h} back hint\n{screen}");
    }
}

#[test]
fn a_finished_child_reads_done() {
    let mut app = parent_with_child(false);
    assert!(app.open_child_view("child-1", &child_items(0)));
    assert!(text(&screen_rows(&app, 80, 24)).contains("done"));
    assert!(!app.child_view_running());
}

#[test]
fn unknown_ids_do_not_open() {
    let mut app = parent_with_child(true);
    assert!(!app.open_child_view("nope", &child_items(0)));
    assert!(app.child_view_id().is_none());
    assert!(!app.child_view_key(KeyKind::Esc));
}

#[test]
fn esc_returns_to_the_parent_exactly_where_it_was() {
    let mut app = parent_with_child(true);
    let before = screen_rows(&app, 80, 24);
    app.transcript_scroll_up(7);
    let _ = screen_rows(&app, 80, 24);
    let scroll = (app.transcript_scroll, app.transcript_follow);
    let scrolled = screen_rows(&app, 80, 24);
    assert_ne!(before, scrolled);

    assert!(app.open_child_view("child-1", &child_items(40)));
    let _ = screen_rows(&app, 80, 24);
    assert!(app.child_view_key(KeyKind::PageUp));
    assert!(app.child_view_key(KeyKind::Home));
    assert!(app.child_view_key(KeyKind::Esc));

    assert!(app.child_view_id().is_none());
    assert_eq!((app.transcript_scroll, app.transcript_follow), scroll);
    assert_eq!(screen_rows(&app, 80, 24), scrolled);
}

#[test]
fn the_child_scrolls_independently_of_the_parent() {
    let mut app = parent_with_child(true);
    assert!(app.open_child_view("child-1", &child_items(80)));
    let _ = screen_rows(&app, 80, 24);
    let tail = text(&screen_rows(&app, 80, 24));
    assert!(tail.contains("extra finding 79"), "{tail}");
    app.child_view_key(KeyKind::Home);
    let top = text(&screen_rows(&app, 80, 24));
    assert!(top.contains("map the repo"), "{top}");
    assert!(!top.contains("extra finding 79"));
    app.child_view_key(KeyKind::End);
    assert!(text(&screen_rows(&app, 80, 24)).contains("extra finding 79"));
    assert!(app.transcript_follow, "parent never left the tail");
}

#[test]
fn live_messages_append_and_a_following_view_stays_on_the_tail() {
    let mut app = parent_with_child(true);
    assert!(app.open_child_view("child-1", &child_items(60)));
    let _ = screen_rows(&app, 80, 24);
    assert!(!text(&screen_rows(&app, 80, 24)).contains("a late finding"));
    let mut items = child_items(60);
    items.push(ReplayItem::Assistant("a late finding".into()));
    app.refresh_child_view(&items);
    let screen = text(&screen_rows(&app, 80, 24));
    assert!(screen.contains("a late finding"), "{screen}");
}

#[test]
fn a_scrolled_up_view_keeps_its_place_when_the_child_grows() {
    let mut app = parent_with_child(true);
    assert!(app.open_child_view("child-1", &child_items(80)));
    let _ = screen_rows(&app, 80, 24);
    app.child_view_key(KeyKind::Home);
    let _ = screen_rows(&app, 80, 24);
    let mut items = child_items(80);
    items.push(ReplayItem::Assistant("a late finding".into()));
    app.refresh_child_view(&items);
    let screen = text(&screen_rows(&app, 80, 24));
    assert!(screen.contains("map the repo"), "{screen}");
    assert!(!screen.contains("a late finding"));
}

#[test]
fn an_empty_child_says_it_is_waiting() {
    let mut app = parent_with_child(true);
    assert!(app.open_child_view("child-1", &[]));
    assert!(text(&screen_rows(&app, 80, 24)).contains("waiting for the agent"));
}

#[test]
fn the_agents_panel_hints_how_to_open_one() {
    let mut app = parent_with_child(true);
    app.activity_focused = true;
    app.activity_idx = 1;
    let screen = text(&screen_rows(&app, 100, 30));
    assert!(
        screen.contains("Enter") && screen.contains("open"),
        "{screen}"
    );
}
