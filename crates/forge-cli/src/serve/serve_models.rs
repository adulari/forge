//! Provider model catalog projection for the Serve control surface.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use axum::extract::{Query, State};
use axum::response::Response;

use super::{err_response, json_response, DaemonState};

/// Serve a cached catalog this old at most before kicking off a background rediscovery.
///
/// The daemon never discovered on its own: this endpoint projected whatever snapshot the last
/// interactive `forge models` / `forge run` on the host happened to write. A phone-only user's
/// list therefore froze — a bridge alias the mesh had already started routing to
/// (`claude-cli::fable`) was invisible in the app while `forge models` on the same host listed it
/// — and past the 24 h TTL the list emptied out entirely.
const CATALOG_REFRESH_AFTER_SECS: u64 = 15 * 60;

/// Ceiling on a caller-forced (`?refresh=true`) or cold-start discovery. Same budget the
/// interactive startup path uses, plus room for the CLI-bridge probes.
const DISCOVERY_BUDGET: std::time::Duration = std::time::Duration::from_secs(20);

/// One rediscovery at a time — every remote surface polls this endpoint, and discovery talks to
/// every configured provider.
static REFRESHING: AtomicBool = AtomicBool::new(false);

#[derive(serde::Deserialize, Default)]
pub(super) struct ModelsQuery {
    /// Discover live before answering instead of projecting the cache. Additive: an older client
    /// simply omits it.
    refresh: Option<bool>,
}

#[derive(serde::Serialize)]
struct ModelsResponse {
    catalog: &'static str,
    providers: Vec<ModelProvider>,
    /// Epoch second the served catalog was discovered, so a surface can show its age instead of
    /// implying the list is live. Null when nothing has ever been discovered on this host.
    refreshed_at: Option<i64>,
    /// A background rediscovery is running; polling again shortly will return a newer catalog.
    refreshing: bool,
}

#[derive(serde::Serialize)]
struct ModelProvider {
    provider: String,
    models: Vec<ModelRow>,
    /// Set when the WHOLE provider is excluded from routing (a `__forge_provider__::<name>` health
    /// row). Without it this page showed every alias of a benched subscription as healthy: the
    /// bench map is keyed by model id, and the provider key matches no model id, so a lookup by
    /// provider prefix silently omitted it.
    excluded: Option<ModelHealth>,
}

#[derive(serde::Serialize)]
struct ModelRow {
    id: String,
    name: String,
    frontier: bool,
    free: bool,
    paid: bool,
    subscription: bool,
    estimated_cost_usd: f64,
    health: Option<ModelHealth>,
    tier: &'static str,
    benchmark_intelligence: Option<f64>,
    benchmark_coding: Option<f64>,
    context_window: Option<u32>,
}

#[derive(serde::Serialize, Clone)]
struct ModelHealth {
    until_epoch: i64,
    reason: String,
}

#[derive(Debug, PartialEq, Eq)]
enum CatalogAction {
    Serve,
    ServeAndRefresh,
    Discover,
}

/// What to do about a cached catalog `age_secs` old (`None` = nothing usable on disk).
fn catalog_action(age_secs: Option<u64>, forced: bool) -> CatalogAction {
    match age_secs {
        _ if forced => CatalogAction::Discover,
        Some(age) if age <= CATALOG_REFRESH_AFTER_SECS => CatalogAction::Serve,
        Some(_) => CatalogAction::ServeAndRefresh,
        None => CatalogAction::Discover,
    }
}

/// Discover live, persist the result, and hand back whatever we ended up with. On a timeout or an
/// empty discovery the on-disk catalog still answers — a slow provider must not blank the list.
async fn discover_now() -> Option<(forge_mesh::ModelCatalog, i64)> {
    let config = forge_config::load().unwrap_or_default();
    let discovered = tokio::time::timeout(
        DISCOVERY_BUDGET,
        crate::cli::commands::models::discover_catalog(&config),
    )
    .await
    .ok()
    .filter(|catalog| !catalog.is_empty());
    if let Some(catalog) = discovered {
        crate::cli::commands::models::save_catalog(&catalog);
    }
    crate::cli::commands::models::load_cached_catalog_aged()
        .map(|(catalog, refreshed_at, _)| (catalog, refreshed_at))
}

/// Kick off a rediscovery behind the response, unless one is already running.
fn spawn_refresh() -> bool {
    if REFRESHING
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return true;
    }
    tokio::spawn(async move {
        discover_now().await;
        REFRESHING.store(false, Ordering::SeqCst);
    });
    true
}

pub(super) async fn models_page(
    State(state): State<Arc<DaemonState>>,
    Query(params): Query<ModelsQuery>,
) -> Response {
    let cached = crate::cli::commands::models::load_cached_catalog_aged();
    let usable = cached
        .as_ref()
        .filter(|(catalog, _, _)| !catalog.is_empty())
        .map(|(_, _, age)| *age);
    let (resolved, refreshing) = match catalog_action(usable, params.refresh.unwrap_or(false)) {
        CatalogAction::Serve => (
            cached.map(|(catalog, refreshed_at, _)| (catalog, refreshed_at)),
            REFRESHING.load(Ordering::SeqCst),
        ),
        CatalogAction::ServeAndRefresh => {
            let refreshing = spawn_refresh();
            (
                cached.map(|(catalog, refreshed_at, _)| (catalog, refreshed_at)),
                refreshing,
            )
        }
        CatalogAction::Discover => (discover_now().await, REFRESHING.load(Ordering::SeqCst)),
    };
    let Some((catalog, refreshed_at)) = resolved else {
        return json_response(&ModelsResponse {
            catalog: "unavailable",
            providers: Vec::new(),
            refreshed_at: None,
            refreshing,
        });
    };

    let store = state.store.clone();
    match tokio::task::spawn_blocking(move || {
        let config = forge_config::load().unwrap_or_default();
        let fetched_prices = store.all_model_pricing().unwrap_or_default();
        let pricing =
            forge_mesh::pricing::Pricing::from_config_with_fetched(&config, fetched_prices);
        let benches: HashMap<_, _> = store
            .current_benched_report()
            .unwrap_or_default()
            .into_iter()
            .map(|(model, until_epoch, reason)| {
                (
                    model,
                    ModelHealth {
                        until_epoch,
                        reason,
                    },
                )
            })
            .collect();
        let context_windows = store.all_model_contexts().unwrap_or_default();
        let excluded: HashMap<_, _> = store
            .current_excluded_providers()
            .unwrap_or_default()
            .into_iter()
            .map(|(provider, until_epoch, reason)| {
                (
                    provider,
                    ModelHealth {
                        until_epoch,
                        reason,
                    },
                )
            })
            .collect();
        ModelsResponse {
            catalog: "available",
            refreshed_at: Some(refreshed_at),
            refreshing,
            providers: catalog
                .by_provider(&pricing)
                .into_iter()
                .map(|provider| ModelProvider {
                    excluded: excluded.get(&provider.provider).cloned(),
                    provider: provider.provider,
                    models: provider
                        .models
                        .into_iter()
                        .map(|model| ModelRow {
                            health: benches.get(&model.id).cloned(),
                            id: model.id.clone(),
                            name: model.name,
                            frontier: model.frontier,
                            free: model.free,
                            paid: model.paid,
                            subscription: model.subscription,
                            estimated_cost_usd: model.cost,
                            tier: if model.frontier {
                                "complex"
                            } else if model.subscription || model.paid {
                                "standard"
                            } else {
                                "trivial"
                            },
                            benchmark_intelligence: catalog
                                .benchmark_for(&model.id)
                                .map(|score| score.0),
                            benchmark_coding: catalog.benchmark_for(&model.id).map(|score| score.1),
                            context_window: context_windows
                                .get(&model.id)
                                .copied()
                                .or_else(|| forge_mesh::pricing::context_limit(&model.id)),
                        })
                        .collect(),
                })
                .collect(),
        }
    })
    .await
    {
        Ok(response) => json_response(&response),
        Err(_) => err_response(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "could not read model catalog",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::{catalog_action, CatalogAction, CATALOG_REFRESH_AFTER_SECS};

    #[test]
    fn a_cold_daemon_discovers_instead_of_reporting_no_catalog() {
        assert_eq!(catalog_action(None, false), CatalogAction::Discover);
    }

    #[test]
    fn an_aging_catalog_is_served_immediately_and_refreshed_behind_the_answer() {
        assert_eq!(
            catalog_action(Some(CATALOG_REFRESH_AFTER_SECS), false),
            CatalogAction::Serve
        );
        assert_eq!(
            catalog_action(Some(CATALOG_REFRESH_AFTER_SECS + 1), false),
            CatalogAction::ServeAndRefresh
        );
        // Past the 24h TTL the cache used to read as "no catalog" and the list emptied out.
        assert_eq!(
            catalog_action(Some(48 * 60 * 60), false),
            CatalogAction::ServeAndRefresh
        );
    }

    #[test]
    fn an_explicit_refresh_always_discovers() {
        assert_eq!(catalog_action(Some(0), true), CatalogAction::Discover);
        assert_eq!(catalog_action(None, true), CatalogAction::Discover);
    }

    fn model_tier(frontier: bool, subscription: bool, paid: bool) -> &'static str {
        if frontier {
            "complex"
        } else if subscription || paid {
            "standard"
        } else {
            "trivial"
        }
    }

    #[test]
    fn model_tiers_prioritize_frontier_then_paid_or_subscription() {
        assert_eq!(model_tier(true, false, false), "complex");
        assert_eq!(model_tier(false, true, false), "standard");
        assert_eq!(model_tier(false, false, true), "standard");
        assert_eq!(model_tier(false, false, false), "trivial");
    }
}
