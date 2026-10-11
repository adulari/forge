use super::*;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use forge_core::push_notify::{format_result, NO_CHANNEL};
use forge_store::Store;
use p256::elliptic_curve::sec1::ToSec1Point;
use tower::ServiceExt;

fn request(message: &str) -> PushRequest {
    PushRequest {
        message: message.into(),
        title: Some("CI".into()),
        priority: Priority::Normal,
        session_id: "s1".into(),
    }
}

fn channels(store: Arc<Store>) -> Channels {
    Channels {
        store,
        web: None,
        apns: None,
        ambient: false,
    }
}

fn web_notifier(store: Arc<Store>) -> Arc<crate::push::PushNotifier> {
    Arc::new(
        crate::push::PushNotifier::with_key(store, crate::push::VapidKey::from_scalar(&[9u8; 32]))
            .unwrap(),
    )
}

#[tokio::test]
async fn nothing_configured_yields_no_channel_not_a_failure() {
    let reports = channels(Arc::new(Store::open_in_memory().unwrap()))
        .deliver(&request("hello there"))
        .await;
    assert!(reports.is_empty());
    assert_eq!(format_result(&reports).0, NO_CHANNEL);
}

#[tokio::test]
async fn configured_but_unsubscribed_channels_report_why_they_failed() {
    let store = Arc::new(Store::open_in_memory().unwrap());
    let c = Channels {
        web: Some(web_notifier(store.clone())),
        apns: Some(Arc::new(
            crate::apns::ApnsNotifier::new_relay(
                store.clone(),
                "https://relay.example.test".into(),
                None,
            )
            .unwrap(),
        )),
        ..channels(store)
    };
    let reports = c.deliver(&request("hello there")).await;
    let names: Vec<_> = reports
        .iter()
        .map(|r| (r.channel.as_str(), r.delivered))
        .collect();
    assert_eq!(names, [("web-push", false), ("phone-apns", false)]);
}

#[tokio::test]
async fn web_push_carries_the_real_text_encrypted_to_the_subscriber() {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
    let service = Router::new().route(
        "/wp/sub",
        axum::routing::post(move |body: axum::body::Bytes| {
            let tx = tx.clone();
            async move {
                let _ = tx.send(body.to_vec());
                StatusCode::CREATED
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, service).await.unwrap() });

    let ua_secret = p256::SecretKey::from_slice(&[42u8; 32]).unwrap();
    let ua_public = ua_secret.public_key().to_sec1_point(false);
    let auth = [7u8; 16];
    let b64 =
        |b: &[u8]| base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, b);
    let store = Arc::new(Store::open_in_memory().unwrap());
    store
        .upsert_push_subscription(
            &format!("http://{addr}/wp/sub"),
            &b64(ua_public.as_bytes()),
            &b64(&auth),
        )
        .unwrap();
    let c = Channels {
        web: Some(web_notifier(store.clone())),
        ..channels(store)
    };

    let reports = c.deliver(&request("Deploy finished")).await;
    assert!(reports[0].delivered, "{reports:?}");
    let sealed = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .unwrap()
        .unwrap();
    let plain = crate::push::decrypt_payload(&ua_secret, &auth, &sealed).unwrap();
    let json: serde_json::Value = serde_json::from_slice(&plain).unwrap();
    assert_eq!(json["kind"], "notify");
    assert_eq!(json["title"], "CI");
    assert_eq!(json["body"], "Deploy finished");
}

#[test]
fn public_relay_is_reported_as_generic_and_a_private_one_is_not() {
    let store = Arc::new(Store::open_in_memory().unwrap());
    let make =
        |url: &str| crate::apns::ApnsNotifier::new_relay(store.clone(), url.into(), None).unwrap();
    assert!(make("https://forge.adulari.dev/relay").alerts_are_generic());
    assert!(!make("https://relay.example.test").alerts_are_generic());
}

#[test]
fn desktop_command_maps_urgency_and_never_interprets_the_message() {
    if cfg!(target_os = "linux") {
        let (program, args) = desktop_command("T", "--urgency=low", Priority::High).unwrap();
        assert_eq!(program, "notify-send");
        assert!(args.contains(&"--urgency=critical".to_string()));
        let dashes = args.iter().position(|a| a == "--").unwrap();
        assert_eq!(&args[dashes + 1..], ["T", "--urgency=low"]);
        let (_, normal) = desktop_command("T", "b", Priority::Normal).unwrap();
        assert!(normal.contains(&"--urgency=normal".to_string()));
    }
}

#[test]
fn notices_are_ordered_bounded_and_filterable() {
    let first = notices::record(&request("notice-order-one"));
    let second = notices::record(&request("notice-order-two"));
    assert!(second > first);
    let (items, latest) = notices::since(first);
    assert!(latest >= second);
    assert!(items.iter().all(|n| n.seq > first));
    assert!(items.iter().any(|n| n.message == "notice-order-two"));
    let mut local = notices::Log::default();
    for i in 0..60 {
        local.push(&request(&format!("flood {i}")));
    }
    assert_eq!(local.len(), 50);
}

fn router() -> Router {
    let state = Arc::new(crate::serve::DaemonState {
        registry: Arc::new(crate::serve::SessionRegistry::new()),
        terminals: Arc::new(crate::serve_terminal::TerminalRegistry::new()),
        store: Arc::new(Store::open_in_memory().unwrap()),
        base: "/tok".into(),
        mock: true,
        default_cwd: ".".into(),
        project_roots: Vec::new(),
        push: None,
        apns: None,
        voice: crate::voice::VoiceState::new(),
        anywhere_enable: tokio::sync::watch::channel(false).0,
    });
    crate::serve::daemon_router(state)
}

async fn call(router: &Router, request: Request<Body>) -> (StatusCode, serde_json::Value) {
    let response = router.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or_default())
}

fn post(body: serde_json::Value) -> Request<Body> {
    Request::post("/tok/api/notify")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

#[tokio::test]
async fn notify_endpoint_validates_dedupes_records_and_serves_the_inbox() {
    let router = router();

    let (status, body) = call(&router, post(serde_json::json!({"title": "x"}))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"]
        .as_str()
        .unwrap()
        .contains("`message` is required"));
    let (status, _) = call(
        &router,
        post(serde_json::json!({"message": "m", "priority": "loud"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let args = serde_json::json!({"message": "endpoint-inbox-check", "session_id": "ep-1"});
    let (status, body) = call(&router, post(args.clone())).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["ok"], false, "no channel in the test daemon");
    assert!(body["note"]
        .as_str()
        .unwrap()
        .contains("no notification channel"));

    let (status, body) = call(&router, post(args)).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert!(body["error"].as_str().unwrap().contains("already sent"));

    let (status, body) = call(
        &router,
        Request::get("/tok/api/notices?after=0")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let notices = body["notices"].as_array().unwrap();
    assert!(notices
        .iter()
        .any(|n| n["message"] == "endpoint-inbox-check" && n["session_id"] == "ep-1"));
    assert_eq!(
        notices
            .iter()
            .filter(|n| n["message"] == "endpoint-inbox-check")
            .count(),
        1,
        "the suppressed duplicate is not recorded"
    );
}

#[tokio::test]
async fn notify_endpoint_caps_a_looping_session() {
    let router = router();
    let mut statuses = Vec::new();
    for i in 0..6 {
        let (status, _) = call(
            &router,
            post(serde_json::json!({"message": format!("loop {i}"), "session_id": "ep-loop"})),
        )
        .await;
        statuses.push(status);
    }
    let ok = statuses.iter().filter(|s| **s == StatusCode::OK).count();
    assert_eq!(ok, forge_core::push_notify::MAX_PER_WINDOW);
    assert_eq!(statuses.last(), Some(&StatusCode::TOO_MANY_REQUESTS));
}
