use ratatui::backend::TestBackend;
use ratatui::Terminal;

use super::*;
use crate::app::App;

fn wt(branch: &str, path: &str, dirty: bool, current: bool, main: bool) -> WtRow {
    WtRow {
        branch: branch.into(),
        path: path.into(),
        display: path.into(),
        dirty,
        current,
        main,
        merged: false,
    }
}

fn data(dirty: bool) -> WorkspaceData {
    WorkspaceData {
        root: "~/code/forge".into(),
        is_git: true,
        is_home: false,
        dirty,
        worktrees: vec![
            wt("main", "~/code/forge", dirty, true, true),
            wt(
                "forge/calm-otter-3f",
                "~/code/forge/.forge/worktrees/calm-otter-3f",
                false,
                false,
                false,
            ),
            wt(
                "forge/quiet-fern-a1",
                "~/code/forge/.forge/worktrees/quiet-fern-a1",
                true,
                false,
                false,
            ),
        ],
        suggested_name: "bright-heron-7c".into(),
        project_name: "forge".into(),
        clone_parent: "~/Projects".into(),
        error: None,
    }
}

fn menu(kind: Kind, data: WorkspaceData) -> WorkspaceMenu {
    let mut m = WorkspaceMenu::default();
    m.open(kind);
    m.apply(WsEvent::Data(data));
    m
}

fn press(m: &mut WorkspaceMenu, keys: &[KeyKind]) -> WsAction {
    let mut last = WsAction::None;
    for k in keys {
        last = m.key(*k);
    }
    last
}

fn typed(text: &str) -> Vec<KeyKind> {
    text.chars().map(KeyKind::Char).collect()
}

fn rows_of(m: &WorkspaceMenu, w: u16, h: u16) -> Vec<String> {
    let mut app = App::default();
    app.workspace_menu = m.clone();
    let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
    terminal
        .draw(|f| {
            let area = f.area();
            let region = ratatui::layout::Rect::new(0, 1, area.width, area.height - 5);
            render_workspace_menu(f, &app, region)
        })
        .unwrap();
    let buffer = terminal.backend().buffer();
    (0..h)
        .map(|y| (0..w).map(|x| buffer[(x, y)].symbol()).collect::<String>())
        .collect()
}

fn both(name: &str, m: &WorkspaceMenu) -> [String; 2] {
    [(80, 24), (160, 48)].map(|(w, h)| {
        let rows = rows_of(m, w, h);
        if let Ok(dir) = std::env::var("WORKSPACE_MENU_DUMP") {
            let _ = std::fs::create_dir_all(&dir);
            let body: Vec<&str> = rows.iter().map(|r| r.trim_end()).collect();
            let _ = std::fs::write(format!("{dir}/{name}-{w}.txt"), body.join("\n"));
        }
        for row in &rows {
            assert!(
                unicode_width::UnicodeWidthStr::width(row.as_str()) <= w as usize,
                "row overflows {w}: {row}"
            );
        }
        rows.join("\n")
    })
}

#[test]
fn list_shows_branch_path_and_markers() {
    for t in both("wt-list", &menu(Kind::Worktrees, data(true))) {
        assert!(t.contains("Worktrees") && t.contains("/worktree"), "{t}");
        assert!(t.contains("forge/calm-otter-3f"), "{t}");
        assert!(
            t.contains("current") && t.contains("dirty") && t.contains("main"),
            "{t}"
        );
        assert!(
            t.contains("n new") && t.contains("d remove") && t.contains("p pull"),
            "{t}"
        );
    }
}

#[test]
fn enter_switches_but_not_into_the_current_checkout() {
    let mut m = menu(Kind::Worktrees, data(false));
    assert_eq!(press(&mut m, &[KeyKind::Enter]), WsAction::None);
    assert!(m.notice.is_some());
    let action = press(&mut m, &[KeyKind::Down, KeyKind::Enter]);
    assert_eq!(
        action,
        WsAction::EnterWorktree {
            path: "~/code/forge/.forge/worktrees/calm-otter-3f".into()
        }
    );
    assert!(matches!(m.screen, Screen::Busy(_)));
}

#[test]
fn search_filters_rows_and_esc_clears_it() {
    let mut m = menu(Kind::Worktrees, data(false));
    press(&mut m, &[KeyKind::Char('/')]);
    press(&mut m, &typed("fern"));
    assert_eq!(m.items(), vec![Item::Worktree(2)]);
    press(&mut m, &[KeyKind::Enter]);
    assert!(!m.searching);
    for t in both("wt-search", &m) {
        assert!(t.contains("quiet-fern") && !t.contains("calm-otter"), "{t}");
    }
    assert_eq!(press(&mut m, &[KeyKind::Esc]), WsAction::None);
    assert_eq!(m.items().len(), 3);
    assert_eq!(press(&mut m, &[KeyKind::Esc]), WsAction::Close);
    assert!(!m.open);
}

#[test]
fn new_prefills_a_friendly_name_and_skips_the_dialog_on_a_clean_tree() {
    let mut m = menu(Kind::Worktrees, data(false));
    press(&mut m, &[KeyKind::Char('n')]);
    let Screen::Input(input) = &m.screen else {
        panic!("{:?}", m.screen)
    };
    assert_eq!(input.value, "bright-heron-7c");
    for t in both("wt-name", &m) {
        assert!(
            t.contains("bright-heron-7c") && t.contains("New worktree"),
            "{t}"
        );
    }
    assert_eq!(
        press(&mut m, &[KeyKind::Enter]),
        WsAction::CreateWorktree {
            name: "bright-heron-7c".into(),
            carry: false
        }
    );
}

#[test]
fn a_dirty_tree_asks_whether_to_carry_the_changes() {
    let mut m = menu(Kind::Worktrees, data(true));
    press(&mut m, &[KeyKind::Char('n'), KeyKind::Enter]);
    let Screen::Choice(choice) = &m.screen else {
        panic!("{:?}", m.screen)
    };
    assert_eq!(choice.options.len(), 3);
    for t in both("wt-dirty", &m) {
        assert!(t.contains("Carry my uncommitted changes"), "{t}");
        assert!(t.contains("Leave them here") && t.contains("Cancel"), "{t}");
    }
    let mut carry = m.clone();
    assert_eq!(
        press(&mut carry, &[KeyKind::Enter]),
        WsAction::CreateWorktree {
            name: "bright-heron-7c".into(),
            carry: true
        }
    );
    let mut leave = m.clone();
    assert_eq!(
        press(&mut leave, &[KeyKind::Right, KeyKind::Enter]),
        WsAction::CreateWorktree {
            name: "bright-heron-7c".into(),
            carry: false
        }
    );
    let mut cancel = m;
    assert_eq!(
        press(
            &mut cancel,
            &[KeyKind::Right, KeyKind::Right, KeyKind::Enter]
        ),
        WsAction::None
    );
    assert_eq!(cancel.screen, Screen::List);
}

#[test]
fn bad_worktree_names_are_refused_inline() {
    let mut m = menu(Kind::Worktrees, data(false));
    press(&mut m, &[KeyKind::Char('n'), KeyKind::KillLineBack]);
    press(&mut m, &typed("bad name"));
    assert_eq!(press(&mut m, &[KeyKind::Enter]), WsAction::None);
    let Screen::Input(input) = &m.screen else {
        panic!("{:?}", m.screen)
    };
    assert!(input.error.is_some());
}

#[test]
fn remove_refuses_main_and_current_and_confirms_the_rest() {
    let mut m = menu(Kind::Worktrees, data(false));
    press(&mut m, &[KeyKind::Char('d')]);
    assert_eq!(m.screen, Screen::List);
    assert!(m.notice.as_ref().unwrap().1.contains("main checkout"));
    press(&mut m, &[KeyKind::Down, KeyKind::Down, KeyKind::Char('d')]);
    let Screen::Choice(choice) = &m.screen else {
        panic!("{:?}", m.screen)
    };
    assert!(choice.danger && choice.selected == 1, "opens on Cancel");
    for t in both("wt-remove", &m) {
        assert!(t.contains("Remove worktree forge/quiet-fern-a1?"), "{t}");
        assert!(
            t.contains("uncommitted changes") && t.contains("Discard changes and remove"),
            "{t}"
        );
        assert!(
            t.contains("not merged") && t.contains("branch is kept"),
            "dialog must say the unmerged branch survives: {t}"
        );
    }
    let mut yes = m.clone();
    assert_eq!(
        press(&mut yes, &[KeyKind::Left, KeyKind::Enter]),
        WsAction::RemoveWorktree {
            path: "~/code/forge/.forge/worktrees/quiet-fern-a1".into(),
            branch: "forge/quiet-fern-a1".into(),
            force: true,
            keep_branch: true
        }
    );
    assert_eq!(press(&mut m, &[KeyKind::Enter]), WsAction::None);
}

#[test]
fn pull_conflict_offers_the_handoff_and_submits_the_prompt() {
    let mut m = menu(Kind::Worktrees, data(false));
    m.apply(WsEvent::Pulled {
        ok: false,
        message: "pull stopped on 1 conflicted file(s)".into(),
        conflicts: vec!["src/lib.rs".into()],
        handoff: Some("Resolve src/lib.rs".into()),
    });
    for t in both("wt-handoff", &m) {
        assert!(
            t.contains("Hand it to the agent") && t.contains("src/lib.rs"),
            "{t}"
        );
    }
    assert_eq!(
        press(&mut m, &[KeyKind::Enter]),
        WsAction::Submit {
            prompt: "Resolve src/lib.rs".into()
        }
    );
    assert!(!m.open);
}

#[test]
fn a_clean_pull_reports_without_a_dialog() {
    let mut m = menu(Kind::Worktrees, data(false));
    m.apply(WsEvent::Pulled {
        ok: true,
        message: "already up to date".into(),
        conflicts: vec![],
        handoff: None,
    });
    assert!(matches!(m.screen, Screen::Message { .. }));
    for t in both("wt-pulled", &m) {
        assert!(t.contains("already up to date"), "{t}");
    }
}

#[test]
fn failed_operations_stay_open_and_successful_ones_close() {
    let mut m = menu(Kind::Worktrees, data(false));
    m.set_busy("Creating");
    m.apply(WsEvent::OpDone {
        ok: false,
        text: "branch already exists".into(),
    });
    assert!(m.open && matches!(m.screen, Screen::Message { .. }));
    m.apply(WsEvent::OpDone {
        ok: true,
        text: String::new(),
    });
    assert!(!m.open);
}

#[test]
fn not_a_git_repository_says_so() {
    let mut d = data(false);
    d.is_git = false;
    d.worktrees.clear();
    let mut m = menu(Kind::Worktrees, d);
    for t in both("wt-nogit", &m) {
        assert!(t.contains("not a git repository"), "{t}");
    }
    press(&mut m, &[KeyKind::Char('n')]);
    assert_eq!(m.screen, Screen::List);
    assert!(m.notice.is_some());
}

#[test]
fn project_overlay_lists_the_onboarding_entries() {
    let mut d = data(false);
    d.is_git = false;
    let m = menu(Kind::Projects, d);
    for t in both("proj-list", &m) {
        assert!(t.contains("Home (no project)"), "{t}");
        assert!(t.contains("Clone a repository"), "{t}");
        assert!(t.contains("Initialize git here"), "{t}");
        assert!(
            t.contains("Rename this project") && t.contains("Hide this project"),
            "{t}"
        );
    }
    let git = menu(Kind::Projects, data(false));
    assert!(!git.items().contains(&Item::InitHere));
}

#[test]
fn home_and_init_and_hide_are_confirmed_or_busy() {
    let mut d = data(false);
    d.is_git = false;
    let mut m = menu(Kind::Projects, d);
    assert_eq!(press(&mut m, &[KeyKind::Enter]), WsAction::GoHome);
    let mut m2 = menu(Kind::Projects, {
        let mut d = data(false);
        d.is_git = false;
        d
    });
    press(&mut m2, &[KeyKind::Down, KeyKind::Down, KeyKind::Enter]);
    assert!(matches!(&m2.screen, Screen::Choice(c) if c.purpose == Purpose::InitHere));
    assert_eq!(press(&mut m2, &[KeyKind::Enter]), WsAction::InitHere);
    let mut m3 = menu(Kind::Projects, data(false));
    press(
        &mut m3,
        &[KeyKind::Down, KeyKind::Down, KeyKind::Down, KeyKind::Enter],
    );
    assert!(
        matches!(&m3.screen, Screen::Choice(c) if c.purpose == Purpose::HideProject && c.selected == 1)
    );
}

#[test]
fn clone_input_loads_repos_filters_them_and_shows_errors() {
    let mut m = menu(Kind::Projects, data(false));
    assert_eq!(
        press(&mut m, &[KeyKind::Down, KeyKind::Enter]),
        WsAction::LoadRepos
    );
    for t in both("proj-clone-loading", &m) {
        assert!(t.contains("loading your GitHub repositories"), "{t}");
    }
    m.apply(WsEvent::Repos(Ok(vec![
        RepoRow {
            name: "octo/forge".into(),
            url: "https://github.com/octo/forge".into(),
            description: "coding harness".into(),
            private: false,
        },
        RepoRow {
            name: "octo/notes".into(),
            url: "https://github.com/octo/notes".into(),
            description: "private notes".into(),
            private: true,
        },
    ])));
    for t in both("proj-clone", &m) {
        assert!(
            t.contains("octo/forge") && t.contains("octo/notes") && t.contains("private"),
            "{t}"
        );
    }
    press(&mut m, &typed("notes"));
    assert_eq!(m.repo_matches("notes"), vec![1]);
    assert_eq!(
        press(&mut m, &[KeyKind::Down, KeyKind::Enter]),
        WsAction::Clone {
            url: "https://github.com/octo/notes".into()
        }
    );
    m.input_error("remote-helper URLs (ext::, fd::, ...) are not accepted");
    for t in both("proj-clone-error", &m) {
        assert!(t.contains("remote-helper URLs"), "{t}");
    }
}

#[test]
fn typed_urls_are_submitted_and_empty_ones_refused() {
    let mut m = menu(Kind::Projects, data(false));
    press(&mut m, &[KeyKind::Char('c')]);
    assert_eq!(press(&mut m, &[KeyKind::Enter]), WsAction::None);
    press(&mut m, &typed("ext::sh -c id"));
    assert_eq!(
        press(&mut m, &[KeyKind::Enter]),
        WsAction::Clone {
            url: "ext::sh -c id".into()
        }
    );
    assert!(matches!(m.screen, Screen::Input(_)), "stays for the error");
}

#[test]
fn gh_unavailable_is_a_quiet_note() {
    let mut m = menu(Kind::Projects, data(false));
    press(&mut m, &[KeyKind::Char('c')]);
    m.apply(WsEvent::Repos(Err(
        "gh is not signed in on this host".into()
    )));
    for t in both("proj-clone-nogh", &m) {
        assert!(t.contains("GitHub list unavailable"), "{t}");
    }
}

#[test]
fn a_finished_clone_offers_the_switch_even_when_the_overlay_was_closed() {
    let mut m = WorkspaceMenu::default();
    m.apply(WsEvent::Cloned(Ok("/home/u/Projects/forge".into())));
    assert!(m.open);
    for t in both("proj-cloned", &m) {
        assert!(t.contains("Switch this session to it?"), "{t}");
    }
    assert_eq!(
        press(&mut m, &[KeyKind::Enter]),
        WsAction::SwitchProject {
            path: "/home/u/Projects/forge".into()
        }
    );
}

#[test]
fn rename_submits_the_new_name() {
    let mut m = menu(Kind::Projects, data(false));
    press(&mut m, &[KeyKind::Down, KeyKind::Down, KeyKind::Enter]);
    assert!(matches!(&m.screen, Screen::Input(i) if i.value == "forge"));
    press(&mut m, &[KeyKind::KillLineBack]);
    press(&mut m, &typed("Forge CLI"));
    assert_eq!(
        press(&mut m, &[KeyKind::Enter]),
        WsAction::RenameProject {
            name: "Forge CLI".into()
        }
    );
}

#[test]
fn loading_state_and_busy_state_render() {
    let mut m = WorkspaceMenu::default();
    m.open(Kind::Worktrees);
    for t in both("wt-loading", &m) {
        assert!(t.contains("reading the workspace"), "{t}");
    }
    m.set_busy("Creating worktree calm-otter");
    for t in both("wt-busy", &m) {
        assert!(t.contains("Creating worktree calm-otter"), "{t}");
    }
}

#[test]
fn long_lists_scroll_to_keep_the_cursor_visible() {
    let mut d = data(false);
    d.worktrees = (0..40)
        .map(|i| {
            wt(
                &format!("forge/task-{i:02}"),
                &format!("~/wt/task-{i:02}"),
                false,
                i == 0,
                i == 0,
            )
        })
        .collect();
    let mut m = menu(Kind::Worktrees, d);
    for _ in 0..30 {
        m.key(KeyKind::Down);
    }
    let rows = rows_of(&m, 80, 24).join("\n");
    assert!(rows.contains("task-30"), "{rows}");
    assert!(!rows.contains("task-01"), "{rows}");
}

#[test]
fn home_is_not_a_project_to_init_rename_or_hide() {
    let mut d = data(false);
    d.is_git = false;
    d.is_home = true;
    let m = menu(Kind::Projects, d);
    assert_eq!(m.items(), vec![Item::Home, Item::Clone]);
}
