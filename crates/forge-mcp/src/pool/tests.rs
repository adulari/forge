use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use forge_config::{McpAllowlist, McpConfig, McpServerConfig, McpTransport};
use serde_json::json;

use super::*;
use crate::testsupport::spawn_echo;
use crate::{McpManager, MCP_CALL};

type Kill = (tokio::sync::oneshot::Sender<()>, Arc<AtomicBool>);

struct Rig {
    pool: Arc<McpPool>,
    connects: Arc<AtomicUsize>,
    servers: Arc<Mutex<Vec<Kill>>>,
}

fn rig(connect_ms: u64, fail: bool) -> Rig {
    let connects = Arc::new(AtomicUsize::new(0));
    let servers: Arc<Mutex<Vec<Kill>>> = Arc::default();
    let (count, kept) = (Arc::clone(&connects), Arc::clone(&servers));
    let connector: Connector = Arc::new(move |server, _deps| {
        let (count, kept) = (Arc::clone(&count), Arc::clone(&kept));
        Box::pin(async move {
            count.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(connect_ms)).await;
            if fail {
                return Err("connection refused".to_string());
            }
            let handle = spawn_echo(&server.name).await;
            kept.lock().push((handle.kill, handle.closed));
            Ok(handle.established)
        })
    });
    Rig {
        pool: Arc::new(McpPool::with_connector(connector)),
        connects,
        servers,
    }
}

fn server(name: &str, transport: McpTransport, shared: Option<bool>) -> McpServerConfig {
    McpServerConfig {
        name: name.into(),
        transport,
        auth: None,
        secret_env: vec![],
        enabled: true,
        shared,
    }
}

fn http(name: &str) -> McpServerConfig {
    server(
        name,
        McpTransport::Http {
            url: "https://helm.example/mcp".into(),
            headers: Default::default(),
        },
        None,
    )
}

fn stdio(name: &str, shared: Option<bool>) -> McpServerConfig {
    server(
        name,
        McpTransport::Stdio {
            command: "forge-test-no-such-binary".into(),
            args: vec![],
            env: Default::default(),
        },
        shared,
    )
}

fn config(servers: Vec<McpServerConfig>) -> McpConfig {
    McpConfig {
        servers,
        connect_timeout_secs: 5,
        ..Default::default()
    }
}

fn manager(rig: &Rig, config: &McpConfig) -> Arc<McpManager> {
    Arc::new(McpManager::connecting(config).with_pool(Arc::clone(&rig.pool)))
}

async fn echo(mgr: &McpManager, msg: &str) -> crate::McpCallOutcome {
    mgr.call(
        MCP_CALL,
        &json!({"name": "test__echo", "arguments": {"msg": msg}}),
    )
    .await
}

fn status(mgr: &McpManager) -> String {
    mgr.status_lines()[0].status.clone()
}

#[tokio::test]
async fn sessions_booting_together_share_one_connect_and_one_connection() {
    let rig = rig(40, false);
    let cfg = config(vec![http("test")]);
    let a = manager(&rig, &cfg);
    let b = manager(&rig, &cfg);
    let c = manager(&rig, &cfg);
    tokio::join!(a.connect_active(), b.connect_active(), c.connect_active());

    assert_eq!(
        rig.connects.load(Ordering::SeqCst),
        1,
        "one handshake for three sessions"
    );
    assert_eq!(rig.pool.live_servers(), 1);
    for mgr in [&a, &b, &c] {
        assert_eq!(status(mgr), "connected");
        assert_eq!(mgr.status_lines()[0].tools, 2);
        let out = echo(mgr, "hi").await;
        assert!(out.ok && out.text.contains("echo: hi"), "{out:?}");
    }
}

#[tokio::test]
async fn a_stdio_server_is_private_unless_it_opts_in() {
    let rig = rig(0, false);
    let cfg = config(vec![stdio("test", None)]);
    let a = manager(&rig, &cfg);
    let b = manager(&rig, &cfg);
    tokio::join!(a.connect_active(), b.connect_active());
    assert_eq!(
        rig.connects.load(Ordering::SeqCst),
        0,
        "the pool never saw it"
    );
    assert_eq!(rig.pool.live_servers(), 0);
    assert_eq!(
        status(&a),
        "failed",
        "each session tried to spawn its own child"
    );
    assert_eq!(status(&b), "failed");

    let cfg = config(vec![stdio("test", Some(true))]);
    let (c, d) = (manager(&rig, &cfg), manager(&rig, &cfg));
    tokio::join!(c.connect_active(), d.connect_active());
    assert_eq!(
        rig.connects.load(Ordering::SeqCst),
        1,
        "opted-in stdio is one child"
    );
    assert_eq!(status(&c), "connected");
    assert_eq!(status(&d), "connected");
}

#[tokio::test]
async fn an_http_server_can_opt_out_of_sharing() {
    let rig = rig(0, false);
    let mut private = http("test");
    private.shared = Some(false);
    let cfg = config(vec![private]);
    let a = manager(&rig, &cfg);
    a.connect_active().await;
    assert_eq!(
        rig.connects.load(Ordering::SeqCst),
        0,
        "not pooled: it takes the real transport"
    );
    assert_eq!(rig.pool.live_servers(), 0);
}

#[tokio::test]
async fn different_definitions_do_not_share() {
    let rig = rig(0, false);
    let a = manager(&rig, &config(vec![http("test")]));
    let mut other = http("test");
    other.transport = McpTransport::Http {
        url: "https://elsewhere.example/mcp".into(),
        headers: Default::default(),
    };
    let b = manager(&rig, &config(vec![other]));
    tokio::join!(a.connect_active(), b.connect_active());
    assert_eq!(rig.connects.load(Ordering::SeqCst), 2);
    assert_eq!(rig.pool.live_servers(), 2);
}

#[tokio::test]
async fn the_connection_closes_with_its_last_session_and_comes_back_on_demand() {
    let rig = rig(0, false);
    let cfg = config(vec![http("test")]);
    let a = manager(&rig, &cfg);
    let b = manager(&rig, &cfg);
    tokio::join!(a.connect_active(), b.connect_active());
    let closed = Arc::clone(&rig.servers.lock()[0].1);

    drop(a);
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(
        !closed.load(Ordering::SeqCst),
        "another session still uses it"
    );
    assert!(echo(&b, "still here").await.ok);

    drop(b);
    for _ in 0..100 {
        if closed.load(Ordering::SeqCst) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(
        closed.load(Ordering::SeqCst),
        "the last lease closed the connection"
    );
    assert_eq!(rig.pool.live_servers(), 0);

    let c = manager(&rig, &cfg);
    c.connect_active().await;
    assert_eq!(
        rig.connects.load(Ordering::SeqCst),
        2,
        "connected afresh on demand"
    );
    assert!(echo(&c, "reborn").await.ok);
}

#[tokio::test]
async fn a_dropped_connection_is_repaired_once_for_every_session() {
    let rig = rig(0, false);
    let cfg = config(vec![http("test")]);
    let a = manager(&rig, &cfg);
    let b = manager(&rig, &cfg);
    tokio::join!(a.connect_active(), b.connect_active());
    assert_eq!(rig.connects.load(Ordering::SeqCst), 1);

    let (kill, _) = rig.servers.lock().remove(0);
    kill.send(()).unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;

    let broken = echo(&a, "x").await;
    assert!(
        !broken.ok && broken.text.contains("disconnected"),
        "{broken:?}"
    );
    assert_eq!(
        status(&b),
        "reconnecting",
        "the drop is visible to every session"
    );

    let healed = echo(&a, "again").await;
    assert!(
        healed.ok && healed.text.contains("echo: again"),
        "{healed:?}"
    );
    let adopted = echo(&b, "me too").await;
    assert!(
        adopted.ok && adopted.text.contains("echo: me too"),
        "{adopted:?}"
    );
    assert_eq!(
        rig.connects.load(Ordering::SeqCst),
        2,
        "one reconnect served both sessions"
    );
    assert_eq!(status(&a), "connected");
    assert_eq!(status(&b), "connected");
}

#[tokio::test]
async fn a_failed_connect_is_not_repeated_by_every_waiting_session() {
    let rig = rig(30, true);
    let cfg = config(vec![http("test")]);
    let (a, b, c) = (
        manager(&rig, &cfg),
        manager(&rig, &cfg),
        manager(&rig, &cfg),
    );
    tokio::join!(a.connect_active(), b.connect_active(), c.connect_active());
    assert_eq!(
        rig.connects.load(Ordering::SeqCst),
        1,
        "waiters take the first failure"
    );
    for mgr in [&a, &b, &c] {
        assert_eq!(status(mgr), "failed");
        assert!(mgr.status_lines()[0]
            .detail
            .as_deref()
            .is_some_and(|d| d.contains("connection refused")));
    }
}

#[tokio::test]
async fn each_session_keeps_its_own_tool_policy_over_a_shared_connection() {
    let rig = rig(0, false);
    let open = config(vec![http("test")]);
    let mut locked = config(vec![http("test")]);
    locked.allow = McpAllowlist {
        servers: vec![],
        tools: vec!["test__boom".into()],
    };
    let (a, b) = (manager(&rig, &open), manager(&rig, &locked));
    tokio::join!(a.connect_active(), b.connect_active());
    assert_eq!(
        rig.connects.load(Ordering::SeqCst),
        1,
        "same server, one connection"
    );

    assert!(echo(&a, "allowed").await.ok);
    let denied = echo(&b, "nope").await;
    assert!(
        !denied.ok && denied.text.contains("denied by policy"),
        "{denied:?}"
    );
}

#[tokio::test]
async fn a_tool_list_change_reaches_every_session() {
    let rig = rig(0, false);
    let cfg = config(vec![http("test")]);
    let (a, b) = (manager(&rig, &cfg), manager(&rig, &cfg));
    tokio::join!(a.connect_active(), b.connect_active());
    let entry = a
        .conns
        .lock()
        .get("test")
        .and_then(|c| c.shared.clone())
        .unwrap();

    entry.update_tools(vec![DiscoveredTool {
        raw_name: "fresh".into(),
        qualified: "test__fresh".into(),
        description: "a new tool".into(),
        schema: json!({}),
    }]);
    for mgr in [&a, &b] {
        let tools = mgr.tool_lines("test");
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].0, "test__fresh");
    }
}

#[tokio::test]
async fn shutdown_of_one_session_leaves_the_connection_to_the_others() {
    let rig = rig(0, false);
    let cfg = config(vec![http("test")]);
    let (a, b) = (manager(&rig, &cfg), manager(&rig, &cfg));
    tokio::join!(a.connect_active(), b.connect_active());
    a.shutdown().await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(
        echo(&b, "alive").await.ok,
        "b's connection survived a's shutdown"
    );
}
