//! Running a competing agent CLI on the same instance, and believing its numbers only when it
//! actually reported them.
//!
//! The comparison is only meaningful if the external agent runs under ITS OWN harness on the same
//! task and repo state, fully unattended. This module owns that invocation and the best-effort
//! usage extraction from whatever machine output the CLI emits — deliberately reporting
//! `metrics_complete = false` rather than guessing, so the comparison table can never claim token
//! numbers it cannot back up.

use std::path::Path;
use std::process::Stdio;

use anyhow::{Context, Result};

use super::Agent;

/// Token/cost figures parsed from an external CLI's machine output.
#[derive(Debug, Clone, Default, PartialEq)]
pub(super) struct ExternalUsage {
    /// Full input processed: uncached + cache reads + cache writes.
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: f64,
    /// False when nothing parsed, so the report never claims numbers it cannot back up.
    pub complete: bool,
    pub cached_input_tokens: Option<u64>,
    pub cache_write_tokens: Option<u64>,
    /// Claude Code's `total_cost_usd` is computed at list prices whatever the billing, so it is
    /// already a notional figure. Codex reports no cost, so this stays `None` for it.
    pub notional_cost_usd: Option<f64>,
}

/// What one external-agent run produced besides the working-tree diff.
pub(super) struct ExternalRun {
    pub usage: ExternalUsage,
    pub timed_out: bool,
    /// The agent's last answer, used only for refusal detection.
    pub final_text: Option<String>,
}

/// Run an external agent CLI (Claude Code / Codex) as its OWN autonomous agent in `dir`, feeding the
/// task on stdin. Both must run fully unattended (edit files + run commands without prompts) so they
/// can actually solve the instance — the clone is disposable, so the broad autonomy is contained.
/// Usage is parsed from the CLI's machine output where available (claude `--output-format json`);
/// `complete = false` when the CLI gave no parseable usage, so the report won't make token claims
/// it can't back up.
/// On timeout the child is killed and the run still returns Ok — the caller extracts whatever
/// partial diff the agent left in the working tree (parity with the Forge arm's timeout handling,
/// which was already submitting partial work while this path failed the whole instance).
pub(super) async fn run_external_agent(
    agent: Agent,
    problem: &str,
    dir: &Path,
    model: Option<&str>,
    timeout_secs: u64,
) -> Result<ExternalRun> {
    use tokio::io::AsyncWriteExt;

    let (bin, mut args): (&str, Vec<String>) = match agent {
        // `-p` reads the prompt from stdin; skip-permissions so edits + shell run unattended.
        // `--output-format json` makes claude emit a final result object with `usage` + cost.
        Agent::ClaudeCode => (
            "claude",
            vec![
                "-p".into(),
                "--output-format".into(),
                "json".into(),
                "--dangerously-skip-permissions".into(),
            ],
        ),
        // `exec` is codex's non-interactive mode; `--full-auto` = workspace-write + never-ask.
        // `--json` emits a JSONL event stream we scan for a token-count event (best-effort).
        Agent::Codex => (
            "codex",
            vec![
                "exec".into(),
                "--json".into(),
                "--skip-git-repo-check".into(),
                "--full-auto".into(),
            ],
        ),
        Agent::Forge => unreachable!("forge takes the in-process path"),
    };
    if let Some(m) = model {
        args.push("--model".into());
        args.push(m.to_string());
    }

    let mut child = tokio::process::Command::new(bin)
        .args(&args)
        .current_dir(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("spawning `{bin}` — is it installed and on PATH?"))?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(problem.as_bytes())
            .await
            .context("feeding problem to benchmark child via stdin")?;
        stdin.shutdown().await.ok();
    }

    // Drain stdout concurrently on a separate task so the child can't block on a full pipe, while we
    // wait on the process with a borrow (so we can still `start_kill` it on timeout — unlike
    // `wait_with_output`, which consumes the child).
    let stdout_pipe = child.stdout.take();
    let reader = tokio::spawn(async move {
        use tokio::io::AsyncReadExt;
        let mut buf = Vec::new();
        if let Some(mut so) = stdout_pipe {
            let _ = so.read_to_end(&mut buf).await;
        }
        buf
    });

    let waited =
        tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), child.wait()).await;
    match waited {
        Ok(Ok(st)) => {
            if !st.success() {
                // A non-zero exit is common (the agent may "fail" yet still have edited files) —
                // don't abort the instance; the diff (possibly empty) is captured by the caller.
                eprintln!("  (note: {bin} exited {st})");
            }
            let buf = reader.await.unwrap_or_default();
            let stdout = String::from_utf8_lossy(&buf);
            Ok(ExternalRun {
                usage: parse_external_usage(agent, &stdout),
                timed_out: false,
                final_text: parse_final_text(agent, &stdout),
            })
        }
        Ok(Err(e)) => {
            reader.abort();
            Err(anyhow::anyhow!("waiting on {bin}: {e}"))
        }
        Err(_) => {
            eprintln!("  {bin} hit the {timeout_secs}s timeout — submitting partial work");
            let _ = child.start_kill();
            let _ = child.wait().await;
            // The kill closed stdout, so the reader finishes with whatever streamed before the
            // timeout — codex's JSONL usage often parses from a partial stream; claude's single
            // final JSON object won't (metrics_complete stays honest either way).
            let buf = reader.await.unwrap_or_default();
            let stdout = String::from_utf8_lossy(&buf);
            Ok(ExternalRun {
                usage: parse_external_usage(agent, &stdout),
                timed_out: true,
                final_text: parse_final_text(agent, &stdout),
            })
        }
    }
}

/// Best-effort token/cost extraction from an external agent's machine output.
/// - claude (`--output-format json`): a single JSON object with `usage.{input_tokens,output_tokens,
///   cache_read_input_tokens}` and `total_cost_usd`.
/// - codex (`--json`): a JSONL event stream; we take the LAST object carrying token fields
///   (`input_tokens`/`output_tokens`, possibly nested under `usage`/`token_usage`/`info`).
///
/// `complete = false` when nothing parsed.
fn parse_external_usage(agent: Agent, stdout: &str) -> ExternalUsage {
    fn u(v: &serde_json::Value, keys: &[&str]) -> Option<u64> {
        keys.iter().find_map(|k| v.get(k).and_then(|x| x.as_u64()))
    }
    // Pull token/cost out of a single JSON object that may nest usage under a few known keys.
    // Claude's `input_tokens` excludes its cache read/write counters. Current Codex JSONL reports
    // cached/write input as subsets of `input_tokens`, matching the Responses API.
    fn from_obj(v: &serde_json::Value, cached_input_is_subset: bool) -> Option<ExternalUsage> {
        let usage = ["usage", "token_usage", "info", "tokens"]
            .iter()
            .find_map(|k| v.get(k))
            .unwrap_or(v);
        let inp = u(usage, &["input_tokens", "prompt_tokens", "input"]);
        let out = u(usage, &["output_tokens", "completion_tokens", "output"]);
        let (inp, out) = (inp?, out?);
        let cache_reported = u(usage, &["cache_read_input_tokens", "cached_input_tokens"]);
        let cache_write_reported = u(
            usage,
            &["cache_creation_input_tokens", "cache_write_input_tokens"],
        );
        let cache = cache_reported.unwrap_or(0);
        let cache_write = cache_write_reported.unwrap_or(0);
        let cost = ["total_cost_usd", "cost_usd", "cost"]
            .iter()
            .find_map(|k| v.get(k).and_then(|x| x.as_f64()))
            .unwrap_or(0.0);
        let full_input = if cached_input_is_subset {
            inp
        } else {
            inp + cache + cache_write
        };
        Some(ExternalUsage {
            input_tokens: full_input,
            output_tokens: out,
            cost_usd: cost,
            complete: true,
            cached_input_tokens: cache_reported,
            cache_write_tokens: cache_write_reported,
            notional_cost_usd: None,
        })
    }

    match agent {
        Agent::ClaudeCode => serde_json::from_str::<serde_json::Value>(stdout.trim())
            .ok()
            .and_then(|v| from_obj(&v, false))
            .map(|usage| ExternalUsage {
                notional_cost_usd: Some(usage.cost_usd),
                ..usage
            })
            .unwrap_or_default(),
        // Last JSONL line that yields token numbers wins (codex prints a running/final tally).
        Agent::Codex => stdout
            .lines()
            .filter_map(|l| serde_json::from_str::<serde_json::Value>(l.trim()).ok())
            .filter_map(|v| from_obj(&v, true))
            .next_back()
            .unwrap_or_default(),
        Agent::Forge => ExternalUsage::default(),
    }
}

/// The agent's final answer: claude's `result` field, or the text of codex's last
/// `agent_message` item.
fn parse_final_text(agent: Agent, stdout: &str) -> Option<String> {
    match agent {
        Agent::ClaudeCode => serde_json::from_str::<serde_json::Value>(stdout.trim())
            .ok()?
            .get("result")?
            .as_str()
            .map(str::to_string),
        Agent::Codex => stdout
            .lines()
            .filter_map(|l| serde_json::from_str::<serde_json::Value>(l.trim()).ok())
            .filter_map(|v| {
                let item = v.get("item")?;
                (item.get("type")?.as_str()? == "agent_message")
                    .then(|| item.get("text")?.as_str().map(str::to_string))
                    .flatten()
            })
            .next_back(),
        Agent::Forge => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_claude_json_usage() {
        let out = r#"{"type":"result","is_error":false,"result":"done","total_cost_usd":0.0345,"usage":{"input_tokens":1200,"output_tokens":340,"cache_read_input_tokens":800}}"#;
        let u = parse_external_usage(Agent::ClaudeCode, out);
        assert!(u.complete);
        assert_eq!(u.input_tokens, 2000, "input + cache_read folded in"); // 1200 + 800
        assert_eq!(u.output_tokens, 340);
        assert!((u.cost_usd - 0.0345).abs() < 1e-9);
        assert_eq!(u.cached_input_tokens, Some(800));
        assert_eq!(u.cache_write_tokens, None, "not reported → unknown");
        assert_eq!(
            u.notional_cost_usd,
            Some(u.cost_usd),
            "claude prices at list rates"
        );
    }

    #[test]
    fn parse_claude_json_usage_counts_cache_reads_and_writes() {
        // The full input the model processed = uncached + cache reads + cache writes. Counting only
        // the uncached `input_tokens` (the old bug) made a prompt-cached run look ~free and broke the
        // Forge-vs-raw-CLI comparison (the bridge undercounted the same way).
        let out = r#"{"usage":{"input_tokens":1000,"output_tokens":50,"cache_read_input_tokens":40000,"cache_creation_input_tokens":5000}}"#;
        let u = parse_external_usage(Agent::ClaudeCode, out);
        assert!(u.complete);
        assert_eq!(
            u.input_tokens, 46000,
            "1000 uncached + 40000 cache-read + 5000 cache-write"
        );
        assert_eq!(u.output_tokens, 50);
        assert_eq!(u.cache_write_tokens, Some(5000));
    }

    #[test]
    fn parse_codex_jsonl_takes_last_token_event() {
        let out = "{\"type\":\"start\"}\n{\"token_usage\":{\"input_tokens\":10,\"output_tokens\":5}}\n{\"token_usage\":{\"input_tokens\":900,\"output_tokens\":120}}\n";
        let u = parse_external_usage(Agent::Codex, out);
        assert!(u.complete);
        assert_eq!(
            (u.input_tokens, u.output_tokens),
            (900, 120),
            "last tally wins"
        );
        assert_eq!(u.notional_cost_usd, None, "codex reports no cost");
    }

    #[test]
    fn parse_codex_cached_input_is_a_subset_not_an_addend() {
        let out = r#"{"type":"turn.completed","usage":{"input_tokens":16646,"cached_input_tokens":9984,"cache_write_input_tokens":512,"output_tokens":8}}"#;
        let u = parse_external_usage(Agent::Codex, out);
        assert!(u.complete);
        assert_eq!(
            (u.input_tokens, u.output_tokens),
            (16646, 8),
            "Codex input already includes cached/write subsets"
        );
        assert_eq!(u.cached_input_tokens, Some(9984));
        assert_eq!(u.cache_write_tokens, Some(512));
    }

    #[test]
    fn parse_external_usage_incomplete_on_garbage() {
        let u = parse_external_usage(Agent::ClaudeCode, "not json at all");
        assert!(
            !u.complete,
            "unparseable output → metrics_complete=false, not a lie"
        );
    }

    #[test]
    fn final_text_comes_from_claude_result_and_the_last_codex_agent_message() {
        let claude = r#"{"type":"result","result":"I can't help with that.","usage":{"input_tokens":1,"output_tokens":1}}"#;
        assert_eq!(
            parse_final_text(Agent::ClaudeCode, claude).as_deref(),
            Some("I can't help with that.")
        );
        let codex = "{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"first\"}}\n{\"type\":\"item.completed\",\"item\":{\"type\":\"command_execution\",\"command\":\"ls\"}}\n{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"done\"}}\n";
        assert_eq!(
            parse_final_text(Agent::Codex, codex).as_deref(),
            Some("done")
        );
    }
}
