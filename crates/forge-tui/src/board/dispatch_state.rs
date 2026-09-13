//! The board's plan-and-dispatch state and everything the user can do with it: review and approve
//! a proposed split (with dependency-aware ticking), revise or cancel it, follow a running one
//! into its workers, merge or discard worktrees, and zoom the board down to one dispatch.
//!
//! Like `actions.rs`, every intent comes out as a [`BoardAction`]; nothing here talks to the
//! daemon.

use std::collections::{BTreeSet, HashMap};

use crossterm::event::{KeyCode, KeyEvent};

use super::dispatch::{self, is_proposed, is_running, item_status, list_words, CardDispatch, Role};
use super::form::DispatchForm;
use super::model::{project_name, Card};
use super::state::{BoardApp, ComposerMode, Confirm, ConfirmKind, DetailTab, Focus, ToastLevel};
use super::wire::DispatchInfo;
use super::BoardAction;

#[derive(Debug, Default)]
pub struct DispatchUi {
    /// The latest `GET /api/dispatches`.
    pub(crate) list: Vec<DispatchInfo>,
    pub(crate) form: Option<DispatchForm>,
    /// Per proposed dispatch: the proposal it was ticked against, and the ticked item numbers.
    pub(crate) selections: HashMap<String, (String, BTreeSet<usize>)>,
    /// The Dispatch tab's row cursor (a position in the item list).
    pub(crate) cursor: usize,
    /// The item number whose full prompt is unfolded in the checklist.
    pub(crate) expanded: Option<usize>,
    /// Show only this dispatch's cards.
    pub(crate) zoom: Option<String>,
    /// A coordinator the user just started: select it the moment it appears.
    pub(crate) pending_select: Option<String>,
    /// Where the last dispatch was started, so a project filter cannot hide its coordinator.
    pub(crate) started_cwd: Option<String>,
}

impl BoardApp {
    pub fn dispatches(&self) -> &[DispatchInfo] {
        &self.dispatch.list
    }

    pub(crate) fn apply_dispatches(&mut self, list: Vec<DispatchInfo>) {
        let mut kept = HashMap::new();
        for d in list.iter().filter(|d| is_proposed(d)) {
            let sig = dispatch::proposal_signature(d);
            let entry = match self.dispatch.selections.remove(&d.id) {
                Some((old, set)) if old == sig => (old, set),
                _ => (sig, d.items.iter().map(|i| i.index).collect()),
            };
            kept.insert(d.id.clone(), entry);
        }
        self.dispatch.selections = kept;
        self.dispatch.list = list;
        self.rebuild();
    }

    pub(crate) fn dispatch_started(&mut self, coordinator: String) {
        self.dispatch.pending_select = Some(coordinator);
        self.dispatch.zoom = None;
        self.query.clear();
        let cwd = self.dispatch.started_cwd.take();
        if let (Some(filter), Some(cwd)) = (&self.project_filter, cwd) {
            if *filter != project_name(&cwd) {
                self.project_filter = None;
            }
        }
        self.rebuild();
    }

    /// Called at the end of every rebuild: the moment the new coordinator's card exists, it is
    /// selected and its pane opens on the Dispatch tab.
    pub(crate) fn try_pending_select(&mut self) {
        let Some(id) = self.dispatch.pending_select.clone() else {
            return;
        };
        let Some(col) = self.card(&id).map(|c| c.column) else {
            return;
        };
        self.dispatch.pending_select = None;
        self.selected = Some(id);
        self.cursor_col = col;
        self.detail_open = true;
        self.detail_tab = DetailTab::Dispatch;
        self.detail_scroll = 0;
        self.dispatch.cursor = 0;
        self.dispatch.expanded = None;
        if matches!(self.focus, Focus::Board | Focus::Detail) {
            self.focus = Focus::Detail;
        }
    }

    pub fn card_dispatch(&self, card: &Card) -> Option<&DispatchInfo> {
        let id = &card.dispatch.as_ref()?.id;
        self.dispatch.list.iter().find(|d| &d.id == id)
    }

    fn selected_membership(&self) -> Option<(&DispatchInfo, &CardDispatch)> {
        let card = self.selected_card()?;
        Some((self.card_dispatch(card)?, card.dispatch.as_ref()?))
    }

    /// The tab actually drawn: a remembered Dispatch tab falls back to Overview on a card that
    /// has no dispatch.
    pub fn effective_tab(&self) -> DetailTab {
        let has = self.selected_card().is_some_and(|c| c.dispatch.is_some());
        if self.detail_tab == DetailTab::Dispatch && !has {
            DetailTab::Overview
        } else {
            self.detail_tab
        }
    }

    pub(crate) fn dispatch_tab_active(&self) -> bool {
        self.detail_open
            && self.effective_tab() == DetailTab::Dispatch
            && self.selected_membership().is_some()
    }

    pub fn dispatch_selection(&self, dispatch_id: &str) -> Option<&BTreeSet<usize>> {
        self.dispatch.selections.get(dispatch_id).map(|(_, s)| s)
    }

    /// A dispatch card opened from Overview lands on its Dispatch tab — that is what it is for.
    pub(crate) fn prefer_dispatch_tab(&mut self) {
        let has = self.selected_card().is_some_and(|c| c.dispatch.is_some());
        if has && self.detail_tab == DetailTab::Overview {
            self.detail_tab = DetailTab::Dispatch;
        }
    }

    /// A worker's Dispatch tab starts on its own row.
    pub(crate) fn sync_dispatch_cursor(&mut self) {
        self.dispatch.expanded = None;
        self.dispatch.cursor = match self.selected_membership() {
            Some((
                d,
                CardDispatch {
                    role: Role::Worker(n),
                    ..
                },
            )) => d.items.iter().position(|i| i.index == *n).unwrap_or(0),
            _ => 0,
        };
    }

    pub(crate) fn move_dispatch_cursor(&mut self, delta: i32) {
        let len = self.selected_membership().map_or(0, |(d, _)| d.items.len());
        if len == 0 {
            return;
        }
        let next = (self.dispatch.cursor as i32 + delta).clamp(0, len as i32 - 1);
        self.dispatch.cursor = next as usize;
    }

    /// Space / a click on `[✓]`: tick or untick one item of a proposed split, dragging its
    /// dependencies or dependants along and saying so.
    pub(crate) fn toggle_dispatch_item(&mut self, pos: usize) {
        let Some((d, _)) = self.selected_membership() else {
            return;
        };
        if !is_proposed(d) {
            return;
        }
        let Some(item) = d.items.get(pos) else {
            return;
        };
        let (id, index, items) = (d.id.clone(), item.index, d.items.clone());
        self.dispatch.cursor = pos;
        let Some((_, set)) = self.dispatch.selections.get_mut(&id) else {
            return;
        };
        let (now_on, also) = dispatch::toggle(&items, set, index);
        if also.is_empty() {
            return;
        }
        let (verb, plural) = (if now_on { "ticked" } else { "unticked" }, also.len() > 1);
        let why = match (now_on, plural) {
            (true, true) => format!("{index} needs them"),
            (true, false) => format!("{index} needs it"),
            (false, true) => format!("they need {index}"),
            (false, false) => format!("it needs {index}"),
        };
        self.toast(
            ToastLevel::Info,
            format!("also {verb} {} — {why}", list_words(&also)),
        );
    }

    /// Enter on a row: unfold a proposed item's prompt, or jump to a running item's session.
    pub(crate) fn activate_dispatch_row(&mut self, pos: usize) -> Vec<BoardAction> {
        let Some((d, _)) = self.selected_membership() else {
            return Vec::new();
        };
        let Some(item) = d.items.get(pos) else {
            return Vec::new();
        };
        let (proposed, index, session) = (is_proposed(d), item.index, item.session_id.clone());
        self.dispatch.cursor = pos;
        if proposed {
            self.dispatch.expanded = (self.dispatch.expanded != Some(index)).then_some(index);
            return Vec::new();
        }
        match session {
            Some(sid) if self.card(&sid).is_some() => {
                self.select(&sid);
                self.detail_open = true;
                self.focus = Focus::Detail;
                self.detail_tab = DetailTab::Tail;
                self.detail_actions_for(&sid)
            }
            Some(_) => {
                self.toast(
                    ToastLevel::Info,
                    format!("{index}'s session left the board"),
                );
                Vec::new()
            }
            None => {
                self.toast(ToastLevel::Info, format!("{index} has no session yet"));
                Vec::new()
            }
        }
    }

    pub(crate) fn approve_dispatch(&mut self) -> Vec<BoardAction> {
        let Some((d, _)) = self.selected_membership() else {
            return Vec::new();
        };
        if !is_proposed(d) {
            return Vec::new();
        }
        let id = d.id.clone();
        let total = d.items.len();
        let chosen: Vec<usize> = self
            .dispatch_selection(&id)
            .map(|s| s.iter().copied().collect())
            .unwrap_or_else(|| d.items.iter().map(|i| i.index).collect());
        if chosen.is_empty() {
            self.toast(ToastLevel::Info, "tick at least one session first");
            return Vec::new();
        }
        let n = chosen.len();
        self.toast(
            ToastLevel::Info,
            format!("starting {n} session{}…", if n == 1 { "" } else { "s" }),
        );
        let selected = (n != total).then_some(chosen);
        vec![BoardAction::ApproveDispatch { id, selected }]
    }

    pub(crate) fn revise_dispatch(&mut self) {
        match self.selected_membership() {
            Some((d, _)) if is_proposed(d) => {
                let dispatch_id = d.id.clone();
                self.open_composer(ComposerMode::Revise { dispatch_id }, "", "");
            }
            _ => self.toast(ToastLevel::Info, "only a proposed split can be revised"),
        }
    }

    pub(crate) fn request_cancel_dispatch(&mut self) {
        let Some((d, _)) = self.selected_membership() else {
            return;
        };
        let running = d
            .items
            .iter()
            .filter(|i| i.status == item_status::RUNNING)
            .count();
        let (title, body) = if is_proposed(d) || d.status == dispatch::dispatch_status::PLANNING {
            (
                "Cancel this dispatch?",
                "Nothing has started. The split is dropped and the coordinator is told."
                    .to_string(),
            )
        } else if is_running(d) {
            (
                "Cancel the remaining sessions?",
                format!(
                    "Queued sessions will not start. {running} running session{} keep going \
                     until they finish; merge or discard them afterwards.",
                    if running == 1 { "" } else { "s" }
                ),
            )
        } else {
            self.toast(ToastLevel::Info, "this dispatch has already ended");
            return;
        };
        self.confirm = Some(Confirm {
            kind: ConfirmKind::CancelDispatch(d.id.clone()),
            title: title.into(),
            body,
        });
        self.focus = Focus::Confirm;
    }

    pub(crate) fn request_merge_finished(&mut self) {
        let Some((d, _)) = self.selected_membership() else {
            self.toast(ToastLevel::Info, "select a dispatch card first");
            return;
        };
        let ready: Vec<usize> = d
            .items
            .iter()
            .filter(|i| i.status == item_status::SUCCEEDED && i.session_id.is_some())
            .map(|i| i.index)
            .collect();
        if !d.worktree {
            self.toast(
                ToastLevel::Info,
                "this dispatch shares one directory — nothing to merge",
            );
            return;
        }
        if ready.is_empty() {
            self.toast(ToastLevel::Info, "no finished session to merge yet");
            return;
        }
        self.confirm = Some(Confirm {
            kind: ConfirmKind::MergeFinished(d.id.clone()),
            title: "Merge every finished session?".into(),
            body: format!(
                "Merges {} into {}, one commit each, in that order. Each merged session stops \
                 and its worktree and branch are removed. The first conflict stops the run: \
                 earlier merges stay committed, that session keeps running.",
                if ready.len() == 1 {
                    format!("item {}", ready[0])
                } else {
                    format!("items {}", list_words(&ready))
                },
                project_name(&d.cwd)
            ),
        });
        self.focus = Focus::Confirm;
    }

    /// The live worktree session `w`/`X` act on, with a toast explaining a refusal.
    fn worktree_target(&mut self) -> Option<(String, String, String)> {
        let card = self.selected_card()?;
        let refusal = if card.past {
            Some("resume it first — merge and discard work on live sessions")
        } else if card.terminal {
            Some("this session runs in a terminal; merge it from there")
        } else if card.worktree.is_none() {
            Some("this session has no worktree to merge or discard")
        } else {
            None
        };
        if let Some(r) = refusal {
            self.toast(ToastLevel::Info, r);
            return None;
        }
        let wt = card
            .worktree
            .clone()
            .map(|w| project_name(&w))
            .unwrap_or_default();
        Some((card.id.clone(), card.display_title(), wt))
    }

    pub(crate) fn request_merge(&mut self) {
        let Some((id, title, wt)) = self.worktree_target() else {
            return;
        };
        self.confirm = Some(Confirm {
            kind: ConfirmKind::Merge(id),
            title: "Merge this worktree back?".into(),
            body: format!(
                "{title} stops and its branch ({wt}) is merged into the project. After a clean \
                 merge its worktree and branch are removed. Conflicts leave everything as it was."
            ),
        });
        self.focus = Focus::Confirm;
    }

    pub(crate) fn request_discard(&mut self) {
        let Some((id, title, wt)) = self.worktree_target() else {
            return;
        };
        self.confirm = Some(Confirm {
            kind: ConfirmKind::Discard(id),
            title: "Discard this worktree?".into(),
            body: format!(
                "{title} stops and its worktree ({wt}) and branch are deleted without merging. \
                 This cannot be undone."
            ),
        });
        self.focus = Focus::Confirm;
    }

    pub(crate) fn resolve_dispatch_confirm(&mut self, kind: ConfirmKind) -> Vec<BoardAction> {
        let (action, toast) = match kind {
            ConfirmKind::CancelDispatch(id) => (BoardAction::CancelDispatch(id), "cancelling…"),
            ConfirmKind::MergeFinished(id) => (BoardAction::MergeFinished(id), "merging…"),
            ConfirmKind::Merge(id) => (BoardAction::Merge(id), "merging…"),
            ConfirmKind::Discard(id) => (BoardAction::Discard(id), "discarding…"),
            ConfirmKind::Archive(id) => (BoardAction::Archive(id), "archiving…"),
        };
        self.toast(ToastLevel::Info, toast);
        vec![action]
    }

    pub(crate) fn toggle_zoom(&mut self) {
        if self.dispatch.zoom.is_some() {
            self.clear_zoom();
            return;
        }
        match self.selected_card().and_then(|c| c.dispatch.clone()) {
            Some(cd) => {
                self.dispatch.zoom = Some(cd.id);
                self.rebuild();
            }
            None => self.toast(ToastLevel::Info, "this card is not part of a dispatch"),
        }
    }

    pub(crate) fn clear_zoom(&mut self) {
        self.dispatch.zoom = None;
        self.rebuild();
    }

    /// The zoomed dispatch, for the header chip.
    pub fn zoomed(&self) -> Option<&DispatchInfo> {
        let id = self.dispatch.zoom.as_ref()?;
        self.dispatch.list.iter().find(|d| &d.id == id)
    }

    /// `y`/`n` once no permission is pending: start or cancel the proposed split on screen.
    pub(crate) fn dispatch_yes_no(&mut self, yes: bool) -> Vec<BoardAction> {
        if !self.dispatch_tab_active() {
            return Vec::new();
        }
        let proposed = self
            .selected_membership()
            .is_some_and(|(d, _)| is_proposed(d));
        match (proposed, yes) {
            (true, true) => self.approve_dispatch(),
            (true, false) => {
                self.request_cancel_dispatch();
                Vec::new()
            }
            (false, false)
                if self.selected_membership().is_some_and(|(d, _)| {
                    is_running(d) || d.status == dispatch::dispatch_status::PLANNING
                }) =>
            {
                self.request_cancel_dispatch();
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    pub(crate) fn dispatch_animating(&self) -> bool {
        self.form_shaking()
            || self
                .dispatch
                .list
                .iter()
                .any(|d| d.status == dispatch::dispatch_status::PLANNING)
    }
}

/// Keys the Dispatch tab takes before the shared key map: the row cursor, ticking, unfolding,
/// merge-all and revise. `None` lets the key fall through.
pub(crate) fn dispatch_tab_key(app: &mut BoardApp, key: KeyEvent) -> Option<Vec<BoardAction>> {
    if !app.dispatch_tab_active() {
        return None;
    }
    let in_detail = app.focus == Focus::Detail;
    match key.code {
        KeyCode::Up | KeyCode::Char('k') if in_detail => app.move_dispatch_cursor(-1),
        KeyCode::Down | KeyCode::Char('j') if in_detail => app.move_dispatch_cursor(1),
        KeyCode::Char(' ') if in_detail => app.toggle_dispatch_item(app.dispatch.cursor),
        KeyCode::Enter if in_detail => {
            return Some(app.activate_dispatch_row(app.dispatch.cursor));
        }
        KeyCode::Char('A') => app.request_merge_finished(),
        KeyCode::Char('e') => {
            let question = app
                .selected_snapshot()
                .is_some_and(|s| s.question.is_some());
            let proposed = app
                .selected_membership()
                .is_some_and(|(d, _)| is_proposed(d));
            if question || !proposed {
                return None;
            }
            app.revise_dispatch();
        }
        _ => return None,
    }
    Some(Vec::new())
}
