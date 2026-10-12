//! Command-line grammar for `forge switch`.

use clap::{Args, ValueEnum};

/// Which tool the conversation moves to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum SwitchTarget {
    Claude,
    Forge,
}

#[derive(Args, Debug)]
pub(crate) struct SwitchArgs {
    /// A Claude Code session id or `.jsonl` path, or a Forge session id or unique prefix. Without
    /// one, the newest session of this directory in either tool is carried to the other.
    pub(crate) session: Option<String>,
    /// Force the direction (default: the opposite of where SESSION lives).
    #[arg(long, value_enum)]
    pub(crate) to: Option<SwitchTarget>,
    /// Say what would happen and write nothing.
    #[arg(long)]
    pub(crate) dry_run: bool,
    /// Sync the other side, print the command that would continue the conversation, and stop.
    #[arg(long)]
    pub(crate) no_launch: bool,
    /// Switch even when the session looks open in the tool it is leaving or entering.
    #[arg(long)]
    pub(crate) force: bool,
}
