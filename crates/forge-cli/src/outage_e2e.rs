//! Layer 2 of the provider-outage drill: a daemon-hosted session against a fake OpenAI-compatible
//! gateway that answers `503 no_capacity` for a few seconds, then a healthy reply.
//!
//! Real driver, real provider code and an isolated store; the gateway and the push service are
//! loopback stand-ins. It proves what a person would see overnight: the turn waits instead of
//! failing, `snapshot.outage` is set during the wait and cleared after, the turn completes with
//! the healthy reply, and the push service gets `outage` and then `recovered`.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::routing::{get, post};
use axum::Router;
use p256::elliptic_curve::sec1::ToSec1Point;

use crate::cli::commands::run::driver::{spawn_session_driver, DriverSpec};
use crate::remote;
use crate::serve::tests::FORGE_DB_LOCK;

const ANSWER: &str = concat!(
    "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"DRILL-OK the gateway is back\"},\"finish_reason\":null}]}\n\n",
    "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
    "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":9,\"completion_tokens\":6,\"total_tokens\":15}}\n\n",
    "data: [DONE]\n\n",
);
const NO_CAPACITY: &str = r#"{"error":{"code":"no_capacity","type":"service_unavailable","message":"No capacity available for drill/flash"}}"#;

/// Restores the process environment this test changes, even when an assertion panics.
struct EnvGuard(Vec<(&'static str, Option<std::ffi::OsString>)>);

impl EnvGuard {
    fn set(vars: &[(&'static str, std::ffi::OsString)]) -> Self {
        let saved = vars
            .iter()
            .map(|(k, v)| {
                let old = std::env::var_os(k);
                std::env::set_var(k, v);
                (*k, old)
            })
            .collect();
        Self(saved)
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (key, old) in self.0.drain(..) {
            match old {
                Some(v) => std::env::set_var(key, v),
                None => std::env::remove_var(key),
            }
        }
        crate::push::OUTAGE_PUSH_AFTER_TEST.store(u64::MAX, Ordering::SeqCst);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_daemon_session_waits_through_a_503_then_completes_and_pushes_both_ends() {
    let _env = FORGE_DB_LOCK.lock().await;
    let dir = std::env::temp_dir().join(format!("forge-outage-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("config/forge")).unwrap();
    std::fs::create_dir_all(dir.join("ws")).unwrap();

    // The fake gateway: 503 no_capacity for `DOWN` after the first chat request, then healthy.
    let down_for = Duration::from_secs(3);
    let first = Arc::new(std::sync::Mutex::new(None::<Instant>));
    let chat_calls = Arc::new(AtomicUsize::new(0));
    let (first_c, calls_c) = (first.clone(), chat_calls.clone());
    let gateway = Router::new()
        .route(
            "/v1/models",
            get(|| async { r#"{"object":"list","data":[{"id":"flash","object":"model"}]}"# }),
        )
        .route(
            "/v1/chat/completions",
            post(move || {
                let (first, calls) = (first_c.clone(), calls_c.clone());
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    let began = *first.lock().unwrap().get_or_insert_with(Instant::now);
                    if began.elapsed() < down_for {
                        (
                            axum::http::StatusCode::SERVICE_UNAVAILABLE,
                            [("content-type", "application/json")],
                            NO_CAPACITY,
                        )
                    } else {
                        (
                            axum::http::StatusCode::OK,
                            [("content-type", "text/event-stream")],
                            ANSWER,
                        )
                    }
                }
            }),
        );
    let gw_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let gw_addr = gw_listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(gw_listener, gateway).await;
    });

    // The push service stand-in captures each POST body.
    let (cap_tx, mut cap_rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
    let push_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let push_addr = push_listener.local_addr().unwrap();
    let push_service = Router::new().route(
        "/wp/{id}",
        post(move |body: axum::body::Bytes| {
            let tx = cap_tx.clone();
            async move {
                let _ = tx.send(body.to_vec());
                axum::http::StatusCode::CREATED
            }
        }),
    );
    tokio::spawn(async move {
        let _ = axum::serve(push_listener, push_service).await;
    });

    std::fs::write(
        dir.join("config/forge/config.toml"),
        format!(
            "[[providers.custom]]\nnamespace = \"drill\"\nbase_url = \"http://{gw_addr}/v1\"\n\
             free = true\nmodels = [\"flash\"]\n"
        ),
    )
    .unwrap();
    let _vars = EnvGuard::set(&[
        ("FORGE_DB", dir.join("outage.db").into()),
        ("XDG_CONFIG_HOME", dir.join("config").into()),
        ("FORGE_PERMISSION_MODE", "default".into()),
        ("FORGE_AUTOFIX__AUTO_DETECT", "false".into()),
        // A one second hot phase hands over to the long wait at once; the long wait's first
        // probe is 12 to 18 seconds out, after the gateway has recovered.
        ("FORGE_MESH__PIN_OUTAGE_WAIT_SECS", "1".into()),
        ("FORGE_MESH__OUTAGE_WAIT_SECS", "600".into()),
    ]);
    crate::push::OUTAGE_PUSH_AFTER_TEST.store(0, Ordering::SeqCst);

    let ua_secret = p256::SecretKey::from_slice(&[42u8; 32]).unwrap();
    let ua_public = ua_secret.public_key().to_sec1_point(false);
    let auth: [u8; 16] = [7u8; 16];
    let store = Arc::new(crate::open_store().unwrap());
    store
        .upsert_push_subscription(
            &format!("http://{push_addr}/wp/sub1"),
            &crate::push::b64url(ua_public.as_bytes()),
            &crate::push::b64url(&auth),
        )
        .unwrap();
    let notifier = Arc::new(
        crate::push::PushNotifier::with_key(
            store.clone(),
            crate::push::VapidKey::from_scalar(&[9u8; 32]),
        )
        .unwrap()
        .permit_any_endpoint(),
    );

    let handle = spawn_session_driver(DriverSpec {
        cwd: dir.join("ws").display().to_string(),
        worktree: None,
        title: "outage-e2e".into(),
        mock: false,
        model: Some("drill::flash".into()),
        resume: None,
        temper: None,
        push: Some(notifier),
        apns: None,
        registry: None,
        dispatch_coordinator: false,
        dispatch_worker: false,
    })
    .await
    .unwrap();

    handle
        .input_tx
        .send(remote::RemoteInput::Prompt {
            text: "say hello".into(),
            attachments: None,
        })
        .await
        .unwrap();

    let started = Instant::now();
    let mut saw_outage = None;
    let finished = loop {
        let snap = handle.snapshot_rx.borrow().snapshot.clone();
        if saw_outage.is_none() {
            saw_outage = snap.outage.clone();
        }
        let answered = snap.transcript.iter().any(|l| l.contains("DRILL-OK"));
        if answered && !snap.busy {
            break snap;
        }
        assert!(
            started.elapsed() < Duration::from_secs(90),
            "the turn never completed; snapshot: {snap:?}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    };

    let outage = saw_outage.expect("snapshot.outage was set while the provider was down");
    assert_eq!(outage.provider, "drill");
    assert!(outage.reason.contains("503"), "{}", outage.reason);
    assert!(
        outage.until_unix > outage.since_unix && outage.next_try_unix >= outage.since_unix,
        "{outage:?}"
    );
    assert!(finished.outage.is_none(), "cleared once the turn completed");
    assert!(
        started.elapsed() >= down_for,
        "the turn waited out the outage: {:?}",
        started.elapsed()
    );
    assert!(
        chat_calls.load(Ordering::SeqCst) >= 2,
        "re-sent after the 503"
    );
    assert!(
        finished.notes.iter().all(|n| !n.contains("Turn failed")),
        "{:?}",
        finished.notes
    );

    // Pushes: `outage`, then `recovered` (a `done` push may follow; it is not asserted on).
    let mut kinds = Vec::new();
    while kinds.iter().filter(|k| *k == "recovered").count() == 0 {
        let sealed = tokio::time::timeout(Duration::from_secs(20), cap_rx.recv())
            .await
            .expect("the push service got the recovery push")
            .expect("capture channel open");
        let plain = crate::push::decrypt_payload(&ua_secret, &auth, &sealed).unwrap();
        let p: serde_json::Value = serde_json::from_slice(&plain).unwrap();
        assert_eq!(p["session"], handle.session_id.as_str());
        kinds.push(p["kind"].as_str().unwrap().to_string());
    }
    let outage_at = kinds.iter().position(|k| k == "outage");
    let recovered_at = kinds.iter().position(|k| k == "recovered");
    assert!(
        matches!((outage_at, recovered_at), (Some(o), Some(r)) if o < r),
        "outage then recovered: {kinds:?}"
    );

    handle.shutdown();
    let _ = std::fs::remove_dir_all(&dir);
}
