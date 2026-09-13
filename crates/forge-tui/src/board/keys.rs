//! Keyboard and mouse → board mutations + [`BoardAction`]s. One place, so the help overlay and
//! the footer keybar can never drift from what the keys actually do.
//!
//! Focus rules: the composer, the confirm dialog, the help overlay and the filter box each take
//! every key while open. Otherwise the board and the detail pane share the action keys
//! (`a p s i m M x r c y n 1-9`) and differ only in what ↑/↓/PgUp/PgDn move.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

use super::state::{BoardApp, Button, ComposerMode, DetailTab, Focus, Hit, ToastLevel};
use super::BoardAction;

/// One row of the help overlay / keybar: key, what it does.
pub const HELP: &[(&str, &str)] = &[
    ("↑↓ j k", "move within the column"),
    ("←→ h l Tab", "switch column"),
    ("Enter o", "open the card (again: focus the pane)"),
    ("Esc", "back — pane focus → board → close pane"),
    ("a", "attach: drop into the session in this terminal"),
    ("p", "send a prompt (queued while busy)"),
    ("s", "steer: jump the queue at the next turn boundary"),
    ("y / n", "allow / deny the pending permission"),
    ("1-9", "pick an option of the pending question"),
    ("e", "answer the pending question in free text"),
    ("i", "interrupt the running turn"),
    ("m", "re-pin the model (/model), empty clears"),
    (
        "M",
        "cycle the mode: default → accept-edits → bypass → plan",
    ),
    ("x", "archive (asks first)"),
    ("r", "resume a Done session"),
    ("N / W", "new session here / in a fresh worktree"),
    ("f", "cycle the project filter"),
    ("/", "filter cards by text"),
    ("[ ]", "switch the pane's section"),
    ("t", "show or hide tool rows in Live"),
    ("F", "follow the live tail again"),
    ("c", "copy the session id"),
    ("R", "refresh now"),
    ("?", "this help"),
    ("q  Ctrl+C", "quit — sessions keep running"),
];

pub fn handle_key(app: &mut BoardApp, key: KeyEvent) -> Vec<BoardAction> {
    if key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('c')) {
        return vec![BoardAction::Quit];
    }
    match app.focus {
        Focus::Composer => composer_key(app, key),
        Focus::Confirm => match key.code {
            KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('Y') => app.resolve_confirm(true),
            KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Char('q') => {
                app.resolve_confirm(false)
            }
            _ => Vec::new(),
        },
        Focus::Help => {
            app.focus = if app.detail_open {
                Focus::Detail
            } else {
                Focus::Board
            };
            Vec::new()
        }
        Focus::Filter => filter_key(app, key),
        Focus::Board | Focus::Detail => shared_key(app, key),
    }
}

fn composer_key(app: &mut BoardApp, key: KeyEvent) -> Vec<BoardAction> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let Some(c) = app.composer.as_mut() else {
        app.focus = Focus::Board;
        return Vec::new();
    };
    match key.code {
        KeyCode::Esc => app.cancel_composer(),
        KeyCode::Enter => return app.submit_composer(),
        KeyCode::Backspace => c.backspace(),
        KeyCode::Left => c.move_left(),
        KeyCode::Right => c.move_right(),
        KeyCode::Home => c.home(),
        KeyCode::End => c.end(),
        KeyCode::Char('u') if ctrl => c.kill_line_back(),
        KeyCode::Char('w') if ctrl => c.delete_word_back(),
        KeyCode::Char('a') if ctrl => c.home(),
        KeyCode::Char('e') if ctrl => c.end(),
        KeyCode::Tab => {
            if let ComposerMode::NewSession { worktree, .. } = &mut c.mode {
                *worktree = !*worktree;
            }
        }
        KeyCode::Char(ch) if !ctrl => c.insert(&ch.to_string()),
        _ => {}
    }
    Vec::new()
}

/// A bracketed paste lands in whichever text box is open.
pub fn handle_paste(app: &mut BoardApp, text: &str) {
    let one_line = text.replace(['\r', '\n'], " ");
    match app.focus {
        Focus::Composer => {
            if let Some(c) = app.composer.as_mut() {
                c.insert(&one_line);
            }
        }
        Focus::Filter => {
            app.query.push_str(&one_line);
            app.rebuild();
        }
        _ => {}
    }
}

fn filter_key(app: &mut BoardApp, key: KeyEvent) -> Vec<BoardAction> {
    match key.code {
        KeyCode::Esc => {
            app.query.clear();
            app.focus = Focus::Board;
            app.rebuild();
        }
        KeyCode::Enter => app.focus = Focus::Board,
        KeyCode::Backspace => {
            app.query.pop();
            app.rebuild();
        }
        KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.query.push(ch);
            app.rebuild();
        }
        _ => {}
    }
    Vec::new()
}

fn shared_key(app: &mut BoardApp, key: KeyEvent) -> Vec<BoardAction> {
    let in_detail = app.focus == Focus::Detail;
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);
    match key.code {
        KeyCode::Char('q') => return vec![BoardAction::Quit],
        KeyCode::Char('?') => app.focus = Focus::Help,
        KeyCode::Esc => {
            if in_detail {
                app.focus = Focus::Board;
            } else if app.detail_open {
                app.close_detail();
            } else if !app.query.is_empty() {
                app.query.clear();
                app.rebuild();
            } else if app.project_filter.is_some() {
                app.project_filter = None;
                app.rebuild();
            }
        }
        KeyCode::Enter | KeyCode::Char('o') => {
            if app.detail_open && !in_detail {
                app.focus = Focus::Detail;
                let id = app.selected.clone().unwrap_or_default();
                return app.detail_actions_for(&id);
            }
            return app.open_detail();
        }
        KeyCode::Up | KeyCode::Char('k') => {
            if in_detail {
                app.scroll_detail(-1);
            } else {
                app.move_in_column(-1);
                return follow_selection(app);
            }
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if in_detail {
                app.scroll_detail(1);
            } else {
                app.move_in_column(1);
                return follow_selection(app);
            }
        }
        KeyCode::PageUp => {
            if in_detail {
                app.scroll_detail(-10);
            } else {
                app.move_in_column(-5);
                return follow_selection(app);
            }
        }
        KeyCode::PageDown => {
            if in_detail {
                app.scroll_detail(10);
            } else {
                app.move_in_column(5);
                return follow_selection(app);
            }
        }
        KeyCode::Home | KeyCode::Char('g') => {
            if in_detail {
                app.detail_scroll = 0;
                app.tail_follow = false;
            } else {
                app.jump_in_column(false);
                return follow_selection(app);
            }
        }
        KeyCode::End | KeyCode::Char('G') => {
            if in_detail {
                app.tail_follow = true;
                app.detail_scroll = usize::MAX / 2;
            } else {
                app.jump_in_column(true);
                return follow_selection(app);
            }
        }
        KeyCode::Left | KeyCode::Char('h') => {
            app.move_column(-1);
            return follow_selection(app);
        }
        KeyCode::Right | KeyCode::Char('l') => {
            app.move_column(1);
            return follow_selection(app);
        }
        KeyCode::Tab => {
            app.move_column(1);
            return follow_selection(app);
        }
        KeyCode::BackTab => {
            app.move_column(-1);
            return follow_selection(app);
        }
        KeyCode::Char(']') => {
            app.detail_tab = app.detail_tab.next();
            app.detail_scroll = 0;
            app.tail_follow = true;
        }
        KeyCode::Char('[') => {
            app.detail_tab = app.detail_tab.prev();
            app.detail_scroll = 0;
            app.tail_follow = true;
        }
        KeyCode::Char('t') => app.show_tools = !app.show_tools,
        KeyCode::Char('F') => {
            app.tail_follow = true;
            app.detail_tab = DetailTab::Tail;
        }
        KeyCode::Char('a') => return app.attach_selected(),
        KeyCode::Char('p') => {
            if let Some(t) = app.writable_target() {
                app.open_composer(ComposerMode::Prompt, &t, "");
            }
        }
        KeyCode::Char('s') => {
            if let Some(t) = app.writable_target() {
                app.open_composer(ComposerMode::Steer, &t, "");
            }
        }
        KeyCode::Char('m') => {
            if let Some(t) = app.writable_target() {
                let current = app
                    .selected_card()
                    .map(|c| c.model.clone())
                    .filter(|m| m != "—")
                    .unwrap_or_default();
                app.open_composer(ComposerMode::Model, &t, &current);
            }
        }
        KeyCode::Char('M') => return app.cycle_mode_selected(),
        KeyCode::Char('e') => {
            if let Some(t) = app.writable_target() {
                let seq = app
                    .snapshots
                    .get(&t)
                    .filter(|s| s.question.is_some())
                    .map(|s| s.prompt_seq);
                match seq {
                    Some(seq) => app.open_composer(ComposerMode::Answer { seq }, &t, ""),
                    None => app.toast(ToastLevel::Info, "no question is pending"),
                }
            }
        }
        KeyCode::Char('y') => return app.answer_permission(true),
        KeyCode::Char('n') => return app.answer_permission(false),
        KeyCode::Char(d @ '1'..='9') => {
            return app.answer_option(d.to_digit(10).unwrap_or(0) as usize);
        }
        KeyCode::Char('i') => return app.interrupt_selected(),
        KeyCode::Char('x') => app.request_archive(),
        KeyCode::Char('r') => return app.resume_selected(),
        KeyCode::Char('R') => {
            app.detail_requested.clear();
            let mut out = vec![BoardAction::Refresh];
            if let Some(id) = app.selected.clone() {
                out.extend(app.detail_actions_for(&id));
            }
            app.toast(ToastLevel::Info, "refreshing…");
            return out;
        }
        KeyCode::Char('N') => {
            let cwd = app.new_session_cwd();
            app.open_composer(
                ComposerMode::NewSession {
                    cwd,
                    worktree: false,
                },
                "",
                "",
            );
        }
        KeyCode::Char('W') => {
            let cwd = app.new_session_cwd();
            app.open_composer(
                ComposerMode::NewSession {
                    cwd,
                    worktree: true,
                },
                "",
                "",
            );
        }
        KeyCode::Char('f') => app.cycle_project_filter(),
        KeyCode::Char('/') => app.focus = Focus::Filter,
        KeyCode::Char('c') => {
            if let Some(c) = app.selected_card() {
                let id = c.id.clone();
                app.toast(ToastLevel::Ok, format!("copied {}", &id[..id.len().min(8)]));
                return vec![BoardAction::Copy(id)];
            }
        }
        _ => {
            let _ = shift;
        }
    }
    Vec::new()
}

/// With the pane open, moving the selection previews the new card in it.
fn follow_selection(app: &mut BoardApp) -> Vec<BoardAction> {
    if !app.detail_open {
        return Vec::new();
    }
    let Some(id) = app.selected.clone() else {
        return Vec::new();
    };
    app.detail_actions_for(&id)
}

pub fn handle_mouse(app: &mut BoardApp, ev: MouseEvent) -> Vec<BoardAction> {
    match ev.kind {
        MouseEventKind::Down(MouseButton::Left) => click(app, ev.column, ev.row),
        MouseEventKind::ScrollUp => wheel(app, ev.column, ev.row, -1),
        MouseEventKind::ScrollDown => wheel(app, ev.column, ev.row, 1),
        _ => Vec::new(),
    }
}

fn wheel(app: &mut BoardApp, col: u16, row: u16, dir: i32) -> Vec<BoardAction> {
    if matches!(app.focus, Focus::Composer | Focus::Confirm | Focus::Help) {
        return Vec::new();
    }
    // The pane spans from its left edge to the right edge of the screen; the close glyph sits at
    // its top-right, so "left of the ✕ by at most a pane width, at or below its row" is the pane.
    let over_detail = app.detail_open
        && app
            .hits
            .iter()
            .any(|(r, h)| *h == Hit::CloseDetail && col + 200 >= r.x && row >= r.y);
    if over_detail || app.focus == Focus::Detail {
        app.scroll_detail(dir * 3);
        return Vec::new();
    }
    if let Some(Hit::Card(_) | Hit::ColumnHeader(_)) = app.hit_at(col, row).cloned().as_ref() {
        app.move_in_column(dir);
        return follow_selection(app);
    }
    Vec::new()
}

fn click(app: &mut BoardApp, col: u16, row: u16) -> Vec<BoardAction> {
    match app.focus {
        Focus::Help => {
            app.focus = if app.detail_open {
                Focus::Detail
            } else {
                Focus::Board
            };
            return Vec::new();
        }
        Focus::Confirm => return Vec::new(),
        Focus::Composer => {
            app.cancel_composer();
        }
        _ => {}
    }
    let Some(hit) = app.hit_at(col, row).cloned() else {
        return Vec::new();
    };
    match hit {
        Hit::Card(id) => {
            if app.selected.as_deref() == Some(id.as_str()) {
                return app.open_detail();
            }
            app.select(&id);
            follow_selection(app)
        }
        Hit::ColumnHeader(c) => {
            app.set_cursor_column(c);
            follow_selection(app)
        }
        Hit::DetailTab(t) => {
            app.detail_tab = t;
            app.detail_scroll = 0;
            app.tail_follow = true;
            app.focus = Focus::Detail;
            Vec::new()
        }
        Hit::CloseDetail => {
            app.close_detail();
            Vec::new()
        }
        Hit::Help => {
            app.focus = Focus::Help;
            Vec::new()
        }
        Hit::ProjectFilter => {
            app.cycle_project_filter();
            Vec::new()
        }
        Hit::Button(b) => button(app, b),
    }
}

fn button(app: &mut BoardApp, b: Button) -> Vec<BoardAction> {
    match b {
        Button::Attach => app.attach_selected(),
        Button::Prompt => {
            if let Some(t) = app.writable_target() {
                app.open_composer(ComposerMode::Prompt, &t, "");
            }
            Vec::new()
        }
        Button::Steer => {
            if let Some(t) = app.writable_target() {
                app.open_composer(ComposerMode::Steer, &t, "");
            }
            Vec::new()
        }
        Button::Interrupt => app.interrupt_selected(),
        Button::Allow => app.answer_permission(true),
        Button::Deny => app.answer_permission(false),
        Button::Answer(n) => app.answer_option(n),
        Button::Model => {
            if let Some(t) = app.writable_target() {
                let current = app
                    .selected_card()
                    .map(|c| c.model.clone())
                    .unwrap_or_default();
                app.open_composer(ComposerMode::Model, &t, &current);
            }
            Vec::new()
        }
        Button::Mode => app.cycle_mode_selected(),
        Button::Archive => {
            app.request_archive();
            Vec::new()
        }
        Button::Resume => app.resume_selected(),
        Button::NewSession => {
            let cwd = app.new_session_cwd();
            app.open_composer(
                ComposerMode::NewSession {
                    cwd,
                    worktree: false,
                },
                "",
                "",
            );
            Vec::new()
        }
        Button::Copy => {
            if let Some(c) = app.selected_card() {
                let id = c.id.clone();
                app.toast(ToastLevel::Ok, "copied the session id");
                return vec![BoardAction::Copy(id)];
            }
            Vec::new()
        }
        Button::ToggleTools => {
            app.show_tools = !app.show_tools;
            Vec::new()
        }
    }
}
