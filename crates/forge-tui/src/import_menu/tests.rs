use ratatui::backend::TestBackend;
use ratatui::Terminal;

use super::*;
use crate::app::App;

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

fn row(agent: &str, id: &str, title: &str, cwd: Option<&str>) -> ImportRow {
    ImportRow {
        agent: agent.into(),
        id: id.into(),
        title: title.into(),
        cwd: cwd.map(str::to_string),
        cwd_exists: cwd.is_some(),
        modified: now() - 3 * 86_400,
        imported: false,
    }
}

fn menu() -> ImportMenu {
    let mut m = ImportMenu::default();
    m.open("/home/me".into());
    m.apply_scan(Ok(vec![
        row("claude", "c1", "fix the flaky test", Some("/work/api")),
        row("codex", "x1", "list the files", Some("/work/cli")),
        ImportRow {
            imported: true,
            ..row("claude", "c2", "already here", Some("/work/api"))
        },
        row("codex", "x2", "port the parser", None),
    ]));
    m
}

fn screen(app: &App, w: u16, h: u16) -> String {
    let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
    t.draw(|f| render_import_menu(f, app, f.area())).unwrap();
    let buf = t.backend().buffer().clone();
    (0..h)
        .map(|y| {
            (0..w)
                .map(|x| buf[(x, y)].symbol().to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn the_search_and_agent_filters_narrow_the_list() {
    let mut m = menu();
    assert_eq!(m.visible().len(), 4);
    m.key(KeyKind::Tab);
    assert_eq!(m.agent, AgentFilter::Claude);
    assert_eq!(m.visible().len(), 2);
    m.key(KeyKind::Tab);
    assert_eq!(
        m.visible()
            .iter()
            .map(|r| r.id.as_str())
            .collect::<Vec<_>>(),
        ["x1", "x2"]
    );
    m.key(KeyKind::Tab);
    for c in "flaky".chars() {
        m.key(KeyKind::Char(c));
    }
    assert_eq!(m.visible().len(), 1);
    m.key(KeyKind::Esc);
    assert!(
        m.open && m.query.is_empty(),
        "the first Esc clears the search"
    );
    assert_eq!(m.key(KeyKind::Esc), ImportAction::Close);
}

#[test]
fn space_selects_and_imported_rows_cannot_be_selected() {
    let mut m = menu();
    m.key(KeyKind::Char(' '));
    assert_eq!(m.selected_count(), 1);
    m.cursor = 2;
    m.key(KeyKind::Char(' '));
    assert_eq!(m.selected_count(), 1, "an imported row stays out");
    m.key(KeyKind::Right);
    assert_eq!(m.selected_count(), 3, "select all skips what is imported");
    m.key(KeyKind::Left);
    assert_eq!(m.selected_count(), 0);
}

#[test]
fn enter_imports_the_selection_or_else_the_focused_row() {
    let mut m = menu();
    m.cursor = 1;
    assert_eq!(
        m.key(KeyKind::Enter),
        ImportAction::Import(vec![("codex".into(), "x1".into())])
    );
    assert!(m.importing);
    assert_eq!(m.key(KeyKind::Enter), ImportAction::None, "busy");
    m.finish_import(1, 0, 0);
    m.key(KeyKind::Right);
    let ImportAction::Import(picks) = m.key(KeyKind::Enter) else {
        panic!("import")
    };
    assert_eq!(picks.len(), 3);
}

#[test]
fn the_summary_is_plural_aware() {
    assert_eq!(summary(1, 0, 0), "Imported 1 session");
    assert_eq!(summary(3, 1, 0), "Imported 3 sessions, 1 already in Forge");
    assert_eq!(
        summary(0, 2, 1),
        "Nothing new imported, 2 already in Forge, 1 failed"
    );
}

#[test]
fn it_renders_at_80x24_and_wide_with_the_fallback_hint() {
    let mut app = App::default();
    app.import_menu = menu();
    app.import_menu.key(KeyKind::Char(' '));
    for (w, h) in [(80, 24), (160, 40)] {
        let s = screen(&app, w, h);
        assert!(s.contains("Import conversations"), "{s}");
        assert!(s.contains("fix the flaky test"), "{s}");
        assert!(s.contains("imported"), "{s}");
        assert!(s.contains("1 selected"), "{s}");
        assert!(s.contains("3d ago"), "{s}");
        assert!(s.contains("Enter import"), "{s}");
    }
    app.import_menu.cursor = 3;
    assert!(screen(&app, 100, 30).contains("opens in /home/me"));
}

#[test]
fn an_empty_scan_and_a_failed_scan_say_so() {
    let mut app = App::default();
    app.import_menu.open("/h".into());
    assert!(screen(&app, 80, 24).contains("Looking for"));
    app.import_menu.apply_scan(Ok(vec![]));
    assert!(screen(&app, 80, 24).contains("No importable sessions found on this computer."));
    app.import_menu.apply_scan(Err("disk on fire".into()));
    assert!(screen(&app, 80, 24).contains("disk on fire"));
}

#[test]
fn a_space_in_the_search_is_typed_not_a_selection() {
    let mut m = menu();
    for c in "fix the".chars() {
        m.key(KeyKind::Char(c));
    }
    assert_eq!(m.query, "fix the");
    assert_eq!(m.selected_count(), 0);
    assert_eq!(m.visible().len(), 1);
    m.key(KeyKind::Right);
    assert_eq!(m.selected_count(), 1, "Right selects the matches");
}
