//! `GET /api/usage/activity` — per-day, per-provider totals behind the Usage Activity tab.

use std::sync::Arc;

use axum::extract::{Query, State};
use axum::response::Response;

use super::{err_response, json_response, DaemonState};

const MAX_DAYS: i64 = 400;

#[derive(serde::Deserialize)]
pub(super) struct ActivityParams {
    days: Option<i64>,
    #[serde(rename = "tzOffset")]
    tz_offset: Option<i64>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ActivityResponse {
    utc_offset_secs: i64,
    since_epoch: i64,
    days: Vec<forge_store::DayUsage>,
}

pub(super) async fn usage_activity_page(
    State(state): State<Arc<DaemonState>>,
    Query(params): Query<ActivityParams>,
) -> Response {
    let store = state.store.clone();
    let offset = params.tz_offset.unwrap_or(0).clamp(-14 * 3600, 14 * 3600);
    let window = params.days.unwrap_or(365).clamp(1, MAX_DAYS);
    let since = chrono::Utc::now().timestamp() - window * 86_400;
    let result = tokio::task::spawn_blocking(move || store.usage_by_day(since, offset)).await;
    match result {
        Ok(Ok(days)) => json_response(&ActivityResponse {
            utc_offset_secs: offset,
            since_epoch: since,
            days,
        }),
        _ => err_response(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "usage activity unavailable",
        ),
    }
}
