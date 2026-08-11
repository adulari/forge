//! `/mesh` overlay assembly — split from dispatch.rs to keep that file within its
//! architecture-size budget.

/// Build a fully-populated [`forge_tui::MeshOverlay`] from a routing explanation.
/// Extracted so both the sync path and the background-task path can share the logic.
pub(crate) fn build_mesh_overlay(
    e: forge_mesh::RoutingExplanation,
    prompt: &str,
) -> forge_tui::MeshOverlay {
    let conserve_line = if !e.conserve.enabled {
        "off".to_string()
    } else if !e.conserve.eligible {
        "no frontier alternative → not applied".to_string()
    } else if e.conserve.fired {
        format!(
            "FIRED (roll {:.2} < P {:.2}) → spread to free frontier",
            e.conserve.roll, e.conserve.probability
        )
    } else {
        format!(
            "not fired (roll {:.2} ≥ P {:.2}) → subscription kept",
            e.conserve.roll, e.conserve.probability
        )
    };
    // Only show candidates `decide()` could actually route to — an unusable row (benched,
    // exhausted, or excluded by credit_mode/context) is noise, not a real alternative. Top 12 of
    // those by score; if the actual pick still ranks below that (ties, or a longer usable tail),
    // always include it too, so the panel never shows 12 rows with none marked `selected`.
    let candidates: Vec<forge_tui::MeshCandRow> = {
        let mut top: Vec<_> = e.candidates.iter().filter(|c| c.usable).take(12).collect();
        if !top.iter().any(|c| c.selected) {
            if let Some(sel) = e.candidates.iter().find(|c| c.selected) {
                top.push(sel);
            }
        }
        top.into_iter()
            .map(|c| forge_tui::MeshCandRow {
                rank: c.rank,
                model: c.row.model.clone(),
                score: c.row.final_score,
                cost_tag: match c.row.cost_class {
                    0 => "free",
                    1 => "subscription",
                    _ => "paid",
                }
                .to_string(),
                frontier: c.row.frontier,
                usable: c.usable,
                selected: c.selected,
                penalty: c.row.conserve_penalty,
            })
            .collect()
    };
    forge_tui::MeshOverlay {
        open: true,
        loading: false,
        prompt: prompt.to_string(),
        classified: e.classified_tier.as_str().to_string(),
        classifier: e.classifier_label.clone(),
        routed: e.routed_tier.as_str().to_string(),
        code_heavy: e.code_heavy,
        reasons: e.classify_reasons.join(", "),
        conserve_fired: e.conserve.fired,
        conserve_line,
        quota: e
            .quota
            .iter()
            .map(|q| forge_tui::MeshQuotaRow {
                provider: q.provider.clone(),
                fraction: q.fraction,
                plan: q.plan.clone(),
                status: format!("{:?}", q.status),
                spread_complex: q.spread_probability,
                projected_fraction_at_reset: q.projected_fraction_at_reset,
                exhaustion_warning: q.exhaustion_warning,
            })
            .collect(),
        candidates: candidates.clone(),
        pick: e.pick.clone(),
        fallbacks: e.fallbacks.clone(),
        rationale: e.rationale.clone(),
        anim_tick: 0,
        // Start the browsing cursor on the actual pick, not row 0 — that's the row the user is
        // most likely to want to look at first.
        cursor: candidates.iter().position(|c| c.selected).unwrap_or(0),
    }
}
