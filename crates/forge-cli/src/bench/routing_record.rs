//! `forge bench report --record`: store each scored Forge run per model so the mesh can rank on
//! measured cost per solve (docs/features/mesh-routing.md, "Measured cost per solve").
//!
//! Only runs that measure a model inside Forge's own harness qualify: the Forge agent, pinned to
//! one `provider::model` (a mesh-routed run spreads over several models), scored by its own eval,
//! with every eligible attempt's cost captured. Anything else is skipped with the reason.

use std::path::Path;

use forge_store::BenchSolveRun;

use super::scoring::{run_pass1, Run};

/// Why a run was not recorded, printed so a skipped run is never silent.
#[derive(Debug, PartialEq)]
pub(super) struct Skipped {
    pub source: String,
    pub reason: String,
}

/// Record every qualifying run in the store and say what was skipped. `--shared-eval` credits
/// runs with each other's solves, so it can never feed routing.
pub(super) fn record<'a>(
    runs: impl Iterator<Item = (&'a Path, &'a Run)>,
    shared_eval: bool,
) -> anyhow::Result<()> {
    if shared_eval {
        anyhow::bail!("--record needs each run scored by its own eval; --shared-eval credits runs with each other's solves");
    }
    let (recorded, skipped) = solve_runs(runs);
    for s in &skipped {
        eprintln!("not recorded: {} ({})", s.source, s.reason);
    }
    if !recorded.is_empty() {
        crate::cli::commands::replay::open_store()?.record_bench_solves(&recorded)?;
    }
    eprintln!(
        "recorded {} run(s) for routing; a model's cost per solve moves its rank once it has {} scored attempts",
        recorded.len(),
        forge_mesh::MIN_SOLVE_ATTEMPTS
    );
    Ok(())
}

/// The routing record for each run, or why it has none.
pub(super) fn solve_runs<'a>(
    runs: impl Iterator<Item = (&'a Path, &'a Run)>,
) -> (Vec<BenchSolveRun>, Vec<Skipped>) {
    let mut recorded = Vec::new();
    let mut skipped = Vec::new();
    for (path, run) in runs {
        let source = std::fs::canonicalize(path)
            .unwrap_or_else(|_| path.to_path_buf())
            .display()
            .to_string();
        match solve_run(source.clone(), run) {
            Ok(r) => recorded.push(r),
            Err(reason) => skipped.push(Skipped { source, reason }),
        }
    }
    (recorded, skipped)
}

fn solve_run(source: String, run: &Run) -> Result<BenchSolveRun, String> {
    let first = run.rows.first().ok_or("no rows")?;
    if let Some(other) = run.rows.iter().find(|r| r.agent != "forge") {
        return Err(format!(
            "agent `{}` is not Forge; the signal ranks models inside Forge's harness",
            other.agent
        ));
    }
    let resolved = run
        .resolved
        .as_ref()
        .ok_or("not scored (pair it with its eval report)")?;
    let model = first
        .runtime
        .as_ref()
        .and_then(|rt| rt.requested_model.as_deref())
        .map(|m| forge_provider::normalize_model_id(m).into_owned())
        .filter(|m| m.contains("::"))
        .ok_or("no pinned provider::model (a mesh-routed run measures several models)")?;
    let eligible: Vec<_> = run
        .rows
        .iter()
        .filter(|r| !r.excluded_from_pass1())
        .collect();
    if eligible.is_empty() {
        return Err("no pass@1-eligible attempts".into());
    }
    let mut cost = 0.0;
    for row in &eligible {
        if !row.metrics_complete {
            return Err(format!("cost capture incomplete for {}", row.instance_id));
        }
        cost += match row.notional_cost_usd {
            Some(notional) => notional,
            None if row.cost_usd > 0.0 => row.cost_usd,
            None => {
                return Err(format!(
                    "no price for {}'s zero-cost calls",
                    row.instance_id
                ))
            }
        };
    }
    let (_, solved, _) = run_pass1(&run.rows, resolved);
    Ok(BenchSolveRun {
        source,
        model,
        attempts: eligible.len() as u32,
        solved: solved.len() as u32,
        cost_usd: cost,
    })
}

#[cfg(test)]
mod tests {
    use super::super::{BenchmarkRuntimeMetadata, InstanceMetric};
    use super::*;
    use std::collections::BTreeSet;

    fn row(id: &str, model: Option<&str>) -> InstanceMetric {
        InstanceMetric {
            instance_id: id.into(),
            agent: "forge".into(),
            cost_usd: 0.0,
            notional_cost_usd: Some(0.25),
            metrics_complete: true,
            runtime: Some(BenchmarkRuntimeMetadata {
                requested_model: model.map(str::to_string),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn scored(rows: Vec<InstanceMetric>, resolved: &[&str]) -> Run {
        Run {
            rows,
            resolved: Some(
                resolved
                    .iter()
                    .map(|s| s.to_string())
                    .collect::<BTreeSet<_>>(),
            ),
        }
    }

    #[test]
    fn a_pinned_scored_forge_run_records_attempts_solves_and_notional_cost() {
        let mut toolless = row("c", Some("openai::gpt-5.5"));
        toolless.tools_unavailable = true;
        let run = scored(
            vec![
                row("a", Some("openai::gpt-5.5")),
                row("b", Some("openai::gpt-5.5")),
                toolless,
            ],
            &["a", "c"],
        );
        let (rec, skipped) = solve_runs(std::iter::once((Path::new("x.metrics.jsonl"), &run)));
        assert!(skipped.is_empty(), "{skipped:?}");
        assert_eq!(rec.len(), 1);
        assert_eq!(rec[0].model, "openai::gpt-5.5");
        assert_eq!(
            rec[0].attempts, 2,
            "toolless attempts leave the denominator"
        );
        assert_eq!(rec[0].solved, 1, "a toolless solve is not credited");
        assert!((rec[0].cost_usd - 0.5).abs() < 1e-9);
    }

    #[test]
    fn unqualified_runs_are_skipped_with_a_reason() {
        let mut codex = row("a", Some("openai::gpt-5.5"));
        codex.agent = "codex".into();
        let mut unpriced = row("a", Some("openai::gpt-5.5"));
        unpriced.notional_cost_usd = None;
        let cases = [
            (scored(vec![codex], &[]), "not Forge"),
            (
                Run {
                    rows: vec![row("a", Some("openai::gpt-5.5"))],
                    resolved: None,
                },
                "not scored",
            ),
            (scored(vec![row("a", None)], &[]), "mesh-routed"),
            (scored(vec![unpriced], &[]), "no price"),
        ];
        for (run, why) in &cases {
            let (rec, skipped) = solve_runs(std::iter::once((Path::new("m.jsonl"), run)));
            assert!(rec.is_empty());
            assert!(skipped[0].reason.contains(why), "{why}: {:?}", skipped[0]);
        }
    }
}
