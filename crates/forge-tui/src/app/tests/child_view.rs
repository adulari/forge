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
    assert_eq!(app.child_view_key(KeyKind::Esc), ChildKey::Inactive);
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
    assert_eq!(app.child_view_key(KeyKind::PageUp), ChildKey::Handled);
    assert_eq!(app.child_view_key(KeyKind::JumpPrevUser), ChildKey::Handled);
    assert_eq!(app.child_view_key(KeyKind::Esc), ChildKey::Handled);

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
    app.child_view_key(KeyKind::JumpPrevUser);
    let top = text(&screen_rows(&app, 80, 24));
    assert!(top.contains("map the repo"), "{top}");
    assert!(!top.contains("extra finding 79"));
    app.child_view_key(KeyKind::JumpBottom);
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
    app.child_view_key(KeyKind::JumpPrevUser);
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

fn two_children(running: bool) -> App {
    let mut app = parent_with_child(running);
    app.apply(PresenterEvent::SubagentStart {
        id: "child-2".into(),
        agent: "review".into(),
        task: "check the tests".into(),
        model: None,
        phase: None,
    });
    app
}

fn press(app: &mut App, keys: &[KeyKind]) {
    for k in keys {
        match app.activity_key(*k) {
            PanelKey::OpenChild(id) => {
                assert!(app.open_child_view(&id, &child_items(0)));
            }
            PanelKey::Unhandled => {
                app.child_view_key(*k);
            }
            _ => {}
        }
    }
}

#[test]
fn ctrl_o_down_enter_opens_the_session_view_not_the_pager() {
    let mut app = parent_with_child(false);
    app.fullscreen = true;
    press(
        &mut app,
        &[KeyKind::ToggleSubagentDetail, KeyKind::Down, KeyKind::Enter],
    );
    assert_eq!(app.child_view_id(), Some("child-1"));
    assert!(app.viewer.is_none(), "the old pager must not open");
    let _ = screen_rows(&app, 100, 30);
    let screen = text(&screen_rows(&app, 100, 30));
    assert!(screen.contains("main chat \u{203a} explore"), "{screen}");
    assert!(
        screen.contains("\u{258e} you"),
        "task as a you block\n{screen}"
    );
    assert!(screen.contains("map the repo"), "{screen}");
    assert!(screen.contains("\u{258f}"), "assistant gutter\n{screen}");
    assert!(screen.contains("entry point"), "{screen}");
    assert!(
        !screen.contains("**entry point**"),
        "markdown rendered\n{screen}"
    );
    assert!(
        screen.contains("Message explore"),
        "composer names the agent\n{screen}"
    );
    assert!(!screen.contains("parent answer"), "{screen}");
}

#[test]
fn enter_on_the_main_chat_row_only_hides_the_panel() {
    let mut app = parent_with_child(false);
    press(&mut app, &[KeyKind::ToggleSubagentDetail, KeyKind::Enter]);
    assert!(app.child_view_id().is_none());
    assert!(app.viewer.is_none());
    assert!(!app.activity_focused);
}

#[test]
fn left_and_right_cycle_through_the_main_chat_and_every_agent() {
    let mut app = two_children(false);
    assert!(app.open_child_view("child-1", &child_items(0)));
    assert_eq!(
        app.child_view_key(KeyKind::Right),
        ChildKey::Switch("child-2".into())
    );
    assert!(app.open_child_view("child-2", &[]));
    assert_eq!(
        app.child_view_key(KeyKind::Left),
        ChildKey::Switch("child-1".into())
    );
    assert!(app.open_child_view("child-1", &[]));
    assert_eq!(app.child_view_key(KeyKind::Left), ChildKey::Handled);
    assert!(
        app.child_view_id().is_none(),
        "before the first agent is the main chat"
    );
    assert!(app.open_child_view("child-2", &[]));
    assert_eq!(app.child_view_key(KeyKind::Right), ChildKey::Handled);
    assert!(
        app.child_view_id().is_none(),
        "past the last agent is the main chat"
    );
}

#[test]
fn each_view_keeps_its_scroll_across_a_switch() {
    let mut app = two_children(false);
    assert!(app.open_child_view("child-1", &child_items(80)));
    let _ = screen_rows(&app, 80, 24);
    app.child_view_key(KeyKind::JumpPrevUser);
    let _ = screen_rows(&app, 80, 24);
    assert!(app.open_child_view("child-2", &[]));
    assert!(app.open_child_view("child-1", &child_items(80)));
    let screen = text(&screen_rows(&app, 80, 24));
    assert!(screen.contains("map the repo"), "back at the top\n{screen}");
    assert!(!screen.contains("extra finding 79"), "{screen}");
}

#[test]
fn arrows_move_the_cursor_instead_of_switching_while_a_message_is_typed() {
    let mut app = two_children(false);
    assert!(app.open_child_view("child-1", &child_items(0)));
    for c in "hey".chars() {
        assert_eq!(app.child_view_key(KeyKind::Char(c)), ChildKey::Handled);
    }
    assert_eq!(app.child_view_key(KeyKind::Left), ChildKey::Handled);
    assert_eq!(app.child_view_id(), Some("child-1"));
    assert_eq!(app.input, "hey");
    assert!(text(&screen_rows(&app, 100, 30)).contains("hey"));
}

#[test]
fn typing_and_enter_submit_to_the_open_child() {
    let mut app = two_children(false);
    assert!(app.open_child_view("child-1", &child_items(0)));
    for c in "and what about sub?".chars() {
        app.child_view_key(KeyKind::Char(c));
    }
    assert_eq!(
        app.child_view_key(KeyKind::Enter),
        ChildKey::Submit("and what about sub?".into())
    );
    assert!(app.input.is_empty());
    assert_eq!(
        app.child_view_key(KeyKind::Enter),
        ChildKey::Handled,
        "empty Enter sends nothing"
    );
    assert_eq!(app.child_view_agent().as_deref(), Some("explore"));
}

#[test]
fn a_running_child_or_busy_chat_is_named_in_the_composer() {
    let mut app = parent_with_child(true);
    assert!(app.open_child_view("child-1", &child_items(0)));
    assert!(text(&screen_rows(&app, 100, 30)).contains("explore is working"));
    let mut app = parent_with_child(false);
    app.busy = true;
    assert!(app.open_child_view("child-1", &child_items(0)));
    assert!(text(&screen_rows(&app, 100, 30)).contains("a turn is running"));
}

#[test]
fn a_notice_shows_in_the_header_until_the_next_key() {
    let mut app = parent_with_child(false);
    assert!(app.open_child_view("child-1", &child_items(0)));
    app.child_view_notice("explore is still working");
    assert!(text(&screen_rows(&app, 100, 30)).contains("explore is still working"));
    app.child_view_key(KeyKind::Char('x'));
    assert!(!text(&screen_rows(&app, 100, 30)).contains("explore is still working"));
}

#[test]
fn a_follow_up_reopens_the_row_and_the_view_reads_running() {
    let mut app = parent_with_child(false);
    assert!(app.open_child_view("child-1", &child_items(0)));
    assert!(!app.child_view_running());
    app.reopen_subagent("child-1");
    assert!(app.child_view_running());
    app.apply(PresenterEvent::SubagentResult {
        id: "child-1".into(),
        agent: "explore".into(),
        ok: true,
        summary: "again".into(),
        cost_usd: 0.0,
    });
    assert!(
        !app.child_view_running(),
        "the second result must settle the row"
    );
}

#[test]
fn the_you_block_spans_the_terminal_even_when_opened_before_any_child_frame() {
    let mut app = parent_with_child(false);
    let _ = screen_rows(&app, 120, 30);
    assert!(app.open_child_view("child-1", &child_items(0)));
    let screen = text(&screen_rows(&app, 120, 30));
    assert!(
        screen.contains("\u{258e} map the repo"),
        "task on one row, not wrapped narrow\n{screen}"
    );
}

#[test]
fn a_permission_prompt_takes_the_screen_back_from_the_view() {
    let mut app = parent_with_child(false);
    assert!(app.open_child_view("child-1", &child_items(0)));
    app.prompt = Some("write_file".into());
    assert_eq!(app.child_view_key(KeyKind::Char('y')), ChildKey::Inactive);
}
