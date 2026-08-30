//! Parsing ChatGPT Codex quota headers into `QuotaHint`s.
//!
//! Split out of the crate root so the header contract has one owner and can be read
//! without the provider's dispatch code wrapped around it.

use super::{now_unix, CODEX_OAUTH_NAMESPACE};

/// Parse account-wide ChatGPT quota from the `x-codex-*` response headers the backend sends on
/// EVERY `POST {CODEX_API_BASE}/responses` (verified live 2026-07-10):
/// `x-codex-{primary,secondary}-used-percent`, `-window-minutes` (300→"five_hour",
/// 10080→"weekly"), `-reset-at` (unix seconds), plus `x-codex-plan-type`. A successful backend
/// response is fresher than the OAuth JWT claim, so its plan is captured as a short-lived shared
/// account observation. Mirrors
/// [`codex_websocket::parse_rate_limits_frame`]'s in-band `codex.rate_limits` mapping exactly:
/// same window labels, same status thresholds, skip a window whose reset has already passed, skip
/// a window missing `used-percent` (no hint over a wrong hint).
pub(crate) fn parse_codex_quota_headers(
    headers: &reqwest::header::HeaderMap,
) -> Vec<forge_types::QuotaHint> {
    if let Some(plan) = headers
        .get("x-codex-plan-type")
        .and_then(|value| value.to_str().ok())
    {
        crate::record_live_codex_plan(plan);
    }
    let now_secs = now_unix();
    let mut hints = Vec::new();
    for (used_key, mins_key, reset_key, fallback_label) in [
        (
            "x-codex-primary-used-percent",
            "x-codex-primary-window-minutes",
            "x-codex-primary-reset-at",
            "primary",
        ),
        (
            "x-codex-secondary-used-percent",
            "x-codex-secondary-window-minutes",
            "x-codex-secondary-reset-at",
            "secondary",
        ),
    ] {
        let Some(used) = header_f64(headers, used_key) else {
            continue;
        };
        let resets = header_i64(headers, reset_key);
        // Skip a window whose period has already reset — same rule as the WS path.
        if let Some(r) = resets {
            if r <= now_secs {
                continue;
            }
        }
        let mins = header_i64(headers, mins_key).unwrap_or(0);
        let fraction = used / 100.0;
        let status = forge_config::quota_status::status_from_fraction(fraction);
        let label = match mins {
            300 => "five_hour".to_string(),
            10080 => "weekly".to_string(),
            m if m > 0 => format!("{m}m"),
            _ => fallback_label.to_string(),
        };
        hints.push(forge_types::QuotaHint {
            provider: CODEX_OAUTH_NAMESPACE.to_string(),
            window: label,
            status,
            resets_at: resets,
            fraction_used: Some(fraction),
        });
    }
    hints
}

pub(crate) fn header_f64(headers: &reqwest::header::HeaderMap, name: &str) -> Option<f64> {
    headers.get(name)?.to_str().ok()?.trim().parse::<f64>().ok()
}

pub(crate) fn header_i64(headers: &reqwest::header::HeaderMap, name: &str) -> Option<i64> {
    headers.get(name)?.to_str().ok()?.trim().parse::<i64>().ok()
}
