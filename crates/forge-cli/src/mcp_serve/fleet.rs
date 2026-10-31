//! Bridge-side `list_sessions` / `message_session`: talks to a running `forge serve` daemon over
//! HTTP (`crate::attach`'s discovery/auth), because a CLI-bridge process (claude/codex running
//! `forge mcp-serve`) is never the same process as the daemon and has no registry access. The
//! daemon does the resolving, guarding and delivery (`crate::fleet_peers`), so the bridge and
//! the direct path cannot drift; this side only adds the sender identity from the parent session
//! id (`ENV_SESSION`) and shares `forge_core::fleet::{run_list, run_send}` for validation,
//! the Plan-mode gate and result formatting.

use forge_core::fleet::{list_sessions_spec, message_session_spec};
use rmcp::model::{CallToolResult, ContentBlock, JsonObject, Tool};
use serde_json::Value;
use std::sync::Arc;

#[path = "control.rs"]
mod control;

use super::ForgeMcp;
use crate::fleet_peers::HttpFleet;

/// The parent session this bridge child works for (empty when unknown).
fn me() -> String {
    std::env::var(forge_core::snapshot::ENV_SESSION).unwrap_or_default()
}

fn tool_of(spec: forge_provider::ToolSpec) -> Tool {
    let schema: JsonObject = spec.schema.as_object().cloned().unwrap_or_default();
    Tool::new(spec.name, spec.description, Arc::new(schema))
}

fn result(text: String, ok: bool) -> CallToolResult {
    let content = vec![ContentBlock::text(text)];
    if ok {
        CallToolResult::success(content)
    } else {
        CallToolResult::error(content)
    }
}

impl ForgeMcp {
    pub(super) fn message_session_tool(&self) -> Tool {
        tool_of(message_session_spec())
    }

    /// `list_sessions` plus the control toolset, which shares its daemon transport.
    pub(super) fn list_sessions_tools(&self) -> Vec<Tool> {
        let mut tools = vec![tool_of(list_sessions_spec())];
        tools.extend(control::tools());
        tools
    }

    pub(super) fn is_fleet_read(name: &str) -> bool {
        name == forge_core::fleet::LIST_SESSIONS_TOOL || forge_core::control::is_control_tool(name)
    }

    pub(super) async fn handle_fleet_read(&self, name: &str, args: &Value) -> CallToolResult {
        if forge_core::control::is_control_tool(name) {
            self.handle_control(name, args).await
        } else {
            self.handle_list_sessions(args).await
        }
    }

    pub(super) async fn handle_message_session(&self, args: &Value) -> CallToolResult {
        let (text, ok) =
            forge_core::fleet::run_send(&HttpFleet::local(), &me(), args, self.effective_mode())
                .await;
        result(text, ok)
    }

    pub(super) async fn handle_list_sessions(&self, args: &Value) -> CallToolResult {
        result(
            forge_core::fleet::run_list(&HttpFleet::local(), &me(), args).await,
            true,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fleet_peers::tests::{scratch, spawn, wait_for_prompt};
    use crate::serve::tests::FORGE_DB_LOCK;
    use crate::serve::{daemon_router, DaemonState, SessionRegistry};
    use forge_types::PermissionMode;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_bridge_child_lists_and_messages_through_the_daemon_api() {
        let _env = FORGE_DB_LOCK.lock().await;
        let dir = scratch("bridge");
        std::env::set_var("FORGE_DB", dir.join("fleet.db"));
        let store = Arc::new(crate::open_store().unwrap());
        let registry = Arc::new(SessionRegistry::new());
        let parent = spawn(&registry, &store, "claude-parent").await;
        let peer = spawn(&registry, &store, "peer").await;
        let router = daemon_router(Arc::new(DaemonState {
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
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let fleet = HttpFleet::at(format!("http://127.0.0.1:{port}"), "tok".into());
        let me = parent.as_str();

        let listing = forge_core::fleet::run_list(&fleet, me, &serde_json::json!({})).await;
        assert!(listing.contains("claude-parent"), "{listing}");
        assert!(listing.contains("<- you"), "{listing}");

        let args = serde_json::json!({"target": "peer", "message": "hello from a bridge child"});
        let (text, ok) = forge_core::fleet::run_send(&fleet, me, &args, PermissionMode::Auto).await;
        assert!(ok, "{text}");
        let seen = wait_for_prompt(&store, &peer, "hello from a bridge child").await;
        assert!(
            seen.contains("\"claude-parent\""),
            "sender identity is the parent session: {seen}"
        );

        let (text, ok) = forge_core::fleet::run_send(&fleet, me, &args, PermissionMode::Plan).await;
        assert!(!ok && text.contains("plan"), "{text}");
        let own = serde_json::json!({"target": "claude-parent", "message": "me"});
        let (text, ok) = forge_core::fleet::run_send(&fleet, me, &own, PermissionMode::Auto).await;
        assert!(!ok && text.contains("own session"), "{text}");

        std::env::remove_var("FORGE_DB");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
