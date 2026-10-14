//! The arithmetic behind `forge bench report`: per-run pass@1, per-group mean ± stdev, pass@k,
//! cost per attempt and per solved task, and the Pareto frontier. Pure, so every number the
//! report prints is pinned by a unit test.
//!
//! A *run* is one metrics sidecar (one agent, one seed) joined with its OWN eval report. A
//! *group* is every run of the same agent on the same resolved model, i.e. the k attempts.

use std::collections::{BTreeMap, BTreeSet};

use super::InstanceMetric;

/// Below this many instances per attempt the Pareto block is refused: cost per solved task on a
/// handful of instances is noise.
pub(super) const MIN_PARETO_INSTANCES: usize = 20;

/// One metrics sidecar plus the resolved ids from its own evaluation (`None` = not scored yet).
pub(super) struct Run {
    pub rows: Vec<InstanceMetric>,
    pub resolved: Option<BTreeSet<String>>,
}

/// One agent + model's numbers across its attempts.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct GroupSummary {
    pub agent: String,
    pub model: String,
    pub attempts: usize,
    /// Largest per-attempt row count (the instance set size).
    pub instances: usize,
    /// Fewest pass@1-eligible rows in any one attempt: what a Pareto claim actually rests on once
    /// toolless attempts are dropped.
    pub min_eligible: usize,
    pub rows: usize,
    pub patched: usize,
    /// Rows left out of pass@1 because Forge's tools never started.
    pub excluded: usize,
    pub refused: usize,
    pub timed_out: usize,
    /// Solved rows over every attempt (resolved by the evaluator and not scored 0).
    pub solved: usize,
    /// pass@1 of each attempt, in run order. Empty unless every run of the group was scored.
    pub pass1: Vec<f64>,
    pub pass_k: Option<f64>,
    /// Rows whose token/cost capture was complete; the cost and token sums cover only these.
    pub complete: usize,
    pub total_cost: f64,
    /// Sum of `notional_cost_usd` over complete rows, `None` if any complete row lacks one.
    pub total_notional: Option<f64>,
    pub total_tokens: u64,
    /// Cached and total input over the complete rows that reported a cached count.
    pub cached_input: u64,
    pub cache_reported_input: u64,
    pub total_wall: f64,
}

impl GroupSummary {
    pub fn mean_pass1(&self) -> Option<f64> {
        mean(&self.pass1)
    }

    pub fn pass1_stdev(&self) -> Option<f64> {
        sample_stdev(&self.pass1)
    }

    pub fn cost_per_attempt(&self) -> Option<f64> {
        (self.complete > 0).then(|| self.total_cost / self.complete as f64)
    }

    pub fn notional_per_attempt(&self) -> Option<f64> {
        let total = self.total_notional?;
        (self.complete > 0).then(|| total / self.complete as f64)
    }

    /// Cost per solved task as the Coding Agent Index defines it: mean cost per attempt divided
    /// by mean pass@1. `None` when either is unknown or nothing was solved.
    pub fn cost_per_solved(&self) -> Option<f64> {
        per_solved(self.cost_per_attempt(), self.mean_pass1())
    }

    pub fn notional_per_solved(&self) -> Option<f64> {
        per_solved(self.notional_per_attempt(), self.mean_pass1())
    }

    pub fn tokens_per_attempt(&self) -> Option<u64> {
        (self.complete > 0).then(|| self.total_tokens / self.complete as u64)
    }

    pub fn cache_hit(&self) -> Option<f64> {
        (self.cache_reported_input > 0)
            .then(|| self.cached_input as f64 / self.cache_reported_input as f64)
    }

    /// Tokens per solved task. Only honest when every row's capture was complete: a partial
    /// capture would understate it and flatter whichever agent lost data.
    pub fn tokens_per_success_cell(&self) -> String {
        if self.complete < self.rows {
            "incomplete".to_string()
        } else if self.pass1.is_empty() || self.solved == 0 {
            "n/a".to_string()
        } else {
            format!("{}", self.total_tokens / self.solved as u64)
        }
    }
}

fn per_solved(per_attempt: Option<f64>, pass1: Option<f64>) -> Option<f64> {
    let pass1 = pass1.filter(|p| *p > 0.0)?;
    Some(per_attempt? / pass1)
}

pub(super) fn mean(values: &[f64]) -> Option<f64> {
    (!values.is_empty()).then(|| values.iter().sum::<f64>() / values.len() as f64)
}

/// Sample standard deviation (n − 1). `None` for fewer than two values: one attempt has no spread.
pub(super) fn sample_stdev(values: &[f64]) -> Option<f64> {
    if values.len() < 2 {
        return None;
    }
    let m = mean(values)?;
    let var = values.iter().map(|v| (v - m).powi(2)).sum::<f64>() / (values.len() - 1) as f64;
    Some(var.sqrt())
}

/// pass@1 of one run: solved / eligible, where eligible rows exclude toolless attempts and solved
/// rows are resolved by THIS run's evaluation and not scored 0 (refusals, integrity failures).
/// Returns `(pass1, solved ids, eligible ids)`; `pass1` is `None` with no eligible rows.
pub(super) fn run_pass1(
    rows: &[InstanceMetric],
    resolved: &BTreeSet<String>,
) -> (Option<f64>, BTreeSet<String>, BTreeSet<String>) {
    let eligible: BTreeSet<String> = rows
        .iter()
        .filter(|r| !r.excluded_from_pass1())
        .map(|r| r.instance_id.clone())
        .collect();
    let solved: BTreeSet<String> = rows
        .iter()
        .filter(|r| !r.excluded_from_pass1() && !r.scores_zero())
        .filter(|r| resolved.contains(&r.instance_id))
        .map(|r| r.instance_id.clone())
        .collect();
    let pass1 = (!eligible.is_empty()).then(|| solved.len() as f64 / eligible.len() as f64);
    (pass1, solved, eligible)
}

/// The group a run belongs to: its agent plus the model that actually answered
/// (`runtime.resolved_model`, else the requested one).
fn group_key(rows: &[InstanceMetric]) -> (String, String) {
    let first = &rows[0];
    let model = first
        .runtime
        .as_ref()
        .and_then(|rt| rt.resolved_model.clone().or(rt.requested_model.clone()))
        .unwrap_or_else(|| "-".to_string());
    (first.agent.clone(), model)
}

/// Fold runs into per-group summaries, in first-seen order. Runs with no rows are skipped.
pub(super) fn summarize_groups(runs: &[Run]) -> Vec<GroupSummary> {
    let mut order: Vec<(String, String)> = Vec::new();
    let mut by_key: BTreeMap<(String, String), Vec<&Run>> = BTreeMap::new();
    for run in runs.iter().filter(|r| !r.rows.is_empty()) {
        let key = group_key(&run.rows);
        if !by_key.contains_key(&key) {
            order.push(key.clone());
        }
        by_key.entry(key).or_default().push(run);
    }
    order
        .into_iter()
        .map(|key| {
            let runs = &by_key[&key];
            summarize_group(key, runs)
        })
        .collect()
}

fn summarize_group((agent, model): (String, String), runs: &[&Run]) -> GroupSummary {
    let mut s = GroupSummary {
        agent,
        model,
        attempts: runs.len(),
        instances: 0,
        min_eligible: usize::MAX,
        rows: 0,
        patched: 0,
        excluded: 0,
        refused: 0,
        timed_out: 0,
        solved: 0,
        pass1: Vec::new(),
        pass_k: None,
        complete: 0,
        total_cost: 0.0,
        total_notional: Some(0.0),
        total_tokens: 0,
        cached_input: 0,
        cache_reported_input: 0,
        total_wall: 0.0,
    };
    let all_scored = runs.iter().all(|r| r.resolved.is_some());
    let mut solved_any: BTreeSet<String> = BTreeSet::new();
    let mut eligible_any: BTreeSet<String> = BTreeSet::new();
    for run in runs {
        s.instances = s.instances.max(run.rows.len());
        let eligible_rows = run.rows.iter().filter(|r| !r.excluded_from_pass1()).count();
        s.min_eligible = s.min_eligible.min(eligible_rows);
        for r in &run.rows {
            s.rows += 1;
            s.patched += usize::from(r.patched);
            s.excluded += usize::from(r.excluded_from_pass1());
            s.refused += usize::from(r.outcome == super::AttemptOutcome::Refused);
            s.timed_out += usize::from(r.timed_out);
            s.total_wall += r.wall_secs;
            if r.metrics_complete {
                s.complete += 1;
                s.total_cost += r.cost_usd;
                s.total_tokens += r.total_tokens;
                s.total_notional = match (s.total_notional, r.notional_cost_usd) {
                    (Some(total), Some(part)) => Some(total + part),
                    _ => None,
                };
                if let Some(cached) = r.cached_input_tokens {
                    s.cached_input += cached.min(r.input_tokens);
                    s.cache_reported_input += r.input_tokens;
                }
            }
        }
        if all_scored {
            let resolved = run.resolved.as_ref().expect("all_scored");
            let (pass1, solved, eligible) = run_pass1(&run.rows, resolved);
            s.solved += solved.len();
            if let Some(p) = pass1 {
                s.pass1.push(p);
            }
            solved_any.extend(solved);
            eligible_any.extend(eligible);
        }
    }
    if s.complete == 0 {
        s.total_notional = None;
    }
    if s.min_eligible == usize::MAX {
        s.min_eligible = 0;
    }
    if all_scored && !eligible_any.is_empty() {
        s.pass_k = Some(solved_any.len() as f64 / eligible_any.len() as f64);
    }
    s
}

/// For each point `(cost, score)`, the index of a point that dominates it: cost lower or equal
/// AND score higher or equal, strictly better on at least one. `None` = on the frontier.
pub(super) fn pareto_dominators(points: &[(f64, f64)]) -> Vec<Option<usize>> {
    points
        .iter()
        .enumerate()
        .map(|(i, &(cost, score))| {
            points.iter().enumerate().position(|(j, &(c, sc))| {
                j != i && c <= cost && sc >= score && (c < cost || sc > score)
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::{AttemptOutcome, BenchmarkRuntimeMetadata};
    use super::*;

    fn row(id: &str, agent: &str) -> InstanceMetric {
        InstanceMetric {
            instance_id: id.into(),
            agent: agent.into(),
            input_tokens: 1_000,
            output_tokens: 100,
            total_tokens: 1_100,
            cost_usd: 0.10,
            wall_secs: 10.0,
            patched: true,
            metrics_complete: true,
            ..Default::default()
        }
    }

    fn ids(list: &[&str]) -> Option<BTreeSet<String>> {
        Some(list.iter().map(|s| s.to_string()).collect())
    }

    #[test]
    fn mean_and_sample_stdev() {
        assert_eq!(mean(&[]), None);
        assert_eq!(sample_stdev(&[0.5]), None, "one attempt has no spread");
        let sd = sample_stdev(&[0.4, 0.6]).unwrap();
        assert!((sd - 0.141_421_356).abs() < 1e-6);
    }

    #[test]
    fn each_seed_gets_its_own_pass1_and_pass_k_is_the_union() {
        // Seed 1 solves a; seed 2 solves b. Unioning the evals (the old bug) credited each seed
        // with both solves, i.e. pass@1 = pass@2 = 100%.
        let runs = vec![
            Run {
                rows: vec![row("a", "forge"), row("b", "forge")],
                resolved: ids(&["a"]),
            },
            Run {
                rows: vec![row("a", "forge"), row("b", "forge")],
                resolved: ids(&["b"]),
            },
        ];
        let g = &summarize_groups(&runs)[0];
        assert_eq!(g.attempts, 2);
        assert_eq!(g.pass1, vec![0.5, 0.5]);
        assert_eq!(g.mean_pass1(), Some(0.5));
        assert_eq!(g.pass_k, Some(1.0));
        assert_eq!(g.solved, 2);
    }

    #[test]
    fn tools_unavailable_rows_leave_the_denominator_and_refusals_score_zero() {
        let mut toolless = row("b", "forge");
        toolless.tools_unavailable = true;
        let mut refused = row("c", "forge");
        refused.outcome = AttemptOutcome::Refused;
        refused.patched = false;
        let runs = vec![Run {
            rows: vec![row("a", "forge"), toolless, refused, row("d", "forge")],
            // The evaluator "resolved" c (say an earlier patch) — a refusal still scores 0.
            resolved: ids(&["a", "b", "c"]),
        }];
        let g = &summarize_groups(&runs)[0];
        assert_eq!(g.excluded, 1);
        assert_eq!(g.refused, 1);
        assert_eq!((g.instances, g.min_eligible), (4, 3));
        // eligible = a, c, d; solved = a  →  1/3
        assert!((g.mean_pass1().unwrap() - 1.0 / 3.0).abs() < 1e-9);
    }

    #[test]
    fn cost_per_solved_is_cost_per_attempt_over_mean_pass1() {
        let runs = vec![
            Run {
                rows: vec![row("a", "forge"), row("b", "forge")],
                resolved: ids(&["a"]),
            },
            Run {
                rows: vec![row("a", "forge"), row("b", "forge")],
                resolved: ids(&[]),
            },
        ];
        let g = &summarize_groups(&runs)[0];
        assert!((g.cost_per_attempt().unwrap() - 0.10).abs() < 1e-9);
        // mean pass@1 = (0.5 + 0) / 2 = 0.25  →  $0.10 / 0.25 = $0.40
        assert!((g.cost_per_solved().unwrap() - 0.40).abs() < 1e-9);
        assert_eq!(g.tokens_per_attempt(), Some(1_100));
        assert_eq!(
            g.notional_per_attempt(),
            None,
            "rows carry no notional cost"
        );
    }

    #[test]
    fn nothing_solved_or_unscored_yields_no_cost_per_solved() {
        let unscored = vec![Run {
            rows: vec![row("a", "forge")],
            resolved: None,
        }];
        let g = &summarize_groups(&unscored)[0];
        assert!(g.pass1.is_empty());
        assert_eq!(g.pass_k, None);
        assert_eq!(g.cost_per_solved(), None);
        assert_eq!(g.tokens_per_success_cell(), "n/a");
        let unsolved = vec![Run {
            rows: vec![row("a", "forge")],
            resolved: ids(&[]),
        }];
        assert_eq!(summarize_groups(&unsolved)[0].cost_per_solved(), None);
    }

    #[test]
    fn notional_cost_and_cache_hit_cover_complete_rows_only() {
        let mut a = row("a", "forge");
        a.cost_usd = 0.0;
        a.notional_cost_usd = Some(0.30);
        a.cached_input_tokens = Some(900);
        let mut b = row("b", "forge");
        b.cost_usd = 0.0;
        b.notional_cost_usd = Some(0.10);
        b.cached_input_tokens = Some(500);
        let mut partial = row("c", "forge");
        partial.metrics_complete = false;
        let runs = vec![Run {
            rows: vec![a, b, partial],
            resolved: ids(&["a"]),
        }];
        let g = &summarize_groups(&runs)[0];
        assert_eq!(g.complete, 2);
        assert!((g.notional_per_attempt().unwrap() - 0.20).abs() < 1e-9);
        assert!((g.cache_hit().unwrap() - 0.7).abs() < 1e-9);
        assert_eq!(g.tokens_per_success_cell(), "incomplete");
    }

    #[test]
    fn groups_split_by_agent_and_resolved_model() {
        let with_model = |id: &str, agent: &str, model: &str| InstanceMetric {
            runtime: Some(BenchmarkRuntimeMetadata {
                resolved_model: Some(model.into()),
                ..Default::default()
            }),
            ..row(id, agent)
        };
        let runs = vec![
            Run {
                rows: vec![with_model("a", "forge", "sonnet")],
                resolved: ids(&["a"]),
            },
            Run {
                rows: vec![with_model("a", "forge", "opus")],
                resolved: ids(&[]),
            },
            Run {
                rows: vec![with_model("a", "forge", "sonnet")],
                resolved: ids(&[]),
            },
        ];
        let groups = summarize_groups(&runs);
        assert_eq!(groups.len(), 2);
        assert_eq!(
            (groups[0].model.as_str(), groups[0].attempts),
            ("sonnet", 2)
        );
        assert_eq!((groups[1].model.as_str(), groups[1].attempts), ("opus", 1));
    }

    #[test]
    fn pareto_marks_only_strictly_dominated_points() {
        let points = [
            (1.0, 0.5), // frontier: cheapest
            (2.0, 0.4), // dominated by 0 (costlier and worse)
            (2.0, 0.7), // frontier: best score
            (3.0, 0.7), // dominated by 2 (same score, costlier)
            (1.0, 0.5), // tie with 0: neither strictly better → frontier
        ];
        assert_eq!(
            pareto_dominators(&points),
            vec![None, Some(0), None, Some(2), None]
        );
    }
}
