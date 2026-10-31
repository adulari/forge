//! The data the `/worktree` and `/project` overlay holds and the messages it exchanges with
//! the binary.

use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Kind {
    #[default]
    Worktrees,
    Projects,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WtRow {
    pub branch: String,
    /// Absolute path, what a switch or removal is addressed by.
    pub path: String,
    /// The path as shown (`~` for the home directory).
    pub display: String,
    pub dirty: bool,
    pub current: bool,
    pub main: bool,
    /// Its branch is merged into the main checkout's HEAD, so removing it loses no commits.
    pub merged: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RepoRow {
    pub name: String,
    pub url: String,
    pub description: String,
    pub private: bool,
}

/// What the binary read from the workspace and the host.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkspaceData {
    pub root: String,
    pub is_git: bool,
    /// The session works in the Home scratch folder, which is not a project to rename or hide.
    pub is_home: bool,
    /// The current checkout has uncommitted changes.
    pub dirty: bool,
    pub worktrees: Vec<WtRow>,
    /// Friendly random name offered when creating a worktree.
    pub suggested_name: String,
    /// Display name of the current project (custom name, else the folder).
    pub project_name: String,
    /// Where a clone lands.
    pub clone_parent: String,
    /// `Some` when the worktree list could not be read.
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Gh {
    #[default]
    Idle,
    Loading,
    Unavailable(String),
    Ready(Vec<RepoRow>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Ok,
    Warn,
    Err,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputKind {
    WorktreeName,
    CloneUrl,
    RenameProject,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Input {
    pub kind: InputKind,
    pub value: String,
    pub error: Option<String>,
    /// Clone only: the `gh` repository highlighted below the input, if any.
    pub sel: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Purpose {
    DirtyTree {
        name: String,
    },
    RemoveWorktree {
        path: String,
        branch: String,
        force: bool,
        keep_branch: bool,
    },
    Handoff {
        prompt: String,
    },
    SwitchTo {
        path: String,
    },
    HideProject,
    InitHere,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    pub title: String,
    pub body: Vec<String>,
    pub options: Vec<String>,
    pub selected: usize,
    pub purpose: Purpose,
    pub danger: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Screen {
    #[default]
    List,
    Input(Input),
    Choice(Choice),
    Busy(String),
    Message {
        title: String,
        lines: Vec<String>,
        tone: Option<Tone>,
    },
}

/// A row ↑/↓ visits on the list screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Item {
    Worktree(usize),
    Home,
    Clone,
    InitHere,
    Rename,
    Hide,
}

/// What the binary should do after a keystroke.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WsAction {
    None,
    Close,
    Refresh,
    LoadRepos,
    EnterWorktree {
        path: String,
    },
    CreateWorktree {
        name: String,
        carry: bool,
    },
    RemoveWorktree {
        path: String,
        branch: String,
        /// The worktree has changes the user agreed to discard.
        force: bool,
        /// Its branch holds commits that are not merged anywhere else; delete only the checkout.
        keep_branch: bool,
    },
    Pull,
    GoHome,
    InitHere,
    /// The URL still needs validating; the binary answers with [`WorkspaceMenu::input_error`].
    Clone {
        url: String,
    },
    SwitchProject {
        path: String,
    },
    RenameProject {
        name: String,
    },
    HideProject,
    /// Send this text as the next user prompt.
    Submit {
        prompt: String,
    },
}

/// Results from background tasks, applied on the next tick.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WsEvent {
    Data(WorkspaceData),
    Repos(Result<Vec<RepoRow>, String>),
    /// A worktree or project operation finished. On success the overlay closes.
    OpDone {
        ok: bool,
        text: String,
    },
    Cloned(Result<String, String>),
    Pulled {
        ok: bool,
        message: String,
        conflicts: Vec<String>,
        handoff: Option<String>,
    },
}

/// Where background tasks leave [`WsEvent`]s. Clones share one queue.
#[derive(Debug, Clone, Default)]
pub struct WsInbox(pub(super) Arc<Mutex<Vec<WsEvent>>>);

impl WsInbox {
    pub fn post(&self, event: WsEvent) {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).push(event);
    }

    pub(super) fn take(&self) -> Vec<WsEvent> {
        std::mem::take(&mut *self.0.lock().unwrap_or_else(|e| e.into_inner()))
    }
}

#[derive(Debug, Clone, Default)]
pub struct WorkspaceMenu {
    pub open: bool,
    pub kind: Kind,
    pub screen: Screen,
    pub data: Option<WorkspaceData>,
    pub gh: Gh,
    pub filter: String,
    pub searching: bool,
    pub cursor: usize,
    pub notice: Option<(Tone, String)>,
    pub tick: u32,
    pub(super) inbox: WsInbox,
    pub(super) refresh: bool,
}
