//! The Files tab, the ship row and the coach line.

use std::collections::HashSet;

use super::*;
use crate::ide::wire::{WireEntries, WireEntry};

impl IdeApp {
    // ------------------------------------------------------------------ the Files tab

    pub fn files_rows(&self) -> Vec<FileRow> {
        let changed: HashSet<&str> = self.rows.iter().map(|r| r.path.as_str()).collect();
        let mut out = Vec::new();
        self.walk_dir("", 0, &changed, &mut out);
        out
    }

    pub(super) fn walk_dir(
        &self,
        dir: &str,
        depth: usize,
        changed: &HashSet<&str>,
        out: &mut Vec<FileRow>,
    ) {
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

    pub(super) fn fetch_dir(&mut self, dir: &str) -> Vec<IdeAction> {
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

    pub(super) fn apply_entries(&mut self, dir: String, result: Result<WireEntries, String>) {
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

    pub(super) fn apply_suggested(&mut self, result: Result<String, String>) {
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
}
