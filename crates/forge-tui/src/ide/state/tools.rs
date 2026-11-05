//! The review tools around the diff: the light editor, why-trace, the review draft, the bottom
//! panel (checks, shells, terminal), the Files tab, the ship row and the coach line. All pure:
//! they read and write [`IdeApp`] and answer with [`IdeAction`]s for the host.

use std::collections::{HashMap, HashSet};

use ratatui::text::Span;

use super::*;
use crate::ide::checks::{
    append_output, check_actions, check_state, failing_changed_files, fix_prompt, parse_file_refs,
    resolve_ref_path, CheckState, FileRef, WireAction,
};
use crate::ide::edit_buf::EditBuf;
use crate::ide::review::{
    build_review_prompt, build_single_comment_prompt, explain_prompt, Drafts, LineKind,
    ReviewComment, ReviewLine, CONTEXT_LINES,
};
use crate::ide::term_grid::TermGrid;
use crate::ide::timeline::item_for_turn;
use crate::ide::wire::{BlameLine, WireEntries, WireEntry, WireFile, WireJob};

/// Ticks (~100 ms) between background reads while a bottom tab is on screen.
const CHECKS_EVERY: u64 = 30;
const JOBS_EVERY: u64 = 20;
const TAIL_EVERY: u64 = 10;
/// A second Esc within this many ticks leaves the terminal.
const ESC_WINDOW: u64 = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Diff,
    Edit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveFailure {
    /// 409: the file changed on disk since it was loaded.
    Conflict,
    ReadOnly,
    TooLarge,
    Other(String),
}

pub struct HlCache {
    pub version: u64,
    pub first: usize,
    pub rows: Vec<Vec<Span<'static>>>,
}

pub struct EditState {
    pub session_id: String,
    pub path: String,
    pub buf: Option<EditBuf>,
    pub hash: String,
    pub loading: bool,
    pub error: Option<String>,
    pub read_only: bool,
    pub scroll: usize,
    /// The queue's content hash when the buffer loaded: when it moves an agent edited the file.
    pub queue_hash: Option<String>,
    /// 1-based line to put the cursor on once the file arrives.
    pub jump: Option<usize>,
    pub hl: Option<HlCache>,
    pub saving: bool,
}

impl EditState {
    fn new(session_id: String, path: String, queue_hash: Option<String>) -> Self {
        Self {
            session_id,
            path,
            buf: None,
            hash: String::new(),
            loading: true,
            error: None,
            read_only: false,
            scroll: 0,
            queue_hash,
            jump: None,
            hl: None,
            saving: false,
        }
    }

    pub fn dirty(&self) -> bool {
        self.buf.as_ref().is_some_and(EditBuf::is_dirty)
    }
}

#[derive(Debug, Clone, Default)]
pub struct BlameData {
    pub hash: String,
    pub lines: HashMap<usize, BlameLine>,
}

#[derive(Debug, Clone)]
pub struct WhyPopup {
    pub entries: Vec<BlameLine>,
    pub index: usize,
}

#[derive(Debug, Clone)]
pub struct CommentTarget {
    pub session_id: String,
    pub path: String,
    pub header: String,
    pub staged: bool,
    pub start_line: usize,
    pub end_line: usize,
    pub lines: Vec<ReviewLine>,
}

#[derive(Default)]
pub struct ChecksState {
    pub rows: Vec<WireAction>,
    pub loaded: bool,
    pub untrusted: bool,
    pub note: Option<String>,
    pub cursor: usize,
    pub outputs: HashMap<String, String>,
    pub refs: Vec<FileRef>,
    pub show_output: bool,
    pub ref_cursor: usize,
    pub failing: HashSet<String>,
    pub streamed: HashSet<String>,
    pub fetched_at: u64,
}

#[derive(Default)]
pub struct JobsState {
    pub rows: Vec<WireJob>,
    pub loaded: bool,
    pub note: Option<String>,
    pub cursor: usize,
    pub viewing: Option<u32>,
    pub tail: Vec<String>,
    pub tail_status: String,
    pub fetched_at: u64,
    pub tail_at: u64,
}

#[derive(Default)]
pub struct TermState {
    pub grid: Option<TermGrid>,
    pub opened: bool,
    pub ended: Option<Option<i64>>,
    pub focused: bool,
    pub view: (u16, u16),
    pub sent: (u16, u16),
    pub esc_tick: Option<u64>,
    pub session: Option<String>,
}

#[derive(Default)]
pub struct FilesState {
    pub dirs: HashMap<String, Vec<WireEntry>>,
    pub expanded: HashSet<String>,
    pub loading: HashSet<String>,
    pub cursor: usize,
    pub error: Option<String>,
    pub truncated: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRow {
    pub path: String,
    pub name: String,
    pub depth: usize,
    pub dir: bool,
    pub open: bool,
    pub changed: bool,
}

#[derive(Default)]
pub struct Tools {
    pub edit: Option<EditState>,
    pub why: bool,
    pub blame: HashMap<(String, String), BlameData>,
    pub blame_pending: HashSet<(String, String)>,
    pub why_popup: Option<WhyPopup>,
    pub pending_turn: Option<(String, i64)>,
    pub drafts: Drafts,
    pub comment: Input,
    pub comment_target: Option<CommentTarget>,
    pub checks: ChecksState,
    pub jobs: JobsState,
    pub term: TermState,
    pub files: FilesState,
    pub suggested_body: Option<(String, String)>,
    pub coach: Option<usize>,
    pub coach_loaded: bool,
}

/// What the coach line says at each step, and what the user does to move past it.
pub const COACH: [(&str, &str); 3] = [
    (
        "Changes queue: files that need you first. j opens the first one",
        "open a file",
    ),
    (
        "Review the diff: n/p hunks, a accept, x revert, c comment, e edit",
        "act on a hunk",
    ),
    (
        "Send it back: S sends your comments as one review, m messages the agent",
        "send",
    ),
];

impl IdeApp {
    // ------------------------------------------------------------------ the editor

    pub fn editing(&self) -> bool {
        self.open.as_ref().is_some_and(|o| o.view == View::Edit) && self.tools.edit.is_some()
    }

    pub fn edit_dirty(&self) -> bool {
        self.tools.edit.as_ref().is_some_and(EditState::dirty)
    }

    pub(crate) fn tracked(&self, session_id: &str, path: &str) -> bool {
        self.file_hash(session_id, path).is_some()
    }

    /// The session that owns workspace reads and writes for `session_id`.
    pub(crate) fn workspace_session(&self, session_id: &str) -> Option<String> {
        if session_id != UNATTRIBUTED {
            return Some(session_id.to_string());
        }
        let routed = self.route_session(session_id);
        (routed != UNATTRIBUTED)
            .then_some(routed)
            .or_else(|| self.panel_session())
    }

    /// `e`: edit the open file here.
    pub fn start_edit(&mut self, jump: Option<usize>) -> Vec<IdeAction> {
        let Some(open) = self.open.as_mut() else {
            self.say(ToastLevel::Info, "open a file first".to_string());
            return Vec::new();
        };
        open.view = View::Edit;
        let (session_id, path) = (open.session_id.clone(), open.path.clone());
        self.focus = Pane::Editor;
        let same = self
            .tools
            .edit
            .as_ref()
            .is_some_and(|e| e.session_id == session_id && e.path == path);
        if same {
            if let (Some(edit), Some(line)) = (self.tools.edit.as_mut(), jump) {
                edit.jump = Some(line);
                if let Some(buf) = edit.buf.as_mut() {
                    buf.set_cursor(line.saturating_sub(1), 0);
                    edit.jump = None;
                    edit.scroll = line.saturating_sub(1).saturating_sub(self.view_h / 3);
                }
            }
            return Vec::new();
        }
        let Some(ws) = self.workspace_session(&session_id) else {
            self.say(ToastLevel::Info, "pick an agent first".to_string());
            return Vec::new();
        };
        let queue_hash = self.file_hash(&session_id, &path).map(str::to_string);
        let mut edit = EditState::new(session_id, path.clone(), queue_hash);
        edit.jump = jump;
        self.tools.edit = Some(edit);
        vec![IdeAction::FetchFile {
            session_id: ws,
            path,
        }]
    }

    /// Open `path` at `line` in the editor (a check's `file:line`, a why-trace line).
    pub fn open_at(&mut self, session_id: String, path: String, line: usize) -> Vec<IdeAction> {
        let mut actions = self.open_file(session_id, path);
        if self.open.is_some() && !self.edit_dirty() {
            actions.extend(self.start_edit(Some(line)));
        }
        actions
    }

    /// Run one buffer operation and keep the cursor on screen; `mutating` ones refuse a read-only
    /// file.
    pub fn edit_op(&mut self, mutating: bool, f: impl FnOnce(&mut EditBuf)) {
        let view_h = self.view_h;
        let read_only = self.tools.edit.as_ref().is_some_and(|e| e.read_only);
        if mutating && read_only {
            self.say(ToastLevel::Info, "this file is read-only".to_string());
            return;
        }
        let Some(edit) = self.tools.edit.as_mut() else {
            return;
        };
        let Some(buf) = edit.buf.as_mut() else {
            return;
        };
        f(buf);
        buf.scroll_to_cursor(&mut edit.scroll, view_h);
    }

    pub fn edit_scroll_by(&mut self, delta: i32) {
        let view_h = self.view_h.max(1);
        let Some(edit) = self.tools.edit.as_mut() else {
            return;
        };
        let max = edit
            .buf
            .as_ref()
            .map_or(0, |b| b.line_count().saturating_sub(view_h));
        edit.scroll = (edit.scroll as i64 + i64::from(delta)).clamp(0, max as i64) as usize;
    }

    /// Esc in the editor: back to the diff (or the empty editor for an unchanged file).
    pub fn leave_edit(&mut self) {
        self.tools.edit = None;
        let tracked = self
            .open
            .as_ref()
            .is_some_and(|o| self.tracked(&o.session_id, &o.path));
        if tracked {
            if let Some(open) = self.open.as_mut() {
                open.view = View::Diff;
            }
        } else {
            self.open = None;
        }
    }

    /// Esc while editing: leave, or ask first when there are unsaved edits.
    pub fn request_leave_edit(&mut self) {
        if self.edit_dirty() {
            self.confirm = Some(Confirm {
                kind: ConfirmKind::DiscardEdits,
                text: "discard unsaved edits?  y/n   (Ctrl+S saves)".to_string(),
            });
        } else {
            self.leave_edit();
        }
    }

    /// Ctrl+S; `overwrite` takes the file's current hash first (after a conflict).
    pub fn save_edit(&mut self, overwrite: bool) -> Vec<IdeAction> {
        let Some(edit) = self.tools.edit.as_mut() else {
            return Vec::new();
        };
        if edit.read_only {
            self.say(ToastLevel::Info, "this file is read-only".to_string());
            return Vec::new();
        }
        let Some(buf) = edit.buf.as_ref() else {
            return Vec::new();
        };
        if !overwrite && !buf.is_dirty() {
            self.say(ToastLevel::Info, "nothing to save".to_string());
            return Vec::new();
        }
        if edit.saving {
            return Vec::new();
        }
        edit.saving = true;
        let content = buf.to_text();
        let expected = (!overwrite).then(|| edit.hash.clone());
        let (session_id, path) = (edit.session_id.clone(), edit.path.clone());
        let Some(ws) = self.workspace_session(&session_id) else {
            return Vec::new();
        };
        vec![IdeAction::SaveFile {
            session_id: ws,
            path,
            content,
            expected_hash: expected,
        }]
    }

    pub fn reload_edit(&mut self) -> Vec<IdeAction> {
        let Some(edit) = self.tools.edit.as_mut() else {
            return Vec::new();
        };
        edit.loading = true;
        edit.buf = None;
        edit.hl = None;
        edit.jump = None;
        edit.queue_hash = None;
        let (session_id, path) = (edit.session_id.clone(), edit.path.clone());
        let queue = self.file_hash(&session_id, &path).map(str::to_string);
        if let Some(edit) = self.tools.edit.as_mut() {
            edit.queue_hash = queue;
        }
        let Some(ws) = self.workspace_session(&session_id) else {
            return Vec::new();
        };
        vec![IdeAction::FetchFile {
            session_id: ws,
            path,
        }]
    }

    /// `E`: hand the file to `$EDITOR` (the host leaves the screen, edits, saves with the hash
    /// check and reloads).
    pub fn external_edit(&mut self) -> Vec<IdeAction> {
        let Some(open) = &self.open else {
            self.say(ToastLevel::Info, "open a file first".to_string());
            return Vec::new();
        };
        if self.edit_dirty() {
            self.say(
                ToastLevel::Info,
                "save first (Ctrl+S) or discard your edits (Esc)".to_string(),
            );
            return Vec::new();
        }
        let Some(ws) = self.workspace_session(&open.session_id) else {
            self.say(ToastLevel::Info, "pick an agent first".to_string());
            return Vec::new();
        };
        vec![IdeAction::ExternalEdit {
            session_id: ws,
            path: open.path.clone(),
        }]
    }

    fn matches_edit(&self, session_id: &str, path: &str) -> bool {
        self.tools.edit.as_ref().is_some_and(|e| {
            e.path == path
                && self
                    .workspace_session(&e.session_id)
                    .is_some_and(|ws| ws == session_id)
        })
    }

    fn apply_file(&mut self, session_id: &str, path: &str, result: Result<WireFile, String>) {
        if !self.matches_edit(session_id, path) {
            return;
        }
        let view_h = self.view_h;
        let Some(edit) = self.tools.edit.as_mut() else {
            return;
        };
        edit.loading = false;
        match result {
            Ok(file) => {
                let mut buf = EditBuf::new(&file.content);
                if let Some(line) = edit.jump.take() {
                    buf.set_cursor(line.saturating_sub(1), 0);
                    edit.scroll = line.saturating_sub(1).saturating_sub(view_h / 3);
                } else {
                    edit.scroll = 0;
                }
                edit.buf = Some(buf);
                edit.hash = file.hash;
                edit.read_only = file.read_only;
                edit.error = None;
                edit.hl = None;
            }
            Err(message) => edit.error = Some(message),
        }
    }

    fn apply_saved(
        &mut self,
        session_id: &str,
        path: &str,
        result: Result<String, SaveFailure>,
    ) -> Vec<IdeAction> {
        if !self.matches_edit(session_id, path) {
            return Vec::new();
        }
        let Some(edit) = self.tools.edit.as_mut() else {
            return Vec::new();
        };
        edit.saving = false;
        match result {
            Ok(hash) => {
                edit.hash = hash;
                if let Some(buf) = edit.buf.as_mut() {
                    buf.mark_saved();
                }
                self.say(ToastLevel::Ok, format!("saved {path}"));
                let mut actions = vec![IdeAction::RefreshChanges];
                if let Some(id) = self.panel_session() {
                    actions.push(IdeAction::FetchGitStatus(self.route_session(&id)));
                }
                actions
            }
            Err(SaveFailure::Conflict) => {
                self.confirm = Some(Confirm {
                    kind: ConfirmKind::SaveConflict,
                    text: "agent changed this file:  r reload  /  o overwrite  /  Esc keep editing"
                        .to_string(),
                });
                Vec::new()
            }
            Err(SaveFailure::ReadOnly) => {
                self.say(
                    ToastLevel::Error,
                    "the daemon refused: read-only".to_string(),
                );
                Vec::new()
            }
            Err(SaveFailure::TooLarge) => {
                self.say(ToastLevel::Error, "file too large to save here".to_string());
                Vec::new()
            }
            Err(SaveFailure::Other(message)) => {
                self.say(ToastLevel::Error, message);
                Vec::new()
            }
        }
    }

    /// After a changes read: an agent edited the open buffer's file.
    fn edit_after_changes(&mut self) -> Vec<IdeAction> {
        let Some(edit) = &self.tools.edit else {
            return Vec::new();
        };
        let now = self
            .file_hash(&edit.session_id, &edit.path)
            .map(str::to_string);
        if now.is_none() || now == edit.queue_hash || edit.loading {
            return Vec::new();
        }
        if edit.dirty() {
            if let Some(edit) = self.tools.edit.as_mut() {
                edit.queue_hash = now;
            }
            self.say(
                ToastLevel::Info,
                "agent changed this file: saving will ask what to do".to_string(),
            );
            return Vec::new();
        }
        self.say(
            ToastLevel::Info,
            "agent changed this file: reloaded".to_string(),
        );
        self.reload_edit()
    }

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

    fn blame_key(&self) -> Option<(String, String)> {
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

    fn apply_blame(
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
    fn review_agent(&self, session_id: &str) -> Option<String> {
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

    fn recompute_failing(&mut self) {
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

    fn refresh_refs(&mut self) {
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

    fn apply_checks(
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
    fn sync_term(&mut self) -> Vec<IdeAction> {
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

    // ------------------------------------------------------------------ the Files tab

    pub fn files_rows(&self) -> Vec<FileRow> {
        let changed: HashSet<&str> = self.rows.iter().map(|r| r.path.as_str()).collect();
        let mut out = Vec::new();
        self.walk_dir("", 0, &changed, &mut out);
        out
    }

    fn walk_dir(&self, dir: &str, depth: usize, changed: &HashSet<&str>, out: &mut Vec<FileRow>) {
        let Some(entries) = self.tools.files.dirs.get(dir) else {
            return;
        };
        let mut sorted: Vec<&WireEntry> = entries.iter().collect();
        sorted.sort_by(|a, b| {
            (a.kind != "directory")
                .cmp(&(b.kind != "directory"))
                .then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        for entry in sorted {
            let dir_entry = entry.kind == "directory";
            let open = dir_entry && self.tools.files.expanded.contains(&entry.path);
            let prefix = format!("{}/", entry.path);
            out.push(FileRow {
                path: entry.path.clone(),
                name: entry.name.clone(),
                depth,
                dir: dir_entry,
                open,
                changed: if dir_entry {
                    changed.iter().any(|p| p.starts_with(&prefix))
                } else {
                    changed.contains(entry.path.as_str())
                },
            });
            if open {
                self.walk_dir(&entry.path, depth + 1, changed, out);
            }
        }
    }

    pub fn files_session(&self) -> Option<String> {
        self.panel_session().map(|id| self.route_session(&id))
    }

    pub fn ensure_files(&mut self) -> Vec<IdeAction> {
        if self.tools.files.dirs.contains_key("") || self.tools.files.loading.contains("") {
            return Vec::new();
        }
        self.fetch_dir("")
    }

    fn fetch_dir(&mut self, dir: &str) -> Vec<IdeAction> {
        let Some(session_id) = self.files_session() else {
            return Vec::new();
        };
        if !self.tools.files.loading.insert(dir.to_string()) {
            return Vec::new();
        }
        vec![IdeAction::FetchEntries {
            session_id,
            dir: dir.to_string(),
        }]
    }

    pub fn move_file(&mut self, step: i32) {
        let len = self.files_rows().len();
        if len > 0 {
            let at = self.tools.files.cursor as i32 + step;
            self.tools.files.cursor = at.clamp(0, len as i32 - 1) as usize;
        }
    }

    /// Enter on a Files row: expand a folder, open a file (its diff when it has changes).
    pub fn open_file_row(&mut self, index: usize) -> Vec<IdeAction> {
        let rows = self.files_rows();
        let Some(row) = rows.get(index).cloned() else {
            return Vec::new();
        };
        self.tools.files.cursor = index;
        if row.dir {
            if self.tools.files.expanded.remove(&row.path) {
                return Vec::new();
            }
            self.tools.files.expanded.insert(row.path.clone());
            if self.tools.files.dirs.contains_key(&row.path) {
                return Vec::new();
            }
            return self.fetch_dir(&row.path);
        }
        let owner = self
            .rows
            .iter()
            .find(|r| r.path == row.path)
            .map(|r| r.session_id.clone())
            .or_else(|| self.panel_session());
        let Some(owner) = owner else {
            self.say(ToastLevel::Info, "pick an agent first".to_string());
            return Vec::new();
        };
        self.focus = Pane::Editor;
        self.open_file(owner, row.path)
    }

    fn apply_entries(&mut self, dir: String, result: Result<WireEntries, String>) {
        self.tools.files.loading.remove(&dir);
        match result {
            Ok(found) => {
                self.tools.files.error = None;
                if dir.is_empty() {
                    self.tools.files.truncated = found.truncated;
                }
                self.tools.files.dirs.insert(dir, found.entries);
            }
            Err(message) => self.tools.files.error = Some(message),
        }
    }

    // ------------------------------------------------------------------ ship

    pub fn git_counts(&self) -> (usize, usize, bool) {
        let Some(status) = self.panel_session().and_then(|id| self.git.get(&id)) else {
            return (0, 0, false);
        };
        (
            status.staged.len(),
            status.unstaged.len() + status.untracked.len(),
            status.has_remote,
        )
    }

    pub fn ship_steps(&self) -> Vec<&'static str> {
        let (staged, unstaged, can_push) = self.git_counts();
        if staged + unstaged == 0 {
            return Vec::new();
        }
        let mut steps = Vec::new();
        if staged == 0 {
            steps.push("stage");
        }
        if self.commit.text.trim().is_empty() {
            steps.push("write a message");
        }
        steps.push("commit");
        if can_push {
            steps.push("push");
        }
        steps
    }

    pub fn ship_hint(&self) -> String {
        let mut words = self.ship_steps();
        let Some(last) = words.pop() else {
            return "Nothing to ship: the working tree is clean".to_string();
        };
        if words.is_empty() {
            format!("Will {last}")
        } else {
            format!("Will {} and {last}", words.join(", "))
        }
    }

    /// `g`: write a commit message for what is staged.
    pub fn suggest_message(&mut self) -> Vec<IdeAction> {
        let Some(id) = self.panel_session() else {
            return Vec::new();
        };
        if self.staged_count() == 0 {
            self.say(
                ToastLevel::Info,
                "stage a file first (s), then ask for a message".to_string(),
            );
            return Vec::new();
        }
        self.say(ToastLevel::Info, "writing a commit message…".to_string());
        vec![IdeAction::SuggestMessage(self.route_session(&id))]
    }

    fn apply_suggested(&mut self, result: Result<String, String>) {
        match result {
            Ok(message) => {
                let mut parts = message.trim().splitn(2, '\n');
                let subject = parts.next().unwrap_or("").trim().to_string();
                let body = parts.next().unwrap_or("").trim().to_string();
                self.commit.text = subject.clone();
                self.tools.suggested_body = (!body.is_empty()).then_some((subject, body));
                self.say(
                    ToastLevel::Ok,
                    "message ready: c edits it, Enter commits".to_string(),
                );
            }
            Err(e) => self.say(ToastLevel::Error, e),
        }
    }

    /// The commit message with its body, while the subject is the suggested one.
    pub fn full_commit_message(&self) -> String {
        let subject = self.commit.text.trim();
        match &self.tools.suggested_body {
            Some((suggested, body)) if suggested == subject => format!("{subject}\n\n{body}"),
            _ => subject.to_string(),
        }
    }

    /// `Z`: stage (when nothing is), write a message (when empty), commit, push.
    pub fn ship(&mut self) -> Vec<IdeAction> {
        let Some(id) = self.panel_session() else {
            return Vec::new();
        };
        let steps = self.ship_steps();
        if steps.is_empty() {
            self.say(ToastLevel::Info, "nothing to ship".to_string());
            return Vec::new();
        }
        let (staged, _, can_push) = self.git_counts();
        let stage = if staged == 0 {
            self.git_list()
                .into_iter()
                .filter(|(b, conflicted, _)| *b != Bucket::Staged && !conflicted)
                .map(|(_, _, row)| row.path)
                .collect()
        } else {
            Vec::new()
        };
        let message = self.full_commit_message();
        let message = (!message.is_empty()).then_some(message);
        self.commit = Input::default();
        self.tools.suggested_body = None;
        self.say(ToastLevel::Info, "shipping…".to_string());
        vec![IdeAction::Ship {
            session_id: self.route_session(&id),
            stage,
            message,
            push: can_push,
        }]
    }

    // ------------------------------------------------------------------ the coach line

    pub fn coach_line(&self) -> Option<(usize, &'static str)> {
        let step = self.tools.coach?;
        COACH.get(step).map(|(text, _)| (step, *text))
    }

    /// Move the coach on when the user did what step `done` asked for.
    pub fn advance_coach(&mut self, done: usize) -> Vec<IdeAction> {
        let Some(step) = self.tools.coach else {
            return Vec::new();
        };
        if step != done.min(2) && !(done > step) {
            return Vec::new();
        }
        let next = done + 1;
        if next >= COACH.len() {
            return self.dismiss_coach();
        }
        self.tools.coach = Some(next);
        Vec::new()
    }

    pub fn dismiss_coach(&mut self) -> Vec<IdeAction> {
        if self.tools.coach.take().is_none() {
            return Vec::new();
        }
        vec![IdeAction::SavePrefs(
            serde_json::json!({ "coach_seen": true }).to_string(),
        )]
    }

    // ------------------------------------------------------------------ events and ticks

    pub(crate) fn apply_tool_event(&mut self, event: IdeEvent) -> Vec<IdeAction> {
        let mut actions = Vec::new();
        match event {
            IdeEvent::File {
                session_id,
                path,
                result,
            } => self.apply_file(&session_id, &path, result),
            IdeEvent::Saved {
                session_id,
                path,
                result,
            } => actions.extend(self.apply_saved(&session_id, &path, result)),
            IdeEvent::Blame {
                session_id,
                path,
                result,
            } => self.apply_blame(session_id, path, result),
            IdeEvent::Checks(_, result) => actions.extend(self.apply_checks(result)),
            IdeEvent::CheckOutput {
                action,
                text,
                reset,
                ..
            } => {
                if reset {
                    self.tools.checks.outputs.remove(&action);
                }
                append_output(self.tools.checks.outputs.entry(action).or_default(), &text);
                self.refresh_refs();
                self.recompute_failing();
            }
            IdeEvent::Jobs(_, Ok(rows)) => {
                self.tools.jobs.rows = rows;
                self.tools.jobs.loaded = true;
                self.tools.jobs.note = None;
                self.tools.jobs.cursor = self
                    .tools
                    .jobs
                    .cursor
                    .min(self.tools.jobs.rows.len().saturating_sub(1));
            }
            IdeEvent::Jobs(_, Err(message)) => {
                self.tools.jobs.loaded = true;
                self.tools.jobs.note = Some(message);
            }
            IdeEvent::JobTail { job, result, .. } => {
                if self.tools.jobs.viewing == Some(job) {
                    match result {
                        Ok(tail) => {
                            self.tools.jobs.tail = tail.text.lines().map(str::to_string).collect();
                            self.tools.jobs.tail_status = tail.status;
                        }
                        Err(message) => self.tools.jobs.note = Some(message),
                    }
                }
            }
            IdeEvent::Entries { dir, result, .. } => self.apply_entries(dir, result),
            IdeEvent::Suggested(_, result) => self.apply_suggested(result),
            IdeEvent::TermBytes(bytes) => {
                if let Some(grid) = self.tools.term.grid.as_mut() {
                    grid.feed(&bytes);
                }
            }
            IdeEvent::TermEnded(code) => {
                self.tools.term.ended = Some(code);
            }
            IdeEvent::Handoff(Ok((cwd, command))) => {
                let id = self.panel_session().unwrap_or_default();
                actions.extend(self.open_terminal_with(&id));
                let quoted = format!("'{}'", cwd.replace('\'', "'\\''"));
                actions.push(IdeAction::TermInput(
                    format!("cd {quoted} && {command}\n").into_bytes(),
                ));
            }
            IdeEvent::Handoff(Err(message)) => self.say(ToastLevel::Error, message),
            IdeEvent::Prefs { coach_seen } => {
                self.tools.coach_loaded = true;
                self.tools.coach = (!coach_seen).then_some(0);
            }
            _ => {}
        }
        actions
    }

    pub(crate) fn tools_after_changes(&mut self) -> Vec<IdeAction> {
        let mut actions = self.edit_after_changes();
        actions.extend(self.ensure_blame());
        actions
    }

    pub(crate) fn tools_tick(&mut self) -> Vec<IdeAction> {
        let mut actions = self.sync_term();
        if let Some(open) = self.open.as_ref() {
            if open.view == View::Diff {
                actions.extend(self.ensure_blame());
            }
        }
        if !self.bottom_open {
            return actions;
        }
        let Some(id) = self.panel_session() else {
            return actions;
        };
        let session = self.route_session(&id);
        match self.bottom_tab {
            BottomTab::Checks => {
                let busy = self.tools.checks.rows.iter().any(|r| r.running);
                let every = if busy { CHECKS_EVERY / 3 } else { CHECKS_EVERY };
                if self.tick.saturating_sub(self.tools.checks.fetched_at) >= every {
                    self.tools.checks.fetched_at = self.tick;
                    actions.push(IdeAction::FetchChecks(session));
                }
            }
            BottomTab::Shells => {
                if self.tick.saturating_sub(self.tools.jobs.fetched_at) >= JOBS_EVERY {
                    self.tools.jobs.fetched_at = self.tick;
                    actions.push(IdeAction::FetchJobs(session.clone()));
                }
                if let Some(job) = self.tools.jobs.viewing {
                    if self.tick.saturating_sub(self.tools.jobs.tail_at) >= TAIL_EVERY {
                        self.tools.jobs.tail_at = self.tick;
                        actions.push(IdeAction::FetchJobTail {
                            session_id: session,
                            job,
                        });
                    }
                }
            }
            BottomTab::Terminal => {}
        }
        actions
    }

    /// Something moves on screen without input (a running check, a followed log, a terminal).
    pub(crate) fn tools_animating(&self) -> bool {
        self.bottom_open
            && match self.bottom_tab {
                BottomTab::Checks => self.tools.checks.rows.iter().any(|r| r.running),
                BottomTab::Shells => {
                    self.tools.jobs.rows.iter().any(|j| j.running)
                        || self.tools.jobs.viewing.is_some()
                }
                BottomTab::Terminal => false,
            }
    }
}
