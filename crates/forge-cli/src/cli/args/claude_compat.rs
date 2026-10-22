//! `claude -p` print-mode flags for `forge run`. The flags that shape the session itself live in
//! [`super::session_flags::SessionFlags`] so `forge chat` can share them.
//!
//! Still rejected by clap rather than silently ignored: `--fork-session`, `--json-schema`,
//! `--settings`, `--tools`, `--agents`, `--worktree`, `--bare`. See
//! docs/features/claude-code-parity.md.

use clap::{Args, ValueEnum};

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum InputFormat {
    Text,
    /// One JSON user message per stdin line; each message is a turn of the same session.
    #[value(name = "stream-json")]
    StreamJson,
}

fn parse_budget(raw: &str) -> Result<f64, String> {
    match raw.trim().parse::<f64>() {
        Ok(usd) if usd.is_finite() && usd > 0.0 => Ok(usd),
        _ => Err(format!("`{raw}` is not a positive dollar amount")),
    }
}

#[derive(Args, Clone, Debug, Default)]
pub(crate) struct ClaudeCompat {
    /// Print mode, as in `claude -p`: stdout carries only the final answer. Progress goes to
    /// stderr when it is a terminal (or with `--verbose`). Without `-p`, `forge run` keeps its
    /// streamed line view.
    #[arg(short = 'p', long = "print", conflicts_with = "tui")]
    pub(crate) print: bool,
    /// With `-p`, also write the progress trace to stderr when stderr is not a terminal. Accepted
    /// with `--output-format stream-json`, whose event stream is always complete.
    #[arg(long)]
    pub(crate) verbose: bool,
    /// Input format: `text` (the prompt argument, plus piped stdin) or `stream-json` (one JSON user
    /// message per stdin line, each run as a turn of the same session).
    #[arg(long, value_enum, value_name = "FORMAT")]
    pub(crate) input_format: Option<InputFormat>,
    /// With `--output-format stream-json`, also emit token-level `stream_event` partials.
    #[arg(long)]
    pub(crate) include_partial_messages: bool,
    /// Stop the turn after this many model steps (maps to `mesh.max_steps`).
    #[arg(long, value_name = "N", value_parser = clap::value_parser!(u32).range(1..))]
    pub(crate) max_turns: Option<u32>,
    /// Stop once this run's estimated spend reaches this many dollars (priced like every other
    /// Forge spend guard). A stopped run exits 1 with an `error_max_budget_usd` result.
    #[arg(long, value_name = "USD", value_parser = parse_budget)]
    pub(crate) max_budget_usd: Option<f64>,
    /// Do not keep this run's session: it is deleted when the run ends, so it cannot be resumed.
    /// Its spend still counts toward `forge stats` and the budget caps.
    #[arg(long)]
    pub(crate) no_session_persistence: bool,
}

impl ClaudeCompat {
    /// Whether a daemon-hosted run could not honour these flags, so the run must stay local.
    pub(crate) fn needs_local_session(&self) -> bool {
        self.print
            || self.input_format == Some(InputFormat::StreamJson)
            || self.max_turns.is_some()
            || self.max_budget_usd.is_some()
            || self.no_session_persistence
    }
}

#[cfg(test)]
mod tests {
    use super::parse_budget;

    #[test]
    fn budgets_must_be_positive_dollar_amounts() {
        assert_eq!(parse_budget("5.00"), Ok(5.0));
        assert!(parse_budget("0").is_err());
        assert!(parse_budget("-1").is_err());
        assert!(parse_budget("lots").is_err());
    }
}
