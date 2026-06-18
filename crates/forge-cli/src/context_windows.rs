//! Fetch per-model context windows from provider model APIs and persist them to the store, so the
//! core can trim each turn's transcript to fit the routed model's window. Without this, a long
//! conversation overflows a free model's (often 32k–128k) window and the request fails — which the
//! mesh sees as the model being "unavailable", cascading through the whole fallback chain.
//!
//! Today only OpenRouter exposes a per-model `context_length` in a key-free list endpoint. Other
//! providers fall back to forge-mesh's family heuristic (`pricing::context_limit`), then a floor.

use std::time::Duration;

const FETCH_TIMEOUT: Duration = Duration::from_secs(5);

/// Fetch OpenRouter per-model context windows and persist the ones present in `models`. Best-effort
/// and fail-soft: a network/parse error just leaves the heuristic in charge. Skips the network
/// entirely when no OpenRouter model is in the catalog.
pub async fn fetch_and_persist(models: &[String]) {
    if !models
        .iter()
        .any(|m| forge_config::provider_of(m) == "openrouter")
    {
        return;
    }
    let windows = openrouter_windows().await;
    if windows.is_empty() {
        return;
    }
    let Ok(store) = crate::open_store() else {
        return;
    };
    let wanted: std::collections::HashSet<&str> = models.iter().map(String::as_str).collect();
    for (id, w) in windows {
        if wanted.contains(id.as_str()) {
            let _ = store.set_model_context(&id, w);
        }
    }
}

/// `GET https://openrouter.ai/api/v1/models` → `{ "data": [ { "id": "vendor/model:free",
/// "context_length": 131072, … } ] }`. Returns `(openrouter::<id>, window)` pairs. The endpoint is
/// public (no key needed). Empty on any failure.
async fn openrouter_windows() -> Vec<(String, u32)> {
    let Some(body) = get_json("https://openrouter.ai/api/v1/models").await else {
        return Vec::new();
    };
    let Some(data) = body["data"].as_array() else {
        return Vec::new();
    };
    data.iter()
        .filter_map(|m| {
            let id = m["id"].as_str()?;
            let window = m["context_length"].as_u64().filter(|w| *w > 0)?;
            Some((
                format!("openrouter::{id}"),
                window.min(u32::MAX as u64) as u32,
            ))
        })
        .collect()
}

async fn get_json(url: &str) -> Option<serde_json::Value> {
    let resp = reqwest::Client::new()
        .get(url)
        .timeout(FETCH_TIMEOUT)
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        tracing::debug!("openrouter models endpoint returned {}", resp.status());
        return None;
    }
    resp.json().await.ok()
}
