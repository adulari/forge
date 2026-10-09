//! Claude-Code hook protocol: the stdin payload a CC hook script expects, the tool-argument
//! translation in both directions, and the interpretation of its exit code + stdout/stderr.
//! Split from `hooks.rs` (which runs the processes) so the protocol is testable without spawning.

use forge_config::HookEvent;
use forge_types::truncate_ellipsis as truncate;
use serde_json::{json, Map, Value};

/// `(claude_key, forge_key)` pairs for the tools whose argument names differ.
fn arg_renames(forge_tool: &str) -> &'static [(&'static str, &'static str)] {
    match forge_tool {
        "edit_file" => &[
            ("file_path", "path"),
            ("old_string", "old"),
            ("new_string", "new"),
        ],
        "write_file" | "create_file" | "read_file" => &[("file_path", "path")],
        "shell" => &[("run_in_background", "background")],
        _ => &[],
    }
}

/// Forge tool args -> the `tool_input` a Claude-Code hook reads. Both spellings are present so a
/// hook written for either works; [`cc_input_to_forge`] collapses them again.
pub(crate) fn forge_args_to_cc(forge_tool: &str, args: &Value) -> Value {
    let Some(map) = args.as_object() else {
        return args.clone();
    };
    let mut out = map.clone();
    for (cc, forge) in arg_renames(forge_tool) {
        if let Some(v) = map.get(*forge) {
            out.entry((*cc).to_string()).or_insert_with(|| v.clone());
        }
    }
    if forge_tool == "shell" {
        if let Some(secs) = map.get("timeout_secs").and_then(Value::as_u64) {
            out.entry("timeout".to_string())
                .or_insert_with(|| json!(secs.saturating_mul(1000)));
        }
    }
    Value::Object(out)
}

/// A hook's `updatedInput` (Claude-Code argument names) -> Forge tool args. A Claude-Code key wins
/// over its Forge twin, because a hook that edits `tool_input` edits the CC spelling it was shown.
pub(crate) fn cc_input_to_forge(forge_tool: &str, input: &Value) -> Value {
    let Some(map) = input.as_object() else {
        return input.clone();
    };
    let mut out = map.clone();
    for (cc, forge) in arg_renames(forge_tool) {
        if let Some(v) = out.remove(*cc) {
            out.insert((*forge).to_string(), v);
        }
    }
    if forge_tool == "shell" {
        out.remove("description");
        if let Some(ms) = out.remove("timeout").and_then(|v| v.as_u64()) {
            out.insert("timeout_secs".to_string(), json!(ms.div_ceil(1000).max(1)));
        }
    }
    Value::Object(out)
}

/// Translate Forge's native hook payload (`{tool, args, result?, ok?}` / `{prompt}`) into the
/// Claude-Code stdin shape so a CC hook script reads the fields it expects (`tool_name`,
/// `tool_input`, `tool_response`, `prompt`, `hook_event_name`, `cwd`, `session_id`,
/// `transcript_path`). `session_id` falls back to the payload's own when the caller passes "".
/// `transcript_path` is empty: Forge keeps its transcript in its store, not a CC jsonl file.
pub(crate) fn to_cc_payload(forge_payload: &str, event: HookEvent, session_id: &str) -> String {
    let v: Value = serde_json::from_str(forge_payload).unwrap_or(Value::Null);
    let cwd = v
        .get("cwd")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| {
            std::env::current_dir()
                .ok()
                .map(|p| p.display().to_string())
        })
        .unwrap_or_default();
    let session_id = if session_id.is_empty() {
        v.get("session_id").and_then(Value::as_str).unwrap_or("")
    } else {
        session_id
    };
    let mut obj = Map::new();
    obj.insert("session_id".into(), session_id.into());
    obj.insert("transcript_path".into(), "".into());
    obj.insert("cwd".into(), cwd.into());
    obj.insert("hook_event_name".into(), event.cc_name().into());
    let tool = v.get("tool").and_then(Value::as_str);
    if let Some(tool) = tool {
        obj.insert("tool_name".into(), forge_config::cc_tool_alias(tool).into());
    }
    if let Some(args) = v.get("args") {
        let input = forge_args_to_cc(tool.unwrap_or(""), args);
        obj.insert("tool_input".into(), input);
    }
    if let Some(result) = v.get("result") {
        obj.insert("tool_response".into(), result.clone());
    }
    if let Some(prompt) = v.get("prompt") {
        obj.insert("prompt".into(), prompt.clone());
    }
    // Carry through any extra lifecycle fields (message, trigger, …) the caller already put in.
    if let Some(map) = v.as_object() {
        for (k, val) in map {
            if !["tool", "args", "result", "ok", "prompt", "event", "cwd"].contains(&k.as_str()) {
                obj.entry(k.clone()).or_insert_with(|| val.clone());
            }
        }
    }
    Value::Object(obj).to_string()
}

/// What a Claude-Code hook's exit code + output asked for. Fields are independent: one JSON
/// object can carry a context string AND an `updatedInput`.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct CcOutput {
    /// Block the call/turn (exit 2, `decision:"block"`, or `permissionDecision:"deny"`).
    pub block: Option<String>,
    /// Model-visible context (`additionalContext`, top level or under `hookSpecificOutput`).
    pub context: Option<String>,
    /// `hookSpecificOutput.updatedInput`, in Claude-Code argument names.
    pub updated_input: Option<Value>,
    /// Text for the user: plain stdout, or JSON this parser does not recognise. The session-start
    /// and prompt events treat plain stdout as context instead (CC semantics).
    pub note: Option<String>,
    /// `{"continue":false}`: the hook wants the agent to stop whatever it was doing.
    pub halt: bool,
}

/// Interpret a CC hook result: exit 2 blocks (stderr = reason); otherwise parse stdout for
/// `{"decision":"block|approve","reason":…}`, `hookSpecificOutput` (`permissionDecision`,
/// `updatedInput`, `additionalContext`), a top-level `additionalContext`, `continue:false`, and
/// `systemMessage`. Plain non-JSON stdout becomes `note`. Empty stdout + exit 0 is a clean no-op.
pub(crate) fn parse_cc_output(code: i32, stdout: &str, stderr: &str) -> CcOutput {
    let mut out = CcOutput::default();
    if code == 2 {
        let err = stderr.trim();
        let reason = if !err.is_empty() {
            err
        } else if !stdout.trim().is_empty() {
            stdout.trim()
        } else {
            "blocked by hook (exit 2)"
        };
        out.block = Some(truncate(reason, 800));
        return out;
    }
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        return out;
    }
    let Ok(v @ Value::Object(_)) = serde_json::from_str::<Value>(trimmed) else {
        out.note = Some(truncate(trimmed, 800));
        return out;
    };
    let mut recognised = false;
    if v.get("decision").and_then(Value::as_str) == Some("block") {
        let reason = v
            .get("reason")
            .and_then(Value::as_str)
            .unwrap_or("blocked by hook");
        out.block = Some(truncate(reason, 800));
        recognised = true;
    } else if matches!(
        v.get("decision").and_then(Value::as_str),
        Some("approve" | "allow")
    ) {
        recognised = true;
    }
    if v.get("continue").and_then(Value::as_bool) == Some(false) {
        out.halt = true;
        recognised = true;
    }
    if let Some(hso) = v.get("hookSpecificOutput").filter(|h| h.is_object()) {
        recognised = true;
        if hso.get("permissionDecision").and_then(Value::as_str) == Some("deny") {
            let reason = hso
                .get("permissionDecisionReason")
                .and_then(Value::as_str)
                .unwrap_or("denied by hook");
            out.block = Some(truncate(reason, 800));
        }
        out.updated_input = hso.get("updatedInput").filter(|i| i.is_object()).cloned();
        out.context = non_blank(hso.get("additionalContext"));
    }
    if out.context.is_none() {
        out.context = non_blank(v.get("additionalContext"));
        recognised |= out.context.is_some();
    }
    if let Some(msg) = non_blank(v.get("systemMessage")) {
        out.note = Some(msg);
        recognised = true;
    }
    if !recognised {
        out.note = Some(truncate(trimmed, 800));
    }
    out
}

fn non_blank(v: Option<&Value>) -> Option<String> {
    v.and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .map(|s| truncate(s, 2000))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rtk_style_output_yields_updated_input_without_a_note() {
        let out = parse_cc_output(
            0,
            r#"{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecisionReason":"RTK auto-rewrite","updatedInput":{"command":"rtk git status"},"permissionDecision":"allow"}}"#,
            "",
        );
        assert_eq!(out.updated_input.unwrap()["command"], "rtk git status");
        assert!(out.block.is_none() && out.note.is_none());
    }

    #[test]
    fn deny_blocks_and_context_and_input_can_coexist() {
        let out = parse_cc_output(
            0,
            r#"{"hookSpecificOutput":{"permissionDecision":"deny","permissionDecisionReason":"no","additionalContext":"why"}}"#,
            "",
        );
        assert_eq!(out.block.as_deref(), Some("no"));
        assert_eq!(out.context.as_deref(), Some("why"));
    }

    #[test]
    fn stop_style_outputs() {
        assert_eq!(
            parse_cc_output(0, r#"{"decision":"block","reason":"run tests"}"#, "")
                .block
                .as_deref(),
            Some("run tests")
        );
        assert!(parse_cc_output(0, r#"{"continue":false,"stopReason":"x"}"#, "").halt);
        assert_eq!(
            parse_cc_output(2, "", "keep going\n").block.as_deref(),
            Some("keep going")
        );
        assert_eq!(
            parse_cc_output(1, "", "oops").block,
            None,
            "exit 1 is a non-blocking error"
        );
    }

    #[test]
    fn unrecognised_json_and_plain_text_become_notes() {
        assert!(parse_cc_output(0, r#"{"foo":1}"#, "").note.is_some());
        assert_eq!(
            parse_cc_output(0, "hello", "").note.as_deref(),
            Some("hello")
        );
        assert_eq!(parse_cc_output(0, "", ""), CcOutput::default());
    }

    #[test]
    fn bash_input_round_trips_through_cc_names() {
        let forge = json!({"command":"ls","timeout_secs":30,"background":true});
        let cc = forge_args_to_cc("shell", &forge);
        assert_eq!(cc["timeout"], 30000);
        assert_eq!(cc["run_in_background"], true);
        let back = cc_input_to_forge(
            "shell",
            &json!({"command":"rtk ls","description":"d","timeout":1500,"run_in_background":false}),
        );
        assert_eq!(back["command"], "rtk ls");
        assert_eq!(back["timeout_secs"], 2);
        assert_eq!(back["background"], false);
        assert!(back.get("description").is_none() && back.get("timeout").is_none());
    }

    #[test]
    fn edit_input_maps_both_ways_and_cc_spelling_wins() {
        let forge = json!({"path":"a.rs","old":"x","new":"y"});
        let cc = forge_args_to_cc("edit_file", &forge);
        assert_eq!(cc["file_path"], "a.rs");
        assert_eq!(cc["old_string"], "x");
        let mut edited = cc.clone();
        edited["new_string"] = json!("z");
        let back = cc_input_to_forge("edit_file", &edited);
        assert_eq!(back, json!({"path":"a.rs","old":"x","new":"z"}));
    }

    #[test]
    fn payload_uses_cc_tool_name_input_and_payload_session_id() {
        let p = to_cc_payload(
            r#"{"tool":"shell","args":{"command":"ls"},"session_id":"s1","cwd":"/w"}"#,
            HookEvent::PreToolUse,
            "",
        );
        let v: Value = serde_json::from_str(&p).unwrap();
        assert_eq!(v["tool_name"], "Bash");
        assert_eq!(v["tool_input"]["command"], "ls");
        assert_eq!(v["session_id"], "s1");
        assert_eq!(v["cwd"], "/w");
    }
}
