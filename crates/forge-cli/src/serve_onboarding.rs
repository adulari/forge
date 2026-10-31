//! `/api/onboarding/*`: the setup flow for the desktop app, served by the daemon.
//!
//! The engine (`forge-onboarding`) does the work; this module only exposes it. The daemon owns
//! the [`Jobs`] table, so the sign-ins that need a pseudo-terminal (`claude setup-token`, the
//! Codex and xAI OAuth flows) run here and the credentials they produce are stored by the daemon
//! itself. Nothing in a response carries a secret: pasted values deserialize into
//! [`forge_onboarding::Secret`], which serializes as `***`, and job output is scrubbed by the
//! engine before it is stored.
//!
//! Every route is loopback-only (the same check the daemon update uses), and none of them is in
//! the Forge Anywhere allowlist: setting a machine up is something you do at the machine.
//!
//! * `GET    /api/onboarding[?force=1&brief=1]`  plan, state, detected machine, recommended list
//! * `POST   /api/onboarding/steps/{id}`         `{"op":"done"|"skip"|"reset"}` for a step id, or
//!   `{"op":"postpone"|"complete"}` for the pseudo step `flow`
//! * `POST   /api/onboarding/actions`            `{"action":{..},"step":"models"}` starts a job;
//!   `{"recommended":true}` returns the "set up everything recommended" checklist
//! * `GET    /api/onboarding/jobs/{id}?cursor=&wait=`  events since `cursor`, long-polling up to
//!   `wait` milliseconds when there are none yet
//! * `POST   /api/onboarding/jobs/{id}/input`    `{"text":".."}` a pasted code for a waiting job
//! * `DELETE /api/onboarding/jobs/{id}`          cancel

use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use axum::extract::{Json, Path, Query};
use axum::http::StatusCode;
use axum::response::Response;
use axum::routing::{get, post};
use axum::Router;
use forge_onboarding::{
    Action, ActionResult, Host, JobEvent, JobState, Jobs, Machine, ModelSource, OnboardingState,
    PlanMode, PlanStep, Recommended, Secret, Session, Start, StepId, StepStatus, Surface,
    FLOW_VERSION,
};
use serde::{Deserialize, Serialize};

use crate::serve::{err_response, json_response, DaemonState};
use crate::serve_update::Direct;

const MAX_WAIT_MS: u64 = 25_000;
const POLL_EVERY: Duration = Duration::from_millis(100);
const MAX_INPUT_CHARS: usize = 16 * 1024;

pub(crate) fn routes(base: &str) -> Router<Arc<DaemonState>> {
    Router::new()
        .route(&format!("{base}/api/onboarding"), get(get_onboarding))
        .route(
            &format!("{base}/api/onboarding/steps/{{id}}"),
            post(post_step),
        )
        .route(&format!("{base}/api/onboarding/actions"), post(post_action))
        .route(
            &format!("{base}/api/onboarding/jobs/{{id}}"),
            get(get_job).delete(cancel_job),
        )
        .route(
            &format!("{base}/api/onboarding/jobs/{{id}}/input"),
            post(post_input),
        )
}

fn jobs() -> &'static Jobs {
    static JOBS: OnceLock<Jobs> = OnceLock::new();
    JOBS.get_or_init(Jobs::new)
}

/// Read-modify-write of `onboarding.json` happens under this lock, so a finishing job and a
/// step update from the UI cannot overwrite each other.
fn state_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(not(test))]
fn host() -> Arc<dyn Host> {
    Arc::new(crate::onboarding::host::CliHost)
}

/// A host that knows nothing but the background service, which tests switch up or down.
#[cfg(test)]
struct TestHost;

#[cfg(test)]
static SERVICE_UP: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

#[cfg(test)]
impl Host for TestHost {
    fn service(&self) -> forge_onboarding::machine::ServiceFacts {
        let up = SERVICE_UP.load(std::sync::atomic::Ordering::SeqCst);
        forge_onboarding::machine::ServiceFacts {
            supported: true,
            installed: up,
            running: up,
        }
    }
}

#[cfg(test)]
fn host() -> Arc<dyn Host> {
    Arc::new(TestHost)
}

fn local_only(local: bool) -> Option<Response> {
    (!local).then(|| {
        err_response(
            StatusCode::FORBIDDEN,
            "setup is only available from the machine the daemon runs on",
        )
    })
}

#[derive(Debug, Serialize)]
struct Snapshot {
    surface: Surface,
    /// Whether the app should open onboarding by itself right now. Absent on replies to a step
    /// update, where the caller already knows.
    #[serde(skip_serializing_if = "Option::is_none")]
    open: Option<bool>,
    /// One line to show instead of opening, once setup was deferred as often as allowed.
    #[serde(skip_serializing_if = "Option::is_none")]
    hint: Option<String>,
    mode: PlanMode,
    /// Every required step passes.
    complete: bool,
    missing: Vec<StepId>,
    flow_version: u32,
    start_index: usize,
    plan: Vec<PlanStep>,
    state: OnboardingState,
    machine: Machine,
    sources: Vec<ModelSource>,
    recommended: Vec<Recommended>,
    claude_reconnect: bool,
}

#[derive(Debug, Serialize)]
struct Brief {
    open: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    hint: Option<String>,
    complete: bool,
    flow_version: u32,
    claude_reconnect: bool,
}

fn snapshot_of(session: &Session, open: Option<bool>, hint: Option<String>) -> Snapshot {
    let gate = session.gate();
    Snapshot {
        surface: session.surface,
        open,
        hint,
        mode: session.mode,
        complete: gate.complete,
        missing: gate.missing,
        flow_version: FLOW_VERSION,
        start_index: session.start_index(),
        plan: session.plan.clone(),
        state: session.state.clone(),
        sources: session.machine.model_sources(),
        recommended: forge_onboarding::recommended(
            &session.machine,
            &session.state,
            session.surface,
        ),
        claude_reconnect: session.machine.claude_needs_reconnect(),
        machine: session.machine.clone(),
    }
}

/// A session over the current state file for a run that is not opening by itself (the app asks
/// for the plan anyway: a settings page that reruns setup, the final summary).
fn current_session(host: Arc<dyn Host>) -> Session {
    let machine = forge_onboarding::detect(host);
    let mut state = forge_onboarding::load().unwrap_or_default();
    state.refresh_required(&machine, Surface::Desktop);
    let plan = forge_onboarding::plan(&state, &machine, Surface::Desktop, PlanMode::Full);
    Session {
        surface: Surface::Desktop,
        mode: PlanMode::Full,
        state,
        machine,
        plan,
    }
}

fn load_snapshot(force: bool) -> Snapshot {
    let host = host();
    let _guard = state_lock();
    match forge_onboarding::start_with(host.clone(), Surface::Desktop, force, FLOW_VERSION, true) {
        Start::Run(session) => snapshot_of(&session, Some(true), None),
        Start::Hint(text) => snapshot_of(&current_session(host), Some(false), Some(text)),
        Start::Quiet => snapshot_of(&current_session(host), Some(false), None),
    }
}

fn load_brief() -> Brief {
    let host = host();
    let _guard = state_lock();
    let started =
        forge_onboarding::start_with(host.clone(), Surface::Desktop, false, FLOW_VERSION, true);
    let (open, hint) = match started {
        Start::Run(_) => (true, None),
        Start::Hint(text) => (false, Some(text)),
        Start::Quiet => (false, None),
    };
    let state = forge_onboarding::load().unwrap_or_default();
    Brief {
        open,
        hint,
        complete: state.required_complete(Surface::Desktop),
        flow_version: FLOW_VERSION,
        claude_reconnect: forge_onboarding::claude_reconnect_needed(),
    }
}

#[derive(Deserialize)]
struct SnapshotQuery {
    #[serde(default)]
    force: Option<String>,
    #[serde(default)]
    brief: Option<String>,
}

fn flag(value: &Option<String>) -> bool {
    value
        .as_deref()
        .is_some_and(|v| !matches!(v, "" | "0" | "false"))
}

async fn get_onboarding(Direct(local): Direct, Query(query): Query<SnapshotQuery>) -> Response {
    if let Some(refused) = local_only(local) {
        return refused;
    }
    if flag(&query.brief) {
        return match tokio::task::spawn_blocking(load_brief).await {
            Ok(brief) => json_response(&brief),
            Err(_) => err_response(StatusCode::INTERNAL_SERVER_ERROR, "setup check failed"),
        };
    }
    let force = flag(&query.force);
    match tokio::task::spawn_blocking(move || load_snapshot(force)).await {
        Ok(snapshot) => json_response(&snapshot),
        Err(_) => err_response(StatusCode::INTERNAL_SERVER_ERROR, "setup check failed"),
    }
}

#[derive(Deserialize)]
struct StepBody {
    op: String,
}

enum StepOp {
    Mark(StepId, StepStatus),
    Postpone,
    Complete,
}

fn parse_step_op(id: &str, op: &str) -> Result<StepOp, &'static str> {
    if id == "flow" {
        return match op {
            "postpone" => Ok(StepOp::Postpone),
            "complete" => Ok(StepOp::Complete),
            _ => Err("the flow can only be postponed or completed"),
        };
    }
    let step = StepId::parse(id).ok_or("unknown step")?;
    let status = match op {
        "done" => StepStatus::Done,
        "skip" => StepStatus::Skipped,
        "reset" => StepStatus::Pending,
        _ => return Err("a step can be marked done, skip or reset"),
    };
    Ok(StepOp::Mark(step, status))
}

fn apply_step_op(op: StepOp) -> Snapshot {
    let host = host();
    let _guard = state_lock();
    let machine = forge_onboarding::detect(host);
    let state = forge_onboarding::load().unwrap_or_default();
    let mut session = Session {
        surface: Surface::Desktop,
        mode: PlanMode::Resume,
        state,
        machine,
        plan: Vec::new(),
    };
    match op {
        StepOp::Mark(step, StepStatus::Done) => session.state.mark_done(step),
        StepOp::Mark(step, StepStatus::Skipped) => session.state.mark_skipped(step),
        StepOp::Mark(step, StepStatus::Pending) => session.state.reset(step),
        StepOp::Postpone => session.postpone(),
        StepOp::Complete => session.complete(),
    }
    session
        .state
        .refresh_required(&session.machine, Surface::Desktop);
    session.plan = forge_onboarding::plan(
        &session.state,
        &session.machine,
        Surface::Desktop,
        PlanMode::Resume,
    );
    session.save();
    snapshot_of(&session, None, None)
}

async fn post_step(
    Direct(local): Direct,
    Path(id): Path<String>,
    Json(body): Json<StepBody>,
) -> Response {
    if let Some(refused) = local_only(local) {
        return refused;
    }
    let op = match parse_step_op(&id, &body.op) {
        Ok(op) => op,
        Err(message) => return err_response(StatusCode::BAD_REQUEST, message),
    };
    match tokio::task::spawn_blocking(move || apply_step_op(op)).await {
        Ok(snapshot) => json_response(&snapshot),
        Err(_) => err_response(StatusCode::INTERNAL_SERVER_ERROR, "could not save the step"),
    }
}

#[derive(Deserialize)]
struct ActionBody {
    #[serde(default)]
    action: Option<Action>,
    /// Mark this step done when the job succeeds (and the check did not refuse the credential).
    #[serde(default)]
    step: Option<StepId>,
    /// Instead of starting a job, return the "set up everything recommended" checklist.
    #[serde(default)]
    recommended: bool,
}

#[derive(Serialize)]
struct Started {
    job: u64,
    label: String,
}

#[derive(Serialize)]
struct Checklist {
    items: Vec<Recommended>,
}

fn job_finished(result: Option<ActionResult>, step: Option<StepId>) {
    let _guard = state_lock();
    let mut state = forge_onboarding::load().unwrap_or_default();
    let mut counts = true;
    if let Some(result) = &result {
        forge_onboarding::apply_result(&mut state, result);
        counts = match result {
            ActionResult::Verified { status, .. } => {
                *status != forge_onboarding::VerifyStatus::Rejected
            }
            ActionResult::PasteNeeded { .. } => false,
            ActionResult::Changed => true,
        };
    }
    if let (Some(step), true) = (step, counts) {
        state.mark_done(step);
    }
    let _ = forge_onboarding::save(&state);
}

/// Folds a finished job's result into the shared state file, whichever client is watching.
fn watch(id: u64, step: Option<StepId>) {
    std::thread::spawn(move || {
        let mut cursor = 0;
        loop {
            let Some(poll) = jobs().poll(id, cursor) else {
                return;
            };
            cursor = poll.next;
            for event in poll.events {
                match event {
                    JobEvent::Done { result, .. } => return job_finished(result, step),
                    JobEvent::Failed { .. } => return,
                    _ => {}
                }
            }
            std::thread::sleep(POLL_EVERY);
        }
    });
}

async fn post_action(Direct(local): Direct, Json(body): Json<ActionBody>) -> Response {
    if let Some(refused) = local_only(local) {
        return refused;
    }
    if body.recommended {
        let items = tokio::task::spawn_blocking(|| {
            let host = host();
            let machine = forge_onboarding::detect(host);
            let state = forge_onboarding::load().unwrap_or_default();
            forge_onboarding::recommended(&machine, &state, Surface::Desktop)
        })
        .await
        .unwrap_or_default();
        return json_response(&Checklist { items });
    }
    let Some(action) = body.action else {
        return err_response(
            StatusCode::BAD_REQUEST,
            "send an action or recommended:true",
        );
    };
    let label = action.label();
    let step = body.step;
    let id = tokio::task::spawn_blocking(move || jobs().run(action, host()))
        .await
        .unwrap_or_default();
    watch(id, step);
    json_response(&Started { job: id, label })
}

#[derive(Deserialize)]
struct JobQuery {
    #[serde(default)]
    cursor: usize,
    #[serde(default)]
    wait: u64,
}

async fn get_job(
    Direct(local): Direct,
    Path(id): Path<u64>,
    Query(query): Query<JobQuery>,
) -> Response {
    if let Some(refused) = local_only(local) {
        return refused;
    }
    let deadline = Instant::now() + Duration::from_millis(query.wait.min(MAX_WAIT_MS));
    loop {
        let Some(poll) = jobs().poll(id, query.cursor) else {
            return err_response(StatusCode::NOT_FOUND, "no such job");
        };
        if !poll.events.is_empty() || poll.state != JobState::Running || Instant::now() >= deadline
        {
            return json_response(&poll);
        }
        tokio::time::sleep(POLL_EVERY).await;
    }
}

#[derive(Deserialize)]
struct InputBody {
    text: Secret,
}

async fn post_input(
    Direct(local): Direct,
    Path(id): Path<u64>,
    Json(body): Json<InputBody>,
) -> Response {
    if let Some(refused) = local_only(local) {
        return refused;
    }
    if body.text.is_empty() || body.text.len() > MAX_INPUT_CHARS {
        return err_response(StatusCode::BAD_REQUEST, "paste the code the page showed");
    }
    if jobs().send_input(id, body.text.expose()) {
        json_response(&serde_json::json!({ "ok": true }))
    } else {
        err_response(StatusCode::CONFLICT, "that job is not waiting for input")
    }
}

async fn cancel_job(Direct(local): Direct, Path(id): Path<u64>) -> Response {
    if let Some(refused) = local_only(local) {
        return refused;
    }
    if jobs().state(id).is_none() {
        return err_response(StatusCode::NOT_FOUND, "no such job");
    }
    jobs().cancel(id);
    json_response(&serde_json::json!({ "ok": true }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::serve::SessionRegistry;
    use axum::body::Body;
    use axum::http::Request;
    use std::ffi::OsString;
    use tower::ServiceExt;

    const PLANTED_ENV_KEY: &str = "gsk-PLANTED-env-key-0123456789";
    const PASTED: &str = "brave-PASTED-secret-9876543210";

    /// Process-wide environment for one test: an empty HOME and data directories, no provider
    /// keys but one planted key, and the lock that keeps tests apart.
    struct Env {
        _guard: tokio::sync::MutexGuard<'static, ()>,
        saved: Vec<(&'static str, Option<OsString>)>,
        _home: tempfile::TempDir,
    }

    // Only what these tests need. HOME and PATH stay untouched: every other test in the binary
    // runs in this process at the same time, and some of them spawn `git` or resolve the home
    // directory.
    const VARS: [&str; 5] = [
        "XDG_CONFIG_HOME",
        "XDG_DATA_HOME",
        "FORGE_DB",
        "GROQ_API_KEY",
        "FORGE_NO_SETUP",
    ];

    /// Serialized with every other test that points `FORGE_DB` or the XDG directories somewhere.
    async fn env() -> Env {
        let guard = crate::serve::tests::FORGE_DB_LOCK.lock().await;
        let home = tempfile::tempdir().unwrap();
        let saved = VARS.iter().map(|k| (*k, std::env::var_os(k))).collect();
        std::env::set_var("XDG_CONFIG_HOME", home.path().join("config"));
        std::env::set_var("XDG_DATA_HOME", home.path().join("data"));
        std::env::set_var("FORGE_DB", home.path().join("forge.db"));
        std::env::remove_var("FORGE_NO_SETUP");
        std::env::set_var("GROQ_API_KEY", PLANTED_ENV_KEY);
        Env {
            _guard: guard,
            saved,
            _home: home,
        }
    }

    impl Drop for Env {
        fn drop(&mut self) {
            for (k, v) in &self.saved {
                match v {
                    Some(v) => std::env::set_var(k, v),
                    None => std::env::remove_var(k),
                }
            }
        }
    }

    fn router() -> Router {
        let state = Arc::new(DaemonState {
            registry: Arc::new(SessionRegistry::new()),
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
        crate::serve::daemon_router(state)
    }

    async fn call(
        method: &str,
        uri: &str,
        host: Option<&str>,
        body: Option<serde_json::Value>,
    ) -> (StatusCode, String) {
        let mut req = Request::builder().method(method).uri(uri);
        if let Some(host) = host {
            req = req.header("host", host);
        }
        let body = match body {
            Some(json) => {
                req = req.header("content-type", "application/json");
                Body::from(json.to_string())
            }
            None => Body::empty(),
        };
        let mut req = req.body(body).unwrap();
        req.extensions_mut()
            .insert(axum::extract::ConnectInfo(std::net::SocketAddr::from((
                [127, 0, 0, 1],
                50000,
            ))));
        let response = router().oneshot(req).await.unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, String::from_utf8(bytes.to_vec()).unwrap())
    }

    const LOCAL: Option<&str> = Some("127.0.0.1:7420");

    async fn local(
        method: &str,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> (StatusCode, String) {
        call(method, &format!("/tok{path}"), LOCAL, body).await
    }

    fn json(text: &str) -> serde_json::Value {
        serde_json::from_str(text).unwrap_or_else(|e| panic!("not json ({e}): {text}"))
    }

    #[tokio::test]
    async fn every_route_is_token_scoped_and_refuses_anything_that_is_not_the_local_machine() {
        let _e = env().await;
        let routes: [(&str, &str, Option<serde_json::Value>); 6] = [
            ("GET", "/api/onboarding", None),
            (
                "POST",
                "/api/onboarding/steps/models",
                Some(serde_json::json!({"op":"done"})),
            ),
            (
                "POST",
                "/api/onboarding/actions",
                Some(serde_json::json!({"recommended":true})),
            ),
            ("GET", "/api/onboarding/jobs/1", None),
            (
                "POST",
                "/api/onboarding/jobs/1/input",
                Some(serde_json::json!({"text":"x"})),
            ),
            ("DELETE", "/api/onboarding/jobs/1", None),
        ];
        for (method, path, body) in routes {
            let wrong_token = call(method, path, LOCAL, body.clone()).await;
            assert_eq!(wrong_token.0, StatusCode::NOT_FOUND, "{method} {path}");
            for host in [Some("192.168.1.5:7420"), Some("tunnel.example.com"), None] {
                let (status, _) = call(method, &format!("/tok{path}"), host, body.clone()).await;
                assert_eq!(
                    status,
                    StatusCode::FORBIDDEN,
                    "{method} {path} via {host:?}"
                );
            }
        }
    }

    #[tokio::test]
    async fn a_remote_peer_is_refused_even_with_a_loopback_host() {
        let _e = env().await;
        for uri in ["/tok/api/onboarding", "/tok/api/onboarding/jobs/1"] {
            let mut request = Request::builder()
                .method("GET")
                .uri(uri)
                .header("host", "127.0.0.1:7420")
                .body(Body::empty())
                .unwrap();
            request.extensions_mut().insert(axum::extract::ConnectInfo(
                std::net::SocketAddr::from(([192, 168, 1, 9], 50000)),
            ));
            let response = router().oneshot(request).await.unwrap();
            assert_eq!(response.status(), StatusCode::FORBIDDEN, "{uri}");
        }
    }

    #[tokio::test]
    async fn a_forwarded_request_is_refused_even_with_a_loopback_host() {
        let _e = env().await;
        let request = Request::builder()
            .method("GET")
            .uri("/tok/api/onboarding")
            .header("host", "127.0.0.1:7420")
            .header("x-forwarded-for", "203.0.113.9")
            .body(Body::empty())
            .unwrap();
        let mut request = request;
        request
            .extensions_mut()
            .insert(axum::extract::ConnectInfo(std::net::SocketAddr::from((
                [127, 0, 0, 1],
                50000,
            ))));
        let response = router().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn the_plan_comes_with_the_machine_and_never_a_credential() {
        let _e = env().await;
        let (status, text) = local("GET", "/api/onboarding", None).await;
        assert_eq!(status, StatusCode::OK, "{text}");
        assert!(
            !text.contains(PLANTED_ENV_KEY),
            "an API key leaked into the snapshot"
        );
        let body = json(&text);
        assert_eq!(body["surface"], "desktop");
        assert_eq!(body["open"], true, "a fresh machine opens setup");
        assert_eq!(
            body["complete"], true,
            "a working key satisfies the model step"
        );
        let plan: Vec<&str> = body["plan"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["id"].as_str().unwrap())
            .collect();
        assert_eq!(plan.first(), Some(&"welcome"));
        assert!(plan.contains(&"models") && plan.contains(&"service"));
        let groq = body["machine"]["keys"]
            .as_array()
            .unwrap()
            .iter()
            .find(|k| k["id"] == "groq")
            .unwrap();
        assert_eq!(groq["configured"], true);
        assert!(body["recommended"].is_array() && body["sources"].is_array());
        assert!(
            forge_onboarding::state_path().unwrap().exists(),
            "the first look writes the shared state file"
        );
    }

    #[tokio::test]
    async fn the_desktop_is_not_ready_until_the_service_runs() {
        let _e = env().await;
        SERVICE_UP.store(false, std::sync::atomic::Ordering::SeqCst);
        let (_, text) = local("GET", "/api/onboarding", None).await;
        SERVICE_UP.store(true, std::sync::atomic::Ordering::SeqCst);
        let body = json(&text);
        assert_eq!(body["complete"], false);
        assert_eq!(body["missing"], serde_json::json!(["service"]));
        let service = body["plan"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["id"] == "service")
            .unwrap();
        assert_eq!(service["required"], true);
    }

    #[tokio::test]
    async fn the_brief_skips_detection_output_and_says_whether_to_open() {
        let _e = env().await;
        let (status, text) = local("GET", "/api/onboarding?brief=1", None).await;
        assert_eq!(status, StatusCode::OK);
        let body = json(&text);
        assert_eq!(body["open"], true);
        assert!(body.get("machine").is_none() && body.get("plan").is_none());
        assert!(body.get("claude_reconnect").is_some());
    }

    #[tokio::test]
    async fn steps_are_saved_in_the_file_both_surfaces_share() {
        let _e = env().await;
        let (status, text) = local(
            "POST",
            "/api/onboarding/steps/behavior",
            Some(serde_json::json!({"op":"done"})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{text}");
        let body = json(&text);
        assert!(body.get("open").is_none());
        assert_eq!(body["state"]["steps"]["behavior"]["status"], "done");
        let on_disk = forge_onboarding::load().unwrap();
        assert_eq!(on_disk.status(StepId::Behavior), StepStatus::Done);

        local(
            "POST",
            "/api/onboarding/steps/web_search",
            Some(serde_json::json!({"op":"skip"})),
        )
        .await;
        assert_eq!(
            forge_onboarding::load().unwrap().status(StepId::WebSearch),
            StepStatus::Skipped
        );

        let (status, _) = local(
            "POST",
            "/api/onboarding/steps/nonsense",
            Some(serde_json::json!({"op":"done"})),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let (status, _) = local(
            "POST",
            "/api/onboarding/steps/behavior",
            Some(serde_json::json!({"op":"explode"})),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn postponing_counts_only_when_everything_required_already_works_and_complete_ends_the_run(
    ) {
        let _e = env().await;
        local("GET", "/api/onboarding", None).await;
        local(
            "POST",
            "/api/onboarding/steps/flow",
            Some(serde_json::json!({"op":"postpone"})),
        )
        .await;
        assert_eq!(forge_onboarding::load().unwrap().deferred_count, 1);

        local(
            "POST",
            "/api/onboarding/steps/flow",
            Some(serde_json::json!({"op":"complete"})),
        )
        .await;
        let state = forge_onboarding::load().unwrap();
        assert_eq!(state.deferred_count, 0);
        assert_eq!(state.flow_version, FLOW_VERSION);
        let (_, text) = local("GET", "/api/onboarding?brief=1", None).await;
        assert_eq!(json(&text)["open"], false, "a finished setup stays closed");
    }

    #[tokio::test]
    async fn the_recommended_checklist_is_returned_without_starting_anything() {
        let _e = env().await;
        let (status, text) = local(
            "POST",
            "/api/onboarding/actions",
            Some(serde_json::json!({"recommended": true})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{text}");
        let body = json(&text);
        let titles: Vec<String> = body["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["title"].as_str().unwrap().to_string())
            .collect();
        assert!(titles
            .iter()
            .any(|t| t.starts_with("Keep the safe defaults")));
        assert!(titles.iter().any(|t| t.contains("error reports")));
        assert!(body["items"][0]["action"]["action"].is_string());
    }

    #[tokio::test]
    async fn a_pasted_secret_is_stored_and_never_comes_back_in_any_response_or_the_state_file() {
        let _e = env().await;
        let (status, text) = local(
            "POST",
            "/api/onboarding/actions",
            Some(serde_json::json!({
                "action": {"action":"store_secret","slot":"brave","value": PASTED},
                "step": "web_search",
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{text}");
        assert!(!text.contains(PASTED));
        let started = json(&text);
        let job = started["job"].as_u64().unwrap();
        assert!(!started["label"].as_str().unwrap().contains(PASTED));

        let mut transcript = String::new();
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let (status, text) =
                local("GET", &format!("/api/onboarding/jobs/{job}?wait=500"), None).await;
            assert_eq!(status, StatusCode::OK, "{text}");
            transcript.push_str(&text);
            if json(&text)["state"] != "running" {
                break;
            }
            assert!(Instant::now() < deadline, "the job never finished");
        }
        assert!(transcript.contains("\"done\""), "{transcript}");
        assert!(
            !transcript.contains(PASTED),
            "the secret leaked: {transcript}"
        );

        // The watcher folds the finished job into the shared file.
        let mut marked = false;
        for _ in 0..50 {
            if forge_onboarding::load()
                .is_some_and(|s| s.status(StepId::WebSearch) == StepStatus::Done)
            {
                marked = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(marked, "the step was not marked done");
        let file = std::fs::read_to_string(forge_onboarding::state_path().unwrap()).unwrap();
        assert!(!file.contains(PASTED));
        assert_eq!(
            forge_config::api_key("brave").ok().as_deref(),
            Some(PASTED),
            "the daemon stored it"
        );
    }

    #[tokio::test]
    async fn jobs_report_unknown_ids_input_refusals_and_cancel() {
        let _e = env().await;
        let (status, _) = local("GET", "/api/onboarding/jobs/999999", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (status, _) = local(
            "POST",
            "/api/onboarding/jobs/999999/input",
            Some(serde_json::json!({"text":"abc"})),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
        let (status, _) = local(
            "POST",
            "/api/onboarding/jobs/1/input",
            Some(serde_json::json!({"text":""})),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let (status, _) = local("DELETE", "/api/onboarding/jobs/999999", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        let (_, text) = local(
            "POST",
            "/api/onboarding/actions",
            Some(serde_json::json!({"action": {"action":"set_theme","theme":"dark"}})),
        )
        .await;
        let job = json(&text)["job"].as_u64().unwrap();
        let (status, _) = local("DELETE", &format!("/api/onboarding/jobs/{job}"), None).await;
        assert_eq!(status, StatusCode::OK);
        let (status, _) = local(
            "POST",
            "/api/onboarding/actions",
            Some(serde_json::json!({})),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn a_pasted_code_reaches_the_waiting_job() {
        let _e = env().await;
        let id = jobs().start("Waiting for a code", vec![], |job| {
            let deadline = Instant::now() + Duration::from_secs(5);
            while Instant::now() < deadline {
                if let Some(text) = job.try_input() {
                    return Ok(forge_onboarding::Success::text(format!(
                        "got {} chars",
                        text.len()
                    )));
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err("timed out".into())
        });
        let (status, _) = local(
            "POST",
            &format!("/api/onboarding/jobs/{id}/input"),
            Some(serde_json::json!({"text":"abcd-efgh"})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let mut transcript = String::new();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let (_, text) =
                local("GET", &format!("/api/onboarding/jobs/{id}?wait=500"), None).await;
            transcript.push_str(&text);
            if json(&text)["state"] != "running" {
                break;
            }
            assert!(Instant::now() < deadline, "the job never finished");
        }
        assert!(transcript.contains("got 9 chars"), "{transcript}");
        assert!(!transcript.contains("abcd-efgh"));
    }

    #[test]
    fn step_ops_parse_strictly() {
        assert!(matches!(
            parse_step_op("flow", "postpone"),
            Ok(StepOp::Postpone)
        ));
        assert!(matches!(
            parse_step_op("flow", "complete"),
            Ok(StepOp::Complete)
        ));
        assert!(parse_step_op("flow", "done").is_err());
        assert!(matches!(
            parse_step_op("models", "done"),
            Ok(StepOp::Mark(StepId::Models, StepStatus::Done))
        ));
        assert!(parse_step_op("models", "postpone").is_err());
        assert!(parse_step_op("", "done").is_err());
    }
}
