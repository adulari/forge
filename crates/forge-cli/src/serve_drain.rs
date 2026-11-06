//! Backlog drain: run several GitHub issues as one batch (`docs/features/issue-drain.md`).
//!
//! A drain is a queue of issue numbers plus a concurrency cap and an optional spend envelope.
//! Each item is started exactly like a single start (`serve_issues::start_in`: fresh-base worktree,
//! the issue quoted as untrusted data, PR Watch armed without raising the permission mode). The
//! next item starts when a running one stops needing a slot: its PR is open or it finished. A
//! session waiting on a prompt or question keeps its slot, so the queue never runs ahead of the
//! owner's answers. The envelope is the `/goal --budget` accounting
//! (`forge_types::goal_envelope`), baselined when the drain is created; once spent, nothing new
//! starts until the window resets, then the drain rebaselines and carries on.
//!
//! Routes (token-scoped, under `/api/issues/` so they need the workspace-write scope to change
//! anything):
//! - `POST /api/issues/drain`         `{session|cwd, numbers, parallel?, budget?, worktree?, mode?, method?}`
//! - `GET  /api/issues/drains[?id=]`  every drain (newest first), or one
//! - `POST /api/issues/drain/cancel`  `{id}`: stop starting items; running sessions are left alone
//!
//! State lives in `drains.json` next to `pr-watch.json`, so a restart picks up where it stopped.
//! [`advance`] is the whole scheduling decision as a pure function; [`sweep`] gathers what it
//! needs from the daemon and acts on its answer.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use axum::extract::{Json, Query, State};
use axum::http::StatusCode;
use axum::response::Response;
use axum::routing::{get, post};
use axum::Router;
use forge_types::goal_envelope::EnvelopeSpec;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::cli::commands::run::goal_envelope as envelope;
use crate::cli::commands::run::goal_envelope::EnvelopeSources;
use crate::serve::{err_response, json_response, DaemonState};
use crate::serve_issues::{project_root, request_root, start_in, StartRequest};
use crate::serve_pr::Gh;
use crate::serve_pr_watch::{Mode, Phase, Watch};

mod model;
pub(crate) use model::*;

const TICK: Duration = Duration::from_secs(4);
const MAX_ITEMS: usize = 30;
const MAX_PARALLEL: usize = 8;
const DEFAULT_PARALLEL: usize = 2;
const KEEP_DRAINS: usize = 20;
/// A session that has not begun a turn this long after it was started is idle, not still
/// starting; also how a session restored by a daemon restart (whose turn counter restarts at 0)
/// is told apart from one that has not picked up its prompt yet.
const START_GRACE_SECS: i64 = 120;

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

pub(crate) struct DrainRegistry {
    path: Option<PathBuf>,
    drains: Mutex<Vec<Drain>>,
    /// Held across a whole sweep or cancel, so two of them never start the same item.
    gate: tokio::sync::Mutex<()>,
}

impl DrainRegistry {
    pub(crate) fn new(path: Option<PathBuf>) -> Self {
        let drains = path
            .as_deref()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|bytes| serde_json::from_slice::<Vec<Drain>>(&bytes).ok())
            .unwrap_or_default();
        Self {
            path,
            drains: Mutex::new(drains),
            gate: tokio::sync::Mutex::new(()),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Drain>> {
        self.drains
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn persist(&self, drains: &[Drain]) {
        let Some(path) = &self.path else { return };
        let Ok(bytes) = serde_json::to_vec_pretty(drains) else {
            return;
        };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let temp = path.with_extension("tmp");
        if std::fs::write(&temp, bytes).is_ok() {
            let _ = std::fs::rename(&temp, path);
        }
    }

    /// Newest first.
    pub(crate) fn all(&self) -> Vec<Drain> {
        let mut rows = self.lock().clone();
        rows.sort_by(|a, b| b.created_at.cmp(&a.created_at).then(b.id.cmp(&a.id)));
        rows
    }

    pub(crate) fn get(&self, id: &str) -> Option<Drain> {
        self.lock().iter().find(|d| d.id == id).cloned()
    }

    fn insert(&self, drain: Drain) {
        let mut guard = self.lock();
        guard.push(drain);
        if guard.len() > KEEP_DRAINS {
            if let Some(at) = guard.iter().position(Drain::is_settled) {
                guard.remove(at);
            }
        }
        self.persist(&guard);
    }

    /// Change one drain; persists only when something changed. `None` when it does not exist.
    fn with<R>(&self, id: &str, change: impl FnOnce(&mut Drain) -> R) -> Option<(R, bool)> {
        let mut guard = self.lock();
        let drain = guard.iter_mut().find(|d| d.id == id)?;
        let before = drain.clone();
        let out = change(drain);
        let changed = *drain != before;
        if changed {
            self.persist(&guard);
        }
        Some((out, changed))
    }
}

static REGISTRY: OnceLock<Arc<DrainRegistry>> = OnceLock::new();

pub(crate) fn registry() -> &'static Arc<DrainRegistry> {
    REGISTRY.get_or_init(|| Arc::new(DrainRegistry::new(None)))
}

/// Load `drains.json`; called before the daemon listens so no request sees an unpersisted registry.
pub(crate) fn init() {
    if let Some(path) = forge_config::data_dir().map(|d| d.join("drains.json")) {
        let _ = REGISTRY.set(Arc::new(DrainRegistry::new(Some(path))));
    }
}

/// The scheduling loop. Started once the fleet has been restored, so a session that survived a
/// restart is found live rather than mistaken for one that ended.
pub(crate) async fn run(state: Arc<DaemonState>) {
    let gh = Gh::system();
    let mut tick = tokio::time::interval(TICK);
    loop {
        tick.tick().await;
        sweep_with_refresh(&state, &gh, registry()).await;
    }
}

async fn sweep_with_refresh(state: &Arc<DaemonState>, gh: &Gh, reg: &DrainRegistry) {
    if !reg.all().iter().any(Drain::wants_scheduling) {
        return;
    }
    if !state.mock {
        for spec in reg
            .all()
            .iter()
            .filter(|d| d.wants_scheduling())
            .filter_map(|d| d.envelope.as_ref().map(|e| e.spec.clone()))
        {
            envelope::refresh_quota_store(&state.store, &spec).await;
        }
    }
    let sources = EnvelopeSources::from_store(&state.store, 0.0);
    sweep(state, gh, reg, &sources).await;
}

/// One pass over every drain: refresh its items, apply the envelope, start what fits.
pub(crate) async fn sweep(
    state: &Arc<DaemonState>,
    gh: &Gh,
    reg: &DrainRegistry,
    sources: &EnvelopeSources,
) {
    let _gate = reg.gate.lock().await;
    for drain in reg.all() {
        if !drain.wants_scheduling() {
            continue;
        }
        let mut live = HashMap::new();
        let mut watches = HashMap::new();
        let mut spent_usd = 0.0;
        for item in &drain.items {
            let Some(session) = item.session_id.as_deref() else {
                continue;
            };
            if let Some(handle) = state.registry.get(session).await {
                let frame = handle.snapshot_rx.borrow();
                let s = &frame.snapshot;
                live.insert(
                    session.to_string(),
                    LiveSession {
                        busy: s.busy,
                        closed: s.closed,
                        waiting: s.permission_prompt.is_some() || s.question.is_some(),
                        turns_finished: s.turns_finished,
                        failed: s.last_turn_outcome.as_deref() == Some("failed"),
                    },
                );
            }
            if let Some(watch) = crate::serve_pr_watch::registry().get(session) {
                watches.insert(session.to_string(), WatchView::of(&watch));
            }
            spent_usd += state.store.session_cost(session).unwrap_or(0.0);
        }
        let now = sources.now;
        let Some((starts, mut changed)) = reg.with(&drain.id, |d| {
            advance(
                d,
                &Inputs {
                    live: &live,
                    watches: &watches,
                    sources,
                    spent_usd,
                    now,
                },
            )
        }) else {
            continue;
        };
        for number in starts {
            let outcome = start_one(state, gh, &drain, number).await;
            changed |= reg
                .with(&drain.id, |d| d.record_start(number, outcome, now))
                .is_some();
        }
        if changed {
            state.registry.notify_fleet();
        }
    }
}

async fn start_one(
    state: &Arc<DaemonState>,
    gh: &Gh,
    drain: &Drain,
    number: u64,
) -> Result<(String, String), String> {
    let request = StartRequest {
        session: None,
        cwd: Some(drain.cwd.clone()),
        number,
        worktree: drain.worktree,
        branch: None,
        mode: drain.mode,
        method: Some(drain.method.clone()),
    };
    match start_in(state, gh, request).await {
        Ok(value) => Ok((
            value["id"].as_str().unwrap_or_default().to_string(),
            value["title"].as_str().unwrap_or_default().to_string(),
        )),
        Err(response) => Err(response_error(*response).await),
    }
}

async fn response_error(response: Response) -> String {
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .unwrap_or_default();
    serde_json::from_slice::<Value>(&bytes)
        .ok()
        .and_then(|v| v["error"].as_str().map(str::to_string))
        .unwrap_or_else(|| format!("start failed ({status})"))
}

pub(crate) fn routes(base: &str) -> Router<Arc<DaemonState>> {
    Router::new()
        .route(&format!("{base}/api/issues/drain"), post(drain_create))
        .route(&format!("{base}/api/issues/drains"), get(drain_list))
        .route(
            &format!("{base}/api/issues/drain/cancel"),
            post(drain_cancel),
        )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateRequest {
    #[serde(default)]
    session: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
    numbers: Vec<u64>,
    #[serde(default)]
    parallel: Option<usize>,
    /// A `/goal --budget` envelope: `claude:30%w`, `codex:15%w`, `$5`.
    #[serde(default)]
    budget: Option<String>,
    #[serde(default = "default_true")]
    worktree: bool,
    #[serde(default = "default_mode")]
    mode: Mode,
    #[serde(default)]
    method: Option<String>,
}

fn default_true() -> bool {
    true
}

fn default_mode() -> Mode {
    Mode::Fix
}

const NO_WORKTREE_NOTE: &str =
    "parallel was set to 1: without a worktree every agent would share one checkout and branch";

fn bad(message: &str) -> Response {
    err_response(StatusCode::BAD_REQUEST, message)
}

static SEQ: AtomicU64 = AtomicU64::new(0);

fn new_id(now: i64) -> String {
    format!("drain-{now:x}-{:x}", SEQ.fetch_add(1, Ordering::Relaxed))
}

async fn drain_create(
    State(state): State<Arc<DaemonState>>,
    Json(request): Json<CreateRequest>,
) -> Response {
    let capped = !request.worktree && request.parallel.unwrap_or(DEFAULT_PARALLEL) > 1;
    match create(&state, registry(), request).await {
        Ok(drain) => {
            let state = state.clone();
            tokio::spawn(
                async move { sweep_with_refresh(&state, &Gh::system(), registry()).await },
            );
            let mut body = drain.to_json();
            if capped {
                body["note"] = json!(NO_WORKTREE_NOTE);
            }
            json_response(&body)
        }
        Err(response) => *response,
    }
}

async fn create(
    state: &Arc<DaemonState>,
    reg: &DrainRegistry,
    request: CreateRequest,
) -> Result<Drain, Box<Response>> {
    let mut numbers: Vec<u64> = Vec::new();
    for n in &request.numbers {
        if *n == 0 {
            return Err(Box::new(bad("issue numbers start at 1")));
        }
        if !numbers.contains(n) {
            numbers.push(*n);
        }
    }
    if numbers.is_empty() {
        return Err(Box::new(bad("name at least one issue to drain")));
    }
    if numbers.len() > MAX_ITEMS {
        return Err(Box::new(bad(&format!(
            "a drain takes at most {MAX_ITEMS} issues"
        ))));
    }
    let parallel = request.parallel.unwrap_or(DEFAULT_PARALLEL);
    if !(1..=MAX_PARALLEL).contains(&parallel) {
        return Err(Box::new(bad(&format!(
            "parallel must be between 1 and {MAX_PARALLEL}"
        ))));
    }
    let parallel = if request.worktree { parallel } else { 1 };
    let method = request.method.as_deref().unwrap_or("squash");
    if !matches!(method, "squash" | "merge" | "rebase" | "auto") {
        return Err(Box::new(bad(
            "method must be squash, merge, rebase or auto",
        )));
    }
    let spec = match request
        .budget
        .as_deref()
        .map(str::trim)
        .filter(|b| !b.is_empty())
    {
        Some(text) => Some(EnvelopeSpec::parse(text, None).map_err(|e| Box::new(bad(&e)))?),
        None => None,
    };
    let root = request_root(state, request.session.as_deref(), request.cwd.as_deref())
        .await
        .map_err(|(status, message)| Box::new(err_response(status, &message)))?;
    // A session in a worktree answers with the worktree; items start from the main checkout, the
    // only path `request_root(cwd)` accepts as a project.
    let cwd = project_root(&root).display().to_string();
    for other in reg
        .all()
        .iter()
        .filter(|d| d.cwd == cwd && d.wants_scheduling())
    {
        if let Some(n) = numbers.iter().find(|n| {
            other
                .items
                .iter()
                .any(|i| i.number == **n && !i.status.is_terminal())
        }) {
            return Err(Box::new(err_response(
                StatusCode::CONFLICT,
                &format!("issue #{n} is already in a running drain"),
            )));
        }
    }
    let envelope = match &spec {
        Some(spec) => Some(
            envelope::baseline_store(&state.store, spec.clone(), !state.mock)
                .await
                .map_err(|e| Box::new(bad(&e)))?,
        ),
        None => None,
    };
    let now = now_secs();
    let drain = Drain::new(
        new_id(now),
        cwd,
        &numbers,
        parallel,
        request.budget.as_deref().map(str::trim).map(str::to_string),
        envelope,
        (request.worktree, request.mode, method.to_string()),
        now,
    );
    reg.insert(drain.clone());
    state.registry.notify_fleet();
    Ok(drain)
}

#[derive(Deserialize)]
struct ListQuery {
    #[serde(default)]
    id: Option<String>,
}

async fn drain_list(Query(query): Query<ListQuery>) -> Response {
    let reg = registry();
    let drains: Vec<Value> = match query.id.as_deref() {
        Some(id) => reg.get(id).iter().map(Drain::to_json).collect(),
        None => reg.all().iter().map(Drain::to_json).collect(),
    };
    json_response(&json!({ "drains": drains }))
}

#[derive(Deserialize)]
struct CancelRequest {
    id: String,
}

async fn drain_cancel(
    State(state): State<Arc<DaemonState>>,
    Json(request): Json<CancelRequest>,
) -> Response {
    let reg = registry();
    let _gate = reg.gate.lock().await;
    let now = now_secs();
    match reg.with(&request.id, |d| d.cancel(now)) {
        Some((_, _)) => {
            state.registry.notify_fleet();
            json_response(&reg.get(&request.id).map_or(Value::Null, |d| d.to_json()))
        }
        None => err_response(StatusCode::NOT_FOUND, "no drain with that id"),
    }
}

/// The brief's drain line: the newest drain that is still going or changed since `since`.
pub(crate) fn brief_line(
    reg: &DrainRegistry,
    since: i64,
) -> Option<crate::serve_brief::DrainBrief> {
    reg.all()
        .into_iter()
        .find(|d| d.wants_scheduling() || d.updated_at >= since)
        .map(|d| crate::serve_brief::DrainBrief {
            id: d.id.clone(),
            line: d.line(),
            status: d.status.as_str().to_string(),
            queued: d.counts().queued,
        })
}

#[cfg(test)]
mod tests;
