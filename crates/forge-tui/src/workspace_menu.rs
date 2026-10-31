//! `/worktree` and `/project`: one navigable overlay for the places a session can work. The
//! worktree list (switch, new with the dirty-tree choice, remove, pull) and project onboarding
//! (Home, clone from a URL or `gh`, `git init`, rename and hide) share its list, text input,
//! choice dialog and busy states.
//!
//! This module is data and key handling only. The binary feeds it snapshots and
//! [`WsEvent`]s from background tasks, and performs what a [`WsAction`] asks for; rendering is in
//! [`render`].

use crate::app::KeyKind;

mod model;
mod render;
pub use model::*;
pub(crate) use render::render_workspace_menu;

#[cfg(test)]
mod tests;

pub fn valid_worktree_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("a name is required".into());
    }
    if name.starts_with('-') || name.starts_with('.') || name.contains("..") {
        return Err("a name cannot start with '-' or '.', or contain '..'".into());
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/'))
    {
        return Err("use letters, digits, '-', '_', '.' or '/'".into());
    }
    Ok(())
}

impl WorkspaceMenu {
    pub fn open(&mut self, kind: Kind) {
        let inbox = self.inbox.clone();
        let data = self.data.take();
        *self = Self {
            open: true,
            kind,
            data,
            inbox,
            refresh: true,
            ..Self::default()
        };
    }

    pub fn close(&mut self) {
        self.open = false;
        self.screen = Screen::List;
    }

    pub fn inbox(&self) -> WsInbox {
        self.inbox.clone()
    }

    pub fn take_refresh(&mut self) -> bool {
        std::mem::take(&mut self.refresh)
    }

    pub fn notify(&mut self, tone: Tone, text: impl Into<String>) {
        self.notice = Some((tone, text.into()));
    }

    pub fn set_busy(&mut self, label: impl Into<String>) {
        self.screen = Screen::Busy(label.into());
    }

    /// The URL was rejected: keep the input open and say why.
    pub fn input_error(&mut self, error: impl Into<String>) {
        self.screen = match std::mem::take(&mut self.screen) {
            Screen::Busy(_) | Screen::List => Screen::Input(Input {
                kind: InputKind::CloneUrl,
                value: String::new(),
                error: None,
                sel: None,
            }),
            other => other,
        };
        if let Screen::Input(input) = &mut self.screen {
            input.error = Some(error.into());
        }
    }

    pub fn offer_choice(&mut self, choice: Choice) {
        self.open = true;
        self.screen = Screen::Choice(choice);
    }

    /// Advance the spinner and apply what background tasks posted. True when a redraw is due.
    pub fn advance(&mut self) -> bool {
        if !self.open && self.inbox.0.lock().is_ok_and(|q| q.is_empty()) {
            return false;
        }
        self.tick = self.tick.wrapping_add(1);
        let events = self.inbox.take();
        let any = !events.is_empty();
        for event in events {
            self.apply(event);
        }
        any || (self.open && matches!(self.screen, Screen::Busy(_) | Screen::Input(_)))
            || (self.open && (self.data.is_none() || self.gh == Gh::Loading))
    }

    pub fn apply(&mut self, event: WsEvent) {
        match event {
            WsEvent::Data(data) => {
                self.data = Some(data);
                self.clamp();
            }
            WsEvent::Repos(Ok(repos)) => self.gh = Gh::Ready(repos),
            WsEvent::Repos(Err(note)) => self.gh = Gh::Unavailable(note),
            WsEvent::OpDone { ok, text } => {
                if !self.open {
                    return;
                }
                if ok {
                    self.close();
                } else {
                    self.screen = Screen::Message {
                        title: "That did not work".into(),
                        lines: text.lines().map(str::to_string).collect(),
                        tone: Some(Tone::Err),
                    };
                }
            }
            WsEvent::Cloned(Ok(path)) => {
                self.kind = Kind::Projects;
                self.refresh |= self.data.is_none();
                self.offer_choice(Choice {
                    title: "Repository cloned".into(),
                    body: vec![path.clone(), "Switch this session to it?".into()],
                    options: vec!["Switch to it".into(), "Stay here".into()],
                    selected: 0,
                    purpose: Purpose::SwitchTo { path },
                    danger: false,
                });
            }
            WsEvent::Cloned(Err(error)) => {
                self.kind = Kind::Projects;
                self.refresh |= self.data.is_none();
                self.open = true;
                self.screen = Screen::Message {
                    title: "Clone failed".into(),
                    lines: error.lines().map(str::to_string).collect(),
                    tone: Some(Tone::Err),
                };
            }
            WsEvent::Pulled {
                ok,
                message,
                conflicts,
                handoff,
            } => {
                // Typed as a command with nothing to decide: the transcript note is enough.
                if !self.open && handoff.is_none() {
                    return;
                }
                self.kind = Kind::Worktrees;
                self.open = true;
                self.refresh |= self.data.is_none();
                let mut lines = vec![message];
                lines.extend(conflicts.iter().map(|f| format!("  conflict: {f}")));
                self.screen = match handoff {
                    Some(prompt) => {
                        lines.push(String::new());
                        lines.push("Hand the conflict to the agent to resolve?".into());
                        Screen::Choice(Choice {
                            title: "Pull stopped on a conflict".into(),
                            body: lines,
                            options: vec!["Hand it to the agent".into(), "I will fix it".into()],
                            selected: 0,
                            purpose: Purpose::Handoff { prompt },
                            danger: false,
                        })
                    }
                    None => Screen::Message {
                        title: if ok { "Pull" } else { "Pull failed" }.into(),
                        lines,
                        tone: Some(if ok { Tone::Ok } else { Tone::Err }),
                    },
                };
            }
        }
    }

    /// The rows the list screen shows, after the search filter.
    pub fn items(&self) -> Vec<Item> {
        let mut all = Vec::new();
        match self.kind {
            Kind::Worktrees => {
                if let Some(data) = &self.data {
                    all.extend((0..data.worktrees.len()).map(Item::Worktree));
                }
            }
            Kind::Projects => {
                all.push(Item::Home);
                all.push(Item::Clone);
                let data = self.data.as_ref();
                if data.is_some_and(|d| !d.is_git && !d.is_home) {
                    all.push(Item::InitHere);
                }
                if !data.is_some_and(|d| d.is_home) {
                    all.push(Item::Rename);
                    all.push(Item::Hide);
                }
            }
        }
        let needle = self.filter.trim().to_lowercase();
        if needle.is_empty() {
            return all;
        }
        all.into_iter()
            .filter(|item| self.label(*item).to_lowercase().contains(&needle))
            .collect()
    }

    /// The searchable text of a row.
    pub fn label(&self, item: Item) -> String {
        match item {
            Item::Worktree(i) => self
                .data
                .as_ref()
                .and_then(|d| d.worktrees.get(i))
                .map(|w| format!("{} {}", w.branch, w.display))
                .unwrap_or_default(),
            Item::Home => "Home (no project)".into(),
            Item::Clone => "Clone a repository".into(),
            Item::InitHere => "Initialize git here".into(),
            Item::Rename => "Rename this project".into(),
            Item::Hide => "Hide this project".into(),
        }
    }

    pub fn focused(&self) -> Option<Item> {
        self.items().get(self.cursor).copied()
    }

    fn clamp(&mut self) {
        self.cursor = self.cursor.min(self.items().len().saturating_sub(1));
    }

    /// The `gh` repositories matching what is typed in the clone input.
    pub fn repo_matches(&self, query: &str) -> Vec<usize> {
        let Gh::Ready(repos) = &self.gh else {
            return Vec::new();
        };
        let needle = query.trim().to_lowercase();
        repos
            .iter()
            .enumerate()
            .filter(|(_, r)| {
                needle.is_empty()
                    || r.name.to_lowercase().contains(&needle)
                    || r.description.to_lowercase().contains(&needle)
            })
            .map(|(i, _)| i)
            .collect()
    }

    pub fn key(&mut self, key: KeyKind) -> WsAction {
        self.notice = None;
        match std::mem::take(&mut self.screen) {
            Screen::List => {
                self.screen = Screen::List;
                self.list_key(key)
            }
            Screen::Input(input) => self.input_key(input, key),
            Screen::Choice(choice) => self.choice_key(choice, key),
            Screen::Busy(label) => {
                self.screen = Screen::Busy(label);
                if matches!(key, KeyKind::Esc | KeyKind::Interrupt) {
                    self.close();
                    return WsAction::Close;
                }
                WsAction::None
            }
            Screen::Message { .. } => {
                if matches!(key, KeyKind::Esc | KeyKind::Enter | KeyKind::Interrupt) {
                    return WsAction::Refresh;
                }
                self.screen = Screen::List;
                WsAction::None
            }
        }
    }

    fn list_key(&mut self, key: KeyKind) -> WsAction {
        let count = self.items().len();
        if self.searching {
            match key {
                KeyKind::Esc | KeyKind::Interrupt => {
                    self.filter.clear();
                    self.searching = false;
                }
                KeyKind::Enter | KeyKind::Down => self.searching = false,
                KeyKind::Backspace => {
                    self.filter.pop();
                }
                KeyKind::KillLineBack => self.filter.clear(),
                KeyKind::Char(c) => self.filter.push(c),
                _ => {}
            }
            self.cursor = 0;
            return WsAction::None;
        }
        match key {
            KeyKind::Esc | KeyKind::Interrupt | KeyKind::Char('q') => {
                if !self.filter.is_empty() && key == KeyKind::Esc {
                    self.filter.clear();
                    self.cursor = 0;
                    return WsAction::None;
                }
                self.close();
                WsAction::Close
            }
            KeyKind::Char('/') => {
                self.searching = true;
                WsAction::None
            }
            KeyKind::Up | KeyKind::Char('k') => {
                self.cursor = self.cursor.saturating_sub(1);
                WsAction::None
            }
            KeyKind::Down | KeyKind::Char('j') => {
                self.cursor = (self.cursor + 1).min(count.saturating_sub(1));
                WsAction::None
            }
            KeyKind::Home => {
                self.cursor = 0;
                WsAction::None
            }
            KeyKind::End => {
                self.cursor = count.saturating_sub(1);
                WsAction::None
            }
            KeyKind::PageUp => {
                self.cursor = self.cursor.saturating_sub(5);
                WsAction::None
            }
            KeyKind::PageDown => {
                self.cursor = (self.cursor + 5).min(count.saturating_sub(1));
                WsAction::None
            }
            KeyKind::Char('r') => WsAction::Refresh,
            KeyKind::Char('n') if self.kind == Kind::Worktrees => self.start_new_worktree(),
            KeyKind::Char('d') if self.kind == Kind::Worktrees => self.start_remove(),
            KeyKind::Char('p') if self.kind == Kind::Worktrees => {
                self.set_busy("Pulling (local changes are stashed and re-applied)");
                WsAction::Pull
            }
            KeyKind::Char('c') if self.kind == Kind::Projects => self.start_clone(),
            KeyKind::Enter | KeyKind::Char(' ') => self.activate(),
            _ => WsAction::None,
        }
    }

    fn start_new_worktree(&mut self) -> WsAction {
        let Some(data) = &self.data else {
            return WsAction::None;
        };
        if !data.is_git {
            self.notify(
                Tone::Warn,
                "Not a git repository. /project can initialize one.",
            );
            return WsAction::None;
        }
        self.screen = Screen::Input(Input {
            kind: InputKind::WorktreeName,
            value: data.suggested_name.clone(),
            error: None,
            sel: None,
        });
        WsAction::None
    }

    fn start_remove(&mut self) -> WsAction {
        let Some(Item::Worktree(i)) = self.focused() else {
            return WsAction::None;
        };
        let Some(row) = self.data.as_ref().and_then(|d| d.worktrees.get(i)).cloned() else {
            return WsAction::None;
        };
        if row.main {
            self.notify(Tone::Warn, "The main checkout cannot be removed.");
            return WsAction::None;
        }
        if row.current {
            self.notify(
                Tone::Warn,
                "This session is working in it. Enter another worktree first.",
            );
            return WsAction::None;
        }
        let mut body = vec![row.display.clone()];
        if row.dirty {
            body.push("It has uncommitted changes. Removing it discards them for good.".into());
        }
        if row.merged {
            body.push(format!("Its merged branch {} is deleted too.", row.branch));
        } else {
            body.push(format!(
                "Its branch {} has commits that are not merged. The branch is kept.",
                row.branch
            ));
        }
        body.push("The cleanup command runs first.".into());
        self.screen = Screen::Choice(Choice {
            title: format!("Remove worktree {}?", row.branch),
            body,
            options: vec![
                if row.dirty {
                    "Discard changes and remove"
                } else {
                    "Remove"
                }
                .into(),
                "Cancel".into(),
            ],
            selected: 1,
            purpose: Purpose::RemoveWorktree {
                path: row.path,
                branch: row.branch,
                force: row.dirty,
                keep_branch: !row.merged,
            },
            danger: true,
        });
        WsAction::None
    }

    fn start_clone(&mut self) -> WsAction {
        self.screen = Screen::Input(Input {
            kind: InputKind::CloneUrl,
            value: String::new(),
            error: None,
            sel: None,
        });
        if self.gh == Gh::Idle {
            self.gh = Gh::Loading;
            return WsAction::LoadRepos;
        }
        WsAction::None
    }

    fn activate(&mut self) -> WsAction {
        match self.focused() {
            Some(Item::Worktree(i)) => {
                let Some(row) = self.data.as_ref().and_then(|d| d.worktrees.get(i)).cloned() else {
                    return WsAction::None;
                };
                if row.current {
                    self.notify(Tone::Ok, "The session is already working there.");
                    return WsAction::None;
                }
                self.set_busy(format!("Switching to {}", row.branch));
                WsAction::EnterWorktree { path: row.path }
            }
            Some(Item::Home) => {
                self.set_busy("Switching to Home");
                WsAction::GoHome
            }
            Some(Item::Clone) => self.start_clone(),
            Some(Item::InitHere) => {
                let root = self
                    .data
                    .as_ref()
                    .map(|d| d.root.clone())
                    .unwrap_or_default();
                self.screen = Screen::Choice(Choice {
                    title: "Initialize a git repository?".into(),
                    body: vec![
                        root,
                        "Runs git init and makes an empty first commit so worktrees can branch."
                            .into(),
                    ],
                    options: vec!["Initialize".into(), "Cancel".into()],
                    selected: 0,
                    purpose: Purpose::InitHere,
                    danger: false,
                });
                WsAction::None
            }
            Some(Item::Rename) => {
                self.screen = Screen::Input(Input {
                    kind: InputKind::RenameProject,
                    value: self
                        .data
                        .as_ref()
                        .map(|d| d.project_name.clone())
                        .unwrap_or_default(),
                    error: None,
                    sel: None,
                });
                WsAction::None
            }
            Some(Item::Hide) => {
                let name = self
                    .data
                    .as_ref()
                    .map(|d| d.project_name.clone())
                    .unwrap_or_default();
                self.screen = Screen::Choice(Choice {
                    title: format!("Hide {name}?"),
                    body: vec![
                        "It disappears from project lists in every Forge app. Nothing on disk is touched."
                            .into(),
                    ],
                    options: vec!["Hide".into(), "Cancel".into()],
                    selected: 1,
                    purpose: Purpose::HideProject,
                    danger: true,
                });
                WsAction::None
            }
            None => WsAction::None,
        }
    }

    fn input_key(&mut self, mut input: Input, key: KeyKind) -> WsAction {
        let clone = input.kind == InputKind::CloneUrl;
        let matches = if clone {
            self.repo_matches(&input.value)
        } else {
            Vec::new()
        };
        match key {
            KeyKind::Esc | KeyKind::Interrupt => {
                self.screen = Screen::List;
                return WsAction::None;
            }
            KeyKind::Down if clone && !matches.is_empty() => {
                input.sel = Some(
                    match input.sel.and_then(|s| matches.iter().position(|m| *m == s)) {
                        Some(p) => matches[(p + 1).min(matches.len() - 1)],
                        None => matches[0],
                    },
                );
            }
            KeyKind::Up if clone => {
                input.sel = input
                    .sel
                    .and_then(|s| matches.iter().position(|m| *m == s))
                    .and_then(|p| p.checked_sub(1))
                    .map(|p| matches[p]);
            }
            KeyKind::Backspace => {
                input.value.pop();
                input.error = None;
                input.sel = None;
            }
            KeyKind::KillLineBack | KeyKind::DeleteWordBack => {
                input.value.clear();
                input.error = None;
                input.sel = None;
            }
            KeyKind::Char(c) => {
                input.value.push(c);
                input.error = None;
                input.sel = None;
            }
            KeyKind::Enter => return self.submit_input(input),
            _ => {}
        }
        self.screen = Screen::Input(input);
        WsAction::None
    }

    fn submit_input(&mut self, mut input: Input) -> WsAction {
        let value = input.value.trim().to_string();
        match input.kind {
            InputKind::WorktreeName => {
                if let Err(error) = valid_worktree_name(&value) {
                    input.error = Some(error);
                    self.screen = Screen::Input(input);
                    return WsAction::None;
                }
                if self.data.as_ref().is_some_and(|d| d.dirty) {
                    self.screen = Screen::Choice(Choice {
                        title: "This checkout has uncommitted changes".into(),
                        body: vec![
                            format!("New worktree: {value}"),
                            "Carry them into the new worktree, or leave them where they are?"
                                .into(),
                        ],
                        options: vec![
                            "Carry my uncommitted changes".into(),
                            "Leave them here".into(),
                            "Cancel".into(),
                        ],
                        selected: 0,
                        purpose: Purpose::DirtyTree { name: value },
                        danger: false,
                    });
                    return WsAction::None;
                }
                self.set_busy(format!("Creating worktree {value}"));
                WsAction::CreateWorktree {
                    name: value,
                    carry: false,
                }
            }
            InputKind::CloneUrl => {
                let url = match input.sel.and_then(|s| match &self.gh {
                    Gh::Ready(repos) => repos.get(s).map(|r| r.url.clone()),
                    _ => None,
                }) {
                    Some(url) => url,
                    None => value,
                };
                if url.is_empty() {
                    input.error = Some("Paste a repository URL, or pick one below.".into());
                    self.screen = Screen::Input(input);
                    return WsAction::None;
                }
                input.value = url.clone();
                self.screen = Screen::Input(input);
                WsAction::Clone { url }
            }
            InputKind::RenameProject => {
                self.set_busy("Saving");
                WsAction::RenameProject { name: value }
            }
        }
    }

    fn choice_key(&mut self, mut choice: Choice, key: KeyKind) -> WsAction {
        let last = choice.options.len().saturating_sub(1);
        match key {
            KeyKind::Esc | KeyKind::Interrupt | KeyKind::Char('n') | KeyKind::Char('q') => {
                self.screen = Screen::List;
                return WsAction::None;
            }
            KeyKind::Left | KeyKind::Up | KeyKind::Char('h') | KeyKind::Char('k') => {
                choice.selected = choice.selected.saturating_sub(1);
            }
            KeyKind::Right
            | KeyKind::Down
            | KeyKind::Char('l')
            | KeyKind::Char('j')
            | KeyKind::Tab => {
                choice.selected = (choice.selected + 1).min(last);
            }
            KeyKind::Char('y') => choice.selected = 0,
            KeyKind::Enter => return self.resolve_choice(choice),
            _ => {}
        }
        if key == KeyKind::Char('y') {
            return self.resolve_choice(choice);
        }
        self.screen = Screen::Choice(choice);
        WsAction::None
    }

    fn resolve_choice(&mut self, choice: Choice) -> WsAction {
        let first = choice.selected == 0;
        match choice.purpose {
            Purpose::DirtyTree { name } => match choice.selected {
                0 | 1 => {
                    self.set_busy(format!("Creating worktree {name}"));
                    WsAction::CreateWorktree {
                        name,
                        carry: choice.selected == 0,
                    }
                }
                _ => {
                    self.screen = Screen::List;
                    WsAction::None
                }
            },
            Purpose::RemoveWorktree {
                path,
                branch,
                force,
                keep_branch,
            } if first => {
                self.set_busy(format!("Removing {branch}"));
                WsAction::RemoveWorktree {
                    path,
                    branch,
                    force,
                    keep_branch,
                }
            }
            Purpose::Handoff { prompt } if first => {
                self.close();
                WsAction::Submit { prompt }
            }
            Purpose::SwitchTo { path } if first => {
                self.set_busy("Switching");
                WsAction::SwitchProject { path }
            }
            Purpose::HideProject if first => {
                self.set_busy("Hiding");
                WsAction::HideProject
            }
            Purpose::InitHere if first => {
                self.set_busy("Running git init");
                WsAction::InitHere
            }
            _ => {
                self.screen = Screen::List;
                WsAction::None
            }
        }
    }
}
