//! Session heartbeats (docs/features/session-heartbeats.md): recurring prompts that re-enter a
//! LIVE session — unlike `forge schedule`, which spawns a fresh `forge run` process off an OS
//! timer, a heartbeat's prompt is submitted as an ordinary queued turn in the SAME running
//! session, once it goes idle. Two kinds share one store table (`forge_store::SessionHeartbeat`):
//! the single user-owned heartbeat (`/heartbeat every`, TUI-only) and agent-created heartbeats
//! (the [`MANAGE_HEARTBEATS_TOOL`] virtual tool, capped at [`MAX_AGENT_HEARTBEATS_PER_SESSION`]
//! per session). They are deliberately kept separate: the model can never modify or clear the
//! user's heartbeat, and the user's commands never touch agent-created ones.

use super::*;

/// Minimum interval a heartbeat may fire at. Below this it stops being a "recurring prompt" and
/// starts being a busy-loop that could dominate the session's turn budget.
pub const MIN_HEARTBEAT_INTERVAL_SECS: i64 = 30;

/// Cap on agent-created heartbeats per session. The user's own heartbeat is a separate singleton
/// (enforced by the store's partial unique index) and never counts against this.
pub const MAX_AGENT_HEARTBEATS_PER_SESSION: usize = 8;

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Parse a heartbeat interval like `30s`, `5m`, `1h` into seconds. A bare number is rejected —
/// requiring the unit suffix prevents a typo'd `/heartbeat every 30 ...` from silently meaning
/// something the user didn't intend. Returns a user-facing error message on any invalid input.
pub fn parse_heartbeat_interval(raw: &str) -> Result<i64, String> {
    let raw = raw.trim();
    let Some(unit) = raw.chars().last() else {
        return Err("interval is required — e.g. 30s, 5m, or 1h".to_string());
    };
    let multiplier = match unit {
        's' => 1,
        'm' => 60,
        'h' => 3600,
        _ => {
            return Err(format!(
                "invalid interval '{raw}' — use a unit suffix: 30s, 5m, or 1h"
            ))
        }
    };
    let digits = &raw[..raw.len() - unit.len_utf8()];
    let n: i64 = digits.trim().parse().map_err(|_| {
        format!("invalid interval '{raw}' — expected a number followed by s/m/h, e.g. 30s")
    })?;
    if n <= 0 {
        return Err(format!(
            "invalid interval '{raw}' — must be a positive number"
        ));
    }
    let secs = n.saturating_mul(multiplier);
    if secs < MIN_HEARTBEAT_INTERVAL_SECS {
        return Err(format!(
            "interval too short ({secs}s) — the minimum is {MIN_HEARTBEAT_INTERVAL_SECS}s"
        ));
    }
    Ok(secs)
}

/// The inverse of [`parse_heartbeat_interval`] for display (`/heartbeat status`, `manage_heartbeats`
/// list). Picks the coarsest unit that divides evenly, falling back to seconds.
pub fn format_heartbeat_interval(secs: i64) -> String {
    if secs % 3600 == 0 {
        format!("{}h", secs / 3600)
    } else if secs % 60 == 0 {
        format!("{}m", secs / 60)
    } else {
        format!("{secs}s")
    }
}

/// One claimed heartbeat tick, formatted as the queued-turn prompt the driver loop submits.
/// Labeled (`[heartbeat: <label>] ...`) so the transcript makes the turn's origin obvious instead
/// of looking like an unexplained user message.
fn format_delivery(hb: &forge_store::SessionHeartbeat) -> String {
    match &hb.label {
        Some(label) => format!("[heartbeat: {label}] {}", hb.prompt),
        None => format!("[heartbeat] {}", hb.prompt),
    }
}

/// Claim every heartbeat due on `session_id` right now and format each as a ready-to-queue turn
/// prompt (see [`format_delivery`]). Call this ONLY when the session is idle — a heartbeat is
/// delivered as an ordinary queued user turn, never injected mid-turn; the caller is responsible
/// for checking `!busy` before calling. A due tick is claimed (its `next_due_at` advanced) by the
/// store as part of the same statement that returns it, so a crash between this call and actually
/// enqueueing the prompt drops at most one tick rather than risking a double-delivery on restart.
pub fn claim_due_heartbeat_prompts(store: &Store, session_id: &str) -> Vec<String> {
    store
        .claim_due_heartbeats(session_id, now_secs())
        .unwrap_or_default()
        .iter()
        .map(format_delivery)
        .collect()
}

/// The `manage_heartbeats` virtual tool name (agent-created heartbeats).
pub const MANAGE_HEARTBEATS_TOOL: &str = "manage_heartbeats";

/// The `ToolSpec` advertised to the model for [`MANAGE_HEARTBEATS_TOOL`]. Shared by the direct
/// path and the CLI-bridge `mcp-serve` handler so a bridged claude/codex sees it too.
pub fn manage_heartbeats_spec() -> ToolSpec {
    ToolSpec {
        name: MANAGE_HEARTBEATS_TOOL.to_string(),
        description: format!(
            "Create and manage your OWN recurring re-entry prompts (heartbeats) for this \
             session — up to {MAX_AGENT_HEARTBEATS_PER_SESSION} at a time. A heartbeat fires by \
             resubmitting its prompt as an ordinary turn once the session goes idle; it never \
             interrupts a running turn, and missed ticks during a long busy stretch coalesce \
             into a single catch-up delivery rather than a replayed backlog. These are separate \
             from — and can never modify or clear — the user's own heartbeat, if they have one. \
             Actions: `create` (requires `label`, `prompt`, `interval` — e.g. \"5m\", minimum \
             {MIN_HEARTBEAT_INTERVAL_SECS}s), `list`, `pause` (requires `label`), `resume` \
             (requires `label`), `delete` (requires `label`)."
        ),
        schema: serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["create", "list", "pause", "resume", "delete"],
                    "description": "what to do"
                },
                "label": {
                    "type": "string",
                    "description": "short identifier for this heartbeat (required for create/pause/resume/delete)"
                },
                "prompt": {
                    "type": "string",
                    "description": "the prompt resubmitted every tick (required for create)"
                },
                "interval": {
                    "type": "string",
                    "description": "e.g. 30s, 5m, 1h — minimum 30s (required for create)"
                }
            },
            "required": ["action"]
        }),
    }
}

/// A label must be short and unambiguous in a transcript prefix (`[heartbeat: <label>]`) — reject
/// anything empty, oversized, or containing characters that would make that prefix confusing.
fn validate_label(label: &str) -> Result<(), String> {
    let label = label.trim();
    if label.is_empty() {
        return Err("label is required".to_string());
    }
    if label.len() > 40 {
        return Err("label too long (max 40 characters)".to_string());
    }
    if !label
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err("label may only contain letters, digits, '-' and '_'".to_string());
    }
    Ok(())
}

/// Render an agent heartbeat listing for the tool result / `/heartbeat status` echo.
pub fn format_heartbeat_list(heartbeats: &[forge_store::SessionHeartbeat]) -> String {
    if heartbeats.is_empty() {
        return "no heartbeats".to_string();
    }
    heartbeats
        .iter()
        .map(|hb| {
            let who = match &hb.label {
                Some(label) => format!("{} ({})", label, hb.owner),
                None => hb.owner.clone(),
            };
            format!(
                "- {who}: every {} — {} — \"{}\"",
                format_heartbeat_interval(hb.interval_secs),
                hb.status,
                hb.prompt
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

impl Session {
    /// Handle a `manage_heartbeats` call: create/list/pause/resume/delete an agent-created
    /// heartbeat on this session. Never touches the user's own heartbeat — every read/write here
    /// is scoped to `owner = 'agent'` at the store layer.
    pub(crate) fn manage_heartbeats(
        &mut self,
        msg_id: &str,
        call: &forge_types::ToolCall,
    ) -> Result<String, CoreError> {
        let args_json = serde_json::to_string(&call.args)?;
        let action = call
            .args
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        let label = call.args.get("label").and_then(|v| v.as_str());

        let (result, ok) = match action {
            "create" => self.create_agent_heartbeat(&call.args),
            "list" => {
                let all = self.store.list_heartbeats(&self.id).unwrap_or_default();
                let agent_only: Vec<_> = all.into_iter().filter(|h| h.owner == "agent").collect();
                (format_heartbeat_list(&agent_only), true)
            }
            "pause" | "resume" => match label {
                None => ("error: `label` is required".to_string(), false),
                Some(label) => self.set_agent_heartbeat_status(label, action == "resume"),
            },
            "delete" => match label {
                None => ("error: `label` is required".to_string(), false),
                Some(label) => {
                    match self.store.delete_agent_heartbeat_by_label(&self.id, label) {
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

    fn create_agent_heartbeat(&mut self, args: &serde_json::Value) -> (String, bool) {
        let label = args.get("label").and_then(|v| v.as_str()).unwrap_or("");
        if let Err(e) = validate_label(label) {
            return (format!("error: {e}"), false);
        }
        let prompt = args
            .get("prompt")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        if prompt.is_empty() {
            return ("error: `prompt` is required".to_string(), false);
        }
        let interval = match args
            .get("interval")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "interval is required — e.g. 30s, 5m, or 1h".to_string())
            .and_then(parse_heartbeat_interval)
        {
            Ok(secs) => secs,
            Err(e) => return (format!("error: {e}"), false),
        };

        let existing = self.store.list_heartbeats(&self.id).unwrap_or_default();
        let agent_count = existing.iter().filter(|h| h.owner == "agent").count();
        if agent_count >= MAX_AGENT_HEARTBEATS_PER_SESSION {
            return (
                format!(
                    "error: at most {MAX_AGENT_HEARTBEATS_PER_SESSION} agent heartbeats per \
                     session — delete one first"
                ),
                false,
            );
        }
        if existing
            .iter()
            .any(|h| h.owner == "agent" && h.label.as_deref() == Some(label))
        {
            return (
                format!("error: an agent heartbeat named '{label}' already exists"),
                false,
            );
        }

        let id = forge_types::new_id();
        match self
            .store
            .add_agent_heartbeat(&id, &self.id, label, prompt, interval, now_secs())
        {
            Ok(()) => (
                format!(
                    "heartbeat '{label}' created — every {}",
                    format_heartbeat_interval(interval)
                ),
                true,
            ),
            Err(e) => (format!("error: {e}"), false),
        }
    }

    fn set_agent_heartbeat_status(&mut self, label: &str, resume: bool) -> (String, bool) {
        let existing = self.store.list_heartbeats(&self.id).unwrap_or_default();
        let Some(hb) = existing
            .iter()
            .find(|h| h.owner == "agent" && h.label.as_deref() == Some(label))
        else {
            return (format!("no agent heartbeat named '{label}'"), false);
        };
        let status = if resume { "active" } else { "paused" };
        match self.store.set_heartbeat_status(&hb.id, status, now_secs()) {
            Ok(true) => (format!("heartbeat '{label}' {status}"), true),
            Ok(false) => (format!("no agent heartbeat named '{label}'"), false),
            Err(e) => (format!("error: {e}"), false),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interval_parsing_enforces_the_30s_floor_and_requires_a_unit() {
        assert_eq!(parse_heartbeat_interval("30s"), Ok(30));
        assert_eq!(parse_heartbeat_interval("5m"), Ok(300));
        assert_eq!(parse_heartbeat_interval("1h"), Ok(3600));
        assert!(parse_heartbeat_interval("29s").is_err(), "below the floor");
        assert!(
            parse_heartbeat_interval("0s").is_err(),
            "zero is not positive"
        );
        assert!(
            parse_heartbeat_interval("-5m").is_err(),
            "negative is not positive"
        );
        assert!(
            parse_heartbeat_interval("5").is_err(),
            "bare number has no unit"
        );
        assert!(parse_heartbeat_interval("").is_err(), "empty");
        assert!(parse_heartbeat_interval("5x").is_err(), "unknown unit");
    }

    #[test]
    fn interval_formatting_picks_the_coarsest_exact_unit() {
        assert_eq!(format_heartbeat_interval(30), "30s");
        assert_eq!(format_heartbeat_interval(300), "5m");
        assert_eq!(format_heartbeat_interval(3600), "1h");
        assert_eq!(format_heartbeat_interval(90), "90s"); // not a whole minute
    }

    fn test_session() -> Session {
        let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let config = Config::default();
        Session::start(
            Arc::new(Store::open_in_memory().unwrap()),
            Arc::new(forge_provider::MockProvider),
            Arc::new(HeuristicRouter::new(config.clone())),
            ToolRegistry::with_core_tools_in(workspace),
            Box::new(forge_tui::HeadlessPresenter::new(false)),
            config,
            workspace.to_str().expect("workspace path is UTF-8"),
        )
        .unwrap()
    }

    /// `record_tool_call` FK-references `message.id`, so tests need a real message row to hang
    /// tool calls off — an arbitrary string id (unlike the store-layer tests, which call the
    /// store directly and never touch this table) fails with a constraint violation.
    fn seed_message(s: &Session) -> String {
        s.store
            .add_message(s.id(), 0, forge_types::Role::User, "hi", None)
            .unwrap()
    }

    fn call(
        action: &str,
        label: Option<&str>,
        prompt: Option<&str>,
        interval: Option<&str>,
    ) -> forge_types::ToolCall {
        let mut args = serde_json::json!({ "action": action });
        if let Some(l) = label {
            args["label"] = serde_json::json!(l);
        }
        if let Some(p) = prompt {
            args["prompt"] = serde_json::json!(p);
        }
        if let Some(i) = interval {
            args["interval"] = serde_json::json!(i);
        }
        forge_types::ToolCall {
            id: "tc1".to_string(),
            name: MANAGE_HEARTBEATS_TOOL.to_string(),
            args,
        }
    }

    #[test]
    fn manage_heartbeats_enforces_the_per_session_cap() {
        let mut s = test_session();
        let msg_id = seed_message(&s);
        for i in 0..MAX_AGENT_HEARTBEATS_PER_SESSION {
            let label = format!("hb{i}");
            let result = s
                .manage_heartbeats(
                    &msg_id,
                    &call("create", Some(&label), Some("ping"), Some("30s")),
                )
                .unwrap();
            assert!(!result.starts_with("error"), "{result}");
        }
        // The cap-th create must be rejected, not silently accepted.
        let result = s
            .manage_heartbeats(
                &msg_id,
                &call("create", Some("one-too-many"), Some("ping"), Some("30s")),
            )
            .unwrap();
        assert!(
            result.starts_with("error"),
            "expected a cap error: {result}"
        );
        assert_eq!(
            s.store.agent_heartbeat_count(s.id()).unwrap() as usize,
            MAX_AGENT_HEARTBEATS_PER_SESSION
        );
    }

    #[test]
    fn manage_heartbeats_never_touches_the_users_own_heartbeat() {
        let mut s = test_session();
        let msg_id = seed_message(&s);
        s.store
            .set_user_heartbeat("user-hb", s.id(), "user prompt", 60, 1_000)
            .unwrap();

        // Agent create/list/pause/delete never sees or mutates the user's row.
        s.manage_heartbeats(
            &msg_id,
            &call("create", Some("watch"), Some("agent prompt"), Some("30s")),
        )
        .unwrap();
        let listed = s
            .manage_heartbeats(&msg_id, &call("list", None, None, None))
            .unwrap();
        assert!(listed.contains("watch"));
        assert!(!listed.contains("user prompt"));

        s.manage_heartbeats(&msg_id, &call("delete", Some("watch"), None, None))
            .unwrap();
        // The agent heartbeat is gone, but the user's is completely untouched.
        assert!(s
            .store
            .list_heartbeats(s.id())
            .unwrap()
            .iter()
            .all(|h| h.owner != "agent"));
        let user_hb = s.store.user_heartbeat(s.id()).unwrap().unwrap();
        assert_eq!(user_hb.prompt, "user prompt");
        assert_eq!(user_hb.status, "active");

        // The agent-facing `delete` action by definition can't target the user's label (it has
        // none), so trying its own label back never removes the user's heartbeat either.
        s.manage_heartbeats(&msg_id, &call("delete", Some("user-hb"), None, None))
            .unwrap();
        assert!(s.store.user_heartbeat(s.id()).unwrap().is_some());
    }

    #[test]
    fn duplicate_label_is_a_clean_error_not_a_panic() {
        let mut s = test_session();
        let msg_id = seed_message(&s);
        s.manage_heartbeats(
            &msg_id,
            &call("create", Some("watch"), Some("first"), Some("30s")),
        )
        .unwrap();
        let result = s
            .manage_heartbeats(
                &msg_id,
                &call("create", Some("watch"), Some("second"), Some("30s")),
            )
            .unwrap();
        assert!(result.starts_with("error"), "{result}");
        assert_eq!(s.store.agent_heartbeat_count(s.id()).unwrap(), 1);
    }
}
