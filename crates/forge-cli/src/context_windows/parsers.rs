//! Provider model-metadata parsing.

use std::collections::HashMap;

use super::native_model_id;

pub(super) fn build_basename_index(body: &serde_json::Value) -> HashMap<String, u32> {
    let Some(data) = body["data"].as_array() else {
        return HashMap::new();
    };
    let mut map: HashMap<String, u32> = HashMap::new();
    for m in data {
        let Some(id) = m["id"].as_str() else {
            continue;
        };
        let Some(window) = m["context_length"].as_u64().filter(|w| *w > 0) else {
            continue;
        };
        let basename = id.split('/').next_back().unwrap_or(id);
        let w = window.min(u32::MAX as u64) as u32;
        map.entry(basename.to_string())
            .and_modify(|e| *e = (*e).max(w))
            .or_insert(w);
    }
    map
}

/// Extract `(anthropic::<id>, window)` from Anthropic's `/v1/models` response.
///
/// The field is `max_input_tokens` ("Maximum input context window size in tokens for this model"),
/// NOT `context_window` — Anthropic's ModelInfo never had a `context_window` key. Reading the
/// wrong name made this return an empty vec for EVERY Anthropic model, so none of them ever got a
/// fetched window and they all silently fell back to `CONSERVATIVE_CONTEXT_WINDOW` — a 1M-token
/// Opus was budgeted as if it were 32k. The old unit test hid this by asserting against a
/// hand-written fixture that used the non-existent key, so it verified the parser against a shape
/// the API does not produce. `max_tokens` is the OUTPUT cap and is deliberately not read here.
/// The `> 0` filter matters: the API returns 0 when a window is unknown rather than omitting it.
pub(super) fn anthropic_windows(body: &serde_json::Value) -> Vec<(String, u32)> {
    let Some(data) = body["data"].as_array() else {
        return Vec::new();
    };
    data.iter()
        .filter_map(|m| {
            let id = m["id"].as_str()?;
            let window = m["max_input_tokens"].as_u64().filter(|w| *w > 0)?;
            Some((
                format!("anthropic::{id}"),
                window.min(u32::MAX as u64) as u32,
            ))
        })
        .collect()
}

/// Extract `(gemini::<id>, window)` from Google's `/v1beta/models` response.
pub(super) fn gemini_windows(body: &serde_json::Value) -> Vec<(String, u32)> {
    let Some(models) = body["models"].as_array() else {
        return Vec::new();
    };
    models
        .iter()
        .filter_map(|m| {
            let name = m["name"].as_str()?;
            let model_id = name.strip_prefix("models/").unwrap_or(name);
            let window = m["inputTokenLimit"].as_u64().filter(|w| *w > 0)?;
            Some((
                format!("gemini::{model_id}"),
                window.min(u32::MAX as u64) as u32,
            ))
        })
        .collect()
}

/// Extract `(<namespace>::<id>, window)` from an OpenAI-compatible `/v1/models` body. There is no
/// standard context field, so accept the established names used by hosted gateways and local
/// OpenAI-compatible servers. Numeric strings are accepted because several proxies serialize all
/// model metadata as strings.
pub(super) fn openai_compatible_windows(
    body: &serde_json::Value,
    namespace: &str,
) -> Vec<(String, u32)> {
    let Some(data) = body["data"].as_array() else {
        return Vec::new();
    };
    data.iter()
        .filter_map(|m| {
            let id = m["id"].as_str()?;
            let window = [
                "context_window",
                "context_length",
                "max_context_length",
                "max_model_len",
                "max_input_tokens",
                "inputTokenLimit",
            ]
            .iter()
            .find_map(|field| {
                m[*field]
                    .as_u64()
                    .or_else(|| m[*field].as_str().and_then(|v| v.parse().ok()))
            })
            .filter(|w| *w > 0)?;
            Some((
                format!("{namespace}::{id}"),
                window.min(u32::MAX as u64) as u32,
            ))
        })
        .collect()
}

/// Extract `(openrouter::<id>, window)` pairs from OR's `/api/v1/models` body.
pub(super) fn openrouter_windows(body: &serde_json::Value) -> Vec<(String, u32)> {
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

/// Cross-map OpenRouter model IDs to native Forge provider namespaces.
///
/// `strip_prefix` controls whether the vendor prefix is stripped from the model part:
/// - `true`  (e.g. anthropic): `anthropic/claude-opus-4-8` → `anthropic::claude-opus-4-8`
/// - `false` (e.g. nvidia): `nvidia/llama-3.1-nemotron-70b-instruct`
///   → `nvidia::nvidia/llama-3.1-nemotron-70b-instruct`
///   NVIDIA NIM returns model IDs with their vendor prefix (`nvidia/model`), so keeping the
///   full path as the model part matches the Forge catalog ID.
pub(super) fn openrouter_native_cross_map(body: &serde_json::Value) -> Vec<(String, u32)> {
    let Some(data) = body["data"].as_array() else {
        return Vec::new();
    };
    data.iter()
        .filter_map(|m| {
            let or_id = m["id"].as_str()?;
            let window = m["context_length"].as_u64().filter(|w| *w > 0)?;
            Some((native_model_id(or_id)?, window.min(u32::MAX as u64) as u32))
        })
        .collect()
}

/// Extract pricing from OR's `/api/v1/models` body.
pub(super) fn openrouter_pricing(body: &serde_json::Value) -> Vec<(String, f64, f64, Option<f64>)> {
    let Some(data) = body["data"].as_array() else {
        return Vec::new();
    };
    let per_1k = |v: &serde_json::Value| -> Option<f64> {
        let n = v
            .as_str()
            .and_then(|s| s.parse::<f64>().ok())
            .or_else(|| v.as_f64())?;
        (n.is_finite() && n >= 0.0).then_some(n * 1000.0)
    };
    data.iter()
        .filter_map(|m| {
            let id = m["id"].as_str()?;
            let pricing = &m["pricing"];
            let input = per_1k(&pricing["prompt"])?;
            let output = per_1k(&pricing["completion"])?;
            let cache_read = per_1k(&pricing["input_cache_read"]);
            Some((format!("openrouter::{id}"), input, output, cache_read))
        })
        .collect()
}

// ── HTTP helpers ─────────────────────────────────────────────────────────────────────────────────
