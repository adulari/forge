use ratatui::backend::TestBackend;
use ratatui::Terminal;

use super::*;

fn row(id: &str, name: &str, outdated: bool, command: Option<&str>) -> AgentRow {
    AgentRow {
        id: id.into(),
        name: name.into(),
        installed: command.is_some(),
        version: Some("2.0.0".into()),
        latest: Some("2.1.0".into()),
        outdated,
        command: command.map(str::to_string),
        hint: command
            .is_none()
            .then(|| "install it from its download page".to_string()),
        ..Default::default()
    }
}

fn menu() -> AgentsMenu {
    let mut m = AgentsMenu::default();
    m.open();
    m.apply_rows(Ok(vec![
        row("claude-cli", "Claude Code", true, Some("claude update")),
        row("codex-cli", "OpenAI Codex", false, Some("codex update")),
        row("agy-cli", "Google Antigravity", false, None),
    ]));
    m
}

fn screen(m: &AgentsMenu, w: u16, h: u16) -> Vec<String> {
    let mut app = App::default();
    app.agents_menu = m.clone();
    let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
    t.draw(|f| {
        let a = f.area();
        render_agents_menu(f, &app, Rect::new(0, 1, a.width, a.height - 5));
    })
    .unwrap();
    let b = t.backend().buffer();
    (0..h)
        .map(|y| (0..w).map(|x| b[(x, y)].symbol()).collect())
        .collect()
}

#[test]
fn enter_asks_first_and_cancel_is_the_default() {
    let mut m = menu();
    assert_eq!(m.key(KeyKind::Enter), MenuAction::None);
    let text = screen(&m, 100, 30).join("\n");
    assert!(text.contains("$ claude update"), "{text}");
    // Enter on the default (Cancel) runs nothing.
    assert_eq!(m.key(KeyKind::Enter), MenuAction::None);
    assert!(!m.running());
}

#[test]
fn confirming_runs_exactly_the_shown_agent() {
    let mut m = menu();
    m.key(KeyKind::Enter);
    assert_eq!(
        m.key(KeyKind::Char('y')),
        MenuAction::Run(vec!["claude-cli".into()])
    );
    assert!(m.running());
    assert_eq!(
        m.key(KeyKind::Enter),
        MenuAction::None,
        "no second job while one runs"
    );
}

#[test]
fn update_all_lists_only_outdated_agents() {
    let mut m = menu();
    m.key(KeyKind::Char('U'));
    m.key(KeyKind::Tab);
    assert_eq!(
        m.key(KeyKind::Enter),
        MenuAction::Run(vec!["claude-cli".into()])
    );
    let mut current = AgentsMenu::default();
    current.open();
    current.apply_rows(Ok(vec![row(
        "codex-cli",
        "OpenAI Codex",
        false,
        Some("codex update"),
    )]));
    current.key(KeyKind::Char('U'));
    assert!(current.notice.as_deref().unwrap().contains("up to date"));
}

#[test]
fn a_curl_pipe_installer_is_flagged_in_the_confirmation() {
    let mut m = AgentsMenu::default();
    m.open();
    let mut r = row(
        "claude-cli",
        "Claude Code",
        false,
        Some("curl -fsSL https://claude.ai/install.sh | bash"),
    );
    r.installed = false;
    m.apply_rows(Ok(vec![r]));
    m.key(KeyKind::Enter);
    let text = screen(&m, 100, 30).join("\n");
    assert!(
        text.contains("curl -fsSL https://claude.ai/install.sh | bash"),
        "{text}"
    );
    assert!(text.contains("Downloads a script"), "{text}");
}

#[test]
fn rows_without_a_command_show_their_hint_and_never_confirm() {
    let mut m = menu();
    m.key(KeyKind::Char('j'));
    m.key(KeyKind::Char('j'));
    assert_eq!(m.key(KeyKind::Enter), MenuAction::None);
    assert!(m.notice.as_deref().unwrap().contains("download page"));
}

#[test]
fn list_fits_a_small_terminal() {
    let mut m = menu();
    m.push_log("$ claude update".into());
    let rows = screen(&m, 80, 24);
    let text = rows.join("\n");
    assert!(
        text.contains("Claude Code") && text.contains("-> 2.1.0 available"),
        "{text}"
    );
    assert!(text.contains("1 agent is outdated"));
}
