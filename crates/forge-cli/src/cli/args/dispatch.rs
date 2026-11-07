use clap::{Subcommand, ValueEnum};

/// `forge dispatch` subcommands. Every dispatch id accepts a unique prefix.
#[derive(Subcommand)]
pub(crate) enum DispatchCmd {
    /// Start a dispatch: a coordinator session reads the project and proposes how to split the
    /// request. Review it with `forge dispatch show` or on `forge board`.
    Start {
        /// The request, as one quoted argument.
        prompt: String,
        /// Project directory. Defaults to the current directory.
        #[arg(long)]
        cwd: Option<std::path::PathBuf>,
        /// Run every item in the project directory itself instead of a git worktree of its own
        /// (items must then not edit the same files). Needed outside a git repository.
        #[arg(long)]
        no_worktree: bool,
        /// Permission mode of the item sessions (default: accept-edits).
        #[arg(long, value_enum)]
        mode: Option<DispatchModeArg>,
        /// How many item sessions may run at once (1-8, default 4).
        #[arg(long, value_parser = clap::value_parser!(u32).range(1..=8))]
        parallel: Option<u32>,
        /// The most items the coordinator may propose (1-12, default 8).
        #[arg(long, value_parser = clap::value_parser!(u32).range(1..=12))]
        max_items: Option<u32>,
        /// Model for the coordinator session only; item sessions route as usual.
        #[arg(long)]
        model: Option<String>,
    },
    /// List dispatches, newest first: status, finished/total items, age, and the request.
    List,
    /// Show a dispatch: the request, the proposed split, and each item's state and session.
    Show {
        /// Dispatch id (a unique prefix is accepted).
        id: String,
    },
    /// Approve the proposed split and start it.
    Approve {
        /// Dispatch id (a unique prefix is accepted).
        id: String,
        /// Approve only these item numbers, e.g. `1,3`. An item that depends on one left out
        /// does not start either.
        #[arg(long)]
        only: Option<String>,
    },
    /// Ask the coordinator for a different split before anything starts.
    Revise {
        /// Dispatch id (a unique prefix is accepted).
        id: String,
        /// What should change, in one quoted argument.
        feedback: String,
    },
    /// Cancel a dispatch: nothing new starts; sessions already running keep running.
    Cancel {
        /// Dispatch id (a unique prefix is accepted).
        id: String,
    },
    /// Merge every finished item back into the project, in item order, one commit each on the
    /// current branch. Stops at the first conflict; earlier merges stay committed.
    Merge {
        /// Dispatch id (a unique prefix is accepted).
        id: String,
    },
}

/// Permission mode of a dispatch's item sessions — `forge_types::PermissionMode` minus plan, which
/// would leave every item unable to do its work.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum DispatchModeArg {
    Default,
    AcceptEdits,
    Bypass,
}

impl DispatchModeArg {
    /// The `mode` value `POST /api/dispatch` accepts.
    pub(crate) fn wire(self) -> &'static str {
        match self {
            DispatchModeArg::Default => "default",
            DispatchModeArg::AcceptEdits => "accept-edits",
            DispatchModeArg::Bypass => "bypass",
        }
    }
}
