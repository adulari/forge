//! Reading benchmark results back out: pass@k across seeds and the agent comparison table.
//!
//! The prediction harness writes two artifacts per run — `predictions.jsonl` for the official
//! scorer and a `*.metrics.jsonl` sidecar of what each instance cost. This module is the read
//! side: it joins those sidecars with the scorer's `resolved_ids` and prints the numbers Forge's
//! efficiency claim rests on. The aggregation is pure, so the honesty rules around
//! tokens-per-success (never report it from a partial capture) are locked down by tests.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::InstanceMetric;

/// Aggregate pass@k from several swebench evaluation reports (the `*.json` written by
/// `run_evaluation`, one per seed). pass@k = an instance counts as solved if ANY seed resolved it;
/// also prints each seed's own resolved count so variance is visible.
pub(crate) fn passk(reports: &[PathBuf]) -> Result<()> {
    use std::collections::BTreeSet;
    if reports.is_empty() {
        anyhow::bail!("pass@k needs at least one report (the *.json from run_evaluation)");
    }
    let mut union: BTreeSet<String> = BTreeSet::new();
    let mut submitted = 0usize;
    for (i, path) in reports.iter().enumerate() {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("reading report {}", path.display()))?;
        let v: serde_json::Value =
            serde_json::from_str(&raw).with_context(|| format!("parsing {}", path.display()))?;
        let resolved: Vec<String> = v
            .get("resolved_ids")
            .and_then(|x| x.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|s| s.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        submitted = submitted.max(
            v.get("submitted_instances")
                .and_then(|x| x.as_u64())
                .unwrap_or(0) as usize,
        );
        eprintln!(
            "  seed {}: {} resolved  ({})",
            i + 1,
            resolved.len(),
            path.display()
        );
        union.extend(resolved);
    }
    let k = reports.len();
    let denom = submitted.max(union.len()).max(1);
    eprintln!(
        "pass@{k}: {} / {} resolved by at least one seed  ({:.0}%)",
        union.len(),
        denom,
        union.len() as f64 / denom as f64 * 100.0
    );
    Ok(())
}

/// Load a `<out>.metrics.jsonl` sidecar (one [`InstanceMetric`] per line).
pub(super) fn load_metrics(path: &Path) -> Result<Vec<InstanceMetric>> {
    let raw =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    raw.lines()
        .filter(|l| !l.trim().is_empty())
        .enumerate()
        .map(|(i, l)| {
            serde_json::from_str::<InstanceMetric>(l)
                .with_context(|| format!("parsing metrics line {}", i + 1))
        })
        .collect()
}

/// Read the set of resolved `instance_id`s from one official `swebench` evaluation report
/// (the `*.json` from `run_evaluation`, which carries `resolved_ids`).
fn resolved_ids_from_report(path: &Path) -> Result<Vec<String>> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("reading report {}", path.display()))?;
    let v: serde_json::Value =
        serde_json::from_str(&raw).with_context(|| format!("parsing {}", path.display()))?;
    Ok(v.get("resolved_ids")
        .and_then(|x| x.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|s| s.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default())
}

/// One agent's aggregated numbers for the comparison table.
struct AgentSummary {
    agent: String,
    instances: usize,
    patched: usize,
    resolved: usize,
    total_tokens: u64,
    total_cost: f64,
    total_wall: f64,
    complete: usize,
}

/// Aggregate one agent's per-instance metric rows against the official `resolved` set into an
/// [`AgentSummary`]. Pure (no I/O) so the headline comparison's arithmetic is unit-testable: an
/// instance counts as `resolved` iff the scorer put its id in `resolved`; token/cost totals only
/// include rows whose capture was `metrics_complete` (so a partial capture can't understate
/// tokens-per-success and flatter Forge). Assumes `rows` is non-empty (one file = one agent).
fn summarize_agent(
    rows: &[InstanceMetric],
    resolved: &std::collections::BTreeSet<String>,
) -> AgentSummary {
    let mut s = AgentSummary {
        agent: rows[0].agent.clone(),
        instances: rows.len(),
        patched: 0,
        resolved: 0,
        total_tokens: 0,
        total_cost: 0.0,
        total_wall: 0.0,
        complete: 0,
    };
    for r in rows {
        if r.patched {
            s.patched += 1;
        }
        if resolved.contains(&r.instance_id) {
            s.resolved += 1;
        }
        if r.metrics_complete {
            s.complete += 1;
            s.total_tokens += r.total_tokens;
            s.total_cost += r.cost_usd;
        }
        s.total_wall += r.wall_secs;
    }
    s
}

/// The tokens-per-success cell: total tokens (across all attempts) per resolved instance — the
/// efficiency number, lower is better. Only an honest number when there ARE eval results, at least
/// one instance resolved, AND every row's token capture was complete; otherwise it's `incomplete`
/// (some capture missing → would understate) or `n/a` (no evals / nothing resolved). Pure so the
/// honesty conditions are locked down by tests — this is the headline efficiency claim.
fn tok_per_success_cell(s: &AgentSummary, have_evals: bool) -> String {
    if have_evals && s.resolved > 0 && s.complete == s.instances {
        format!("{}", s.total_tokens / s.resolved as u64)
    } else if s.complete < s.instances {
        "incomplete".to_string()
    } else {
        "n/a".to_string()
    }
}

/// The headline comparison: join per-instance metrics with the official eval's `resolved_ids` and
/// print, per agent, **both** the resolve rate AND tokens-per-success (+ cost/wall). This is how
/// "Forge bridging model X beats running model X's own CLI" is shown — same instances, same scorer,
/// fewer tokens per solved task. `metrics` files come from `bench swe`; `evals` are the official
/// `run_evaluation` `*.json` reports (their resolved-id sets are unioned, then intersected with each
/// agent's instances, so one combined report or per-agent reports both work).
pub(crate) fn report(metrics: &[PathBuf], evals: &[PathBuf]) -> Result<()> {
    use std::collections::BTreeSet;
    if metrics.is_empty() {
        anyhow::bail!("report needs at least one --metrics <file.metrics.jsonl>");
    }
    let mut resolved: BTreeSet<String> = BTreeSet::new();
    for e in evals {
        resolved.extend(resolved_ids_from_report(e)?);
    }
    let have_evals = !evals.is_empty();

    let mut summaries = Vec::new();
    for m in metrics {
        let rows = load_metrics(m)?;
        if rows.is_empty() {
            continue;
        }
        summaries.push(summarize_agent(&rows, &resolved));
    }
    if summaries.is_empty() {
        anyhow::bail!("no metrics rows found in the given files");
    }

    println!(
        "{:<14} {:>5} {:>8} {:>9} {:>13} {:>11} {:>9}",
        "agent", "n", "patched", "resolved", "tok/success", "mean cost", "mean s"
    );
    for s in &summaries {
        let resolved_str = if have_evals {
            format!("{} ({:.0}%)", s.resolved, pct(s.resolved, s.instances))
        } else {
            "n/a".to_string()
        };
        let tok_per_success = tok_per_success_cell(s, have_evals);
        let mean_cost = if s.complete > 0 {
            format!("${:.4}", s.total_cost / s.complete as f64)
        } else {
            "n/a".to_string()
        };
        println!(
            "{:<14} {:>5} {:>8} {:>9} {:>13} {:>11} {:>9.1}",
            s.agent,
            s.instances,
            s.patched,
            resolved_str,
            tok_per_success,
            mean_cost,
            s.total_wall / s.instances as f64,
        );
    }
    if !have_evals {
        eprintln!(
            "\nnote: no --eval reports given → resolve rate + tok/success omitted. Score predictions\nwith the official evaluator, then re-run with --eval <report.json>."
        );
    }
    Ok(())
}

fn pct(n: usize, d: usize) -> f64 {
    if d == 0 {
        0.0
    } else {
        n as f64 / d as f64 * 100.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk(id: &str, patched: bool, complete: bool, tokens: u64) -> InstanceMetric {
        InstanceMetric {
            instance_id: id.into(),
            agent: "forge".into(),
            input_tokens: tokens,
            output_tokens: 0,
            total_tokens: tokens,
            cost_usd: 0.01,
            wall_secs: 2.0,
            patched,
            metrics_complete: complete,
            timed_out: false,
            tools_unavailable: false,
            runtime: None,
        }
    }

    #[test]
    fn summarize_agent_counts_resolved_patched_and_complete() {
        use std::collections::BTreeSet;
        let rows = vec![
            mk("a-1", true, true, 100),  // patched, complete, RESOLVED
            mk("a-2", true, true, 200),  // patched, complete, not resolved
            mk("a-3", false, false, 50), // not patched, INCOMPLETE, resolved
        ];
        let resolved: BTreeSet<String> = ["a-1", "a-3"].iter().map(|s| s.to_string()).collect();
        let s = summarize_agent(&rows, &resolved);
        assert_eq!(s.agent, "forge");
        assert_eq!(s.instances, 3);
        assert_eq!(s.patched, 2);
        assert_eq!(s.resolved, 2, "a-1 + a-3 are in the resolved set");
        assert_eq!(s.complete, 2, "a-3's capture was incomplete");
        // Only complete rows contribute tokens — a-3's 50 is excluded so tok/success can't be understated.
        assert_eq!(s.total_tokens, 300);
        assert_eq!(s.total_wall, 6.0);
    }

    #[test]
    fn tok_per_success_is_honest_only_with_complete_capture() {
        // All complete, evals present, 2 resolved, 300 tokens → 150 per success.
        let full = AgentSummary {
            agent: "forge".into(),
            instances: 2,
            patched: 2,
            resolved: 2,
            total_tokens: 300,
            total_cost: 0.02,
            total_wall: 4.0,
            complete: 2,
        };
        assert_eq!(tok_per_success_cell(&full, true), "150");
        // No eval reports → can't claim a success rate → n/a (even with complete capture).
        assert_eq!(tok_per_success_cell(&full, false), "n/a");
        // Resolved zero → dividing would be meaningless → n/a.
        let none_resolved = AgentSummary {
            resolved: 0,
            ..AgentSummary {
                agent: "forge".into(),
                instances: 2,
                patched: 0,
                resolved: 0,
                total_tokens: 300,
                total_cost: 0.0,
                total_wall: 0.0,
                complete: 2,
            }
        };
        assert_eq!(tok_per_success_cell(&none_resolved, true), "n/a");
        // Partial token capture (complete < instances) → refuse to print a flattering number.
        let partial = AgentSummary {
            instances: 3,
            complete: 2,
            resolved: 2,
            total_tokens: 300,
            ..full
        };
        assert_eq!(tok_per_success_cell(&partial, true), "incomplete");
    }
}
