use super::*;

fn rows() -> Vec<CiRow> {
    let row = |state, name: &str| CiRow {
        state,
        name: name.into(),
        detail: String::new(),
        secs: 0,
        command: String::new(),
        env: Vec::new(),
        tail: String::new(),
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

fn failing() -> Vec<CiRow> {
    let mut v = rows();
    v[1].command = "cargo clippy --all-targets".into();
    v[1].env = vec!["RUSTFLAGS=-D warnings".into()];
    v[1].tail = (1..=30)
        .map(|n| format!("error line {n}"))
        .collect::<Vec<_>>()
        .join("\n");
    v
}

#[test]
fn enter_toggles_the_output_pane_and_never_starts_a_run() {
    let mut m = CiMenu::default();
    m.open();
    m.apply(rows(), None);
    assert_eq!(m.key(KeyKind::Enter), CiAction::None);
    assert!(m.expanded && !m.running);
    m.key(KeyKind::Enter);
    assert!(!m.expanded);
    m.key(KeyKind::Right);
    assert!(m.expanded);
    m.key(KeyKind::Left);
    assert!(!m.expanded);
}

#[test]
fn a_run_that_failed_lands_on_the_failing_step_with_its_output_open() {
    let mut m = CiMenu::default();
    m.open();
    m.apply(rows(), None);
    m.cursor = 0;
    assert_eq!(m.key(KeyKind::Char('r')), CiAction::Run);
    m.apply(failing(), None);
    assert_eq!(m.cursor, 1);
    assert!(m.expanded);
    let mut reloaded = CiMenu::default();
    reloaded.open();
    reloaded.apply(failing(), None);
    assert_eq!(
        reloaded.cursor, 0,
        "a plain load does not jump or open anything"
    );
    assert!(!reloaded.expanded);
}

fn screen(m: &CiMenu, w: u16, h: u16) -> Vec<String> {
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    let mut app = crate::app::App::default();
    app.ci_menu = m.clone();
    let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
    terminal
        .draw(|f| {
            let area = f.area();
            render::render_ci_menu(
                f,
                &app,
                ratatui::layout::Rect::new(0, 0, area.width, area.height),
            )
        })
        .unwrap();
    let buffer = terminal.backend().buffer();
    (0..h)
        .map(|y| (0..w).map(|x| buffer[(x, y)].symbol()).collect::<String>())
        .collect()
}

#[test]
fn the_open_pane_shows_command_env_and_the_last_lines_of_the_output() {
    let mut m = CiMenu::default();
    m.open();
    m.apply(rows(), None);
    m.running = true;
    m.apply(failing(), None);
    let text = screen(&m, 110, 40).join("\n");
    assert!(text.contains("$ cargo clippy --all-targets"), "{text}");
    assert!(text.contains("env  RUSTFLAGS=-D warnings"), "{text}");
    assert!(
        text.contains("error line 30"),
        "the end of the output is kept\n{text}"
    );
    assert!(
        !text.contains("error line 1\n") && !text.contains("error line 2 "),
        "{text}"
    );
    assert!(text.contains("Enter hide output"), "{text}");
}

#[test]
fn a_skipped_row_opens_to_its_whole_reason() {
    let mut m = CiMenu::default();
    m.open();
    let mut r = rows();
    r[2].detail = "network/deploy command (`npm publish`)".into();
    m.apply(r, None);
    m.cursor = 2;
    m.key(KeyKind::Enter);
    let text = screen(&m, 110, 30).join("\n");
    assert!(
        text.contains("Skipped: network/deploy command (`npm publish`)"),
        "{text}"
    );
}

#[test]
fn a_failing_step_with_no_captured_output_says_so() {
    let mut m = CiMenu::default();
    m.open();
    m.apply(rows(), None);
    m.cursor = 1;
    m.key(KeyKind::Enter);
    assert!(screen(&m, 110, 30)
        .join("\n")
        .contains("(no output captured)"));
}

#[test]
fn a_running_plan_names_the_step_in_flight() {
    let mut m = CiMenu::default();
    m.open();
    m.apply(rows(), None);
    m.running = true;
    m.progress = Some("2/3 lint / Unit tests".into());
    assert!(screen(&m, 110, 30)
        .join("\n")
        .contains("Running 2/3 lint / Unit tests"));
}
