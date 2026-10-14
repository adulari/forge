//! `forge stats` arguments.

use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub(crate) enum StatsCmd {
    /// Prompt-cache hit rate per model, the dollars the misses cost, and each miss's cause
    /// (`append`, `harness_rewrite`, `floor_advance`, `provider_eviction`, ...).
    Cache {
        /// Only this session (id or unique prefix); bare `--session` means the most recent one.
        #[arg(long, num_args = 0..=1, default_missing_value = "")]
        session: Option<String>,
        /// Look back this many days (ignored with `--session`).
        #[arg(long, default_value_t = 30)]
        days: u32,
    },
    /// Codemode (`run_code`) adoption: runs, script error rate, inner tool calls, and output
    /// tokens per step that ran a script next to other tool steps.
    Codemode {
        /// Only this session (id or unique prefix); bare `--session` means the most recent one.
        #[arg(long, num_args = 0..=1, default_missing_value = "")]
        session: Option<String>,
        /// Look back this many days (ignored with `--session`).
        #[arg(long, default_value_t = 30)]
        days: u32,
    },
}
