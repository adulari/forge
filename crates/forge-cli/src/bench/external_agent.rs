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

/// What the agent's own machine output says about how the turn went (Claude Code only).
#[derive(Debug, Clone, Default, PartialEq)]
pub(super) struct ExternalStats {
    /// Model round trips: the result object's `num_turns`, else the assistant messages seen.
    pub num_turns: Option<u64>,
    /// `tool_use` blocks in the stream; `None` when the output carried no stream to count.
    pub tool_calls: Option<u64>,
    /// Tool results that came back flagged as errors.
    pub tool_errors: Option<u64>,
    /// The CLI's own timing of the turn, from the result object.
    pub duration_ms: Option<u64>,
    pub ttft_ms: Option<u64>,
}

/// What one external-agent run produced besides the working-tree diff.
pub(super) struct ExternalRun {
    pub usage: ExternalUsage,
    pub stats: ExternalStats,
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
    clean: bool,
) -> Result<ExternalRun> {
    use tokio::io::AsyncWriteExt;

    let (bin, mut args): (&str, Vec<String>) = match agent {
        // `-p` reads the prompt from stdin; skip-permissions so edits + shell run unattended.
        // `stream-json --verbose` emits every message and ends with the result object (`usage`,
        // cost, `num_turns`, timings); the messages are where the tool calls are counted from.
        Agent::ClaudeCode => (
            "claude",
            vec![
                "-p".into(),
                "--output-format".into(),
                "stream-json".into(),
                "--verbose".into(),
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
    if clean && agent == Agent::ClaudeCode {
        // No user hooks, CLAUDE.md, skills or MCP servers: only what the workspace itself holds.
        args.extend(["--setting-sources", "project", "--strict-mcp-config"].map(String::from));
    }
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
                stats: parse_external_stats(agent, &stdout),
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
            // timeout — codex's JSONL usage often parses from a partial stream; claude's usage
            // comes only with its final result line, so a killed run keeps its tool and turn
            // counts but not its tokens (metrics_complete stays honest either way).
            let buf = reader.await.unwrap_or_default();
            let stdout = String::from_utf8_lossy(&buf);
            Ok(ExternalRun {
                usage: parse_external_usage(agent, &stdout),
                stats: parse_external_stats(agent, &stdout),
                timed_out: true,
                final_text: parse_final_text(agent, &stdout),
            })
        }
    }
}

/// Best-effort token/cost extraction from an external agent's machine output.
/// - claude (`--output-format stream-json`, or the older single-object `json`): the `result` object
///   with `usage.{input_tokens,output_tokens,cache_read_input_tokens}` and `total_cost_usd`.
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
        Agent::ClaudeCode => claude_result(stdout)
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

/// Claude Code's `result` object from its output: the whole document in `--output-format json`
/// mode, the last `{"type":"result"}` line of a `stream-json` run. `None` when the run never got
/// that far (killed, or not JSON at all).
fn claude_result(stdout: &str) -> Option<serde_json::Value> {
    if let Ok(whole) = serde_json::from_str::<serde_json::Value>(stdout.trim()) {
        return whole.is_object().then_some(whole);
    }
    stdout
        .lines()
        .filter(|l| l.contains(r#""type":"result""#) || l.contains(r#""type": "result""#))
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l.trim()).ok())
        .rfind(|v| v.get("type").and_then(|t| t.as_str()) == Some("result"))
}

/// Turns, tool calls and timings of a Claude Code run, from its `stream-json` output. A run that
/// was killed before its result line still has every message it printed, so its tool calls and
/// assistant turns are counted from those.
fn parse_external_stats(agent: Agent, stdout: &str) -> ExternalStats {
    if agent != Agent::ClaudeCode {
        return ExternalStats::default();
    }
    let mut tool_ids = std::collections::HashSet::new();
    let mut message_ids = std::collections::HashSet::new();
    let mut errors = 0u64;
    let mut streamed = false;
    for line in stdout.lines() {
        let Ok(event) = serde_json::from_str::<serde_json::Value>(line.trim()) else {
            continue;
        };
        let kind = event.get("type").and_then(|t| t.as_str());
        streamed |= matches!(kind, Some("system" | "assistant" | "user"));
        // Sub-agent traffic is nested under the tool call that started it; only the main
        // conversation's calls are this agent's.
        if !event.get("parent_tool_use_id").is_none_or(|p| p.is_null()) {
            continue;
        }
        let blocks = event
            .pointer("/message/content")
            .and_then(|c| c.as_array())
            .into_iter()
            .flatten();
        match kind {
            Some("assistant") => {
                if let Some(id) = event.pointer("/message/id").and_then(|i| i.as_str()) {
                    message_ids.insert(id.to_string());
                }
                for block in blocks {
                    if block.get("type").and_then(|t| t.as_str()) == Some("tool_use") {
                        let id = block.get("id").and_then(|i| i.as_str()).unwrap_or_default();
                        tool_ids.insert(id.to_string());
                    }
                }
            }
            Some("user") => {
                errors += blocks
                    .filter(|b| b.get("type").and_then(|t| t.as_str()) == Some("tool_result"))
                    .filter(|b| b.get("is_error").and_then(|e| e.as_bool()) == Some(true))
                    .count() as u64;
            }
            _ => {}
        }
    }
    let result = claude_result(stdout);
    let field = |key: &str| result.as_ref().and_then(|r| r.get(key)?.as_u64());
    ExternalStats {
        num_turns: field("num_turns")
            .or_else(|| (!message_ids.is_empty()).then_some(message_ids.len() as u64)),
        tool_calls: streamed.then_some(tool_ids.len() as u64),
        tool_errors: streamed.then_some(errors),
        duration_ms: field("duration_ms"),
        ttft_ms: field("ttft_ms"),
    }
}

/// The agent's final answer: claude's `result` field, or the text of codex's last
/// `agent_message` item.
fn parse_final_text(agent: Agent, stdout: &str) -> Option<String> {
    match agent {
        Agent::ClaudeCode => claude_result(stdout)?
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

    /// A trimmed real `claude -p --output-format stream-json --verbose` run: one assistant message
    /// split over two lines (same id), two tool calls (one fails), a final message, the result.
    const CLAUDE_STREAM: &str = concat!(
        r#"{"type":"system","subtype":"init","session_id":"s"}"#,
        "\n",
        r#"{"type":"assistant","message":{"id":"m1","content":[{"type":"thinking","thinking":"hm"}]}}"#,
        "\n",
        r#"{"type":"assistant","message":{"id":"m1","content":[{"type":"tool_use","id":"t1","name":"Read","input":{}}]}}"#,
        "\n",
        r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","is_error":false,"content":"ok"}]}}"#,
        "\n",
        r#"{"type":"assistant","message":{"id":"m2","content":[{"type":"tool_use","id":"t2","name":"Bash","input":{}}]}}"#,
        "\n",
        r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t2","is_error":true,"content":"boom"}]}}"#,
        "\n",
        r#"{"type":"assistant","parent_tool_use_id":"t2","message":{"id":"sub1","content":[{"type":"tool_use","id":"t9","name":"Read","input":{}}]}}"#,
        "\n",
        r#"{"type":"assistant","message":{"id":"m3","content":[{"type":"text","text":"done"}]}}"#,
        "\n",
        r#"{"type":"result","subtype":"success","is_error":false,"result":"done","num_turns":3,"duration_ms":4200,"ttft_ms":900,"total_cost_usd":0.01,"usage":{"input_tokens":10,"output_tokens":5,"cache_read_input_tokens":100}}"#,
        "\n",
    );

    #[test]
    fn stream_json_gives_turns_tool_calls_errors_and_timings() {
        let stats = parse_external_stats(Agent::ClaudeCode, CLAUDE_STREAM);
        assert_eq!(
            stats,
            ExternalStats {
                num_turns: Some(3),
                tool_calls: Some(2),
                tool_errors: Some(1),
                duration_ms: Some(4200),
                ttft_ms: Some(900),
            },
            "the sub-agent call (parent_tool_use_id) is not this agent's"
        );
        let usage = parse_external_usage(Agent::ClaudeCode, CLAUDE_STREAM);
        assert!(usage.complete);
        assert_eq!(usage.input_tokens, 110, "input + cache read");
        assert_eq!(usage.output_tokens, 5);
        assert_eq!(
            parse_final_text(Agent::ClaudeCode, CLAUDE_STREAM).as_deref(),
            Some("done")
        );
    }

    #[test]
    fn a_killed_stream_keeps_its_counts_but_claims_no_tokens() {
        let killed: String = CLAUDE_STREAM
            .lines()
            .filter(|l| !l.contains(r#""type":"result""#))
            .map(|l| format!("{l}\n"))
            .collect();
        let stats = parse_external_stats(Agent::ClaudeCode, &killed);
        assert_eq!(stats.tool_calls, Some(2));
        assert_eq!(stats.tool_errors, Some(1));
        assert_eq!(
            stats.num_turns,
            Some(3),
            "distinct assistant message ids m1..m3"
        );
        assert_eq!((stats.duration_ms, stats.ttft_ms), (None, None));
        assert!(!parse_external_usage(Agent::ClaudeCode, &killed).complete);
    }

    #[test]
    fn the_older_single_object_output_still_parses_without_tool_counts() {
        let old = r#"{"type":"result","result":"x","num_turns":2,"duration_ms":10,"usage":{"input_tokens":1,"output_tokens":1}}"#;
        let stats = parse_external_stats(Agent::ClaudeCode, old);
        assert_eq!(stats.num_turns, Some(2));
        assert_eq!(stats.duration_ms, Some(10));
        assert_eq!(
            stats.tool_calls, None,
            "nothing streamed, so nothing to count"
        );
        assert_eq!(
            parse_external_stats(Agent::Codex, CLAUDE_STREAM),
            ExternalStats::default()
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
