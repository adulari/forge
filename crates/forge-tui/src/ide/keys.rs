//! Keyboard and mouse input for IDE mode. Keys follow the desktop IDE: `j/k` file, `n/p` hunk,
//! `a` accept (stage), `x` revert, `u` undo, `r` reviewed, `m` message the agent, `t` bottom
//! panel, `1-4` rail tabs, `Tab` cycles panes, `?` help, `F2` or `Esc` back out.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

use super::state::{BottomTab, Hit, IdeAction, IdeApp, NeedsHit, Pane, RailFocus, RailTab};

/// What `?` shows: `(keys, what it does)`.
pub const HELP: &[(&str, &str)] = &[
    (
        "j / k",
        "next / previous changed file (in the agent panel: timeline; in Agents: agent)",
    ),
    ("n / p", "next / previous hunk"),
    (
        "a",
        "accept: stage the hunk (unstage when it is already staged)",
    ),
    ("x", "revert the hunk (asks first)"),
    ("X", "revert the whole file (asks first)"),
    ("u", "undo the last revert"),
    (
        "r",
        "mark the file reviewed (clears when an agent edits it again), then next",
    ),
    ("m", "message the agent: type, Enter sends, Esc cancels"),
    (
        "y / n / 1-9",
        "in the agent panel: allow / deny a permission, pick a question option",
    ),
    (
        "Y / N",
        "allow / deny the agent's pending permission from anywhere",
    ),
    (
        "Enter",
        "agents: scope the queue to that agent; files: open; timeline: open the edit",
    ),
    ("Up Down PgUp PgDn g G", "scroll the diff"),
    ("v", "unified / side-by-side diff"),
    ("1", "focus the agents list"),
    ("2 / 4", "Changes / Git tab (3 is Files, coming next)"),
    ("s / Space", "Git tab: stage or unstage the file"),
    ("c / P", "Git tab: write the commit message / push"),
    ("Tab / Shift+Tab", "cycle the panes"),
    ("t", "toggle the bottom panel"),
    ("R", "reload everything from the daemon"),
    ("?", "this help"),
    ("F2 / Esc", "back to where you came from"),
];

fn plain(key: &KeyEvent) -> bool {
    !key.modifiers
        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
}

pub fn handle_key(app: &mut IdeApp, key: KeyEvent) -> Vec<IdeAction> {
    if matches!(key.code, KeyCode::F(2)) {
        return vec![IdeAction::Close];
    }
    if app.help_open {
        app.help_open = false;
        return Vec::new();
    }
    if app.confirm.is_some() {
        return match key.code {
            KeyCode::Char('y' | 'Y') | KeyCode::Enter => app.answer_confirm(true),
            _ => app.answer_confirm(false),
        };
    }
    if app.is_typing() {
        return typing(app, key);
    }
    if !plain(&key) {
        return if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('d') => {
                    app.scroll_by(app.view_h as i32 / 2);
                    Vec::new()
                }
                KeyCode::Char('u') => {
                    app.scroll_by(-(app.view_h as i32 / 2));
                    Vec::new()
                }
                _ => Vec::new(),
            }
        } else {
            Vec::new()
        };
    }
    match key.code {
        KeyCode::Esc => return vec![IdeAction::Close],
        KeyCode::Tab => {
            app.cycle_pane(false);
            return app.sync_watch();
        }
        KeyCode::BackTab => {
            app.cycle_pane(true);
            return app.sync_watch();
        }
        KeyCode::Char('?') => {
            app.help_open = true;
            return Vec::new();
        }
        KeyCode::Char('1') => {
            app.focus_agents();
            return Vec::new();
        }
        KeyCode::Char('2') => return app.set_rail_tab(RailTab::Changes),
        KeyCode::Char('3') => return app.set_rail_tab(RailTab::Files),
        KeyCode::Char('4') => return app.set_rail_tab(RailTab::Git),
        KeyCode::Char('t') => {
            app.toggle_bottom();
            return Vec::new();
        }
        KeyCode::Char('v') => {
            app.split = !app.split;
            if let Some(open) = app.open.as_mut() {
                open.scroll = 0;
                open.hunk = 0;
            }
            return Vec::new();
        }
        KeyCode::Char('R') => {
            let mut actions = vec![IdeAction::RefreshChanges];
            if let Some(id) = app.panel_session() {
                actions.push(IdeAction::FetchHistory(id.clone()));
                actions.push(IdeAction::FetchGitStatus(app.route_session(&id)));
            }
            return actions;
        }
        KeyCode::Char('Y') => return app.answer_permission(true),
        KeyCode::Char('N') => return app.answer_permission(false),
        KeyCode::Char('m') => {
            app.start_composer();
            return Vec::new();
        }
        _ => {}
    }
    match app.focus {
        Pane::Agent => agent_key(app, key),
        Pane::Rail => rail_key(app, key),
        Pane::Editor => editor_key(app, key),
        Pane::Bottom => bottom_key(app, key),
    }
}

fn typing(app: &mut IdeApp, key: KeyEvent) -> Vec<IdeAction> {
    let composing = app.composer.active;
    match key.code {
        KeyCode::Esc => {
            app.composer.active = false;
            app.commit.active = false;
        }
        KeyCode::Enter => {
            return if composing {
                app.send_composer()
            } else {
                app.commit_now()
            };
        }
        KeyCode::Backspace => app.backspace(),
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => app.clear_input(),
        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => app.type_char(c),
        _ => {}
    }
    Vec::new()
}

pub fn handle_paste(app: &mut IdeApp, text: &str) {
    if app.is_typing() {
        app.paste(text);
    }
}

/// The keys that review a file, wherever focus is (the agent panel and the rail's lists claim
/// `j/k` for themselves).
fn review_key(app: &mut IdeApp, key: KeyEvent) -> Option<Vec<IdeAction>> {
    Some(match key.code {
        KeyCode::Char('j') => app.step_file(1),
        KeyCode::Char('k') => app.step_file(-1),
        KeyCode::Char('n') => {
            app.step_hunk(1);
            Vec::new()
        }
        KeyCode::Char('p') => {
            app.step_hunk(-1);
            Vec::new()
        }
        KeyCode::Char('a') => app.accept_hunk(),
        KeyCode::Char('x') => {
            app.request_revert_hunk();
            Vec::new()
        }
        KeyCode::Char('X') => {
            app.request_revert_file();
            Vec::new()
        }
        KeyCode::Char('u') => app.undo_last(),
        KeyCode::Char('r') => app.toggle_reviewed(),
        _ => return None,
    })
}

fn scroll_key(app: &mut IdeApp, key: KeyEvent) -> bool {
    let page = app.view_h.saturating_sub(1).max(1) as i32;
    match key.code {
        KeyCode::Down => app.scroll_by(1),
        KeyCode::Up => app.scroll_by(-1),
        KeyCode::PageDown | KeyCode::Char(' ') => app.scroll_by(page),
        KeyCode::PageUp => app.scroll_by(-page),
        KeyCode::Home | KeyCode::Char('g') => app.scroll_by(i32::MIN / 2),
        KeyCode::End | KeyCode::Char('G') => app.scroll_by(i32::MAX / 2),
        _ => return false,
    }
    true
}

fn editor_key(app: &mut IdeApp, key: KeyEvent) -> Vec<IdeAction> {
    if let Some(actions) = review_key(app, key) {
        return actions;
    }
    scroll_key(app, key);
    Vec::new()
}

fn rail_key(app: &mut IdeApp, key: KeyEvent) -> Vec<IdeAction> {
    match app.rail_focus {
        RailFocus::Agents => match key.code {
            KeyCode::Char('j') | KeyCode::Down => return app.move_agent(1),
            KeyCode::Char('k') | KeyCode::Up => return app.move_agent(-1),
            KeyCode::Enter => {
                let id = app.agents.get(app.agent_cursor).map(|a| a.id.clone());
                return id.map(|id| app.toggle_scope(&id)).unwrap_or_default();
            }
            KeyCode::Char('0') => return app.clear_scope(),
            _ => {}
        },
        RailFocus::List => match app.rail_tab {
            RailTab::Changes => match key.code {
                KeyCode::Down => return app.step_file(1),
                KeyCode::Up => return app.step_file(-1),
                KeyCode::Enter => {
                    app.focus = Pane::Editor;
                    return Vec::new();
                }
                _ => {}
            },
            RailTab::Git => match key.code {
                KeyCode::Char('j') | KeyCode::Down => {
                    app.move_git(1);
                    return Vec::new();
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    app.move_git(-1);
                    return Vec::new();
                }
                KeyCode::Enter => return app.open_git_row(app.git_cursor),
                KeyCode::Char('s' | ' ') => return app.toggle_stage_git_row(),
                KeyCode::Char('c') => {
                    app.commit.active = true;
                    return Vec::new();
                }
                KeyCode::Char('P') => return app.push_now(),
                _ => {}
            },
            RailTab::Files => {}
        },
    }
    // On the Changes list `j/k` walk the files, as everywhere else; elsewhere they are the
    // list's own keys and were handled above.
    let lists_own_jk = !(app.rail_focus == RailFocus::List && app.rail_tab == RailTab::Changes);
    if !(lists_own_jk && matches!(key.code, KeyCode::Char('j' | 'k'))) {
        if let Some(actions) = review_key(app, key) {
            return actions;
        }
    }
    Vec::new()
}

fn agent_key(app: &mut IdeApp, key: KeyEvent) -> Vec<IdeAction> {
    match key.code {
        KeyCode::Char('y') | KeyCode::Enter
            if app
                .snapshot()
                .is_some_and(|s| s.permission_prompt.is_some()) =>
        {
            return app.answer_permission(true);
        }
        KeyCode::Char('n')
            if app
                .snapshot()
                .is_some_and(|s| s.permission_prompt.is_some()) =>
        {
            return app.answer_permission(false);
        }
        KeyCode::Char(c @ '1'..='9') if app.snapshot().is_some_and(|s| s.question.is_some()) => {
            return app.answer_option(c as usize - '0' as usize);
        }
        KeyCode::Char('i') => {
            app.start_composer();
            return Vec::new();
        }
        KeyCode::Char('j') | KeyCode::Down => {
            app.move_timeline(1);
            return Vec::new();
        }
        KeyCode::Char('k') | KeyCode::Up => {
            app.move_timeline(-1);
            return Vec::new();
        }
        KeyCode::Enter => return app.open_timeline(app.timeline_cursor),
        _ => {}
    }
    if matches!(key.code, KeyCode::Char('j' | 'k')) {
        return Vec::new();
    }
    review_key(app, key).unwrap_or_default()
}

fn bottom_key(app: &mut IdeApp, key: KeyEvent) -> Vec<IdeAction> {
    match key.code {
        KeyCode::Left => app.bottom_tab = prev_bottom(app.bottom_tab),
        KeyCode::Right => app.bottom_tab = next_bottom(app.bottom_tab),
        KeyCode::Char('h') => app.bottom_tab = prev_bottom(app.bottom_tab),
        KeyCode::Char('l') => app.bottom_tab = next_bottom(app.bottom_tab),
        _ => return review_key(app, key).unwrap_or_default(),
    }
    Vec::new()
}

fn next_bottom(tab: BottomTab) -> BottomTab {
    match tab {
        BottomTab::Checks => BottomTab::Shells,
        BottomTab::Shells => BottomTab::Terminal,
        BottomTab::Terminal => BottomTab::Checks,
    }
}

fn prev_bottom(tab: BottomTab) -> BottomTab {
    match tab {
        BottomTab::Checks => BottomTab::Terminal,
        BottomTab::Shells => BottomTab::Checks,
        BottomTab::Terminal => BottomTab::Shells,
    }
}

pub fn handle_mouse(app: &mut IdeApp, mouse: MouseEvent) -> Vec<IdeAction> {
    let (col, row) = (mouse.column, mouse.row);
    if app.help_open {
        if matches!(mouse.kind, MouseEventKind::Down(_)) {
            app.help_open = false;
        }
        return Vec::new();
    }
    match mouse.kind {
        MouseEventKind::ScrollUp => wheel(app, col, row, -1),
        MouseEventKind::ScrollDown => wheel(app, col, row, 1),
        MouseEventKind::Down(MouseButton::Left) => match app.hit_at(col, row).cloned() {
            Some(hit) => click(app, hit),
            None => Vec::new(),
        },
        _ => Vec::new(),
    }
}

fn wheel(app: &mut IdeApp, col: u16, row: u16, dir: i32) -> Vec<IdeAction> {
    match app.hit_at(col, row).cloned() {
        Some(Hit::Timeline(_) | Hit::Needs(_) | Hit::Composer | Hit::Pane(Pane::Agent)) => {
            app.move_timeline(dir);
            Vec::new()
        }
        Some(Hit::Queue(_) | Hit::Agent(_) | Hit::GitRow(_)) => match app.rail_focus {
            RailFocus::Agents => app.move_agent(dir),
            RailFocus::List if app.rail_tab == RailTab::Git => {
                app.move_git(dir);
                Vec::new()
            }
            RailFocus::List => Vec::new(),
        },
        _ => {
            app.scroll_by(dir * 3);
            Vec::new()
        }
    }
}

fn click(app: &mut IdeApp, hit: Hit) -> Vec<IdeAction> {
    app.composer.active = false;
    app.commit.active = false;
    match hit {
        Hit::Agent(id) => {
            app.focus_agents();
            app.toggle_scope(&id)
        }
        Hit::AllAgents => {
            app.focus_agents();
            app.clear_scope()
        }
        Hit::RailTab(tab) => app.set_rail_tab(tab),
        Hit::NarrowRail(focus, tab) => {
            app.focus = Pane::Rail;
            app.rail_focus = focus;
            app.rail_tab = tab;
            Vec::new()
        }
        Hit::Queue(key) => {
            app.focus = Pane::Editor;
            app.open_key(&key)
        }
        Hit::GitRow(index) => {
            app.rail_focus = RailFocus::List;
            app.open_git_row(index)
        }
        Hit::Pane(pane) => {
            app.focus = pane;
            app.sync_watch()
        }
        Hit::Hunk(index) => {
            app.focus = Pane::Editor;
            app.set_hunk(index);
            Vec::new()
        }
        Hit::Timeline(index) => {
            app.focus = Pane::Agent;
            app.open_timeline(index)
        }
        Hit::Needs(NeedsHit::Allow) => app.answer_permission(true),
        Hit::Needs(NeedsHit::Deny) => app.answer_permission(false),
        Hit::Needs(NeedsHit::Option(n)) => app.answer_option(n),
        Hit::BottomToggle => {
            app.toggle_bottom();
            Vec::new()
        }
        Hit::BottomTab(tab) => {
            app.bottom_tab = tab;
            app.focus = Pane::Bottom;
            Vec::new()
        }
        Hit::CommitInput => {
            app.focus = Pane::Rail;
            app.commit.active = true;
            Vec::new()
        }
        Hit::Commit => app.commit_now(),
        Hit::Push => app.push_now(),
        Hit::Composer => {
            app.start_composer();
            Vec::new()
        }
        Hit::Overlay => Vec::new(),
    }
}
