//! `forge notes` subcommands.

use clap::Subcommand;

#[derive(Subcommand)]
pub(crate) enum NotesCmd {
    /// Print the project's scratchpad and numbered to-do list (the default).
    List {
        /// Project directory (default: the current directory).
        #[arg(long)]
        project: Option<String>,
    },
    /// Add a to-do.
    Add {
        /// The to-do text (quote it).
        text: Vec<String>,
        #[arg(long)]
        project: Option<String>,
    },
    /// Tick a to-do, by its number in `forge notes list` or an id prefix.
    Done {
        item: String,
        #[arg(long)]
        project: Option<String>,
    },
    /// Untick a to-do.
    Reopen {
        item: String,
        #[arg(long)]
        project: Option<String>,
    },
    /// Delete a to-do.
    Rm {
        item: String,
        #[arg(long)]
        project: Option<String>,
    },
    /// Print the scratchpad; with text, append it as a new line.
    Pad {
        text: Vec<String>,
        #[arg(long)]
        project: Option<String>,
    },
}
