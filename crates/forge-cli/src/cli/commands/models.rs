use anyhow::{Context, Result};
use std::sync::Arc;

use forge_config::ClassifierKind;
use forge_core::LlmRouter;
use forge_mesh::{HeuristicRouter, ModelCatalog, Router};
use forge_provider::{DispatchProvider, MockProvider, Provider};
use forge_store::Store;
use forge_types::TaskTier;

use crate::*;

pub(crate) fn cli_bridge_harness_enabled(config: &forge_config::Config) -> bool {
    config.mesh.bridge_mode == forge_config::BridgeMode::Harness
}

pub(crate) fn build_dispatch_provider(config: &forge_config::Config) -> DispatchProvider {
    DispatchProvider::new(cli_bridge_harness_enabled(config))
}

/// Resolve the classifier once at composition time. Classification must not recursively mesh-route
/// across whichever free models discovery happened to return on this launch: that made identical
/// input change tier when a different classifier answered first.
fn classifier_candidates(config: &forge_config::Config, mock: bool) -> Vec<String> {
    config
        .mesh
        .classifier_model
        .as_deref()
        .map(str::trim)
        .filter(|model| {
            !model.is_empty()
                && (mock || forge_config::has_api_key(forge_config::provider_of(model)))
        })
        .map(str::to_string)
        .into_iter()
        .collect()
}

/// Apply the local, privacy-preserving outcome ledger to a freshly discovered catalog.  The Mesh
/// itself enforces the sample gate and score bound; keeping this transformation at composition
/// roots makes interactive runs, `forge mesh`, API requests and daemon sessions share one score.
pub(crate) fn apply_outcome_calibration(catalog: ModelCatalog, store: &Store) -> ModelCatalog {
    catalog.with_runtime_calibration(
        store
            .model_outcome_calibration()
            .unwrap_or_default()
            .into_iter()
            .map(|row| {
                (
                    row.model,
                    forge_mesh::RuntimeCalibration {
                        samples: row.samples,
                        success_rate: row.success_rate,
                        mean_latency_ms: row.mean_latency_ms,
                    },
                )
            })
            .collect(),
    )
}

/// Maximum age of a cached catalog before it is considered stale and re-discovered.
const CATALOG_CACHE_MAX_AGE_SECS: u64 = 24 * 60 * 60;

/// Codex quota changes whenever the shared ChatGPT account is used outside Forge. A cached
/// reading older than this must never keep the mesh away from Codex; the live OAuth probe below
/// refreshes it before routing whenever possible.
pub(crate) const CODEX_QUOTA_MAX_AGE_SECS: i64 = forge_types::CODEX_QUOTA_FRESHNESS_SECS;

mod codex_quota;
pub(crate) use codex_quota::refresh_codex_quota;

mod discovery;
pub(crate) use discovery::{
    discover_catalog, invalidate_catalog_cache, load_cached_catalog, save_catalog,
};

pub(crate) fn build_provider_and_router(
    config: &forge_config::Config,
    mock: bool,
    pin: Option<String>,
    catalog: Option<forge_mesh::ModelCatalog>,
    context_windows: std::collections::HashMap<String, u32>,
    // Per-repo routing boosts learned from past `/duel` outcomes (docs/features/duel.md). Callers
    // with no store (e.g. `mcp_serve`) pass an empty map — this is a pure no-op then.
    repo_boosts: std::collections::HashMap<String, f64>,
) -> (Arc<dyn Provider>, Arc<dyn Router>) {
    let provider: Arc<dyn Provider> = if mock {
        Arc::new(MockProvider)
    } else {
        Arc::new(
            build_dispatch_provider(config)
                .with_max_output_tokens(config.mesh.effective_max_output_tokens()),
        )
    };
    let mut heuristic = HeuristicRouter::new(config.clone())
        .with_pin(pin)
        .with_context_windows(context_windows)
        .with_repo_boosts(repo_boosts);
    if let Some(cat) = catalog {
        heuristic = heuristic.with_catalog(cat);
    }
    let router: Arc<dyn Router> = if matches!(
        config.mesh.classifier,
        ClassifierKind::Llm | ClassifierKind::Hybrid
    ) {
        // A cheap model labels every unhinted tier; the deterministic router only performs
        // pin/budget/cost-aware model selection. Heuristic tier classification is reserved for
        // the final availability fallback after every LLM candidate fails.
        let classify_provider: Arc<dyn Provider> = if mock {
            Arc::new(MockProvider)
        } else {
            // classification needs no tools/harness; cap output (one tier word) so a free
            // classifier model isn't 402'd on a huge default max-token request.
            Arc::new(
                DispatchProvider::new(false)
                    .with_max_output_tokens(config.mesh.effective_max_output_tokens()),
            )
        };
        Arc::new(LlmRouter::new(
            classify_provider,
            classifier_candidates(config, mock),
            heuristic,
        ))
    } else {
        Arc::new(heuristic)
    };
    (provider, router)
}

/// Build a session around a caller-provided presenter, wiring all subsystems.
/// Discover the models the user can actually use, as a [`forge_mesh::ModelCatalog`] for
/// auto-discovery routing: query each provider that has a key (plus keyless local `ollama`) for
/// its model list, with a short per-provider timeout, and skip any that error. Providers are
/// probed concurrently so startup pays the slowest single provider's budget, not their sum.
/// Discover one provider's listable models, honoring its timeout `budget` and logging failures with
/// the right severity. Returns an empty Vec on any skip/failure/timeout so the caller can flatten
/// concurrently. A KEYED provider failing/timing out means the user configured a key but its models
/// silently vanish from routing (the mesh falls back to built-in defaults) — make that LOUD. Keyless
/// `ollama` failing just means it isn't running: debug.
/// `forge models [--probe]`: discover the usable models + show the mesh's capability-ranked pick
/// per tier. With `--probe`, also ping each model and persist health (the user-driven rescan).
pub(crate) async fn models(probe: bool, probe_all: bool, clear: bool) -> Result<()> {
    if clear {
        let store = open_store()?;
        let n = store
            .clear_all_model_health()
            .context("clearing model benches")?;
        println!("cleared {n} model bench(es) — the mesh will reconsider every model");
        return Ok(());
    }
    forge_config::inject_provider_keys();
    let config = super::load_config()?;
    let cat = discover_catalog(&config).await;
    if cat.is_empty() {
        println!(
            "no models discovered — set a provider key (`forge auth <provider>`) or run ollama"
        );
        return Ok(());
    }
    // An explicit `forge models` runs with the user's full key environment — persist what it
    // found so the daemon's /api/models (which serves this cache) reflects it immediately.
    save_catalog(&cat);
    let store = open_store()?;

    if probe {
        // Default: only re-probe the benched/excluded models (cheap — that's the whole point of a
        // recheck). `--all` pings every discovered model (costs real money on paid providers).
        let targets: Vec<String> = if probe_all {
            cat.models().to_vec()
        } else {
            let benched =
                forge_core::readiness::ProviderReadiness::snapshot(&config, &store).health;
            cat.models()
                .iter()
                .filter(|m| benched.is_benched(m))
                .cloned()
                .collect()
        };
        if targets.is_empty() {
            println!(
                "no benched models to recheck — all {} discovered models are healthy. \
                 Use `--probe --all` to force a full re-ping.",
                cat.models().len()
            );
        } else {
            if !probe_all {
                println!("rechecking {} benched model(s)…", targets.len());
            }
            probe_models(&targets, &config, &store).await?;
        }
        println!();
    }

    let pricing = forge_mesh::pricing::Pricing::from_config(&config);
    let benched = forge_core::readiness::ProviderReadiness::snapshot(&config, &store).health;
    let s = cat.stats(&pricing);
    println!(
        "{} models · {} frontier · {} free · {} subscription · {} paid · {} providers\n",
        s.total, s.frontier, s.free, s.subscription, s.paid, s.providers
    );
    for g in cat.by_provider(&pricing) {
        println!("{} ({} models)", g.provider, g.total());
        for m in &g.models {
            let name = if m.name.is_empty() {
                "(default)"
            } else {
                m.name.as_str()
            };
            let mut tags: Vec<String> = Vec::new();
            if m.subscription {
                tags.push("subscription".into());
            }
            if m.frontier {
                tags.push("frontier".into());
            }
            if m.free {
                tags.push("free".into());
            }
            if m.cost > f64::EPSILON {
                tags.push(format!("paid ~${:.4}/turn", m.cost));
            } else if m.paid {
                tags.push("paid".into());
            }
            if benched.is_benched(&m.id) {
                tags.push("benched".into());
            }
            println!("  {name:<30} {}", tags.join(" · "));
        }
    }
    println!("\nmesh auto-pick per tier:");
    for tier in [TaskTier::Trivial, TaskTier::Standard, TaskTier::Complex] {
        // Mirror routing: skip benched models so the shown pick is the one the mesh would
        // actually use right now (docs/features/mesh-routing.md).
        let pick = cat
            .ranked_for(tier, &pricing, 5)
            .into_iter()
            .find(|m| !benched.is_benched(m))
            .unwrap_or_else(|| "—".into());
        println!("  {:<9} {pick}", tier.as_str());
    }
    if !probe {
        println!(
            "\ntip: `forge models --probe` rechecks only the benched models (cheap); \
             add `--all` to re-ping every model (costs money on paid providers)."
        );
    }
    Ok(())
}

/// `forge benchmarks [--refresh]` — show measured model scores + catalog coverage (ADR-0011).
pub(crate) async fn benchmarks_cmd(refresh: bool) -> Result<()> {
    forge_config::inject_provider_keys();
    let config = super::load_config()?;
    if !config.mesh.benchmark_ranking {
        println!("benchmark ranking is disabled (`mesh.benchmark_ranking = false`).");
        return Ok(());
    }
    let cat = discover_catalog(&config).await;
    let models = cat.models().to_vec();
    let scores = benchmarks::ensure(&config, &models, refresh).await;
    let Some(scores) = scores.filter(|s| !s.is_empty()) else {
        println!(
            "no benchmark data yet. Set a free Artificial Analysis key to enable real-performance \
             ranking:\n  export ARTIFICIALANALYSIS_API_KEY=…   (or `forge auth artificialanalysis`)\n\
             then `forge benchmarks --refresh`. Until then the mesh ranks on the family heuristic."
        );
        return Ok(());
    };
    let (covered, total) = cat.benchmark_coverage();
    println!(
        "{} models scored · {covered}/{total} catalog models matched\n",
        scores.len()
    );
    let mut rows: Vec<(String, Option<forge_mesh::BenchScore>)> = cat
        .models()
        .iter()
        .filter(|m| forge_mesh::catalog::is_routable(m))
        .map(|m| (m.clone(), scores.score_for(m)))
        .collect();
    // Scored first (by intelligence desc), then the unmatched (heuristic fallback).
    rows.sort_by(|a, b| match (a.1, b.1) {
        (Some(x), Some(y)) => y.intelligence.total_cmp(&x.intelligence),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.0.cmp(&b.0),
    });
    for (id, score) in rows {
        match score {
            Some(s) => println!(
                "  {:<40} intelligence {:>5.1}  coding {:>5.1}",
                id, s.intelligence, s.coding
            ),
            None => println!("  {:<40} —  (heuristic fallback)", id),
        }
    }
    Ok(())
}

/// `forge mesh [PROMPT]` — explain how the mesh routes. With a prompt: the full decision trace.
/// Without one: the per-tier picks + subscription-quota overview. The non-interactive sibling of
/// the `/mesh` TUI inspector; both read the same [`forge_mesh::RoutingExplanation`] engine.
#[derive(Debug)]
struct MeshSmokeRow {
    tier: TaskTier,
    model: String,
    fallbacks: usize,
    viable: bool,
    detail: String,
}

/// Exercise the real mesh-selection path for every task tier without dispatching a model. This is
/// deliberately safe to run often: discovery/quota freshness happen in the caller, but these
/// routes are local and classifier-free because each tier is explicitly hinted.
async fn mesh_smoke_rows(
    router: &HeuristicRouter,
    budget: forge_mesh::BudgetState,
    health: &forge_types::ModelHealth,
    quota: &forge_types::SubscriptionQuota,
) -> Vec<MeshSmokeRow> {
    let project = forge_types::ProjectContext::default();
    let cases = [
        (TaskTier::Trivial, "fix a typo"),
        (TaskTier::Standard, "add a small endpoint with tests"),
        (
            TaskTier::Complex,
            "design and prove a safe concurrent refactor across modules",
        ),
    ];
    let mut rows = Vec::with_capacity(cases.len());
    for (tier, prompt) in cases {
        let decision = router
            .route_hinted(
                prompt,
                false,
                budget,
                health,
                quota,
                Some(tier),
                None,
                &project,
            )
            .await;
        let viable = decision.model != "unknown" && !decision.rationale.contains("no usable key");
        rows.push(MeshSmokeRow {
            tier,
            model: decision.model,
            fallbacks: decision.fallbacks.len(),
            viable,
            detail: decision.rationale,
        });
    }
    rows
}

fn print_mesh_smoke(rows: &[MeshSmokeRow], json: bool) {
    if json {
        let rows: Vec<_> = rows
            .iter()
            .map(|row| {
                serde_json::json!({
                    "tier": row.tier.as_str(),
                    "model": row.model,
                    "fallbacks": row.fallbacks,
                    "viable": row.viable,
                    "detail": row.detail,
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::json!({
                "kind": "mesh-readiness",
                "selection_only": true,
                "ready": rows.iter().all(|row| row["viable"] == true),
                "tiers": rows,
            })
        );
        return;
    }

    println!("⚒ mesh readiness — selection-only; no model requests\n");
    for row in rows {
        let mark = if row.viable { "✓" } else { "✗" };
        let fallback_label = match row.fallbacks {
            0 => "no fallback".to_string(),
            1 => "1 fallback".to_string(),
            n => format!("{n} fallbacks"),
        };
        println!(
            "  {mark} {:<8} {:<42} {fallback_label}",
            row.tier.as_str(),
            row.model
        );
        if !row.viable {
            println!("      → {}", row.detail);
        }
    }
    if rows.iter().all(|row| row.viable) {
        println!("\nMesh is ready for every task tier.");
    } else {
        println!(
            "\nMesh has no viable route for one or more tiers — run `forge doctor` for fixes."
        );
    }
}

pub(crate) async fn mesh_explain(prompt: String, json: bool, smoke: bool) -> Result<()> {
    forge_config::inject_provider_keys();
    let config = super::load_config()?;
    let cat = discover_catalog(&config).await;
    if cat.is_empty() {
        println!(
            "no models discovered — set a provider key (`forge auth <provider>`) or run ollama"
        );
        return Ok(());
    }
    let store = open_store()?;
    // Codex prefers a fresh account-wide OAuth header reading; a fresh CLI rollout is the
    // no-cost fallback. Expired readings are never allowed to bias this route.
    refresh_codex_quota(&store).await;
    // `/mesh` must score exactly like a real session: static benchmark data remains dominant,
    // while sufficiently broad local outcome evidence provides a small quality/latency tie-break.
    let cat = apply_outcome_calibration(cat, &store);
    if store
        .subscription_age_secs("claude-cli")
        .is_none_or(|a| a > 300)
    {
        let limits = tokio::task::spawn_blocking(bridge_stats::probe_claude_limits)
            .await
            .unwrap_or_default();
        for (window, frac) in limits {
            // Live probe — its observation time genuinely is now.
            seed_store_quota(&store, "claude-cli", &window, Some(frac * 100.0), None);
        }
    }
    let readiness = forge_core::readiness::ProviderReadiness::snapshot(&config, &store);
    let quota = readiness.quota;
    let health = readiness.health;
    let budget = forge_mesh::BudgetState {
        spent_today_usd: store.spend_today_usd().unwrap_or(0.0),
        daily_cap_usd: config.mesh.daily_budget_usd,
        spent_week_usd: store.spend_this_week_usd().unwrap_or(0.0),
        weekly_cap_usd: config.mesh.weekly_budget_usd,
        spent_month_usd: store.spend_this_month_usd().unwrap_or(0.0),
        monthly_cap_usd: config.mesh.monthly_cap_usd,
        warn_fraction: config.mesh.warn_threshold,
        min_context_tokens: None,
    };
    let router = HeuristicRouter::new(config.clone()).with_catalog(cat.clone());

    if smoke {
        if !prompt.trim().is_empty() {
            anyhow::bail!("`forge mesh --smoke` does not take a prompt");
        }
        let rows = mesh_smoke_rows(&router, budget, &health, &quota).await;
        print_mesh_smoke(&rows, json);
        if rows.iter().all(|row| row.viable) {
            return Ok(());
        }
        anyhow::bail!("mesh readiness check failed");
    }

    if prompt.trim().is_empty() {
        if json {
            println!("{}", mesh_overview_json(&cat, &config, &quota));
        } else {
            mesh_overview(&cat, &config, &quota);
        }
        return Ok(());
    }
    let project = std::env::current_dir()
        .map(|cwd| forge_core::project_context::compute(&cwd))
        .unwrap_or_default();
    let e = match config.mesh.classifier {
        ClassifierKind::Heuristic => {
            router.explain(&prompt, budget, &health, &quota, None, &project)
        }
        ClassifierKind::Llm | ClassifierKind::Hybrid => {
            // `forge mesh <prompt>` must describe the same LLM classification that a real turn
            // uses, rather than rendering a heuristic preview that can disagree with routing.
            let classifier: Arc<dyn Router> = Arc::new(LlmRouter::new(
                Arc::new(
                    DispatchProvider::new(false)
                        .with_max_output_tokens(config.mesh.effective_max_output_tokens()),
                ),
                classifier_candidates(&config, false),
                HeuristicRouter::new(config.clone()).with_catalog(cat.clone()),
            ));
            let decision = classifier
                .route(&prompt, false, budget, &health, &quota, None, &project)
                .await;
            let fallback = decision.rationale.contains("llm classify unavailable");
            // `decide` appends the model-selection reason after an em dash. Keep only the
            // classifier portion here: `explain_classified` recomputes the same selection once,
            // avoiding duplicate “auto-selected …” text in the inspector.
            let classifier_reason = decision
                .rationale
                .split(" — ")
                .next()
                .unwrap_or(&decision.rationale)
                .to_string();
            let mut explained = router.explain_classified(
                &prompt,
                decision.tier,
                vec![classifier_reason],
                budget,
                &health,
                &quota,
                None,
            );
            explained.classifier_label = if fallback {
                "heuristic fallback (all LLM candidates unavailable)".to_string()
            } else {
                "llm".to_string()
            };
            explained
        }
    };
    if json {
        println!("{}", mesh_explanation_json(&e));
    } else {
        print_mesh_explanation(&e);
    }
    Ok(())
}

/// Record a subscription window fraction (0–100 pct) into the store, mapping it to a status. Used
/// to seed the mesh quota from the Claude/Codex rate-limit caches in the `forge mesh` CLI path.
///
/// `observed_at` is when the reading was actually OBSERVED (rollout line timestamp / file mtime)
/// — pass it for cache-derived readings so a re-seeded old observation can't mask a fresher one
/// (`Store::record_quota_at`'s stale guard). `None` means "observed now" (live probes).
pub(crate) fn seed_store_quota(
    store: &Store,
    provider: &str,
    window: &str,
    pct: Option<f64>,
    observed_at: Option<i64>,
) {
    let Some(pct) = pct else { return };
    let frac = (pct / 100.0).clamp(0.0, 1.0);
    let status = if frac >= 0.98 {
        forge_types::QuotaStatus::Exhausted
    } else if frac >= 0.80 {
        forge_types::QuotaStatus::Warning
    } else {
        forge_types::QuotaStatus::Ok
    };
    let hint = forge_types::QuotaHint {
        provider: provider.to_string(),
        window: window.to_string(),
        status,
        resets_at: None,
        fraction_used: Some(frac),
    };
    let _ = match observed_at {
        Some(ts) => store.record_quota_at(&hint, ts),
        None => store.record_quota(&hint),
    };
}

mod presentation;
pub(crate) use presentation::{
    mesh_explanation_json, mesh_overview, mesh_overview_json, print_mesh_explanation,
};

/// ones that rate-limit / fail auth / are down, so the mesh routes around them.
pub(crate) async fn probe_models(
    targets: &[String],
    config: &forge_config::Config,
    store: &Store,
) -> Result<()> {
    use std::time::Duration;
    let harness = config.mesh.bridge_mode == forge_config::BridgeMode::Harness;
    let provider = DispatchProvider::new(harness)
        .with_max_output_tokens(config.mesh.effective_max_output_tokens());
    let default_cooldown = Duration::from_secs(config.mesh.failover_cooldown_secs);
    let ping = [forge_types::Message::user("ping")];
    // Probe WITH a representative tool: the real agent loop always advertises tools, so a model
    // that can't do function calling (groq compound-mini, many OpenRouter models) must fail the
    // probe too — a no-tool ping would falsely pass it. This is what *confirms* a model (incl. any
    // marked "free") can actually serve a turn, not just answer a bare prompt.
    let probe_tool = [forge_provider::ToolSpec {
        name: "noop".to_string(),
        description: "A no-op used to verify the model accepts tool calls.".to_string(),
        schema: serde_json::json!({"type": "object", "properties": {}}),
    }];
    let mut sink = |_: forge_provider::StreamEvent| {};
    let mut auth_failed_providers = std::collections::HashSet::new();

    println!("probing {} model(s)…", targets.len());
    for m in targets {
        let provider_name = forge_config::provider_of(m);
        if auth_failed_providers.contains(provider_name) {
            println!("  ↷ {m} — provider auth already failed (skipped)");
            continue;
        }
        let res = tokio::time::timeout(
            Duration::from_secs(20),
            provider.complete(m, &ping, &probe_tool, &mut sink),
        )
        .await;
        match res {
            Ok(Ok(_)) => {
                store.clear_model_health(m).ok();
                store.clear_provider_health(provider_name).ok();
                println!("  ✓ {m}");
            }
            // A bad credential invalidates every alias for this provider. Persist a provider-wide
            // exclusion and stop probing its siblings; a later successful probe clears it.
            Ok(Err(e)) if e.is_auth() => {
                auth_failed_providers.insert(provider_name.to_string());
                if let Err(err) = store.exclude_provider(provider_name, e.reason()) {
                    eprintln!("  ⚠ {m}: provider exclusion not persisted: {err}");
                }
                println!(
                    "  ⊘ {provider_name} — {} (all aliases excluded)",
                    e.reason()
                );
            }
            // A PERMANENT incapability (no tool support / unaffordable) → exclude for a long window
            // so discovery stops resurrecting it every run.
            Ok(Err(e)) if e.is_permanent() => {
                if let Err(err) = store.exclude_model(m, e.reason()) {
                    eprintln!("  ⚠ {m}: exclusion not persisted: {err}");
                }
                println!("  ⊘ {m} — {} (excluded)", e.reason());
            }
            Ok(Err(e)) if e.is_retryable() => {
                let cooldown = e.cooldown(default_cooldown);
                if let Err(err) = store.bench_for(m, cooldown, e.reason()) {
                    eprintln!("  ⚠ {m}: benching not persisted: {err}");
                }
                println!("  ✗ {m} — {} (benched {}s)", e.reason(), cooldown.as_secs());
            }
            Ok(Err(e)) => {
                // Non-retryable (e.g. the ping payload upset the model) → don't bench it.
                println!("  ? {m} — {} (not benched)", e.reason());
            }
            Err(_) => {
                if let Err(err) = store.bench_for(m, default_cooldown, "probe timeout") {
                    eprintln!("  ⚠ {m}: benching not persisted: {err}");
                }
                println!(
                    "  ✗ {m} — timeout (benched {}s)",
                    default_cooldown.as_secs()
                );
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod bridge_harness_tests {
    use super::*;

    #[test]
    fn default_classifier_uses_one_fixed_capable_model() {
        assert_eq!(
            classifier_candidates(&forge_config::Config::default(), true),
            ["groq::llama-3.3-70b-versatile"]
        );
    }

    #[test]
    fn serve_dispatch_provider_uses_configured_bridge_mode() {
        let mut config = forge_config::Config::default();
        config.mesh.bridge_mode = forge_config::BridgeMode::Harness;
        assert!(build_dispatch_provider(&config).harness_enabled());
        config.mesh.bridge_mode = forge_config::BridgeMode::Text;
        assert!(!build_dispatch_provider(&config).harness_enabled());
    }

    #[tokio::test]
    async fn mesh_smoke_has_a_viable_route_for_every_tier() {
        // A smoke run never calls a model. It exercises the actual selection seam, forcing each
        // task tier so an authenticated user can prove their current catalog is routeable before
        // they start a real task.
        let router = HeuristicRouter::new(forge_config::Config::default())
            .with_catalog(ModelCatalog::new(vec!["ollama::llama3.2".into()]));
        let rows = mesh_smoke_rows(
            &router,
            forge_mesh::BudgetState::default(),
            &forge_types::ModelHealth::default(),
            &forge_types::SubscriptionQuota::default(),
        )
        .await;

        assert_eq!(rows.len(), 3);
        assert!(rows.iter().all(|row| row.viable), "rows: {rows:?}");
        assert_eq!(
            rows.iter().map(|row| row.tier).collect::<Vec<_>>(),
            vec![TaskTier::Trivial, TaskTier::Standard, TaskTier::Complex]
        );
    }
}
