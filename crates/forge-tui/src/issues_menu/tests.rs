use super::*;

fn rows() -> Vec<IssueRow> {
    (1..=3)
        .map(|n| IssueRow {
            number: n,
            title: format!("issue {n}"),
            ..IssueRow::default()
        })
        .collect()
}

fn ready() -> IssuesMenu {
    let mut menu = IssuesMenu::default();
    menu.open();
    assert!(menu.take_refresh());
    menu.apply_list(Ok(rows()));
    menu
}

#[test]
fn enter_starts_the_focused_issue_with_the_chosen_options() {
    let mut menu = ready();
    menu.key(KeyKind::Down);
    menu.key(KeyKind::Tab);
    menu.key(KeyKind::Char('w'));
    assert_eq!(
        menu.key(KeyKind::Enter),
        IssuesAction::Start {
            number: 2,
            worktree: false,
            watch: WatchChoice::Merge
        }
    );
    assert!(menu.starting);
    assert_eq!(
        menu.key(KeyKind::Enter),
        IssuesAction::None,
        "no double start"
    );
    menu.finish_start(Ok("started".into()));
    assert!(!menu.starting);
    assert_eq!(menu.notice.as_ref().unwrap().tone, Tone::Ok);
}

#[test]
fn defaults_are_a_worktree_and_fix_watch() {
    let mut menu = ready();
    assert_eq!(
        menu.key(KeyKind::Enter),
        IssuesAction::Start {
            number: 1,
            worktree: true,
            watch: WatchChoice::Fix
        }
    );
}

#[test]
fn cursor_stays_in_range_and_refresh_reloads() {
    let mut menu = ready();
    for _ in 0..10 {
        menu.key(KeyKind::Down);
    }
    assert_eq!(menu.cursor, 2);
    menu.key(KeyKind::Home);
    assert_eq!(menu.cursor, 0);
    menu.key(KeyKind::Char('r'));
    assert_eq!(menu.load, Load::Loading);
    assert!(menu.take_refresh());
    menu.apply_list(Ok(vec![]));
    assert_eq!(menu.key(KeyKind::Enter), IssuesAction::None);
}

#[test]
fn watch_cycles_and_escape_closes() {
    let mut menu = ready();
    for expected in [WatchChoice::Merge, WatchChoice::Off, WatchChoice::Fix] {
        menu.key(KeyKind::Tab);
        assert_eq!(menu.watch, expected);
    }
    assert_eq!(menu.key(KeyKind::Esc), IssuesAction::Close);
    assert!(!menu.open);
}

fn drain_view(status: &str) -> DrainView {
    DrainView {
        id: "drain-1".into(),
        status: status.into(),
        line: "Drain: 2 started, 1 queued".into(),
        items: vec![
            (1, "running".into()),
            (2, "pr_open".into()),
            (3, "queued".into()),
        ],
        ..DrainView::default()
    }
}

#[test]
fn space_marks_and_advances_and_d_drains_the_marked_in_order() {
    let mut menu = ready();
    menu.key(KeyKind::Down);
    menu.key(KeyKind::Char(' '));
    assert_eq!(menu.marked, vec![2]);
    assert_eq!(menu.cursor, 2);
    menu.key(KeyKind::Home);
    menu.key(KeyKind::Char('m'));
    assert_eq!(menu.marked, vec![2, 1]);
    menu.key(KeyKind::Up);
    menu.key(KeyKind::Char('m'));
    assert_eq!(menu.marked, vec![2], "marking a marked issue clears it");

    menu.key(KeyKind::Char('+'));
    menu.key(KeyKind::Char('b'));
    menu.key(KeyKind::Char('b'));
    menu.key(KeyKind::Char('b'));
    assert_eq!(
        menu.key(KeyKind::Char('D')),
        IssuesAction::Drain {
            numbers: vec![2],
            worktree: true,
            watch: WatchChoice::Fix,
            parallel: 3,
            budget: Some("claude:30%w"),
        }
    );
    assert!(menu.starting);
    menu.finish_drain(Ok("queued".into()));
    assert!(menu.marked.is_empty(), "a queued drain clears the marks");
}

#[test]
fn draining_with_nothing_marked_says_so_and_failure_keeps_the_marks() {
    let mut menu = ready();
    assert_eq!(menu.key(KeyKind::Char('D')), IssuesAction::None);
    assert_eq!(menu.notice.as_ref().unwrap().tone, Tone::Warn);
    assert!(!menu.starting);
    menu.key(KeyKind::Char(' '));
    menu.key(KeyKind::Char('D'));
    menu.finish_drain(Err("no fresh usage reading".into()));
    assert_eq!(menu.marked, vec![1]);
    menu.key(KeyKind::Char('u'));
    assert!(menu.marked.is_empty());
}

#[test]
fn parallel_stays_between_one_and_the_cap() {
    let mut menu = ready();
    for _ in 0..12 {
        menu.key(KeyKind::Char('+'));
    }
    assert_eq!(menu.parallel, MAX_PARALLEL);
    for _ in 0..12 {
        menu.key(KeyKind::Char('-'));
    }
    assert_eq!(menu.parallel, 1);
}

#[test]
fn stopping_a_live_drain_takes_two_presses_and_other_keys_disarm_it() {
    let mut menu = ready();
    assert_eq!(
        menu.key(KeyKind::Char('x')),
        IssuesAction::None,
        "no drain yet"
    );
    menu.apply_drains(vec![drain_view("finished"), drain_view("active")]);
    assert_eq!(
        menu.drain.as_ref().unwrap().status,
        "active",
        "the live one wins"
    );
    assert_eq!(menu.key(KeyKind::Char('x')), IssuesAction::None);
    assert!(menu.confirm_cancel);
    menu.key(KeyKind::Down);
    assert!(!menu.confirm_cancel);
    menu.key(KeyKind::Char('x'));
    assert_eq!(
        menu.key(KeyKind::Char('x')),
        IssuesAction::CancelDrain {
            id: "drain-1".into()
        }
    );
    menu.apply_drains(vec![drain_view("cancelled")]);
    assert_eq!(
        menu.key(KeyKind::Char('x')),
        IssuesAction::None,
        "nothing live to stop"
    );
}

#[test]
fn drain_items_name_their_state_per_issue() {
    let view = drain_view("active");
    assert_eq!(view.item_label(1), Some("running"));
    assert_eq!(view.item_label(2), Some("PR open"));
    assert_eq!(view.item_label(3), Some("queued"));
    assert_eq!(view.item_label(9), None);
}
