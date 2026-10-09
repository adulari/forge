//! `schedule_wakeup`: a self-paced, one-shot resume of the current session after a delay
//! (Claude Code's `ScheduleWakeup`). The model uses it to check back on something slow — a CI run,
//! a deploy, a long job — without a human re-prompting and without holding a turn open.
//!
//! It is a one-shot specialisation of the session-heartbeat machinery rather than a second
//! scheduler: the wakeup is an agent-owned heartbeat under the reserved [`WAKEUP_LABEL`], so it
//! reuses the heartbeat table (no migration), its atomic claim, its idle-only delivery, and its
//! reach through the CLI bridge (which shares the store with the parent session). Delivery
//! deletes the row, which is what makes it one-shot. Calling the tool again replaces the pending
//! wakeup, so a model that re-arms itself each turn never accumulates timers.
//!
//! Like every heartbeat it fires only while the session is open and idle (the TUI and the daemon
//! driver loop deliver it); it does not survive the session ending, and a one-shot `forge run`
//! that exits first never sees it.

use forge_store::Store;
use serde_json::Value;

use super::*;

pub const SCHEDULE_WAKEUP_TOOL: &str = "schedule_wakeup";
/// The reserved heartbeat label a wakeup is stored under.
pub const WAKEUP_LABEL: &str = "wakeup";
pub const MIN_WAKEUP_SECS: i64 = 60;
pub const MAX_WAKEUP_SECS: i64 = 3600;

pub fn schedule_wakeup_spec() -> ToolSpec {
    ToolSpec {
        name: SCHEDULE_WAKEUP_TOOL.to_string(),
        description: format!(
            "Schedule a one-shot wakeup: after `delay_seconds` ({MIN_WAKEUP_SECS}-{MAX_WAKEUP_SECS}, \
             clamped) this session is re-entered with `prompt` as an ordinary turn, once it is \
             idle. Use it to check back on something slow (CI, a deploy, a long job) instead of \
             polling in a loop. Calling it again replaces the pending wakeup. It only fires while \
             this session stays open."
        ),
        schema: serde_json::json!({
            "type": "object",
            "properties": {
                "delay_seconds": {
                    "type": "integer",
                    "description": format!("seconds until the wakeup, clamped to {MIN_WAKEUP_SECS}-{MAX_WAKEUP_SECS}")
                },
                "prompt": {
                    "type": "string",
                    "description": "what to do when the wakeup fires — it is submitted to you verbatim"
                },
                "reason": {
                    "type": "string",
                    "description": "optional: why you are waiting (shown to you when it fires)"
                }
            },
            "required": ["delay_seconds", "prompt"]
        }),
    }
}

/// Clamp a requested delay into the allowed window.
pub fn clamp_delay(secs: i64) -> i64 {
    secs.clamp(MIN_WAKEUP_SECS, MAX_WAKEUP_SECS)
}

/// The text submitted when the wakeup fires.
pub(crate) fn compose_prompt(prompt: &str, reason: &str) -> String {
    let reason = reason.trim();
    if reason.is_empty() {
        prompt.trim().to_string()
    } else {
        format!("{}\n\n(scheduled because: {reason})", prompt.trim())
    }
}

/// Arm (or re-arm) the session's wakeup. Shared by the direct path and the CLI-bridge handler,
/// which both hold a [`Store`] and a session id. Returns the tool-result text and success flag.
pub fn schedule(store: &Store, session_id: &str, args: &Value, now: i64) -> (String, bool) {
    let Some(requested) = args.get("delay_seconds").and_then(|v| {
        v.as_i64()
            .or_else(|| v.as_f64().map(|f| f.round() as i64))
            .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
    }) else {
        return (
            "error: `delay_seconds` is required (a number)".to_string(),
            false,
        );
    };
    let prompt = args.get("prompt").and_then(Value::as_str).unwrap_or("");
    if prompt.trim().is_empty() {
        return ("error: `prompt` is required".to_string(), false);
    }
    let reason = args.get("reason").and_then(Value::as_str).unwrap_or("");
    let delay = clamp_delay(requested);

    if let Err(e) = store.delete_agent_heartbeat_by_label(session_id, WAKEUP_LABEL) {
        return (format!("error: {e}"), false);
    }
    let existing_agents = match store.list_heartbeats(session_id) {
        Ok(all) => all.iter().filter(|h| h.owner == "agent").count(),
        Err(e) => return (format!("error: failed to list heartbeats: {e}"), false),
    };
    if existing_agents >= heartbeat::MAX_AGENT_HEARTBEATS_PER_SESSION {
        return (
            "error: no heartbeat slot free for a wakeup — delete an agent heartbeat first"
                .to_string(),
            false,
        );
    }
    match store.add_agent_heartbeat(
        &forge_types::new_id(),
        session_id,
        WAKEUP_LABEL,
        &compose_prompt(prompt, reason),
        delay,
        now,
    ) {
        Ok(()) => {
            let note = if delay != requested {
                format!(" (clamped from {requested}s)")
            } else {
                String::new()
            };
            (
                format!(
                    "wakeup scheduled in {}{note}; it fires when this session is idle",
                    heartbeat::format_heartbeat_interval(delay)
                ),
                true,
            )
        }
        Err(e) => (format!("error: {e}"), false),
    }
}

/// Bridge entry point: the parent session id arrives in the environment the parent exported to its
/// `forge mcp-serve` child, and the store is the one both processes share.
pub fn schedule_for_bridge(store: &Store, args: &Value) -> (String, bool) {
    let Ok(session_id) = std::env::var(snapshot::ENV_SESSION) else {
        return (
            "schedule_wakeup unavailable: no parent session".to_string(),
            false,
        );
    };
    schedule(store, &session_id, args, heartbeat::now_secs())
}

impl Session {
    /// Handle a `schedule_wakeup` call on the direct path.
    pub(crate) fn schedule_wakeup(
        &mut self,
        msg_id: &str,
        call: &forge_types::ToolCall,
    ) -> Result<String, CoreError> {
        let args_json = serde_json::to_string(&call.args)?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let (result, ok) = schedule(&self.store, &self.id, &call.args, now);
        self.store.record_tool_call(
            msg_id,
            &call.name,
            &args_json,
            &result,
            "allowed",
            if ok { "ok" } else { "error" },
        )?;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn setup() -> (Store, String) {
        let store = Store::open_in_memory().unwrap();
        let id = store.create_session("/tmp", "Default").unwrap();
        (store, id)
    }

    fn now() -> i64 {
        1_000_000
    }

    #[test]
    fn delay_is_clamped_to_the_window() {
        assert_eq!(clamp_delay(5), MIN_WAKEUP_SECS);
        assert_eq!(clamp_delay(600), 600);
        assert_eq!(clamp_delay(99_999), MAX_WAKEUP_SECS);
        assert_eq!(clamp_delay(-10), MIN_WAKEUP_SECS);
    }

    #[test]
    fn schedule_arms_a_clamped_agent_heartbeat() {
        let (store, sid) = setup();
        let (msg, ok) = schedule(
            &store,
            &sid,
            &json!({ "delay_seconds": 5, "prompt": "check CI", "reason": "build running" }),
            now(),
        );
        assert!(ok, "{msg}");
        assert!(msg.contains("clamped from 5s"), "{msg}");
        let hbs = store.list_heartbeats(&sid).unwrap();
        assert_eq!(hbs.len(), 1);
        assert_eq!(hbs[0].owner, "agent");
        assert_eq!(hbs[0].label.as_deref(), Some(WAKEUP_LABEL));
        assert_eq!(hbs[0].interval_secs, MIN_WAKEUP_SECS);
        assert!(hbs[0].prompt.contains("check CI"));
        assert!(hbs[0].prompt.contains("build running"));
    }

    #[test]
    fn rescheduling_replaces_instead_of_stacking() {
        let (store, sid) = setup();
        for p in ["first", "second"] {
            let (m, ok) = schedule(
                &store,
                &sid,
                &json!({ "delay_seconds": 120, "prompt": p }),
                now(),
            );
            assert!(ok, "{m}");
        }
        let hbs = store.list_heartbeats(&sid).unwrap();
        assert_eq!(hbs.len(), 1);
        assert_eq!(hbs[0].prompt, "second");
    }

    #[test]
    fn missing_arguments_are_errors() {
        let (store, sid) = setup();
        assert!(!schedule(&store, &sid, &json!({ "prompt": "x" }), now()).1);
        assert!(!schedule(&store, &sid, &json!({ "delay_seconds": 90 }), now()).1);
        assert!(
            !schedule(
                &store,
                &sid,
                &json!({ "delay_seconds": 90, "prompt": "  " }),
                now()
            )
            .1
        );
        assert!(store.list_heartbeats(&sid).unwrap().is_empty());
    }

    #[test]
    fn wakeup_fires_once_then_is_gone() {
        let (store, sid) = setup();
        let (_, ok) = schedule(
            &store,
            &sid,
            &json!({ "delay_seconds": 60, "prompt": "ping" }),
            heartbeat::now_secs() - 120,
        );
        assert!(ok);
        let first = heartbeat::claim_due_heartbeat_prompts(&store, &sid).unwrap();
        assert_eq!(first, vec!["[scheduled wakeup] ping"]);
        assert!(
            store.list_heartbeats(&sid).unwrap().is_empty(),
            "one-shot row removed"
        );
        assert!(heartbeat::claim_due_heartbeat_prompts(&store, &sid)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn not_due_wakeup_does_not_fire() {
        let (store, sid) = setup();
        schedule(
            &store,
            &sid,
            &json!({ "delay_seconds": 600, "prompt": "later" }),
            heartbeat::now_secs(),
        );
        assert!(heartbeat::claim_due_heartbeat_prompts(&store, &sid)
            .unwrap()
            .is_empty());
        assert_eq!(store.list_heartbeats(&sid).unwrap().len(), 1);
    }
}
