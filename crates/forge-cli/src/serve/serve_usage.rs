//! Usage and subscription-window projection for the Serve control surface.

use std::sync::Arc;

use axum::extract::{Query, State};
use axum::response::Response;

use super::{err_response, json_response, DaemonState};

#[path = "serve_usage_activity.rs"]
pub(crate) mod activity;

#[derive(serde::Deserialize)]
pub(super) struct UsageParams {
    session: Option<String>,
}

#[derive(Debug, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct UsageTotals {
    input_tokens: u64,
    /// Summed over the providers that report caching. `None` when none of them do — the total is
    /// unknown rather than zero, and a partial total covers only the reporting providers.
    cached_input_tokens: Option<u64>,
    output_tokens: u64,
    cost_usd: f64,
}

#[derive(Debug, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct UsageProvider {
    provider: String,
    kind: String,
    input_tokens: u64,
    /// `None` when this provider does not report prompt-cache hits at all.
    cached_input_tokens: Option<u64>,
    output_tokens: u64,
    cost_usd: f64,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct UsageQuota {
    provider: String,
    kind: String,
    window_kind: String,
    status: String,
    resets_at: Option<i64>,
    fraction: Option<f64>,
    updated_at: i64,
    /// The router's pacing verdict, present only on the window pacing is currently judged by.
    pacing: Option<UsagePacing>,
}

/// [`forge_types::SubscriptionPacing`] as the phone renders it — the mesh's own numbers and
/// summary line, never a client-side re-derivation from `fraction` and `resets_at`.
#[derive(Debug, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct UsagePacing {
    fraction_used: f64,
    allowed_fraction: f64,
    elapsed_fraction: f64,
    over_pace: bool,
    /// True when no reset time was known and timing began at the oldest observation; the
    /// summary then reads "pace unknown" and `over_pace` must not be acted on.
    used_nominal_fallback: bool,
    summary: String,
}

fn usage_pacing(
    pacing: Option<&forge_types::SubscriptionPacing>,
    window_kind: &str,
) -> Option<UsagePacing> {
    let pacing = pacing.filter(|pacing| pacing.window == window_kind)?;
    Some(UsagePacing {
        fraction_used: pacing.fraction_used,
        allowed_fraction: pacing.allowed_fraction,
        elapsed_fraction: pacing.elapsed_fraction(),
        over_pace: pacing.is_over_pace(),
        used_nominal_fallback: pacing.used_nominal_fallback,
        summary: forge_mesh::pacing_summary(Some(pacing), None),
    })
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct UsageWindow {
    since_epoch: i64,
    combined: UsageTotals,
    providers: Vec<UsageProvider>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct SessionUsage {
    session_id: String,
    combined: UsageTotals,
    providers: Vec<UsageProvider>,
}

#[derive(serde::Serialize)]
struct UsageResponse {
    week: UsageWindow,
    /// Since local midnight on the daemon host, so "today" on the fleet is a real day's spend.
    today: UsageWindow,
    session: Option<SessionUsage>,
    quota: Vec<UsageQuota>,
}

fn provider_kind(provider: &str) -> &'static str {
    if provider.ends_with("-cli") {
        "bridge"
    } else if provider.ends_with("-oauth") {
        "oauth"
    } else {
        "api"
    }
}

/// Claude and Codex subscription windows are learned by probes that cost a tiny turn on the plan, so
/// the page never waits on them: the refresh runs in the background (at most one at a time) and the
/// next poll shows its result. The gate is wider than routing's five minutes because every open app
/// polls this page continuously; a running session still refreshes on routing's cadence.
const SUBSCRIPTION_PROBE_MAX_AGE_SECS: i64 = 15 * 60;

fn spawn_subscription_refresh(store: Arc<forge_store::Store>) {
    static IN_FLIGHT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if IN_FLIGHT.swap(true, std::sync::atomic::Ordering::AcqRel) {
        return;
    }
    tokio::spawn(async move {
        let codex_age = ["codex-oauth", "codex-cli"]
            .into_iter()
            .filter_map(|provider| store.subscription_age_secs(provider))
            .min();
        if codex_age.is_none_or(|age| age > SUBSCRIPTION_PROBE_MAX_AGE_SECS) {
            crate::cli::commands::models::refresh_codex_quota(&store).await;
        }
        crate::cli::commands::models::refresh_claude_quota_if_stale(
            &store,
            SUBSCRIPTION_PROBE_MAX_AGE_SECS,
        )
        .await;
        IN_FLIGHT.store(false, std::sync::atomic::Ordering::Release);
    });
}

fn local_midnight(now: i64) -> i64 {
    use chrono::TimeZone;
    let day = chrono::Local
        .timestamp_opt(now, 0)
        .single()
        .map(|time| time.date_naive())
        .unwrap_or_default();
    day.and_hms_opt(0, 0, 0)
        .and_then(|midnight| chrono::Local.from_local_datetime(&midnight).earliest())
        .map_or(now - 86_400, |midnight| midnight.timestamp())
}

/// A subscription bills nothing per call. Rows recorded before Codex calls were priced at $0 still
/// carry a notional dollar figure; showing it would present plan usage as API spend.
fn billed_cost(kind: &str, cost_usd: f64) -> f64 {
    if kind == "api" {
        cost_usd
    } else {
        0.0
    }
}

fn usage_providers(rows: Vec<forge_store::ProviderUsage>) -> (UsageTotals, Vec<UsageProvider>) {
    let total = rows.iter().fold(
        UsageTotals {
            input_tokens: 0,
            cached_input_tokens: None,
            output_tokens: 0,
            cost_usd: 0.0,
        },
        |mut total, row| {
            total.input_tokens += row.input_tokens;
            // Stays None until some provider reports, so a fleet of non-reporting providers does
            // not add up to a confident zero.
            if let Some(cached) = row.cached_input_tokens {
                total.cached_input_tokens = Some(total.cached_input_tokens.unwrap_or(0) + cached);
            }
            total.output_tokens += row.output_tokens;
            total.cost_usd += billed_cost(provider_kind(&row.provider), row.cost_usd);
            total
        },
    );
    let providers = rows
        .into_iter()
        .map(|row| UsageProvider {
            cost_usd: billed_cost(provider_kind(&row.provider), row.cost_usd),
            kind: provider_kind(&row.provider).into(),
            provider: row.provider,
            input_tokens: row.input_tokens,
            cached_input_tokens: row.cached_input_tokens,
            output_tokens: row.output_tokens,
        })
        .collect();
    (total, providers)
}

pub(super) async fn usage_page(
    State(state): State<Arc<DaemonState>>,
    Query(params): Query<UsageParams>,
) -> Response {
    let store = state.store.clone();
    // OpenCode Go and Kimi Code only reveal their windows through a poll, so the daemon refreshes
    // them here before reading. Each refresher's freshness gate collapses repeated page loads into
    // at most one request every few minutes, which is also the cadence routing needs.
    crate::cli::commands::models::refresh_polled_quotas(&store).await;
    spawn_subscription_refresh(store.clone());
    let result = tokio::task::spawn_blocking(move || -> anyhow::Result<UsageResponse> {
        let now = chrono::Utc::now().timestamp();
        let week_rows = store.usage_by_provider_since(now - 604800)?;
        let (combined, providers) = usage_providers(week_rows);
        let midnight = local_midnight(now);
        let (today_combined, today_providers) =
            usage_providers(store.usage_by_provider_since(midnight)?);
        let session = match params.session.filter(|session| !session.is_empty()) {
            Some(id) => {
                let (combined, providers) =
                    usage_providers(store.usage_by_provider_for_session(&id)?);
                Some(SessionUsage {
                    session_id: id,
                    combined,
                    providers,
                })
            }
            None => None,
        };
        let pacing = store.subscription_pacing().unwrap_or_default();
        let quota = store
            .subscription_windows_for_display(now)?
            .into_iter()
            .map(|quota| UsageQuota {
                kind: provider_kind(&quota.provider).into(),
                pacing: usage_pacing(pacing.get(&quota.provider), &quota.window_kind),
                provider: quota.provider,
                window_kind: quota.window_kind,
                status: quota.status,
                resets_at: quota.resets_at,
                fraction: quota.fraction,
                updated_at: quota.updated_at,
            })
            .collect();
        Ok(UsageResponse {
            week: UsageWindow {
                since_epoch: now - 604800,
                combined,
                providers,
            },
            today: UsageWindow {
                since_epoch: midnight,
                combined: today_combined,
                providers: today_providers,
            },
            session,
            quota,
        })
    })
    .await;
    match result {
        Ok(Ok(body)) => json_response(&body),
        Ok(Err(error)) => err_response(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            &format!("usage unavailable: {error}"),
        ),
        Err(_) => err_response(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "usage unavailable",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::{local_midnight, provider_kind, usage_pacing, usage_providers, UsageTotals};

    fn pacing(window: &str, resets_at: Option<i64>) -> forge_types::SubscriptionPacing {
        forge_types::SubscriptionPacing {
            window: window.into(),
            fraction_used: 0.32,
            allowed_fraction: 0.22,
            elapsed_secs: 172_800,
            total_secs: 604_800,
            resets_at,
            used_nominal_fallback: resets_at.is_none(),
        }
    }

    #[test]
    fn the_binding_window_carries_the_routers_pacing_verdict_and_the_others_do_not() {
        let weekly = pacing("weekly", Some(1_000_000));
        let row = usage_pacing(Some(&weekly), "weekly").expect("the paced window is annotated");
        assert!(row.over_pace);
        assert!(!row.used_nominal_fallback);
        assert_eq!(row.summary, "weekly 32% used · 22% allowed · OVER PACE");
        assert!(usage_pacing(Some(&weekly), "five_hour").is_none());
        assert!(usage_pacing(None, "weekly").is_none());
    }

    #[test]
    fn a_window_without_a_reset_time_is_reported_as_pace_unknown() {
        let guessed = pacing("weekly", None);
        let row = usage_pacing(Some(&guessed), "weekly").unwrap();
        assert!(row.used_nominal_fallback);
        assert!(row.summary.contains("pace unknown"), "{}", row.summary);
        assert!(
            row.summary.contains("used_nominal_fallback"),
            "{}",
            row.summary
        );
    }

    #[test]
    fn today_starts_at_the_most_recent_local_midnight() {
        let now = chrono::Utc::now().timestamp();
        let midnight = local_midnight(now);
        assert!(
            midnight <= now && now - midnight <= 25 * 3600,
            "{midnight} vs {now}"
        );
        assert_eq!(local_midnight(midnight), midnight);
    }

    #[test]
    fn provider_kinds_preserve_client_grouping_contract() {
        assert_eq!(provider_kind("claude-cli"), "bridge");
        assert_eq!(provider_kind("codex-oauth"), "oauth");
        assert_eq!(provider_kind("gemini"), "api");
        assert_eq!(provider_kind("xai-oauth"), "oauth");
        assert_eq!(provider_kind("agy-cli"), "bridge");
        assert_eq!(provider_kind("openai"), "api");
    }

    #[test]
    fn provider_rows_sum_without_losing_cached_tokens_or_metered_cost() {
        let rows = vec![
            forge_store::ProviderUsage {
                provider: "openai".into(),
                input_tokens: 10,
                cached_input_tokens: Some(3),
                output_tokens: 5,
                cost_usd: 0.25,
            },
            forge_store::ProviderUsage {
                provider: "claude-cli".into(),
                input_tokens: 20,
                cached_input_tokens: Some(7),
                output_tokens: 9,
                cost_usd: 0.5,
            },
        ];
        let (total, providers) = usage_providers(rows);
        assert_eq!(
            total,
            UsageTotals {
                input_tokens: 30,
                cached_input_tokens: Some(10),
                output_tokens: 14,
                cost_usd: 0.25,
            }
        );
        assert_eq!(
            providers[1].cost_usd, 0.0,
            "a subscription row never reads as spend"
        );
        assert_eq!(providers[0].kind, "api");
        assert_eq!(providers[1].kind, "bridge");
    }
}
