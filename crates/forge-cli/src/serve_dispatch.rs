//! Plan and dispatch over the daemon (`forge_core::dispatch`, docs/features/project-board.md):
//! the `/api/dispatch*` routes and their JSON, the scheduling pass that starts worker sessions, and
//! delivery of `[dispatch]` messages to the coordinator. The task that notices workers finishing
//! lives in [`supervisor`].
//!
//! Every state change of one dispatch runs under [`dispatch_lock`]: an approval, a cancel, a
//! proposal, and the supervisor recording a finished worker all read the row, decide, and write
//! while holding it, so two of them can never interleave their writes or start one item twice.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock};

use axum::extract::rejection::JsonRejection;
use axum::extract::{Path as AxumPath, Query, State};
use axum::http::StatusCode;
use axum::response::Response;
use axum::routing::{get, post};
use axum::Router;
use forge_core::dispatch::{
    self as plan, dispatch_status, item_status, DispatchPlan, SENDER_LABEL,
};
use forge_store::{DispatchRow, NewDispatchItem, Store};
use forge_types::PermissionMode;

use crate::remote;
use crate::serve::{
    deliver_pending_fleet_messages, err_response, json_response, start_session, CreateSessionReq,
    DaemonState, SessionRegistry,
};

mod messages;
mod start;
mod supervisor;
use messages::approval_text;
pub(crate) use messages::send_to_coordinator;
use start::start_dispatch;
pub(crate) use supervisor::run_supervisor;
#[cfg(test)]
use {messages::combine_messages, start::coordinator_session_title};

const MAX_DISPATCH_PROMPT_BYTES: usize = 16 * 1024;
const MAX_COORDINATOR_TITLE_CHARS: usize = 60;
/// `forge_store`'s per-sender cap on undelivered fleet messages and its body limit (not exported).
/// Dispatch folds into one message at the cap instead of losing an update for an idle coordinator.
const FLEET_PENDING_CAP: usize = 8;
const FLEET_MESSAGE_MAX_BYTES: usize = 16 * 1024;

pub(crate) fn routes(base: &str) -> Router<Arc<DaemonState>> {
    Router::new()
        .route(&format!("{base}/api/dispatch"), post(start_dispatch))
        .route(&format!("{base}/api/dispatches"), get(list_dispatches))
        .route(&format!("{base}/api/dispatches/{{id}}"), get(get_dispatch))
        .route(
            &format!("{base}/api/dispatches/{{id}}/proposal"),
            post(propose),
        )
        .route(
            &format!("{base}/api/dispatches/{{id}}/approve"),
            post(approve),
        )
        .route(
            &format!("{base}/api/dispatches/{{id}}/revise"),
            post(revise),
        )
        .route(
            &format!("{base}/api/dispatches/{{id}}/cancel"),
            post(cancel),
        )
}

pub(crate) fn dispatch_lock(id: &str) -> Arc<tokio::sync::Mutex<()>> {
    type Locks = std::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>;
    static LOCKS: LazyLock<Locks> = LazyLock::new(Locks::default);
    let mut locks = LOCKS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    // An entry only the map still holds is idle; dropping it cannot split a live critical section.
    locks.retain(|_, lock| Arc::strong_count(lock) > 1);
    locks.entry(id.to_string()).or_default().clone()
}

#[derive(Debug)]
pub(crate) enum DispatchError {
    NotFound,
    Conflict(String),
    Invalid(String),
    Internal(String),
}

impl DispatchError {
    fn response(self) -> Response {
        match self {
            Self::NotFound => err_response(
                StatusCode::NOT_FOUND,
                "no dispatch with that id — see GET /api/dispatches",
            ),
            Self::Conflict(m) => err_response(StatusCode::CONFLICT, &m),
            Self::Invalid(m) => err_response(StatusCode::BAD_REQUEST, &m),
            Self::Internal(m) => err_response(StatusCode::INTERNAL_SERVER_ERROR, &m),
        }
    }

    pub(crate) fn into_message(self) -> String {
        match self {
            Self::NotFound => "the dispatch no longer exists".to_string(),
            Self::Conflict(m) | Self::Invalid(m) | Self::Internal(m) => m,
        }
    }
}

impl From<forge_store::StoreError> for DispatchError {
    fn from(error: forge_store::StoreError) -> Self {
        Self::Internal(format!("dispatch store: {error}"))
    }
}

fn load(store: &Store, id: &str) -> Result<DispatchRow, DispatchError> {
    store.dispatch(id)?.ok_or(DispatchError::NotFound)
}

#[derive(Debug, serde::Serialize)]
pub(crate) struct DispatchJson {
    id: String,
    coordinator_session_id: String,
    coordinator_title: String,
    cwd: String,
    prompt: String,
    summary: String,
    status: String,
    worktree: bool,
    permission_mode: Option<String>,
    max_running: i64,
    max_items: i64,
    created_at: i64,
    updated_at: i64,
    items: Vec<ItemJson>,
}

#[derive(Debug, serde::Serialize)]
pub(crate) struct ItemJson {
    index: i64,
    title: String,
    prompt: String,
    depends_on: Vec<i64>,
    status: String,
    session_id: Option<String>,
    outcome: Option<String>,
    started_at: Option<i64>,
    finished_at: Option<i64>,
}

/// The coordinator's display title: its live handle's, else the stored one, else "".
pub(crate) async fn coordinator_title(state: &DaemonState, row: &DispatchRow) -> String {
    if let Some(handle) = state.registry.get(&row.coordinator_session_id).await {
        let title = handle.title();
        if !title.is_empty() {
            return title;
        }
    }
    state
        .store
        .session_title(&row.coordinator_session_id)
        .ok()
        .flatten()
        .unwrap_or_default()
}

async fn to_json(state: &DaemonState, row: DispatchRow) -> DispatchJson {
    let coordinator_title = coordinator_title(state, &row).await;
    DispatchJson {
        coordinator_title,
        id: row.id,
        coordinator_session_id: row.coordinator_session_id,
        cwd: row.cwd,
        prompt: row.prompt,
        summary: row.summary,
        status: row.status,
        worktree: row.worktree,
        permission_mode: row.permission_mode,
        max_running: row.max_running,
        max_items: row.max_items,
        created_at: row.created_at,
        updated_at: row.updated_at,
        items: row
            .items
            .into_iter()
            .map(|i| ItemJson {
                index: i.idx,
                title: i.title,
                prompt: i.prompt,
                depends_on: i.depends_on,
                status: i.status,
                session_id: i.session_id,
                outcome: i.outcome,
                started_at: i.started_at,
                finished_at: i.finished_at,
            })
            .collect(),
    }
}

/// The additive dispatch columns of a `GET /api/sessions` row.
#[derive(Debug, Default, serde::Serialize)]
pub(crate) struct SessionDispatchFields {
    dispatch_id: Option<String>,
    dispatch_role: Option<&'static str>,
    dispatch_index: Option<i64>,
}

pub(crate) fn session_dispatch_fields(store: &Store, session_id: &str) -> SessionDispatchFields {
    if let Ok(Some(d)) = store.dispatch_for_coordinator(session_id) {
        return SessionDispatchFields {
            dispatch_id: Some(d.id),
            dispatch_role: Some("coordinator"),
            dispatch_index: None,
        };
    }
    if let Ok(Some((d, idx))) = store.dispatch_item_for_session(session_id) {
        return SessionDispatchFields {
            dispatch_id: Some(d.id),
            dispatch_role: Some("worker"),
            dispatch_index: Some(idx),
        };
    }
    SessionDispatchFields::default()
}

#[derive(serde::Deserialize)]
pub(crate) struct ListParams {
    limit: Option<usize>,
}

async fn list_dispatches(
    State(state): State<Arc<DaemonState>>,
    Query(params): Query<ListParams>,
) -> Response {
    let limit = params.limit.unwrap_or(20).clamp(1, 100);
    match state.store.list_dispatches(limit) {
        Ok(rows) => {
            let mut out = Vec::with_capacity(rows.len());
            for row in rows {
                out.push(to_json(&state, row).await);
            }
            json_response(&out)
        }
        Err(error) => DispatchError::from(error).response(),
    }
}

async fn get_dispatch(
    State(state): State<Arc<DaemonState>>,
    AxumPath(id): AxumPath<String>,
) -> Response {
    match load(&state.store, &id) {
        Ok(row) => json_response(&to_json(&state, row).await),
        Err(error) => error.response(),
    }
}

/// Record a proposal (route §4 and the in-process coordinator host). `parse` receives the
/// dispatch's `max_items` so both callers validate against the stored limit.
pub(crate) async fn record_proposal(
    store: &Store,
    id: &str,
    parse: impl FnOnce(usize) -> Result<DispatchPlan, String>,
) -> Result<DispatchRow, DispatchError> {
    let lock = dispatch_lock(id);
    let _held = lock.lock().await;
    let row = load(store, id)?;
    if !matches!(
        row.status.as_str(),
        dispatch_status::PLANNING | dispatch_status::PROPOSED
    ) {
        return Err(DispatchError::Conflict(format!(
            "the dispatch is {}; a split can only be proposed while it is planning or proposed",
            row.status
        )));
    }
    let proposal = parse(row.max_items.max(1) as usize).map_err(DispatchError::Invalid)?;
    let items: Vec<NewDispatchItem> = proposal
        .items
        .iter()
        .map(|i| NewDispatchItem {
            title: i.title.clone(),
            prompt: i.prompt.clone(),
            depends_on: i.depends_on.iter().map(|d| *d as i64).collect(),
        })
        .collect();
    store
        .replace_dispatch_proposal(id, &proposal.summary, &items)
        .map_err(|error| match error {
            forge_store::StoreError::InvalidValue(message) => DispatchError::Conflict(message),
            other => other.into(),
        })?;
    load(store, id)
}

async fn propose(
    State(state): State<Arc<DaemonState>>,
    AxumPath(id): AxumPath<String>,
    body: Result<axum::Json<serde_json::Value>, JsonRejection>,
) -> Response {
    let body = match body {
        Ok(axum::Json(body)) => body,
        Err(error) => {
            return err_response(
                StatusCode::BAD_REQUEST,
                &format!(
                    "invalid proposal: {} — send {{\"summary\", \"items\": [{{\"title\", \"prompt\", \"depends_on\"?}}]}}",
                    error.body_text()
                ),
            )
        }
    };
    match record_proposal(&state.store, &id, |max| plan::parse_plan(&body, max)).await {
        Ok(row) => {
            state.registry.notify_fleet();
            json_response(&to_json(&state, row).await)
        }
        Err(error) => error.response(),
    }
}

#[derive(serde::Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct ApproveReq {
    #[serde(default)]
    selected: Option<Vec<usize>>,
}

async fn approve(
    State(state): State<Arc<DaemonState>>,
    AxumPath(id): AxumPath<String>,
    body: axum::body::Bytes,
) -> Response {
    let req: ApproveReq = if body.iter().all(u8::is_ascii_whitespace) {
        ApproveReq::default()
    } else {
        match serde_json::from_slice(&body) {
            Ok(req) => req,
            Err(error) => {
                return err_response(
                    StatusCode::BAD_REQUEST,
                    &format!(
                        "invalid approval: {error} — send {{\"selected\": [item numbers]}}, or no \
                         body to approve every item"
                    ),
                )
            }
        }
    };
    let lock = dispatch_lock(&id);
    let held = lock.lock().await;
    let result = approve_locked(&state, &id, req.selected.as_deref()).await;
    drop(held);
    match result {
        Ok(row) => json_response(&to_json(&state, row).await),
        Err(error) => error.response(),
    }
}

async fn approve_locked(
    state: &DaemonState,
    id: &str,
    selected: Option<&[usize]>,
) -> Result<DispatchRow, DispatchError> {
    let row = load(&state.store, id)?;
    if row.status != dispatch_status::PROPOSED {
        return Err(DispatchError::Conflict(format!(
            "the dispatch is {}; only a proposed split can be approved",
            row.status
        )));
    }
    let deps: Vec<Vec<usize>> = row
        .items
        .iter()
        .map(|i| i.depends_on.iter().map(|d| *d as usize).collect())
        .collect();
    let approval = plan::resolve_selection(&deps, selected).map_err(DispatchError::Invalid)?;
    let store = &state.store;
    for (indices, status) in [
        (&approval.skipped, item_status::SKIPPED),
        (&approval.dropped_for_deps, item_status::CANCELLED),
        (&approval.queued, item_status::QUEUED),
    ] {
        for idx in indices {
            store.set_dispatch_item(id, *idx as i64, status, None, None)?;
        }
    }
    store.set_dispatch_status(id, dispatch_status::RUNNING)?;
    let advanced = advance_locked(state, id).await?;
    let row = load(store, id)?;
    let text = approval_text(&row, &advanced);
    send_to_coordinator(store, &state.registry, &row.coordinator_session_id, &text).await;
    state.registry.notify_fleet();
    Ok(row)
}

/// What one scheduling pass did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct Advance {
    pub(crate) started: Vec<(i64, String)>,
    pub(crate) start_failed: Vec<(i64, String)>,
    pub(crate) cancelled: Vec<i64>,
}

impl Advance {
    fn is_empty(&self) -> bool {
        self.started.is_empty() && self.start_failed.is_empty() && self.cancelled.is_empty()
    }
}

/// Start the queued items that can start and cancel the ones that never will, until nothing
/// changes (a failed start frees its slot and may cancel its dependents). The caller holds the
/// dispatch lock, which is what keeps two passes from starting one item twice.
pub(crate) async fn advance_locked(
    state: &DaemonState,
    id: &str,
) -> Result<Advance, DispatchError> {
    let mut out = Advance::default();
    loop {
        let row = load(&state.store, id)?;
        if row.status != dispatch_status::RUNNING {
            break;
        }
        let states: Vec<plan::ItemState> = row
            .items
            .iter()
            .map(|i| plan::ItemState {
                index: i.idx as usize,
                status: i.status.clone(),
                depends_on: i.depends_on.iter().map(|d| *d as usize).collect(),
            })
            .collect();
        let next = plan::schedule(&states, row.max_running.max(1) as usize);
        if next.start.is_empty() && next.cancel.is_empty() {
            break;
        }
        for idx in next.cancel {
            state
                .store
                .set_dispatch_item(id, idx as i64, item_status::CANCELLED, None, None)?;
            out.cancelled.push(idx as i64);
        }
        for idx in next.start {
            let idx = idx as i64;
            match start_worker(state, &row, idx).await {
                Ok(session_id) => out.started.push((idx, session_id)),
                Err(error) => {
                    state.store.set_dispatch_item(
                        id,
                        idx,
                        item_status::FAILED,
                        None,
                        Some(&error),
                    )?;
                    out.start_failed.push((idx, error));
                }
            }
        }
    }
    Ok(out)
}

async fn start_worker(state: &DaemonState, row: &DispatchRow, idx: i64) -> Result<String, String> {
    let item = row
        .items
        .iter()
        .find(|i| i.idx == idx)
        .ok_or_else(|| format!("item {idx} is not part of the dispatch"))?;
    let req = CreateSessionReq {
        cwd: Some(row.cwd.clone()),
        worktree: row.worktree,
        title: Some(item.title.clone()),
        mode: row.permission_mode.clone(),
        dispatch_worker: true,
        ..Default::default()
    };
    let handle = start_session(state, req)
        .await
        .map_err(|error| format!("could not start its session: {}", error.message))?;
    state
        .store
        .set_dispatch_item(
            &row.id,
            idx,
            item_status::RUNNING,
            Some(&handle.session_id),
            None,
        )
        .map_err(|error| error.to_string())?;
    let coordinator_title = coordinator_title(state, row).await;
    let siblings: Vec<(usize, &str)> = row
        .items
        .iter()
        .filter(|i| i.idx != idx)
        .map(|i| (i.idx as usize, i.title.as_str()))
        .collect();
    let prompt = plan::worker_prompt(&plan::WorkerContext {
        summary: &row.summary,
        coordinator_title: &coordinator_title,
        coordinator_id: &row.coordinator_session_id,
        index: idx as usize,
        total: row.items.len(),
        title: &item.title,
        prompt: &item.prompt,
        siblings: &siblings,
        worktree: row.worktree,
    });
    // A driver that already stopped is noticed by the supervisor (no live handle → stopped).
    let _ = handle
        .input_tx
        .send(remote::RemoteInput::Prompt {
            text: prompt,
            attachments: Vec::new(),
        })
        .await;
    Ok(handle.session_id.clone())
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ReviseReq {
    feedback: String,
}

async fn revise(
    State(state): State<Arc<DaemonState>>,
    AxumPath(id): AxumPath<String>,
    body: Result<axum::Json<ReviseReq>, JsonRejection>,
) -> Response {
    let feedback = match body {
        Ok(axum::Json(req)) => req.feedback.trim().to_string(),
        Err(error) => {
            return err_response(
                StatusCode::BAD_REQUEST,
                &format!(
                    "invalid revision: {} — send {{\"feedback\": \"what to change\"}}",
                    error.body_text()
                ),
            )
        }
    };
    if feedback.is_empty() {
        return err_response(
            StatusCode::BAD_REQUEST,
            "feedback must not be empty: say what should change in the split",
        );
    }
    let lock = dispatch_lock(&id);
    let held = lock.lock().await;
    let result = async {
        let row = load(&state.store, &id)?;
        if row.status != dispatch_status::PROPOSED {
            return Err(DispatchError::Conflict(format!(
                "the dispatch is {}; only a proposed split can be revised",
                row.status
            )));
        }
        state
            .store
            .set_dispatch_status(&id, dispatch_status::PLANNING)?;
        let text = plan::revise_message(&feedback);
        send_to_coordinator(
            &state.store,
            &state.registry,
            &row.coordinator_session_id,
            &text,
        )
        .await;
        state.registry.notify_fleet();
        load(&state.store, &id)
    }
    .await;
    drop(held);
    match result {
        Ok(row) => json_response(&to_json(&state, row).await),
        Err(error) => error.response(),
    }
}

async fn cancel(State(state): State<Arc<DaemonState>>, AxumPath(id): AxumPath<String>) -> Response {
    let lock = dispatch_lock(&id);
    let held = lock.lock().await;
    let result = async {
        let row = load(&state.store, &id)?;
        if dispatch_status::is_terminal(&row.status) {
            return Err(DispatchError::Conflict(format!(
                "the dispatch is already {}",
                row.status
            )));
        }
        let mut running = 0;
        for item in &row.items {
            if item.status == item_status::RUNNING {
                running += 1;
            } else if !item_status::is_terminal(&item.status) {
                state
                    .store
                    .set_dispatch_item(&id, item.idx, item_status::CANCELLED, None, None)?;
            }
        }
        state
            .store
            .set_dispatch_status(&id, dispatch_status::CANCELLED)?;
        let text = plan::cancelled_message(running);
        send_to_coordinator(
            &state.store,
            &state.registry,
            &row.coordinator_session_id,
            &text,
        )
        .await;
        state.registry.notify_fleet();
        load(&state.store, &id)
    }
    .await;
    drop(held);
    match result {
        Ok(row) => json_response(&to_json(&state, row).await),
        Err(error) => error.response(),
    }
}

/// A worker's worktree was merged or discarded (`POST /api/sessions/{id}/merge|discard`).
pub(crate) async fn worker_left(state: &DaemonState, session_id: &str, status: &str) {
    let Ok(Some((row, idx))) = state.store.dispatch_item_for_session(session_id) else {
        return;
    };
    let lock = dispatch_lock(&row.id);
    let _held = lock.lock().await;
    if state
        .store
        .set_dispatch_item(&row.id, idx, status, None, None)
        .is_ok()
    {
        state.registry.notify_fleet();
    }
}

#[cfg(test)]
mod tests;
