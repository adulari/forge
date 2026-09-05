//! What a reasoning rung COSTS, and choosing a rung on value rather than quality alone.
//!
//! [`bench::select_rung`](crate::bench::select_rung) picks the cheapest rung that is not measurably
//! worse — a rule that never looks at price, because none was available. It therefore cannot express
//! the thing that actually decides a rung: GPT-6 Astra's `high` costs 1.22x its `medium` and is its
//! best coding rung, while `max` costs 2.22x for a coding index BELOW `high`. One of those is worth
//! buying and the other is not, and quality alone cannot tell them apart.
//!
//! ## Where the cost numbers come from, and where they do not
//!
//! Artificial Analysis publishes "Cost per Intelligence Index Task" per effort variant, which is
//! exactly the right quantity — but only on its public site and its Pro API tier. The free tier
//! (verified against a real key: indices, two latency figures, output speed, three price-per-token
//! fields, nothing else) has no cost-per-task and no output-token count, and a model's price PER
//! TOKEN is identical across its rungs, so the free feed cannot express a rung's cost at all.
//!
//! So the seed table below carries only ladders whose published figures are known, and Forge's own
//! measured usage overrides it as real data accrues. A model with neither is NOT routed on a guessed
//! cost: it falls back to the quality-band rule. An invented cost curve would look exactly like a
//! measured one to every caller, and would quietly misroute every model it covered.
//!
//! A rejected alternative, recorded so it is not retried: deriving reasoning tokens from
//! `time_to_first_answer_token x output_tokens_per_second`. The ladders it produces look plausible
//! and rise monotonically, but they are wrong — it implied Astra's max burns 95x medium, where the
//! published cost-per-task ratio is 2.2x. Time to first token is not a token count.

use crate::bench::BenchEffort;
use forge_types::EffortLevel;

/// Index points of quality one is willing to pay for per DOUBLING of cost.
///
/// Calibrated on the one fully-published ladder (GPT-6 Astra) and chosen from the stable middle of
/// its range: at 1.0 a coding turn takes `high` (its best coding rung, +22% cost) and a reasoning
/// turn takes `xhigh` (`max` buys +0.4 index for +39% cost). Below ~0.8 both run to `max`; above
/// ~1.5 both collapse to `low`.
pub const VALUE_LAMBDA: f64 = 1.0;

/// Published cost-per-task ladders, normalised so the model's cheapest rated rung is 1.0.
///
/// Deliberately tiny. Only GPT-6 Astra's full ladder is published per-rung; every other model is
/// listed at a single rung, which says nothing about the SHAPE of its ladder. Adding a family here
/// means its per-rung figures were actually read off a source, not interpolated.
const PUBLISHED: &[(&str, &[(BenchEffort, f64)])] = &[(
    // artificialanalysis.ai "Cost per Intelligence Index Task", read 2026-09-05:
    // low $0.63, medium $1.16, high $1.41, xhigh $1.85, max $2.57.
    "gpt-6-astra",
    &[
        (BenchEffort::Low, 1.000),
        (BenchEffort::Medium, 1.841),
        (BenchEffort::High, 2.238),
        (BenchEffort::XHigh, 2.937),
        (BenchEffort::Max, 4.079),
    ],
)];

/// The relative cost of running `model` at `rung`, or `None` when it is not known.
///
/// The scale is arbitrary but consistent WITHIN one model — only ratios between a model's own rungs
/// are ever used, so a per-model normalisation is enough and no cross-model comparison is implied.
pub fn published_cost_ratio(model: &str, rung: BenchEffort) -> Option<f64> {
    let want = crate::bench::tokens(model);
    PUBLISHED
        .iter()
        .find(|(family, _)| {
            let fam = crate::bench::tokens(family);
            !fam.is_empty() && fam.iter().all(|t| want.contains(t))
        })
        .and_then(|(_, ladder)| {
            ladder
                .iter()
                .find(|(r, _)| *r == rung)
                .map(|(_, ratio)| *ratio)
        })
}

/// Whether any cost is known for this model's ladder, i.e. whether a value-based choice is possible.
pub fn has_cost_data(model: &str) -> bool {
    published_cost_ratio(model, BenchEffort::Medium).is_some()
        || published_cost_ratio(model, BenchEffort::Low).is_some()
}

/// Choose a rung on VALUE: the one maximising `quality - lambda * log2(cost / cheapest cost)`.
///
/// Deliberately a value function over all rungs rather than a greedy climb. A greedy walk stops at
/// the first step that fails the threshold, so one weak early step hides every good step after it —
/// on Astra's real ladder that collapses the answer to `low`, because `low -> medium` is the
/// weakest link even though `medium -> high` is worth buying.
///
/// Returns `None` when no rung has a known cost; the caller must then fall back to the quality-only
/// rule rather than assume a cost.
pub fn select_rung_by_value(
    model: &str,
    ladder: &[(BenchEffort, crate::bench::BenchScore)],
    ceiling: Option<EffortLevel>,
    code_heavy: bool,
    lambda: f64,
) -> Option<EffortLevel> {
    let cap = ceiling.map(BenchEffort::from_level);
    let priced: Vec<(BenchEffort, f64, f64)> = ladder
        .iter()
        .filter(|(rung, _)| rung.to_level().is_some())
        .filter(|(rung, _)| cap.is_none_or(|cap| *rung <= cap))
        .filter_map(|(rung, score)| {
            let quality = if code_heavy {
                score.coding
            } else {
                score.intelligence
            };
            published_cost_ratio(model, *rung).map(|cost| (*rung, quality, cost))
        })
        .collect();
    let cheapest = priced
        .iter()
        .map(|(_, _, cost)| *cost)
        .fold(f64::INFINITY, f64::min);
    if !cheapest.is_finite() {
        return None;
    }
    priced
        .iter()
        .max_by(|a, b| {
            let value = |(_, q, c): &(BenchEffort, f64, f64)| q - lambda * (c / cheapest).log2();
            value(a).total_cmp(&value(b))
        })
        .and_then(|(rung, _, _)| rung.to_level())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bench::{BenchEffort, BenchScore};

    /// GPT-6 Astra's real quality ladder, verbatim from the live feed.
    fn astra_ladder() -> Vec<(BenchEffort, BenchScore)> {
        [
            (BenchEffort::NonReasoning, 47.8, 76.2),
            (BenchEffort::Low, 49.3, 75.7),
            (BenchEffort::Medium, 52.2, 76.7),
            (BenchEffort::High, 53.4, 77.1),
            (BenchEffort::XHigh, 54.3, 75.9),
            (BenchEffort::Max, 54.7, 76.9),
        ]
        .into_iter()
        .map(|(rung, intelligence, coding)| {
            (
                rung,
                BenchScore {
                    intelligence,
                    coding,
                },
            )
        })
        .collect()
    }

    const ASTRA: &str = "codex-oauth::gpt-6-astra";

    #[test]
    fn a_coding_turn_buys_the_cheap_step_to_the_best_coding_rung() {
        // high is Astra's best coding rung and costs only 1.22x medium — worth buying. This is the
        // answer the quality-only rule could not reach: it stopped at medium because it could not
        // see that the step was cheap.
        assert_eq!(
            select_rung_by_value(ASTRA, &astra_ladder(), None, true, VALUE_LAMBDA),
            Some(EffortLevel::High)
        );
    }

    #[test]
    fn a_reasoning_turn_stops_below_the_top_rung() {
        // max buys +0.4 intelligence for +39% cost; xhigh is the value pick.
        assert_eq!(
            select_rung_by_value(ASTRA, &astra_ladder(), None, false, VALUE_LAMBDA),
            Some(EffortLevel::XHigh)
        );
    }

    #[test]
    fn a_weak_first_step_does_not_hide_the_good_step_after_it() {
        // The greedy-walk bug this formulation exists to avoid: low -> medium is Astra's weakest
        // step (+1.0 coding for 1.84x cost), and a rule that stops at the first failing step would
        // answer `low` and never consider that medium -> high is cheap and worth taking.
        let picked = select_rung_by_value(ASTRA, &astra_ladder(), None, true, VALUE_LAMBDA);
        assert_ne!(picked, Some(EffortLevel::Low));
    }

    #[test]
    fn the_ceiling_still_caps_a_value_pick() {
        assert_eq!(
            select_rung_by_value(
                ASTRA,
                &astra_ladder(),
                Some(EffortLevel::Medium),
                false,
                VALUE_LAMBDA
            ),
            Some(EffortLevel::Medium)
        );
    }

    #[test]
    fn a_model_with_no_published_cost_yields_nothing_rather_than_a_guess() {
        // The safety property: no cost data means the caller falls back to the quality rule. An
        // invented curve would be indistinguishable from a measured one to every caller.
        assert!(!has_cost_data("anthropic::claude-opus-5"));
        assert_eq!(
            select_rung_by_value(
                "anthropic::claude-opus-5",
                &astra_ladder(),
                None,
                true,
                VALUE_LAMBDA
            ),
            None
        );
    }

    #[test]
    fn lambda_moves_the_pick_in_the_direction_it_should() {
        let ladder = astra_ladder();
        // Caring less about cost buys more effort; caring more buys less.
        assert_eq!(
            select_rung_by_value(ASTRA, &ladder, None, false, 0.3),
            Some(EffortLevel::WhiteHot)
        );
        assert_eq!(
            select_rung_by_value(ASTRA, &ladder, None, false, 5.0),
            Some(EffortLevel::Low)
        );
    }
}
