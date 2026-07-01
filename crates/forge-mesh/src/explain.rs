//! A structured, human-readable explanation of a single routing decision — the data behind the
//! `/mesh` interactive inspector and `forge mesh explain`. It re-runs the exact production scoring
//! (no parallel logic) and records every step: classification, the per-model scored candidate
//! table, the quota snapshot, the conservation roll, and the final pick + fallback chain. The goal
//! is to make "why did the mesh choose this?" answerable, and to verify the policy is behaving.

use forge_types::{
    EffortLevel, ModelHealth, ProjectContext, QuotaStatus, SubscriptionQuota, TaskTier,
};

use crate::catalog::{self, ConserveDecision, ScoreRow};
use crate::{score_prompt, BudgetState, HeuristicRouter, RouteHints};

/// One model in the ranked candidate table, with the router's usability overlay.
#[derive(Debug, Clone)]
pub struct CandidateRow {
    pub rank: usize,
    pub row: ScoreRow,
    /// Provider key present (or keyless) AND not benched AND not an exhausted subscription.
    pub usable: bool,
    /// The model the mesh actually routed this prompt to.
    pub selected: bool,
}

/// A subscription provider's quota pressure + the spread probability for the explained tier.
#[derive(Debug, Clone)]
pub struct ProviderQuotaView {
    pub provider: String,
    pub status: QuotaStatus,
    pub fraction: f64,
    pub plan: String,
    /// Probability a task of this tier spreads OFF this subscription (the conservation pull).
    pub spread_probability: f64,
}

/// The full explanation of one routing decision.
#[derive(Debug, Clone)]
pub struct RoutingExplanation {
    pub prompt: String,
    /// Tier from prompt classification.
    pub classified_tier: TaskTier,
    /// Tier actually routed (may differ: budget downshift, pin override).
    pub routed_tier: TaskTier,
    pub classify_reasons: Vec<String>,
    pub code_heavy: bool,
    pub seed: u64,
    pub conserve: ConserveDecision,
    pub quota: Vec<ProviderQuotaView>,
    /// Ranked best-first; empty when auto-discovery routing is inactive (manual `[mesh.models]`).
    pub candidates: Vec<CandidateRow>,
    pub pick: String,
    pub fallbacks: Vec<String>,
    pub rationale: String,
    /// Human-readable label for which classifier produced this tier — set by the caller (forge-core)
    /// based on the configured `mesh.classifier`. Defaults to `"heuristic"`.
    pub classifier_label: String,
}

impl HeuristicRouter {
    /// Produce a full [`RoutingExplanation`] for `prompt` — the same decision [`route`](Self::route)
    /// would make, with every intermediate step exposed.
    pub fn explain(
        &self,
        prompt: &str,
        budget: BudgetState,
        health: &ModelHealth,
        quota: &SubscriptionQuota,
        effort: Option<EffortLevel>,
        project: &ProjectContext,
    ) -> RoutingExplanation {
        let cls = score_prompt(prompt, project);
        let hints = RouteHints::from_prompt(prompt);
        let tier = cls.tier;

        // The authoritative decision (pin / budget / fallback handling all live here). Compute it
        // FIRST: `decide` can downshift the tier (e.g. budget exhausted → Trivial), and the candidate
        // table + conservation data must describe the tier that ACTUALLY drove the pick, not the
        // classified one — otherwise `/mesh` shows the Trivial pick ranked last among Complex rows
        // with a Complex-tier conservation probability.
        let decision = self.decide(
            tier,
            cls.reasons.join(", "),
            budget,
            health,
            hints,
            quota,
            effort,
        );
        let routed_tier = decision.tier;

        let (conserve, rows) = if self.auto_active() {
            self.catalog.as_ref().unwrap().ranked_rows(
                routed_tier,
                &self.pricing,
                hints.code_heavy,
                hints.seed,
                quota,
                effort,
            )
        } else {
            (ConserveDecision::default(), Vec::new())
        };

        // Match `decide()`'s REAL routing filter exactly (`ordered_usable_for_tier`) — not just
        // `is_usable`. A row that's `is_usable` but paid under `credit_mode = Strict`, or whose
        // context window doesn't fit, is something `decide()` would NEVER actually pick; showing
        // it as `usable: true` here made the real pick (further down the list, first genuinely
        // eligible row) look inconsistent with the table, when the pick was correct all along.
        let min_context = crate::effective_min_context(budget.min_context_tokens, effort);
        let candidates = rows
            .into_iter()
            .enumerate()
            .map(|(i, row)| CandidateRow {
                rank: i + 1,
                usable: self.is_usable(&row.model, health, quota)
                    && self.allowed_under_credit_mode(&row.model)
                    && self.context_fits(&row.model, min_context),
                selected: row.model == decision.model,
                row,
            })
            .collect();

        // Quota views for each subscription provider present in the catalog.
        let mut sub_providers: Vec<String> = self
            .catalog
            .as_ref()
            .map(|c| {
                let mut v: Vec<String> = c
                    .models()
                    .iter()
                    .filter(|m| catalog::is_subscription(m))
                    .map(|m| catalog::provider_of(m).to_string())
                    .collect();
                v.sort();
                v.dedup();
                v
            })
            .unwrap_or_default();
        sub_providers.retain(|p| !p.is_empty());
        let quota_views = sub_providers
            .into_iter()
            .map(|p| {
                let fraction = quota.fraction_for(&p);
                let plan = quota.plan_for(&p).to_string();
                ProviderQuotaView {
                    spread_probability: crate::ModelCatalog::spread_probability(
                        routed_tier,
                        fraction,
                        &plan,
                        hints.code_heavy,
                    ),
                    status: quota.status_for(&p),
                    fraction,
                    plan,
                    provider: p,
                }
            })
            .collect();

        RoutingExplanation {
            prompt: prompt.to_string(),
            classified_tier: tier,
            routed_tier: decision.tier,
            classify_reasons: cls.reasons.iter().map(|s| s.to_string()).collect(),
            code_heavy: hints.code_heavy,
            seed: hints.seed,
            conserve,
            quota: quota_views,
            candidates,
            pick: decision.model,
            fallbacks: decision.fallbacks,
            rationale: decision.rationale,
            classifier_label: "heuristic".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{HeuristicRouter, ModelCatalog};
    use forge_config::Config;

    fn router() -> HeuristicRouter {
        HeuristicRouter::new(Config::default())
            .with_availability(|_| true)
            .with_catalog(ModelCatalog::new(vec![
                "claude-cli::opus".into(),
                "codex-cli::gpt-5.5".into(),
                "groq::llama-3.3-70b-versatile".into(),
                "groq::llama-3.1-8b-instant".into(),
            ]))
    }

    #[test]
    fn explanation_pick_matches_the_real_route() {
        let r = router();
        let prompt = "design and prove correct a lock-free queue";
        let e = r.explain(
            prompt,
            BudgetState::default(),
            &ModelHealth::default(),
            &SubscriptionQuota::default(),
            None,
            &ProjectContext::default(),
        );
        // The explained pick must equal what the selected candidate row says, and the top usable
        // row must be the pick (the table is the decision, made legible).
        let selected = e.candidates.iter().find(|c| c.selected).unwrap();
        assert_eq!(selected.row.model, e.pick);
        assert!(!e.candidates.is_empty());
        assert_eq!(e.classified_tier, TaskTier::Complex);
    }

    #[test]
    fn candidate_usable_flag_matches_decides_real_credit_mode_filter() {
        // Regression for a real bug: `CandidateRow.usable` only checked `is_usable` (key present +
        // not benched), not the FULL filter `decide()` actually routes with (`is_usable` AND
        // `allowed_under_credit_mode` AND `context_fits`). Under `credit_mode = Strict`, a paid
        // model showed `usable: true` in the table (able to mislead a viewer into thinking it was a
        // real candidate) while `decide()` silently skipped it — the real pick, further down the
        // list, then looked "inconsistent" with a table that was itself wrong.
        let mut config = Config::default();
        config.mesh.credit_mode = forge_types::CreditMode::Strict;
        let r = HeuristicRouter::new(config)
            .with_availability(|_| true)
            .with_catalog(ModelCatalog::new(vec![
                "openrouter::openai/gpt-5.5".into(), // paid, non-subscription — excluded by Strict
                "groq::llama-3.3-70b-versatile".into(), // free — still allowed
            ]));
        let e = r.explain(
            "design and prove correct a lock-free queue",
            BudgetState::default(),
            &ModelHealth::default(),
            &SubscriptionQuota::default(),
            None,
            &ProjectContext::default(),
        );
        let paid_row = e
            .candidates
            .iter()
            .find(|c| c.row.model == "openrouter::openai/gpt-5.5")
            .expect("paid model still appears in the ranked table");
        assert!(
            !paid_row.usable,
            "a paid model must show usable:false under credit_mode=Strict, matching decide()"
        );
        assert!(!paid_row.selected, "an unusable row can never be the pick");
        // The pick must be a row the table ALSO marks usable (no more "pick isn't in the usable
        // set" confusion) — and here that can only be the free groq model.
        let selected = e.candidates.iter().find(|c| c.selected).unwrap();
        assert!(selected.usable);
        assert_eq!(selected.row.model, e.pick);
        assert_eq!(e.pick, "groq::llama-3.3-70b-versatile");
    }

    #[test]
    fn explanation_surfaces_the_conservation_roll() {
        let r = router();
        let mut fr = std::collections::HashMap::new();
        fr.insert("claude-cli".to_string(), 0.5);
        fr.insert("codex-cli".to_string(), 0.5);
        let quota = SubscriptionQuota::new(std::collections::HashMap::new())
            .with_fractions(fr)
            .with_conserve(true);
        let e = r.explain(
            "design and prove correct a lock-free queue",
            BudgetState::default(),
            &ModelHealth::default(),
            &quota,
            None,
            &ProjectContext::default(),
        );
        assert!(e.conserve.enabled);
        assert!(e.conserve.eligible, "a free frontier alternative exists");
        assert!(e.conserve.probability > 0.0);
        // When conservation fires, the selected model is non-subscription and carries no penalty,
        // while the demoted subscriptions show the penalty.
        if e.conserve.fired {
            let sel = e.candidates.iter().find(|c| c.selected).unwrap();
            assert!(!sel.row.subscription, "fired → spread to free frontier");
        }
    }
}
