//! Two mock sessions in an isolated daemon: list, send, wake an idle recipient, reply, and the
//! guards; plus a second "machine" behind a stand-in `RemoteHosts`.

use std::sync::Arc;

use forge_core::fleet::{
    FleetMessaging, ListFilter, MessageMode, CHAIN_HARD_LIMIT, RATE_PER_MINUTE,
};
use forge_store::Store;
use tower::ServiceExt;

use super::*;
use crate::cli::commands::run::driver::daemon_fleet::DaemonFleetMessaging;
use crate::cli::commands::run::{spawn_session_driver, DriverSpec};
use crate::serve::tests::FORGE_DB_LOCK;
use crate::serve::{daemon_router, DaemonState};

pub(crate) async fn spawn(
    registry: &Arc<SessionRegistry>,
    store: &Arc<Store>,
    title: &str,
) -> String {
    let handle = spawn_session_driver(DriverSpec {
        cwd: std::env::temp_dir().display().to_string(),
        worktree: None,
        title: title.into(),
        mock: true,
        model: None,
        resume: None,
        temper: None,
        push: None,
        apns: None,
        registry: None,
        dispatch_coordinator: false,
        dispatch_worker: false,
    })
    .await
    .unwrap();
    registry.insert(handle, store).await.session_id.clone()
}

/// The daemon-side fleet plus the session id it acts as (the core passes this per call).
struct As {
    fleet: DaemonFleetMessaging,
    me: String,
}

impl As {
    async fn list(&self, filter: &ListFilter) -> forge_core::fleet::SessionListing {
        self.fleet.list(&self.me, filter).await
    }

    async fn send(
        &self,
        address: &str,
        mode: MessageMode,
        text: &str,
    ) -> Result<forge_core::fleet::SendReceipt, String> {
        self.fleet.send(&self.me, address, mode, text).await
    }
}

fn fleet_of(registry: &Arc<SessionRegistry>, store: &Arc<Store>, id: &str) -> As {
    As {
        fleet: DaemonFleetMessaging {
            registry: registry.clone(),
            store: store.clone(),
        },
        me: id.to_string(),
    }
}

pub(crate) async fn wait_for_prompt(store: &Store, session: &str, needle: &str) -> String {
    for _ in 0..200 {
        let found = store
            .load_all_messages(session)
            .unwrap()
            .into_iter()
            .find(|m| m.content.contains(needle));
        if let Some(m) = found {
            return m.content;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("session {session} never saw {needle:?}");
}

/// A second machine: its own registry and store, reached through the trait the relay implements.
struct OtherMachine {
    registry: Arc<SessionRegistry>,
    store: Arc<Store>,
    online: bool,
}

#[async_trait::async_trait]
impl RemoteHosts for OtherMachine {
    fn local_name(&self) -> String {
        "laptop".into()
    }

    async fn list(&self) -> Vec<RemoteHostSessions> {
        let status = |online| FleetHostStatus {
            name: "ssu-dev".into(),
            online,
            last_seen: Some("2026-10-12T08:00:00Z".into()),
            error: (!online).then(|| "host did not answer".to_string()),
        };
        if !self.online {
            return vec![RemoteHostSessions {
                status: status(false),
                sessions: Vec::new(),
            }];
        }
        let mut sessions = local_sessions(&self.store, &self.registry, None).await;
        for s in &mut sessions {
            s.host = Some("ssu-dev".into());
        }
        vec![RemoteHostSessions {
            status: status(true),
            sessions,
        }]
    }

    async fn send(
        &self,
        _host: &str,
        session_id: &str,
        message: &RemoteMessage,
    ) -> Result<(), String> {
        let sessions = local_sessions(&self.store, &self.registry, None).await;
        let info = sessions.iter().find(|s| s.id == session_id);
        let sender = Sender {
            kind: "session",
            id: Some(message.sender_id.clone()),
            label: message.sender_label.clone(),
        };
        submit_local(
            &self.store,
            &self.registry,
            &sender,
            session_id,
            info,
            message.mode,
            &message.text,
        )
        .await
        .map(|_| ())
    }
}

fn backdate(dir: &std::path::Path, id: &str, at: i64) {
    rusqlite::Connection::open(dir.join("fleet.db"))
        .unwrap()
        .execute(
            "UPDATE fleet_message SET created_at = ?1 WHERE id = ?2",
            (at, id),
        )
        .unwrap();
}

pub(crate) fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("forge-fleet-peers-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn list_send_wake_reply_and_the_guards() {
    let _env = FORGE_DB_LOCK.lock().await;
    let dir = scratch("local");
    std::env::set_var("FORGE_DB", dir.join("fleet.db"));
    let store = Arc::new(crate::open_store().unwrap());
    let registry = Arc::new(SessionRegistry::new());
    let a = spawn(&registry, &store, "alpha").await;
    let b = spawn(&registry, &store, "beta").await;
    let fleet_a = fleet_of(&registry, &store, &a);
    let fleet_b = fleet_of(&registry, &store, &b);

    // A lists: both sessions, and only A is "me".
    let listing = fleet_a.list(&ListFilter::default()).await;
    assert_eq!(listing.sessions.len(), 2);
    let me: Vec<_> = listing.sessions.iter().filter(|s| s.is_me).collect();
    assert_eq!(me.len(), 1);
    assert_eq!(me[0].id, a);
    assert!(listing.sessions.iter().all(|s| s.host.is_none()));
    let filtered = fleet_a
        .list(&ListFilter {
            cwd: Some("/definitely/not/here".into()),
            local_only: false,
        })
        .await;
    assert!(filtered.sessions.is_empty());

    // Self-send is refused, an unknown name lists candidates.
    let err = fleet_a
        .send("alpha", MessageMode::FollowUp, "hi me")
        .await
        .unwrap_err();
    assert!(err.contains("own session"), "{err}");
    let err = fleet_a
        .send("nobody", MessageMode::FollowUp, "x")
        .await
        .unwrap_err();
    assert!(err.contains("beta") && err.contains("alpha"), "{err}");

    // A -> B by title: B is idle, so the message starts a turn and lands labelled.
    let receipt = fleet_a
        .send("beta", MessageMode::FollowUp, "please check the build")
        .await
        .unwrap();
    assert_eq!(receipt.id, b);
    assert!(!receipt.no_reply);
    let seen = wait_for_prompt(&store, &b, "please check the build").await;
    assert!(
        seen.starts_with("[message from session \"alpha\""),
        "{seen}"
    );
    assert!(seen.contains("not from your user"));
    assert!(seen.contains(&format!("message_session to {}", &a[..8])));

    // B replies to A by id prefix.
    fleet_b
        .send(&a[..8], MessageMode::FollowUp, "build is green")
        .await
        .unwrap();
    let reply = wait_for_prompt(&store, &a, "build is green").await;
    assert!(reply.contains("\"beta\""), "{reply}");

    // Rate limit: a burst beyond RATE_PER_MINUTE to one session is refused.
    let mut refused = None;
    for i in 0..(RATE_PER_MINUTE + 2) {
        if let Err(e) = fleet_a
            .send("beta", MessageMode::Steer, &format!("burst {i}"))
            .await
        {
            refused = Some(e);
            break;
        }
    }
    assert!(refused
        .expect("rate limit must trip")
        .contains("rate limit"));

    // Ping-pong guard: an unbroken run of agent messages is delivered with a no-auto-reply
    // notice and then refused. Pre-age the rows so the per-minute limit stays out of the way.
    let c = spawn(&registry, &store, "gamma").await;
    let fleet_c = fleet_of(&registry, &store, &c);
    let now = chrono::Utc::now().timestamp();
    for i in 0..forge_core::fleet::AUTO_REPLY_LIMIT - 1 {
        let id = format!("seed-{i}");
        store
            .enqueue_fleet_message(&id, "session", Some(&a), "alpha", &c, "x", "follow_up")
            .unwrap();
        store.mark_fleet_message_delivered(&id, now).unwrap();
        backdate(&dir, &id, now - 300 + i);
    }
    let receipt = fleet_a
        .send("gamma", MessageMode::FollowUp, "one more")
        .await
        .unwrap();
    assert!(
        receipt.no_reply,
        "depth at the limit asks the recipient not to auto-reply"
    );
    let framed = wait_for_prompt(&store, &c, "one more").await;
    assert!(framed.contains("do NOT reply automatically"), "{framed}");
    for i in 0..(CHAIN_HARD_LIMIT - forge_core::fleet::AUTO_REPLY_LIMIT) {
        let id = format!("deep-{i}");
        store
            .enqueue_fleet_message(&id, "session", Some(&a), "alpha", &c, "x", "follow_up")
            .unwrap();
        store.mark_fleet_message_delivered(&id, now).unwrap();
        backdate(&dir, &id, now - 200 + i);
    }
    let err = fleet_a
        .send("gamma", MessageMode::FollowUp, "too deep")
        .await
        .unwrap_err();
    assert!(err.contains("depth limit"), "{err}");
    // A human message to the recipient breaks the run.
    let _ = fleet_c;
    store
        .enqueue_fleet_message("human", "cli", None, "cli", &c, "carry on", "follow_up")
        .unwrap();
    store.mark_fleet_message_delivered("human", now).unwrap();
    fleet_a
        .send("gamma", MessageMode::FollowUp, "after the human")
        .await
        .unwrap();

    std::env::remove_var("FORGE_DB");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn http_routes_list_and_message_like_the_bridge_does() {
    let _env = FORGE_DB_LOCK.lock().await;
    let dir = scratch("http");
    std::env::set_var("FORGE_DB", dir.join("fleet.db"));
    let store = Arc::new(crate::open_store().unwrap());
    let registry = Arc::new(SessionRegistry::new());
    let a = spawn(&registry, &store, "alpha").await;
    let b = spawn(&registry, &store, "beta").await;
    let state = Arc::new(DaemonState {
        registry: registry.clone(),
        terminals: Arc::new(crate::serve_terminal::TerminalRegistry::new()),
        store: store.clone(),
        base: "/tok".into(),
        mock: true,
        default_cwd: dir.display().to_string(),
        project_roots: Vec::new(),
        push: None,
        apns: None,
        voice: crate::voice::VoiceState::new(),
        anywhere_enable: tokio::sync::watch::channel(false).0,
    });
    let router = daemon_router(state);
    let body_of = |resp: axum::response::Response| async move {
        let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
            .await
            .unwrap();
        serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()
    };

    let resp = router
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .uri(format!("/tok/api/peers/sessions?self={a}"))
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), axum::http::StatusCode::OK);
    let listing = body_of(resp).await;
    let rows = listing["sessions"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows
        .iter()
        .any(|r| r["id"] == a.as_str() && r["is_me"] == true));

    let post = |body: serde_json::Value| {
        axum::http::Request::builder()
            .method(axum::http::Method::POST)
            .uri("/tok/api/peers/message")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(body.to_string()))
            .unwrap()
    };
    let resp = router
        .clone()
        .oneshot(post(serde_json::json!({
            "target": "beta", "text": "from the bridge", "sender_id": a
        })))
        .await
        .unwrap();
    assert_eq!(resp.status(), axum::http::StatusCode::OK);
    let receipt = body_of(resp).await;
    assert_eq!(receipt["id"], b.as_str());
    let seen = wait_for_prompt(&store, &b, "from the bridge").await;
    assert!(
        seen.contains("\"alpha\""),
        "sender title is read live: {seen}"
    );

    let resp = router
        .clone()
        .oneshot(post(serde_json::json!({
            "target": "alpha", "text": "to myself", "sender_id": a
        })))
        .await
        .unwrap();
    assert_eq!(resp.status(), axum::http::StatusCode::BAD_REQUEST);
    assert!(body_of(resp).await["error"]
        .as_str()
        .unwrap()
        .contains("own session"));

    std::env::remove_var("FORGE_DB");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn other_machines_are_listed_addressed_and_degrade_when_offline() {
    let _env = FORGE_DB_LOCK.lock().await;
    let dir = scratch("remote");
    let home = dir.join("home.db");
    let away = dir.join("away.db");
    std::env::set_var("FORGE_DB", &home);
    let store = Arc::new(crate::open_store().unwrap());
    let registry = Arc::new(SessionRegistry::new());
    let a = spawn(&registry, &store, "alpha").await;

    std::env::set_var("FORGE_DB", &away);
    let away_store = Arc::new(crate::open_store().unwrap());
    let away_registry = Arc::new(SessionRegistry::new());
    let r = spawn(&away_registry, &away_store, "worker").await;
    std::env::remove_var("FORGE_DB");

    // Anywhere off: purely local, and a host-qualified address just misses.
    let fleet_a = fleet_of(&registry, &store, &a);
    let listing = fleet_a.list(&ListFilter::default()).await;
    assert!(listing.hosts.is_empty() && listing.sessions.iter().all(|s| s.host.is_none()));
    assert!(fleet_a
        .send("ssu-dev/worker", MessageMode::FollowUp, "x")
        .await
        .is_err());

    registry.set_remote_hosts(Some(Arc::new(OtherMachine {
        registry: away_registry.clone(),
        store: away_store.clone(),
        online: true,
    })));
    let listing = fleet_a.list(&ListFilter::default()).await;
    let remote_row = listing
        .sessions
        .iter()
        .find(|s| s.host.as_deref() == Some("ssu-dev"))
        .expect("the other machine's session is listed");
    assert_eq!(remote_row.id, r);
    assert_eq!(listing.hosts.len(), 1);
    assert!(listing.hosts[0].online);
    let text = forge_core::fleet::format_listing(&listing, &ListFilter::default());
    assert!(text.contains("host ssu-dev (1 session(s))"), "{text}");

    // A client paired to this host alone must not reach the account's other machines.
    let denied = super::send_from_scoped(
        &store,
        &registry,
        &super::Sender::cli(),
        "worker",
        MessageMode::FollowUp,
        "scoped",
        true,
    )
    .await
    .unwrap_err();
    assert!(denied.contains("no session matches"), "{denied}");

    // Bare name resolves across machines when nothing local matches; the receipt says where.
    let receipt = fleet_a
        .send("worker", MessageMode::FollowUp, "cross-host hello")
        .await
        .unwrap();
    assert_eq!(receipt.host.as_deref(), Some("ssu-dev"));
    assert_eq!(
        receipt.to_string(),
        format!("message sent to ssu-dev/worker ({}) [follow_up]", &r[..8])
    );
    let seen = wait_for_prompt(&away_store, &r, "cross-host hello").await;
    assert!(seen.contains("\"laptop/alpha\""), "{seen}");
    assert!(
        seen.contains(&format!("message_session to laptop/{}", &a[..8])),
        "the reply address is host-qualified: {seen}"
    );
    // The receiving host's guards key the sender as <host>/<id>.
    assert_eq!(
        away_store
            .fleet_pair_activity(&format!("laptop/{a}"), &r, chrono::Utc::now().timestamp())
            .unwrap()
            .last_minute,
        1
    );
    assert!(fleet_a
        .send("ssu-dev/worker", MessageMode::Steer, "qualified")
        .await
        .is_ok());

    // Offline host: listed as offline with last-seen, the tool does not fail.
    registry.set_remote_hosts(Some(Arc::new(OtherMachine {
        registry: away_registry,
        store: away_store,
        online: false,
    })));
    let listing = fleet_a.list(&ListFilter::default()).await;
    assert!(!listing.hosts[0].online);
    let text = forge_core::fleet::format_listing(&listing, &ListFilter::default());
    assert!(
        text.contains("host ssu-dev: OFFLINE (last seen 2026-10-12T08:00:00Z)"),
        "{text}"
    );
    let local_only = fleet_a
        .list(&ListFilter {
            cwd: None,
            local_only: true,
        })
        .await;
    assert!(local_only.hosts.is_empty());

    let _ = std::fs::remove_dir_all(&dir);
}

/// A `forge chat` terminal is not in the registry: it is reached through the control channel it
/// published, with the same `RemoteInput` frame the phone app uses.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_terminal_session_receives_through_its_published_control_channel() {
    let store = Arc::new(Store::open_in_memory().unwrap());
    let registry = Arc::new(SessionRegistry::new());
    let term = store.create_session("/work/project", "Default").unwrap();
    store.set_session_local_live(&term, true).unwrap();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (frames_tx, mut frames_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    tokio::spawn(async move {
        use futures::StreamExt as _;
        while let Ok((stream, _)) = listener.accept().await {
            let tx = frames_tx.clone();
            tokio::spawn(async move {
                let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
                while let Some(Ok(msg)) = ws.next().await {
                    if let tokio_tungstenite::tungstenite::Message::Text(t) = msg {
                        let _ = tx.send(t.to_string());
                    }
                }
            });
        }
    });

    // Watch-only until it publishes a control channel: refused, and nothing is left pending.
    let sender = Sender {
        kind: "session",
        id: Some("sender-session".into()),
        label: "builder".into(),
    };
    let err = send_from(
        &store,
        &registry,
        &sender,
        &term[..8],
        MessageMode::FollowUp,
        "hello",
    )
    .await
    .unwrap_err();
    assert!(err.contains("watch-only"), "{err}");
    assert!(store.pending_fleet_messages_for(&term).unwrap().is_empty());

    store
        .set_session_local_control_url(&term, Some(&format!("http://127.0.0.1:{port}/tok")))
        .unwrap();
    let listing = list_all(&store, &registry, None, &ListFilter::default()).await;
    assert!(listing.sessions[0].terminal && listing.sessions[0].can_receive);

    let receipt = send_from(
        &store,
        &registry,
        &sender,
        &term[..8],
        MessageMode::Steer,
        "ship it",
    )
    .await
    .unwrap();
    assert_eq!(receipt.id, term);
    let frame = tokio::time::timeout(std::time::Duration::from_secs(5), frames_rx.recv())
        .await
        .unwrap()
        .unwrap();
    let input: crate::remote::RemoteInput = serde_json::from_str(&frame).unwrap();
    let crate::remote::RemoteInput::Steer { text } = input else {
        panic!("a steer message is a steer frame: {frame}");
    };
    assert!(
        text.starts_with("[message from session \"builder\" (sender-s)]"),
        "{text}"
    );
    assert!(text.ends_with("---\nship it"));
    assert!(store.pending_fleet_messages_for(&term).unwrap().is_empty());
}
