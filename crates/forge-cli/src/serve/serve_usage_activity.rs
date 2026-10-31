//! `GET /api/usage/activity` — per-day, per-provider totals behind the Usage Activity tab.

use std::sync::Arc;

use axum::extract::{Query, State};
use axum::response::Response;

use crate::serve::{err_response, json_response, DaemonState};

const MAX_DAYS: i64 = 400;

#[derive(serde::Deserialize)]
pub(crate) struct ActivityParams {
    days: Option<i64>,
    #[serde(rename = "tzOffset")]
    tz_offset: Option<i64>,
    /// `epoch:offset,...` — the caller's UTC offset from each epoch onwards, so a window that
    /// crosses a DST change is bucketed with the offset that applied at the time.
    #[serde(rename = "tzChanges")]
    tz_changes: Option<String>,
}

const MAX_TZ_CHANGES: usize = 64;

fn parse_tz_changes(raw: &str) -> Vec<(i64, i64)> {
    let mut changes: Vec<(i64, i64)> = raw
        .split(',')
        .filter_map(|pair| {
            let (epoch, offset) = pair.split_once(':')?;
            Some((
                epoch.trim().parse().ok()?,
                offset
                    .trim()
                    .parse::<i64>()
                    .ok()?
                    .clamp(-14 * 3600, 14 * 3600),
            ))
        })
        .take(MAX_TZ_CHANGES)
        .collect();
    changes.sort_unstable_by_key(|(epoch, _)| *epoch);
    changes
}

/// The offset in force at `ts`: the last change at or before it, else `fallback`.
fn offset_at(changes: &[(i64, i64)], fallback: i64, ts: i64) -> i64 {
    changes
        .iter()
        .rev()
        .find(|(epoch, _)| *epoch <= ts)
        .map_or(fallback, |(_, offset)| *offset)
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ActivityResponse {
    utc_offset_secs: i64,
    since_epoch: i64,
    days: Vec<forge_store::DayUsage>,
}

pub(crate) async fn page(
    State(state): State<Arc<DaemonState>>,
    Query(params): Query<ActivityParams>,
) -> Response {
    let store = state.store.clone();
    let offset = params.tz_offset.unwrap_or(0).clamp(-14 * 3600, 14 * 3600);
    let window = params.days.unwrap_or(365).clamp(1, MAX_DAYS);
    let since = chrono::Utc::now().timestamp() - window * 86_400;
    let changes = params
        .tz_changes
        .as_deref()
        .map(parse_tz_changes)
        .unwrap_or_default();
    let result = tokio::task::spawn_blocking(move || {
        store.usage_by_day_with(since, |ts| offset_at(&changes, offset, ts))
    })
    .await;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_offset_follows_the_change_list() {
        let changes = parse_tz_changes("1800:3600,0:7200,bad,5000:9999999");
        assert_eq!(offset_at(&changes, 11, -5), 11);
        assert_eq!(offset_at(&changes, 11, 100), 7200);
        assert_eq!(offset_at(&changes, 11, 1800), 3600);
        assert_eq!(offset_at(&changes, 11, 6000), 14 * 3600);
        assert!(parse_tz_changes("").is_empty());
    }
}
