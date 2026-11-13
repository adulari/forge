use super::FailOnSeverity;
use clap::Subcommand;

#[derive(Subcommand)]
pub(crate) enum QueueCmd {
    /// Queue a task for the next drain.
    ///
    /// Examples:
    ///   forge queue add "migrate the auth module to the new API" --budget 2.50
    ///   forge queue add "add integration tests for billing" --mode bypass
    Add {
        /// The task each drained run works on (quote it).
        task: Vec<String>,
        /// Kill the run when its session cost crosses this USD cap (partial work is kept on the
        /// branch, marked `over-budget`).
        #[arg(long, value_name = "USD")]
        budget: Option<f64>,
        /// Permission mode for the run (default `accept-edits` — the worktree is the sandbox).
        #[arg(long)]
        mode: Option<String>,
        /// Pin a specific model for the run.
        #[arg(long)]
        model: Option<String>,
    },
    /// List queued and drained tasks.
    List,
    /// Remove a task (id or unambiguous prefix). A running task refuses.
    Remove {
        /// Queue task id, or a prefix of one (see `forge queue list`).
        id: String,
    },
    /// Drain this project's pending tasks now — one isolated worktree + result branch per task.
    /// Point a `forge schedule` timer at this for the overnight autopilot.
    Run {
        /// Run the assay gate on each result diff; tasks with findings at or above this
        /// severity are marked `gated` (their branch is still kept). Values: low|medium|high.
        #[arg(long, value_name = "SEVERITY")]
        gate: Option<FailOnSeverity>,
        /// Drain at most this many tasks.
        #[arg(long)]
        max: Option<usize>,
        /// Forward --mock to each run (offline deterministic provider; for tests/demos).
        #[arg(long)]
        mock: bool,
        /// After each task that finishes `done`, re-run it on a free/subscription model
        /// (credit-mode strict — a shadow never spends paid credit) in a throwaway worktree,
        /// and record whether it kept up into the per-repo routing boosts `/duel` feeds.
        /// See `forge scoreboard`.
        #[arg(long)]
        shadow: bool,
    },
    /// The morning digest: outcomes, branches, costs, and replay pointers, most recent first.
    Report,
}
