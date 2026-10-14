//! `forge bench export`: one redacted trajectory JSONL per benchmark attempt.
//!
//! Reads a metrics sidecar (`bench swe` or `bench tasks`), joins each row's Forge session back to
//! the store through `Store::load_replay` (the same read path as `forge replay`) and writes
//! `<out>/<task>.attempt<k>.jsonl`. Line 1 is an `attempt` header (task, agent, model, outcome,
//! integrity verdict, usage totals); every following line is one transcript `step`. Every string
//! goes through the shared redactor (configured secret values, known key shapes, keyword-adjacent
//! values) BEFORE strings longer than 8 KB are truncated, so truncation never splits a secret
//! past the redactor. Rows without a session (external CLI agents) get a header-only file.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use forge_core::redact::Redactor;
use forge_store::ReplayEntry;
use serde_json::{json, Value};

use super::integrity::{integrity_path, IntegrityReport};
use super::report::load_metrics;
use super::InstanceMetric;

/// Longest string kept verbatim in an exported record.
pub(super) const MAX_FIELD_BYTES: usize = 8 * 1024;

/// `x.metrics.jsonl` → `x.integrity.jsonl`, the integrity sidecar `bench swe` wrote next to it.
fn sibling_integrity(metrics: &Path) -> Option<PathBuf> {
    let name = metrics.file_name()?.to_str()?;
    let (stem, ext) = name.split_once(".metrics.")?;
    Some(integrity_path(
        &metrics.with_file_name(format!("{stem}.{ext}")),
    ))
}

fn load_integrity(path: &Path) -> Result<HashMap<String, IntegrityReport>> {
    let raw =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    raw.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let r: IntegrityReport = serde_json::from_str(l)
                .with_context(|| format!("parsing integrity row in {}", path.display()))?;
            Ok((r.instance_id.clone(), r))
        })
        .collect()
}

/// Truncate `s` to at most `max` bytes on a char boundary, saying how much was dropped.
pub(super) fn truncate_field(s: &mut String, max: usize) {
    if s.len() <= max {
        return;
    }
    let mut cut = max;
    while !s.is_char_boundary(cut) {
        cut -= 1;
    }
    let dropped = s.len() - cut;
    s.truncate(cut);
    s.push_str(&format!("…[truncated {dropped} bytes]"));
}

fn truncate_strings(value: &mut Value, max: usize) {
    match value {
        Value::String(s) => truncate_field(s, max),
        Value::Array(items) => items.iter_mut().for_each(|v| truncate_strings(v, max)),
        Value::Object(map) => map.values_mut().for_each(|v| truncate_strings(v, max)),
        _ => {}
    }
}

/// Redact, then truncate, every string in `value`.
pub(super) fn scrub(redactor: &Redactor, mut value: Value) -> Value {
    redactor.redact_json(&mut value);
    truncate_strings(&mut value, MAX_FIELD_BYTES);
    value
}

fn model_of(metric: &InstanceMetric, entries: &[ReplayEntry]) -> Option<String> {
    metric
        .runtime
        .as_ref()
        .and_then(|r| {
            r.resolved_model
                .clone()
                .or_else(|| r.requested_model.clone())
        })
        .or_else(|| entries.iter().rev().find_map(|e| e.model.clone()))
}

pub(super) fn header_record(
    metric: &InstanceMetric,
    entries: &[ReplayEntry],
    integrity: Option<&IntegrityReport>,
    trajectory: &str,
) -> Value {
    json!({
        "type": "attempt",
        "task": metric.instance_id,
        "agent": metric.agent,
        "model": model_of(metric, entries),
        "attempt": metric.attempt,
        "session_id": metric.session_id,
        "outcome": metric.outcome,
        "outcome_detail": metric.outcome_detail,
        "patched": metric.patched,
        "timed_out": metric.timed_out,
        "integrity": integrity.map(|r| json!({
            "verdict": r.verdict,
            "findings": r.findings,
            "judge": r.judge,
        })),
        "usage": {
            "input_tokens": metric.input_tokens,
            "cached_input_tokens": metric.cached_input_tokens,
            "cache_write_tokens": metric.cache_write_tokens,
            "reasoning_tokens": metric.reasoning_tokens,
            "output_tokens": metric.output_tokens,
            "cost_usd": metric.cost_usd,
            "notional_cost_usd": metric.notional_cost_usd,
            "model_calls": metric.model_calls,
            "tool_calls": metric.tool_calls,
            "wall_secs": metric.wall_secs,
        },
        "steps": entries.len(),
        "trajectory": trajectory,
    })
}

pub(super) fn step_record(entry: &ReplayEntry) -> Value {
    json!({
        "type": "step",
        "seq": entry.seq,
        "role": entry.role,
        "model": entry.model,
        "created_at": entry.created_at,
        "content": entry.content,
        "tool_calls": entry.tool_calls.iter().map(|c| json!({
            "id": c.id,
            "name": c.name,
            "args": c.args,
        })).collect::<Vec<_>>(),
        "input_tokens": entry.input_tokens,
        "output_tokens": entry.output_tokens,
        "cost_usd": entry.cost_usd,
    })
}

fn file_name_for(metric: &InstanceMetric) -> String {
    let task: String = metric
        .instance_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("{task}.attempt{}.jsonl", metric.attempt)
}

/// Render one attempt's export: a header line then one line per step, all scrubbed.
pub(super) fn render_attempt(
    redactor: &Redactor,
    metric: &InstanceMetric,
    entries: &[ReplayEntry],
    integrity: Option<&IntegrityReport>,
    trajectory: &str,
) -> String {
    let mut out = String::new();
    let header = scrub(
        redactor,
        header_record(metric, entries, integrity, trajectory),
    );
    out.push_str(&serde_json::to_string(&header).expect("header serializes"));
    out.push('\n');
    for entry in entries {
        let step = scrub(redactor, step_record(entry));
        out.push_str(&serde_json::to_string(&step).expect("step serializes"));
        out.push('\n');
    }
    out
}

pub(crate) fn export_cmd(metrics: &[PathBuf], out: &Path, integrity: Option<&Path>) -> Result<()> {
    std::fs::create_dir_all(out).with_context(|| format!("creating {}", out.display()))?;
    let store = crate::open_store()?;
    let redactor = Redactor::from_environment();
    let mut written = 0usize;
    let mut steps = 0usize;
    for metrics_file in metrics {
        let rows = load_metrics(metrics_file)?;
        let integrity_file = integrity
            .map(Path::to_path_buf)
            .or_else(|| sibling_integrity(metrics_file).filter(|p| p.is_file()));
        let reports = match &integrity_file {
            Some(p) => load_integrity(p)?,
            None => HashMap::new(),
        };
        for metric in &rows {
            let (entries, trajectory) = match &metric.session_id {
                Some(id) => (
                    store
                        .load_replay(id)
                        .with_context(|| format!("loading session {id}"))?,
                    "forge_session",
                ),
                None => (Vec::new(), "unavailable_no_forge_session"),
            };
            if metric.session_id.is_some() && entries.is_empty() {
                eprintln!(
                    "⚠ {} attempt {}: session not in this store (set FORGE_DB to the store the run used)",
                    metric.instance_id, metric.attempt
                );
            }
            let body = render_attempt(
                &redactor,
                metric,
                &entries,
                reports.get(&metric.instance_id),
                if metric.session_id.is_some() && entries.is_empty() {
                    "missing_from_store"
                } else {
                    trajectory
                },
            );
            let path = out.join(file_name_for(metric));
            std::fs::write(&path, body).with_context(|| format!("writing {}", path.display()))?;
            written += 1;
            steps += entries.len();
        }
    }
    eprintln!(
        "exported {written} attempt(s), {steps} step(s) → {} (redacted with {} known secret value(s) + shape rules; review before publishing)",
        out.display(),
        redactor.known_count()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_types::{Role, ToolCall};

    fn entry(seq: i64, role: Role, content: &str, args: Value) -> ReplayEntry {
        ReplayEntry {
            seq,
            role,
            content: content.into(),
            model: Some("mock::m".into()),
            created_at: 0,
            tool_calls: if args.is_null() {
                vec![]
            } else {
                vec![ToolCall {
                    id: "c1".into(),
                    name: "shell".into(),
                    args,
                }]
            },
            input_tokens: Some(10),
            output_tokens: Some(2),
            cost_usd: Some(0.0),
        }
    }

    #[test]
    fn export_redacts_planted_secrets_everywhere_and_keeps_shas() {
        let planted = "zq9-PLANTED-bench-secret-value-77";
        let redactor = Redactor::new([planted.to_string()]);
        let sha = "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b";
        let metric = InstanceMetric {
            instance_id: "task/a".into(),
            agent: "forge".into(),
            attempt: 2,
            session_id: Some("s-1".into()),
            outcome_detail: Some(format!("error mentioning {planted}")),
            ..Default::default()
        };
        let entries = vec![
            entry(
                1,
                Role::User,
                &format!("use key {planted} at {sha}"),
                Value::Null,
            ),
            entry(
                2,
                Role::Assistant,
                "running",
                json!({"command": format!("curl -H 'Authorization: Bearer {planted}'")}),
            ),
            entry(
                3,
                Role::Tool,
                "ghp_abcdefghijklmnopqrstuvwxyz0123456789",
                Value::Null,
            ),
        ];
        let out = render_attempt(&redactor, &metric, &entries, None, "forge_session");
        assert!(!out.contains(planted), "{out}");
        assert!(!out.contains("ghp_abcdefghijklmnopqrstuvwxyz0123456789"));
        assert!(out.contains(sha), "git SHAs must survive redaction");
        assert!(out.contains("[redacted]"));
        let lines: Vec<Value> = out
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(lines.len(), 4);
        assert_eq!(lines[0]["type"], "attempt");
        assert_eq!(lines[0]["attempt"], 2);
        assert_eq!(lines[0]["model"], "mock::m");
        assert_eq!(lines[2]["tool_calls"][0]["name"], "shell");
        assert_eq!(file_name_for(&metric), "task_a.attempt2.jsonl");
    }

    #[test]
    fn long_fields_are_truncated_after_redaction() {
        let planted = "zq9-PLANTED-bench-secret-value-77";
        let redactor = Redactor::new([planted.to_string()]);
        let big = format!("{}{planted}", "é".repeat(MAX_FIELD_BYTES));
        let v = scrub(&redactor, json!({ "content": big }));
        let s = v["content"].as_str().unwrap();
        assert!(s.contains("…[truncated "), "{}", &s[s.len() - 40..]);
        assert!(!s.contains(planted));
        assert!(s.len() < MAX_FIELD_BYTES + 64);
    }

    #[test]
    fn integrity_sidecar_is_found_next_to_the_metrics() {
        assert_eq!(
            sibling_integrity(Path::new("/r/preds.seed1.metrics.jsonl")),
            Some(PathBuf::from("/r/preds.seed1.integrity.jsonl"))
        );
        assert_eq!(sibling_integrity(Path::new("/r/plain.jsonl")), None);
    }
}
