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
        let quota = store
            .subscription_windows()?
            .into_iter()
            .map(|quota| UsageQuota {
                kind: provider_kind(&quota.provider).into(),
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
    use super::{provider_kind, usage_providers, UsageTotals};

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
