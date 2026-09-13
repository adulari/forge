//! Bridge-side `dispatch_sessions`: a coordinator whose model runs on a CLI bridge executes its
//! tools here, in `forge mcp-serve`, a separate process from the daemon that owns the dispatch. So
//! the proposal travels the daemon's own route (`POST /api/dispatches/{id}/proposal`), with the
//! same discovery and auth as `forge send` and `mcp_serve::fleet`.
//!
//! The tool is advertised only to the session that coordinates a dispatch still open to a
//! proposal, found by asking the daemon (`GET /api/dispatches`) with the session id the bridge
//! exports as `FORGE_CHECKPOINT_SESSION`. The lookup is bounded and fails closed: an ordinary
//! bridge session, a dispatched worker, or a bridge with no reachable daemon never sees the tool,
//! so a slow daemon cannot stall tool listing and no other session can start a dispatch.
//!
//! The plan is validated locally with `forge_core::dispatch::parse_plan` before anything is sent,
//! so a bridge model reads exactly the error texts a direct-API coordinator reads.

use std::sync::Arc;
use std::time::Duration;

use forge_core::dispatch::{
    dispatch_sessions_spec, dispatch_status, parse_plan, proposal_recorded_result, ProposalReceipt,
    DEFAULT_MAX_ITEMS,
};
use rmcp::model::{CallToolResult, ContentBlock, JsonObject, Tool};
use serde_json::{json, Value};

use super::ForgeMcp;
use crate::cli::commands::dispatch::{Daemon, DaemonError, DispatchView};

/// Tool listing waits at most this long for the daemon, total.
const ADVERTISE_TIMEOUT: Duration = Duration::from_secs(1);
/// A proposal is one small write; anything slower is a daemon in trouble the model should hear about.
const PROPOSE_TIMEOUT: Duration = Duration::from_secs(15);
/// The dispatches route's own ceiling; a coordinator's dispatch is recent, so it is in the page.
const LOOKUP_LIMIT: usize = 100;

/// The dispatch `session_id` coordinates that still accepts a proposal, if any. The list arrives
/// most recently updated first, so the first match is the live one.
pub(super) fn awaiting_proposal<'a>(
    dispatches: &'a [DispatchView],
    session_id: &str,
) -> Option<&'a DispatchView> {
    if session_id.trim().is_empty() {
        return None;
    }
    dispatches.iter().find(|d| {
        d.coordinator_session_id == session_id
            && (d.status == dispatch_status::PLANNING || d.status == dispatch_status::PROPOSED)
    })
}

fn max_items_of(dispatch: &DispatchView) -> usize {
    match dispatch.max_items {
        0 => DEFAULT_MAX_ITEMS,
        n => n as usize,
    }
}

fn bridge_session_id() -> Option<String> {
    std::env::var(forge_core::snapshot::ENV_SESSION)
        .ok()
        .filter(|id| !id.trim().is_empty())
}

fn local_daemon(timeout: Duration) -> Result<Daemon, String> {
    let token = crate::attach::resolve_token(None).map_err(|e| e.to_string())?;
    let http = reqwest::Client::builder()
        .connect_timeout(timeout)
        .timeout(timeout)
        .build()
        .map_err(|e| e.to_string())?;
    Ok(Daemon::new(
        http,
        crate::attach::resolve_base_url(None),
        token,
    ))
}

pub(super) async fn coordinated_dispatch(
    daemon: &Daemon,
    session_id: &str,
) -> Result<Option<DispatchView>, DaemonError> {
    let dispatches = daemon.list(LOOKUP_LIMIT).await?;
    Ok(awaiting_proposal(&dispatches, session_id).cloned())
}

/// Validate `args` and record them as `session_id`'s proposal. `Err` is the text after `error: `.
pub(super) async fn propose(
    daemon: &Daemon,
    session_id: &str,
    args: &Value,
) -> Result<String, String> {
    let dispatch = coordinated_dispatch(daemon, session_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| {
            "this session has no dispatch waiting for a proposal — the split was already \
             approved or cancelled, so it can no longer change"
                .to_string()
        })?;
    let plan = parse_plan(args, max_items_of(&dispatch))?;
    let body = json!({ "summary": plan.summary, "items": plan.items });
    daemon
        .post(&format!("dispatches/{}/proposal", dispatch.id), &body)
        .await
        .map_err(|e| match e {
            DaemonError::Refused { status, message } => {
                format!("the daemon did not record the proposal ({status}): {message}")
            }
            other => other.to_string(),
        })?;
    Ok(proposal_recorded_result(&ProposalReceipt {
        dispatch_id: dispatch.id,
        items: plan.items.len(),
    }))
}

impl ForgeMcp {
    /// `dispatch_sessions`, when this bridge's session coordinates a dispatch open to a proposal.
    pub(super) async fn dispatch_sessions_tool(&self) -> Option<Tool> {
        let session_id = bridge_session_id()?;
        let daemon = local_daemon(ADVERTISE_TIMEOUT).ok()?;
        let lookup = tokio::time::timeout(
            ADVERTISE_TIMEOUT,
            coordinated_dispatch(&daemon, &session_id),
        );
        let dispatch = lookup.await.ok()?.ok()??;
        let spec = dispatch_sessions_spec(max_items_of(&dispatch));
        let schema: JsonObject = spec.schema.as_object().cloned().unwrap_or_default();
        Some(Tool::new(spec.name, spec.description, Arc::new(schema)))
    }

    pub(super) async fn handle_dispatch_sessions(&self, args: &Value) -> CallToolResult {
        let Some(session_id) = bridge_session_id() else {
            return CallToolResult::error(vec![ContentBlock::text(
                "error: dispatch_sessions is only available to a dispatch coordinator session",
            )]);
        };
        let daemon = match local_daemon(PROPOSE_TIMEOUT) {
            Ok(daemon) => daemon,
            Err(e) => {
                return CallToolResult::error(vec![ContentBlock::text(format!("error: {e}"))])
            }
        };
        match propose(&daemon, &session_id, args).await {
            Ok(text) => CallToolResult::success(vec![ContentBlock::text(text)]),
            Err(e) => CallToolResult::error(vec![ContentBlock::text(format!("error: {e}"))]),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    use axum::extract::{Path, State};
    use axum::http::StatusCode;
    use axum::response::{IntoResponse as _, Response};
    use axum::routing::{get, post};
    use axum::{Json, Router};

    use super::*;

    const TOKEN: &str = "tok";

    #[derive(Default)]
    struct Fixture {
        proposals: AtomicUsize,
        last_body: Mutex<Option<Value>>,
    }

    fn dispatch_json(id: &str, coordinator: &str, status: &str, max_items: u64) -> Value {
        json!({
            "id": id, "coordinator_session_id": coordinator, "coordinator_title": "Dispatch: x",
            "cwd": "/repo", "prompt": "x", "summary": "", "status": status, "worktree": true,
            "permission_mode": "accept-edits", "max_running": 4, "max_items": max_items,
            "created_at": 1, "updated_at": 2, "items": []
        })
    }

    async fn serve(fixture: Arc<Fixture>) -> String {
        let app = Router::new()
            .route(&format!("/{TOKEN}/api/dispatches"), get(list))
            .route(
                &format!("/{TOKEN}/api/dispatches/{{id}}/proposal"),
                post(proposal),
            )
            .with_state(fixture);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        format!("http://{addr}")
    }

    async fn list() -> Json<Value> {
        Json(json!([
            dispatch_json("d-plan", "coord-1", "planning", 3),
            dispatch_json("d-run", "coord-2", "running", 8),
            dispatch_json("d-raced", "coord-3", "proposed", 8),
        ]))
    }

    /// `d-raced` was approved between the lookup and the proposal — the daemon's real 409.
    async fn proposal(
        State(fixture): State<Arc<Fixture>>,
        Path(id): Path<String>,
        Json(body): Json<Value>,
    ) -> Response {
        fixture.proposals.fetch_add(1, Ordering::SeqCst);
        *fixture.last_body.lock().unwrap() = Some(body.clone());
        if id == "d-raced" {
            return (
                StatusCode::CONFLICT,
                Json(json!({ "error": "dispatch d-raced is running; its proposal can no longer change" })),
            )
                .into_response();
        }
        let mut reply = dispatch_json(&id, "coord-1", "proposed", 3);
        reply["summary"] = body["summary"].clone();
        Json(reply).into_response()
    }

    fn daemon(base: String, token: &str) -> Daemon {
        Daemon::new(reqwest::Client::new(), base, token.to_string())
    }

    fn plan(items: usize) -> Value {
        let items: Vec<Value> = (1..=items)
            .map(|i| json!({ "title": format!("part {i}"), "prompt": format!("do part {i}") }))
            .collect();
        json!({ "summary": "Split in parts.", "items": items })
    }

    fn views() -> Vec<DispatchView> {
        serde_json::from_value(json!([
            dispatch_json("d-done", "coord-1", "done", 8),
            dispatch_json("d-plan", "coord-1", "planning", 8),
            dispatch_json("d-prop", "coord-2", "proposed", 8),
            dispatch_json("d-run", "coord-3", "running", 8),
            dispatch_json("d-cancel", "coord-4", "cancelled", 8),
        ]))
        .unwrap()
    }

    #[test]
    fn advertised_only_to_the_coordinator_of_a_dispatch_open_to_a_proposal() {
        let dispatches = views();
        assert_eq!(
            awaiting_proposal(&dispatches, "coord-1").unwrap().id,
            "d-plan"
        );
        assert_eq!(
            awaiting_proposal(&dispatches, "coord-2").unwrap().id,
            "d-prop"
        );
        assert!(
            awaiting_proposal(&dispatches, "coord-3").is_none(),
            "running"
        );
        assert!(
            awaiting_proposal(&dispatches, "coord-4").is_none(),
            "cancelled"
        );
        assert!(
            awaiting_proposal(&dispatches, "worker-9").is_none(),
            "not a coordinator"
        );
        assert!(awaiting_proposal(&dispatches, "").is_none());
        assert!(awaiting_proposal(&[], "coord-1").is_none());
    }

    #[tokio::test]
    async fn an_unreachable_daemon_fails_closed() {
        let lookup =
            coordinated_dispatch(&daemon("http://127.0.0.1:1".into(), TOKEN), "coord-1").await;
        assert!(matches!(lookup, Err(DaemonError::Unreachable { .. })));
    }

    #[tokio::test]
    async fn a_wrong_token_reads_as_a_rejected_token() {
        let base = serve(Arc::new(Fixture::default())).await;
        let lookup = coordinated_dispatch(&daemon(base, "wrong"), "coord-1").await;
        assert!(matches!(lookup, Err(DaemonError::TokenRejected)));
    }

    #[tokio::test]
    async fn a_valid_plan_is_recorded_and_the_model_reads_the_receipt() {
        let fixture = Arc::new(Fixture::default());
        let base = serve(fixture.clone()).await;
        let text = propose(&daemon(base, TOKEN), "coord-1", &plan(2))
            .await
            .unwrap();
        assert_eq!(
            text,
            proposal_recorded_result(&ProposalReceipt {
                dispatch_id: "d-plan".into(),
                items: 2
            })
        );
        assert_eq!(fixture.proposals.load(Ordering::SeqCst), 1);
        let body = fixture.last_body.lock().unwrap().clone().unwrap();
        assert_eq!(body["summary"], "Split in parts.");
        assert_eq!(body["items"].as_array().unwrap().len(), 2);
        assert_eq!(body["items"][1]["title"], "part 2");
    }

    #[tokio::test]
    async fn a_conflict_reaches_the_model_verbatim() {
        let fixture = Arc::new(Fixture::default());
        let base = serve(fixture.clone()).await;
        let error = propose(&daemon(base, TOKEN), "coord-3", &plan(1))
            .await
            .unwrap_err();
        assert!(error.contains("409"), "{error}");
        assert!(
            error.contains("its proposal can no longer change"),
            "{error}"
        );
        assert_eq!(fixture.proposals.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn an_invalid_plan_never_reaches_the_daemon() {
        let fixture = Arc::new(Fixture::default());
        let base = serve(fixture.clone()).await;
        let d = daemon(base, TOKEN);

        let empty = propose(&d, "coord-1", &json!({ "summary": "s", "items": [] })).await;
        assert_eq!(
            empty.unwrap_err(),
            parse_plan(&json!({ "summary": "s", "items": [] }), 3).unwrap_err()
        );

        // The dispatch's own max_items (3) applies, not the default.
        let too_many = propose(&d, "coord-1", &plan(4)).await.unwrap_err();
        assert!(too_many.contains("at most 3"), "{too_many}");

        assert_eq!(fixture.proposals.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn a_session_without_an_open_dispatch_is_told_why() {
        let fixture = Arc::new(Fixture::default());
        let base = serve(fixture.clone()).await;
        let error = propose(&daemon(base, TOKEN), "coord-2", &plan(1))
            .await
            .unwrap_err();
        assert!(
            error.contains("no dispatch waiting for a proposal"),
            "{error}"
        );
        assert_eq!(fixture.proposals.load(Ordering::SeqCst), 0);
    }
}
