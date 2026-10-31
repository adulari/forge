use ratatui::backend::TestBackend;
use ratatui::Terminal;

use super::*;
use crate::app::App;

const NOW: i64 = 1_793_390_000_000;

fn row(id: &str, command: &str, state: RunState, output: &[&str]) -> ActionRow {
    ActionRow {
        action: ActionView {
            id: id.into(),
            name: id.into(),
            command: command.into(),
            cwd: None,
        },
        state,
        output: output.iter().map(|s| s.to_string()).collect(),
    }
}

fn port(port: u16, host: &str, action: Option<&str>) -> PortView {
    PortView {
        port,
        host: host.into(),
        pid: 4242,
        process: "python3".into(),
        command: format!("python3 -m http.server {port}"),
        source: if action.is_some() {
            "action"
        } else {
            "workspace"
        }
        .into(),
        action: action.map(Into::into),
    }
}

fn menu() -> RunMenu {
    let mut m = RunMenu {
        project: "tui-ws".into(),
        ..Default::default()
    };
    m.open();
    m.advance(NOW);
    m.apply_actions(
        vec![
            row("dev", "python3 -m http.server 5173", RunState::Idle, &[]),
            row(
                "slow",
                "sleep 600",
                RunState::Running {
                    since_ms: NOW - 72_000,
                },
                &["\u{1b}[32mready\u{1b}[0m on :5173", "GET / 200"],
            ),
            row(
                "build",
                "cargo build",
                RunState::Exited {
                    code: Some(2),
                    stopped: false,
                },
                &["error: boom"],
            ),
        ],
        false,
    );
    m
}

#[test]
fn enter_runs_an_idle_action_and_stops_a_running_one() {
    let mut m = menu();
    assert_eq!(m.key(KeyKind::Enter), MenuAction::Start("dev".into()));
    m.key(KeyKind::Down);
    assert_eq!(m.key(KeyKind::Enter), MenuAction::Stop("slow".into()));
    assert_eq!(m.key(KeyKind::Char('s')), MenuAction::Stop("slow".into()));
    assert_eq!(
        m.key(KeyKind::Char('r')),
        MenuAction::Restart("slow".into())
    );
    m.key(KeyKind::Down);
    assert_eq!(m.key(KeyKind::Char('s')), MenuAction::None);
    assert_eq!(m.key(KeyKind::Enter), MenuAction::Start("build".into()));
}

#[test]
fn navigation_is_clamped() {
    let mut m = menu();
    m.key(KeyKind::Up);
    assert_eq!(m.cursor, 0);
    m.key(KeyKind::End);
    assert_eq!(m.cursor, 2);
    m.key(KeyKind::Down);
    assert_eq!(m.cursor, 2);
    m.key(KeyKind::Home);
    assert_eq!(m.cursor, 0);
}

#[test]
fn tab_switches_views_and_ports_copy_and_open() {
    let mut m = menu();
    m.apply_ports(vec![
        port(5173, "loopback", Some("dev")),
        port(8080, "all", None),
    ]);
    assert_eq!(
        m.key(KeyKind::Char('c')),
        MenuAction::None,
        "copy is a ports action"
    );
    m.key(KeyKind::Tab);
    assert_eq!(m.view, View::Ports);
    m.key(KeyKind::Down);
    assert_eq!(m.key(KeyKind::Char('c')), MenuAction::CopyUrl(8080));
    assert_eq!(m.key(KeyKind::Enter), MenuAction::CopyUrl(8080));
    assert_eq!(m.key(KeyKind::Char('o')), MenuAction::OpenUrl(8080));
    assert_eq!(m.key(KeyKind::Char('r')), MenuAction::None);
    m.key(KeyKind::Tab);
    assert_eq!(m.view, View::Actions);
    assert_eq!(m.cursor, 0);
}

#[test]
fn slash_filters_and_esc_clears_before_closing() {
    let mut m = menu();
    m.key(KeyKind::Char('/'));
    assert!(m.filtering);
    for c in "sleep".chars() {
        m.key(KeyKind::Char(c));
    }
    assert_eq!(m.visible_actions(), vec![1]);
    m.key(KeyKind::Enter);
    assert!(!m.filtering);
    assert_eq!(m.key(KeyKind::Enter), MenuAction::Stop("slow".into()));
    assert_eq!(m.key(KeyKind::Esc), MenuAction::None);
    assert!(m.open && m.filter.is_empty());
    assert_eq!(m.visible_actions().len(), 3);
    assert_eq!(m.key(KeyKind::Esc), MenuAction::Close);
    assert!(!m.open);
}

#[test]
fn letters_are_text_while_filtering() {
    let mut m = menu();
    m.key(KeyKind::Char('/'));
    assert_eq!(m.key(KeyKind::Char('r')), MenuAction::None);
    assert_eq!(m.filter, "r");
    m.key(KeyKind::Backspace);
    assert!(m.filter.is_empty());
}

#[test]
fn running_count_and_name_feed_the_status_badge() {
    let mut m = menu();
    assert_eq!(m.running_count(), 1);
    assert_eq!(m.sole_running_name(), Some("slow"));
    m.actions[0].state = RunState::Running { since_ms: NOW };
    assert_eq!(m.running_count(), 2);
    assert_eq!(m.sole_running_name(), None);
}

#[test]
fn refresh_is_requested_once_per_open() {
    let mut m = RunMenu::default();
    m.open();
    assert!(m.take_refresh());
    assert!(!m.take_refresh());
}

#[test]
fn elapsed_redraws_once_a_second_only_while_open() {
    let mut m = menu();
    assert!(!m.advance(NOW + 100));
    assert!(m.advance(NOW + 1100));
    m.close();
    assert!(!m.advance(NOW + 3000));
}

fn draw(m: RunMenu, w: u16, h: u16) -> Vec<String> {
    let mut app = App::default();
    app.run_menu = m;
    let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
    terminal
        .draw(|f| {
            let area = f.area();
            let region = ratatui::layout::Rect::new(0, 1, area.width, area.height - 5);
            render_run_menu(f, &app, region)
        })
        .unwrap();
    let buffer = terminal.backend().buffer();
    (0..h)
        .map(|y| (0..w).map(|x| buffer[(x, y)].symbol()).collect::<String>())
        .collect()
}

fn text(m: RunMenu, w: u16, h: u16) -> String {
    draw(m, w, h).join("\n")
}

#[test]
fn actions_render_at_80x24() {
    let out = text(menu(), 80, 24);
    for want in [
        "Run",
        "tui-ws",
        "1 running",
        "dev",
        "slow",
        "running 1m 12s",
        "exited 2",
        "Enter run",
        "ready on :5173",
    ] {
        assert!(
            out.contains(want) || want == "ready on :5173",
            "{want} missing:\n{out}"
        );
    }
    let mut m = menu();
    m.key(KeyKind::Down);
    let out = text(m, 80, 24);
    assert!(out.contains("slow output"), "{out}");
    assert!(
        out.contains("ready on :5173") && !out.contains("\u{1b}"),
        "{out}"
    );
    assert!(out.contains("Enter stop"));
}

#[test]
fn ports_render_with_detail() {
    let mut m = menu();
    m.apply_ports(vec![
        port(5173, "loopback", Some("dev")),
        port(8080, "all", None),
    ]);
    m.key(KeyKind::Tab);
    let out = text(m, 100, 30);
    for want in [
        ":5173",
        "python3",
        "action dev",
        "local only",
        "network",
        "http://localhost:5173",
        "c copy",
    ] {
        assert!(out.contains(want), "{want} missing:\n{out}");
    }
}

#[test]
fn wide_and_narrow_terminals_do_not_panic() {
    for (w, h) in [(160, 45), (50, 20), (30, 12), (20, 8), (80, 10)] {
        let mut m = menu();
        draw(m.clone(), w, h);
        m.apply_ports(vec![port(5173, "loopback", Some("dev"))]);
        m.key(KeyKind::Tab);
        draw(m.clone(), w, h);
        m.key(KeyKind::Char('/'));
        draw(m, w, h);
    }
}

#[test]
fn narrow_50_cols_keeps_name_and_status() {
    let out = text(menu(), 50, 20);
    assert!(out.contains("slow") && out.contains("running"), "{out}");
}

#[test]
fn empty_states_explain_themselves() {
    let mut m = RunMenu::default();
    m.open();
    let out = text(m.clone(), 80, 24);
    assert!(
        out.contains("[[actions]]") && out.contains(".forge/config.toml"),
        "{out}"
    );
    m.apply_actions(Vec::new(), true);
    let out = text(m.clone(), 80, 24);
    assert!(out.contains("forge trust"), "{out}");
    m.key(KeyKind::Tab);
    assert!(text(m.clone(), 80, 24).contains("Looking for listening ports"));
    m.apply_ports(Vec::new());
    assert!(text(m, 80, 24).contains("Nothing is listening yet"));
}

#[test]
fn no_match_filter_says_so() {
    let mut m = menu();
    m.key(KeyKind::Char('/'));
    for c in "zzz".chars() {
        m.key(KeyKind::Char(c));
    }
    assert!(text(m, 80, 24).contains("Nothing matches"));
}

#[test]
fn closed_menu_draws_nothing() {
    let mut m = menu();
    m.close();
    assert!(text(m, 80, 24).trim().is_empty());
}
