//! Live subscription-window awareness for Kimi Code (`kimi`).
//!
//! Like OpenCode Go, Kimi Code's chat completions carry no rate-limit headers — only a token
//! `usage` block — so the plan window is observable solely through a poll of
//! `GET https://api.kimi.com/coding/v1/usages`. Verified live against a real subscription on
//! 2026-09-14:
//!
//! ```text
//! 200 {"limits":[{"window":{"duration":300,"timeUnit":"TIME_UNIT_MINUTE"},
//!                 "detail":{"limit":"100","used":"1","remaining":"99",
//!                           "resetTime":"2026-09-14T22:21:14.589596Z"}}],
//!      "usages":{"limit_5h":{"used_ratio":0,"reset_time":"2026-09-14T22:21:14Z"}}}
//! ```
//!
//! `limits[].detail` is the reading used here. `usages.limit_5h.used_ratio` read 0 in the same
//! response that reported `used: 1`, so it is not trusted. A fresh window omits `used` entirely
//! (`limit` 100, `remaining` 100), and every count arrives as a string.
//!
//! Every failure mode degrades to "unobserved" — no store write — never to a fabricated 0%: a
//! window Forge cannot read must not tell the router a spent plan is fresh.

use forge_store::Store;

const USAGE_URL: &str = "https://api.kimi.com/coding/v1/usages";

/// Forge's provider namespace for Kimi Code, and the key that store rows are written under.
pub(crate) const KIMI_PROVIDER: &str = "kimi";

/// How long a reading stays authoritative. The plan can be spent outside Forge (Kimi's own CLI,
/// other machines), so this matches the Codex freshness bound; it is also the minimum interval
/// between polls, which keeps the refresher off the per-request path.
const KIMI_QUOTA_MAX_AGE_SECS: i64 = forge_types::CODEX_QUOTA_FRESHNESS_SECS;

/// A count that may arrive as a JSON string (`"100"`) or a number.
fn count(value: &serde_json::Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str().and_then(|s| s.trim().parse().ok()))
}

/// Map a `{duration, timeUnit}` window onto Forge's window vocabulary. Windows Forge has no kind
/// for are dropped rather than forced into the nearest one.
fn window_kind(window: &serde_json::Value) -> Option<&'static str> {
    let duration = count(window.get("duration")?)?;
    let minutes = match window.get("timeUnit")?.as_str()? {
        "TIME_UNIT_MINUTE" => duration,
        "TIME_UNIT_HOUR" => duration * 60.0,
        "TIME_UNIT_DAY" => duration * 1440.0,
        _ => return None,
    };
    match minutes.round() as i64 {
        300 => Some("five_hour"),
        10_080 => Some("weekly"),
        _ => None,
    }
}

/// Parse the usages body into quota hints. Pure and total: anything malformed yields no hint for
/// that window, so an unreadable response is indistinguishable from never having polled.
fn parse_usages(body: &str) -> Vec<forge_types::QuotaHint> {
    let Ok(root) = serde_json::from_str::<serde_json::Value>(body) else {
        return Vec::new();
    };
    let Some(limits) = root.get("limits").and_then(serde_json::Value::as_array) else {
        return Vec::new();
    };
    limits
        .iter()
        .filter_map(|entry| {
            let window = window_kind(entry.get("window")?)?;
            let detail = entry.get("detail")?;
            let limit = count(detail.get("limit")?)?;
            if limit <= 0.0 {
                return None;
            }
            let used = match detail.get("remaining").and_then(count) {
                Some(remaining) => limit - remaining,
                None => count(detail.get("used")?)?,
            };
            let fraction = (used / limit).clamp(0.0, 1.0);
            // A window without a reset instant cannot be paced, so it is dropped, not recorded.
            let resets_at = detail
                .get("resetTime")
                .and_then(serde_json::Value::as_str)
                .and_then(|iso| chrono::DateTime::parse_from_rfc3339(iso).ok())?
                .timestamp();
            Some(forge_types::QuotaHint {
                provider: KIMI_PROVIDER.to_string(),
                window: window.to_string(),
                status: forge_config::quota_status::status_from_fraction(fraction),
                resets_at: Some(resets_at),
                fraction_used: Some(fraction),
            })
        })
        .collect()
}

fn is_fresh(store: &Store) -> bool {
    store
        .subscription_age_secs(KIMI_PROVIDER)
        .is_some_and(|age| age <= KIMI_QUOTA_MAX_AGE_SECS)
}

/// Refresh the Kimi Code subscription window before a routing decision.
///
/// Best-effort in every direction: no key, an unreachable endpoint, a non-200 status or an
/// unparseable body all leave the store untouched.
pub(crate) async fn refresh_kimi_quota(store: &Store) {
    if is_fresh(store) {
        return;
    }
    let Ok(key) = forge_config::api_key(KIMI_PROVIDER) else {
        return;
    };
    if key.is_empty() {
        return;
    }
    let Some(body) = fetch_usages(USAGE_URL, &key).await else {
        return;
    };
    let now = chrono::Utc::now().timestamp();
    for hint in parse_usages(&body) {
        let _ = store.record_quota_at(&hint, now);
    }
}

async fn fetch_usages(url: &str, key: &str) -> Option<String> {
    let response = forge_provider::bundled_http_client()
        .get(url)
        .bearer_auth(key)
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        tracing::debug!("kimi usages endpoint returned {}", response.status());
        return None;
    }
    response.text().await.ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The exact body observed live on 2026-09-14, including the disagreeing `used_ratio`.
    const REAL_SHAPE: &str = r#"{"limits":[{"window":{"duration":300,"timeUnit":"TIME_UNIT_MINUTE"},
        "detail":{"limit":"100","used":"37","remaining":"63","resetTime":"2026-09-14T22:21:14.589596Z"}}],
        "usages":{"limit_5h":{"used_ratio":0,"reset_time":"2026-09-14T22:21:14Z"}}}"#;

    #[test]
    fn the_five_hour_window_reads_from_detail_not_used_ratio() {
        let hints = parse_usages(REAL_SHAPE);
        assert_eq!(hints.len(), 1);
        let hint = &hints[0];
        assert_eq!(hint.provider, KIMI_PROVIDER);
        assert_eq!(hint.window, "five_hour");
        assert!((hint.fraction_used.unwrap() - 0.37).abs() < 1e-9);
        assert_eq!(
            hint.resets_at,
            Some(
                chrono::DateTime::parse_from_rfc3339("2026-09-14T22:21:14.589596Z")
                    .unwrap()
                    .timestamp()
            )
        );
    }

    #[test]
    fn a_fresh_window_without_a_used_field_reads_as_empty_not_unobserved() {
        let body = r#"{"limits":[{"window":{"duration":300,"timeUnit":"TIME_UNIT_MINUTE"},
            "detail":{"limit":"100","remaining":"100","resetTime":"2026-09-14T22:21:14Z"}}]}"#;
        let hints = parse_usages(body);
        assert_eq!(hints.len(), 1);
        assert_eq!(hints[0].fraction_used, Some(0.0));
    }

    #[test]
    fn a_weekly_window_in_days_maps_and_an_unknown_length_is_dropped() {
        let body = r#"{"limits":[
            {"window":{"duration":7,"timeUnit":"TIME_UNIT_DAY"},
             "detail":{"limit":100,"used":90,"resetTime":"2026-09-20T00:00:00Z"}},
            {"window":{"duration":42,"timeUnit":"TIME_UNIT_MINUTE"},
             "detail":{"limit":"100","used":"5","resetTime":"2026-09-20T00:00:00Z"}}]}"#;
        let hints = parse_usages(body);
        assert_eq!(hints.len(), 1);
        assert_eq!(hints[0].window, "weekly");
        assert!((hints[0].fraction_used.unwrap() - 0.9).abs() < 1e-9);
    }

    #[test]
    fn unreadable_bodies_record_nothing() {
        for body in [
            "",
            "not json",
            r#"{"error":{"message":"The requested resource was not found"}}"#,
            r#"{"limits":[{"window":{"duration":300,"timeUnit":"TIME_UNIT_MINUTE"},"detail":{"limit":"100","used":"1"}}]}"#,
            r#"{"limits":[{"window":{"duration":300,"timeUnit":"TIME_UNIT_MINUTE"},"detail":{"limit":"0","used":"0","resetTime":"2026-09-14T22:21:14Z"}}]}"#,
        ] {
            assert!(parse_usages(body).is_empty(), "{body}");
        }
    }
}
