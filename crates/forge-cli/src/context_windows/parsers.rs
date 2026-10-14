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

/// models.dev provider keys → Forge namespaces that bill that exact model id. OpenRouter carries
/// no `openai/gpt-5.6*` entries at all, so without this source the whole GPT-5.6 line priced as
/// $0 and only a hardcoded (stale) ladder ordered codex-cli / codex-oauth decisions. The CLI
/// bridges bill the same model as the API, so they get the same row.
///
/// A provider is listed only when models.dev's ids match Forge's for that namespace, measured
/// against a real catalog rather than assumed. `opencode-go` is deliberately ABSENT: models.dev
/// prices 34 models under it and not one id matches an `opencode_go::` id Forge routes to, so
/// mapping it would add rows that can never be looked up while implying the gateway is priced.
const MODELS_DEV_NAMESPACES: &[(&str, &[&str])] = &[
    ("openai", &["openai", "codex-cli", "codex-oauth"]),
    ("anthropic", &["anthropic"]),
    ("google", &["gemini"]),
    ("xai", &["xai", "xai-oauth"]),
    ("deepseek", &["deepseek"]),
    ("mistral", &["mistral"]),
    ("groq", &["groq"]),
    // Meta's own API. Every one of its five priced ids matches a `meta::` id in the catalog, and
    // without this row the whole Muse line — including the metered `-contributor` models people
    // actually run — billed as $0 and the statusline read "untracked" while real money was spent.
    ("meta", &["meta"]),
    // OpenCode Zen. 66 of its 97 priced ids match. It also publishes EXPLICIT zeros for the
    // `-free` tiers, which is what turns a "we have no idea" into a provable "free".
    ("opencode", &["opencode"]),
    ("nvidia", &["nvidia"]),
    ("cerebras", &["cerebras"]),
];

/// Extract pricing from models.dev's `api.json` (`{provider: {models: {id: {cost: {input,
/// output, cache_read}}}}}`, USD per 1M tokens) as `(namespace::id, in_1k, out_1k, cache_1k)`.
/// Missing or non-numeric costs are skipped, never written as $0.
pub(super) fn models_dev_pricing(body: &serde_json::Value) -> Vec<(String, f64, f64, Option<f64>)> {
    let per_1k = |v: &serde_json::Value| -> Option<f64> {
        let n = v.as_f64()?;
        (n.is_finite() && n >= 0.0).then_some(n / 1000.0)
    };
    let mut out = Vec::new();
    for (provider, namespaces) in MODELS_DEV_NAMESPACES {
        let Some(models) = body[provider]["models"].as_object() else {
            continue;
        };
        for (id, model) in models {
            let cost = &model["cost"];
            let (Some(input), Some(output)) = (per_1k(&cost["input"]), per_1k(&cost["output"]))
            else {
                continue;
            };
            let cache_read = per_1k(&cost["cache_read"]);
            for ns in namespaces.iter() {
                out.push((format!("{ns}::{id}"), input, output, cache_read));
            }
        }
    }
    out
}

/// Extract prices from a MARKETPLACE-shaped OpenAI-compatible `/models` body, where each entry
/// carries `pricing: {official_in, official_out, asks_in: [...], asks_out: [...]}` in USD per 1M
/// tokens (InferHub). Returns `(namespace::id, in_1k, out_1k, None)`.
///
/// One rate per model is chosen by `stat` over the positive asks, falling back to the vendor's
/// official price when a model has no asks (or `stat` is `official`). The cheapest ask was the
/// old choice and it understates spend: the gateway may route a request to any seller.
///
/// An unprefixed alias that lists no pricing of its own (`claude-opus-5.5`) is priced from the
/// asks of every routed entry whose last path segment names the same model (`cc/claude-opus-5-5`,
/// `kiro/claude-opus-5.5`), compared case-insensitively with `.` and `-` treated alike. A model
/// with no evidence at all is left unpriced — never written as $0, which would read as free. No
/// cache-read rate is produced: InferHub publishes none and documents no discount, so cached
/// tokens bill at the input rate until the response's own `usage.cost` says otherwise.
pub(super) fn marketplace_pricing(
    body: &serde_json::Value,
    namespace: &str,
    stat: forge_config::PricingStat,
) -> Vec<(String, f64, f64, Option<f64>)> {
    let Some(data) = body["data"].as_array() else {
        return Vec::new();
    };
    let nums = |v: &serde_json::Value| -> Vec<f64> {
        v.as_array()
            .map(|a| {
                a.iter()
                    .filter_map(serde_json::Value::as_f64)
                    .filter(|n| n.is_finite() && *n > 0.0)
                    .collect()
            })
            .unwrap_or_default()
    };
    let one = |v: &serde_json::Value| v.as_f64().filter(|n| n.is_finite() && *n >= 0.0);
    let key = |id: &str| {
        id.rsplit('/')
            .next()
            .unwrap_or(id)
            .to_ascii_lowercase()
            .replace('.', "-")
    };
    // Evidence per model: (asks_in, asks_out, officials_in, officials_out).
    #[derive(Default)]
    struct Evidence {
        asks_in: Vec<f64>,
        asks_out: Vec<f64>,
        official_in: Vec<f64>,
        official_out: Vec<f64>,
    }
    let mut own: Vec<(String, Evidence)> = Vec::new();
    let mut pooled: HashMap<String, Evidence> = HashMap::new();
    let mut bare: Vec<String> = Vec::new();
    for m in data {
        let Some(id) = m["id"].as_str() else {
            continue;
        };
        let p = &m["pricing"];
        if !p.is_object() {
            bare.push(id.to_string());
            continue;
        }
        let ev = Evidence {
            asks_in: nums(&p["asks_in"]),
            asks_out: nums(&p["asks_out"]),
            official_in: one(&p["official_in"]).into_iter().collect(),
            official_out: one(&p["official_out"]).into_iter().collect(),
        };
        let pool = pooled.entry(key(id)).or_default();
        pool.asks_in.extend(&ev.asks_in);
        pool.asks_out.extend(&ev.asks_out);
        pool.official_in.extend(&ev.official_in);
        pool.official_out.extend(&ev.official_out);
        own.push((id.to_string(), ev));
    }
    let pick = |asks: &[f64], official: &[f64]| -> Option<f64> {
        let asks_stat = match stat {
            forge_config::PricingStat::Official => None,
            forge_config::PricingStat::Min => asks.iter().copied().reduce(f64::min),
            forge_config::PricingStat::Mean => {
                (!asks.is_empty()).then(|| asks.iter().sum::<f64>() / asks.len() as f64)
            }
            forge_config::PricingStat::Median => median(asks),
        };
        asks_stat
            .or_else(|| median(official))
            .or_else(|| (stat == forge_config::PricingStat::Official).then(|| median(asks))?)
    };
    let price = |ev: &Evidence| -> Option<(f64, f64)> {
        let input = pick(&ev.asks_in, &ev.official_in)?;
        let output = pick(&ev.asks_out, &ev.official_out)?;
        Some((input / 1000.0, output / 1000.0))
    };
    let mut out: Vec<(String, f64, f64, Option<f64>)> = own
        .iter()
        .filter_map(|(id, ev)| {
            let (i, o) = price(ev)?;
            Some((format!("{namespace}::{id}"), i, o, None))
        })
        .collect();
    out.extend(bare.iter().filter_map(|id| {
        let (i, o) = price(pooled.get(&key(id))?)?;
        Some((format!("{namespace}::{id}"), i, o, None))
    }));
    out
}

/// The median of a list, `None` when empty. Order-independent.
fn median(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut v = values.to_vec();
    v.sort_by(f64::total_cmp);
    let mid = v.len() / 2;
    Some(if v.len().is_multiple_of(2) {
        (v[mid - 1] + v[mid]) / 2.0
    } else {
        v[mid]
    })
}

// ── HTTP helpers ─────────────────────────────────────────────────────────────────────────────────

/// True for a new-api gateway's `/models` body (ModelGateway): its entries carry
/// `supported_endpoint_types`, and such a gateway publishes prices at `<origin>/api/pricing`
/// rather than in the listing.
pub(super) fn is_new_api_listing(body: &serde_json::Value) -> bool {
    body["data"]
        .as_array()
        .is_some_and(|d| d.iter().any(|m| m["supported_endpoint_types"].is_array()))
}

/// Prices from a new-api `/api/pricing` body, as `(namespace::model, in_1k, out_1k, None)`.
///
/// The billed rate is the vendor's official per-1M price less `discount_percent`, for input and
/// output alike: on ModelGateway (2026-10-14) a 20k-token glm-5.3-flash prompt, a 56k-token
/// claude-sonnet-5 prompt and a 2.9k-token sonnet completion each billed exactly that. The
/// `model_ratio` / `lowest_price` fields do not match the bill and are ignored. Per-request priced
/// models (`quota_type` 1) and models without an official price are left unpriced, never $0.
pub(super) fn new_api_pricing(
    body: &serde_json::Value,
    namespace: &str,
) -> Vec<(String, f64, f64, Option<f64>)> {
    let Some(data) = body["data"].as_array() else {
        return Vec::new();
    };
    data.iter()
        .filter_map(|m| {
            let id = m["model_name"].as_str()?;
            if m["quota_type"].as_u64().unwrap_or(0) != 0 {
                return None;
            }
            let official = |field: &str| m[field].as_f64().filter(|p| p.is_finite() && *p > 0.0);
            let (input, output) = (
                official("official_input_price")?,
                official("official_output_price")?,
            );
            let keep = 1.0
                - m["discount_percent"]
                    .as_f64()
                    .unwrap_or(0.0)
                    .clamp(0.0, 100.0)
                    / 100.0;
            Some((
                format!("{namespace}::{id}"),
                input * keep / 1000.0,
                output * keep / 1000.0,
                None,
            ))
        })
        .collect()
}

#[cfg(test)]
mod marketplace_tests {
    use super::*;
    use forge_config::PricingStat;

    /// Verbatim InferHub `/models` entries (2026-10-14), plus the unprefixed alias the user's
    /// config routes to, which lists no pricing of its own.
    fn body() -> serde_json::Value {
        serde_json::json!({"data": [
            {"id": "cc/claude-opus-5", "pricing": {"official_in": 5, "official_out": 25,
                "asks_in": [0.8, 1, 1.75, 2, 2.465, 2.495, 2.5],
                "asks_out": [4, 5, 8.75, 10, 12.325, 12.475, 12.5],
                "min_ask_in": 0.8, "min_ask_out": 4}},
            {"id": "kiro/claude-opus-5", "pricing": {"official_in": 5, "official_out": 25,
                "asks_in": [2.5], "asks_out": [12.5], "min_ask_in": 2.5, "min_ask_out": 12.5}},
            {"id": "claude-opus-5"},
            {"id": "no-asks/model", "pricing": {"official_in": 1, "official_out": 4}},
            {"id": "kling-3.0"}
        ]})
    }

    fn rate(rows: &[(String, f64, f64, Option<f64>)], id: &str) -> Option<(f64, f64)> {
        rows.iter()
            .find(|(i, ..)| i == id)
            .map(|(_, i, o, _)| (*i, *o))
    }

    fn close(a: (f64, f64), b: (f64, f64)) -> bool {
        (a.0 - b.0).abs() < 1e-12 && (a.1 - b.1).abs() < 1e-12
    }

    #[test]
    fn the_median_ask_prices_a_model_not_the_cheapest_seller() {
        let rows = marketplace_pricing(&body(), "inferhub", PricingStat::Median);
        // 7 asks: the middle one is $2 / $10 per 1M → per 1k.
        assert!(close(
            rate(&rows, "inferhub::cc/claude-opus-5").unwrap(),
            (0.002, 0.010)
        ));
        let min = marketplace_pricing(&body(), "inferhub", PricingStat::Min);
        assert!(close(
            rate(&min, "inferhub::cc/claude-opus-5").unwrap(),
            (0.0008, 0.004)
        ));
        let official = marketplace_pricing(&body(), "inferhub", PricingStat::Official);
        assert!(close(
            rate(&official, "inferhub::cc/claude-opus-5").unwrap(),
            (0.005, 0.025)
        ));
        let mean = marketplace_pricing(&body(), "inferhub", PricingStat::Mean);
        let (i, _) = rate(&mean, "inferhub::cc/claude-opus-5").unwrap();
        assert!((i - 13.01 / 7.0 / 1000.0).abs() < 1e-12);
    }

    #[test]
    fn an_alias_pools_the_asks_of_every_route_to_the_same_model() {
        let rows = marketplace_pricing(&body(), "inferhub", PricingStat::Median);
        // Pooled asks_in: 0.8 1 1.75 2 2.465 2.495 2.5 2.5 → median (2 + 2.465) / 2.
        let (i, o) = rate(&rows, "inferhub::claude-opus-5").expect("alias is priced");
        assert!((i - 2.2325 / 1000.0).abs() < 1e-12, "{i}");
        assert!((o - 11.1625 / 1000.0).abs() < 1e-12, "{o}");
    }

    #[test]
    fn missing_asks_fall_back_to_official_and_no_evidence_stays_unpriced() {
        let rows = marketplace_pricing(&body(), "inferhub", PricingStat::Median);
        assert!(close(
            rate(&rows, "inferhub::no-asks/model").unwrap(),
            (0.001, 0.004)
        ));
        assert!(
            rate(&rows, "inferhub::kling-3.0").is_none(),
            "no evidence must never be written as $0 (which reads as free)"
        );
        assert!(rows.iter().all(|(_, _, _, cache)| cache.is_none()));
    }

    #[test]
    fn a_plain_openai_listing_yields_nothing() {
        let body = serde_json::json!({"data": [{"id": "gpt-x"}, {"id": "y", "context_length": 8}]});
        assert!(marketplace_pricing(&body, "custom", PricingStat::Median).is_empty());
    }

    #[test]
    fn new_api_pricing_bills_official_less_discount() {
        // ModelGateway `/api/pricing` entries, trimmed (2026-10-14).
        let body = serde_json::json!({"data": [
            {"model_name": "glm-5.3-flash", "quota_type": 0, "model_ratio": 0.0049995,
             "official_input_price": 0.15, "official_output_price": 0.5, "discount_percent": 98},
            {"model_name": "claude-sonnet-5", "quota_type": 0, "model_ratio": 0.1,
             "official_input_price": 2, "official_output_price": 10, "discount_percent": 98},
            {"model_name": "per-request", "quota_type": 1, "official_input_price": 1,
             "official_output_price": 1, "discount_percent": 0},
            {"model_name": "unlisted", "quota_type": 0, "official_input_price": 0,
             "official_output_price": 0, "discount_percent": 98}
        ]});
        let rows = new_api_pricing(&body, "modelgateway");
        assert_eq!(rows.len(), 2);
        let (id, i, o, cache) = &rows[1];
        assert_eq!(id, "modelgateway::claude-sonnet-5");
        assert!((i - 0.00004).abs() < 1e-12 && (o - 0.0002).abs() < 1e-12);
        assert!(cache.is_none());
        assert!((rows[0].1 - 0.000003).abs() < 1e-12);
    }

    #[test]
    fn only_new_api_listings_are_detected() {
        let gateway =
            serde_json::json!({"data": [{"id": "glm-5", "supported_endpoint_types": ["openai"]}]});
        let plain = serde_json::json!({"data": [{"id": "glm-5", "context_window": 1000}]});
        assert!(is_new_api_listing(&gateway));
        assert!(!is_new_api_listing(&plain));
    }
}
