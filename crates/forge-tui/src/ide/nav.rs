//! Navigation and review: which file and hunk are open, reviewed marks, stage / revert / undo.

use super::state::View;
use std::rc::Rc;

use crate::board::ToastLevel;

use super::hunk::{
    actions_for, build_patch, undo_step_for_hunk, undo_steps_for_file, Bucket, HunkAction,
    UndoRecord,
};
use super::model::{mark_reviewed, next_to_review, step_queue, unmark_reviewed, Tier};

use super::state::IdeApp;
use super::types::*;

impl IdeApp {
    // ------------------------------------------------------------------ navigation

    pub(super) fn set_scope(&mut self, scope: Option<String>) -> Vec<IdeAction> {
        self.scope = scope;
        self.rebuild_queue();
        let mut actions = Vec::new();
        if let Some(open) = &self.open {
            if self.scope.as_ref().is_some_and(|s| s != &open.session_id) && !self.edit_dirty() {
                self.open = None;
                self.tools.edit = None;
            }
        }
        actions.extend(self.sync_watch());
        actions
    }

    /// Enter on an agent: scope the queue to it, or clear the scope if it already is.
    pub fn toggle_scope(&mut self, id: &str) -> Vec<IdeAction> {
        if let Some(index) = self.agents.iter().position(|a| a.id == id) {
            self.agent_cursor = index;
        }
        let next = if self.scope.as_deref() == Some(id) {
            None
        } else {
            Some(id.to_string())
        };
        self.set_scope(next)
    }

    pub fn clear_scope(&mut self) -> Vec<IdeAction> {
        self.set_scope(None)
    }

    pub fn move_agent(&mut self, step: i32) -> Vec<IdeAction> {
        if self.agents.is_empty() {
            return Vec::new();
        }
        let max = self.agents.len() as i32 - 1;
        self.agent_cursor = (self.agent_cursor as i32 + step).clamp(0, max) as usize;
        self.sync_watch()
    }

    /// Open `key`'s file in the editor.
    pub fn open_key(&mut self, key: &str) -> Vec<IdeAction> {
        let Some(row) = self.rows.iter().find(|r| r.key == key) else {
            return Vec::new();
        };
        let (session_id, path) = (row.session_id.clone(), row.path.clone());
        self.open_file(session_id, path)
    }

    pub fn open_file(&mut self, session_id: String, path: String) -> Vec<IdeAction> {
        let same = self
            .open
            .as_ref()
            .is_some_and(|o| o.session_id == session_id && o.path == path);
        if self.edit_dirty() && !same {
            self.say(
                ToastLevel::Info,
                "unsaved edits: Ctrl+S saves, Esc discards".to_string(),
            );
            return Vec::new();
        }
        let tracked = self.tracked(&session_id, &path);
        if !same {
            self.tools.edit = None;
            self.tools.why_popup = None;
        }
        self.open = Some(OpenFile {
            session_id,
            path,
            loading: false,
            error: None,
            hunk: 0,
            scroll: 0,
            view: if tracked { View::Diff } else { View::Edit },
            seek: None,
        });
        self.confirm = None;
        self.advance_coach(0);
        let mut actions = if tracked {
            let mut actions = self.ensure_diff();
            actions.extend(self.ensure_blame());
            actions
        } else {
            self.start_edit(None)
        };
        actions.extend(self.sync_watch());
        actions
    }

    pub fn step_file(&mut self, step: i32) -> Vec<IdeAction> {
        let current = self.open_row().map(|r| r.key.clone());
        let Some(next) = step_queue(&self.rows, current.as_deref(), step).map(|r| r.key.clone())
        else {
            return Vec::new();
        };
        self.open_key(&next)
    }

    pub fn step_hunk(&mut self, step: i32) {
        let Some((parsed, _)) = self.parsed() else {
            return;
        };
        let (_, starts) = parsed.layout(self.split);
        if starts.is_empty() {
            return;
        }
        let split = self.split;
        let view_h = self.view_h;
        let Some(open) = self.open.as_mut() else {
            return;
        };
        let max = starts.len() as i32 - 1;
        open.hunk = (open.hunk as i32 + step).clamp(0, max) as usize;
        let (rows, starts) = parsed.layout(split);
        open.scroll = reveal(
            open.scroll,
            starts[open.hunk],
            hunk_end(rows.len(), starts, open.hunk),
            view_h,
        );
    }

    pub fn scroll_by(&mut self, delta: i32) {
        if self.editing() {
            self.edit_scroll_by(delta);
            return;
        }
        let Some((parsed, _)) = self.parsed() else {
            return;
        };
        let (rows, starts) = parsed.layout(self.split);
        let max = rows.len().saturating_sub(self.view_h.max(1));
        let Some(open) = self.open.as_mut() else {
            return;
        };
        open.scroll = (open.scroll as i64 + i64::from(delta)).clamp(0, max as i64) as usize;
        // The cursor follows the viewport: the hunk whose rows are at the top.
        if let Some(index) = starts.iter().rposition(|&s| s <= open.scroll) {
            open.hunk = index;
        }
    }

    pub fn set_hunk(&mut self, hunk: usize) {
        let Some((parsed, _)) = self.parsed() else {
            return;
        };
        let (rows, starts) = parsed.layout(self.split);
        let view_h = self.view_h;
        let Some(open) = self.open.as_mut() else {
            return;
        };
        if hunk < starts.len() {
            open.hunk = hunk;
            open.scroll = reveal(
                open.scroll,
                starts[hunk],
                hunk_end(rows.len(), starts, hunk),
                view_h,
            );
        }
    }

    pub fn cycle_pane(&mut self, back: bool) {
        let mut order = vec![Pane::Rail, Pane::Editor, Pane::Agent];
        if self.bottom_open {
            order.push(Pane::Bottom);
        }
        let at = order.iter().position(|p| *p == self.focus).unwrap_or(0) as i32;
        let step = if back { -1 } else { 1 };
        let next = (at + step).rem_euclid(order.len() as i32) as usize;
        self.focus = order[next];
    }

    pub fn toggle_bottom(&mut self) -> Vec<IdeAction> {
        self.bottom_open = !self.bottom_open;
        if !self.bottom_open && self.focus == Pane::Bottom {
            self.focus = Pane::Editor;
        } else if self.bottom_open {
            self.focus = Pane::Bottom;
            return self.activate_bottom();
        }
        Vec::new()
    }

    pub fn set_rail_tab(&mut self, tab: RailTab) -> Vec<IdeAction> {
        self.rail_tab = tab;
        self.rail_focus = RailFocus::List;
        self.focus = Pane::Rail;
        if tab == RailTab::Files {
            return self.ensure_files();
        }
        if tab == RailTab::Git {
            return self
                .panel_session()
                .map(|id| vec![IdeAction::FetchGitStatus(self.route_session(&id))])
                .unwrap_or_default();
        }
        Vec::new()
    }

    pub fn focus_agents(&mut self) {
        self.focus = Pane::Rail;
        self.rail_focus = RailFocus::Agents;
    }

    // ------------------------------------------------------------------ reviewing

    /// Mark the open file reviewed (or clear the mark) and move to the next file that still needs
    /// a look.
    pub fn toggle_reviewed(&mut self) -> Vec<IdeAction> {
        let Some(row) = self.open_row().cloned() else {
            return Vec::new();
        };
        let mut actions = Vec::new();
        let was = row.tier == Tier::Reviewed;
        let changed = if was {
            unmark_reviewed(&mut self.marks, &row.session_id, &row.path)
        } else {
            mark_reviewed(&mut self.marks, &row.session_id, &row.path, &row.file.hash)
        };
        if changed {
            actions.push(self.save_marks());
        }
        self.rebuild_queue();
        if !was {
            if let Some(next) = next_to_review(&self.rows, &row.key).map(|r| r.key.clone()) {
                actions.extend(self.open_key(&next));
            }
        }
        actions
    }

    pub(super) fn current_hunk(&self) -> Option<(Rc<ParsedDiff>, Bucket, usize)> {
        let (parsed, bucket) = self.parsed()?;
        let hunk = self.open.as_ref()?.hunk;
        (hunk < parsed.wire.hunks.len()).then_some((parsed, bucket, hunk))
    }

    pub(super) fn read_only_session(&self, session_id: &str) -> bool {
        self.agents
            .iter()
            .any(|a| a.id == session_id && a.read_only)
    }

    /// `a`: stage the hunk under the cursor (unstage when the diff is the staged side).
    pub fn accept_hunk(&mut self) -> Vec<IdeAction> {
        let Some((parsed, bucket, index)) = self.current_hunk() else {
            return Vec::new();
        };
        let Some(open) = &self.open else {
            return Vec::new();
        };
        let read_only = self.read_only_session(&self.route_session(&open.session_id));
        let allowed = actions_for(bucket, &parsed.wire, read_only);
        let action = if allowed.contains(&HunkAction::Stage) {
            HunkAction::Stage
        } else if allowed.contains(&HunkAction::Unstage) {
            HunkAction::Unstage
        } else {
            self.say(
                ToastLevel::Info,
                "this hunk cannot be staged on its own".to_string(),
            );
            return Vec::new();
        };
        self.hunk_action(&parsed, index, action)
    }

    pub(super) fn hunk_action(
        &mut self,
        parsed: &ParsedDiff,
        index: usize,
        action: HunkAction,
    ) -> Vec<IdeAction> {
        let Some(open) = &self.open else {
            return Vec::new();
        };
        let hunk = &parsed.wire.hunks[index];
        let patch = build_patch(&parsed.wire.path, &parsed.wire.kind, hunk);
        let undo = (action == HunkAction::Revert).then(|| UndoRecord {
            session_id: self.route_session(&open.session_id),
            path: parsed.wire.path.clone(),
            label: format!("hunk {}", hunk.header),
            steps: vec![undo_step_for_hunk(&parsed.wire.path, hunk)],
        });
        vec![IdeAction::Hunk {
            session_id: self.route_session(&open.session_id),
            path: parsed.wire.path.clone(),
            header: hunk.header.clone(),
            patch,
            action,
            undo,
            is_undo: false,
        }]
    }

    /// `x`: ask before reverting the hunk under the cursor.
    pub fn request_revert_hunk(&mut self) {
        let Some((parsed, bucket, index)) = self.current_hunk() else {
            return;
        };
        let read_only = self
            .open
            .as_ref()
            .is_some_and(|o| self.read_only_session(&self.route_session(&o.session_id)));
        if !actions_for(bucket, &parsed.wire, read_only).contains(&HunkAction::Revert) {
            self.say(
                ToastLevel::Info,
                "only working-tree hunks can be reverted: unstage this one first".to_string(),
            );
            return;
        }
        let header = parsed.wire.hunks[index].header.clone();
        let lines = parsed.wire.hunks[index].lines.clone();
        self.confirm = Some(Confirm {
            text: format!("revert hunk {header}?  y/n"),
            kind: ConfirmKind::RevertHunk { header, lines },
        });
    }

    /// `X`: ask before reverting the whole file.
    pub fn request_revert_file(&mut self) {
        let Some(open) = &self.open else {
            return;
        };
        self.confirm = Some(Confirm {
            text: format!("revert all changes to {}?  y/n", open.path),
            kind: ConfirmKind::RevertFile,
        });
    }

    pub fn answer_confirm(&mut self, yes: bool) -> Vec<IdeAction> {
        let Some(confirm) = self.confirm.take() else {
            return Vec::new();
        };
        if !yes {
            self.say(ToastLevel::Info, "cancelled".to_string());
            return Vec::new();
        }
        match confirm.kind {
            ConfirmKind::DiscardEdits => {
                self.leave_edit();
                Vec::new()
            }
            ConfirmKind::QuitDiscard => {
                self.tools.edit = None;
                vec![IdeAction::Close]
            }
            ConfirmKind::SaveConflict => Vec::new(),
            ConfirmKind::RevertHunk { header, lines } => {
                let Some((parsed, _, _)) = self.current_hunk() else {
                    return Vec::new();
                };
                match parsed
                    .wire
                    .hunks
                    .iter()
                    .position(|h| h.header == header && h.lines == lines)
                {
                    Some(index) => self.hunk_action(&parsed, index, HunkAction::Revert),
                    None => {
                        self.say(
                            ToastLevel::Error,
                            "the file changed since you asked: hunk not reverted".to_string(),
                        );
                        Vec::new()
                    }
                }
            }
            ConfirmKind::RevertFile => {
                let Some(open) = &self.open else {
                    return Vec::new();
                };
                let undo = self.parsed().and_then(|(parsed, _)| {
                    let steps = undo_steps_for_file(&parsed.wire);
                    (!steps.is_empty()).then(|| UndoRecord {
                        session_id: self.route_session(&open.session_id),
                        path: parsed.wire.path.clone(),
                        label: format!("file {}", parsed.wire.path),
                        steps,
                    })
                });
                vec![IdeAction::DiscardFile {
                    session_id: self.route_session(&open.session_id),
                    path: open.path.clone(),
                    undo,
                }]
            }
        }
    }

    /// `u`: put back the last revert by re-applying its reversed patch(es).
    pub fn undo_last(&mut self) -> Vec<IdeAction> {
        let Some(record) = self.undo.pop() else {
            self.say(ToastLevel::Info, "nothing to undo".to_string());
            return Vec::new();
        };
        let session_id = record.session_id.clone();
        record
            .steps
            .iter()
            .map(|step| IdeAction::Hunk {
                session_id: session_id.clone(),
                path: record.path.clone(),
                header: step.hunk_header.clone(),
                patch: step.patch.clone(),
                action: HunkAction::Revert,
                undo: None,
                is_undo: true,
            })
            .collect()
    }
}

fn hunk_end(total: usize, starts: &[usize], hunk: usize) -> usize {
    starts.get(hunk + 1).copied().unwrap_or(total)
}

/// Scroll so rows `start..end` are visible, preferring the start of the hunk at the top.
fn reveal(scroll: usize, start: usize, end: usize, view_h: usize) -> usize {
    let view_h = view_h.max(1);
    if start < scroll || end > scroll + view_h {
        start
    } else {
        scroll
    }
}
