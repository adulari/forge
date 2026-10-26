//! End-to-end tests for `POST /v1/reports` against a local mock GitHub API (never the real one).

use crate::ratelimit::RateLimiters;
use crate::reports::{test_config, ReportsConfig, ReportsState};
use crate::{build_router, AppState};
use axum::extract::{ConnectInfo, Path, State};
use axum::http::{Request, StatusCode};
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use tower::ServiceExt;

const TOKEN: &str = "ghp_TESTTOKEN0123456789abcdefghijklmnop";

#[derive(Default)]
struct Mock {
    requests: Vec<(String, String, Value)>,
    open: HashMap<u64, bool>,
    next: u64,
    fail: bool,
}

type MockState = Arc<Mutex<Mock>>;

async fn create(State(m): State<MockState>, Json(body): Json<Value>) -> (StatusCode, Json<Value>) {
    let mut m = m.lock().unwrap();
    if m.fail {
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({})));
    }
    m.next += 1;
    let n = m.next;
    m.open.insert(n, true);
    m.requests.push(("POST".into(), "/issues".into(), body));
    (StatusCode::CREATED, Json(json!({ "number": n })))
}

async fn read(State(m): State<MockState>, Path(n): Path<u64>) -> (StatusCode, Json<Value>) {
    let m = m.lock().unwrap();
    match m.open.get(&n) {
        Some(open) => (
            StatusCode::OK,
            Json(json!({"state": if *open { "open" } else { "closed" }})),
        ),
        None => (StatusCode::NOT_FOUND, Json(json!({}))),
    }
}

async fn edit(
    State(m): State<MockState>,
    Path(n): Path<u64>,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    let mut m = m.lock().unwrap();
    if body["state"] == "open" {
        m.open.insert(n, true);
    }
    m.requests
        .push(("PATCH".into(), format!("/issues/{n}"), body));
    (StatusCode::OK, Json(json!({})))
}

async fn comment(
    State(m): State<MockState>,
    Path(n): Path<u64>,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    m.lock()
        .unwrap()
        .requests
        .push(("POST".into(), format!("/issues/{n}/comments"), body));
    (StatusCode::CREATED, Json(json!({})))
}

async fn start_mock() -> (String, MockState) {
    let state: MockState = Arc::default();
    let app = Router::new()
        .route("/repos/Adulari/forge/issues", post(create))
        .route(
            "/repos/Adulari/forge/issues/{n}",
            get(read).merge(patch(edit)),
        )
        .route("/repos/Adulari/forge/issues/{n}/comments", post(comment))
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}"), state)
}

struct Harness {
    app: Router,
    reports: Arc<ReportsState>,
    mock: MockState,
}

async fn harness_with(tweak: impl FnOnce(&mut ReportsConfig)) -> Harness {
    let (base, mock) = start_mock().await;
    let mut cfg = test_config(Some(&base));
    tweak(&mut cfg);
    let reports = ReportsState::new(cfg).unwrap();
    let state = Arc::new(AppState {
        auth: None,
        reports: reports.clone(),
        client: reqwest::Client::new(),
        allowed_topics: vec![],
        relay_token: None,
        generic_alerts: false,
        trust_proxy_headers: false,
        limiters: Arc::new(RateLimiters::new(1000, 60, 1_000_000)),
        upstream_base_url: None,
    });
    Harness {
        app: build_router(state),
        reports,
        mock,
    }
}

async fn harness() -> Harness {
    harness_with(|_| {}).await
}

fn report(version: &str) -> Value {
    json!({
        "v": 1, "source": "auto",
        "install_id": "0123456789abcdef0123456789abcdef",
        "fingerprint": "aaaaaaaaaaaaaaaa",
        "error_kind": "panic",
        "site": "forge_core::session::run_turn",
        "location": "forge-core/src/session.rs:412",
        "message": "index out of bounds",
        "frames": ["forge_core::session::run_turn"],
        "context": {"version": version, "git_sha": "abcdef1", "surface": "tui",
                    "os": "linux", "arch": "x86_64", "flags": []},
        "events": [{"k": "turn_start", "ms": 12, "ago": 3}],
        "occurred_at": 1_800_000_000
    })
}

async fn send(h: &Harness, body: String) -> (StatusCode, Value, axum::http::HeaderMap) {
    let mut request = Request::post("/v1/reports")
        .header("content-type", "application/json")
        .body(axum::body::Body::from(body))
        .unwrap();
    let peer: SocketAddr = "203.0.113.5:4000".parse().unwrap();
    request.extensions_mut().insert(ConnectInfo(peer));
    let response = h.app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        headers,
    )
}

async fn post_json(h: &Harness, v: &Value) -> (StatusCode, Value) {
    let (s, b, _) = send(h, v.to_string()).await;
    (s, b)
}

fn log(h: &Harness) -> Vec<(String, String, Value)> {
    h.mock.lock().unwrap().requests.clone()
}

fn count(h: &Harness, method: &str, path: &str) -> usize {
    log(h)
        .iter()
        .filter(|(m, p, _)| m == method && p == path)
        .count()
}

#[tokio::test]
async fn rejects_unknown_fields_and_oversize_bodies() {
    let h = harness().await;
    let mut extra = report("2.23.6");
    extra["prompt"] = json!("write me a poem");
    assert_eq!(post_json(&h, &extra).await.0, StatusCode::BAD_REQUEST);
    let mut ctx = report("2.23.6");
    ctx["context"]["hostname"] = json!("laptop");
    assert_eq!(post_json(&h, &ctx).await.0, StatusCode::BAD_REQUEST);
    let mut bad = report("2.23.6");
    bad["install_id"] = json!("short");
    assert_eq!(post_json(&h, &bad).await.0, StatusCode::BAD_REQUEST);
    let mut note = report("2.23.6");
    note["user_note"] = json!("auto reports cannot carry notes");
    assert_eq!(post_json(&h, &note).await.0, StatusCode::BAD_REQUEST);
    let mut huge = report("2.23.6");
    huge["message"] = json!("x".repeat(40 * 1024));
    assert_eq!(
        send(&h, huge.to_string()).await.0,
        StatusCode::PAYLOAD_TOO_LARGE
    );
    assert_eq!(count(&h, "POST", "/issues"), 0);
}

#[tokio::test]
async fn scrubs_secrets_paths_emails_and_urls_before_publishing() {
    let h = harness().await;
    let mut r = report("2.23.6");
    r["message"] = json!(
        "open /home/alice/secret-project/src/lib.rs failed; key sk-ant-api03-AbCdEfGhIjKlMnOpQrStUv \
         mail bob@corp.io GET https://api.internal.example.com/v1/users?token=abc"
    );
    r["location"] = json!("/home/alice/forge/crates/x/src/a.rs:9");
    r["context"]["model"] = json!("cb/deepseek-v4.1-flash");
    assert_eq!(post_json(&h, &r).await.0, StatusCode::ACCEPTED);
    h.reports.sync_dirty().await.unwrap();
    let published = serde_json::to_string(&log(&h)).unwrap();
    for leaked in [
        "alice",
        "secret-project",
        "AbCdEfGhIjKlMnOpQrStUv",
        "bob@corp.io",
        "/v1/users",
        "0123456789abcdef0123456789abcdef",
        TOKEN,
    ] {
        assert!(!published.contains(leaked), "{leaked} leaked: {published}");
    }
    assert!(published.contains("api.internal.example.com"));
    assert!(published.contains("lib.rs"));
}

#[tokio::test]
async fn duplicate_fingerprint_creates_one_issue_then_edits_counts() {
    let h = harness().await;
    let r = report("2.23.6");
    assert_eq!(post_json(&h, &r).await.1["deduped"], false);
    h.reports.sync_dirty().await.unwrap();
    assert_eq!(count(&h, "POST", "/issues"), 1);
    let (_, body) = post_json(&h, &r).await;
    assert_eq!(body["deduped"], true);
    assert_eq!(body["issue"], 1);
    h.reports.sync_dirty().await.unwrap();
    assert_eq!(count(&h, "POST", "/issues"), 1);
    let patches: Vec<_> = log(&h)
        .into_iter()
        .filter(|(m, _, _)| m == "PATCH")
        .collect();
    assert_eq!(patches.len(), 1);
    assert!(patches[0].2["body"]
        .as_str()
        .unwrap()
        .contains("| Occurrences | 2 |"));
    let created = log(&h)[0].2.clone();
    assert_eq!(created["labels"], json!(["auto-report"]));
    assert_eq!(
        created["title"],
        "[auto-report] panic: forge-core/src/session.rs:412"
    );
    assert!(created["body"]
        .as_str()
        .unwrap()
        .contains("Representative report"));
    assert_eq!(count(&h, "POST", "/issues/1/comments"), 0);
}

#[tokio::test]
async fn comments_once_per_newly_affected_version() {
    let h = harness().await;
    post_json(&h, &report("2.23.6")).await;
    h.reports.sync_dirty().await.unwrap();
    post_json(&h, &report("2.24.0")).await;
    h.reports.sync_dirty().await.unwrap();
    assert_eq!(count(&h, "POST", "/issues/1/comments"), 1);
    post_json(&h, &report("2.24.0")).await;
    h.reports.sync_dirty().await.unwrap();
    assert_eq!(count(&h, "POST", "/issues/1/comments"), 1);
}

#[tokio::test]
async fn reopens_only_when_recurring_on_a_newer_version() {
    let h = harness().await;
    post_json(&h, &report("2.23.6")).await;
    h.reports.sync_dirty().await.unwrap();
    h.mock.lock().unwrap().open.insert(1, false);

    post_json(&h, &report("2.23.6")).await;
    h.reports.sync_dirty().await.unwrap();
    assert!(!log(&h).iter().any(|(_, _, b)| b["state"] == "open"));
    assert!(!h.mock.lock().unwrap().open[&1]);

    post_json(&h, &report("2.25.0")).await;
    h.reports.sync_dirty().await.unwrap();
    assert!(log(&h).iter().any(|(_, _, b)| b["state"] == "open"));
    assert!(h.mock.lock().unwrap().open[&1]);
    let comments: Vec<_> = log(&h)
        .into_iter()
        .filter(|(_, p, _)| p == "/issues/1/comments")
        .collect();
    assert_eq!(comments.len(), 1);
    assert!(comments[0].2["body"].as_str().unwrap().contains("Reopened"));
}

#[tokio::test]
async fn per_install_rate_limit_answers_429_with_retry_after() {
    let h = harness_with(|c| c.per_install_per_hour = 2).await;
    for _ in 0..2 {
        assert_eq!(
            post_json(&h, &report("2.23.6")).await.0,
            StatusCode::ACCEPTED
        );
    }
    let (status, _, headers) = send(&h, report("2.23.6").to_string()).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert!(headers.contains_key("retry-after"));
}

#[tokio::test]
async fn global_rate_limit_applies_across_installs() {
    let h = harness_with(|c| c.global_per_hour = 1).await;
    assert_eq!(
        post_json(&h, &report("2.23.6")).await.0,
        StatusCode::ACCEPTED
    );
    let mut other = report("2.23.6");
    other["install_id"] = json!("fedcba9876543210fedcba9876543210");
    assert_eq!(post_json(&h, &other).await.0, StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn kill_switch_and_missing_token_answer_503() {
    let h = harness_with(|c| c.disabled = true).await;
    let (status, _, headers) = send(&h, report("2.23.6").to_string()).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(headers.contains_key("retry-after"));

    let h = harness_with(|c| c.token = None).await;
    assert_eq!(
        post_json(&h, &report("2.23.6")).await.0,
        StatusCode::SERVICE_UNAVAILABLE
    );
}

#[tokio::test]
async fn manual_reports_never_dedup() {
    let h = harness().await;
    let mut r = report("2.23.6");
    r["source"] = json!("manual");
    r["error_kind"] = json!("manual");
    r["fingerprint"] = json!("0000000000000000");
    r["user_note"] = json!("it froze, my key is sk-ant-api03-AbCdEfGhIjKlMnOpQrStUv");
    r["last_error"] = json!("provider returned garbage");
    assert_eq!(post_json(&h, &r).await.0, StatusCode::ACCEPTED);
    assert_eq!(post_json(&h, &r).await.0, StatusCode::ACCEPTED);
    h.reports.sync_dirty().await.unwrap();
    assert_eq!(count(&h, "POST", "/issues"), 2);
    let first = log(&h)[0].2.clone();
    assert_eq!(first["labels"], json!(["user-report"]));
    assert!(!first.to_string().contains("AbCdEfGhIjKlMnOpQrStUv"));
}

#[tokio::test]
async fn github_outage_keeps_the_count_for_a_later_retry() {
    let h = harness().await;
    h.mock.lock().unwrap().fail = true;
    assert_eq!(
        post_json(&h, &report("2.23.6")).await.0,
        StatusCode::ACCEPTED
    );
    h.reports.sync_dirty().await.unwrap();
    let problem = h
        .reports
        .db
        .as_ref()
        .unwrap()
        .problem("aaaaaaaaaaaaaaaa")
        .unwrap()
        .unwrap();
    assert_eq!(problem.count, 1);
    assert_eq!(problem.issue_number, None);
    assert!(h
        .reports
        .db
        .as_ref()
        .unwrap()
        .due(i64::MAX)
        .unwrap()
        .contains(&problem.fingerprint));
}

#[test]
fn token_is_never_printed() {
    let cfg = test_config(Some("http://x"));
    assert!(!format!("{cfg:?}").contains(TOKEN));
}
