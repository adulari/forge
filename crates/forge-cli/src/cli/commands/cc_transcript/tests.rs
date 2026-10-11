use std::io::Write;

use forge_types::Role;
use serde_json::{json, Value};

use super::*;

fn line(v: Value) -> String {
    v.to_string()
}

fn user(uuid: &str, parent: Option<&str>, content: Value) -> String {
    line(json!({
        "type": "user", "uuid": uuid, "parentUuid": parent, "isSidechain": false,
        "cwd": "/work/proj", "timestamp": "2026-10-01T10:00:00.000Z",
        "message": {"role": "user", "content": content},
    }))
}

fn assistant(uuid: &str, parent: &str, msg_id: &str, model: &str, blocks: Value) -> String {
    line(json!({
        "type": "assistant", "uuid": uuid, "parentUuid": parent, "isSidechain": false,
        "timestamp": "2026-10-01T10:00:05.000Z",
        "message": {"id": msg_id, "role": "assistant", "model": model, "content": blocks},
    }))
}

fn tool_result(uuid: &str, parent: &str, id: &str, text: &str) -> String {
    user(
        uuid,
        Some(parent),
        json!([{"type": "tool_result", "tool_use_id": id, "content": text}]),
    )
}

fn boundary(uuid: &str, logical_parent: &str, segment: Option<(&str, &str)>) -> String {
    let mut meta = json!({"trigger": "auto"});
    if let Some((head, tail)) = segment {
        meta["preservedSegment"] = json!({"headUuid": head, "anchorUuid": "s1", "tailUuid": tail});
    }
    line(json!({
        "type": "system", "subtype": "compact_boundary", "uuid": uuid, "parentUuid": null,
        "logicalParentUuid": logical_parent, "isSidechain": false, "compactMetadata": meta,
    }))
}

fn summary(uuid: &str, parent: &str) -> String {
    line(json!({
        "type": "user", "uuid": uuid, "parentUuid": parent, "isSidechain": false,
        "isCompactSummary": true,
        "message": {"role": "user", "content":
            "This session is being continued from a previous conversation.\n\nSummary:\nthe plan"},
    }))
}

fn write(lines: &[String]) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.jsonl");
    let mut f = std::fs::File::create(&path).unwrap();
    for l in lines {
        writeln!(f, "{l}").unwrap();
    }
    (dir, path)
}

fn collect(t: &Transcript) -> Vec<Imported> {
    let mut out = Vec::new();
    t.for_each_message(|m| {
        out.push(m);
        Ok(())
    })
    .unwrap();
    out
}

/// u1 -> a1(text, call 1) -> r1 -> a1(call 2) -> r2 -> a2 -> u2 -> a3, with noise lines mixed in.
fn plain_session() -> Vec<String> {
    vec![
        line(json!({"type": "file-history-snapshot", "messageId": "x", "snapshot": {}})),
        user("u1", None, json!("first question")),
        "{ this is not json".into(),
        assistant(
            "a1",
            "u1",
            "m1",
            "claude-opus-5-5",
            json!([{"type": "thinking", "thinking": "hm"}]),
        ),
        assistant(
            "a1t",
            "a1",
            "m1",
            "claude-opus-5-5",
            json!([{"type": "text", "text": "Checking."}]),
        ),
        assistant(
            "a1c",
            "a1t",
            "m1",
            "claude-opus-5-5",
            json!([{"type": "tool_use", "id": "toolu_1", "name": "Bash", "input": {"command": "ls"}}]),
        ),
        tool_result("r1", "a1c", "toolu_1", "file.txt"),
        assistant(
            "a1d",
            "r1",
            "m1",
            "claude-opus-5-5",
            json!([{"type": "tool_use", "id": "toolu_2", "name": "Read", "input": {"file_path": "/x"}}]),
        ),
        tool_result("r2", "a1d", "toolu_2", "contents"),
        line(
            json!({"type": "attachment", "uuid": "att", "parentUuid": "r2", "isSidechain": false,
            "attachment": {"type": "todo_reminder"}}),
        ),
        assistant(
            "a2",
            "att",
            "m2",
            "claude-opus-5-5",
            json!([{"type": "text", "text": "All done."}]),
        ),
        line(json!({"type": "ai-title", "aiTitle": "Listing files", "sessionId": "s"})),
        user(
            "u2",
            Some("a2"),
            json!([{"type": "text", "text": "second question"}]),
        ),
        line(
            json!({"type": "assistant", "uuid": "side", "parentUuid": "u2", "isSidechain": true,
            "message": {"id": "ms", "role": "assistant", "model": "other", "content": "subagent"}}),
        ),
        assistant(
            "a3",
            "u2",
            "m3",
            "claude-opus-5-5",
            json!([{"type": "text", "text": "Second answer."}]),
        ),
    ]
}

#[test]
fn merges_assistant_fragments_and_keeps_results_after_the_merged_message() {
    let (_d, path) = write(&plain_session());
    let t = Transcript::scan(&path).unwrap();
    let m = collect(&t);
    let shape: Vec<(Role, &str)> = m.iter().map(|m| (m.role, m.content.as_str())).collect();
    assert_eq!(
        shape,
        vec![
            (Role::User, "first question"),
            (Role::Assistant, "Checking."),
            (Role::Tool, "file.txt"),
            (Role::Tool, "contents"),
            (Role::Assistant, "All done."),
            (Role::User, "second question"),
            (Role::Assistant, "Second answer."),
        ]
    );
    let calls = &m[1].tool_calls;
    assert_eq!(
        calls
            .iter()
            .map(|c| (c.id.as_str(), c.name.as_str()))
            .collect::<Vec<_>>(),
        vec![("toolu_1", "Bash"), ("toolu_2", "Read")]
    );
    assert_eq!(calls[0].args["command"], "ls");
    assert_eq!(m[2].tool_call_id.as_deref(), Some("toolu_1"));
    assert_eq!(m[3].tool_call_id.as_deref(), Some("toolu_2"));
    assert!(
        m.iter().all(|m| m.live),
        "no compaction: everything is live"
    );
    assert!(t.summary().unwrap().is_none());
}

#[test]
fn skips_sidechains_and_malformed_lines_and_reads_metadata() {
    let (_d, path) = write(&plain_session());
    let t = Transcript::scan(&path).unwrap();
    assert_eq!(t.skipped_lines, 1);
    assert_eq!(t.cwd.as_deref(), Some("/work/proj"));
    assert_eq!(t.title.as_deref(), Some("Listing files"));
    assert_eq!(t.model.as_deref(), Some("claude-opus-5-5"));
    assert!(!collect(&t).iter().any(|m| m.content.contains("subagent")));
    assert_eq!(t.first_prompt().unwrap().as_deref(), Some("first question"));
}

#[test]
fn a_custom_title_beats_the_auto_title() {
    let mut lines = plain_session();
    lines.push(line(
        json!({"type": "custom-title", "customTitle": "My name for it"}),
    ));
    let (_d, path) = write(&lines);
    assert_eq!(
        Transcript::scan(&path).unwrap().title.as_deref(),
        Some("My name for it")
    );
}

#[test]
fn walks_the_last_leaf_and_ignores_abandoned_branches() {
    let mut lines = plain_session();
    // An abandoned reply to u1, recorded before the final leaf, must not be on the chain.
    lines.insert(
        3,
        assistant(
            "dead",
            "u1",
            "mdead",
            "claude-opus-5-5",
            json!([{"type": "text", "text": "dead end"}]),
        ),
    );
    let (_d, path) = write(&lines);
    let m = collect(&Transcript::scan(&path).unwrap());
    assert!(!m.iter().any(|m| m.content == "dead end"));
    assert_eq!(m.len(), 7);
}

fn compacted_session(segment: Option<(&str, &str)>) -> Vec<String> {
    let mut lines = plain_session();
    lines.push(boundary("b1", "a3", segment));
    lines.push(summary("s1", "b1"));
    lines.push(user("u3", Some("s1"), json!("after compaction")));
    lines.push(assistant(
        "a4",
        "u3",
        "m4",
        "claude-sonnet-5",
        json!([{"type": "text", "text": "Third answer."}]),
    ));
    lines
}

#[test]
fn only_what_follows_the_last_boundary_is_live() {
    let (_d, path) = write(&compacted_session(None));
    let t = Transcript::scan(&path).unwrap();
    assert_eq!(t.compactions(), 1);
    assert!(t.summary().unwrap().unwrap().contains("Summary:\nthe plan"));
    let m = collect(&t);
    let live: Vec<&str> = m
        .iter()
        .filter(|m| m.live)
        .map(|m| m.content.as_str())
        .collect();
    assert_eq!(live, vec!["after compaction", "Third answer."]);
    let history: Vec<&str> = m
        .iter()
        .filter(|m| !m.live)
        .map(|m| m.content.as_str())
        .collect();
    assert!(history.contains(&"first question"));
    assert!(history
        .iter()
        .any(|c| c.starts_with("This session is being continued")));
    assert_eq!(
        t.model.as_deref(),
        Some("claude-sonnet-5"),
        "model of the LAST assistant message"
    );
}

#[test]
fn the_preserved_segment_stays_live_in_front_of_post_compaction_turns() {
    let (_d, path) = write(&compacted_session(Some(("u2", "a3"))));
    let m = collect(&Transcript::scan(&path).unwrap());
    let live: Vec<&str> = m
        .iter()
        .filter(|m| m.live)
        .map(|m| m.content.as_str())
        .collect();
    assert_eq!(
        live,
        vec![
            "second question",
            "Second answer.",
            "after compaction",
            "Third answer."
        ]
    );
    assert!(!m.iter().find(|m| m.content == "All done.").unwrap().live);
}

#[test]
fn earlier_boundaries_are_crossed_through_logical_parent() {
    let mut lines = compacted_session(None);
    lines.push(boundary("b2", "a4", None));
    lines.push(summary("s2", "b2"));
    lines.push(user("u4", Some("s2"), json!("latest")));
    let (_d, path) = write(&lines);
    let t = Transcript::scan(&path).unwrap();
    assert_eq!(t.compactions(), 2);
    let m = collect(&t);
    assert_eq!(m.iter().filter(|m| m.live).count(), 1);
    assert!(
        m.iter().any(|m| m.content == "first question"),
        "full history kept"
    );
}

#[test]
fn unanswered_calls_get_a_placeholder_and_orphan_results_are_dropped() {
    let lines = vec![
        user("u1", None, json!("go")),
        assistant(
            "a1",
            "u1",
            "m1",
            "claude-opus-5-5",
            json!([{"type": "tool_use", "id": "toolu_9", "name": "Bash", "input": {}}]),
        ),
        user("u2", Some("a1"), json!("never mind")),
        tool_result("r9", "u2", "toolu_nope", "stray"),
    ];
    let (_d, path) = write(&lines);
    let t = Transcript::scan(&path).unwrap();
    let mut out = Vec::new();
    let stats = t
        .for_each_message(|m| {
            out.push(m);
            Ok(())
        })
        .unwrap();
    assert_eq!(stats.synthetic_results, 1);
    assert_eq!(stats.dropped_orphan_results, 1);
    assert_eq!(out[2].role, Role::Tool);
    assert_eq!(out[2].tool_call_id.as_deref(), Some("toolu_9"));
    assert_eq!(out[3].content, "never mind");
}

#[test]
fn a_transcript_with_only_noise_is_empty() {
    let (_d, path) = write(&[line(json!({"type": "mode", "mode": "x"})), "garbage".into()]);
    assert!(Transcript::scan(&path).unwrap().is_empty());
}

#[test]
fn results_with_blocks_and_images_flatten_to_text() {
    let lines = vec![
        user("u1", None, json!("look")),
        assistant(
            "a1",
            "u1",
            "m1",
            "m",
            json!([{"type": "tool_use", "id": "t1", "name": "Read", "input": {}}]),
        ),
        user(
            "r1",
            Some("a1"),
            json!([{"type": "tool_result", "tool_use_id": "t1", "content": [
            {"type": "text", "text": "line"}, {"type": "image", "source": {}}]}]),
        ),
    ];
    let (_d, path) = write(&lines);
    let m = collect(&Transcript::scan(&path).unwrap());
    assert_eq!(m[2].content, "line\n[image]");
}

#[test]
fn parallel_tool_results_on_side_branches_are_recovered() {
    let lines = vec![
        user("u1", None, json!("do two things")),
        assistant(
            "a1",
            "u1",
            "m1",
            "m",
            json!([{"type": "tool_use", "id": "t1", "name": "Bash", "input": {}}]),
        ),
        assistant(
            "a2",
            "a1",
            "m1",
            "m",
            json!([{"type": "tool_use", "id": "t2", "name": "Bash", "input": {}}]),
        ),
        tool_result("r1", "a1", "t1", "first"),
        tool_result("r2", "a2", "t2", "second"),
        assistant(
            "a3",
            "r2",
            "m2",
            "m",
            json!([{"type": "text", "text": "both done"}]),
        ),
    ];
    let (_d, path) = write(&lines);
    let t = Transcript::scan(&path).unwrap();
    let m = collect(&t);
    let shape: Vec<(Role, &str)> = m.iter().map(|m| (m.role, m.content.as_str())).collect();
    assert_eq!(
        shape,
        vec![
            (Role::User, "do two things"),
            (Role::Assistant, ""),
            (Role::Tool, "first"),
            (Role::Tool, "second"),
            (Role::Assistant, "both done"),
        ]
    );
    assert_eq!(m[1].tool_calls.len(), 2);
}

#[test]
fn results_are_pulled_ahead_of_text_typed_mid_turn() {
    let lines = vec![
        user("u1", None, json!("start")),
        assistant(
            "a1",
            "u1",
            "m1",
            "m",
            json!([{"type": "tool_use", "id": "t1", "name": "Bash", "input": {}}]),
        ),
        tool_result("r1", "a1", "t1", "one"),
        user("u2", Some("r1"), json!("also this")),
        assistant(
            "a2",
            "u2",
            "m1",
            "m",
            json!([{"type": "tool_use", "id": "t2", "name": "Bash", "input": {}}]),
        ),
        tool_result("r2", "a2", "t2", "two"),
    ];
    let (_d, path) = write(&lines);
    let t = Transcript::scan(&path).unwrap();
    let mut out = Vec::new();
    let stats = t
        .for_each_message(|m| {
            out.push(m);
            Ok(())
        })
        .unwrap();
    let shape: Vec<(Role, &str)> = out.iter().map(|m| (m.role, m.content.as_str())).collect();
    assert_eq!(
        shape,
        vec![
            (Role::User, "start"),
            (Role::Assistant, ""),
            (Role::Tool, "one"),
            (Role::Tool, "two"),
            (Role::User, "also this"),
        ]
    );
    assert_eq!(
        (stats.synthetic_results, stats.dropped_orphan_results),
        (0, 0)
    );
}
