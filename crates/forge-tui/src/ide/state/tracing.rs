//! Why-trace and the review draft: who wrote a line, comments on hunks, sending a review.

use super::*;
use crate::ide::review::{
    build_review_prompt, build_single_comment_prompt, explain_prompt, LineKind, ReviewComment,
    ReviewLine, CONTEXT_LINES,
};
use crate::ide::timeline::item_for_turn;
use crate::ide::wire::BlameLine;

impl IdeApp {
    // ------------------------------------------------------------------ why-trace

    pub fn toggle_why(&mut self) -> Vec<IdeAction> {
        self.tools.why = !self.tools.why;
        if self.tools.why {
            self.ensure_blame()
        } else {
            self.tools.why_popup = None;
            Vec::new()
        }
    }

    pub(super) fn blame_key(&self) -> Option<(String, String)> {
        let open = self.open.as_ref()?;
        Some((self.workspace_session(&open.session_id)?, open.path.clone()))
    }

    pub(crate) fn ensure_blame(&mut self) -> Vec<IdeAction> {
        if !self.tools.why {
            return Vec::new();
        }
        let Some(key) = self.blame_key() else {
            return Vec::new();
        };
        let hash = self
            .open
            .as_ref()
            .and_then(|o| self.file_hash(&o.session_id, &o.path))
            .unwrap_or("")
            .to_string();
        let fresh = self.tools.blame.get(&key).is_some_and(|b| b.hash == hash);
        if fresh || !self.tools.blame_pending.insert(key.clone()) {
            return Vec::new();
        }
        vec![IdeAction::FetchBlame {
            session_id: key.0,
            path: key.1,
        }]
    }

    pub fn blame_for_open(&self) -> Option<&BlameData> {
        self.tools.blame.get(&self.blame_key()?)
    }

    pub(super) fn apply_blame(
        &mut self,
        session_id: String,
        path: String,
        result: Result<Vec<BlameLine>, String>,
    ) {
        let key = (session_id, path);
        self.tools.blame_pending.remove(&key);
        let hash = self
            .open
            .as_ref()
            .and_then(|o| self.file_hash(&o.session_id, &o.path))
            .unwrap_or("")
            .to_string();
        match result {
            Ok(lines) => {
                let lines = lines.into_iter().map(|l| (l.line, l)).collect();
                self.tools.blame.insert(key, BlameData { hash, lines });
            }
            Err(message) => {
                self.tools.why = false;
                self.say(
                    ToastLevel::Info,
                    format!("why-trace unavailable: {message}"),
                );
            }
        }
    }

    /// `W`: who wrote the lines under the cursor (the hunk in a diff, the line in the editor).
    pub fn open_why(&mut self) {
        if !self.tools.why {
            self.say(
                ToastLevel::Info,
                "press w first to show the why-trace".to_string(),
            );
            return;
        }
        let Some(blame) = self.blame_for_open() else {
            self.say(ToastLevel::Info, "reading the why-trace…".to_string());
            return;
        };
        let mut entries: Vec<BlameLine> = Vec::new();
        let mut push = |entry: &BlameLine| {
            let dup = entries.iter().any(|e| {
                e.turn == entry.turn
                    && e.tool_call_id == entry.tool_call_id
                    && e.session_id == entry.session_id
            });
            if !dup {
                entries.push(entry.clone());
            }
        };
        if self.editing() {
            let row = self
                .tools
                .edit
                .as_ref()
                .and_then(|e| e.buf.as_ref())
                .map_or(0, |b| b.cursor().0);
            if let Some(entry) = blame.lines.get(&(row + 1)) {
                push(entry);
            }
        } else if let Some((parsed, _, hunk)) = self.current_hunk() {
            for line in &parsed.overlay.hunks[hunk].lines {
                if let Some(entry) = line.new_no.and_then(|n| blame.lines.get(&(n as usize))) {
                    push(entry);
                }
            }
        }
        if entries.is_empty() {
            self.say(
                ToastLevel::Info,
                "no agent wrote these lines (or the edit is not recorded)".to_string(),
            );
            return;
        }
        self.tools.why_popup = Some(WhyPopup { entries, index: 0 });
    }

    pub fn move_why(&mut self, step: i32) {
        if let Some(popup) = self.tools.why_popup.as_mut() {
            let max = popup.entries.len() as i32 - 1;
            popup.index = (popup.index as i32 + step).clamp(0, max) as usize;
        }
    }

    /// Enter in the why popup: show that turn in the agent's timeline.
    pub fn jump_to_turn(&mut self) -> Vec<IdeAction> {
        let Some(popup) = self.tools.why_popup.take() else {
            return Vec::new();
        };
        let Some(entry) = popup.entries.get(popup.index).cloned() else {
            return Vec::new();
        };
        let mut actions = Vec::new();
        if self.panel_session().as_deref() != Some(entry.session_id.as_str()) {
            if let Some(index) = self.agents.iter().position(|a| a.id == entry.session_id) {
                self.agent_cursor = index;
                self.tools.pending_turn = Some((entry.session_id.clone(), entry.turn));
                actions.extend(self.sync_watch());
            } else {
                self.say(ToastLevel::Info, "that agent is not running".to_string());
                return actions;
            }
        }
        self.focus = Pane::Agent;
        if let Some(index) = item_for_turn(&self.timeline, entry.turn) {
            self.timeline_cursor = index;
            self.tools.pending_turn = None;
        } else if self.tools.pending_turn.is_none() {
            self.say(
                ToastLevel::Info,
                "that turn is older than the timeline".to_string(),
            );
        }
        actions
    }

    pub(crate) fn apply_pending_turn(&mut self) {
        let Some((session, turn)) = self.tools.pending_turn.clone() else {
            return;
        };
        if self.panel_session().as_deref() != Some(session.as_str()) {
            return;
        }
        if let Some(index) = item_for_turn(&self.timeline, turn) {
            self.timeline_cursor = index;
            self.tools.pending_turn = None;
        }
    }

    // ------------------------------------------------------------------ the review draft

    /// The agent a comment on this file goes to.
    pub(super) fn review_agent(&self, session_id: &str) -> Option<String> {
        self.workspace_session(session_id)
    }

    /// The agent a comment on `session_id`'s file is filed under (for the hunk markers).
    pub fn review_target_session(&self, session_id: &str) -> Option<String> {
        self.review_agent(session_id)
    }

    pub fn draft_count(&self) -> usize {
        self.panel_session()
            .map_or(0, |id| self.tools.drafts.count(&id))
    }

    /// `c`: comment on the hunk under the cursor.
    pub fn start_comment(&mut self) {
        let Some((parsed, bucket, index)) = self.current_hunk() else {
            self.say(
                ToastLevel::Info,
                "put the cursor on a hunk first (n / p)".to_string(),
            );
            return;
        };
        let Some(open) = &self.open else {
            return;
        };
        let Some(agent) = self.review_agent(&open.session_id) else {
            self.say(ToastLevel::Info, "pick an agent first".to_string());
            return;
        };
        let hunk = &parsed.overlay.hunks[index];
        let lines: Vec<ReviewLine> = hunk
            .lines
            .iter()
            .take(CONTEXT_LINES)
            .map(|l| ReviewLine {
                line_no: l.new_no.or(l.old_no).unwrap_or(0) as usize,
                kind: match l.kind {
                    crate::diff_overlay::LineKind::Add => LineKind::Add,
                    crate::diff_overlay::LineKind::Del => LineKind::Del,
                    crate::diff_overlay::LineKind::Context => LineKind::Context,
                },
                text: l.text.clone(),
            })
            .collect();
        let numbers = lines.iter().map(|l| l.line_no).filter(|n| *n > 0);
        let (start, end) = (
            numbers.clone().min().unwrap_or(0),
            numbers.max().unwrap_or(0),
        );
        self.tools.comment_target = Some(CommentTarget {
            session_id: agent,
            path: parsed.wire.path.clone(),
            header: hunk.header.clone(),
            staged: bucket == Bucket::Staged,
            start_line: start,
            end_line: end,
            lines,
        });
        self.tools.comment = Input {
            active: true,
            text: String::new(),
        };
    }

    pub fn cancel_comment(&mut self) {
        self.tools.comment = Input::default();
        self.tools.comment_target = None;
    }

    /// Enter adds the comment to the draft; `send_now` sends just this one.
    pub fn finish_comment(&mut self, send_now: bool) -> Vec<IdeAction> {
        let text = self.tools.comment.text.trim().to_string();
        let Some(target) = self.tools.comment_target.take() else {
            self.tools.comment = Input::default();
            return Vec::new();
        };
        self.tools.comment = Input::default();
        if text.is_empty() {
            return Vec::new();
        }
        let comment = ReviewComment {
            id: 0,
            session_id: target.session_id.clone(),
            path: target.path,
            header: target.header,
            staged: target.staged,
            start_line: target.start_line,
            end_line: target.end_line,
            lines: target.lines,
            text,
        };
        if send_now {
            if self.read_only_session(&comment.session_id) {
                self.say(
                    ToastLevel::Info,
                    "view only: this session runs in a terminal".to_string(),
                );
                return Vec::new();
            }
            self.advance_coach(2);
            self.say(ToastLevel::Ok, "comment sent".to_string());
            return vec![IdeAction::Input(
                comment.session_id.clone(),
                serde_json::json!({ "kind": "prompt", "text": build_single_comment_prompt(&comment) }),
            )];
        }
        let session = comment.session_id.clone();
        self.tools.drafts.add(comment);
        self.advance_coach(2);
        let count = self.tools.drafts.count(&session);
        self.say(
            ToastLevel::Ok,
            format!(
                "{}  (S sends the review)",
                crate::ide::review::draft_label(count)
            ),
        );
        Vec::new()
    }

    /// `S`: send every comment of the panel agent as one prompt.
    pub fn send_review(&mut self) -> Vec<IdeAction> {
        let Some(id) = self.panel_session() else {
            return Vec::new();
        };
        if self.tools.drafts.count(&id) == 0 {
            self.say(
                ToastLevel::Info,
                "no comments yet: c comments on a hunk".to_string(),
            );
            return Vec::new();
        }
        if self.read_only_session(&id) {
            self.say(
                ToastLevel::Info,
                "view only: this session runs in a terminal".to_string(),
            );
            return Vec::new();
        }
        let comments = self.tools.drafts.take(&id);
        let count = comments.len();
        self.advance_coach(2);
        self.say(
            ToastLevel::Ok,
            format!("review sent: {}", crate::ide::review::draft_label(count)),
        );
        vec![IdeAction::Input(
            id,
            serde_json::json!({ "kind": "prompt", "text": build_review_prompt(&comments) }),
        )]
    }

    pub fn drop_last_comment(&mut self) {
        let Some(id) = self.panel_session() else {
            return;
        };
        match self.tools.drafts.pop(&id) {
            Some(_) => self.say(ToastLevel::Info, "last comment removed".to_string()),
            None => self.say(ToastLevel::Info, "the draft is empty".to_string()),
        }
    }

    /// `b`: ask the agent, as a `/btw` side question, why the hunk under the cursor exists.
    pub fn explain_hunk(&mut self) -> Vec<IdeAction> {
        let Some((parsed, _, index)) = self.current_hunk() else {
            self.say(
                ToastLevel::Info,
                "put the cursor on a hunk first (n / p)".to_string(),
            );
            return Vec::new();
        };
        let Some(open) = &self.open else {
            return Vec::new();
        };
        let Some(agent) = self.review_agent(&open.session_id) else {
            return Vec::new();
        };
        if self.read_only_session(&agent) {
            self.say(
                ToastLevel::Info,
                "view only: this session runs in a terminal".to_string(),
            );
            return Vec::new();
        }
        let hunk = &parsed.wire.hunks[index];
        self.say(
            ToastLevel::Ok,
            "asked: the answer shows in the agent's transcript".to_string(),
        );
        vec![IdeAction::Input(
            agent,
            serde_json::json!({ "kind": "prompt", "text": explain_prompt(&parsed.wire.path, &hunk.header, &hunk.lines) }),
        )]
    }
}
