//! The project board — `forge board` (docs/features/project-board.md).
//!
//! A full-screen, live overview of every Forge session working on a project, laid out like a
//! project board: **Needs you · Working · Ready · Done**. Each card says which agent is doing
//! what — model, current task, last line, cost, context, signals such as "waiting on a
//! permission", "repeating the same opening", "pinned model is benched" — and opening a card
//! shows the live tail, tasks, subagents, changes and recent tool calls, with the actions that
//! resolve it: answer, prompt, interrupt, re-pin the model, change the mode, archive, or drop
//! into the session's own terminal client (`forge attach`).
//!
//! This module is the renderer-independent half (ADR-0004): pure state that folds
//! [`BoardEvent`]s in, answers key/mouse input with [`BoardAction`]s for the host to perform, and
//! draws itself into a ratatui frame. It never touches the network or the terminal — `forge-cli`'s
//! `board` module is the host that talks to the daemon over exactly the surface `forge attach`
//! uses, so the same board can later render in the mobile and desktop apps.

pub mod actions;
pub mod detail;
pub mod keys;
pub mod model;
pub mod render;
pub mod state;
pub mod types;
pub mod widgets;
pub mod wire;

#[cfg(test)]
mod tests;

pub use keys::{handle_key, handle_mouse, handle_paste, HELP};
pub use model::{
    fmt_age, fmt_cost, live_card, live_signals, model_short, past_card, project_name,
    repeated_opening, stop_reason_words, Card, Column, Health, Signal, SignalLevel,
};
pub use state::{
    BoardApp, Button, Composer, ComposerMode, Confirm, ConfirmKind, ConnState, DetailTab, Focus,
    Hit, Toast, ToastLevel, Totals,
};
pub use wire::{
    DiffCard, DiffFile, FleetRow, GitFile, GitInfo, HistoryRow, LiveSnapshot, PastRow, PlanCard,
    PlanStep, QOption, Subagent, Task, TranscriptRow, WorkflowCard,
};

/// Something the host learned and the board should reflect.
// `Snapshot` carries a whole per-session frame; every other variant is small. Boxing it would
// only move one allocation from the channel into the event for no measurable gain at ≤10 Hz.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq)]
pub enum BoardEvent {
    /// A fresh `GET /api/sessions`.
    Fleet(Vec<FleetRow>),
    /// A fresh `GET /api/sessions/past`.
    Past(Vec<PastRow>),
    /// One per-session snapshot frame from its WebSocket.
    Snapshot(String, LiveSnapshot),
    /// The session's WebSocket ended (the daemon closed it or the session was archived).
    SessionClosed(String),
    /// `GET /api/git/status` for one session.
    Git(String, GitInfo),
    /// `GET /api/history?include_tools=1` for one session (newest first, as served).
    History(String, Vec<HistoryRow>),
    /// Feedback about an action the host performed, or a failure worth showing.
    Toast(ToastLevel, String),
    /// Daemon connectivity.
    Connection(ConnState),
    /// ~10 Hz: advances animation, ages, toast expiry.
    Tick,
    /// The terminal was resized.
    Resize(u16, u16),
}

/// What the host must do because of user input. The board never performs these itself.
#[derive(Debug, Clone, PartialEq)]
pub enum BoardAction {
    /// Leave the board, run `forge attach` on this session (the daemon's single-writer terminal
    /// client), come back when it exits.
    Attach(String),
    /// Send this `RemoteInput` JSON down the session's WebSocket (prompt / allow / answer /
    /// interrupt / steer, exactly the daemon's tagged shape).
    Input(String, serde_json::Value),
    /// `POST /api/sessions/{id}/interrupt`.
    Interrupt(String),
    /// `POST /api/sessions/{id}/archive`.
    Archive(String),
    /// `POST /api/sessions/{id}/mode` with this canonical mode key.
    SetMode(String, String),
    /// `POST /api/sessions {resume:<id>, cwd}`.
    Resume(String),
    /// `POST /api/sessions {cwd, worktree, title}` then send `prompt` once it streams.
    NewSession {
        cwd: String,
        worktree: bool,
        prompt: String,
    },
    /// The user opened this card: fetch its git status + recent tool history now.
    WantDetail(String),
    /// Re-fetch the fleet and past lists now.
    Refresh,
    /// Copy this text to the clipboard (OSC 52 / system clipboard, host's choice).
    Copy(String),
    Quit,
}
