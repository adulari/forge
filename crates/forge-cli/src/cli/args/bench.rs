//! `forge bench` arguments.

use clap::Subcommand;

use crate::bench;

#[derive(Subcommand)]
pub(crate) enum BenchCmd {
    /// Generate SWE-bench predictions: run Forge on each instance and write predictions.jsonl
    /// (score it with the official `swebench` evaluator — see docs/benchmarks/swe-bench.md).
    Swe {
        /// SWE-bench dataset file (JSONL or a JSON array of instances).
        #[arg(long)]
        dataset: std::path::PathBuf,
        /// Where to write predictions.jsonl.
        #[arg(long, default_value = "predictions.jsonl")]
        out: std::path::PathBuf,
        /// Only run the first N instances (smoke runs).
        #[arg(long)]
        limit: Option<usize>,
        /// Pin a specific model. For --agent forge a Forge model id (provider::model); for
        /// claude-code / codex the CLI's own model name (e.g. `opus`, `gpt-5-codex`).
        #[arg(long)]
        model: Option<String>,
        /// Directory to clone/reuse instance repos under.
        #[arg(long, default_value = ".forge/swe-bench")]
        workdir: std::path::PathBuf,
        /// Which agent solves each instance — compare Forge against another CLI's own harness.
        #[arg(long, value_enum, default_value_t = bench::Agent::Forge)]
        agent: bench::Agent,
        /// Per-instance wall-clock budget for an external agent (seconds).
        #[arg(long, default_value_t = 1200)]
        timeout_secs: u64,
        /// Run each instance this many times (separate seeds) for pass@k / best-of-k. >1 writes
        /// one predictions file per seed (`<out>.seed1.jsonl`, …); aggregate with `bench passk`.
        #[arg(long, default_value_t = 1)]
        attempts: usize,
        /// Model that reviews each patch for reward hacking after the deterministic integrity
        /// checks. It can only escalate an attempt to an integrity FAIL, never clear one.
        #[arg(long)]
        judge: Option<String>,
    },
    /// Run the deterministic integrity checks over an existing predictions file (any harness):
    /// skip/xfail markers, collection-config edits, deleted asserts and FAIL_TO_PASS
    /// special-casing score 0; touched test files warn. Writes `<predictions>.integrity.jsonl`.
    Integrity {
        /// predictions.jsonl from `bench swe` or another harness.
        #[arg(long)]
        predictions: std::path::PathBuf,
        /// The SWE-bench dataset the predictions answer (supplies FAIL_TO_PASS).
        #[arg(long)]
        dataset: std::path::PathBuf,
        /// Optional reviewer model; it can only escalate to FAIL.
        #[arg(long)]
        judge: Option<String>,
    },
    /// Aggregate pass@k from several swebench evaluation reports (one per seed).
    Passk {
        /// The `*.json` reports written by `run_evaluation`, one per seed.
        #[arg(required = true)]
        reports: Vec<std::path::PathBuf>,
    },
    /// Join per-instance metrics with the official eval report(s) and print resolve rate AND
    /// tokens-per-success per agent — the headline efficiency comparison (e.g. Forge-bridging-X
    /// vs X's own CLI).
    Report {
        /// A metrics sidecar and the eval report that scores it, as `<metrics.jsonl>=<eval.json>`.
        /// Repeat once per agent and per seed; runs of the same agent + model are grouped.
        #[arg(long = "run", required_unless_present = "metrics")]
        runs: Vec<String>,
        /// `<out>.metrics.jsonl` sidecars from `bench swe`. Paired with `--eval` by position when
        /// the counts match.
        #[arg(long = "metrics")]
        metrics: Vec<std::path::PathBuf>,
        /// Official `run_evaluation` `*.json` report(s), one per `--metrics` in the same order;
        /// omit to print token/cost stats only.
        #[arg(long = "eval")]
        evals: Vec<std::path::PathBuf>,
        /// Apply the union of every `--eval` to every `--metrics` file (the old behaviour, for a
        /// single combined report). Credits each run with solves from the others.
        #[arg(long)]
        shared_eval: bool,
    },
}
