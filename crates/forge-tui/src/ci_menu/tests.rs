use super::*;

fn rows() -> Vec<CiRow> {
    let row = |state, name: &str| CiRow {
        state,
        name: name.into(),
        detail: String::new(),
        secs: 0,
    };
    vec![
        row(CiState::Pass, "fmt"),
        row(CiState::Fail, "clippy"),
        row(CiState::Skip, "deploy"),
    ]
}

#[test]
fn opening_asks_for_the_plan_once() {
    let mut m = CiMenu::default();
    m.open();
    assert!(m.loading && m.take_refresh() && !m.take_refresh());
}

#[test]
fn rows_count_by_state_and_the_cursor_stays_in_range() {
    let mut m = CiMenu::default();
    m.open();
    m.cursor = 9;
    m.apply(rows(), Some("Last run".into()));
    assert_eq!(m.counts(), (1, 1, 1));
    assert_eq!(m.cursor, 2);
    assert!(!m.loading);
}

#[test]
fn r_runs_once_and_is_ignored_while_running() {
    let mut m = CiMenu::default();
    m.open();
    m.apply(rows(), None);
    assert_eq!(m.key(KeyKind::Char('r')), CiAction::Run);
    assert!(m.running);
    assert_eq!(m.key(KeyKind::Char('r')), CiAction::None);
    m.apply(rows(), None);
    assert!(!m.running);
}

#[test]
fn esc_closes_and_arrows_move() {
    let mut m = CiMenu::default();
    m.open();
    m.apply(rows(), None);
    m.key(KeyKind::Down);
    assert_eq!(m.cursor, 1);
    m.key(KeyKind::Home);
    assert_eq!(m.cursor, 0);
    assert_eq!(m.key(KeyKind::Esc), CiAction::Close);
    assert!(!m.open);
}
