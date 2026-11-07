//! `forge ci` subcommands.

use clap::{Args, Subcommand};

#[derive(Args)]
pub(crate) struct CiArgs {
    #[command(subcommand)]
    pub(crate) cmd: Option<CiCmd>,
}

#[derive(Subcommand)]
pub(crate) enum CiCmd {
    /// Print the local CI plan derived from `.github/workflows`: the steps that would run with
    /// their environment, and every skipped step with its reason (the default).
    Plan {
        /// Project directory (default: the current directory).
        #[arg(long)]
        project: Option<String>,
        /// Print the plan as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Run the plan. Stops at the first failing step, naming it with its command and output.
    Run {
        #[arg(long)]
        project: Option<String>,
        /// Run every step even after one fails.
        #[arg(long)]
        keep_going: bool,
        /// Ignore results from an identical earlier tree and run everything.
        #[arg(long)]
        fresh: bool,
        /// Run only steps whose key (`workflow::job::name`) contains this text.
        #[arg(long)]
        step: Option<String>,
        /// Print the report as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Show the result of the last run in this project.
    Status {
        #[arg(long)]
        project: Option<String>,
        #[arg(long)]
        json: bool,
    },
}
