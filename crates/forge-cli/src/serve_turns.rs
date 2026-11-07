//! How turns ended, for the apps (docs/features/session-replay.md, "Turn endings"):
//! `GET /api/sessions/{id}/turns` is the "Turn endings" sheet behind the turn-stopped banner and
//! `GET /api/turns/summary` the counts across a window. Plain labels and the way out ride along,
//! so no client prints a raw end-reason name.

use std::sync::Arc;

use axum::extract::{Path as AxumPath, Query, State};
use axum::http::StatusCode;
use axum::response::Response;
use axum::routing::get;
use axum::Router;
use forge_core::TurnEndView;
use forge_store::Store;

use crate::serve::{err_response, json_response, DaemonState};

const DEFAULT_LIMIT: usize = 20;
const MAX_LIMIT: usize = 100;
const DEFAULT_DAYS: i64 = 7;
const MAX_DAYS: i64 = 365;

pub(crate) fn routes(base: &str) -> Router<Arc<DaemonState>> {
    Router::new()
        .route(
            &format!("{base}/api/sessions/{{id}}/turns"),
            get(session_turns),
        )
        .route(&format!("{base}/api/turns/summary"), get(turns_summary))
}

#[derive(Debug, PartialEq, serde::Serialize)]
pub(crate) struct EndCount {
    reason: String,
    label: String,
    count: i64,
}

#[derive(Debug, PartialEq, serde::Serialize)]
pub(crate) struct SessionTurns {
    session_id: String,
    counts: Vec<EndCount>,
    turns: Vec<TurnEndView>,
}

#[derive(Debug, PartialEq, serde::Serialize)]
pub(crate) struct TurnsSummary {
    since: i64,
    days: i64,
    counts: Vec<EndCount>,
    unfinished: Vec<TurnEndView>,
}

fn counts(rows: Vec<(String, i64)>) -> Vec<EndCount> {
    rows.into_iter()
        .map(|(reason, count)| EndCount {
            label: forge_core::end_label(&reason),
            reason,
            count,
        })
        .collect()
}

pub(crate) fn build_session_turns(
    store: &Store,
    session_id: &str,
    limit: usize,
) -> anyhow::Result<SessionTurns> {
    Ok(SessionTurns {
        session_id: session_id.to_string(),
        counts: counts(store.turn_end_counts(0, Some(session_id))?),
        turns: store
            .recent_turns(session_id, limit)?
            .iter()
            .map(TurnEndView::from_row)
            .collect(),
    })
}

pub(crate) fn build_summary(
    store: &Store,
    now: i64,
    days: i64,
    session: Option<&str>,
    limit: usize,
) -> anyhow::Result<TurnsSummary> {
    let since = now - days * 86_400;
    Ok(TurnsSummary {
        since,
        days,
        counts: counts(store.turn_end_counts(since, session)?),
        unfinished: store
            .recent_unfinished_turns(since, session, limit)?
            .iter()
            .map(TurnEndView::from_row)
            .collect(),
    })
}

#[derive(serde::Deserialize, Default)]
struct TurnParams {
    limit: Option<usize>,
    days: Option<i64>,
    session: Option<String>,
}

async fn session_turns(
    State(state): State<Arc<DaemonState>>,
    AxumPath(id): AxumPath<String>,
    Query(params): Query<TurnParams>,
) -> Response {
    let store = state.store.clone();
    let limit = params.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let result = tokio::task::spawn_blocking(move || {
        if store.session_cwd(&id).ok().flatten().is_none() {
            return Ok(None);
        }
        build_session_turns(&store, &id, limit).map(Some)
    })
    .await;
    match result {
        Ok(Ok(Some(turns))) => json_response(&turns),
        Ok(Ok(None)) => err_response(StatusCode::NOT_FOUND, "no such session"),
        _ => err_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "could not read the turn endings",
        ),
    }
}

async fn turns_summary(
    State(state): State<Arc<DaemonState>>,
    Query(params): Query<TurnParams>,
) -> Response {
    let store = state.store.clone();
    let days = params.days.unwrap_or(DEFAULT_DAYS).clamp(1, MAX_DAYS);
    let limit = params.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let session = params.session.filter(|s| !s.is_empty());
    let result = tokio::task::spawn_blocking(move || {
        let now = chrono::Utc::now().timestamp();
        build_summary(&store, now, days, session.as_deref(), limit)
    })
    .await;
    match result {
        Ok(Ok(summary)) => json_response(&summary),
        _ => err_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "could not summarise the turn endings",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store_with_turns() -> (Store, String) {
        let store = Store::open_in_memory().unwrap();
        let sid = store.create_session("/repo", "default").unwrap();
        for (seq, reason) in [(1, "final_answer"), (3, "stalled"), (5, "budget_exhausted")] {
            let id = store.open_turn(&sid, seq).unwrap();
            store.close_turn(id, reason, None).unwrap();
        }
        (store, sid)
    }

    #[test]
    fn a_sessions_turns_carry_plain_labels_and_whether_continue_helps() {
        let (store, sid) = store_with_turns();
        let out = build_session_turns(&store, &sid, 10).unwrap();
        assert_eq!(out.turns.len(), 3);
        let stalled = out.turns.iter().find(|t| t.reason == "stalled").unwrap();
        assert_eq!(stalled.label, "no progress");
        assert!(stalled.resumable && !stalled.finished);
        let budget = out
            .turns
            .iter()
            .find(|t| t.reason == "budget_exhausted")
            .unwrap();
        assert!(!budget.resumable);
        let finished = out
            .counts
            .iter()
            .find(|c| c.reason == "final_answer")
            .unwrap();
        assert_eq!((finished.label.as_str(), finished.count), ("finished", 1));
        assert_eq!(build_session_turns(&store, &sid, 2).unwrap().turns.len(), 2);
    }

    #[test]
    fn the_summary_lists_only_turns_that_did_not_finish_inside_the_window() {
        let (store, sid) = store_with_turns();
        let now = chrono::Utc::now().timestamp();
        let all = build_summary(&store, now + 60, 7, None, 10).unwrap();
        assert_eq!(all.unfinished.len(), 2);
        assert!(all.unfinished.iter().all(|t| !t.finished));
        assert_eq!(all.counts.iter().map(|c| c.count).sum::<i64>(), 3);
        let other = build_summary(&store, now, 7, Some("nope"), 10).unwrap();
        assert!(other.counts.is_empty() && other.unfinished.is_empty());
        let mine = build_summary(&store, now, 7, Some(&sid), 10).unwrap();
        assert_eq!(mine.unfinished.len(), 2);
        let aged = build_summary(&store, now + 30 * 86_400, 7, None, 10).unwrap();
        assert!(aged.counts.is_empty());
    }
}
