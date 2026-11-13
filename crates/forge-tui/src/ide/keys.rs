//! Keyboard and mouse input for IDE mode. Keys follow the desktop IDE: `j/k` file, `n/p` hunk,
//! `a` accept (stage), `x` revert, `u` undo, `r` reviewed, `m` message the agent, `t` bottom
//! panel, `1-4` rail tabs, `Tab` cycles panes, `?` help, `F2` or `Esc` back out.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

use super::keys_input::{edit_key, term_key};
use super::state::{
    BottomTab, ConfirmKind, Hit, IdeAction, IdeApp, NeedsHit, Pane, RailFocus, RailTab,
};

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
    (
        "A",
        "accept routine: stage every hunk the queue marks routine (lockfiles, whitespace, import reorders); u undoes",
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
    ("2 / 3 / 4", "Changes / Files / Git tab"),
    ("s / Space", "Git tab: stage or unstage the file"),
    (
        "c / g / P",
        "Git tab: write the commit message / suggest one / push",
    ),
    (
        "Z",
        "Git tab: ship it (stage, message, commit, push as needed)",
    ),
    (
        "e",
        "edit the open file here: type, Tab / Shift+Tab indent, Ctrl+S saves",
    ),
    ("Ctrl+Z / Ctrl+Y", "editor: undo / redo"),
    (
        "Ctrl+W / Ctrl+T",
        "editor: why-trace gutter / who wrote this line",
    ),
    ("E", "open the file in $VISUAL / $EDITOR"),
    (
        "w / W",
        "why-trace: show who wrote each line / details of the hunk or line",
    ),
    (
        "c",
        "comment on the hunk (Enter adds it to the draft, Alt+Enter sends just it)",
    ),
    (
        "S / Ctrl+Enter",
        "send the review draft to the agent as one prompt",
    ),
    ("d", "drop the last comment of the draft"),
    (
        "b",
        "explain the hunk: asks the agent as a /btw side question",
    ),
    (
        "C",
        "agent panel: continue this session in Claude Code (terminal tab)",
    ),
    (
        "[ / ]",
        "bottom panel: previous / next tab (Checks, Shells, Terminal)",
    ),
    (
        "Checks tab",
        "Enter run, o output, f ask the agent to fix, x stop; in output Enter opens file:line",
    ),
    ("Shells tab", "j/k pick, Enter follows the log"),
    (
        "Terminal tab",
        "keys go to the shell; Esc Esc leaves it, Alt+Left / Right switch tab",
    ),
    ("H", "hide the first-visit hints"),
    ("Tab / Shift+Tab", "cycle the panes"),
    ("t", "toggle the bottom panel"),
    ("R", "reload everything from the daemon"),
    ("?", "this help"),
    ("F2 / Esc", "back to where you came from"),
];

/// Every way out of IDE mode goes through here, so unsaved edits are always asked about first.
fn request_close(app: &mut IdeApp) -> Vec<IdeAction> {
    if app.edit_dirty() {
        app.confirm = Some(super::state::Confirm {
            kind: ConfirmKind::QuitDiscard,
            text: "leave with unsaved edits?  y/n   (Ctrl+S saves)".to_string(),
        });
        return Vec::new();
    }
    vec![IdeAction::Close]
}

fn plain(key: &KeyEvent) -> bool {
    !key.modifiers
        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
}

pub fn handle_key(app: &mut IdeApp, key: KeyEvent) -> Vec<IdeAction> {
    if matches!(key.code, KeyCode::F(2)) {
        return request_close(app);
    }
    if app.help_open {
        app.help_open = false;
        return Vec::new();
    }
    if app.tools.why_popup.is_some() {
        return match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                app.move_why(1);
                Vec::new()
            }
            KeyCode::Char('k') | KeyCode::Up => {
                app.move_why(-1);
                Vec::new()
            }
            KeyCode::Enter => app.jump_to_turn(),
            _ => {
                app.tools.why_popup = None;
                Vec::new()
            }
        };
    }
    if let Some(confirm) = &app.confirm {
        if confirm.kind == ConfirmKind::SaveConflict {
            app.confirm = None;
            return match key.code {
                KeyCode::Char('r' | 'R') => app.reload_edit(),
                KeyCode::Char('o' | 'O') => app.save_edit(true),
                _ => Vec::new(),
            };
        }
        return match key.code {
            KeyCode::Char('y' | 'Y') | KeyCode::Enter => app.answer_confirm(true),
            _ => app.answer_confirm(false),
        };
    }
    if app.is_typing() {
        return typing(app, key);
    }
    if matches!(key.code, KeyCode::Enter)
        && key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
    {
        return app.send_review();
    }
    if app.term_focused() {
        return term_key(app, key);
    }
    if app.editing() && app.focus == Pane::Editor {
        return edit_key(app, key);
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
        KeyCode::Esc if !esc_is_local(app) => return request_close(app),
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
        KeyCode::Char('t') => return app.toggle_bottom(),
        KeyCode::Char('[') if app.bottom_open => {
            return app.set_bottom_tab(prev_bottom(app.bottom_tab));
        }
        KeyCode::Char(']') if app.bottom_open => {
            return app.set_bottom_tab(next_bottom(app.bottom_tab));
        }
        KeyCode::Char('H') => return app.dismiss_coach(),
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

/// Esc backs out of an open check output or followed shell log before it leaves IDE mode.
fn esc_is_local(app: &IdeApp) -> bool {
    app.focus == Pane::Bottom
        && app.bottom_open
        && match app.bottom_tab {
            BottomTab::Checks => app.tools.checks.show_output,
            BottomTab::Shells => app.tools.jobs.viewing.is_some(),
            BottomTab::Terminal => false,
        }
}

fn typing(app: &mut IdeApp, key: KeyEvent) -> Vec<IdeAction> {
    let composing = app.composer.active;
    let commenting = app.tools.comment.active;
    if commenting
        && matches!(key.code, KeyCode::Enter)
        && key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
    {
        return app.finish_comment(true);
    }
    match key.code {
        KeyCode::Esc => {
            app.composer.active = false;
            app.commit.active = false;
            app.cancel_comment();
        }
        KeyCode::Enter => {
            return if composing {
                app.send_composer()
            } else if commenting {
                app.finish_comment(false)
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

pub fn handle_paste(app: &mut IdeApp, text: &str) -> Vec<IdeAction> {
    if app.is_typing() {
        app.paste(text);
    } else if app.term_focused() {
        return vec![IdeAction::TermInput(text.as_bytes().to_vec())];
    } else if app.editing() && app.focus == Pane::Editor {
        app.edit_op(true, |b| b.insert_str(text));
    }
    Vec::new()
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
        KeyCode::Char('a') => {
            app.advance_coach(1);
            app.accept_hunk()
        }
        KeyCode::Char('A') => app.accept_routine(),
        KeyCode::Char('x') => {
            app.advance_coach(1);
            app.request_revert_hunk();
            Vec::new()
        }
        KeyCode::Char('X') => {
            app.request_revert_file();
            Vec::new()
        }
        KeyCode::Char('u') => app.undo_last(),
        KeyCode::Char('r') => {
            app.advance_coach(1);
            app.toggle_reviewed()
        }
        KeyCode::Char('c') => {
            app.start_comment();
            Vec::new()
        }
        KeyCode::Char('b') => app.explain_hunk(),
        KeyCode::Char('e') => app.start_edit(None),
        KeyCode::Char('E') => app.external_edit(),
        KeyCode::Char('w') => app.toggle_why(),
        KeyCode::Char('W') => {
            app.open_why();
            Vec::new()
        }
        KeyCode::Char('S') => app.send_review(),
        KeyCode::Char('d') => {
            app.drop_last_comment();
            Vec::new()
        }
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
                KeyCode::Char('g') => return app.suggest_message(),
                KeyCode::Char('Z') => return app.ship(),
                _ => {}
            },
            RailTab::Files => match key.code {
                KeyCode::Char('j') | KeyCode::Down => {
                    app.move_file(1);
                    return Vec::new();
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    app.move_file(-1);
                    return Vec::new();
                }
                KeyCode::Enter | KeyCode::Char('l') | KeyCode::Right => {
                    return app.open_file_row(app.tools.files.cursor);
                }
                _ => {}
            },
        },
    }
    // On the Changes list `j/k` walk the files, as everywhere else; elsewhere they are the
    // list's own keys and were handled above.
    let lists_own_jk = !(app.rail_focus == RailFocus::List && app.rail_tab == RailTab::Changes);
    if app.rail_focus == RailFocus::List && app.rail_tab == RailTab::Git {
        if let KeyCode::Char('c') = key.code {
            return Vec::new();
        }
    }
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
        KeyCode::Char('C') => return app.continue_in_claude(),
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
        KeyCode::Left | KeyCode::Char('h') => {
            return app.set_bottom_tab(prev_bottom(app.bottom_tab))
        }
        KeyCode::Right | KeyCode::Char('l') => {
            return app.set_bottom_tab(next_bottom(app.bottom_tab))
        }
        _ => {}
    }
    match app.bottom_tab {
        BottomTab::Checks => match key.code {
            KeyCode::Char('j') | KeyCode::Down => app.move_check(1),
            KeyCode::Char('k') | KeyCode::Up => app.move_check(-1),
            KeyCode::Enter => return app.run_selected_check(),
            KeyCode::Char('o') => return app.view_check_output(),
            KeyCode::Char('f') => return app.fix_selected_check(),
            KeyCode::Char('x') if app.selected_check().is_some_and(|c| c.running) => {
                return app.stop_selected_check();
            }
            KeyCode::Esc if app.tools.checks.show_output => app.tools.checks.show_output = false,
            _ => return review_key(app, key).unwrap_or_default(),
        },
        BottomTab::Shells => match key.code {
            KeyCode::Char('j') | KeyCode::Down => app.move_job(1),
            KeyCode::Char('k') | KeyCode::Up => app.move_job(-1),
            KeyCode::Enter => return app.view_job(),
            KeyCode::Esc if app.tools.jobs.viewing.is_some() => app.tools.jobs.viewing = None,
            _ => return review_key(app, key).unwrap_or_default(),
        },
        BottomTab::Terminal => return review_key(app, key).unwrap_or_default(),
    }
    Vec::new()
}

pub(super) fn next_bottom(tab: BottomTab) -> BottomTab {
    match tab {
        BottomTab::Checks => BottomTab::Shells,
        BottomTab::Shells => BottomTab::Terminal,
        BottomTab::Terminal => BottomTab::Checks,
    }
}

pub(super) fn prev_bottom(tab: BottomTab) -> BottomTab {
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
            Some(hit) => click(app, hit, col),
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
        Some(Hit::Queue(_) | Hit::Agent(_) | Hit::GitRow(_) | Hit::File(_)) => match app.rail_focus
        {
            RailFocus::Agents => app.move_agent(dir),
            RailFocus::List if app.rail_tab == RailTab::Git => {
                app.move_git(dir);
                Vec::new()
            }
            RailFocus::List if app.rail_tab == RailTab::Files => {
                app.move_file(dir);
                Vec::new()
            }
            RailFocus::List => Vec::new(),
        },
        Some(Hit::Check(_)) => {
            app.move_check(dir);
            Vec::new()
        }
        Some(Hit::Job(_)) => {
            app.move_job(dir);
            Vec::new()
        }
        _ => {
            app.scroll_by(dir * 3);
            Vec::new()
        }
    }
}

fn click(app: &mut IdeApp, hit: Hit, mouse_col: u16) -> Vec<IdeAction> {
    app.composer.active = false;
    app.commit.active = false;
    app.cancel_comment();
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
            let actions = if focus == RailFocus::List {
                app.set_rail_tab(tab)
            } else {
                app.rail_tab = tab;
                Vec::new()
            };
            app.focus = Pane::Rail;
            app.rail_focus = focus;
            actions
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
            let mut actions = app.sync_watch();
            if pane == Pane::Bottom {
                actions.extend(app.activate_bottom());
            }
            actions
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
        Hit::BottomToggle => app.toggle_bottom(),
        Hit::BottomTab(tab) => {
            app.focus = Pane::Bottom;
            app.set_bottom_tab(tab)
        }
        Hit::File(index) => {
            app.rail_focus = RailFocus::List;
            app.open_file_row(index)
        }
        Hit::Check(index) => {
            app.focus = Pane::Bottom;
            let again = app.tools.checks.cursor == index && !app.tools.checks.show_output;
            app.tools.checks.cursor = index;
            if again {
                app.run_selected_check()
            } else {
                app.tools.checks.show_output = false;
                Vec::new()
            }
        }
        Hit::Job(index) => {
            app.focus = Pane::Bottom;
            app.tools.jobs.cursor = index;
            app.view_job()
        }
        Hit::Ship => app.ship(),
        Hit::Suggest => app.suggest_message(),
        Hit::Draft => app.send_review(),
        Hit::EditLine(line, text_x, hscroll) => {
            app.focus = Pane::Editor;
            // The click is a screen cell; the cursor is a char index. Undo the sideways scroll,
            // then find the char drawn at that cell (tabs and wide chars span several).
            let cell = usize::from(mouse_col.saturating_sub(text_x)) + hscroll;
            app.edit_op(false, |b| {
                let text = b.lines().get(line).map_or("", String::as_str);
                let col = super::render::char_at_display_col(text, cell);
                b.set_cursor(line, col);
            });
            Vec::new()
        }
        Hit::Terminal => {
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
