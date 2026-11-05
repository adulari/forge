//! The light editor: load, edit, save with the hash check, and the conflict prompt.

use super::*;
use crate::ide::edit_buf::EditBuf;
use crate::ide::wire::WireFile;

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

    pub(super) fn matches_edit(&self, session_id: &str, path: &str) -> bool {
        self.tools.edit.as_ref().is_some_and(|e| {
            e.path == path
                && self
                    .workspace_session(&e.session_id)
                    .is_some_and(|ws| ws == session_id)
        })
    }

    pub(super) fn apply_file(
        &mut self,
        session_id: &str,
        path: &str,
        result: Result<WireFile, String>,
    ) {
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

    pub(super) fn apply_saved(
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
    pub(super) fn edit_after_changes(&mut self) -> Vec<IdeAction> {
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
}
