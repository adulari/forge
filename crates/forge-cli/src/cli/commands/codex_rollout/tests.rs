use std::io::Write;

use serde_json::json;

use super::*;

pub(crate) const CODEX_ID: &str = "0199aaaa-1111-7222-8333-444455556666";

/// A synthetic rollout in the shape Codex writes: wrapped lines, an injected-context user message,
/// a shell call with its output, parallel calls, reasoning and mirrored `event_msg` lines.
pub(crate) fn write_rollout(dir: &Path, cwd: &str) -> PathBuf {
    let day = dir.join("2026").join("10").join("01");
    std::fs::create_dir_all(&day).unwrap();
    let path = day.join(format!("rollout-2026-10-01T10-00-00-{CODEX_ID}.jsonl"));
    let mut f = std::fs::File::create(&path).unwrap();
    let ts = |s: u32| format!("2026-10-01T10:00:{s:02}.000Z");
    let lines = [
        json!({"timestamp": ts(0), "type": "session_meta", "payload": {"id": CODEX_ID, "cwd": cwd,
            "originator": "codex_cli_rs", "cli_version": "0.99.0"}}),
        json!({"timestamp": ts(1), "type": "response_item", "payload": {"type": "message", "role": "user",
            "content": [{"type": "input_text", "text": "<environment_context>\n  <cwd>/x</cwd>\n</environment_context>"}]}}),
        json!({"timestamp": ts(1), "type": "turn_context", "payload": {"cwd": cwd, "model": "gpt-5-codex"}}),
        json!({"timestamp": ts(2), "type": "response_item", "payload": {"type": "message", "role": "user",
            "content": [{"type": "input_text", "text": "list the files and count them"}]}}),
        json!({"timestamp": ts(2), "type": "event_msg", "payload": {"type": "user_message", "message": "list the files and count them"}}),
        json!({"timestamp": ts(3), "type": "response_item", "payload": {"type": "reasoning", "summary": [], "encrypted_content": "zzz"}}),
        json!({"timestamp": ts(4), "type": "response_item", "payload": {"type": "message", "role": "assistant",
            "content": [{"type": "output_text", "text": "Listing now."}]}}),
        json!({"timestamp": ts(5), "type": "response_item", "payload": {"type": "function_call", "name": "shell",
            "arguments": "{\"command\":[\"bash\",\"-lc\",\"ls\"]}", "call_id": "call_1"}}),
        json!({"timestamp": ts(5), "type": "response_item", "payload": {"type": "function_call", "name": "shell",
            "arguments": "{\"command\":[\"bash\",\"-lc\",\"ls | wc -l\"]}", "call_id": "call_2"}}),
        json!({"timestamp": ts(6), "type": "response_item", "payload": {"type": "function_call_output", "call_id": "call_1",
            "output": "{\"output\":\"a.rs\\nb.rs\\n\",\"metadata\":{\"exit_code\":0}}"}}),
        json!({"timestamp": ts(6), "type": "response_item", "payload": {"type": "function_call_output", "call_id": "call_2",
            "output": "2"}}),
        json!({"timestamp": ts(7), "type": "response_item", "payload": {"type": "message", "role": "assistant",
            "content": [{"type": "output_text", "text": "Two files."}]}}),
        json!({"timestamp": ts(8), "type": "response_item", "payload": {"type": "function_call", "name": "shell",
            "arguments": "{}", "call_id": "call_orphan"}}),
    ];
    for l in lines {
        writeln!(f, "{l}").unwrap();
    }
    writeln!(f, "this line is not json").unwrap();
    path
}

#[test]
fn a_rollout_becomes_user_assistant_and_tool_rows_in_order() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_rollout(dir.path(), "/work/proj");
    let r = read(&path).unwrap();
    assert_eq!(r.meta.id.as_deref(), Some(CODEX_ID));
    assert_eq!(r.meta.cwd.as_deref(), Some("/work/proj"));
    assert_eq!(r.meta.model.as_deref(), Some("gpt-5-codex"));
    let rows: Vec<(Role, &str)> = r
        .messages
        .iter()
        .map(|m| (m.role, m.content.as_str()))
        .collect();
    assert_eq!(rows[0], (Role::User, "list the files and count them"));
    assert_eq!(rows[1], (Role::Assistant, "Listing now."));
    assert_eq!(r.messages[1].tool_calls.len(), 2);
    assert_eq!(r.messages[1].tool_calls[0].args["command"][2], "ls");
    assert_eq!(r.messages[1].model.as_deref(), Some("gpt-5-codex"));
    assert_eq!(rows[2], (Role::Tool, "a.rs\nb.rs\n"));
    assert_eq!(r.messages[2].tool_call_id.as_deref(), Some("call_1"));
    assert_eq!(rows[3], (Role::Tool, "2"));
    assert_eq!(rows[4], (Role::Assistant, "Two files."));
    // The call that never got an output is closed with a placeholder so the history round-trips.
    assert_eq!(r.messages[4].tool_calls[0].id, "call_orphan");
    assert_eq!(r.messages[5].role, Role::Tool);
    assert_eq!(r.messages[5].tool_call_id.as_deref(), Some("call_orphan"));
    assert_eq!(r.messages.len(), 6);
}

#[test]
fn the_listing_head_skips_injected_context_for_the_title() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_rollout(dir.path(), "/work/proj");
    let meta = head(&path);
    assert_eq!(
        meta.first_prompt.as_deref(),
        Some("list the files and count them")
    );
    assert_eq!(meta.id.as_deref(), Some(CODEX_ID));
}

#[test]
fn the_id_comes_from_the_file_name_when_the_meta_line_is_missing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir
        .path()
        .join(format!("rollout-2026-10-01T10-00-00-{CODEX_ID}.jsonl"));
    std::fs::write(
        &path,
        "{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"old format\"}]}\n",
    )
    .unwrap();
    let r = read(&path).unwrap();
    assert_eq!(r.meta.id.as_deref(), Some(CODEX_ID));
    assert_eq!(
        r.messages[0].content, "old format",
        "an unwrapped line is its own payload"
    );
}

#[test]
fn locate_finds_a_rollout_by_session_id() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_rollout(dir.path(), "/work/proj");
    assert_eq!(rollout_files(dir.path()), vec![path.clone()]);
    assert_eq!(id_from_filename(&path).as_deref(), Some(CODEX_ID));
    assert!(locate("no-such-session").is_err());
}

fn write_compacted(dir: &Path) -> PathBuf {
    let path = dir.join(format!("rollout-2026-10-02T10-00-00-{CODEX_ID}.jsonl"));
    let msg = |role: &str, kind: &str, text: &str| {
        json!({"type": "response_item", "payload": {"type": "message", "role": role,
            "content": [{"type": kind, "text": text}]}})
    };
    let lines = [
        json!({"type": "session_meta", "payload": {"id": CODEX_ID, "cwd": "/work/proj"}}),
        msg("user", "input_text", "first task"),
        msg("assistant", "output_text", "first answer"),
        json!({"type": "compacted", "payload": {"message": "we did the first task"}}),
        msg("user", "input_text", "second task"),
        msg("assistant", "output_text", "second answer"),
    ];
    let text: Vec<String> = lines.iter().map(ToString::to_string).collect();
    std::fs::write(&path, text.join("\n") + "\n").unwrap();
    path
}

#[test]
fn a_compaction_takes_the_earlier_rows_out_of_the_live_context() {
    let dir = tempfile::tempdir().unwrap();
    let r = read(&write_compacted(dir.path())).unwrap();
    assert_eq!(r.summary.as_deref(), Some("we did the first task"));
    let live: Vec<(&str, bool)> = r
        .messages
        .iter()
        .map(|m| (m.content.as_str(), m.live))
        .collect();
    assert_eq!(
        live,
        [
            ("first task", false),
            ("first answer", false),
            ("second task", true),
            ("second answer", true)
        ]
    );
}

#[test]
fn a_compaction_without_a_message_falls_back_to_the_replacement_history() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir
        .path()
        .join(format!("rollout-2026-10-02T10-00-00-{CODEX_ID}.jsonl"));
    let compacted = json!({"type": "compacted", "payload": {"message": "", "replacement_history": [
        {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "summary as a user turn"}]}]}});
    std::fs::write(&path, format!("{compacted}\n")).unwrap();
    assert_eq!(
        read(&path).unwrap().summary.as_deref(),
        Some("summary as a user turn")
    );
}
