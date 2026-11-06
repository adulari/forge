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
