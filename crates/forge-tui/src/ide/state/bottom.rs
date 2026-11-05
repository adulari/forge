//! The bottom panel: checks, background shells and the terminal.

use std::collections::HashSet;

use super::*;
use crate::ide::checks::{
    check_actions, check_state, failing_changed_files, fix_prompt, parse_file_refs,
    resolve_ref_path, CheckState, WireAction,
};
use crate::ide::term_grid::TermGrid;

/// Ticks (~100 ms) between background reads while a bottom tab is on screen.
use super::tools::ESC_WINDOW;

impl IdeApp {
    // ------------------------------------------------------------------ the bottom panel

    pub fn set_bottom_tab(&mut self, tab: BottomTab) -> Vec<IdeAction> {
        self.bottom_tab = tab;
        self.activate_bottom()
    }

    /// What the visible bottom tab needs read right now.
    pub fn activate_bottom(&mut self) -> Vec<IdeAction> {
        if !self.bottom_open {
            return Vec::new();
        }
        let Some(id) = self.panel_session() else {
            return Vec::new();
        };
        let session = self.route_session(&id);
        match self.bottom_tab {
            BottomTab::Checks => vec![IdeAction::FetchChecks(session)],
            BottomTab::Shells => vec![IdeAction::FetchJobs(session)],
            BottomTab::Terminal => {
                self.tools.term.focused = true;
                if self.tools.term.opened {
                    return Vec::new();
                }
                self.tools.term.opened = true;
                self.tools.term.session = Some(session.clone());
                let (cols, rows) = self.tools.term.view;
                let (cols, rows) = (cols.max(20), rows.max(4));
                self.tools.term.grid = Some(TermGrid::new(cols, rows));
                self.tools.term.sent = (cols, rows);
                self.tools.term.ended = None;
                vec![IdeAction::TermOpen {
                    session_id: session,
                    cols,
                    rows,
                }]
            }
        }
    }

    /// IDE mode was reopened: the terminal's socket is gone, the daemon's shell is not.
    pub fn reconnect_terminal(&mut self) -> Vec<IdeAction> {
        let term = &mut self.tools.term;
        let (Some(session_id), true) = (term.session.clone(), term.opened) else {
            return Vec::new();
        };
        let (cols, rows) = term.sent;
        term.grid = Some(TermGrid::new(cols.max(20), rows.max(4)));
        term.ended = None;
        vec![IdeAction::TermOpen {
            session_id,
            cols: cols.max(20),
            rows: rows.max(4),
        }]
    }

    /// Run `command` in the terminal tab, opening it first (Continue in Claude Code).
    pub fn open_terminal_with(&mut self, session: &str) -> Vec<IdeAction> {
        self.bottom_open = true;
        self.bottom_tab = BottomTab::Terminal;
        self.focus = Pane::Bottom;
        self.tools.term.focused = true;
        let mut actions = Vec::new();
        if self.tools.term.opened {
            actions.push(IdeAction::TermClose);
            self.tools.term.opened = false;
        }
        let _ = session;
        actions.extend(self.activate_bottom());
        actions
    }

    pub fn move_check(&mut self, step: i32) {
        if self.tools.checks.show_output {
            let len = self.tools.checks.refs.len();
            if len > 0 {
                let at = self.tools.checks.ref_cursor as i32 + step;
                self.tools.checks.ref_cursor = at.clamp(0, len as i32 - 1) as usize;
            }
            return;
        }
        let len = self.tools.checks.rows.len();
        if len > 0 {
            let at = self.tools.checks.cursor as i32 + step;
            self.tools.checks.cursor = at.clamp(0, len as i32 - 1) as usize;
        }
    }

    pub fn selected_check(&self) -> Option<&WireAction> {
        self.tools.checks.rows.get(self.tools.checks.cursor)
    }

    /// Enter on a check: run it, or open the output of one that already ran.
    pub fn run_selected_check(&mut self) -> Vec<IdeAction> {
        let Some(row) = self.selected_check().cloned() else {
            return Vec::new();
        };
        let Some(id) = self.panel_session() else {
            return Vec::new();
        };
        let session = self.route_session(&id);
        if self.tools.checks.show_output {
            return self.open_selected_ref();
        }
        if row.running {
            self.tools.checks.show_output = true;
            return Vec::new();
        }
        self.tools.checks.outputs.remove(&row.id);
        self.tools.checks.streamed.insert(row.id.clone());
        self.tools.checks.refs.clear();
        self.tools.checks.show_output = true;
        self.tools.checks.ref_cursor = 0;
        self.recompute_failing();
        vec![IdeAction::RunCheck {
            session_id: session,
            action: row.id,
        }]
    }

    /// `o`: read the last output of the selected check.
    pub fn view_check_output(&mut self) -> Vec<IdeAction> {
        let Some(row) = self.selected_check().cloned() else {
            return Vec::new();
        };
        let Some(id) = self.panel_session() else {
            return Vec::new();
        };
        self.tools.checks.show_output = true;
        self.tools.checks.ref_cursor = 0;
        match row.terminal_id {
            Some(terminal_id) if !self.tools.checks.outputs.contains_key(&row.id) => {
                vec![IdeAction::StreamCheck {
                    session_id: self.route_session(&id),
                    action: row.id,
                    terminal_id,
                }]
            }
            Some(_) => Vec::new(),
            None => {
                self.say(
                    ToastLevel::Info,
                    "this check has not run yet: Enter runs it".to_string(),
                );
                self.tools.checks.show_output = false;
                Vec::new()
            }
        }
    }

    pub fn stop_selected_check(&mut self) -> Vec<IdeAction> {
        let (Some(row), Some(id)) = (self.selected_check().cloned(), self.panel_session()) else {
            return Vec::new();
        };
        if !row.running {
            return Vec::new();
        }
        vec![IdeAction::StopCheck {
            session_id: self.route_session(&id),
            action: row.id,
        }]
    }

    /// `f`: hand the failing check's output to the agent.
    pub fn fix_selected_check(&mut self) -> Vec<IdeAction> {
        let Some(row) = self.selected_check().cloned() else {
            return Vec::new();
        };
        if check_state(&row) != CheckState::Fail {
            self.say(
                ToastLevel::Info,
                "only a failed check can be sent to the agent".to_string(),
            );
            return Vec::new();
        }
        let Some(id) = self.panel_session() else {
            return Vec::new();
        };
        if self.read_only_session(&id) {
            self.say(
                ToastLevel::Info,
                "view only: this session runs in a terminal".to_string(),
            );
            return Vec::new();
        }
        let output = self
            .tools
            .checks
            .outputs
            .get(&row.id)
            .cloned()
            .unwrap_or_default();
        self.say(ToastLevel::Ok, "asked the agent to fix it".to_string());
        vec![IdeAction::Input(
            id,
            serde_json::json!({ "kind": "prompt", "text": fix_prompt(&row.name, &row.command, row.exit_code, &output) }),
        )]
    }

    pub fn open_selected_ref(&mut self) -> Vec<IdeAction> {
        let Some(found) = self
            .tools
            .checks
            .refs
            .get(self.tools.checks.ref_cursor)
            .cloned()
        else {
            return Vec::new();
        };
        let changed: Vec<String> = self.rows.iter().map(|r| r.path.clone()).collect();
        let path = resolve_ref_path(&found, &changed);
        let session = self
            .rows
            .iter()
            .find(|r| r.path == path)
            .map(|r| r.session_id.clone())
            .or_else(|| self.panel_session());
        let Some(session) = session else {
            return Vec::new();
        };
        if self.edit_dirty() {
            self.say(
                ToastLevel::Info,
                "unsaved edits: Ctrl+S saves, Esc discards".to_string(),
            );
            return Vec::new();
        }
        self.focus = Pane::Editor;
        self.open_at(session, path, found.line)
    }

    pub(super) fn recompute_failing(&mut self) {
        let changed: Vec<String> = self.rows.iter().map(|r| r.path.clone()).collect();
        let mut failing: HashSet<String> = HashSet::new();
        for row in &self.tools.checks.rows {
            if check_state(row) != CheckState::Fail {
                continue;
            }
            if let Some(output) = self.tools.checks.outputs.get(&row.id) {
                let refs = parse_file_refs(output, 400);
                failing.extend(failing_changed_files(&changed, &refs));
            }
        }
        if failing != self.tools.checks.failing {
            self.tools.checks.failing = failing;
            self.rebuild_queue();
        }
    }

    pub(super) fn refresh_refs(&mut self) {
        let Some(row) = self.selected_check() else {
            return;
        };
        let refs = self
            .tools
            .checks
            .outputs
            .get(&row.id)
            .map(|o| parse_file_refs(o, 200))
            .unwrap_or_default();
        self.tools.checks.ref_cursor = self
            .tools
            .checks
            .ref_cursor
            .min(refs.len().saturating_sub(1));
        self.tools.checks.refs = refs;
    }

    pub(super) fn apply_checks(
        &mut self,
        result: Result<crate::ide::checks::WireActions, String>,
    ) -> Vec<IdeAction> {
        let mut actions = Vec::new();
        match result {
            Ok(found) => {
                let rows = check_actions(&found.actions);
                let selected = self.selected_check().map(|r| r.id.clone());
                self.tools.checks.rows = rows;
                self.tools.checks.untrusted = found.untrusted;
                self.tools.checks.note = None;
                self.tools.checks.loaded = true;
                if let Some(id) = selected {
                    if let Some(at) = self.tools.checks.rows.iter().position(|r| r.id == id) {
                        self.tools.checks.cursor = at;
                    }
                }
                self.tools.checks.cursor = self
                    .tools
                    .checks
                    .cursor
                    .min(self.tools.checks.rows.len().saturating_sub(1));
                // A run that started elsewhere (the desktop, an agent) still gets its output read.
                if let Some(session) = self.panel_session().map(|id| self.route_session(&id)) {
                    let checks = &mut self.tools.checks;
                    for row in &checks.rows {
                        let Some(terminal_id) = row.terminal_id.clone() else {
                            continue;
                        };
                        let stream = if row.running {
                            checks.streamed.insert(row.id.clone())
                        } else if row.exit_code.is_some() {
                            if checks.outputs.contains_key(&row.id) {
                                checks.streamed.remove(&row.id);
                                false
                            } else {
                                checks.streamed.insert(row.id.clone())
                            }
                        } else {
                            false
                        };
                        if stream {
                            actions.push(IdeAction::StreamCheck {
                                session_id: session.clone(),
                                action: row.id.clone(),
                                terminal_id,
                            });
                        }
                    }
                }
                self.recompute_failing();
            }
            Err(message) => {
                self.tools.checks.note = Some(message);
                self.tools.checks.loaded = true;
            }
        }
        actions
    }

    pub fn move_job(&mut self, step: i32) {
        let len = self.tools.jobs.rows.len();
        if len > 0 {
            let at = self.tools.jobs.cursor as i32 + step;
            self.tools.jobs.cursor = at.clamp(0, len as i32 - 1) as usize;
        }
    }

    /// Enter on a shell: follow its log.
    pub fn view_job(&mut self) -> Vec<IdeAction> {
        let Some(job) = self.tools.jobs.rows.get(self.tools.jobs.cursor).cloned() else {
            return Vec::new();
        };
        let Some(id) = self.panel_session() else {
            return Vec::new();
        };
        self.tools.jobs.viewing = Some(job.id);
        self.tools.jobs.tail.clear();
        vec![IdeAction::FetchJobTail {
            session_id: self.route_session(&id),
            job: job.id,
        }]
    }

    // ------------------------------------------------------------------ the terminal

    pub fn term_size_hint(&mut self, cols: u16, rows: u16) {
        self.tools.term.view = (cols, rows);
    }

    /// Keep the daemon's PTY the size of the pane that shows it.
    pub(super) fn sync_term(&mut self) -> Vec<IdeAction> {
        let term = &mut self.tools.term;
        if !term.opened || term.view == (0, 0) || term.view == term.sent {
            return Vec::new();
        }
        term.sent = term.view;
        let (cols, rows) = term.view;
        if let Some(grid) = term.grid.as_mut() {
            grid.resize(cols, rows);
        }
        vec![IdeAction::TermResize(cols, rows)]
    }

    pub fn term_focused(&self) -> bool {
        self.bottom_open
            && self.focus == Pane::Bottom
            && self.bottom_tab == BottomTab::Terminal
            && self.tools.term.opened
    }

    /// A key while the terminal has focus: Esc Esc leaves, everything else goes to the shell.
    pub fn term_key(&mut self, bytes: Vec<u8>, is_esc: bool) -> Vec<IdeAction> {
        if is_esc {
            if self
                .tools
                .term
                .esc_tick
                .is_some_and(|t| self.tick.saturating_sub(t) <= ESC_WINDOW)
            {
                self.tools.term.esc_tick = None;
                self.tools.term.focused = false;
                self.focus = Pane::Editor;
                return Vec::new();
            }
            self.tools.term.esc_tick = Some(self.tick);
        } else {
            self.tools.term.esc_tick = None;
        }
        if self.tools.term.ended.is_some() {
            self.tools.term.opened = false;
            self.tools.term.ended = None;
            let mut actions = vec![IdeAction::TermClose];
            actions.extend(self.activate_bottom());
            return actions;
        }
        vec![IdeAction::TermInput(bytes)]
    }

    /// `C`: continue this session in Claude Code (wired to the daemon once it can hand off).
    pub fn continue_in_claude(&mut self) -> Vec<IdeAction> {
        let Some(id) = self.panel_session() else {
            return Vec::new();
        };
        self.say(
            ToastLevel::Info,
            "handing the session to Claude Code…".to_string(),
        );
        vec![IdeAction::ContinueClaude(id)]
    }
}
