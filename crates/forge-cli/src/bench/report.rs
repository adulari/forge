//! Reading benchmark results back out: pass@k across seeds and the agent comparison table.
//!
//! The prediction harness writes two artifacts per run — `predictions.jsonl` for the official
//! scorer and a `*.metrics.jsonl` sidecar of what each instance cost. This module is the read
//! side: it joins each sidecar with ITS OWN scorer report and prints the numbers Forge's
//! efficiency claim rests on. The arithmetic lives in `scoring` and is pure, so the honesty rules
//! (no cross-run credit, no tokens-per-success from a partial capture, no Pareto claim on a
//! handful of instances) are locked down by tests.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::scoring::{self, GroupSummary, Run, MIN_PARETO_INSTANCES};
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

/// What `forge bench report` was given on the command line.
pub(crate) struct ReportInputs {
    /// `--run <metrics.jsonl>=<eval.json>` pairs, as typed.
    pub runs: Vec<String>,
    pub metrics: Vec<PathBuf>,
    pub evals: Vec<PathBuf>,
    /// Apply the union of every `--eval` to every `--metrics` file (one combined report).
    pub shared_eval: bool,
    /// Store each qualifying run for the mesh's cost-per-solve signal.
    pub record: bool,
}

/// One metrics sidecar and the evaluation report that scores it.
#[derive(Debug, PartialEq)]
struct RunFiles {
    metrics: PathBuf,
    eval: Option<PathBuf>,
}

/// Which eval scores which metrics file. Explicit `--run` pairs come first; bare `--metrics` are
/// paired with `--eval` by position when the counts match, left unscored when no `--eval` is
/// given, and otherwise refused, because guessing would credit one run with another's solves.
fn pair_runs(inputs: &ReportInputs) -> Result<Vec<RunFiles>> {
    let mut runs = Vec::new();
    for spec in &inputs.runs {
        let (metrics, eval) = spec
            .split_once('=')
            .filter(|(m, e)| !m.is_empty() && !e.is_empty())
            .with_context(|| format!("--run expects <metrics.jsonl>=<eval.json>, got `{spec}`"))?;
        runs.push(RunFiles {
            metrics: PathBuf::from(metrics),
            eval: Some(PathBuf::from(eval)),
        });
    }
    if inputs.shared_eval {
        if !inputs.runs.is_empty() {
            anyhow::bail!("--shared-eval applies one combined report to every --metrics file; it cannot be mixed with --run pairs");
        }
        if inputs.evals.is_empty() {
            anyhow::bail!("--shared-eval needs at least one --eval report");
        }
        runs.extend(inputs.metrics.iter().map(|m| RunFiles {
            metrics: m.clone(),
            eval: None,
        }));
    } else if inputs.evals.is_empty() {
        runs.extend(inputs.metrics.iter().map(|m| RunFiles {
            metrics: m.clone(),
            eval: None,
        }));
    } else if inputs.evals.len() == inputs.metrics.len() {
        runs.extend(
            inputs
                .metrics
                .iter()
                .zip(&inputs.evals)
                .map(|(m, e)| RunFiles {
                    metrics: m.clone(),
                    eval: Some(e.clone()),
                }),
        );
    } else {
        anyhow::bail!(
            "got {} --metrics but {} --eval, so which report scores which run is ambiguous. Pass each \
             pair as --run <metrics.jsonl>=<eval.json>, give one --eval per --metrics in the same \
             order, or add --shared-eval to apply the union of every report to every run (the old \
             behaviour, which credits a run with solves from the others)",
            inputs.metrics.len(),
            inputs.evals.len()
        );
    }
    if runs.is_empty() {
        anyhow::bail!(
            "report needs at least one --metrics <file.metrics.jsonl> or --run <metrics>=<eval>"
        );
    }
    Ok(runs)
}

/// The headline comparison: per agent + model, mean pass@1 ± stdev across attempts, pass@k, cost
/// per attempt and per solved task (real and notional), tokens per attempt, cache hit rate, then a
/// Pareto block when the instance count can carry one. `metrics` files come from `bench swe`;
/// evals are the official `run_evaluation` `*.json` reports.
pub(crate) fn report(inputs: &ReportInputs) -> Result<()> {
    use std::collections::BTreeSet;
    let files = pair_runs(inputs)?;
    let shared: Option<BTreeSet<String>> = if inputs.shared_eval {
        let mut union = BTreeSet::new();
        for e in &inputs.evals {
            union.extend(resolved_ids_from_report(e)?);
        }
        Some(union)
    } else {
        None
    };
    let mut runs = Vec::with_capacity(files.len());
    for f in &files {
        let rows = load_metrics(&f.metrics)?;
        let resolved = match &f.eval {
            Some(e) => Some(resolved_ids_from_report(e)?.into_iter().collect()),
            None => shared.clone(),
        };
        runs.push(Run { rows, resolved });
    }
    if inputs.record {
        let paths = files.iter().map(|f| f.metrics.as_path());
        super::routing_record::record(paths.zip(&runs), inputs.shared_eval)?;
    }
    let groups = scoring::summarize_groups(&runs);
    if groups.is_empty() {
        anyhow::bail!("no metrics rows found in the given files");
    }
    print!("{}", render_table(&groups));
    if let Some(block) = render_pareto(&groups) {
        print!("{block}");
    }
    if inputs.shared_eval {
        eprintln!(
            "\nnote: --shared-eval unions every report, so a run is credited with solves from the \
             others; per-attempt pass@1 and cross-agent numbers are overstated."
        );
    }
    if groups.iter().all(|g| g.pass1.is_empty()) {
        eprintln!(
            "\nnote: no eval reports given → pass@1, $/solved and tok/success omitted. Score predictions\nwith the official evaluator, then re-run with --run <metrics>=<report.json>."
        );
    }
    Ok(())
}

fn money(v: Option<f64>) -> String {
    v.map_or_else(|| "n/a".to_string(), |v| format!("${v:.4}"))
}

fn percent(v: Option<f64>) -> String {
    v.map_or_else(|| "n/a".to_string(), |v| format!("{:.1}%", v * 100.0))
}

fn pass1_cell(g: &GroupSummary) -> String {
    match (g.mean_pass1(), g.pass1_stdev()) {
        (Some(m), Some(sd)) => format!("{:.1}% ±{:.1}", m * 100.0, sd * 100.0),
        (Some(m), None) => format!("{:.1}%", m * 100.0),
        _ => "n/a".to_string(),
    }
}

/// The comparison table plus a legend. Pure so its labelling (notional is never shown as spend)
/// is testable.
fn render_table(groups: &[GroupSummary]) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    let _ = writeln!(
        out,
        "{:<12} {:<24} {:>3} {:>4} {:>14} {:>7} {:>9} {:>6} {:>9} {:>10} {:>9} {:>10} {:>11} {:>7}",
        "agent",
        "model",
        "k",
        "n",
        "pass@1",
        "pass@k",
        "tok/att",
        "cache",
        "$/att",
        "$/solved",
        "ntl$/att",
        "ntl$/solv",
        "tok/success",
        "mean s"
    );
    for g in groups {
        let _ = writeln!(
            out,
            "{:<12} {:<24} {:>3} {:>4} {:>14} {:>7} {:>9} {:>6} {:>9} {:>10} {:>9} {:>10} {:>11} {:>7.1}",
            g.agent,
            g.model,
            g.attempts,
            g.instances,
            pass1_cell(g),
            percent(g.pass_k),
            g.tokens_per_attempt()
                .map_or_else(|| "n/a".to_string(), |t| t.to_string()),
            g.cache_hit()
                .map_or_else(|| "n/a".to_string(), |c| format!("{:.0}%", c * 100.0)),
            money(g.cost_per_attempt()),
            money(g.cost_per_solved()),
            money(g.notional_per_attempt()),
            money(g.notional_per_solved()),
            g.tokens_per_success_cell(),
            g.total_wall / g.rows.max(1) as f64,
        );
    }
    for g in groups {
        let mut flags = Vec::new();
        if g.excluded > 0 {
            flags.push(format!(
                "{} tools-unavailable excluded from pass@1",
                g.excluded
            ));
        }
        if g.refused > 0 {
            flags.push(format!("{} refused (scored 0)", g.refused));
        }
        if g.timed_out > 0 {
            flags.push(format!("{} timed out", g.timed_out));
        }
        if g.complete < g.rows {
            flags.push(format!(
                "{} of {} rows without token capture (left out of cost/tokens)",
                g.rows - g.complete,
                g.rows
            ));
        }
        if !flags.is_empty() {
            let _ = writeln!(out, "  {} {}: {}", g.agent, g.model, flags.join("; "));
        }
    }
    let _ = writeln!(
        out,
        "\npass@1 = mean over attempts (± sample stdev); $/solved = $/att ÷ mean pass@1.\n\
         ntl = NOTIONAL: zero-cost (subscription/free) calls priced at list rates. A comparison \
         axis, not spend."
    );
    out
}

/// The Pareto block over (cost per attempt, mean pass@1). Uses notional cost when every group has
/// one, so subscription arms are comparable, else real cost. `None` (with the reason printed in
/// its place) when there is nothing honest to show.
fn render_pareto(groups: &[GroupSummary]) -> Option<String> {
    use std::fmt::Write;
    if groups.len() < 2 || groups.iter().any(|g| g.mean_pass1().is_none()) {
        return None;
    }
    let mut out = String::from("\nPareto (cost per attempt vs mean pass@1)\n");
    // Toolless attempts leave the denominator, so the floor applies to what is left.
    let smallest = groups.iter().map(|g| g.min_eligible).min().unwrap_or(0);
    if smallest < MIN_PARETO_INSTANCES {
        let _ = writeln!(
            out,
            "  refused: {smallest} eligible instance(s) in the smallest attempt; need at least {MIN_PARETO_INSTANCES} before claiming a Pareto frontier"
        );
        return Some(out);
    }
    let notional = groups.iter().all(|g| g.notional_per_attempt().is_some());
    let cost = |g: &GroupSummary| {
        if notional {
            g.notional_per_attempt()
        } else {
            g.cost_per_attempt()
        }
    };
    // A $0 recorded cost is a subscription or free route, not a free solve: without a notional
    // figure for every group there is no common cost axis.
    if groups
        .iter()
        .any(|g| cost(g).is_none_or(|c| !notional && c <= 0.0))
    {
        let _ = writeln!(
            out,
            "  refused: a group has no usable cost (no capture, or $0 recorded spend without a notional price)"
        );
        return Some(out);
    }
    let points: Vec<(f64, f64)> = groups
        .iter()
        .map(|g| (cost(g).unwrap_or(0.0), g.mean_pass1().unwrap_or(0.0)))
        .collect();
    let dominators = scoring::pareto_dominators(&points);
    let mut order: Vec<usize> = (0..groups.len()).collect();
    order.sort_by(|&a, &b| points[a].0.total_cmp(&points[b].0));
    let basis = if notional { "notional" } else { "real" };
    for i in order {
        let g = &groups[i];
        let verdict = match dominators[i] {
            None => "frontier".to_string(),
            Some(j) => format!("dominated by {} {}", groups[j].agent, groups[j].model),
        };
        let _ = writeln!(
            out,
            "  {:<12} {:<24} ${:.4} ({basis})  {:.1}%  {verdict}",
            g.agent,
            g.model,
            points[i].0,
            points[i].1 * 100.0
        );
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn inputs(runs: &[&str], metrics: &[&str], evals: &[&str], shared: bool) -> ReportInputs {
        ReportInputs {
            runs: runs.iter().map(|s| s.to_string()).collect(),
            metrics: metrics.iter().map(PathBuf::from).collect(),
            evals: evals.iter().map(PathBuf::from).collect(),
            shared_eval: shared,
            record: false,
        }
    }

    #[test]
    fn explicit_runs_and_positional_pairs_are_kept_together() {
        let got = pair_runs(&inputs(
            &["f.jsonl=f.json"],
            &["c.jsonl"],
            &["c.json"],
            false,
        ))
        .unwrap();
        assert_eq!(
            got,
            vec![
                RunFiles {
                    metrics: "f.jsonl".into(),
                    eval: Some("f.json".into())
                },
                RunFiles {
                    metrics: "c.jsonl".into(),
                    eval: Some("c.json".into())
                },
            ]
        );
    }

    #[test]
    fn mismatched_counts_fail_instead_of_guessing() {
        let err = pair_runs(&inputs(&[], &["a.jsonl", "b.jsonl"], &["one.json"], false))
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("--run") && err.contains("--shared-eval"),
            "{err}"
        );
        assert!(pair_runs(&inputs(&["no-equals-sign"], &[], &[], false)).is_err());
        assert!(pair_runs(&inputs(&["a=b"], &[], &["c.json"], true)).is_err());
        assert!(pair_runs(&inputs(&[], &["a.jsonl"], &[], true)).is_err());
    }

    #[test]
    fn no_evals_leaves_runs_unscored_and_shared_eval_keeps_the_old_union() {
        let unscored = pair_runs(&inputs(&[], &["a.jsonl"], &[], false)).unwrap();
        assert_eq!(unscored[0].eval, None);
        let shared = pair_runs(&inputs(&[], &["a.jsonl", "b.jsonl"], &["all.json"], true)).unwrap();
        assert_eq!(shared.len(), 2);
    }

    fn metric(id: &str, agent: &str) -> InstanceMetric {
        InstanceMetric {
            instance_id: id.into(),
            agent: agent.into(),
            input_tokens: 100,
            output_tokens: 10,
            total_tokens: 110,
            cost_usd: 0.01,
            wall_secs: 1.0,
            patched: true,
            metrics_complete: true,
            attempt: 1,
            ..Default::default()
        }
    }

    /// The bug this module was rewritten for: `x-2` is solved only by Forge, yet the old union of
    /// both eval reports credited Codex with it too.
    #[test]
    fn a_solve_by_one_agent_is_never_credited_to_the_other() {
        let dir =
            std::env::temp_dir().join(format!("forge-bench-report-{}", forge_types::new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let write_metrics = |name: &str, agent: &str| {
            let path = dir.join(name);
            let rows = vec![metric("x-1", agent), metric("x-2", agent)];
            std::fs::write(&path, super::super::metrics_to_jsonl(&rows)).unwrap();
            path
        };
        let write_eval = |name: &str, resolved: &[&str]| {
            let path = dir.join(name);
            std::fs::write(
                &path,
                serde_json::json!({"submitted_instances": 2, "resolved_ids": resolved}).to_string(),
            )
            .unwrap();
            path
        };
        let forge_m = write_metrics("forge.metrics.jsonl", "forge");
        let codex_m = write_metrics("codex.metrics.jsonl", "codex");
        let forge_e = write_eval("forge.json", &["x-1", "x-2"]);
        let codex_e = write_eval("codex.json", &["x-1"]);

        let load = |m: &PathBuf, e: &PathBuf| Run {
            rows: load_metrics(m).unwrap(),
            resolved: Some(resolved_ids_from_report(e).unwrap().into_iter().collect()),
        };
        let groups =
            scoring::summarize_groups(&[load(&forge_m, &forge_e), load(&codex_m, &codex_e)]);
        assert_eq!(groups[0].agent, "forge");
        assert_eq!(groups[0].mean_pass1(), Some(1.0));
        assert_eq!(groups[1].agent, "codex");
        assert_eq!(
            groups[1].mean_pass1(),
            Some(0.5),
            "codex solved only x-1; x-2 was forge's"
        );

        // The legacy union, still reachable via --shared-eval, reproduces the old inflation.
        let union: BTreeSet<String> = ["x-1", "x-2"].iter().map(|s| s.to_string()).collect();
        let legacy = scoring::summarize_groups(&[Run {
            rows: load_metrics(&codex_m).unwrap(),
            resolved: Some(union),
        }]);
        assert_eq!(legacy[0].mean_pass1(), Some(1.0));

        // The CLI entry point runs end to end on the same fixtures.
        let spec = |m: &PathBuf, e: &PathBuf| format!("{}={}", m.display(), e.display());
        report(&ReportInputs {
            runs: vec![spec(&forge_m, &forge_e), spec(&codex_m, &codex_e)],
            metrics: vec![],
            evals: vec![],
            shared_eval: false,
            record: false,
        })
        .unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn group(agent: &str, instances: usize, pass1: f64, cost: f64) -> GroupSummary {
        GroupSummary {
            agent: agent.into(),
            model: "m".into(),
            attempts: 1,
            instances,
            min_eligible: instances,
            rows: instances,
            patched: instances,
            excluded: 0,
            refused: 0,
            timed_out: 0,
            solved: (pass1 * instances as f64) as usize,
            pass1: vec![pass1],
            pass_k: Some(pass1),
            complete: instances,
            total_cost: cost * instances as f64,
            total_notional: None,
            total_tokens: 0,
            cached_input: 0,
            cache_reported_input: 0,
            total_wall: 0.0,
        }
    }

    #[test]
    fn pareto_is_refused_below_the_minimum_instance_count() {
        let small =
            render_pareto(&[group("forge", 5, 0.4, 0.1), group("codex", 5, 0.6, 0.2)]).unwrap();
        assert!(small.contains("refused"), "{small}");
        assert!(
            !small.contains("dominated") && !small.contains("(real)"),
            "no groups are placed: {small}"
        );
    }

    #[test]
    fn pareto_floor_counts_eligible_rows_not_rows_written() {
        let mut toolless = group("forge", 20, 0.6, 0.10);
        toolless.excluded = 5;
        toolless.min_eligible = 15;
        let block = render_pareto(&[toolless, group("codex", 20, 0.5, 0.20)]).unwrap();
        assert!(block.contains("refused: 15 eligible"), "{block}");
    }

    #[test]
    fn pareto_marks_the_dominated_group_on_real_cost_when_notional_is_missing() {
        let block =
            render_pareto(&[group("forge", 20, 0.6, 0.10), group("codex", 20, 0.5, 0.20)]).unwrap();
        assert!(block.contains("(real)"), "{block}");
        assert!(block.contains("dominated by forge m"), "{block}");
        assert!(block.contains("frontier"), "{block}");
    }

    #[test]
    fn pareto_is_refused_when_a_zero_cost_group_has_no_notional_price() {
        let block =
            render_pareto(&[group("forge", 20, 0.6, 0.10), group("codex", 20, 0.5, 0.0)]).unwrap();
        assert!(block.contains("refused"), "{block}");
        let mut free = group("codex", 20, 0.5, 0.0);
        free.total_notional = Some(1.0);
        let mut billed = group("forge", 20, 0.6, 0.10);
        billed.total_notional = Some(2.0);
        let block = render_pareto(&[billed, free]).unwrap();
        assert!(block.contains("(notional)"), "{block}");
        assert!(block.contains("frontier"), "{block}");
    }

    #[test]
    fn the_table_labels_notional_cost_as_not_spend() {
        let mut g = group("forge", 20, 0.5, 0.0);
        g.total_notional = Some(2.0);
        let table = render_table(&[g]);
        assert!(table.contains("ntl$/att"));
        assert!(table.contains("NOTIONAL"));
        assert!(table.contains("not spend"));
        assert!(table.contains("$0.1000"), "notional per attempt: {table}");
    }
}
