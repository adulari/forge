//! `manage_heartbeats` on the CLI bridge: agent-created heartbeats on the parent session.

use rmcp::model::{CallToolResult, ContentBlock};
use serde_json::Value;

use super::{report_to_sink, ForgeMcp};

impl ForgeMcp {
    /// Handle `manage_heartbeats` — create/list/pause/resume/delete an agent-created heartbeat on
    /// the parent session (parity with the direct path's `Session::manage_heartbeats`, which this
    /// mirrors action-for-action). No parent session id (bridge run standalone) → a clear error
    /// instead of silently no-oping.
    pub(super) fn handle_manage_heartbeats(&self, args: &Value) -> CallToolResult {
        let Ok(session_id) = std::env::var(forge_core::snapshot::ENV_SESSION) else {
            return CallToolResult::error(vec![ContentBlock::text(
                "manage_heartbeats unavailable: no parent session",
            )]);
        };
        let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("");
        let label = args.get("label").and_then(|v| v.as_str());
        let now = chrono::Utc::now().timestamp();

        let (text, ok) = match action {
            "create" => {
                let label = label.unwrap_or("");
                let prompt = args
                    .get("prompt")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim();
                let interval = args.get("interval").and_then(|v| v.as_str()).unwrap_or("");
                if label.trim().is_empty() {
                    ("error: `label` is required".to_string(), false)
                } else if prompt.is_empty() {
                    ("error: `prompt` is required".to_string(), false)
                } else {
                    match forge_core::heartbeat::parse_heartbeat_interval(interval) {
                        Err(e) => (format!("error: {e}"), false),
                        Ok(secs) => {
                            let existing =
                                self.tasks_store.list_heartbeats(&session_id).unwrap_or_default();
                            let agent_count =
                                existing.iter().filter(|h| h.owner == "agent").count();
                            if agent_count >= forge_core::heartbeat::MAX_AGENT_HEARTBEATS_PER_SESSION
                            {
                                (
                                    format!(
                                        "error: at most {} agent heartbeats per session — \
                                         delete one first",
                                        forge_core::heartbeat::MAX_AGENT_HEARTBEATS_PER_SESSION
                                    ),
                                    false,
                                )
                            } else if existing
                                .iter()
                                .any(|h| h.owner == "agent" && h.label.as_deref() == Some(label))
                            {
                                (
                                    format!("error: an agent heartbeat named '{label}' already exists"),
                                    false,
                                )
                            } else {
                                match self.tasks_store.add_agent_heartbeat(
                                    &forge_types::new_id(),
                                    &session_id,
                                    label,
                                    prompt,
                                    secs,
                                    now,
                                ) {
                                    Ok(()) => (
                                        format!(
                                            "heartbeat '{label}' created — every {}",
                                            forge_core::heartbeat::format_heartbeat_interval(secs)
                                        ),
                                        true,
                                    ),
                                    Err(e) => (format!("error: {e}"), false),
                                }
                            }
                        }
                    }
                }
            }
            "list" => {
                let all = self
                    .tasks_store
                    .list_heartbeats(&session_id)
                    .unwrap_or_default();
                let agent_only: Vec<_> = all.into_iter().filter(|h| h.owner == "agent").collect();
                (forge_core::heartbeat::format_heartbeat_list(&agent_only), true)
            }
            "pause" | "resume" => match label {
                None => ("error: `label` is required".to_string(), false),
                Some(label) => {
                    let existing =
                        self.tasks_store.list_heartbeats(&session_id).unwrap_or_default();
                    match existing
                        .iter()
                        .find(|h| h.owner == "agent" && h.label.as_deref() == Some(label))
                    {
                        None => (format!("no agent heartbeat named '{label}'"), false),
                        Some(hb) => {
                            let status = if action == "resume" { "active" } else { "paused" };
                            match self.tasks_store.set_heartbeat_status(&hb.id, status, now) {
                                Ok(true) => (format!("heartbeat '{label}' {status}"), true),
                                Ok(false) => (format!("no agent heartbeat named '{label}'"), false),
                                Err(e) => (format!("error: {e}"), false),
                            }
                        }
                    }
                }
            },
            "delete" => match label {
                None => ("error: `label` is required".to_string(), false),
                Some(label) => {
                    match self
                        .tasks_store
                        .delete_agent_heartbeat_by_label(&session_id, label)
                    {
                        Ok(true) => (format!("heartbeat '{label}' deleted"), true),
                        Ok(false) => (format!("no agent heartbeat named '{label}'"), false),
                        Err(e) => (format!("error: {e}"), false),
                    }
                }
            },
            other => (
                format!(
                    "error: unknown action '{other}' — expected create, list, pause, resume, or delete"
                ),
                false,
            ),
        };
        report_to_sink(serde_json::json!({ "k": "heartbeat", "action": action, "text": text }));
        let content = vec![ContentBlock::text(text)];
        if ok {
            CallToolResult::success(content)
        } else {
            CallToolResult::error(content)
        }
    }
}
