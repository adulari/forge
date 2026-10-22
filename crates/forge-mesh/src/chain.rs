//! The ordered failover chain a routing decision carries: the routed tier's usable models first,
//! then the other tiers as cross-tier fallbacks, or only a pinned set's members.

use super::*;

impl HeuristicRouter {
    /// Build the ordered failover chain for the routed tier: that tier's usable models first,
    /// then the other tiers (Complex → Standard → Trivial) as cross-tier fallbacks, deduped.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn build_chain(
        &self,
        routed: TaskTier,
        health: &ModelHealth,
        hints: RouteHints,
        quota: &SubscriptionQuota,
        effort: Option<EffortLevel>,
        min_context: Option<u32>,
        has_images: bool,
    ) -> Vec<String> {
        let routed_usable = self.ordered_usable_for_tier(
            routed,
            health,
            hints,
            quota,
            effort,
            min_context,
            has_images,
        );
        self.build_chain_from(
            routed_usable,
            routed,
            health,
            hints,
            quota,
            effort,
            min_context,
            has_images,
        )
    }

    /// [`build_chain`](Self::build_chain) for a caller that already ranked the routed tier, so
    /// that ranking (one pass over the whole catalog) is not repeated.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn build_chain_from(
        &self,
        routed_usable: Vec<String>,
        routed: TaskTier,
        health: &ModelHealth,
        hints: RouteHints,
        quota: &SubscriptionQuota,
        effort: Option<EffortLevel>,
        min_context: Option<u32>,
        has_images: bool,
    ) -> Vec<String> {
        let mut seen: std::collections::HashSet<String> = routed_usable.iter().cloned().collect();
        let mut chain = routed_usable;
        for tier in [TaskTier::Complex, TaskTier::Standard, TaskTier::Trivial] {
            if tier == routed {
                continue;
            }
            for m in self.ordered_usable_for_tier(
                tier,
                health,
                hints,
                quota,
                effort,
                min_context,
                has_images,
            ) {
                if seen.insert(m.clone()) {
                    chain.push(m);
                }
            }
        }
        // A twin can appear in a different tier's configured list. Apply the same rule across
        // the completed chain so a cross-tier fallback never reintroduces a bridge that has a
        // usable OAuth surface elsewhere in this turn.
        Self::suppress_usable_oauth_superseded_bridges(&mut chain);
        chain
    }

    /// Build the ordered failover chain for a PIN SET: the set's members ranked by the mesh's
    /// normal ordering (usable first, then by tier fit / cost / health / quota / benchmark), and
    /// ONLY the set's members — failover never escapes the set. Usable members are ranked first;
    /// unusable (benched/rate-limited/unhealthy) members are skipped during ranking. Returns
    /// `Vec::new()` when no member is usable — the caller then dispatches the first member anyway
    /// so the provider's real error surfaces (mirrors single-pin's "unavailable" behaviour).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn build_chain_restricted_to(
        &self,
        pin: &[String],
        routed: TaskTier,
        health: &ModelHealth,
        hints: RouteHints,
        quota: &SubscriptionQuota,
        effort: Option<EffortLevel>,
        min_context: Option<u32>,
        has_images: bool,
    ) -> Vec<String> {
        let full = self.build_chain(
            routed,
            health,
            hints,
            quota,
            effort,
            min_context,
            has_images,
        );
        // Preserve the mesh's ranked order over the full chain, restricted to the set. This ranks
        // WITHIN the set exactly as the mesh ranks the full catalog (tier fit, cost, health,
        // quota, benchmarks), and drops every member the set didn't pin.
        full.into_iter().filter(|m| pin.contains(m)).collect()
    }
}
