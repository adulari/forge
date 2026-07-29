//! Provider model catalog projection for the Serve control surface.

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::State;
use axum::response::Response;

use super::{err_response, json_response, DaemonState};

#[derive(serde::Serialize)]
struct ModelsResponse {
    catalog: &'static str,
    providers: Vec<ModelProvider>,
}

#[derive(serde::Serialize)]
struct ModelProvider {
    provider: String,
    models: Vec<ModelRow>,
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

pub(super) async fn models_page(State(state): State<Arc<DaemonState>>) -> Response {
    let store = state.store.clone();
    match tokio::task::spawn_blocking(move || {
        let Some(catalog) = crate::cli::commands::models::load_cached_catalog() else {
            return ModelsResponse {
                catalog: "unavailable",
                providers: Vec::new(),
            };
        };
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
        ModelsResponse {
            catalog: "available",
            providers: catalog
                .by_provider(&pricing)
                .into_iter()
                .map(|provider| ModelProvider {
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
