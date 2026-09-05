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

/// What subscription pacing currently withholds, over the whole discovered catalog — the same
/// rule `decide()` applies per tier, asked provider-wide for the overview.
fn pacing_holds(
    cat: &forge_mesh::ModelCatalog,
    config: &forge_config::Config,
    quota: &forge_types::SubscriptionQuota,
) -> Vec<forge_mesh::PacingHold> {
    forge_mesh::HeuristicRouter::new(config.clone())
        .with_catalog(cat.clone())
        .pacing_holds(cat.models(), quota)
}

/// The pace marker for one provider's most-constrained window, quoting the router's own
/// [`forge_types::SubscriptionPacing`] rather than a display-side approximation.
pub(crate) fn pacing_note(
    pacing: Option<&forge_types::SubscriptionPacing>,
    hold: Option<&forge_mesh::PacingHold>,
) -> String {
    forge_mesh::pacing_summary(pacing, hold)
}

/// Machine-readable form of the same decision, including `used_nominal_fallback` so a consumer
/// can tell a provider-timed pace from one inferred off the nominal window length.
pub(crate) fn pacing_json(
    pacing: Option<&forge_types::SubscriptionPacing>,
    hold: Option<&forge_mesh::PacingHold>,
) -> serde_json::Value {
    let Some(pacing) = pacing else {
        return serde_json::Value::Null;
    };
    serde_json::json!({
        "window": pacing.window,
        "fraction_used": pacing.fraction_used,
        "allowed_fraction": pacing.allowed_fraction,
        "elapsed_fraction": pacing.elapsed_fraction(),
        "over_pace": pacing.is_over_pace(),
        "used_nominal_fallback": pacing.used_nominal_fallback,
        "summary": pacing_note(Some(pacing), hold),
        "held": hold.map(|hold| hold.held.clone()).unwrap_or_default(),
        "kept": hold.map(|hold| hold.kept.clone()).unwrap_or_default(),
    })
}

/// A provider currently excluded from routing: `(provider, cooldown_until, reason)`, as read from
/// `Store::current_excluded_providers`.
pub(crate) type ProviderExclusion = (String, i64, String);

/// Render the excluded-provider block shared by the mesh overview and the per-prompt explanation.
///
/// An excluded provider is the single most consequential thing the mesh can be doing, and it was
/// the one thing no surface said: a whole subscription vanished from every candidate table with no
/// row, no note, and no reason. It prints before the rankings because it explains them.
pub(crate) fn print_provider_exclusions(excluded: &[ProviderExclusion]) {
    if excluded.is_empty() {
        return;
    }
    println!("excluded providers (every alias is out of routing):");
    for (provider, until, reason) in excluded {
        println!(
            "  ⊘ {provider:<11} {} · expires in {}",
            reason,
            remaining(*until)
        );
    }
    println!("  → `forge models --probe` re-verifies them now\n");
}

/// JSON form of [`print_provider_exclusions`], shared by both `--json` shapes.
pub(crate) fn provider_exclusions_json(excluded: &[ProviderExclusion]) -> Vec<serde_json::Value> {
    excluded
        .iter()
        .map(|(provider, until, reason)| {
            serde_json::json!({
                "provider": provider,
                "reason": reason,
                "expires_at": until,
                "expires_in_secs": (until - chrono::Utc::now().timestamp()).max(0),
            })
        })
        .collect()
}

/// A compact "12m" / "3h 20m" until an absolute epoch-second expiry.
fn remaining(until_epoch: i64) -> String {
    let secs = (until_epoch - chrono::Utc::now().timestamp()).max(0);
    if secs >= 3600 {
        format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
    } else if secs >= 60 {
        format!("{}m", secs / 60)
    } else {
        format!("{secs}s")
    }
}

/// The no-prompt overview: subscription quota gauges + per-tier ranked picks.
pub(crate) fn mesh_overview(
    cat: &forge_mesh::ModelCatalog,
    config: &forge_config::Config,
    quota: &forge_types::SubscriptionQuota,
    excluded: &[ProviderExclusion],
    windows: &[forge_store::SubscriptionWindow],
) {
    print_provider_exclusions(excluded);
    let pricing = super::discovery::pricing_with_fetched_rates(config);
    let holds = pacing_holds(cat, config, quota);
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
        .map(|m| match forge_mesh::catalog::provider_of(m) {
            "codex-oauth" => "codex-cli",
            provider => provider,
        })
        .collect();
    subs.sort_unstable();
    subs.dedup();
    if subs.is_empty() {
        println!("  (no subscription bridges installed)");
    }
    for p in &subs {
        let observed = quota.observed_fraction_for(p);
        let frac = quota.fraction_for(p);
        let plan = quota.plan_for(p);
        let plan = if plan.is_empty() { "?" } else { plan };
        let pc = forge_mesh::ModelCatalog::spread_probability(TaskTier::Complex, frac, plan, false);
        let ps =
            forge_mesh::ModelCatalog::spread_probability(TaskTier::Standard, frac, plan, false);
        let hold = holds
            .iter()
            .find(|hold| forge_mesh::display_provider(&hold.provider) == *p);
        let pacing = quota
            .pacing_for(p)
            .cloned()
            .or_else(|| hold.map(|hold| hold.pacing.clone()));
        println!(
            "  {:<11} {} {} · plan {plan} · {:?} · {} · spread P(complex)={:.0}% P(standard)={:.0}%",
            p,
            observed.map(meter).unwrap_or_else(|| "unknown".to_string()),
            observed
                .map(|f| format!("{:.0}%", f * 100.0))
                .unwrap_or_else(|| "unknown".to_string()),
            quota.status_for(p),
            pacing_note(pacing.as_ref(), hold),
            pc * 100.0,
            ps * 100.0,
        );
        // The provider line above collapses to the single strictest window. Providers that report
        // several simultaneous windows (OpenCode Go reports three) need each one shown with its
        // own reset, or a healthy 5-hour reading hides a nearly-spent month.
        for window in windows.iter().filter(|window| &window.provider == p) {
            let Some(fraction) = window.fraction else {
                continue;
            };
            println!(
                "      {:<10} {} {:>4.0}%{}",
                window.window_kind,
                meter(fraction),
                fraction * 100.0,
                window
                    .resets_at
                    .map(|resets_at| format!(" · resets in {}", remaining(resets_at)))
                    .unwrap_or_default(),
            );
        }
        for line in opencode_go_quota_lines(cat, p) {
            println!("      {line}");
        }
    }
    // A held model can still rank at the top of this table, so mark the hold per MODEL: the
    // provider line above only names the hold, and failover now treats it as a last resort.
    let held: std::collections::HashSet<&str> = holds
        .iter()
        .flat_map(|hold| hold.held.iter().map(String::as_str))
        .collect();
    println!("\nper-tier ranking (top 5):");
    for tier in [TaskTier::Trivial, TaskTier::Standard, TaskTier::Complex] {
        let (_, rows) = cat.ranked_rows(tier, &pricing, false, 0, quota, None);
        println!("  {}:", tier.as_str());
        for r in rows.iter().take(5) {
            println!(
                "    {:<34} score {:>6.2}  {}{}{}",
                r.model,
                r.final_score,
                cost_tag(r.cost_class),
                quota_suffix(&r.model),
                if held.contains(r.model.as_str()) {
                    " · held (pacing): last resort only"
                } else {
                    ""
                }
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
    excluded: &[ProviderExclusion],
    windows: &[forge_store::SubscriptionWindow],
) -> String {
    let pricing = super::discovery::pricing_with_fetched_rates(config);
    let holds = pacing_holds(cat, config, quota);
    let mut providers: Vec<&str> = cat
        .models()
        .iter()
        .filter(|model| forge_mesh::catalog::is_subscription(model))
        .map(|model| match forge_mesh::catalog::provider_of(model) {
            "codex-oauth" => "codex-cli",
            provider => provider,
        })
        .collect();
    providers.sort_unstable();
    providers.dedup();

    let subscriptions: Vec<_> = providers
        .into_iter()
        .map(|provider| {
            let observed_fraction = quota.observed_fraction_for(provider);
            let fraction = quota.fraction_for(provider);
            let plan = quota.plan_for(provider);
            let hold = holds
                .iter()
                .find(|hold| forge_mesh::display_provider(&hold.provider) == provider);
            let pacing = quota
                .pacing_for(provider)
                .cloned()
                .or_else(|| hold.map(|hold| hold.pacing.clone()));
            serde_json::json!({
                "provider": provider,
                "pacing": pacing_json(pacing.as_ref(), hold),
                "fraction": observed_fraction,
                "windows": windows
                    .iter()
                    .filter(|window| window.provider == provider)
                    .map(|window| serde_json::json!({
                        "window": window.window_kind,
                        "fraction": window.fraction,
                        "status": window.status,
                        "resets_at": window.resets_at,
                    }))
                    .collect::<Vec<_>>(),
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
                    "pacing_held": holds
                        .iter()
                        .any(|hold| hold.held.contains(&row.model)),
                    "frontier": row.frontier,
                    "weekly_quota_usd": forge_mesh::opencode_go_weekly_quota(&row.model),
                    "quota_multiplier": forge_mesh::opencode_go_quota_multiplier(&row.model),
                })
            })
            .collect();
                (tier.as_str().to_string(), serde_json::Value::Array(rows))
            })
            .collect();
    serde_json::to_string_pretty(&serde_json::json!({
        "subscription_conservation": config.mesh.subscription_conserve,
        "excluded_providers": provider_exclusions_json(excluded),
        "subscriptions": subscriptions,
        "rankings": rankings,
    }))
    .unwrap_or_else(|_| "{}".to_string())
}

/// ` · quota $7.50/wk → x4.0` for an OpenCode Go model, empty otherwise.
fn quota_suffix(model: &str) -> String {
    forge_mesh::opencode_go_quota_note(model)
        .map(|note| format!(" · {note}"))
        .unwrap_or_default()
}

/// The per-model weekly quota buckets for `provider`'s OpenCode Go models, largest multiplier
/// first. The weekly pool percentage is the sum of per-model percentages and the usage endpoint
/// exposes no per-model split, so this is the operator's only view of which models drain the
/// pool fastest per dollar.
fn opencode_go_quota_lines(cat: &forge_mesh::ModelCatalog, provider: &str) -> Vec<String> {
    if provider != "opencode_go" {
        return Vec::new();
    }
    let mut buckets: std::collections::BTreeMap<u64, Vec<&str>> = std::collections::BTreeMap::new();
    for model in cat.models() {
        if forge_mesh::catalog::provider_of(model) != provider {
            continue;
        }
        let cents = forge_mesh::opencode_go_weekly_quota(model)
            .map(|quota| (quota * 100.0).round() as u64)
            .unwrap_or(0);
        buckets
            .entry(cents)
            .or_default()
            .push(model.split_once("::").map_or(model.as_str(), |(_, m)| m));
    }
    let mut lines = vec!["weekly quota per model (pool % = sum of per-model %):".to_string()];
    for (cents, models) in &buckets {
        let label = if *cents == 0 {
            "unknown   x1.0".to_string()
        } else {
            let quota = *cents as f64 / 100.0;
            format!(
                "${quota:>5.2}/wk x{:.1}",
                forge_mesh::OPENCODE_GO_LARGEST_WEEKLY_QUOTA / quota
            )
        };
        lines.push(format!("  {label}  {}", models.join(", ")));
    }
    lines
}

pub(crate) fn cost_tag(class: u8) -> &'static str {
    match class {
        0 => "free",
        1 => "subscription",
        _ => "paid",
    }
}

/// The formatted single-prompt explanation.
pub(crate) fn print_mesh_explanation(
    e: &forge_mesh::RoutingExplanation,
    excluded: &[ProviderExclusion],
) {
    print_provider_exclusions(excluded);
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
                "  {:<11} {} {} · plan {plan} · {:?} · {} · spread P={:.0}%{}",
                q.provider,
                q.fraction
                    .map(meter)
                    .unwrap_or_else(|| "unknown".to_string()),
                q.fraction
                    .map(|fraction| format!("{:.0}%", fraction * 100.0))
                    .unwrap_or_else(|| "unknown".to_string()),
                q.status,
                pacing_note(q.pacing.as_ref(), q.hold.as_ref()),
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
        let mut any_reordered = false;
        for c in shown {
            any_reordered |= c.reorder_reason.is_some();
            let marker = if c.selected { "*" } else { " " };
            let pen = if c.row.conserve_penalty > 0.0 {
                format!(" −{:.0}", c.row.conserve_penalty)
            } else {
                String::new()
            };
            // The score stays the catalog score; a rank a routing rule decided is marked, not
            // renumbered — a rank that contradicts its own number is worse than no number at all.
            let reorder = match c.reorder_reason {
                Some(reason) => format!(" · ↕ {reason}"),
                None => String::new(),
            };
            // The rung this row would run at, in the same ⟨…⟩ notation the statusline and the
            // /mesh overlay use. Two rows for one model on different providers can differ only
            // here, which the rest of the line cannot express.
            let rung = c
                .effort
                .map_or_else(String::new, |rung| format!(" · ⟨{}⟩", rung.as_str()));
            println!(
                "  {marker} #{:<2} {:<34} score {:>6.2}  cap {:>5.2}  {}{}{}{}{}{}",
                c.rank,
                c.row.model,
                c.row.final_score,
                c.row.capability,
                cost_tag(c.row.cost_class),
                pen,
                rung,
                if c.row.frontier { " · frontier" } else { "" },
                quota_suffix(&c.row.model),
                reorder,
            );
        }
        if any_reordered {
            println!("  ranks marked ↕ were decided by routing rules; score is the catalog score");
        }
    }

    match e.pick_effort {
        Some(rung) => println!("\npick: {} ⟨{}⟩", e.pick, rung.as_str()),
        None => println!("\npick: {}", e.pick),
    }
    if !e.fallbacks.is_empty() {
        println!("fallbacks: {}", e.fallbacks.join(", "));
    }
    println!("why: {}", e.rationale);
}

/// JSON form of the explanation (stable shape for scripting / tests).
pub(crate) fn mesh_explanation_json(
    e: &forge_mesh::RoutingExplanation,
    excluded: &[ProviderExclusion],
) -> String {
    let candidates: Vec<_> = e
        .candidates
        .iter()
        .map(|c| {
            serde_json::json!({
                "rank": c.rank,
                "model": c.row.model,
                "provider": c.row.provider,
                "final_score": c.row.final_score,
                // Added, not renamed: the routing rule that decided this row's rank, or null
                // when its catalog score did.
                "reorder_reason": c.reorder_reason,
                "capability": c.row.capability,
                "cost_class": c.row.cost_class,
                "conserve_penalty": c.row.conserve_penalty,
                "subscription": c.row.subscription,
                "frontier": c.row.frontier,
                "usable": c.usable,
                "selected": c.selected,
                // Added, not renamed: the reasoning rung this row would run at, already resolved
                // against its provider surface. Null when the model has no measured effort ladder
                // or no reasoning control at all.
                "effort": c.effort.map(|rung| rung.as_str()),
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
                "pacing": pacing_json(q.pacing.as_ref(), q.hold.as_ref()),
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
        "excluded_providers": provider_exclusions_json(excluded),
        "candidates": candidates,
        "pick": e.pick,
        "pick_effort": e.pick_effort.map(|rung| rung.as_str()),
        "fallbacks": e.fallbacks,
        "rationale": e.rationale,
    }))
    .unwrap_or_else(|_| "{}".into())
}

/// How much of a listed catalog is actually callable right now.
///
/// The catalog always carries the configured tier candidates as cold-start seeds (discovery.rs),
/// so `forge models` lists entries even on a virgin install with no credentials at all. Listing
/// them without saying they are unreachable told a first-run user the setup was done, and the
/// only correction came later as an unroutable turn.
pub(crate) struct Reachability {
    pub reachable: usize,
    pub total: usize,
}

pub(crate) fn reachability(models: &[String], callable: impl Fn(&str) -> bool) -> Reachability {
    Reachability {
        reachable: models.iter().filter(|m| callable(m)).count(),
        total: models.len(),
    }
}

impl Reachability {
    /// The line to print above the catalog when some or all of it cannot be called, naming the
    /// exact next command. `None` when every listed model is callable.
    pub(crate) fn warning(&self) -> Option<String> {
        match (self.reachable, self.total) {
            (_, 0) => None,
            (0, _) => Some(format!(
                "✗ none of these {} models is callable — no provider credentials are configured.\n  \
                 They are cold-start seed entries, not models you can run yet.\n  \
                 → run `forge setup` (or `forge auth <provider>`), then `forge models` again",
                self.total
            )),
            (r, t) if r < t => Some(format!(
                "⚠ {} of {t} listed models have no credentials (marked `no key` below) — \
                 `forge auth <provider>` to enable them",
                t - r
            )),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{mesh_explanation_json, meter, pace_suffix, pacing_note, reachability, remaining};
    use forge_mesh::ProviderQuotaView;
    use forge_types::SubscriptionPacing;

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
            pick_effort: None,
            fallbacks: vec![],
            rationale: "test".into(),
            classifier_label: "heuristic fallback".into(),
        }
    }

    #[test]
    fn explanation_json_exposes_classifier_label() {
        let value: serde_json::Value =
            serde_json::from_str(&mesh_explanation_json(&explanation(), &[]))
                .expect("valid explanation JSON");
        assert_eq!(value["classifier"], "heuristic fallback");
        assert_eq!(
            value["excluded_providers"],
            serde_json::json!([]),
            "the key is always present so a consumer can distinguish 'none' from 'not reported'"
        );
    }

    /// An excluded provider must be visible wherever the mesh explains a routing choice — with its
    /// reason and its expiry, not merely as an absence from the candidate table.
    #[test]
    fn explanation_json_surfaces_an_excluded_provider_with_reason_and_expiry() {
        let until = chrono::Utc::now().timestamp() + 900;
        let excluded = vec![(
            "claude-cli".to_string(),
            until,
            "excluded: provider auth failed: auth failed".to_string(),
        )];
        let value: serde_json::Value =
            serde_json::from_str(&mesh_explanation_json(&explanation(), &excluded))
                .expect("valid explanation JSON");
        let row = &value["excluded_providers"][0];
        assert_eq!(row["provider"], "claude-cli");
        assert_eq!(row["reason"], "excluded: provider auth failed: auth failed");
        assert_eq!(row["expires_at"], until);
        assert!(
            row["expires_in_secs"].as_i64().unwrap() > 0,
            "the user must be told how long the subscription stays gone: {row}"
        );
    }

    #[test]
    fn remaining_reads_as_a_human_duration() {
        let now = chrono::Utc::now().timestamp();
        assert_eq!(remaining(now - 5), "0s");
        assert_eq!(remaining(now + 90), "1m");
        assert_eq!(remaining(now + 3 * 3600 + 20 * 60), "3h 20m");
    }

    #[test]
    fn explanation_json_preserves_an_unknown_quota_fraction_as_null() {
        let mut explanation = explanation();
        explanation.quota.push(ProviderQuotaView {
            provider: "claude-cli".into(),
            status: forge_types::QuotaStatus::Ok,
            fraction: None,
            plan: String::new(),
            spread_probability: 0.0,
            projected_fraction_at_reset: None,
            exhaustion_warning: false,
            pacing: None,
            hold: None,
        });
        let value: serde_json::Value =
            serde_json::from_str(&mesh_explanation_json(&explanation, &[]))
                .expect("valid explanation JSON");
        assert!(value["quota"][0]["fraction"].is_null());
    }

    #[test]
    fn meter_clamps_and_rounds_fraction_boundaries() {
        assert_eq!(meter(-1.0), "[░░░░░░░░░░]");
        assert_eq!(meter(0.05), "[█░░░░░░░░░]");
        assert_eq!(meter(1.0), "[██████████]");
    }

    fn pacing(
        fraction_used: f64,
        allowed_fraction: f64,
        resets_at: Option<i64>,
    ) -> SubscriptionPacing {
        SubscriptionPacing {
            window: "weekly".into(),
            fraction_used,
            allowed_fraction,
            elapsed_secs: 172_800,
            total_secs: 604_800,
            resets_at,
            used_nominal_fallback: resets_at.is_none(),
        }
    }

    /// The whole point of this view: an operator reading `forge mesh` must be able to tell a
    /// paced window from a healthy one, and a real pace from a guessed one.
    #[test]
    fn the_provider_line_states_whether_the_window_is_over_pace() {
        let over = pacing(0.27, 0.21, Some(1_000_000));
        let hold = forge_mesh::PacingHold {
            provider: "codex-oauth".into(),
            pacing: over.clone(),
            held: vec![
                "codex-oauth::gpt-5.6-sol".into(),
                "codex-oauth::gpt-5.6-terra".into(),
            ],
            kept: vec!["codex-oauth::gpt-5.6-luna".into()],
        };
        assert_eq!(
            pacing_note(Some(&over), Some(&hold)),
            "weekly 27% used · 21% allowed · OVER PACE → gpt-5.6-sol/gpt-5.6-terra held, gpt-5.6-luna"
        );
        assert_eq!(
            pacing_note(Some(&pacing(0.10, 0.21, Some(1_000_000))), None),
            "weekly 10% used · 21% allowed · on pace"
        );
        let unknown = pacing_note(Some(&pacing(0.27, 0.21, None)), None);
        assert!(unknown.contains("pace unknown"), "{unknown}");
        assert!(unknown.contains("used_nominal_fallback"), "{unknown}");
        assert_eq!(pacing_note(None, None), "pace unknown (no observed window)");
    }

    #[test]
    fn the_explanation_json_carries_the_pacing_decision_it_printed() {
        let mut explanation = explanation();
        let over = pacing(0.27, 0.21, Some(1_000_000));
        explanation.quota.push(ProviderQuotaView {
            provider: "codex-cli".into(),
            status: forge_types::QuotaStatus::Ok,
            fraction: Some(0.27),
            plan: "plus".into(),
            spread_probability: 0.0,
            projected_fraction_at_reset: None,
            exhaustion_warning: false,
            pacing: Some(over.clone()),
            hold: Some(forge_mesh::PacingHold {
                provider: "codex-oauth".into(),
                pacing: over,
                held: vec!["codex-oauth::gpt-5.6-sol".into()],
                kept: vec!["codex-oauth::gpt-5.6-luna".into()],
            }),
        });
        let value: serde_json::Value =
            serde_json::from_str(&mesh_explanation_json(&explanation, &[]))
                .expect("valid explanation JSON");
        let pacing = &value["quota"][0]["pacing"];
        assert_eq!(pacing["over_pace"], true);
        assert_eq!(pacing["used_nominal_fallback"], false);
        assert_eq!(pacing["held"][0], "codex-oauth::gpt-5.6-sol");
    }

    #[test]
    fn pace_suffix_includes_warning_only_with_a_projection() {
        assert_eq!(pace_suffix(None, true), "");
        assert_eq!(pace_suffix(Some(0.93), true), " → 93% at reset ⚠");
    }

    #[test]
    fn a_seed_only_catalog_warns_that_nothing_is_callable() {
        let models = vec!["groq::llama-3.1-8b-instant".into(), "openai::gpt-4o".into()];
        let warning = reachability(&models, |_| false)
            .warning()
            .expect("a keyless catalog must warn");
        assert!(
            warning.contains("none of these 2 models is callable"),
            "{warning}"
        );
        assert!(warning.contains("forge setup"), "{warning}");
    }

    #[test]
    fn a_partly_keyed_catalog_names_how_many_are_unusable() {
        let models = vec!["groq::llama-3.1-8b-instant".into(), "openai::gpt-4o".into()];
        let warning = reachability(&models, |m| m.starts_with("groq::"))
            .warning()
            .expect("a partly-keyed catalog must warn");
        assert!(
            warning.contains("1 of 2 listed models have no credentials"),
            "{warning}"
        );
    }

    #[test]
    fn a_fully_keyed_catalog_is_quiet() {
        let models = vec!["groq::llama-3.1-8b-instant".into()];
        assert!(reachability(&models, |_| true).warning().is_none());
        assert!(reachability(&[], |_| false).warning().is_none());
    }
}
