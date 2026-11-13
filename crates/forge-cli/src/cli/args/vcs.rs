use clap::Subcommand;

#[derive(Subcommand)]
pub(crate) enum GitCmd {
    /// Install the `prepare-commit-msg` git hook that strips Claude/Codex co-author lines and
    /// adds `Co-Authored-By: Forge <forge@adulari.dev>`. Works in linked worktrees and submodules,
    /// keeps (and chains) an existing hook, and reports `core.hooksPath` instead of ignoring it. Requires `[git] coauthor = true` in
    /// `.forge/config.toml` (or pass `--force` to install regardless).
    Setup {
        /// Install the hook even if `[git] coauthor` is not set in config.
        #[arg(long)]
        force: bool,
    },
    /// Print the hook script, for calling from your own hook when `core.hooksPath` is set.
    Hook,
}

#[derive(Subcommand)]
pub(crate) enum PrsOp {
    /// The merge queue per repository: position, state and reason for every PR in `merge` mode.
    /// `hold`, `release`, `front` and `retry` act on one PR in it.
    #[command(args_conflicts_with_subcommands = true)]
    Queue {
        /// Print the daemon's JSON instead of the table.
        #[arg(long)]
        json: bool,
        /// Daemon base URL. Defaults to the local daemon's loopback origin.
        #[arg(long)]
        url: Option<String>,
        /// Daemon token. Defaults to the persisted `serve-token` in the config dir.
        #[arg(long)]
        token: Option<String>,
        #[command(subcommand)]
        action: Option<QueueAction>,
    },
    /// The independent review of a watched PR: verdict, model and every finding with file:line.
    /// Needs the daemon (docs/features/pr-review-gate.md).
    /// `--dismiss <n>` drops one finding; `--merge-anyway` lets the current head merge unreviewed.
    Review(ReviewArgs),
}

#[derive(Subcommand)]
pub(crate) enum QueueAction {
    /// Take a PR out of the line without dropping its watch; the PRs behind it carry on.
    Hold(QueueTarget),
    /// Put a held PR back at its place.
    Release(QueueTarget),
    /// Move a PR to the front: it is the next to land.
    Front(QueueTarget),
    /// Re-enter a PR the queue handed back to its session, without waiting for a new push.
    Retry(QueueTarget),
}

#[derive(clap::Args)]
pub(crate) struct QueueTarget {
    /// The PR number, or the session id when the same number is queued in two repositories.
    pub(crate) pr: String,
    /// Daemon base URL. Defaults to the local daemon's loopback origin.
    #[arg(long)]
    pub(crate) url: Option<String>,
    /// Daemon token. Defaults to the persisted `serve-token` in the config dir.
    #[arg(long)]
    pub(crate) token: Option<String>,
}

#[derive(clap::Args)]
pub(crate) struct ReviewArgs {
    #[command(flatten)]
    pub(crate) target: QueueTarget,
    /// Let this PR's current head merge once without waiting for a clean review.
    #[arg(long, conflicts_with = "dismiss")]
    pub(crate) merge_anyway: bool,
    /// Dismiss finding N (as numbered in the listing) so it no longer blocks the merge.
    #[arg(long, value_name = "N")]
    pub(crate) dismiss: Option<usize>,
}

#[derive(Subcommand)]
pub(crate) enum WorktreeOp {
    /// Every worktree with source and build-artifact sizes reported separately, age, branch, and
    /// merged/dirty/live state — worst offender first.
    List,
    /// Remove the worktrees that are provably safe: a stale registration, or a branch merged into
    /// the default branch with a clean tree, no unpushed commits, and no live session. Everything
    /// else is listed with the reason it was kept. Dry run unless `--yes`.
    Reclaim {
        /// Actually delete. Without this nothing is removed.
        #[arg(long)]
        yes: bool,
        /// Also prune stale Cargo `target/` directories, including from worktrees being kept.
        #[arg(long)]
        artifacts: bool,
        /// Require this many quiet hours since the newest artifact modification.
        #[arg(long, default_value_t = 24)]
        artifact_min_age_hours: u64,
    },
}
