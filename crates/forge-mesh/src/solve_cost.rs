//! Measured cost per solved task as a routing signal (docs/features/mesh-routing.md, "Measured
//! cost per solve").
//!
//! Artificial Analysis scores say how capable a model is in general. `forge bench report --record`
//! measures what a model costs per task it actually solves in Forge's own harness, and that order
//! can differ: harness fit varies by model family. This module turns those measurements into a
//! bounded score adjustment. It only applies to a model with at least [`MIN_SOLVE_ATTEMPTS`]
//! scored attempts, and only relative to the other measured models, because cost per solve on a
//! handful of tasks is noise and a lone measurement has nothing to be compared against.

use std::collections::HashMap;

/// Scored attempts a model needs before its measurement moves its rank. The same floor the bench
/// report uses before it prints a Pareto claim.
pub const MIN_SOLVE_ATTEMPTS: u32 = 20;

/// Largest score change, in route-score points. The outcome calibration uses the same bound: the
/// AA-based score stays dominant and this breaks close calls.
const MAX_ADJUSTMENT: f64 = 1.25;

/// Points per doubling of cost per solve relative to the median measured model.
const POINTS_PER_DOUBLING: f64 = 0.75;

/// One model's pooled bench measurements.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MeasuredSolveCost {
    /// pass@1-eligible attempts across every recorded run.
    pub attempts: u32,
    pub solved: u32,
    /// Spend over those attempts, with zero-cost calls priced at list rates (notional).
    pub cost_usd: f64,
}

impl MeasuredSolveCost {
    /// `None` when nothing was solved: the cost per solve is unbounded.
    pub fn cost_per_solve(&self) -> Option<f64> {
        (self.solved > 0).then(|| self.cost_usd / f64::from(self.solved))
    }

    pub fn pass_rate(&self) -> f64 {
        if self.attempts == 0 {
            0.0
        } else {
            f64::from(self.solved) / f64::from(self.attempts)
        }
    }
}

/// What the router did with a model's measurement, for `/mesh` and `forge mesh explain`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SolveCostSignal {
    pub measured: MeasuredSolveCost,
    /// The median cost per solve of the models that cleared the floor (`None` with fewer than
    /// two such models: nothing to compare against).
    pub reference: Option<f64>,
    /// Points added to the route score (0.0 when the gate did not pass).
    pub adjustment: f64,
}

impl SolveCostSignal {
    pub fn applied(&self) -> bool {
        self.adjustment != 0.0
    }

    /// Why the signal did or did not move the model, in one line.
    pub fn describe(&self) -> String {
        let m = &self.measured;
        let cps = m
            .cost_per_solve()
            .map_or_else(|| "no solve".to_string(), |c| format!("${c:.3}/solve"));
        let base = format!("{cps}, {}/{} solved", m.solved, m.attempts);
        if m.attempts < MIN_SOLVE_ATTEMPTS {
            return format!(
                "{base}; not applied (n {} < {MIN_SOLVE_ATTEMPTS})",
                m.attempts
            );
        }
        match self.reference {
            None => format!("{base}; not applied (no other measured model to compare)"),
            Some(r) => format!("{base} vs median ${r:.3}; {:+.2}", self.adjustment),
        }
    }
}

impl crate::catalog::ModelCatalog {
    /// Attach measured cost per solve (keyed by catalog id). Models below the attempt floor are
    /// kept for display and have no effect on ordering.
    pub fn with_measured_solve_cost(
        mut self,
        measured: HashMap<String, crate::solve_cost::MeasuredSolveCost>,
    ) -> Self {
        self.solve_cost = crate::solve_cost::signals(&measured);
        self
    }

    /// The cost-per-solve signal for `model`, when it has bench measurements.
    pub fn solve_cost_for(&self, model: &str) -> Option<&crate::solve_cost::SolveCostSignal> {
        self.solve_cost.get(model)
    }

    pub(crate) fn solve_cost_adjustment(&self, model: &str) -> f64 {
        self.solve_cost.get(model).map_or(0.0, |s| s.adjustment)
    }
}

/// Signals for every measured model. Gated models carry an adjustment of zero.
pub fn signals(measured: &HashMap<String, MeasuredSolveCost>) -> HashMap<String, SolveCostSignal> {
    let mut gated: Vec<f64> = measured
        .values()
        .filter(|m| m.attempts >= MIN_SOLVE_ATTEMPTS)
        .filter_map(MeasuredSolveCost::cost_per_solve)
        .collect();
    let qualifying = measured
        .values()
        .filter(|m| m.attempts >= MIN_SOLVE_ATTEMPTS)
        .count();
    gated.sort_by(f64::total_cmp);
    let reference = (qualifying >= 2 && !gated.is_empty()).then(|| median(&gated));
    measured
        .iter()
        .map(|(model, m)| {
            let adjustment = match reference {
                Some(r) if m.attempts >= MIN_SOLVE_ATTEMPTS => adjustment(m.cost_per_solve(), r),
                _ => 0.0,
            };
            (
                model.clone(),
                SolveCostSignal {
                    measured: *m,
                    reference,
                    adjustment,
                },
            )
        })
        .collect()
}

/// Summary over the explained candidates: how many were measured and which ones moved.
pub(crate) fn note<'a>(
    rows: impl Iterator<Item = (&'a str, Option<SolveCostSignal>)>,
) -> Option<String> {
    let measured: Vec<(&str, SolveCostSignal)> =
        rows.filter_map(|(m, s)| s.map(|s| (m, s))).collect();
    if measured.is_empty() {
        return None;
    }
    let moved: Vec<String> = measured
        .iter()
        .filter(|(_, s)| s.applied())
        .map(|(m, s)| format!("{m} {:+.2}", s.adjustment))
        .collect();
    Some(if moved.is_empty() {
        format!(
            "measured cost per solve: {} model(s) measured, none applied (needs n >= {MIN_SOLVE_ATTEMPTS} and a second measured model)",
            measured.len()
        )
    } else {
        format!("measured cost per solve moved {}", moved.join(", "))
    })
}

fn median(sorted: &[f64]) -> f64 {
    let mid = sorted.len() / 2;
    if sorted.len().is_multiple_of(2) {
        (sorted[mid - 1] + sorted[mid]) / 2.0
    } else {
        sorted[mid]
    }
}

/// Cheaper per solve than the median earns points, dearer loses them, logarithmically and capped.
fn adjustment(cost_per_solve: Option<f64>, reference: f64) -> f64 {
    let Some(cps) = cost_per_solve else {
        return -MAX_ADJUSTMENT;
    };
    if reference <= 0.0 {
        return if cps <= 0.0 { 0.0 } else { -MAX_ADJUSTMENT };
    }
    if cps <= 0.0 {
        return MAX_ADJUSTMENT;
    }
    (-(cps / reference).log2() * POINTS_PER_DOUBLING).clamp(-MAX_ADJUSTMENT, MAX_ADJUSTMENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(attempts: u32, solved: u32, cost: f64) -> MeasuredSolveCost {
        MeasuredSolveCost {
            attempts,
            solved,
            cost_usd: cost,
        }
    }

    fn map(rows: &[(&str, MeasuredSolveCost)]) -> HashMap<String, MeasuredSolveCost> {
        rows.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    }

    #[test]
    fn the_router_adds_the_adjustment_to_the_score_and_explains_it() {
        use crate::{BudgetState, HeuristicRouter, ModelCatalog};
        use forge_types::{ModelHealth, ProjectContext, SubscriptionQuota};
        let models = vec![
            "groq::llama-3.3-70b-versatile".to_string(),
            "groq::llama-3.1-8b-instant".to_string(),
            "openai::gpt-5.5".to_string(),
        ];
        let explain = |catalog: ModelCatalog| {
            HeuristicRouter::new(forge_config::Config::default())
                .with_availability(|_| true)
                .with_catalog(catalog)
                .explain(
                    "refactor the parser and add tests",
                    BudgetState::default(),
                    &ModelHealth::default(),
                    &SubscriptionQuota::default(),
                    None,
                    &ProjectContext::default(),
                )
        };
        let score = |e: &crate::RoutingExplanation, model: &str| {
            e.candidates
                .iter()
                .find(|c| c.row.model == model)
                .map(|c| c.row.final_score)
                .unwrap()
        };
        let plain = explain(ModelCatalog::new(models.clone()));
        assert!(plain.solve_cost_note().is_none());
        let measured = explain(ModelCatalog::new(models).with_measured_solve_cost(map(&[
            ("groq::llama-3.3-70b-versatile", m(20, 10, 1.0)),
            ("openai::gpt-5.5", m(20, 10, 4.0)),
            ("groq::llama-3.1-8b-instant", m(5, 1, 0.1)),
        ])));
        let cheap = "groq::llama-3.3-70b-versatile";
        let dear = "openai::gpt-5.5";
        let small = "groq::llama-3.1-8b-instant";
        // Median of $0.10 and $0.40 is $0.25: log2(0.4) and log2(1.6) times 0.75 points.
        assert!((score(&measured, cheap) - score(&plain, cheap) - 0.991_446).abs() < 1e-4);
        assert!((score(&measured, dear) - score(&plain, dear) + 0.508_554).abs() < 1e-4);
        assert_eq!(score(&measured, small), score(&plain, small), "n 5 < 20");
        let note = measured.solve_cost_note().unwrap();
        assert!(
            note.contains(cheap) && note.contains(dear) && !note.contains(small),
            "{note}"
        );
    }

    #[test]
    fn below_twenty_attempts_nothing_moves() {
        let s = signals(&map(&[("a::x", m(19, 10, 1.0)), ("b::y", m(40, 20, 8.0))]));
        assert_eq!(s["a::x"].adjustment, 0.0);
        assert_eq!(
            s["b::y"].adjustment, 0.0,
            "one qualifying model has no peer"
        );
        assert!(
            s["a::x"].describe().contains("n 19 < 20"),
            "{}",
            s["a::x"].describe()
        );
        assert!(s["b::y"].describe().contains("no other measured model"));
    }

    #[test]
    fn cheaper_per_solve_ranks_up_and_dearer_ranks_down() {
        // $0.10, $0.20, $0.40 per solve: the median is $0.20.
        let s = signals(&map(&[
            ("cheap::a", m(20, 10, 1.0)),
            ("mid::b", m(20, 10, 2.0)),
            ("dear::c", m(20, 10, 4.0)),
        ]));
        assert!((s["cheap::a"].adjustment - 0.75).abs() < 1e-9);
        assert_eq!(s["mid::b"].adjustment, 0.0);
        assert!((s["dear::c"].adjustment + 0.75).abs() < 1e-9);
        assert_eq!(s["mid::b"].reference, Some(0.2));
    }

    #[test]
    fn the_adjustment_is_bounded_and_a_model_that_never_solves_takes_the_floor() {
        let s = signals(&map(&[
            ("free::a", m(25, 5, 0.0)),
            ("mid::b", m(20, 10, 2.0)),
            ("huge::c", m(20, 1, 500.0)),
            ("zero::d", m(30, 0, 3.0)),
        ]));
        assert_eq!(s["free::a"].adjustment, MAX_ADJUSTMENT);
        assert_eq!(s["huge::c"].adjustment, -MAX_ADJUSTMENT);
        assert_eq!(s["zero::d"].adjustment, -MAX_ADJUSTMENT);
        assert!(s["zero::d"].describe().contains("no solve"));
    }
}
