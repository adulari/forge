//! Bridge-side `message_session`: talks to a running `forge serve` daemon over HTTP exactly like
//! `forge send` (`crate::attach`'s discovery/auth, `forge_core::fleet::resolve_target` for
//! id/name resolution). A CLI-bridge process (claude/codex running `forge mcp-serve`) is never
//! the same process as the daemon and so has no direct registry access — only `forge serve`'s own
//! in-process driver gets the [`forge_core::fleet::FleetMessaging`] implementation (see
//! `cli::commands::run::driver::daemon_fleet`); this is the "bridge-shaped half" `subagents.rs`'s
//! module doc describes, reproducing the same semantics with no presenter and no registry.

use forge_core::fleet::{
    describe_peers, message_session_spec, resolve_target, FleetPeer, MessageMode,
};
use rmcp::model::{CallToolResult, ContentBlock, JsonObject, Tool};
use serde_json::Value;
use std::sync::Arc;

use super::ForgeMcp;

impl ForgeMcp {
    pub(super) fn message_session_tool(&self) -> Tool {
        let spec = message_session_spec();
        let schema: JsonObject = spec.schema.as_object().cloned().unwrap_or_default();
        Tool::new(spec.name, spec.description, Arc::new(schema))
    }

    pub(super) async fn handle_message_session(&self, args: &Value) -> CallToolResult {
        let target = args
            .get("target")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let message = args
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let mode_raw = args
            .get("mode")
            .and_then(|v| v.as_str())
            .unwrap_or("follow_up");
        let Some(mode) = MessageMode::parse(mode_raw) else {
            return CallToolResult::error(vec![ContentBlock::text(format!(
                "error: unknown mode '{mode_raw}' — use follow_up or steer"
            ))]);
        };
        if target.is_empty() || message.is_empty() {
            return CallToolResult::error(vec![ContentBlock::text(
                "error: message_session needs both `target` and `message`",
            )]);
        }
        if message.len() > 16 * 1024 {
            return CallToolResult::error(vec![ContentBlock::text(format!(
                "error: message is {} bytes, exceeds the 16KB limit",
                message.len()
            ))]);
        }

        let base = crate::attach::resolve_base_url(None);
        let token = match crate::attach::resolve_token(None) {
            Ok(t) => t,
            Err(e) => {
                return CallToolResult::error(vec![ContentBlock::text(format!("error: {e}"))])
            }
        };
        let http = reqwest::Client::new();
        let sessions = match crate::attach::fetch_sessions(&http, &base, &token).await {
            Ok(s) => s,
            Err(e) => {
                return CallToolResult::error(vec![ContentBlock::text(format!("error: {e}"))])
            }
        };
        let peers: Vec<FleetPeer> = sessions
            .iter()
            .map(|s| FleetPeer {
                id: s.id.clone(),
                title: (!s.title.is_empty()).then(|| s.title.clone()),
            })
            .collect();
        let peer = match resolve_target(&peers, &target) {
            Ok(p) => p,
            Err(e) => {
                return CallToolResult::error(vec![ContentBlock::text(format!(
                    "error: {e} for '{target}'. Live fleet sessions: [{}]",
                    describe_peers(&peers)
                ))]);
            }
        };
        let send_url = format!("{base}/{token}/api/sessions/{}/message", peer.id);
        let resp = http
            .post(&send_url)
            .json(&serde_json::json!({
                "text": message,
                "mode": mode.as_str(),
                "sender_kind": "cli",
                "sender_label": "cli-bridge",
            }))
            .send()
            .await;
        match resp {
            Ok(r) if r.status().is_success() => {
                CallToolResult::success(vec![ContentBlock::text(format!(
                    "message sent to {} ({}) [{}]",
                    peer.title.as_deref().unwrap_or("unnamed"),
                    &peer.id[..peer.id.len().min(8)],
                    mode.as_str()
                ))])
            }
            Ok(r) => {
                let status = r.status();
                let body = r.text().await.unwrap_or_default();
                CallToolResult::error(vec![ContentBlock::text(format!(
                    "error: daemon rejected the message ({status}): {body}"
                ))])
            }
            Err(e) => CallToolResult::error(vec![ContentBlock::text(format!(
                "error: could not reach the forge serve daemon at {base} — is it running? [{e}]"
            ))]),
        }
    }
}
