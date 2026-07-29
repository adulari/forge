//! Mesh CLI presentation helpers.

use forge_types::TaskTier;

/// A 10-cell ASCII meter for a 0.0–1.0 fraction.
pub(crate) fn meter(frac: f64) -> String {
    let filled = (frac.clamp(0.0, 1.0) * 10.0).round() as usize;
    format!("[{}{}]", "█".repeat(filled), "░".repeat(10 - filled))
}

/// A compact `→ 93% at reset ⚠` suffix for a quota line when a pace projection exists
/// (mesh-routing.md) — `""` when there isn't enough history to project one yet.
pub(crate) fn pace_suffix(
    projected_fraction_at_reset: Option<f64>,
    exhaustion_warning: bool,
) -> String {
    match projected_fraction_at_reset {
        Some(p) => format!(
            " → {:.0}% at reset{}",
            p * 100.0,
            if exhaustion_warning { " ⚠" } else { "" }
        ),
        None => String::new(),
    }
}

/// The no-prompt overview: subscription quota gauges + per-tier ranked picks.
pub(crate) fn mesh_overview(
    cat: &forge_mesh::ModelCatalog,
    config: &forge_config::Config,
    quota: &forge_types::SubscriptionQuota,
) {
    let pricing = forge_mesh::pricing::Pricing::from_config(config);
    println!(
        "subscription quota (conservation {}):",
        if config.mesh.subscription_conserve {
            "on"
        } else {
            "off"
        }
    );
    let mut subs: Vec<&str> = cat
        .models()
        .iter()
        .filter(|m| forge_mesh::catalog::is_subscription(m))
        .map(|m| forge_mesh::catalog::provider_of(m))
        .collect();
    subs.sort_unstable();
    subs.dedup();
    if subs.is_empty() {
        println!("  (no subscription bridges installed)");
    }
    for p in &subs {
        let frac = quota.fraction_for(p);
        let plan = quota.plan_for(p);
        let plan = if plan.is_empty() { "?" } else { plan };
        let pc = forge_mesh::ModelCatalog::spread_probability(TaskTier::Complex, frac, plan, false);
        let ps =
            forge_mesh::ModelCatalog::spread_probability(TaskTier::Standard, frac, plan, false);
        println!(
            "  {:<11} {} {:>3.0}% · plan {plan} · {:?} · spread P(complex)={:.0}% P(standard)={:.0}%",
            p,
            meter(frac),
            frac * 100.0,
            quota.status_for(p),
            pc * 100.0,
            ps * 100.0,
        );
    }
    println!("\nper-tier ranking (top 5):");
    for tier in [TaskTier::Trivial, TaskTier::Standard, TaskTier::Complex] {
        let (_, rows) = cat.ranked_rows(tier, &pricing, false, 0, quota, None);
        println!("  {}:", tier.as_str());
        for r in rows.iter().take(5) {
            println!(
                "    {:<34} score {:>6.2}  {}",
                r.model,
                r.final_score,
                cost_tag(r.cost_class)
            );
        }
    }
    println!("\ntip: `forge mesh \"<your task>\"` explains exactly how one prompt routes.");
}

/// JSON form of the no-prompt mesh overview, matching the documented `--json` behavior.
pub(crate) fn mesh_overview_json(
    cat: &forge_mesh::ModelCatalog,
    config: &forge_config::Config,
    quota: &forge_types::SubscriptionQuota,
) -> String {
    let pricing = forge_mesh::pricing::Pricing::from_config(config);
    let mut providers: Vec<&str> = cat
        .models()
        .iter()
        .filter(|model| forge_mesh::catalog::is_subscription(model))
        .map(|model| forge_mesh::catalog::provider_of(model))
        .collect();
    providers.sort_unstable();
    providers.dedup();

    let subscriptions: Vec<_> = providers
        .into_iter()
        .map(|provider| {
            let fraction = quota.fraction_for(provider);
            let plan = quota.plan_for(provider);
            serde_json::json!({
                "provider": provider,
                "fraction": fraction,
                "plan": plan,
                "status": format!("{:?}", quota.status_for(provider)),
                "complex_spread_probability": forge_mesh::ModelCatalog::spread_probability(
                    TaskTier::Complex, fraction, plan, false,
                ),
                "standard_spread_probability": forge_mesh::ModelCatalog::spread_probability(
                    TaskTier::Standard, fraction, plan, false,
                ),
            })
        })
        .collect();
    let rankings: serde_json::Map<String, serde_json::Value> =
        [TaskTier::Trivial, TaskTier::Standard, TaskTier::Complex]
            .into_iter()
            .map(|tier| {
                let (_, rows) = cat.ranked_rows(tier, &pricing, false, 0, quota, None);
                let rows: Vec<_> = rows
                    .into_iter()
                    .take(5)
                    .map(|row| {
                        serde_json::json!({
                            "model": row.model,
                            "provider": row.provider,
                            "final_score": row.final_score,
                            "cost_class": row.cost_class,
                            "subscription": row.subscription,
                            "frontier": row.frontier,
                        })
                    })
                    .collect();
                (tier.as_str().to_string(), serde_json::Value::Array(rows))
            })
            .collect();
    serde_json::to_string_pretty(&serde_json::json!({
        "subscription_conservation": config.mesh.subscription_conserve,
        "subscriptions": subscriptions,
        "rankings": rankings,
    }))
    .unwrap_or_else(|_| "{}".to_string())
}

pub(crate) fn cost_tag(class: u8) -> &'static str {
    match class {
        0 => "free",
        1 => "subscription",
        _ => "paid",
    }
}

/// The formatted single-prompt explanation.
pub(crate) fn print_mesh_explanation(e: &forge_mesh::RoutingExplanation) {
    println!("prompt: {:?}", e.prompt);
    println!("classifier: {}", e.classifier_label);
    print!("classified: {}", e.classified_tier.as_str());
    if e.routed_tier != e.classified_tier {
        print!(" → routed {}", e.routed_tier.as_str());
    }
    println!(
        "  ·  code-heavy: {}  ·  reasons: {}",
        if e.code_heavy { "yes" } else { "no" },
        e.classify_reasons.join(", ")
    );

    if !e.quota.is_empty() {
        println!("\nquota:");
        for q in &e.quota {
            let plan = if q.plan.is_empty() { "?" } else { &q.plan };
            println!(
                "  {:<11} {} {:>3.0}% · plan {plan} · {:?} · spread P={:.0}%{}",
                q.provider,
                meter(q.fraction),
                q.fraction * 100.0,
                q.status,
                q.spread_probability * 100.0,
                pace_suffix(q.projected_fraction_at_reset, q.exhaustion_warning),
            );
        }
    }

    let c = &e.conserve;
    if c.enabled {
        let verdict = if !c.eligible {
            "no frontier alternative → not applied".to_string()
        } else if c.fired {
            format!(
                "FIRED (roll {:.2} < P {:.2}) → spread off subscriptions",
                c.roll, c.probability
            )
        } else {
            format!(
                "not fired (roll {:.2} ≥ P {:.2}) → subscription kept",
                c.roll, c.probability
            )
        };
        println!("\nconservation: {verdict}");
    } else {
        println!("\nconservation: off");
    }

    if !e.candidates.is_empty() {
        // Only show candidates decide() could actually route to (see the TUI overlay's matching
        // fix in dispatch.rs::build_mesh_overlay) — top-8 of the usable ones, always including the
        // actual pick even if it ranks below that.
        let mut shown: Vec<_> = e.candidates.iter().filter(|c| c.usable).take(8).collect();
        if !shown.iter().any(|c| c.selected) {
            if let Some(sel) = e.candidates.iter().find(|c| c.selected) {
                shown.push(sel);
            }
        }
        println!("\ncandidates (top {}):", shown.len());
        for c in shown {
            let marker = if c.selected { "*" } else { " " };
            let pen = if c.row.conserve_penalty > 0.0 {
                format!(" −{:.0}", c.row.conserve_penalty)
            } else {
                String::new()
            };
            println!(
                "  {marker} #{:<2} {:<34} score {:>6.2}  cap {:>5.2}  {}{}{}",
                c.rank,
                c.row.model,
                c.row.final_score,
                c.row.capability,
                cost_tag(c.row.cost_class),
                pen,
                if c.row.frontier { " · frontier" } else { "" },
            );
        }
    }

    println!("\npick: {}", e.pick);
    if !e.fallbacks.is_empty() {
        println!("fallbacks: {}", e.fallbacks.join(", "));
    }
    println!("why: {}", e.rationale);
}

/// JSON form of the explanation (stable shape for scripting / tests).
pub(crate) fn mesh_explanation_json(e: &forge_mesh::RoutingExplanation) -> String {
    let candidates: Vec<_> = e
        .candidates
        .iter()
        .map(|c| {
            serde_json::json!({
                "rank": c.rank,
                "model": c.row.model,
                "provider": c.row.provider,
                "final_score": c.row.final_score,
                "capability": c.row.capability,
                "cost_class": c.row.cost_class,
                "conserve_penalty": c.row.conserve_penalty,
                "subscription": c.row.subscription,
                "frontier": c.row.frontier,
                "usable": c.usable,
                "selected": c.selected,
            })
        })
        .collect();
    let quota: Vec<_> = e
        .quota
        .iter()
        .map(|q| {
            serde_json::json!({
                "provider": q.provider,
                "status": format!("{:?}", q.status),
                "fraction": q.fraction,
                "plan": q.plan,
                "spread_probability": q.spread_probability,
                "projected_fraction_at_reset": q.projected_fraction_at_reset,
                "exhaustion_warning": q.exhaustion_warning,
            })
        })
        .collect();
    serde_json::to_string_pretty(&serde_json::json!({
        "prompt": e.prompt,
        "classifier": e.classifier_label,
        "classified_tier": e.classified_tier.as_str(),
        "routed_tier": e.routed_tier.as_str(),
        "classify_reasons": e.classify_reasons,
        "code_heavy": e.code_heavy,
        "seed": e.seed,
        "conserve": {
            "enabled": e.conserve.enabled,
            "eligible": e.conserve.eligible,
            "probability": e.conserve.probability,
            "roll": e.conserve.roll,
            "fired": e.conserve.fired,
        },
        "quota": quota,
        "candidates": candidates,
        "pick": e.pick,
        "fallbacks": e.fallbacks,
        "rationale": e.rationale,
    }))
    .unwrap_or_else(|_| "{}".into())
}

#[cfg(test)]
mod tests {
    use super::{mesh_explanation_json, meter, pace_suffix};

    fn explanation() -> forge_mesh::RoutingExplanation {
        forge_mesh::RoutingExplanation {
            prompt: "test prompt".into(),
            classified_tier: forge_types::TaskTier::Standard,
            routed_tier: forge_types::TaskTier::Standard,
            classify_reasons: vec![],
            code_heavy: false,
            seed: 0,
            conserve: forge_mesh::catalog::ConserveDecision::default(),
            quota: vec![],
            candidates: vec![],
            pick: "model".into(),
            fallbacks: vec![],
            rationale: "test".into(),
            classifier_label: "heuristic fallback".into(),
        }
    }

    #[test]
    fn explanation_json_exposes_classifier_label() {
        let value: serde_json::Value = serde_json::from_str(&mesh_explanation_json(&explanation()))
            .expect("valid explanation JSON");
        assert_eq!(value["classifier"], "heuristic fallback");
    }

    #[test]
    fn meter_clamps_and_rounds_fraction_boundaries() {
        assert_eq!(meter(-1.0), "[░░░░░░░░░░]");
        assert_eq!(meter(0.05), "[█░░░░░░░░░]");
        assert_eq!(meter(1.0), "[██████████]");
    }

    #[test]
    fn pace_suffix_includes_warning_only_with_a_projection() {
        assert_eq!(pace_suffix(None, true), "");
        assert_eq!(pace_suffix(Some(0.93), true), " → 93% at reset ⚠");
    }
}
