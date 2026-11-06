//! The agent panel's actions, the Git tab, and text input.

use crate::board::ToastLevel;

use super::hunk::Bucket;
use super::model::{is_reviewed, ChangedFile};
use super::timeline::{resolve_changed_path, TimelineKind};
use super::wire::UNATTRIBUTED;

use super::state::IdeApp;
use super::types::*;

impl IdeApp {
    // ------------------------------------------------------------------ the agent

    /// Y / N: answer the panel agent's pending permission prompt, seq-checked like the board.
    pub fn answer_permission(&mut self, yes: bool) -> Vec<IdeAction> {
        let Some(id) = self.panel_session() else {
            return Vec::new();
        };
        let Some(snap) = self.snapshots.get(&id) else {
            return Vec::new();
        };
        if snap.permission_prompt.is_none() {
            self.say(
                ToastLevel::Info,
                "nothing is waiting on a permission".to_string(),
            );
            return Vec::new();
        }
        let seq = snap.prompt_seq;
        self.say(
            ToastLevel::Ok,
            if yes { "allowed" } else { "denied" }.to_string(),
        );
        vec![IdeAction::Input(
            id,
            serde_json::json!({ "kind": "allow", "yes": yes, "seq": seq }),
        )]
    }

    /// 1-9 on a question: pick option `n` (1-based).
    pub fn answer_option(&mut self, n: usize) -> Vec<IdeAction> {
        let Some(id) = self.panel_session() else {
            return Vec::new();
        };
        let Some(snap) = self.snapshots.get(&id) else {
            return Vec::new();
        };
        if snap.question.is_none() || n == 0 || n > snap.question_options.len() {
            return Vec::new();
        }
        let seq = snap.prompt_seq;
        let label = snap.question_options[n - 1].label.clone();
        self.say(ToastLevel::Ok, format!("answered: {label}"));
        vec![IdeAction::Input(
            id,
            serde_json::json!({ "kind": "answer", "text": n.to_string(), "seq": seq }),
        )]
    }

    pub fn start_composer(&mut self) {
        if self.panel_session().is_none() {
            return;
        }
        self.focus = Pane::Agent;
        self.composer.active = true;
    }

    pub fn send_composer(&mut self) -> Vec<IdeAction> {
        let text = self.composer.text.trim().to_string();
        let Some(id) = self.panel_session() else {
            return Vec::new();
        };
        if text.is_empty() {
            self.composer.active = false;
            return Vec::new();
        }
        if self.read_only_session(&id) {
            self.say(
                ToastLevel::Info,
                "view only: this session runs in a terminal".to_string(),
            );
            return Vec::new();
        }
        self.composer = Input::default();
        self.say(ToastLevel::Ok, "sent".to_string());
        vec![IdeAction::Input(
            id,
            serde_json::json!({ "kind": "prompt", "text": text }),
        )]
    }

    /// Enter on a timeline edit: open the file it touched.
    pub fn open_timeline(&mut self, index: usize) -> Vec<IdeAction> {
        let Some(item) = self.timeline.get(index).cloned() else {
            return Vec::new();
        };
        self.timeline_cursor = index;
        if item.kind != TimelineKind::Edit {
            return Vec::new();
        }
        let Some(written) = item.path else {
            return Vec::new();
        };
        let Some(agent) = self.panel_session() else {
            return Vec::new();
        };
        let changed: Vec<String> = self
            .queue_agents
            .iter()
            .filter(|a| a.session_id == agent || a.session_id == UNATTRIBUTED)
            .flat_map(|a| a.files.iter().map(|f| f.path.clone()))
            .collect();
        let Some(path) = resolve_changed_path(&written, &changed) else {
            self.say(
                ToastLevel::Info,
                "that file has no changes left".to_string(),
            );
            return Vec::new();
        };
        let session = self
            .queue_agents
            .iter()
            .find(|a| {
                (a.session_id == agent || a.session_id == UNATTRIBUTED)
                    && a.files.iter().any(|f| f.path == path)
            })
            .map_or(agent, |a| a.session_id.clone());
        self.focus = Pane::Editor;
        let mut actions = self.open_file(session, path);
        if let Some(open) = self.open.as_mut() {
            open.seek = item.needle.clone();
        }
        self.seek_hunk();
        actions.extend(self.sync_watch());
        actions
    }

    /// Put the hunk cursor on the hunk that holds the line an edit wrote, once that diff is here.
    pub(super) fn seek_hunk(&mut self) {
        let Some(needle) = self.open.as_ref().and_then(|o| o.seek.clone()) else {
            return;
        };
        let Some((parsed, _)) = self.parsed() else {
            return;
        };
        let wanted = needle.trim();
        let found = parsed.overlay.hunks.iter().position(|h| {
            h.lines
                .iter()
                .any(|l| l.kind != crate::diff_overlay::LineKind::Del && l.text.trim() == wanted)
        });
        if let Some(open) = self.open.as_mut() {
            open.seek = None;
        }
        if let Some(index) = found {
            self.set_hunk(index);
        }
    }

    pub fn move_timeline(&mut self, step: i32) {
        if self.timeline.is_empty() {
            return;
        }
        let max = self.timeline.len() as i32 - 1;
        self.timeline_cursor = (self.timeline_cursor as i32 + step).clamp(0, max) as usize;
    }

    // ------------------------------------------------------------------ git

    pub fn move_git(&mut self, step: i32) {
        let len = self.git_list().len();
        if len == 0 {
            return;
        }
        self.git_cursor = (self.git_cursor as i32 + step).clamp(0, len as i32 - 1) as usize;
    }

    /// Enter on a Git row: show that file's diff (staged side for a staged row).
    pub fn open_git_row(&mut self, index: usize) -> Vec<IdeAction> {
        let list = self.git_list();
        let Some((_, _, row)) = list.get(index) else {
            return Vec::new();
        };
        self.git_cursor = index;
        let path = row.path.clone();
        let Some(session) = self.panel_session() else {
            return Vec::new();
        };
        // Prefer the queue's own row for the file so reviewed marks and hunks line up.
        let owner = self
            .queue_agents
            .iter()
            .find(|a| a.session_id == session && a.files.iter().any(|f| f.path == path))
            .or_else(|| {
                self.queue_agents
                    .iter()
                    .find(|a| a.files.iter().any(|f| f.path == path))
            })
            .map(|a| a.session_id.clone());
        match owner {
            Some(owner) => {
                self.focus = Pane::Editor;
                self.open_file(owner, path)
            }
            None => {
                self.say(
                    ToastLevel::Info,
                    "that file is not in the change queue yet".to_string(),
                );
                Vec::new()
            }
        }
    }

    /// `s` / Space on a Git row: stage it, or unstage it when it is staged.
    pub fn toggle_stage_git_row(&mut self) -> Vec<IdeAction> {
        let list = self.git_list();
        let Some((bucket, conflicted, row)) = list.get(self.git_cursor) else {
            return Vec::new();
        };
        if *conflicted {
            self.say(
                ToastLevel::Info,
                "resolve the conflict in your editor first".to_string(),
            );
            return Vec::new();
        }
        let Some(session) = self.panel_session() else {
            return Vec::new();
        };
        vec![IdeAction::StagePaths {
            session_id: self.route_session(&session),
            paths: vec![row.path.clone()],
            stage: *bucket != Bucket::Staged,
        }]
    }

    pub fn staged_count(&self) -> usize {
        self.panel_session()
            .and_then(|id| self.git.get(&id))
            .map_or(0, |s| s.staged.len())
    }

    pub fn commit_now(&mut self) -> Vec<IdeAction> {
        let message = self.full_commit_message();
        let Some(session) = self.panel_session() else {
            return Vec::new();
        };
        if message.is_empty() {
            self.say(ToastLevel::Info, "write a commit message first".to_string());
            return Vec::new();
        }
        if self.staged_count() == 0 {
            self.say(
                ToastLevel::Info,
                "nothing is staged: stage a file with s first".to_string(),
            );
            return Vec::new();
        }
        self.pending_commit = Some((
            std::mem::take(&mut self.commit),
            self.tools.suggested_body.take(),
        ));
        vec![IdeAction::Commit {
            session_id: self.route_session(&session),
            message,
        }]
    }

    pub fn push_now(&mut self) -> Vec<IdeAction> {
        let Some(session) = self.panel_session() else {
            return Vec::new();
        };
        let ahead = self.git.get(&session).map_or(0, |s| s.ahead);
        if ahead == 0 {
            self.say(ToastLevel::Info, "no commits to push".to_string());
            return Vec::new();
        }
        vec![IdeAction::Push {
            session_id: self.route_session(&session),
        }]
    }

    // ------------------------------------------------------------------ text input

    pub fn type_char(&mut self, ch: char) {
        if self.composer.active {
            self.composer.push(ch);
        } else if self.tools.comment.active {
            self.tools.comment.push(ch);
        } else if self.commit.active {
            self.commit.push(ch);
        }
    }

    pub fn backspace(&mut self) {
        let input = if self.composer.active {
            &mut self.composer
        } else if self.tools.comment.active {
            &mut self.tools.comment
        } else {
            &mut self.commit
        };
        input.text.pop();
    }

    pub fn clear_input(&mut self) {
        let input = if self.composer.active {
            &mut self.composer
        } else if self.tools.comment.active {
            &mut self.tools.comment
        } else {
            &mut self.commit
        };
        input.text.clear();
    }

    pub fn paste(&mut self, text: &str) {
        for ch in text.chars() {
            if ch == '\n' || ch == '\r' {
                self.type_char(' ');
            } else {
                self.type_char(ch);
            }
        }
    }

    pub fn is_reviewed_file(&self, session_id: &str, file: &ChangedFile) -> bool {
        is_reviewed(&self.marks, session_id, &file.path, &file.hash)
    }
}
