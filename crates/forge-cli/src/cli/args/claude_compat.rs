//! `claude -p` flags that map cleanly onto `forge run`.
//!
//! Supported: `-p/--print`, `--verbose`, `--input-format text`, `--permission-mode`,
//! `--dangerously-skip-permissions`, `--max-turns`, `--allowedTools`, `--disallowedTools`
//! (plus `--append-system-prompt`, `--output-format`, `--continue`, `--resume`, `--model`, which
//! `forge run` always had). Not supported, and rejected by clap rather than silently ignored:
//! `--system-prompt` (replacing Forge's own prompt), `--input-format stream-json`, `--session-id`,
//! `--fork-session`, `--add-dir`, `--mcp-config`, `--settings`, `--fallback-model`,
//! `--max-budget-usd`. See docs/features/claude-flag-compat.md.

use clap::{Args, ValueEnum};

use super::Mode;

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum InputFormat {
    Text,
}

/// Claude Code's `--permission-mode` spellings.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum ClaudePermissionMode {
    #[value(name = "default")]
    Default,
    #[value(name = "acceptEdits")]
    AcceptEdits,
    #[value(name = "bypassPermissions")]
    BypassPermissions,
    #[value(name = "plan")]
    Plan,
}

impl From<ClaudePermissionMode> for Mode {
    fn from(mode: ClaudePermissionMode) -> Self {
        match mode {
            ClaudePermissionMode::Default => Mode::Default,
            ClaudePermissionMode::AcceptEdits => Mode::AcceptEdits,
            ClaudePermissionMode::BypassPermissions => Mode::Bypass,
            ClaudePermissionMode::Plan => Mode::Plan,
        }
    }
}

#[derive(Args, Clone, Debug, Default)]
pub(crate) struct ClaudeCompat {
    /// Accepted for `claude -p` compatibility; `forge run` is already non-interactive.
    #[arg(short = 'p', long = "print")]
    pub(crate) print: bool,
    /// Accepted for `claude -p --output-format stream-json --verbose` compatibility; the event
    /// stream is always complete.
    #[arg(long)]
    pub(crate) verbose: bool,
    /// Input format (only `text`; `stream-json` input is not supported).
    #[arg(long, value_enum, value_name = "FORMAT")]
    pub(crate) input_format: Option<InputFormat>,
    /// Claude Code's spelling of `--mode`: default, acceptEdits, bypassPermissions or plan.
    #[arg(long, value_enum, value_name = "MODE", conflicts_with = "mode")]
    pub(crate) permission_mode: Option<ClaudePermissionMode>,
    /// Same as `--mode bypass`.
    #[arg(
        long = "dangerously-skip-permissions",
        conflicts_with_all = ["mode", "permission_mode"]
    )]
    pub(crate) dangerously_skip_permissions: bool,
    /// Stop the turn after this many model steps (maps to `mesh.max_steps`).
    #[arg(long, value_name = "N", value_parser = clap::value_parser!(u32).range(1..))]
    pub(crate) max_turns: Option<u32>,
    /// Tools to allow without asking, Claude Code syntax: `"Bash(git log:*) Read"`. Space or comma
    /// separated; repeat the flag for more.
    #[arg(
        long = "allowedTools",
        visible_alias = "allowed-tools",
        value_name = "TOOLS"
    )]
    pub(crate) allowed_tools: Vec<String>,
    /// Tools to deny outright, same syntax as `--allowedTools`.
    #[arg(
        long = "disallowedTools",
        visible_alias = "disallowed-tools",
        value_name = "TOOLS"
    )]
    pub(crate) disallowed_tools: Vec<String>,
}

impl ClaudeCompat {
    /// The permission mode these flags and `--mode` ask for together.
    pub(crate) fn effective_mode(&self, mode: Option<Mode>) -> Option<Mode> {
        mode.or(self.permission_mode.map(Mode::from))
            .or(self.dangerously_skip_permissions.then_some(Mode::Bypass))
    }
}
