//! What the user can *do* to a session from the board, expressed as [`BoardAction`]s for the
//! host. Every answer to a permission prompt or question echoes the `prompt_seq` the board saw,
//! exactly like the remote page — a stale keypress can never approve a newer prompt.

use super::model::{live_card, past_card, Card};
use super::state::{BoardApp, Confirm, ConfirmKind, Focus, ToastLevel, PAST_LIMIT};
use super::BoardAction;

impl BoardApp {
    // ------------------------------------------------------------ actions

    /// The seq-checked yes/no for the selected session's pending permission prompt.
    pub(crate) fn answer_permission(&mut self, yes: bool) -> Vec<BoardAction> {
        let Some(id) = self.selected.clone() else {
            return Vec::new();
        };
        let Some(snap) = self.snapshots.get(&id) else {
            return Vec::new();
        };
        if snap.permission_prompt.is_none() {
            return Vec::new();
        }
        let seq = snap.prompt_seq;
        self.toast(
            ToastLevel::Ok,
            if yes { "allowed" } else { "denied" }.to_string(),
        );
        vec![BoardAction::Input(
            id,
            serde_json::json!({ "kind": "allow", "yes": yes, "seq": seq }),
        )]
    }

    /// Pick option `n` (1-based) of the selected session's pending question.
    pub(crate) fn answer_option(&mut self, n: usize) -> Vec<BoardAction> {
        let Some(id) = self.selected.clone() else {
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
        self.toast(ToastLevel::Ok, format!("answered: {label}"));
        vec![BoardAction::Input(
            id,
            serde_json::json!({ "kind": "answer", "text": n.to_string(), "seq": seq }),
        )]
    }

    pub(crate) fn interrupt_selected(&mut self) -> Vec<BoardAction> {
        match self.selected_card() {
            Some(c) if c.busy && !c.past => {
                let id = c.id.clone();
                self.toast(ToastLevel::Info, "interrupting the current turn…");
                vec![BoardAction::Interrupt(id)]
            }
            Some(_) => {
                self.toast(ToastLevel::Info, "nothing running to interrupt");
                Vec::new()
            }
            None => Vec::new(),
        }
    }

    pub(crate) fn request_archive(&mut self) {
        let Some(c) = self.selected_card() else {
            return;
        };
        if c.past || c.terminal {
            self.toast(
                ToastLevel::Info,
                "only daemon-hosted live sessions can be archived",
            );
            return;
        }
        self.confirm = Some(Confirm {
            kind: ConfirmKind::Archive(c.id.clone()),
            title: "Archive this session?".into(),
            body: format!(
                "{} stops and leaves the board. Its history and worktree are kept; \
                 it can be resumed from Done.",
                c.display_title()
            ),
        });
        self.focus = Focus::Confirm;
    }

    pub(crate) fn resolve_confirm(&mut self, yes: bool) -> Vec<BoardAction> {
        let Some(c) = self.confirm.take() else {
            return Vec::new();
        };
        self.focus = if self.detail_open {
            Focus::Detail
        } else {
            Focus::Board
        };
        if !yes {
            return Vec::new();
        }
        match c.kind {
            ConfirmKind::Archive(id) => {
                self.toast(ToastLevel::Info, "archiving…");
                vec![BoardAction::Archive(id)]
            }
        }
    }

    /// `default → accept-edits → bypass → plan → default`.
    pub(crate) fn cycle_mode_selected(&mut self) -> Vec<BoardAction> {
        let Some(c) = self.selected_card() else {
            return Vec::new();
        };
        if c.past || c.terminal {
            self.toast(
                ToastLevel::Info,
                "mode can only change on daemon-hosted sessions",
            );
            return Vec::new();
        }
        let id = c.id.clone();
        let current = self
            .snapshots
            .get(&id)
            .map(|s| s.permission_mode.clone())
            .filter(|m| !m.is_empty())
            .or_else(|| {
                self.rows
                    .iter()
                    .find(|r| r.id == id)
                    .and_then(|r| r.permission_mode.clone())
            })
            .unwrap_or_else(|| "default".into());
        let next = match current.as_str() {
            "default" => "accept-edits",
            "accept-edits" => "bypass",
            "bypass" => "plan",
            _ => "default",
        };
        self.toast(ToastLevel::Info, format!("mode → {next}"));
        vec![BoardAction::SetMode(id, next.to_string())]
    }

    pub(crate) fn resume_selected(&mut self) -> Vec<BoardAction> {
        match self.selected_card() {
            Some(c) if c.past => {
                let id = c.id.clone();
                self.toast(ToastLevel::Info, "resuming…");
                vec![BoardAction::Resume(id)]
            }
            _ => Vec::new(),
        }
    }

    pub(crate) fn attach_selected(&mut self) -> Vec<BoardAction> {
        match self.selected_card() {
            Some(c) if !c.past && !c.read_only => vec![BoardAction::Attach(c.id.clone())],
            Some(c) if c.past => {
                self.toast(ToastLevel::Info, "resume it first (r), then attach");
                Vec::new()
            }
            Some(_) => {
                self.toast(ToastLevel::Info, "this session has no input path");
                Vec::new()
            }
            None => Vec::new(),
        }
    }

    pub(crate) fn cycle_project_filter(&mut self) {
        let all: Vec<Card> = {
            let saved = (self.project_filter.take(), std::mem::take(&mut self.query));
            let mut cards = Vec::new();
            for row in &self.rows {
                cards.push(live_card(row, self.snapshots.get(&row.id), self.now));
            }
            for row in self.past.iter().take(PAST_LIMIT) {
                cards.push(past_card(row));
            }
            self.project_filter = saved.0;
            self.query = saved.1;
            cards
        };
        let projects = super::model::projects(&all);
        self.project_filter = match &self.project_filter {
            None => projects.first().cloned(),
            Some(cur) => {
                let i = projects.iter().position(|p| p == cur);
                i.and_then(|i| projects.get(i + 1).cloned())
            }
        };
        self.rebuild();
    }

    /// The composer target for the selected live card, with a toast when there is none.
    pub(crate) fn writable_target(&mut self) -> Option<String> {
        match self.selected_card() {
            Some(c) if c.past => {
                self.toast(
                    ToastLevel::Info,
                    "this session is not running — resume it (r)",
                );
                None
            }
            Some(c) if c.read_only => {
                self.toast(ToastLevel::Info, "this session has no input path");
                None
            }
            Some(c) => Some(c.id.clone()),
            None => None,
        }
    }

    pub(crate) fn new_session_cwd(&self) -> String {
        self.selected_card()
            .map(|c| c.cwd.clone())
            .filter(|c| !c.is_empty())
            .or_else(|| self.board_cwd.clone())
            .unwrap_or_else(|| ".".into())
    }
}
