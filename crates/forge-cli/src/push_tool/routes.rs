//! `POST /api/notify` (used by non-daemon processes) and `GET /api/notices` (used by the app).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::Response;
use axum::routing::{get, post};
use axum::Router;
use forge_core::push_notify::{parse_args, ChannelReport, Limiter, NO_CHANNEL};

use super::{notices, Channels};
use crate::serve::{err_response, json_response, DaemonState};

/// A daemon-wide limiter, separate from each caller's own: a bridge child is a fresh process per
/// turn and cannot remember what an earlier one sent, so the daemon is the one place that can
/// bound a loop across them.
static LIMITER: Mutex<Option<Limiter>> = Mutex::new(None);

pub(crate) fn routes(base: &str) -> Router<Arc<DaemonState>> {
    Router::new()
        .route(&format!("{base}/api/notify"), post(notify))
        .route(&format!("{base}/api/notices"), get(list_notices))
}

pub(crate) fn channels_for(state: &DaemonState, ambient: bool) -> Channels {
    Channels {
        store: state.store.clone(),
        web: state.push.clone(),
        apns: state.apns.clone(),
        ambient,
    }
}

async fn notify(
    State(state): State<Arc<DaemonState>>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> Response {
    let session = body
        .get("session_id")
        .and_then(|v| v.as_str())
        .unwrap_or("api")
        .to_string();
    let request = match parse_args(&body, &session) {
        Ok(r) => r,
        Err(e) => return err_response(StatusCode::BAD_REQUEST, &e),
    };
    let verdict = LIMITER
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_or_insert_with(Limiter::default)
        .check(&session, &request.message, Instant::now());
    if let Err(refusal) = verdict {
        return err_response(StatusCode::TOO_MANY_REQUESTS, &refusal.text());
    }
    let reports = channels_for(&state, !cfg!(test)).deliver(&request).await;
    json_response(&serde_json::json!({
        "ok": reports.iter().any(|r| r.delivered),
        "channels": reports.iter().map(report_json).collect::<Vec<_>>(),
        "note": if reports.is_empty() { Some(NO_CHANNEL) } else { None },
    }))
}

fn report_json(r: &ChannelReport) -> serde_json::Value {
    serde_json::json!({ "channel": r.channel, "delivered": r.delivered, "detail": r.detail })
}

async fn list_notices(Query(q): Query<HashMap<String, String>>) -> Response {
    let after = q.get("after").and_then(|v| v.parse().ok()).unwrap_or(0);
    let (items, latest) = notices::since(after);
    json_response(&serde_json::json!({ "notices": items, "latest": latest }))
}
