use ratatui::backend::TestBackend;
use ratatui::Terminal;

use super::*;
use crate::app::App;

const NOW: i64 = 1_793_390_000_000;

fn cat(id: &str, label: &str, enabled: bool) -> CategoryView {
    CategoryView {
        id: id.into(),
        label: label.into(),
        enabled,
        items: if enabled { 4 } else { 0 },
        last_sync_at: enabled.then_some(NOW - 120_000),
        requires_direct: id == "keys",
        ..Default::default()
    }
}

fn signed_in(categories: Vec<CategoryView>) -> AnywhereSnapshot {
    AnywhereSnapshot {
        signed_in: true,
        login: Some("octo".into()),
        connector: Connector::Live,
        connector_text: "live · relay 4s ago".into(),
        device_label: "a1b2c3d4 · workstation".into(),
        sync_enabled: true,
        worker_running: true,
        last_sync_at: Some(NOW - 120_000),
        categories,
        taken_at_ms: NOW,
        ..Default::default()
    }
}

fn all_off() -> Vec<CategoryView> {
    vec![
        cat("keys", "Keys & tokens", false),
        cat("skills", "Skills & agents", false),
        cat("settings", "Settings", false),
        cat("mcp", "MCP servers", false),
        cat("instructions", "Instructions", false),
    ]
}

fn busy_categories() -> Vec<CategoryView> {
    let mut c = all_off();
    c[1] = CategoryView {
        pending: 2,
        ..cat("skills", "Skills & agents", true)
    };
    c[2] = CategoryView {
        conflicts: vec![(
            "setting: tui.theme".into(),
            "changed here and on another device; kept the other device's".into(),
        )],
        ..cat("settings", "Settings", true)
    };
    c[3] = CategoryView {
        skipped: vec![(
            "mcp: github".into(),
            "needs `npx`, not installed here".into(),
        )],
        remote_only: 1,
        ..cat("mcp", "MCP servers", true)
    };
    c
}

fn devices() -> Devices {
    Devices::Loaded {
        hosts_online: 2,
        hosts_total: 3,
        devices: vec![
            DeviceView {
                id: "a1b2c3d4".into(),
                name: "workstation".into(),
                enrolled: "2026-09-12".into(),
                last_seen_at: Some(NOW),
                this_device: true,
            },
            DeviceView {
                id: "99aa88bb".into(),
                name: "laptop".into(),
                enrolled: "2026-10-02".into(),
                last_seen_at: Some(NOW - 180_000),
                this_device: false,
            },
        ],
    }
}

fn menu(snapshot: Option<AnywhereSnapshot>, devices: Devices) -> AnywhereMenu {
    let mut m = AnywhereMenu::default();
    m.open();
    m.now_ms = NOW;
    if let Some(s) = snapshot {
        m.apply_snapshot(Ok(s));
    }
    m.apply_devices(devices);
    m
}

fn rows_of(m: &AnywhereMenu, w: u16, h: u16) -> Vec<String> {
    let mut app = App::default();
    app.anywhere_menu = m.clone();
    let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
    terminal
        .draw(|f| {
            // Between the one-row header and the composer + status rows, as in the live layout.
            let area = f.area();
            let region = ratatui::layout::Rect::new(0, 1, area.width, area.height - 5);
            render_anywhere_menu(f, &app, region)
        })
        .unwrap();
    let buffer = terminal.backend().buffer();
    (0..h)
        .map(|y| (0..w).map(|x| buffer[(x, y)].symbol()).collect::<String>())
        .collect()
}

fn dump(name: &str, w: u16, rows: &[String]) {
    if let Ok(dir) = std::env::var("ANYWHERE_MENU_DUMP") {
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(
            format!("{dir}/{name}-{w}.txt"),
            rows.iter()
                .map(|r| r.trim_end())
                .collect::<Vec<_>>()
                .join("\n"),
        );
    }
}

fn text(rows: &[String]) -> String {
    rows.join("\n")
}

fn both(name: &str, m: &AnywhereMenu) -> [String; 2] {
    [(80, 24), (160, 48)].map(|(w, h)| {
        let rows = rows_of(m, w, h);
        dump(name, w, &rows);
        for row in &rows {
            assert!(
                unicode_width::UnicodeWidthStr::width(row.as_str()) <= w as usize,
                "row overflows {w}: {row}"
            );
        }
        text(&rows)
    })
}

#[test]
fn loading_shows_a_spinner_line() {
    for t in both("loading", &menu(None, Devices::Loading)) {
        assert!(t.contains("Reading Anywhere status"), "{t}");
        assert!(t.contains("Forge Anywhere") && t.contains("Esc close"));
    }
}

#[test]
fn a_failed_first_read_names_the_error_and_the_retry_key() {
    let mut m = menu(None, Devices::Loading);
    m.apply_snapshot(Err("state file is unreadable".into()));
    for t in both("error", &m) {
        assert!(t.contains("Could not read Anywhere status"), "{t}");
        assert!(t.contains("state file is unreadable"));
        assert!(t.contains("try again"));
    }
    assert_eq!(m.key(KeyKind::Char('r')), MenuAction::Refresh);
    assert!(m.take_refresh());
}

#[test]
fn not_signed_in_leads_with_the_sign_in_step() {
    let snapshot = AnywhereSnapshot {
        signed_in: false,
        connector: Connector::Off,
        connector_text: "not enabled on this machine".into(),
        categories: all_off(),
        taken_at_ms: NOW,
        ..Default::default()
    };
    let mut m = menu(Some(snapshot), Devices::Unavailable("not signed in".into()));
    for t in both("signed-out", &m) {
        assert!(t.contains("not signed in"), "{t}");
        assert!(t.contains("Sign in to Forge Anywhere"));
        assert!(t.contains("Open Recovery Center"));
    }
    assert_eq!(m.focused(), Some(Row::SignInToAnywhere));
    assert_eq!(m.key(KeyKind::Enter), MenuAction::SetupAnywhere);
    m.key(KeyKind::Down);
    m.key(KeyKind::Down);
    assert_eq!(m.focused(), Some(Row::Category(0)));
    assert_eq!(m.key(KeyKind::Char(' ')), MenuAction::None);
    assert!(
        m.notice.is_some(),
        "enabling while signed out explains itself"
    );
}

#[test]
fn signed_in_with_everything_off() {
    let m = menu(Some(signed_in(all_off())), devices());
    assert_eq!(m.focused(), Some(Row::Category(0)));
    for t in both("sync-off", &m) {
        assert!(t.contains("octo"), "{t}");
        assert!(t.contains("Keys & tokens") && t.contains("Instructions"));
        assert!(t.contains("off"));
        assert!(t.contains("never") || t.contains("last sync"));
    }
}

#[test]
fn enabled_categories_show_pending_conflicts_and_skips_with_reasons() {
    let mut m = menu(Some(signed_in(busy_categories())), devices());
    for _ in 0..3 {
        m.key(KeyKind::Down);
    }
    let [narrow, wide] = both("sync-busy", &m);
    for t in [&narrow, &wide] {
        assert!(t.contains("2 pending"), "{t}");
        assert!(t.contains("1 conflict"));
        assert!(t.contains("setting: tui.theme"));
        assert!(t.contains("needs `npx`, not installed here"));
        assert!(t.contains("1 waiting to apply") || t.contains("skipped"));
    }
    assert!(wide.contains("workstation") && wide.contains("this device"));
    assert!(wide.contains("laptop"));
}

#[test]
fn keys_switch_on_only_after_the_confirmation_and_never_by_accident() {
    let mut m = menu(Some(signed_in(all_off())), devices());
    assert_eq!(m.focused(), Some(Row::Category(0)));
    assert_eq!(m.key(KeyKind::Char(' ')), MenuAction::None);
    assert!(m.confirm.is_some());
    assert!(!m.data.as_ref().unwrap().categories[0].enabled);
    for t in both("keys-confirm", &m) {
        assert!(t.contains("Sync API keys across your machines?"), "{t}");
        assert!(t.contains("second vault key"));
        assert!(t.contains("never sync"));
        assert!(t.contains("Turn on key sync") && t.contains("Cancel"));
    }
    // Enter on the default button is Cancel.
    assert_eq!(m.key(KeyKind::Enter), MenuAction::None);
    assert!(m.confirm.is_none());
    assert!(!m.data.as_ref().unwrap().categories[0].enabled);
    // Right arrow must not turn it on either: the confirmation has to be answered.
    assert_eq!(m.key(KeyKind::Right), MenuAction::None);
    assert!(m.confirm.is_some());
    m.key(KeyKind::Esc);
    assert!(
        m.confirm.is_none() && m.open,
        "Esc closes only the confirmation"
    );
    // Tab to the primary button, Enter confirms.
    m.key(KeyKind::Right);
    m.key(KeyKind::Tab);
    assert_eq!(
        m.key(KeyKind::Enter),
        MenuAction::Toggle {
            category: "keys".into(),
            enabled: true
        }
    );
}

#[test]
fn other_categories_toggle_immediately_and_disabling_never_asks() {
    let mut m = menu(Some(signed_in(busy_categories())), devices());
    m.key(KeyKind::Down);
    assert_eq!(
        m.key(KeyKind::Char(' ')),
        MenuAction::Toggle {
            category: "skills".into(),
            enabled: false
        }
    );
    assert!(!m.data.as_ref().unwrap().categories[1].enabled);
    m.key(KeyKind::Down);
    assert_eq!(
        m.key(KeyKind::Left),
        MenuAction::Toggle {
            category: "settings".into(),
            enabled: false
        }
    );
    assert_eq!(m.key(KeyKind::Left), MenuAction::None, "already off");
}

#[test]
fn disabling_keys_does_not_ask() {
    let mut cats = all_off();
    cats[0] = cat("keys", "Keys & tokens", true);
    let mut m = menu(Some(signed_in(cats)), devices());
    assert_eq!(
        m.key(KeyKind::Enter),
        MenuAction::Toggle {
            category: "keys".into(),
            enabled: false
        }
    );
    assert!(m.confirm.is_none());
}

#[test]
fn tab_walks_the_sections_and_sign_ins_are_rows() {
    let mut snapshot = signed_in(busy_categories());
    snapshot.sign_ins = vec![SignInView {
        provider: "codex".into(),
        label: "Codex (ChatGPT)".into(),
        command: "forge auth codex-oauth".into(),
    }];
    snapshot.missing_tools = vec![MissingToolView {
        label: "Claude Code".into(),
        hint: "npm install -g @anthropic-ai/claude-code".into(),
    }];
    let mut m = menu(Some(snapshot), devices());
    m.key(KeyKind::Tab);
    assert_eq!(m.focused().map(Row::section), Some(Section::Devices));
    m.key(KeyKind::Tab);
    assert_eq!(m.focused().map(Row::section), Some(Section::Account));
    m.key(KeyKind::CycleTemper);
    assert_eq!(m.focused().map(Row::section), Some(Section::Devices));
    m.key(KeyKind::End);
    m.key(KeyKind::Up);
    m.key(KeyKind::Up);
    assert_eq!(m.focused(), Some(Row::SignIn(0)));
    assert_eq!(
        m.key(KeyKind::Enter),
        MenuAction::SignInProvider {
            label: "Codex (ChatGPT)".into(),
            command: "forge auth codex-oauth".into()
        }
    );
    let t = both("sign-in-again", &m);
    for t in &t {
        assert!(
            t.contains("Sign in again") && t.contains("forge auth codex-oauth"),
            "{t}"
        );
    }
}

#[test]
fn sync_now_marks_syncing_until_the_next_snapshot() {
    let mut m = menu(Some(signed_in(busy_categories())), devices());
    assert_eq!(m.key(KeyKind::Char('s')), MenuAction::SyncNow);
    assert!(m.syncing);
    assert!(text(&rows_of(&m, 80, 24)).contains("Syncing"));
    m.apply_snapshot(Ok(signed_in(busy_categories())));
    assert!(!m.syncing);
}

#[test]
fn a_dead_worker_and_a_sync_error_are_called_out() {
    let mut snapshot = signed_in(busy_categories());
    snapshot.worker_running = false;
    snapshot.last_error = Some("service returned 503".into());
    snapshot.sync_enabled = false;
    let m = menu(Some(snapshot), Devices::Unavailable("offline".into()));
    for t in both("worker-down", &m) {
        assert!(t.contains("forge serve"), "{t}");
        assert!(t.contains("service returned 503"));
        assert!(t.contains("paused"));
    }
}

#[test]
fn a_failed_refresh_keeps_the_last_good_data() {
    let mut m = menu(Some(signed_in(busy_categories())), devices());
    m.apply_snapshot(Err("timed out".into()));
    assert_eq!(m.load, Load::Ready);
    assert!(m.notice.as_ref().unwrap().text.contains("timed out"));
    assert!(text(&rows_of(&m, 80, 24)).contains("Skills & agents"));
}

#[test]
fn notices_expire() {
    let mut m = menu(Some(signed_in(all_off())), devices());
    m.notify(Tone::Ok, "hello");
    for _ in 0..(NOTICE_TICKS + 2) {
        m.advance(NOW);
    }
    assert!(m.notice.is_none());
}

#[test]
fn revoke_is_explained_not_performed() {
    let mut m = menu(Some(signed_in(all_off())), devices());
    m.key(KeyKind::End);
    assert_eq!(m.focused(), Some(Row::Device(1)));
    assert_eq!(m.key(KeyKind::Enter), MenuAction::None);
    assert!(m
        .notice
        .as_ref()
        .unwrap()
        .text
        .contains("--revoke 99aa88bb"));
}

#[test]
fn no_screen_ever_contains_a_secret_looking_value() {
    let m = menu(Some(signed_in(busy_categories())), devices());
    for t in both("secrets", &m) {
        assert!(!t.contains("sk-") && !t.contains("Bearer"), "{t}");
    }
}

#[test]
fn the_focused_row_stays_visible_at_80x24_with_many_rows() {
    let mut cats = busy_categories();
    cats[2].conflicts = (0..6)
        .map(|i| (format!("setting: k{i}"), "changed on both".into()))
        .collect();
    let mut m = menu(Some(signed_in(cats)), devices());
    m.key(KeyKind::End);
    let t = text(&rows_of(&m, 80, 24));
    assert!(t.contains("laptop"), "{t}");
    m.key(KeyKind::Home);
    let t = text(&rows_of(&m, 80, 24));
    assert!(t.contains("Open Recovery Center"), "{t}");
}

#[test]
fn closed_menu_draws_nothing() {
    let mut m = menu(Some(signed_in(all_off())), devices());
    m.close();
    assert!(text(&rows_of(&m, 80, 24)).trim().is_empty());
}

#[test]
fn ago_reads_naturally() {
    assert_eq!(render::ago(NOW, NOW - 2_000), "just now");
    assert_eq!(render::ago(NOW, NOW - 30_000), "30s ago");
    assert_eq!(render::ago(NOW, NOW - 125_000), "2m ago");
    assert_eq!(render::ago(NOW, NOW - 7_300_000), "2h ago");
    assert_eq!(render::ago(NOW, NOW - 200_000_000), "2d ago");
}
