use super::*;
use axum::body::Body;
use axum::http::Request;
use tower::ServiceExt;

fn state() -> Arc<DaemonState> {
    Arc::new(DaemonState {
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
    })
}

static INSTALL_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

struct Harness {
    router: Router,
    dir: PathBuf,
    _guard: tokio::sync::MutexGuard<'static, ()>,
}

async fn harness(name: &str) -> Harness {
    let guard = INSTALL_LOCK.lock().await;
    let dir = std::env::temp_dir().join(format!("forge-access-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    install(Access::new(
        Some(dir.join("serve-clients.json")),
        Some(dir.join(AUDIT_FILE)),
        &["phone.example.com".to_string()],
        &[],
    ));
    Harness {
        router: crate::serve::daemon_router(state()),
        dir,
        _guard: guard,
    }
}

async fn send(
    h: &Harness,
    method: &str,
    path: &str,
    host: Option<&str>,
) -> (StatusCode, serde_json::Value) {
    let mut builder = Request::builder().method(method).uri(path);
    if let Some(host) = host {
        builder = builder.header("host", host);
    }
    let response = h
        .router
        .clone()
        .oneshot(builder.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
    )
}

async fn pair(h: &Harness, preset: &str, label: &str) -> String {
    let request = Request::post("/tok/api/clients/pair")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({"preset": preset, "label": label}).to_string(),
        ))
        .unwrap();
    let response = h.router.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    body["token"].as_str().unwrap().to_string()
}

#[test]
fn classification_maps_routes_to_scopes() {
    let scope = |m: Method, p: &str| match required(&m, p) {
        Need::Scope(s) => s.as_str(),
        Need::Owner => "owner",
    };
    assert_eq!(scope(Method::GET, "/api/sessions"), "view");
    assert_eq!(scope(Method::GET, "/ws"), "view");
    assert_eq!(scope(Method::POST, "/api/sessions"), "prompt");
    assert_eq!(scope(Method::POST, "/api/answer"), "approve");
    assert_eq!(scope(Method::POST, "/api/dispatches/x/approve"), "approve");
    assert_eq!(scope(Method::GET, "/ws/terminal"), "terminal");
    assert_eq!(scope(Method::GET, "/api/terminals"), "terminal");
    assert_eq!(scope(Method::PUT, "/api/workspace/file"), "workspace-write");
    assert_eq!(scope(Method::POST, "/api/git/push"), "workspace-write");
    assert_eq!(scope(Method::POST, "/api/pr/merge"), "workspace-write");
    assert_eq!(scope(Method::GET, "/api/git/diff"), "view");
    assert_eq!(scope(Method::GET, "/api/providers"), "manage-providers");
    assert_eq!(scope(Method::PUT, "/api/config"), "manage-providers");
    assert_eq!(scope(Method::GET, "/api/config"), "view");
    assert_eq!(scope(Method::GET, "/api/import/scan"), "manage-providers");
    assert_eq!(scope(Method::POST, "/api/import"), "manage-providers");
    assert_eq!(scope(Method::POST, "/api/push/subscribe"), "view");
    assert_eq!(scope(Method::GET, "/api/remote-access"), "owner");
    assert_eq!(scope(Method::DELETE, "/api/clients/abc"), "owner");
    assert_eq!(scope(Method::GET, "/"), "owner");
    assert_eq!(scope(Method::GET, "/app.js"), "owner");
}

#[test]
fn host_policy_blocks_unknown_domains_but_not_addresses_or_tunnels() {
    let access = Access::new(
        None,
        None,
        &["phone.example.com".into(), "*.corp.example".into()],
        &[],
    );
    for ok in [
        "127.0.0.1:7420",
        "[::1]:7420",
        "192.168.1.5:7420",
        "localhost:7420",
        "mybox:7420",
        "laptop.local",
        "x.tail1234.ts.net",
        "abc.trycloudflare.com",
        "phone.example.com:443",
        "a.corp.example",
    ] {
        assert!(access.host_allowed(ok), "{ok} should be allowed");
    }
    for bad in [
        "evil.com",
        "evil.com:7420",
        "127.0.0.1.evil.com",
        "example.com",
    ] {
        assert!(!access.host_allowed(bad), "{bad} should be refused");
    }
}

#[test]
fn forwarded_for_is_only_believed_from_trusted_peers() {
    let proxy: IpAddr = "10.0.0.2".parse().unwrap();
    let loopback: IpAddr = "127.0.0.1".parse().unwrap();
    let stranger: IpAddr = "203.0.113.9".parse().unwrap();
    let real: IpAddr = "198.51.100.7".parse().unwrap();
    assert_eq!(
        client_ip(Some(loopback), Some("198.51.100.7"), &[]),
        Some(real)
    );
    assert_eq!(
        client_ip(Some(stranger), Some("198.51.100.7"), &[]),
        Some(stranger)
    );
    assert_eq!(
        client_ip(Some(proxy), Some("198.51.100.7, 10.0.0.2"), &[proxy]),
        Some(real)
    );
    assert_eq!(
        client_ip(Some(loopback), Some("garbage"), &[]),
        Some(loopback)
    );
    assert_eq!(client_ip(None, Some("198.51.100.7"), &[]), None);
}

#[tokio::test]
async fn revoking_one_paired_phone_leaves_the_other_connected() {
    let h = harness("revoke").await;
    let a = pair(&h, "full", "Phone A").await;
    let b = pair(&h, "full", "Phone B").await;
    assert_eq!(
        send(&h, "GET", &format!("/{a}/api/sessions"), None).await.0,
        StatusCode::OK
    );
    assert_eq!(
        send(&h, "GET", &format!("/{b}/api/sessions"), None).await.0,
        StatusCode::OK
    );

    let (_, listing) = send(&h, "GET", "/tok/api/remote-access", None).await;
    let id = listing["clients"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["label"] == "Phone A")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        send(&h, "DELETE", &format!("/tok/api/clients/{id}"), None)
            .await
            .0,
        StatusCode::OK
    );

    assert_eq!(
        send(&h, "GET", &format!("/{a}/api/sessions"), None).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        send(&h, "GET", &format!("/{b}/api/sessions"), None).await.0,
        StatusCode::OK
    );
    assert_eq!(
        send(&h, "GET", "/tok/api/sessions", None).await.0,
        StatusCode::OK,
        "owner unaffected"
    );
    let _ = std::fs::remove_dir_all(&h.dir);
}

#[tokio::test]
async fn a_viewer_reads_but_gets_typed_refusals_for_everything_else() {
    let h = harness("viewer").await;
    let viewer = pair(&h, "viewer", "Kitchen tablet").await;
    assert_eq!(
        send(&h, "GET", &format!("/{viewer}/api/sessions"), None)
            .await
            .0,
        StatusCode::OK
    );
    for (method, path, scope) in [
        ("POST", "/api/sessions", "prompt"),
        ("POST", "/api/answer", "approve"),
        ("GET", "/api/terminals", "terminal"),
        ("PUT", "/api/workspace/file", "workspace-write"),
        ("GET", "/api/providers", "manage-providers"),
    ] {
        let (status, body) = send(&h, method, &format!("/{viewer}{path}"), None).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{method} {path}");
        assert_eq!(body["code"], "scope_denied");
        assert_eq!(body["scope"], scope);
    }
    for path in ["/", "/app.js", "/api/remote-access", "/api/clients/pair"] {
        let (status, body) = send(&h, "GET", &format!("/{viewer}{path}"), None).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{path}");
        assert_eq!(body["scope"], "owner");
    }
    let (_, me) = send(&h, "GET", &format!("/{viewer}/api/whoami"), None).await;
    assert_eq!(me["scopes"], serde_json::json!(["view"]));
    assert_eq!(me["owner"], false);
    let _ = std::fs::remove_dir_all(&h.dir);
}

#[tokio::test]
async fn a_bad_host_header_is_refused_on_the_token_path() {
    let h = harness("host").await;
    let path = "/tok/api/sessions";
    assert_eq!(
        send(&h, "GET", path, Some("evil.example.net")).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        send(&h, "GET", path, Some("127.0.0.1:7420")).await.0,
        StatusCode::OK
    );
    assert_eq!(
        send(&h, "GET", path, Some("phone.example.com")).await.0,
        StatusCode::OK
    );
    assert_eq!(send(&h, "GET", path, None).await.0, StatusCode::OK);
    let (status, _) = send(
        &h,
        "GET",
        "/nottoken/api/sessions",
        Some("evil.example.net"),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "unauthenticated paths reveal nothing new"
    );
    let _ = std::fs::remove_dir_all(&h.dir);
}

#[tokio::test]
async fn a_ws_ticket_stands_in_for_the_bearer_once() {
    let h = harness("ticket").await;
    let viewer = pair(&h, "viewer", "tv").await;
    let (status, body) = send(&h, "POST", &format!("/{viewer}/api/ws-ticket"), None).await;
    assert_eq!(status, StatusCode::OK);
    let ticket = body["ticket"].as_str().unwrap().to_string();
    let path = format!("/ws?session=none&ticket={ticket}");
    let (first, _) = send(&h, "GET", &path, None).await;
    assert!(
        first != StatusCode::NOT_FOUND && first != StatusCode::FORBIDDEN,
        "routed: {first}"
    );
    let (second, _) = send(&h, "GET", &path, None).await;
    assert_eq!(
        second,
        StatusCode::NOT_FOUND,
        "a ticket is spent after one use"
    );
    let (terminal, body) = send(&h, "POST", &format!("/{viewer}/api/ws-ticket"), None).await;
    assert_eq!(terminal, StatusCode::OK);
    let t2 = body["ticket"].as_str().unwrap();
    let (denied, body) = send(
        &h,
        "GET",
        &format!("/ws/terminal?session=x&ticket={t2}"),
        None,
    )
    .await;
    assert_eq!(
        denied,
        StatusCode::FORBIDDEN,
        "a ticket carries the client's scopes"
    );
    assert_eq!(body["scope"], "terminal");
    let _ = std::fs::remove_dir_all(&h.dir);
}

#[tokio::test]
async fn approvals_and_refusals_land_in_the_audit_log() {
    let h = harness("audit").await;
    let viewer = pair(&h, "viewer", "tv").await;
    let _ = send(&h, "POST", &format!("/{viewer}/api/answer"), None).await;
    let _ = send(&h, "POST", "/tok/api/answer", None).await;

    let gate = InputGate::new(ClientAuth::owner(), "sess-1");
    assert!(gate
        .check(r#"{"kind":"allow","yes":true,"seq":3}"#)
        .is_none());
    let viewer_auth = access().clients.authenticate(&viewer).unwrap();
    let gate = InputGate::new(viewer_auth, "sess-1");
    assert_eq!(
        gate.check(r#"{"kind":"prompt","text":"hi"}"#).as_deref(),
        Some("scope_denied:prompt")
    );
    assert_eq!(
        gate.check(r#"{"kind":"allow","yes":true,"seq":3}"#)
            .as_deref(),
        Some("scope_denied:approve")
    );

    let log = std::fs::read_to_string(h.dir.join(AUDIT_FILE)).unwrap();
    let lines: Vec<serde_json::Value> = log
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let has = |f: &dyn Fn(&serde_json::Value) -> bool| lines.iter().any(f);
    assert!(has(&|l| l["client"] == "owner"
        && l["route"] == "POST /api/answer"
        && l["decision"] == "allow"));
    assert!(has(&|l| l["label"] == "tv"
        && l["decision"] == "deny:approve"
        && l["route"] == "POST /api/answer"));
    assert!(has(&|l| l["route"] == "WS input allow"
        && l["detail"] == "approve"
        && l["decision"] == "allow"
        && l["session"] == "sess-1"));
    assert!(has(
        &|l| l["route"] == "WS input prompt" && l["decision"] == "deny"
    ));
    assert!(!log.contains(&viewer), "client tokens are never logged");
    let _ = std::fs::remove_dir_all(&h.dir);
}
