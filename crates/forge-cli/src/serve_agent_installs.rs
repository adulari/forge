//! `/api/agent-installs`: install and update the subscription CLIs (`claude`, `codex`, `agy`) from
//! a client, with installed-vs-latest and per-agent progress.
//!
//! `GET` reports each CLI plus the state of the newest job started for it. `POST .../{id}/run`
//! starts the vendor's installer or updater and returns at once; clients poll `GET` for the
//! streamed output. Like replacing Forge's own binary, running an installer is loopback-only:
//! the token gates every route, but executing code from the network should never be reachable
//! through a tunnel or LAN listener.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use axum::extract::{Path, Query};
use axum::http::StatusCode;
use axum::response::Response;
use axum::routing::{get, post};
use axum::Router;
use forge_provider::agent_install::{self, Action, AgentStatus};
use forge_provider::CliKind;
use serde::{Deserialize, Serialize};

use crate::serve::{err_response, json_response, DaemonState};
use crate::serve_update::Direct;

const LINE_KEEP: usize = 40;
/// The registry lookup is cached so a polling client cannot hammer it.
const LATEST_TTL: Duration = Duration::from_secs(15 * 60);

pub(crate) fn routes(base: &str) -> Router<Arc<DaemonState>> {
    Router::new()
        .route(&format!("{base}/api/agent-installs"), get(list))
        .route(
            &format!("{base}/api/agent-installs/update-all"),
            post(update_all),
        )
        .route(
            &format!("{base}/api/agent-installs/{{id}}/run"),
            post(run_one),
        )
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct Job {
    /// `queued`, `running`, `done` or `failed`.
    pub state: &'static str,
    pub action: Option<Action>,
    pub lines: Vec<String>,
    pub error: Option<String>,
    pub before: Option<String>,
    pub after: Option<String>,
}

impl Job {
    fn queued() -> Self {
        Self {
            state: "queued",
            action: None,
            lines: Vec::new(),
            error: None,
            before: None,
            after: None,
        }
    }

    fn active(&self) -> bool {
        matches!(self.state, "queued" | "running")
    }
}

fn jobs() -> &'static Mutex<HashMap<&'static str, Job>> {
    static JOBS: OnceLock<Mutex<HashMap<&'static str, Job>>> = OnceLock::new();
    JOBS.get_or_init(Mutex::default)
}

/// Mark `id` queued unless a job for it is already active. `false` means someone else holds it.
fn claim(id: &'static str) -> bool {
    let mut map = jobs().lock().unwrap_or_else(|e| e.into_inner());
    if map.get(id).is_some_and(Job::active) {
        return false;
    }
    map.insert(id, Job::queued());
    true
}

fn update_job(id: &'static str, change: impl FnOnce(&mut Job)) {
    let mut map = jobs().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(job) = map.get_mut(id) {
        change(job);
    }
}

fn install_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(Default::default)
}

async fn run_job(kind: CliKind) {
    let id = kind.prefix();
    // One installer at a time: two global npm installs racing in one prefix corrupt it.
    let _one_at_a_time = install_lock().lock().await;
    update_job(id, |job| job.state = "running");
    let result = agent_install::apply(kind, |line| {
        update_job(id, |job| {
            job.lines.push(line);
            let excess = job.lines.len().saturating_sub(LINE_KEEP);
            job.lines.drain(..excess);
        });
    })
    .await;
    // A fresh registry answer is wanted next, since the version just changed.
    latest_cache()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take();
    update_job(id, |job| match result {
        Ok(applied) => {
            job.state = "done";
            job.action = Some(applied.action);
            job.before = applied.before;
            job.after = applied.after;
        }
        Err(error) => {
            job.state = "failed";
            job.error = Some(error);
        }
    });
}

type LatestCache = Option<(Instant, HashMap<&'static str, Option<String>>)>;

fn latest_cache() -> &'static Mutex<LatestCache> {
    static CACHE: OnceLock<Mutex<LatestCache>> = OnceLock::new();
    CACHE.get_or_init(Mutex::default)
}

async fn statuses(refresh: bool) -> Vec<AgentStatus> {
    let mut statuses = agent_install::statuses(false).await;
    let cached = {
        let cache = latest_cache().lock().unwrap_or_else(|e| e.into_inner());
        cache
            .as_ref()
            .filter(|(at, _)| !refresh && at.elapsed() < LATEST_TTL)
            .map(|(_, map)| map.clone())
    };
    let latest = match cached {
        Some(map) => map,
        None => {
            let fetched: HashMap<&'static str, Option<String>> =
                futures::future::join_all(CliKind::all().map(|kind| async move {
                    (kind.prefix(), agent_install::latest_version(kind).await)
                }))
                .await
                .into_iter()
                .collect();
            *latest_cache().lock().unwrap_or_else(|e| e.into_inner()) =
                Some((Instant::now(), fetched.clone()));
            fetched
        }
    };
    for status in &mut statuses {
        status.latest = latest.get(status.id).cloned().flatten();
        status.outdated = match (&status.version, &status.latest) {
            (Some(v), Some(l)) => agent_install::is_older(v, l),
            _ => false,
        };
    }
    statuses
}

#[derive(Serialize)]
struct Row {
    #[serde(flatten)]
    status: AgentStatus,
    job: Option<Job>,
}

#[derive(Serialize)]
struct ListResponse {
    agents: Vec<Row>,
    outdated: usize,
    /// Whether this request may start installs (loopback only).
    can_run: bool,
}

#[derive(Deserialize)]
struct ListQuery {
    #[serde(default)]
    refresh: Option<String>,
}

async fn list(Direct(local): Direct, Query(query): Query<ListQuery>) -> Response {
    let statuses = statuses(query.refresh.is_some()).await;
    let outdated = statuses.iter().filter(|s| s.outdated).count();
    let map = jobs().lock().unwrap_or_else(|e| e.into_inner()).clone();
    let agents = statuses
        .into_iter()
        .map(|status| Row {
            job: map.get(status.id).cloned(),
            status,
        })
        .collect();
    json_response(&ListResponse {
        agents,
        outdated,
        can_run: local,
    })
}

fn kind_for(id: &str) -> Option<CliKind> {
    CliKind::all().into_iter().find(|kind| kind.prefix() == id)
}

async fn run_one(Direct(local): Direct, Path(id): Path<String>) -> Response {
    if !local {
        return err_response(
            StatusCode::FORBIDDEN,
            "installing agents is only allowed from the machine running the daemon",
        );
    }
    let Some(kind) = kind_for(&id) else {
        return err_response(StatusCode::NOT_FOUND, "unknown agent");
    };
    let current = agent_install::status(kind, false).await;
    if current.action.is_none() {
        let hint = current.manual_hint.unwrap_or_default();
        return err_response(StatusCode::UNPROCESSABLE_ENTITY, &hint);
    }
    if !claim(kind.prefix()) {
        return err_response(
            StatusCode::CONFLICT,
            "an install is already running for this agent",
        );
    }
    tokio::spawn(run_job(kind));
    accepted(vec![kind.prefix()])
}

async fn update_all(Direct(local): Direct) -> Response {
    if !local {
        return err_response(
            StatusCode::FORBIDDEN,
            "updating agents is only allowed from the machine running the daemon",
        );
    }
    let outdated: Vec<CliKind> = statuses(true)
        .await
        .into_iter()
        .filter(|s| s.outdated)
        .filter_map(|s| kind_for(s.id))
        .filter(|kind| claim(kind.prefix()))
        .collect();
    let ids = outdated.iter().map(|k| k.prefix()).collect();
    // One at a time: two global npm installs racing in one prefix corrupt it.
    tokio::spawn(async move {
        for kind in outdated {
            run_job(kind).await;
        }
    });
    accepted(ids)
}

fn accepted(started: Vec<&'static str>) -> Response {
    use axum::response::IntoResponse;
    let mut response = json_response(&serde_json::json!({ "started": started }));
    *response.status_mut() = StatusCode::ACCEPTED;
    response.into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_claim_is_refused_until_the_job_finishes() {
        let id = "claude-cli";
        assert!(claim(id));
        assert!(!claim(id), "queued job blocks a second start");
        update_job(id, |job| job.state = "running");
        assert!(!claim(id));
        update_job(id, |job| job.state = "failed");
        assert!(claim(id), "a finished job can be retried");
        update_job(id, |job| job.state = "done");
    }

    #[test]
    fn ids_map_to_kinds() {
        assert_eq!(kind_for("codex-cli"), Some(CliKind::Codex));
        assert_eq!(kind_for("nope"), None);
    }
}
