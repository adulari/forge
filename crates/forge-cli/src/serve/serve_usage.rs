//! Usage and subscription-window projection for the Serve control surface.

use std::sync::Arc;

use axum::extract::{Query, State};
use axum::response::Response;

use super::{err_response, json_response, DaemonState};

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
            total.cost_usd += row.cost_usd;
            total
        },
    );
    let providers = rows
        .into_iter()
        .map(|row| UsageProvider {
            kind: provider_kind(&row.provider).into(),
            provider: row.provider,
            input_tokens: row.input_tokens,
            cached_input_tokens: row.cached_input_tokens,
            output_tokens: row.output_tokens,
            cost_usd: row.cost_usd,
        })
        .collect();
    (total, providers)
}

pub(super) async fn usage_page(
    State(state): State<Arc<DaemonState>>,
    Query(params): Query<UsageParams>,
) -> Response {
    let store = state.store.clone();
    // OpenCode Go only reveals its windows through a poll, so the daemon refreshes them here
    // before reading. The refresher's own freshness gate collapses repeated page loads into at
    // most one request every few minutes, which is also the cadence routing needs.
    crate::cli::commands::models::refresh_opencode_go_quota(&store).await;
    let result = tokio::task::spawn_blocking(move || -> anyhow::Result<UsageResponse> {
        let now = chrono::Utc::now().timestamp();
        let week_rows = store.usage_by_provider_since(now - 604800)?;
        let (combined, providers) = usage_providers(week_rows);
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
            .subscription_windows()?
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
    use super::{provider_kind, usage_pacing, usage_providers, UsageTotals};

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
    fn provider_kinds_preserve_client_grouping_contract() {
        assert_eq!(provider_kind("claude-cli"), "bridge");
        assert_eq!(provider_kind("codex-oauth"), "oauth");
        assert_eq!(provider_kind("gemini"), "api");
        assert_eq!(provider_kind("xai-oauth"), "oauth");
        assert_eq!(provider_kind("agy-cli"), "bridge");
        assert_eq!(provider_kind("openai"), "api");
    }

    #[test]
    fn provider_rows_sum_without_losing_cached_tokens_or_cost() {
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
                cost_usd: 0.75,
            }
        );
        assert_eq!(providers[0].kind, "api");
        assert_eq!(providers[1].kind, "bridge");
    }
}
