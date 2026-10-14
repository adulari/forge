//! The lean direct surface and `tool_call` unwrapping (docs/features/deferred-tools.md).

use super::*;
use crate::completion::{VerificationLedger, VerificationObservation};
use crate::deferred_tools::{TOOL_CALL, TOOL_SEARCH};
use std::sync::atomic::{AtomicU64, Ordering};

const NOTEBOOK: &str = r#"{"cells":[{"cell_type":"code","source":["print(1)"],"metadata":{},"outputs":[],"execution_count":null}],"metadata":{},"nbformat":4,"nbformat_minor":5}"#;

fn session_in(dir: &std::path::Path, config: Config) -> (Session, Arc<Mutex<Vec<PresenterEvent>>>) {
    let capture = CapturePresenter::default();
    let events = capture.events.clone();
    let session = Session::start(
        Arc::new(Store::open_in_memory().unwrap()),
        Arc::new(MockProvider),
        Arc::new(HeuristicRouter::new(config.clone())),
        ToolRegistry::with_core_tools_in(dir),
        Box::new(capture),
        config,
        dir.to_str().unwrap(),
    )
    .unwrap();
    (session, events)
}

fn mode_config(mode: forge_types::PermissionMode) -> Config {
    Config {
        permission_mode: mode,
        ..Config::default()
    }
}

struct StepOutcome {
    mutations: u64,
    ledger: VerificationLedger,
    started: Vec<String>,
}

/// Run one model step holding `tool_call{notebook_edit}`, after a successful check, and report
/// what the loop's counters saw.
async fn run_wrapped_notebook_edit(
    session: &mut Session,
    events: &Arc<Mutex<Vec<PresenterEvent>>>,
    path: &str,
) -> StepOutcome {
    let msg_id = session
        .store
        .add_message(session.id(), 0, Role::User, "edit the notebook", None)
        .unwrap();
    let call = forge_types::ToolCall {
        id: "call-1".into(),
        name: TOOL_CALL.into(),
        args: serde_json::json!({
            "name": "notebook_edit",
            "arguments": {"path": path, "cell": 0, "source": "print(2)"}
        }),
    };
    let mutations = Arc::new(AtomicU64::new(0));
    let ledger = Arc::new(Mutex::new(VerificationLedger::default()));
    ledger
        .lock()
        .unwrap()
        .observe(VerificationObservation::Generic, true);
    let halted = session
        .execute_model_tool_step(
            &msg_id,
            &[call],
            &Arc::new(AtomicU64::new(0)),
            &Arc::new(AtomicU64::new(0)),
            &mutations,
            &ledger,
            &mut None,
            &mut 0,
            &mut std::collections::VecDeque::new(),
            &mut false,
            &mut std::collections::HashMap::new(),
            &mut false,
            &mut Vec::new(),
        )
        .await
        .unwrap();
    assert!(!halted);
    let started = events
        .lock()
        .unwrap()
        .iter()
        .filter_map(|e| match e {
            PresenterEvent::ToolStart { name, .. } => Some(name.clone()),
            _ => None,
        })
        .collect();
    let ledger = std::mem::take(&mut *ledger.lock().unwrap());
    StepOutcome {
        mutations: mutations.load(Ordering::Relaxed),
        ledger,
        started,
    }
}

#[tokio::test]
async fn deferred_write_through_tool_call_is_denied_in_plan_mode() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nb.ipynb");
    std::fs::write(&path, NOTEBOOK).unwrap();
    let (mut session, events) =
        session_in(dir.path(), mode_config(forge_types::PermissionMode::Plan));

    let out = run_wrapped_notebook_edit(&mut session, &events, path.to_str().unwrap()).await;

    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        NOTEBOOK,
        "plan mode wrote the notebook"
    );
    assert_eq!(out.mutations, 0);
    let result = session
        .transcript
        .iter()
        .rev()
        .find(|m| m.role == Role::Tool)
        .unwrap();
    assert_eq!(
        result.tool_call_id.as_deref(),
        Some("call-1"),
        "result pairs with the outer call id"
    );
    assert!(result.content.contains("denied"), "{}", result.content);
}

#[tokio::test]
async fn deferred_write_through_tool_call_counts_as_a_mutation() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nb.ipynb");
    std::fs::write(&path, NOTEBOOK).unwrap();
    let (mut session, events) =
        session_in(dir.path(), mode_config(forge_types::PermissionMode::Bypass));

    let out = run_wrapped_notebook_edit(&mut session, &events, path.to_str().unwrap()).await;

    assert!(std::fs::read_to_string(&path).unwrap().contains("print(2)"));
    assert_eq!(
        out.mutations, 1,
        "phantom-edit gate must see the inner write"
    );
    assert!(
        !out.ledger.verified_since(0),
        "the inner write must make the earlier check stale in the verification ledger"
    );
    assert_eq!(
        out.started,
        ["notebook_edit"],
        "dispatch and presenter see the inner tool"
    );
}

#[test]
fn lean_surface_advertises_core_plus_the_meta_pair_and_has_a_kill_switch() {
    let dir = tempfile::tempdir().unwrap();
    let (session, _) = session_in(dir.path(), Config::default());
    let lean: Vec<String> = session.tool_specs().into_iter().map(|s| s.name).collect();
    for name in [
        "read_file",
        "shell",
        "ask_user",
        "update_tasks",
        TOOL_SEARCH,
        TOOL_CALL,
    ] {
        assert!(
            lean.iter().any(|n| n == name),
            "{name} missing from {lean:?}"
        );
    }
    for name in [
        "notebook_edit",
        "device",
        "browser",
        "remember",
        "push_notification",
    ] {
        assert!(
            !lean.iter().any(|n| n == name),
            "{name} should be deferred: {lean:?}"
        );
    }
    assert!(lean.windows(2).all(|pair| pair[0] <= pair[1]), "{lean:?}");
    let search = session
        .tool_specs()
        .into_iter()
        .find(|s| s.name == TOOL_SEARCH)
        .unwrap();
    assert!(search
        .description
        .contains("notebook_edit (Edit a Jupyter notebook"));

    let mut off = Config::default();
    off.mesh.direct_lean = false;
    let (session, _) = session_in(dir.path(), off);
    let full: Vec<String> = session.tool_specs().into_iter().map(|s| s.name).collect();
    assert!(full.iter().any(|n| n == "notebook_edit"));
    assert!(!full.iter().any(|n| n == TOOL_SEARCH));

    let mut promoted = Config::default();
    promoted.mesh.lean_core_extra = vec!["web_fetch".into()];
    let (session, _) = session_in(dir.path(), promoted);
    assert!(session.tool_specs().iter().any(|s| s.name == "web_fetch"));
}

#[tokio::test]
async fn tool_search_returns_the_schema_and_a_runnable_example() {
    let dir = tempfile::tempdir().unwrap();
    let (mut session, _) = session_in(dir.path(), Config::default());
    let msg_id = session
        .store
        .add_message(session.id(), 0, Role::User, "find it", None)
        .unwrap();
    let call = forge_types::ToolCall {
        id: "s1".into(),
        name: TOOL_SEARCH.into(),
        args: serde_json::json!({"query": "notebook_edit"}),
    };
    let out = session.invoke_tool(&msg_id, &call).await.unwrap();
    assert!(out.starts_with("## notebook_edit"), "{out}");
    assert!(
        out.contains(
            r#"example: tool_call{"arguments":{"cell":0,"path":"…"},"name":"notebook_edit"}"#
        ),
        "{out}"
    );

    let bad = forge_types::ToolCall {
        id: "s2".into(),
        name: TOOL_CALL.into(),
        args: serde_json::json!({"name": TOOL_SEARCH}),
    };
    let out = session.invoke_tool(&msg_id, &bad).await.unwrap();
    assert!(out.contains("needs the exact name"), "{out}");
}

#[test]
fn wrapped_read_only_tool_still_batches() {
    let dir = tempfile::tempdir().unwrap();
    let (session, _) = session_in(dir.path(), Config::default());
    let wrapped = forge_types::ToolCall {
        id: "w".into(),
        name: TOOL_CALL.into(),
        args: serde_json::json!({"name": "list_dir", "arguments": {"path": "."}}),
    };
    assert!(!session.is_concurrent_readonly(&wrapped));
    assert!(session.is_concurrent_readonly(&session.resolve_deferred_call(&wrapped)));
}

#[tokio::test]
async fn tool_call_reaches_external_mcp_tools_and_tool_search_finds_them() {
    let mcp = forge_config::McpConfig {
        allow: forge_config::McpAllowlist {
            servers: vec!["test".into()],
            tools: vec!["test__echo".into()],
        },
        ..Default::default()
    };
    let config = Config {
        mcp: mcp.clone(),
        ..mode_config(forge_types::PermissionMode::Bypass)
    };
    let dir = tempfile::tempdir().unwrap();
    let (mut session, _) = session_in(dir.path(), config);
    session.set_mcp(Some(Arc::new(
        forge_mcp::testsupport::manager_with_echo(&mcp).await,
    )));
    let msg_id = session
        .store
        .add_message(session.id(), 0, Role::User, "echo", None)
        .unwrap();

    let found = session
        .invoke_tool(
            &msg_id,
            &forge_types::ToolCall {
                id: "s".into(),
                name: TOOL_SEARCH.into(),
                args: serde_json::json!({"query": "echo"}),
            },
        )
        .await
        .unwrap();
    assert!(
        found.contains("MCP server tools") && found.contains("test__echo"),
        "{found}"
    );

    let wrapped = forge_types::ToolCall {
        id: "c".into(),
        name: TOOL_CALL.into(),
        args: serde_json::json!({"name": "test__echo", "arguments": {"msg": "hi"}}),
    };
    let resolved = session.resolve_deferred_call(&wrapped);
    assert_eq!(resolved.name, forge_mcp::MCP_CALL);
    assert_eq!(resolved.id, "c");
    assert_eq!(
        session.invoke_tool(&msg_id, &wrapped).await.unwrap(),
        "echo: hi"
    );
}
