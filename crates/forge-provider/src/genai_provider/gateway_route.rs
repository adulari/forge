//! Gateway routing failures: the gateway is up but has no route for the request right now.

use super::*;

/// Phrases of a gateway that is up but cannot route this request right now: every upstream it
/// fronts is down, saturated or not yet registered. A gateway such as ModelGateway answers
/// `503 No provider route is available ... Try again shortly` for a model it serves fine a second
/// later, and the same body carries codes (`model_not_available`) that read as a dead model id.
/// Checked before the permanent classifications so a pinned turn waits instead of ending as "not
/// available on this account". The model owner's own refusal ("model not found") has none of
/// these phrases.
const GATEWAY_ROUTE_MARKERS: &[&str] = &[
    "no provider route",
    "no_provider_route",
    "no available provider",
    "no_available_provider",
    "provider route",
    "provider_route",
    "provider unavailable",
    "provider_unavailable",
    "upstream unavailable",
    "upstream_unavailable",
    "no healthy upstream",
    "no available upstream",
    "no available channel",
];

/// The first [`GATEWAY_ROUTE_MARKERS`] phrase in `text`, in words (`no provider route`).
fn gateway_route_phrase(text: &str) -> Option<String> {
    let l = text.to_ascii_lowercase();
    GATEWAY_ROUTE_MARKERS
        .iter()
        .find(|m| l.contains(**m))
        .map(|m| m.replace('_', " "))
}

pub(super) fn is_gateway_route_failure(text: &str) -> bool {
    gateway_route_phrase(text).is_some()
}

/// What to show for a route failure: the gateway's own category/code when that already names the
/// route (`provider_routes_exhausted`), else the phrase that matched.
fn route_reason(body: &str, message: &str) -> String {
    let value = serde_json::from_str::<serde_json::Value>(body).ok();
    let error = value.as_ref().map(|v| v.get("error").unwrap_or(v));
    let named = [
        error
            .and_then(|e| e.get("details"))
            .and_then(|d| d.get("category")),
        error.and_then(|e| e.get("code")),
        error.and_then(|e| e.get("type")),
    ]
    .into_iter()
    .flatten()
    .filter_map(|v| v.as_str())
    .find(|n| is_gateway_route_failure(n));
    named
        .map(str::to_string)
        .or_else(|| gateway_route_phrase(message))
        .or_else(|| gateway_route_phrase(body))
        .unwrap_or_else(|| "no provider route".to_string())
}

/// The busy-gateway error for a routing failure. `status` is kept when it is a 5xx, else 503: a
/// gateway that files this under 400/404 is still saying "try again shortly".
pub(super) fn gateway_route_error(
    code: u16,
    body: &str,
    message: &str,
    provider_message: Option<&str>,
    retry_after: Option<std::time::Duration>,
) -> ProviderError {
    ProviderError::Overloaded {
        status: if (500..=599).contains(&code) {
            code
        } else {
            503
        },
        reason: route_reason(body, message),
        message: overload_message(provider_message.unwrap_or(message)),
        retry_after,
    }
}

/// Why a 5xx is the provider shedding load rather than failing: 503/529, a `Retry-After`, a body
/// that says `retryable: true`, or `overloaded`/`busy`/`capacity` in its type/code/category.
/// Returns the short name to show (`gateway_overloaded`, `overloaded_error`, or `overloaded`).
/// A plain `500 Internal server error` is none of these and stays [`ProviderError::Unavailable`],
/// which keeps the identical-fast-error rejection heuristic for models a provider does not serve.
pub(super) fn overload_reason(
    code: u16,
    body: &str,
    message: &str,
    retry_after: Option<std::time::Duration>,
) -> Option<String> {
    let value = serde_json::from_str::<serde_json::Value>(body).ok();
    let error = value.as_ref().map(|v| v.get("error").unwrap_or(v));
    let field = |obj: Option<&serde_json::Value>, key: &str| {
        obj.and_then(|o| o.get(key))
            .and_then(|v| v.as_str())
            .map(str::to_string)
    };
    let details = error.and_then(|e| e.get("details"));
    let names: Vec<String> = [
        field(details, "category"),
        field(error, "code"),
        field(error, "type"),
    ]
    .into_iter()
    .flatten()
    .collect();
    let marked = |n: &str| {
        let l = n.to_ascii_lowercase();
        ["overload", "busy", "capacity"]
            .iter()
            .any(|k| l.contains(k))
    };
    let retryable = details
        .and_then(|d| d.get("retryable"))
        .or_else(|| error.and_then(|e| e.get("retryable")))
        .and_then(|v| v.as_bool())
        == Some(true);
    let named = names.iter().find(|n| marked(n)).cloned();
    let prose = marked(message) && message.to_ascii_lowercase().contains("overload");
    if matches!(code, 503 | 529) || retry_after.is_some() || retryable || named.is_some() || prose {
        return Some(
            named
                .or_else(|| names.first().cloned())
                .unwrap_or_else(|| "overloaded".to_string()),
        );
    }
    None
}

/// The provider's own sentence without the per-request `(request id: …)` tail, which differs on
/// every attempt and has nothing to act on.
pub(super) fn overload_message(message: &str) -> String {
    let first = message.lines().next().unwrap_or(message);
    let cut = first.find("(request id").map_or(first, |i| &first[..i]);
    let cut = cut.trim();
    if cut.chars().count() > 300 {
        let head: String = cut.chars().take(297).collect();
        format!("{head}…")
    } else {
        cut.to_string()
    }
}
