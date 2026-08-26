//! Finding out which models are actually usable, and remembering the answer.
//!
//! Discovery asks every configured provider what it offers, under a per-provider time budget so a
//! slow or unreachable one cannot stall a launch, and the result is cached on disk because paying
//! that cost on every command would be absurd. The cache is deliberately conservative: it expires,
//! and anything that changes which models exist (a new key, a provider edit) invalidates it
//! explicitly rather than waiting for the clock.

use forge_mesh::ModelCatalog;

use super::CATALOG_CACHE_MAX_AGE_SECS;
use crate::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DiscoveryStatusKind {
    Discovered,
    Failed,
    TimedOut,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderDiscoveryStatus {
    pub provider: String,
    pub kind: DiscoveryStatusKind,
    pub models: usize,
    pub detail: Option<String>,
}

/// Path to the on-disk catalog cache (`~/.local/share/forge/catalog.json`).
fn catalog_cache_path() -> Option<std::path::PathBuf> {
    forge_config::data_dir().map(|d| d.join("catalog.json"))
}

/// Load the on-disk catalog if it exists and is fresh (< 24 h old).
pub(crate) fn load_cached_catalog() -> Option<ModelCatalog> {
    let path = catalog_cache_path()?;
    let meta = std::fs::metadata(&path).ok()?;
    let age = meta.modified().ok()?.elapsed().ok()?;
    if age.as_secs() > CATALOG_CACHE_MAX_AGE_SECS {
        return None;
    }
    let bytes = std::fs::read(&path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Persist `catalog` to disk for the next startup to load instantly.
///
/// Clobber guard: a refresh that lost most of the catalog — a daemon spawned without the
/// provider env keys in scope, an offline moment, several providers timing out at once —
/// must not overwrite a healthy recent cache with its degraded view. That exact failure
/// shrank a 259-model cache to 21 and every surface's model list with it. A genuinely
/// smaller catalog (keys removed on purpose) still wins once the cache ages past its TTL.
pub(crate) fn save_catalog(catalog: &ModelCatalog) {
    if let Some(existing) = load_cached_catalog() {
        if catalog.models().len() * 2 < existing.models().len() {
            tracing::warn!(
                "model discovery found {} models but the cache holds {} — keeping the \
                 cache (degraded discovery: missing keys / offline / provider timeouts?)",
                catalog.models().len(),
                existing.models().len()
            );
            return;
        }
    }
    let Some(path) = catalog_cache_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_vec(catalog) {
        let _ = std::fs::write(&path, json);
    }
}

/// Delete the on-disk catalog cache so the next lookup re-discovers from scratch. A new
/// subscription login must surface its models without waiting for the 24h cache to age out.
pub(crate) fn invalidate_catalog_cache() {
    let Some(path) = catalog_cache_path() else {
        return;
    };
    let _ = std::fs::remove_file(path);
}

/// Construct the model backend + router from config. Shared by interactive sessions and the
/// `mcp-serve` subagent path (RFC subagent-orchestration Phase 3), so both route identically.
async fn discover_provider_models(
    p: &str,
    budget: std::time::Duration,
) -> (Vec<String>, ProviderDiscoveryStatus) {
    let keyed = p != "ollama";
    // Some keyed providers are completion-only — they answer turns fine (via the custom
    // service-target resolver) but have no model-LISTING API, so auto-discovery can't enumerate
    // them. That's expected, not a key/network failure: skip them quietly with accurate guidance
    // (configure their models explicitly) instead of a scary "discovery failed — check your key".
    if keyed && !forge_provider::is_discoverable(p) {
        tracing::debug!(
            "'{p}' has no model-listing API — it's completion-only; pin a `{p}::<model>` id \
             (or add it under [mesh.models]) to route it. (Not a key/network problem.)"
        );
        return (
            Vec::new(),
            ProviderDiscoveryStatus {
                provider: p.to_string(),
                kind: DiscoveryStatusKind::Unsupported,
                models: 0,
                detail: Some("completion-only; configure a model explicitly".into()),
            },
        );
    }
    match tokio::time::timeout(budget, forge_provider::list_models(p)).await {
        Ok(Ok(list)) => {
            let count = list.len();
            (
                list,
                ProviderDiscoveryStatus {
                    provider: p.to_string(),
                    kind: DiscoveryStatusKind::Discovered,
                    models: count,
                    detail: None,
                },
            )
        }
        Ok(Err(e)) if keyed => {
            tracing::warn!(
                "model discovery FAILED for keyed provider '{p}': {e} — its models won't be routable this session (check the key / network)"
            );
            (
                Vec::new(),
                ProviderDiscoveryStatus {
                    provider: p.to_string(),
                    kind: DiscoveryStatusKind::Failed,
                    models: 0,
                    detail: Some(e.to_string()),
                },
            )
        }
        Ok(Err(e)) => {
            tracing::debug!("model discovery skipped {p}: {e}");
            (
                Vec::new(),
                ProviderDiscoveryStatus {
                    provider: p.to_string(),
                    kind: DiscoveryStatusKind::Failed,
                    models: 0,
                    detail: Some(e.to_string()),
                },
            )
        }
        Err(_) if keyed => {
            tracing::warn!(
                "model discovery TIMED OUT for keyed provider '{p}' after {}s — its models won't be routable this session",
                budget.as_secs()
            );
            (
                Vec::new(),
                ProviderDiscoveryStatus {
                    provider: p.to_string(),
                    kind: DiscoveryStatusKind::TimedOut,
                    models: 0,
                    detail: Some(format!("after {}s", budget.as_secs())),
                },
            )
        }
        Err(_) => {
            tracing::debug!("model discovery timed out for {p}");
            (
                Vec::new(),
                ProviderDiscoveryStatus {
                    provider: p.to_string(),
                    kind: DiscoveryStatusKind::TimedOut,
                    models: 0,
                    detail: Some(format!("after {}s", budget.as_secs())),
                },
            )
        }
    }
}

pub(crate) async fn discover_catalog_with_status(
    config: &forge_config::Config,
) -> (forge_mesh::ModelCatalog, Vec<ProviderDiscoveryStatus>) {
    use std::time::Duration;
    let mut models = Vec::new();
    // Keyless local first, then every key-holding provider.
    let mut providers = vec!["ollama".to_string()];
    providers.extend(
        forge_config::known_key_providers()
            .filter(|p| forge_config::has_api_key(p))
            .map(str::to_string),
    );
    // Probe every provider CONCURRENTLY: each `list_models` is an independent network call to a
    // different endpoint, so a sequential loop made startup pay the SUM of every provider's budget
    // (3 keyed providers × 8s ≈ 24s worst case). `join_all` makes it the MAX instead (~8s), the same
    // pattern `drop_unaffordable_models` already uses. Results are flattened in provider order so the
    // catalog stays deterministic (dedup below relies on a stable first-seen order).
    let probes = providers.iter().map(|p| {
        discover_provider_models(p, Duration::from_secs(if p != "ollama" { 8 } else { 4 }))
    });
    let mut statuses = Vec::new();
    for (list, status) in futures::future::join_all(probes).await {
        models.extend(list);
        statuses.push(status);
    }
    // Custom OpenAI-compatible providers (NVIDIA NIM, SambaNova, Mistral, Cerebras, …) have no genai
    // SDK adapter, so the genai probe above skips them — but they DO expose an OpenAI `/v1/models`
    // endpoint. List them LIVE (the full catalog the key can reach) so EVERY model is visible, not a
    // hand-seeded few; fall back to the curated seed ids only if the live call fails (offline /
    // endpoint down). Generic over the registry — future providers need no code here. Probed
    // concurrently with an 8s budget each, like the genai providers above.
    let custom: Vec<_> = forge_config::custom_providers()
        .filter(|cp| forge_config::has_api_key(cp.namespace))
        .collect();
    let custom_lists = futures::future::join_all(custom.iter().map(|cp| async move {
        let seeds = || {
            cp.seed_models
                .iter()
                .map(|m| format!("{}::{}", cp.namespace, m))
                .collect::<Vec<_>>()
        };
        match tokio::time::timeout(
            Duration::from_secs(8),
            forge_provider::list_custom_models(cp.namespace),
        )
        .await
        {
            Ok(Ok(list)) if !list.is_empty() => {
                let count = list.len();
                (
                    list,
                    ProviderDiscoveryStatus {
                        provider: cp.namespace.to_string(),
                        kind: DiscoveryStatusKind::Discovered,
                        models: count,
                        detail: None,
                    },
                )
            }
            Ok(Err(e)) => {
                tracing::debug!(
                    "{} live model list failed: {e} — using seed ids",
                    cp.namespace
                );
                let list = seeds();
                (
                    list.clone(),
                    ProviderDiscoveryStatus {
                        provider: cp.namespace.to_string(),
                        kind: DiscoveryStatusKind::Failed,
                        models: list.len(),
                        detail: Some(format!("using configured seed models: {e}")),
                    },
                )
            }
            Err(_) => {
                let list = seeds();
                (
                    list.clone(),
                    ProviderDiscoveryStatus {
                        provider: cp.namespace.to_string(),
                        kind: DiscoveryStatusKind::TimedOut,
                        models: list.len(),
                        detail: Some("using configured seed models after 8s".into()),
                    },
                )
            }
            Ok(Ok(_)) => {
                let list = seeds();
                (
                    list.clone(),
                    ProviderDiscoveryStatus {
                        provider: cp.namespace.to_string(),
                        kind: DiscoveryStatusKind::Failed,
                        models: list.len(),
                        detail: Some(
                            "live endpoint returned no models; using configured seed models".into(),
                        ),
                    },
                )
            }
        }
    }))
    .await;
    for (list, status) in custom_lists {
        models.extend(list);
        statuses.push(status);
    }
    // Azure OpenAI: deployments are configured (`[providers.azure]`), not enumerable via an API in our
    // flow, so seed each `azure::<deployment>` when a key is present. Routing reaches them through the
    // genai per-request override (deployment URL + api-key header).
    if forge_config::has_api_key("azure") {
        if let Some(az) = forge_config::azure_provider() {
            models.extend(az.deployments.iter().map(|d| format!("azure::{d}")));
        }
    }
    // xAI OAuth (SuperGrok/X Premium subscription, `forge auth xai-oauth`): only worth probing if
    // a session is actually stored — skips a needless network call/timeout for the vast majority
    // of users who never signed in. `list_xai_oauth_models` itself falls back to a small seed list
    // on any live-listing failure, so this can't leave the catalog empty on a blip.
    if forge_provider::has_xai_oauth_session() {
        match tokio::time::timeout(
            Duration::from_secs(8),
            forge_provider::list_xai_oauth_models(),
        )
        .await
        {
            Ok(Ok(list)) => models.extend(list),
            Ok(Err(e)) => tracing::debug!("xai-oauth model discovery failed: {e}"),
            Err(_) => tracing::debug!("xai-oauth model discovery timed out"),
        }
    }
    // ChatGPT subscription OAuth (`forge auth codex-oauth`): seed models when a session is stored.
    if forge_provider::has_codex_oauth_session() {
        match tokio::time::timeout(
            Duration::from_secs(8),
            forge_provider::list_codex_oauth_models(),
        )
        .await
        {
            Ok(Ok(list)) => models.extend(list),
            Ok(Err(e)) => tracing::debug!("codex-oauth model discovery failed: {e}"),
            Err(_) => tracing::debug!("codex-oauth model discovery timed out"),
        }
    }
    // Always-available subscription bridges (claude-cli/codex-cli) if their CLI is installed.
    // They don't rate-limit like the free API tiers, so the mesh can rely on them — and being
    // $0 subscriptions they rank first (prefer_subscription), so routing reaches a working model
    // instead of erroring out when metered providers are throttled. Each installed bridge
    // contributes one id per model alias — config override, else whatever the CLI itself
    // advertises (`claude --help` / `agy models`, probed concurrently), else the built-in
    // fallback table — so the mesh can size each turn (haiku/mini ↔ opus) and a model newly
    // shipped to subscribers appears without a Forge release. The bare default id
    // (`claude-cli::`) is NOT cataloged: it's a valid manual pin for the CLI's own default, but
    // as a catalog row it's empty-named and can never match a benchmark. A stale alias just
    // benches itself via failover — never a hard error.
    let bridge_lists = futures::future::join_all(
        forge_provider::CliKind::all()
            .into_iter()
            .filter(|k| k.available())
            .map(|k| async move {
                let prefix = k.prefix();
                let aliases = match config.mesh.bridge_models.get(prefix) {
                    Some(custom) if !custom.is_empty() => custom.clone(),
                    _ => k.bridge_models().await,
                };
                aliases
                    .into_iter()
                    .filter(|m| !m.is_empty())
                    .map(|m| format!("{prefix}::{m}"))
                    .collect::<Vec<_>>()
            }),
    )
    .await;
    for list in bridge_lists {
        models.extend(list);
    }
    // Keep the configured tier candidates as a cold-start safety net. A provider's model-list
    // endpoint can be unavailable while its completion endpoint still works (the doctor output
    // calls this out); without these seeds, a transient listing failure silently removed that
    // provider from an otherwise healthy auto-discovery mesh. Key/health/credit checks still gate
    // actual routing, and successfully discovered models retain their normal ranked preference.
    for tier in [TaskTier::Trivial, TaskTier::Standard, TaskTier::Complex] {
        models.extend(config.candidates_for(tier));
    }
    // Dedup while preserving discovery order (a provider could list the same id twice).
    let mut seen = std::collections::HashSet::new();
    models.retain(|m| seen.insert(m.clone()));
    // Drop NON-chat models (image/video/audio generation, embeddings, reranking, OCR, moderation):
    // they can't serve chat completions, so routing them only churns failover, and they never get a
    // chat-intelligence benchmark (showing as a heuristic "—"). Applies to EVERY source — genai
    // `list_models`, OpenRouter, the custom `/v1/models` listers — so e.g. gemini imagen/veo,
    // mistral voxtral/ocr, and groq orpheus never enter the catalog.
    models.retain(|m| !forge_config::is_non_chat_model(m));
    // Drop any model/provider the user disabled (`[mesh] disabled`), so the mesh never routes to
    // or fails over onto it (known-issues.md: disable a flaky model without deleting its key).
    models.retain(|m| !forge_config::is_model_disabled(m, &config.mesh.disabled));
    // Pre-flight balance: for each provider that exposes a key-authenticated balance API, drop its
    // PAID models when the account is out of credit — so the mesh never tries (and 402s on) a model
    // it can't pay for (e.g. OpenRouter at $0 balance). Free variants + providers without a balance
    // API are untouched (fail open). Probes run concurrently across providers; each is short-timed.
    drop_unaffordable_models(&mut models).await;
    // Fetch + persist real per-model context windows (OpenRouter exposes `context_length`) so the
    // core can trim each turn to the routed model's window instead of overflowing it. Best-effort;
    // the family heuristic covers everything else.
    context_windows::fetch_and_persist(&models).await;
    // Attach measured benchmark scores (ADR-0011) so the mesh ranks on real performance. Cache-
    // first + incremental: only hits the API when a newly-discovered model has no rating yet.
    let bench = benchmarks::ensure(config, &models, false).await;
    (
        forge_mesh::ModelCatalog::new(models).with_benchmarks(bench),
        statuses,
    )
}

pub(crate) async fn discover_catalog(config: &forge_config::Config) -> forge_mesh::ModelCatalog {
    discover_catalog_with_status(config).await.0
}

/// Remove a provider's metered models from `models` when its account balance is confirmed below
/// [`balance::MIN_CREDIT_USD`]. Only providers exposing a key-authenticated balance API are probed
/// (others return `None` → kept); genuinely-free variants (e.g. OpenRouter `:free`) are kept too.
pub(crate) async fn drop_unaffordable_models(models: &mut Vec<String>) {
    let mut providers: Vec<String> = models
        .iter()
        .map(|m| forge_config::provider_of(m).to_string())
        .filter(|p| !p.is_empty())
        .collect();
    providers.sort();
    providers.dedup();

    // Probe every provider concurrently; collect the ones confirmed broke.
    let checks = providers.into_iter().map(|p| async move {
        match balance::remaining_credit(&p).await {
            Some(bal) if bal < balance::MIN_CREDIT_USD => Some((p, bal)),
            _ => None,
        }
    });
    let broke: Vec<(String, f64)> = futures::future::join_all(checks)
        .await
        .into_iter()
        .flatten()
        .collect();

    for (p, bal) in broke {
        let before = models.len();
        models.retain(|m| forge_config::provider_of(m) != p || balance::is_free_model_id(m));
        let dropped = before - models.len();
        if dropped > 0 {
            tracing::info!(
                "{p} balance {bal:.2} < {:.2} — dropped {dropped} paid model(s) from discovery (free variants kept)",
                balance::MIN_CREDIT_USD
            );
        }
    }
}
