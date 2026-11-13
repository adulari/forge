use super::*;
use crate::app::App;

fn row(n: u64, state: &str, position: Option<usize>) -> QueueRow {
    QueueRow {
        session_id: format!("s{n}"),
        repo: "git@host:me/repo.git @ main".into(),
        pr: Some(n),
        position,
        state: state.into(),
        reason: match state {
            "queued" => format!("#2 in the merge queue, behind PR #{}", n - 1),
            "skipped" => "conflicts in src/a.rs".into(),
            "held" => "held by you".into(),
            _ => String::new(),
        },
        pushes: 0,
        review: String::new(),
    }
}

fn ready() -> QueueMenu {
    let mut menu = QueueMenu::default();
    menu.open();
    assert!(menu.take_refresh());
    menu.apply_list(Ok(vec![
        row(7, "head", Some(1)),
        row(8, "queued", Some(2)),
        row(9, "held", None),
        row(10, "skipped", None),
        row(6, "merged", None),
    ]));
    menu
}

#[test]
fn keys_ask_for_the_action_that_fits_the_focused_pr() {
    let mut menu = ready();
    menu.key(KeyKind::Down);
    assert_eq!(
        menu.key(KeyKind::Char('f')),
        QueueAction::Act {
            session_id: "s8".into(),
            op: QueueOp::Front
        }
    );
    assert!(menu.busy);
    assert_eq!(
        menu.key(KeyKind::Char('h')),
        QueueAction::None,
        "no double send"
    );
    menu.finish_action(Ok((
        "PR #8 moved to the front".into(),
        vec![row(8, "head", Some(1))],
    )));
    assert!(!menu.busy);
    assert_eq!(menu.notice.as_ref().unwrap().tone, Tone::Ok);
    assert_eq!(menu.rows.len(), 1);
}

#[test]
fn an_action_a_row_cannot_take_is_explained_not_sent() {
    let mut menu = ready();
    let cases = [
        (0, 'r', "PR #7 is not held"),
        (0, 'f', "PR #7 is already next"),
        (0, 't', "nothing to retry"),
        (2, 'h', "PR #9 is already held"),
        (4, 'h', "PR #6 has left the queue"),
    ];
    for (at, key, expect) in cases {
        menu.cursor = at;
        assert_eq!(menu.key(KeyKind::Char(key)), QueueAction::None);
        let note = menu.notice.take().unwrap();
        assert_eq!(note.tone, Tone::Warn);
        assert!(note.text.contains(expect), "{}", note.text);
    }
    assert!(!menu.busy);
}

#[test]
fn enter_holds_a_waiting_pr_releases_a_held_one_and_retries_a_handed_back_one() {
    let mut menu = ready();
    for (at, session, op) in [
        (1, "s8", QueueOp::Hold),
        (2, "s9", QueueOp::Release),
        (3, "s10", QueueOp::Retry),
    ] {
        menu.cursor = at;
        assert_eq!(
            menu.key(KeyKind::Enter),
            QueueAction::Act {
                session_id: session.into(),
                op
            }
        );
        menu.finish_action(Err("daemon said no".into()));
        assert_eq!(menu.notice.as_ref().unwrap().tone, Tone::Err);
    }
}

#[test]
fn the_cursor_follows_its_pr_when_the_list_reorders() {
    let mut menu = ready();
    menu.cursor = 1;
    menu.apply_list(Ok(vec![row(8, "head", Some(1)), row(7, "queued", Some(2))]));
    assert_eq!(menu.rows[menu.cursor].session_id, "s8");
    menu.apply_list(Ok(vec![]));
    assert_eq!(menu.cursor, 0);
}

#[test]
fn esc_closes_and_r_capital_reloads() {
    let mut menu = ready();
    menu.key(KeyKind::Char('R'));
    assert!(menu.take_refresh());
    assert_eq!(menu.load, Load::Loading);
    assert_eq!(menu.key(KeyKind::Esc), QueueAction::Close);
    assert!(!menu.open);
}

fn drawn(menu: QueueMenu, w: u16, h: u16) -> String {
    let mut app = App::default();
    app.queue_menu = menu;
    let backend = ratatui::backend::TestBackend::new(w, h);
    let mut terminal = ratatui::Terminal::new(backend).unwrap();
    terminal
        .draw(|f| render_queue_menu(f, &app, f.area()))
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    (0..h)
        .map(|y| {
            (0..w)
                .map(|x| buffer[(x, y)].symbol().to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn the_card_lists_position_pr_state_and_reason_per_repository() {
    let text = drawn(ready(), 100, 20);
    assert!(text.contains("Merge queue"), "{text}");
    assert!(text.contains("2 waiting"), "{text}");
    assert!(text.contains("git@host:me/repo.git @ main"), "{text}");
    assert!(
        text.contains("#1") && text.contains("PR #7") && text.contains("next to land"),
        "{text}"
    );
    assert!(text.contains("behind PR #7"), "{text}");
    assert!(
        text.contains("held")
            && text.contains("handed back")
            && text.contains("conflicts in src/a.rs"),
        "{text}"
    );
    assert!(text.contains("merged"), "{text}");
}

#[test]
fn an_empty_queue_says_how_to_fill_it() {
    let mut menu = QueueMenu::default();
    menu.open();
    menu.apply_list(Ok(vec![]));
    assert!(drawn(menu, 100, 12).contains("No PRs are queued"));
}

#[test]
fn a_rows_review_verdict_shows_beside_its_reason() {
    let mut menu = QueueMenu::default();
    menu.open();
    assert!(menu.take_refresh());
    let mut head = row(7, "head", Some(1));
    head.review = "review: 2 confirmed, fixing".into();
    let mut clean = row(8, "queued", Some(2));
    clean.review = "review clean".into();
    menu.apply_list(Ok(vec![head, clean]));
    let text = drawn(menu, 120, 14);
    assert!(text.contains("[review: 2 confirmed, fixing]"), "{text}");
    assert!(text.contains("[review clean]"), "{text}");
}

fn reviewed(text: &str) -> QueueMenu {
    let mut menu = ready();
    menu.rows[0].review = text.into();
    menu
}

#[test]
fn merge_anyway_asks_twice_before_it_acts() {
    let mut menu = reviewed("review: 2 confirmed, fixing");
    assert_eq!(menu.key(KeyKind::Char('m')), QueueAction::None);
    assert!(menu.notice.as_ref().unwrap().text.contains("Press m again"));
    assert_eq!(
        menu.key(KeyKind::Char('m')),
        QueueAction::Act {
            session_id: "s7".into(),
            op: QueueOp::MergeAnyway
        }
    );
    assert_eq!(QueueOp::MergeAnyway.wire(), "merge_anyway");
    assert!(QueueOp::MergeAnyway.is_review() && !QueueOp::Hold.is_review());
}

#[test]
fn another_key_cancels_the_merge_anyway_confirmation() {
    let mut menu = reviewed("review running");
    menu.key(KeyKind::Char('m'));
    menu.key(KeyKind::Down);
    menu.key(KeyKind::Up);
    assert_eq!(
        menu.key(KeyKind::Char('m')),
        QueueAction::None,
        "asks again"
    );
}

#[test]
fn overrides_are_explained_when_nothing_holds_the_pr() {
    for review in [
        "",
        "review clean",
        "review clean (1 dismissed)",
        "review overridden by tui",
    ] {
        let mut menu = reviewed(review);
        assert_eq!(menu.key(KeyKind::Char('m')), QueueAction::None);
        assert!(
            menu.notice
                .as_ref()
                .unwrap()
                .text
                .contains("nothing to override"),
            "{review}"
        );
    }
    let mut menu = reviewed("review running");
    assert_eq!(menu.key(KeyKind::Char('d')), QueueAction::None);
    assert!(menu
        .notice
        .as_ref()
        .unwrap()
        .text
        .contains("no confirmed finding"));
}

#[test]
fn d_dismisses_a_confirmed_finding_in_one_press() {
    let mut menu = reviewed("review: 1 confirmed");
    assert_eq!(
        menu.key(KeyKind::Char('d')),
        QueueAction::Act {
            session_id: "s7".into(),
            op: QueueOp::Dismiss
        }
    );
}
