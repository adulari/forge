//! Command-line grammar for host durability: `forge backup` and `forge service logs`.

use clap::{Args, Subcommand};

/// `forge backup` copies the store; with a subcommand it lists or restores backups instead.
#[derive(Args)]
#[command(args_conflicts_with_subcommands = true)]
pub(crate) struct BackupArgs {
    #[command(subcommand)]
    pub(crate) cmd: Option<BackupCmd>,
    /// Where to write the copy. Default: a timestamped file in `backups/` beside the store.
    #[arg(long)]
    pub(crate) out: Option<String>,
}

#[derive(Subcommand)]
pub(crate) enum BackupCmd {
    /// List the pre-migration copies Forge keeps and any backups made with `forge backup`.
    List,
    /// Replace the session store with a backup. The daemon must be stopped; the store it
    /// replaces is kept beside it.
    Restore {
        /// The backup file.
        path: String,
    },
}

#[derive(Args)]
pub(crate) struct LogsArgs {
    /// How many recent lines to show.
    #[arg(short = 'n', long, default_value_t = 100)]
    pub(crate) lines: u32,
    /// Keep printing new lines as they arrive.
    #[arg(short = 'f', long)]
    pub(crate) follow: bool,
}
