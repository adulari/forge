//! IDE mode: the project review workbench in the terminal. Rail (agents, change queue, git), the
//! editor (a diff with per-hunk stage / revert / undo), the agent panel (needs-you, timeline,
//! tasks, composer) and a bottom panel, adapting to the terminal's width.
//!
//! The module is renderer-independent state (ADR-0004), the same shape as `board`: it folds
//! [`IdeEvent`]s in, answers key and mouse input with [`IdeAction`]s for the host to perform, and
//! draws itself into a ratatui frame. `forge-cli`'s `ide_host` talks to the daemon.

pub mod checks;
pub mod edit_buf;
pub mod hunk;
pub mod keys;
pub mod model;
mod nav;
mod ops;
pub mod render;
pub mod review;
pub mod state;
pub mod term_grid;
pub mod timeline;
mod types;
pub mod wire;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_tools;

pub use keys::{handle_key, handle_mouse, handle_paste, HELP};
pub use render::{draw, mode_for, Mode, AGENT_W, MID, RAIL_W, WIDE};
pub use state::{
    AgentRow, BottomTab, DiffLoaded, Hit, HunkFailure, HunkOutcome, IdeAction, IdeApp, IdeEvent,
    Pane, RailFocus, RailTab, SaveFailure, View,
};
