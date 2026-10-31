//! Add-a-machine routes: the same `forge host add` engine the terminal uses, run as a job by the
//! daemon so the Connect screen on desktop, web and a directly paired phone can start it, watch
//! it and approve the new device.
//!
//! - `GET  {base}/api/hosts`                        machines added so far
//! - `POST {base}/api/hosts/add`                    `{target, name?, lan?, pair?}` -> `{job_id}`
//! - `GET  {base}/api/hosts/jobs/{id}`              progress lines, state, safety code
//! - `POST {base}/api/hosts/jobs/{id}/approve`      `{approve: bool}`
//! - `POST {base}/api/hosts/{name}/remove`          forget a machine
//!
//! The daemon only ever starts `ssh` with the daemon user's own configuration and keys, never
//! with a password prompt (BatchMode), and the target is validated before it gets near a process.

use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use axum::extract::{Path as AxumPath, State};
use axum::http::StatusCode;
use axum::response::Response;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use crate::host_add::{
    add_host, AddOptions, Approver, Event, LocalApprover, SshRemote, DEFAULT_INSTALL_URL,
};
use crate::hosts;
use crate::serve::{err_response, json_response, DaemonState};

const MAX_LINES: usize = 300;
const MAX_ACTIVE_JOBS: usize = 4;
const APPROVAL_WAIT: Duration = Duration::from_secs(11 * 60);

pub(crate) fn routes(base: &str) -> Router<Arc<DaemonState>> {
    Router::new()
        .route(&format!("{base}/api/hosts"), get(list_hosts))
        .route(&format!("{base}/api/hosts/add"), post(start_add))
        .route(&format!("{base}/api/hosts/jobs/{{id}}"), get(job_status))
        .route(
            &format!("{base}/api/hosts/jobs/{{id}}/approve"),
            post(job_approve),
        )
        .route(
            &format!("{base}/api/hosts/{{name}}/remove"),
            post(remove_host),
        )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum JobPhase {
    Running,
    AwaitingApproval,
    Done,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
struct JobView {
    id: String,
    target: String,
    phase: JobPhase,
    step: String,
    lines: Vec<String>,
    safety_code: Option<String>,
    error: Option<String>,
    host: Option<hosts::HostRecord>,
}

struct Job {
    view: Mutex<JobView>,
    decision: Mutex<Option<Sender<bool>>>,
}

type Jobs = Mutex<HashMap<String, Arc<Job>>>;

fn jobs() -> &'static Jobs {
    static JOBS: OnceLock<Jobs> = OnceLock::new();
    JOBS.get_or_init(Default::default)
}

struct JobApprover {
    job: Arc<Job>,
    decisions: Receiver<bool>,
}

impl Approver for JobApprover {
    fn approve(&self, challenge: &str, safety_code: Option<&str>) -> anyhow::Result<bool> {
        {
            let mut view = self.job.view.lock().unwrap();
            view.phase = JobPhase::AwaitingApproval;
            view.safety_code = safety_code.map(str::to_string);
        }
        let allowed = match self.decisions.recv_timeout(APPROVAL_WAIT) {
            Ok(allowed) => allowed,
            Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => false,
        };
        if allowed {
            self.job.view.lock().unwrap().phase = JobPhase::Running;
            return LocalApprover { confirm: true }.approve(challenge, safety_code);
        }
        Ok(false)
    }
}

#[derive(Deserialize)]
struct AddRequest {
    target: String,
    name: Option<String>,
    #[serde(default)]
    lan: bool,
    #[serde(default = "yes")]
    pair: bool,
}

fn yes() -> bool {
    true
}

async fn list_hosts() -> Response {
    match tokio::task::spawn_blocking(hosts::list).await {
        Ok(Ok(list)) => json_response(&serde_json::json!({ "hosts": list })),
        Ok(Err(e)) => err_response(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
        Err(e) => err_response(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

async fn start_add(State(_): State<Arc<DaemonState>>, Json(request): Json<AddRequest>) -> Response {
    let target = match hosts::validate_target(&request.target) {
        Ok(t) => t.to_string(),
        Err(e) => return err_response(StatusCode::BAD_REQUEST, &e.to_string()),
    };
    if let Some(name) = request.name.as_deref().filter(|n| !n.trim().is_empty()) {
        if let Err(e) = hosts::validate_name(name) {
            return err_response(StatusCode::BAD_REQUEST, &e.to_string());
        }
    }
    let id = forge_types::new_id();
    let (tx, rx) = channel();
    let job = Arc::new(Job {
        view: Mutex::new(JobView {
            id: id.clone(),
            target: target.clone(),
            phase: JobPhase::Running,
            step: "Starting".into(),
            lines: Vec::new(),
            safety_code: None,
            error: None,
            host: None,
        }),
        decision: Mutex::new(Some(tx)),
    });
    {
        let mut all = jobs().lock().unwrap();
        let active = |j: &Arc<Job>| {
            matches!(
                j.view.lock().unwrap().phase,
                JobPhase::Running | JobPhase::AwaitingApproval
            )
        };
        if all.values().filter(|j| active(j)).count() >= MAX_ACTIVE_JOBS {
            return err_response(
                StatusCode::TOO_MANY_REQUESTS,
                "too many machines are being added",
            );
        }
        if all
            .values()
            .any(|j| active(j) && j.view.lock().unwrap().target == target)
        {
            return err_response(StatusCode::CONFLICT, "that machine is already being added");
        }
        all.insert(id.clone(), job.clone());
    }
    let options = AddOptions {
        target: target.clone(),
        name: request.name.filter(|n| !n.trim().is_empty()),
        lan: request.lan,
        port: None,
        pair: request.pair,
        installer: None,
        install_url: DEFAULT_INSTALL_URL.to_string(),
    };
    std::thread::spawn(move || run_job(job, options, rx));
    json_response(&serde_json::json!({ "job_id": id }))
}

fn run_job(job: Arc<Job>, options: AddOptions, decisions: Receiver<bool>) {
    let remote = SshRemote {
        ssh: std::env::var("FORGE_SSH").unwrap_or_else(|_| "ssh".into()),
        target: options.target.clone(),
        extra: Vec::new(),
    };
    let approver = JobApprover {
        job: job.clone(),
        decisions,
    };
    let progress_job = job.clone();
    let result = add_host(&options, &remote, &approver, &mut |event| {
        let mut view = progress_job.view.lock().unwrap();
        match event {
            Event::Step(text) => view.step = text,
            Event::Remote(line) => {
                if view.lines.len() >= MAX_LINES {
                    view.lines.remove(0);
                }
                view.lines.push(line);
            }
            Event::AwaitingApproval { .. } => {}
        }
    });
    let outcome = result.and_then(|o| hosts::upsert(o.record.clone()).map(|()| o));
    let mut view = job.view.lock().unwrap();
    match outcome {
        Ok(o) => {
            view.phase = JobPhase::Done;
            view.host = Some(o.record);
        }
        Err(e) => {
            view.phase = JobPhase::Failed;
            view.error = Some(e.to_string());
        }
    }
    view.safety_code = None;
    job.decision.lock().unwrap().take();
}

fn find_job(id: &str) -> Option<Arc<Job>> {
    jobs().lock().unwrap().get(id).cloned()
}

async fn job_status(AxumPath(id): AxumPath<String>) -> Response {
    match find_job(&id) {
        Some(job) => json_response(&*job.view.lock().unwrap()),
        None => err_response(StatusCode::NOT_FOUND, "no such job"),
    }
}

#[derive(Deserialize)]
struct ApproveRequest {
    approve: bool,
}

async fn job_approve(
    AxumPath(id): AxumPath<String>,
    Json(request): Json<ApproveRequest>,
) -> Response {
    let Some(job) = find_job(&id) else {
        return err_response(StatusCode::NOT_FOUND, "no such job");
    };
    if job.view.lock().unwrap().phase != JobPhase::AwaitingApproval {
        return err_response(
            StatusCode::CONFLICT,
            "this machine is not waiting for approval",
        );
    }
    let sent = job
        .decision
        .lock()
        .unwrap()
        .as_ref()
        .map(|tx| tx.send(request.approve).is_ok())
        .unwrap_or(false);
    if sent {
        json_response(&serde_json::json!({ "ok": true }))
    } else {
        err_response(StatusCode::CONFLICT, "the approval window has closed")
    }
}

async fn remove_host(AxumPath(name): AxumPath<String>) -> Response {
    match tokio::task::spawn_blocking(move || hosts::remove(&name)).await {
        Ok(Ok(true)) => json_response(&serde_json::json!({ "ok": true })),
        Ok(Ok(false)) => err_response(StatusCode::NOT_FOUND, "no such machine"),
        Ok(Err(e)) => err_response(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
        Err(e) => err_response(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    fn app() -> Router {
        let state = Arc::new(DaemonState {
            registry: Arc::new(crate::serve::SessionRegistry::new()),
            terminals: Arc::new(crate::serve_terminal::TerminalRegistry::new()),
            store: Arc::new(forge_store::Store::open_in_memory().unwrap()),
            base: "/tok".into(),
            mock: true,
            default_cwd: "/tmp".into(),
            project_roots: Vec::new(),
            push: None,
            apns: None,
            voice: crate::voice::VoiceState::new(),
            anywhere_enable: tokio::sync::watch::channel(false).0,
        });
        routes("/tok").with_state(state)
    }

    async fn call(method: &str, uri: &str, body: &str) -> (StatusCode, String) {
        let response = app()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(uri)
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
            .await
            .unwrap();
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    #[tokio::test]
    async fn a_target_ssh_could_read_as_an_option_is_refused_before_anything_runs() {
        let (status, body) = call(
            "POST",
            "/tok/api/hosts/add",
            r#"{"target":"-oProxyCommand=x"}"#,
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        let (status, _) = call("POST", "/tok/api/hosts/add", r#"{"target":"a;b"}"#).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let (status, _) = call(
            "POST",
            "/tok/api/hosts/add",
            r#"{"target":"u@h","name":"x'y"}"#,
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn unknown_jobs_are_not_found_and_only_a_waiting_job_takes_an_answer() {
        let (status, _) = call("GET", "/tok/api/hosts/jobs/nope", "").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (status, _) = call(
            "POST",
            "/tok/api/hosts/jobs/nope/approve",
            r#"{"approve":true}"#,
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        let job = Arc::new(Job {
            view: Mutex::new(JobView {
                id: "j1".into(),
                target: "u@h".into(),
                phase: JobPhase::Running,
                step: String::new(),
                lines: vec![],
                safety_code: None,
                error: None,
                host: None,
            }),
            decision: Mutex::new(None),
        });
        jobs().lock().unwrap().insert("j1".into(), job.clone());
        let (status, _) = call(
            "POST",
            "/tok/api/hosts/jobs/j1/approve",
            r#"{"approve":true}"#,
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "not waiting yet");
        let (tx, rx) = channel();
        *job.decision.lock().unwrap() = Some(tx);
        job.view.lock().unwrap().phase = JobPhase::AwaitingApproval;
        let (status, _) = call(
            "POST",
            "/tok/api/hosts/jobs/j1/approve",
            r#"{"approve":false}"#,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(!rx.recv().unwrap());
        let (status, body) = call("GET", "/tok/api/hosts/jobs/j1", "").await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("awaiting_approval"), "{body}");
    }
}
