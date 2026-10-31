//! The control toolset through the session: advertised only with a host, gated by the
//! permission broker, refused when switched off (docs/features/control-tools.md).

use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

struct FakeHost {
    calls: AtomicUsize,
    last_me: Mutex<String>,
    off: Vec<String>,
}

#[async_trait::async_trait]
impl crate::control::ControlHost for FakeHost {
    fn disabled(&self) -> Vec<String> {
        self.off.clone()
    }
    async fn call(
        &self,
        me: &str,
        tool: &str,
        _args: &serde_json::Value,
    ) -> Result<String, String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        *self.last_me.lock().unwrap() = me.to_string();
        Ok(format!("ran {tool}"))
    }
}

fn session_with(
    dir: &std::path::Path,
    mode: forge_types::PermissionMode,
    host: Option<Arc<FakeHost>>,
) -> Session {
    let config = Config {
        permission_mode: mode,
        ..Config::default()
    };
    let mut session = Session::start(
        Arc::new(Store::open_in_memory().unwrap()),
        Arc::new(MockProvider),
        Arc::new(HeuristicRouter::new(config.clone())),
        ToolRegistry::with_core_tools_in(dir),
        Box::new(CapturePresenter::default()),
        config,
        dir.to_str().unwrap(),
    )
    .unwrap();
    if let Some(host) = host {
        session.set_control_host(Some(host));
    }
    session
}

fn host(off: &[&str]) -> Arc<FakeHost> {
    Arc::new(FakeHost {
        calls: AtomicUsize::new(0),
        last_me: Mutex::new(String::new()),
        off: off.iter().map(|s| s.to_string()).collect(),
    })
}

fn call(name: &str, args: serde_json::Value) -> forge_types::ToolCall {
    forge_types::ToolCall {
        id: format!("call-{name}"),
        name: name.into(),
        args,
    }
}

async fn run(session: &mut Session, c: &forge_types::ToolCall) -> String {
    let msg = session
        .store
        .add_message(session.id(), 0, Role::User, "go", None)
        .unwrap();
    session.invoke_tool(&msg, c).await.unwrap()
}

fn advertised(session: &Session) -> Vec<String> {
    session
        .full_tool_specs()
        .into_iter()
        .map(|s| s.name)
        .collect()
}

#[tokio::test]
async fn the_tools_are_advertised_only_with_a_host_and_minus_the_switched_off() {
    let dir = tempfile::tempdir().unwrap();
    let bare = session_with(dir.path(), forge_types::PermissionMode::Default, None);
    assert!(!advertised(&bare).iter().any(|n| n == "create_session"));
    let wired = session_with(
        dir.path(),
        forge_types::PermissionMode::Default,
        Some(host(&["merge_worktree"])),
    );
    let names = advertised(&wired);
    assert!(names.iter().any(|n| n == "create_session"));
    assert!(names.iter().any(|n| n == "read_terminal"));
    assert!(!names.iter().any(|n| n == "merge_worktree"));
}

#[tokio::test]
async fn reads_run_unasked_but_acting_tools_wait_for_the_permission_gate() {
    let dir = tempfile::tempdir().unwrap();
    let h = host(&[]);
    let mut session = session_with(
        dir.path(),
        forge_types::PermissionMode::Default,
        Some(h.clone()),
    );
    let read = run(&mut session, &call("get_usage", serde_json::json!({}))).await;
    assert_eq!(read, "ran get_usage");
    let denied = run(
        &mut session,
        &call("create_session", serde_json::json!({"prompt": "x"})),
    )
    .await;
    assert!(denied.contains("permission denied"), "{denied}");
    assert_eq!(h.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn plan_mode_refuses_acting_tools_and_bypass_runs_them_as_the_calling_session() {
    let dir = tempfile::tempdir().unwrap();
    let h = host(&[]);
    let mut plan = session_with(
        dir.path(),
        forge_types::PermissionMode::Plan,
        Some(h.clone()),
    );
    let refused = run(
        &mut plan,
        &call(
            "add_schedule",
            serde_json::json!({"task": "t", "every": "1h"}),
        ),
    )
    .await;
    assert!(refused.contains("permission denied by policy"), "{refused}");
    assert_eq!(h.calls.load(Ordering::SeqCst), 0);

    let mut open = session_with(
        dir.path(),
        forge_types::PermissionMode::Bypass,
        Some(h.clone()),
    );
    let ok = run(
        &mut open,
        &call(
            "add_schedule",
            serde_json::json!({"task": "t", "every": "1h"}),
        ),
    )
    .await;
    assert_eq!(ok, "ran add_schedule");
    assert_eq!(*h.last_me.lock().unwrap(), open.id());
}

#[tokio::test]
async fn a_missing_argument_and_a_switched_off_tool_never_reach_the_host() {
    let dir = tempfile::tempdir().unwrap();
    let h = host(&["notify_user"]);
    let mut session = session_with(
        dir.path(),
        forge_types::PermissionMode::Bypass,
        Some(h.clone()),
    );
    let missing = run(&mut session, &call("read_session", serde_json::json!({}))).await;
    assert!(missing.contains("needs `session`"), "{missing}");
    let off = run(
        &mut session,
        &call("notify_user", serde_json::json!({"message": "hi"})),
    )
    .await;
    assert!(off.contains("switched off"), "{off}");
    assert_eq!(h.calls.load(Ordering::SeqCst), 0);
}
