//! `wait_for_subagents` and `steer_subagent`: blocking on, and redirecting, detached children.
//!
//! `spawn_agents(detached: true)` returns at once and delivers results at the next turn boundary.
//! These two close the loop inside a turn: wait until the first (`any`) or every (`all`) chosen
//! child finishes, and redirect a child that is heading the wrong way without losing its context.

use std::time::{Duration, Instant};

use forge_store::{DetachedChild, DetachedChildStatus};

use super::*;

pub const WAIT_FOR_SUBAGENTS_TOOL: &str = "wait_for_subagents";
pub const STEER_SUBAGENT_TOOL: &str = "steer_subagent";

const DEFAULT_WAIT_SECS: u64 = 300;
const MAX_WAIT_SECS: u64 = 1800;
const POLL: Duration = Duration::from_millis(400);
/// Characters of each child's result returned when the model gives no `after_chars`.
const DEFAULT_RESULT_CHARS: usize = 4000;

pub fn wait_for_subagents_spec() -> ToolSpec {
    ToolSpec {
        name: WAIT_FOR_SUBAGENTS_TOOL.to_string(),
        description: "Block until detached agents finish. `mode: \"any\"` returns as soon as the \
             first of the chosen agents finishes (the rest keep running; wait again for them); \
             `mode: \"all\"` (default) returns when every chosen agent has finished. Choose \
             agents by name or id prefix in `agents`, or omit it to wait on every detached agent \
             still running. Returns each finished agent's result, capped at `after_chars` \
             characters (default 4000), and lists those still running. Gives up after \
             `timeout_secs` (default 300, max 1800) and says who is still running."
            .to_string(),
        schema: serde_json::json!({
            "type": "object",
            "properties": {
                "agents": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "agent names or session-id prefixes; omit for all running detached agents"
                },
                "mode": {"type": "string", "enum": ["any", "all"], "description": "any = first finisher, all = every one (default)"},
                "after_chars": {"type": "integer", "minimum": 200, "description": "cap on the characters returned per finished agent"},
                "timeout_secs": {"type": "integer", "minimum": 1, "maximum": MAX_WAIT_SECS}
            }
        }),
    }
}

pub fn steer_subagent_spec() -> ToolSpec {
    ToolSpec {
        name: STEER_SUBAGENT_TOOL.to_string(),
        description: "Redirect a detached agent that is running (or a finished one) without \
             starting over: its current turn is stopped, its whole conversation is kept, and it \
             continues with your `message` as the newest instruction. Blocks until it answers \
             and returns that answer. Use it when a running agent is on the wrong track; use \
             `send_to_agent` to follow up on one that already finished."
            .to_string(),
        schema: serde_json::json!({
            "type": "object",
            "properties": {
                "agent": {"type": "string", "description": "the child's agent name, or its session-id prefix"},
                "message": {"type": "string", "description": "the new instruction"}
            },
            "required": ["agent", "message"]
        }),
    }
}

/// The detached children a wait covers: those named in `agents`, or every running one plus any
/// that finished but whose result has not been handed to the model yet (`undelivered`), so a
/// child that ended a moment before the wait still counts as the first finisher.
fn wait_targets<'a>(
    children: &'a [DetachedChild],
    wanted: &[String],
    undelivered: &[String],
) -> Result<Vec<&'a DetachedChild>, String> {
    if wanted.is_empty() {
        return Ok(children
            .iter()
            .filter(|c| {
                c.status == DetachedChildStatus::Running || undelivered.contains(&c.child_id)
            })
            .collect());
    }
    let mut out: Vec<&DetachedChild> = Vec::new();
    for address in wanted {
        let Some(child) = subagent::resolve_detached_address(children, address.trim()) else {
            let known: Vec<String> = children.iter().map(|c| c.name.clone()).collect();
            return Err(format!(
                "error: no detached agent matches '{address}'. Detached agents this session: [{}]",
                known.join(", ")
            ));
        };
        if !out.iter().any(|c| c.child_id == child.child_id) {
            out.push(child);
        }
    }
    Ok(out)
}

fn short(id: &str) -> &str {
    &id[..id.len().min(8)]
}

fn clip(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let kept: String = text.chars().take(max_chars).collect();
    format!(
        "{kept}\n[... cut at {max_chars} characters; the full result is in the agent's session]"
    )
}

/// What a wait returns once it settles: finished results first, then who is still running.
pub(crate) fn format_wait(
    targets: &[DetachedChild],
    max_chars: usize,
    timed_out_after: Option<u64>,
) -> String {
    let mut out = String::new();
    for c in targets.iter().filter(|c| c.status.is_finished()) {
        out.push_str(&format!(
            "[{} ({}) {}]\n{}\n\n",
            c.name,
            short(&c.child_id),
            c.status.as_str(),
            clip(
                c.result_ref.as_deref().unwrap_or("(no result recorded)"),
                max_chars
            )
        ));
    }
    let running: Vec<String> = targets
        .iter()
        .filter(|c| !c.status.is_finished())
        .map(|c| format!("{} ({})", c.name, short(&c.child_id)))
        .collect();
    if let Some(secs) = timed_out_after {
        out.push_str(&format!("timed out after {secs}s. "));
    }
    if running.is_empty() {
        out.push_str("No chosen agent is still running.");
    } else {
        out.push_str(&format!("Still running: {}.", running.join(", ")));
    }
    out
}

impl Session {
    pub(crate) async fn wait_for_subagents(
        &mut self,
        msg_id: &str,
        call: &forge_types::ToolCall,
    ) -> Result<String, CoreError> {
        let args_json = serde_json::to_string(&call.args)?;
        let wanted: Vec<String> = call
            .args
            .get("agents")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        let any = call.args.get("mode").and_then(|v| v.as_str()) == Some("any");
        let max_chars = call
            .args
            .get("after_chars")
            .and_then(|v| v.as_u64())
            .map_or(DEFAULT_RESULT_CHARS, |n| (n as usize).max(200));
        let timeout = call
            .args
            .get("timeout_secs")
            .and_then(|v| v.as_u64())
            .unwrap_or(DEFAULT_WAIT_SECS)
            .clamp(1, MAX_WAIT_SECS);

        let children = self.store.list_detached_children(&self.id)?;
        let undelivered: Vec<String> = self
            .store
            .undelivered_detached_children(&self.id)?
            .into_iter()
            .map(|c| c.child_id)
            .collect();
        let ids: Vec<String> = match wait_targets(&children, &wanted, &undelivered) {
            Ok(targets) if targets.is_empty() => {
                let result = "no detached agent is running - nothing to wait for. Finished ones \
                              are delivered automatically; list_subagents shows them."
                    .to_string();
                self.store
                    .record_tool_call(msg_id, &call.name, &args_json, &result, "allowed", "ok")?;
                return Ok(result);
            }
            Ok(targets) => targets.iter().map(|c| c.child_id.clone()).collect(),
            Err(message) => {
                self.store.record_tool_call(
                    msg_id, &call.name, &args_json, &message, "allowed", "error",
                )?;
                return Ok(message);
            }
        };

        // Finished and already handed over before this wait began: not a new finisher, so a
        // `mode: "any"` loop over the same list blocks for the rest instead of re-reading them.
        let already: Vec<String> = children
            .iter()
            .filter(|c| {
                ids.contains(&c.child_id)
                    && c.status.is_finished()
                    && !undelivered.contains(&c.child_id)
            })
            .map(|c| c.child_id.clone())
            .collect();
        let started = Instant::now();
        let (snapshot, timed_out) = loop {
            let now = self.store.list_detached_children(&self.id)?;
            let snapshot: Vec<DetachedChild> = now
                .into_iter()
                .filter(|c| ids.contains(&c.child_id))
                .collect();
            let finished = snapshot.iter().filter(|c| c.status.is_finished()).count();
            let fresh = snapshot
                .iter()
                .filter(|c| c.status.is_finished() && !already.contains(&c.child_id))
                .count();
            let done = if any {
                fresh > 0 || finished == snapshot.len()
            } else {
                finished == snapshot.len()
            };
            if done {
                break (snapshot, false);
            }
            if started.elapsed() >= Duration::from_secs(timeout) {
                break (snapshot, true);
            }
            tokio::time::sleep(POLL).await;
        };

        // What the wait hands back is delivered now, so the turn-boundary message must not
        // repeat it.
        for c in snapshot.iter().filter(|c| c.status.is_finished()) {
            self.store.mark_detached_delivered(&c.child_id)?;
        }
        let shown: Vec<DetachedChild> = if any && snapshot.iter().any(|c| !c.status.is_finished()) {
            snapshot
                .into_iter()
                .filter(|c| !already.contains(&c.child_id))
                .collect()
        } else {
            snapshot
        };
        let result = format_wait(&shown, max_chars, timed_out.then_some(timeout));
        self.store
            .record_tool_call(msg_id, &call.name, &args_json, &result, "allowed", "ok")?;
        Ok(result)
    }

    pub(crate) async fn steer_subagent(
        &mut self,
        msg_id: &str,
        call: &forge_types::ToolCall,
    ) -> Result<String, CoreError> {
        let args_json = serde_json::to_string(&call.args)?;
        let text = |key: &str| {
            call.args
                .get(key)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_string()
        };
        let (address, message) = (text("agent"), text("message"));
        let fail = |result: String, store: &Store| -> Result<String, CoreError> {
            store.record_tool_call(msg_id, &call.name, &args_json, &result, "allowed", "error")?;
            Ok(result)
        };
        if address.is_empty() || message.is_empty() {
            return fail(
                "error: steer_subagent needs both `agent` (name or id prefix) and `message`".into(),
                &self.store,
            );
        }
        let detached = self.store.list_detached_children(&self.id)?;
        let Some(child) = subagent::resolve_detached_address(&detached, &address) else {
            return fail(
                format!(
                    "error: no detached agent matches '{address}'. Detached agents this session: [{}]",
                    detached.iter().map(|c| c.name.clone()).collect::<Vec<_>>().join(", ")
                ),
                &self.store,
            );
        };
        let (child_id, name) = (child.child_id.clone(), child.name.clone());
        if child.status == DetachedChildStatus::Running {
            // Stop the live turn; the child's session keeps every message it already persisted.
            self.detached_registry.abort_and_wait(&child_id).await;
            self.store.cancel_detached_child(&child_id)?;
        }
        self.store.mark_detached_delivered(&child_id)?;
        let steer = format!(
            "New instruction from the parent agent (it replaces whatever you were doing; keep \
             what you already learned): {message}"
        );
        // A failed resume is the tool's answer, not a reason to end the parent's turn.
        let outcome = match self.follow_up_child(&child_id, &name, &steer, true).await {
            Ok(outcome) => outcome,
            Err(error) => {
                return fail(
                    format!("error: could not steer '{name}': {error}"),
                    &self.store,
                );
            }
        };
        let result = format!("[{name} ({})] {}", short(&child_id), outcome.final_text);
        self.store.record_tool_call(
            msg_id,
            &call.name,
            &args_json,
            &result,
            "allowed",
            if outcome.ok { "ok" } else { "error" },
        )?;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn child(
        name: &str,
        id: &str,
        status: DetachedChildStatus,
        result: Option<&str>,
    ) -> DetachedChild {
        DetachedChild {
            child_id: id.to_string(),
            parent_session: "p".into(),
            name: name.into(),
            model: "m".into(),
            status,
            result_ref: result.map(str::to_string),
            created_at: 0,
            finished_at: None,
        }
    }

    #[test]
    fn wait_targets_default_to_running_and_resolve_names() {
        let kids = vec![
            child("a", "aaaaaaaa11", DetachedChildStatus::Running, None),
            child("b", "bbbbbbbb22", DetachedChildStatus::Done, Some("ok")),
        ];
        let all = wait_targets(&kids, &[], &[]).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].name, "a");
        let with_unread = wait_targets(&kids, &[], &["bbbbbbbb22".to_string()]).unwrap();
        assert_eq!(with_unread.len(), 2, "an undelivered finisher is a target");
        let named = wait_targets(&kids, &["b".into(), "bbbb".into()], &[]).unwrap();
        assert_eq!(named.len(), 1, "a name and its own id prefix are one agent");
        assert!(wait_targets(&kids, &["zzz".into()], &[])
            .unwrap_err()
            .contains("no detached agent"));
    }

    #[test]
    fn wait_report_lists_finished_results_then_stragglers() {
        let kids = vec![
            child(
                "fast",
                "ffffffff00",
                DetachedChildStatus::Done,
                Some(&"x".repeat(500)),
            ),
            child("slow", "ssssssss00", DetachedChildStatus::Running, None),
        ];
        let text = format_wait(&kids, 200, None);
        assert!(text.starts_with("[fast (ffffffff) done]"));
        assert!(text.contains("cut at 200 characters"));
        assert!(text.ends_with("Still running: slow (ssssssss)."));
        assert!(format_wait(&kids, 200, Some(9)).contains("timed out after 9s"));
    }
}
