use std::io::Write;
use std::path::{Path, PathBuf};

use forge_store::{ExportRow, Store};
use forge_types::{Role, ToolCall, Visibility};
use serde_json::{json, Value};

use super::export::{build_entries, cc_tool_id, ExportContext};
use super::sync::{export_to_claude_in, mirror_from_claude, Mirrored};
use super::*;

const CWD: &str = "/work/proj";

fn ctx() -> ExportContext<'static> {
    ExportContext {
        cc_session_id: "sess",
        cwd: CWD,
        version: "9.9.9",
        git_branch: Some("main"),
    }
}

fn row(seq: i64, role: Role, content: &str) -> ExportRow {
    ExportRow {
        seq,
        role,
        content: content.into(),
        model: None,
        tool_calls: vec![],
        tool_call_id: None,
        visibility: Visibility::Llm,
        nudge: false,
        active: true,
        created_at: 1_700_000_000 + seq,
    }
}

fn call(id: &str, name: &str, args: Value) -> ToolCall {
    ToolCall {
        id: id.into(),
        name: name.into(),
        args,
    }
}

fn assistant_with(seq: i64, text: &str, calls: Vec<ToolCall>) -> ExportRow {
    ExportRow {
        tool_calls: calls,
        model: Some("claude-cli::claude-haiku-5-5".into()),
        ..row(seq, Role::Assistant, text)
    }
}

fn tool_row(seq: i64, id: &str, content: &str) -> ExportRow {
    ExportRow {
        tool_call_id: Some(id.into()),
        ..row(seq, Role::Tool, content)
    }
}

fn parse(lines: &[String]) -> Vec<Value> {
    lines
        .iter()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

fn chain(entries: &[Value]) -> Vec<&Value> {
    entries.iter().filter(|e| e["uuid"].is_string()).collect()
}

#[test]
fn a_fixture_session_serializes_to_a_valid_claude_code_chain() {
    let long = "toolu_01ABCDEFGHIJKLMNOP";
    let rows = vec![
        row(0, Role::User, "build it"),
        row(1, Role::System, "[time] now"),
        ExportRow {
            nudge: true,
            ..row(2, Role::User, "keep going")
        },
        ExportRow {
            visibility: Visibility::UiOnly,
            ..row(3, Role::Assistant, "display copy")
        },
        assistant_with(
            4,
            "Running two things.",
            vec![
                call(
                    "call_0",
                    "shell",
                    json!({"command": "ls", "timeout_secs": 4}),
                ),
                call("weird id/1", "read_file", json!({"path": "/etc/hostname"})),
                call("x", "spawn_agents", json!({})),
                call(long, "Bash", json!({"command": "pwd"})),
            ],
        ),
        tool_row(5, "call_0", "a.rs\n"),
        tool_row(6, "weird id/1", "myhost"),
        tool_row(7, "ghost", "answers nothing"),
        tool_row(8, long, "/work"),
        assistant_with(9, "Done.", vec![]),
        row(10, Role::User, "thanks"),
        row(11, Role::User, "and one more thing"),
    ];
    let out = build_entries(&ctx(), None, Some("My title"), None, &rows);
    let entries = parse(&out.lines);

    assert_eq!(entries[0]["type"], "custom-title");
    assert_eq!(entries[0]["customTitle"], "My title");
    let msgs = chain(&entries);
    let shape: Vec<&str> = msgs.iter().map(|e| e["type"].as_str().unwrap()).collect();
    assert_eq!(shape, ["user", "assistant", "user", "assistant", "user"]);
    for pair in msgs.windows(2) {
        assert_eq!(
            pair[1]["parentUuid"], pair[0]["uuid"],
            "entries chain one after another"
        );
    }
    assert_eq!(msgs[0]["parentUuid"], Value::Null);
    assert_eq!(msgs[0]["message"]["content"], "build it");
    for e in &msgs {
        assert_eq!(e["entrypoint"], "cli");
        assert_eq!(e["version"], "9.9.9");
        assert_eq!(e["gitBranch"], "main");
        assert_eq!(e["cwd"], CWD);
        assert_eq!(e["sessionId"], "sess");
    }

    let blocks = msgs[1]["message"]["content"].as_array().unwrap();
    assert_eq!(blocks[0]["text"], "Running two things.");
    let uses: Vec<&Value> = blocks.iter().filter(|b| b["type"] == "tool_use").collect();
    assert_eq!(uses.len(), 4, "parallel calls share one assistant entry");
    assert_eq!(uses[0]["name"], "Bash");
    assert_eq!(uses[0]["input"], json!({"command": "ls", "timeout": 4000}));
    assert_eq!(uses[1]["name"], "Read");
    assert_eq!(uses[1]["input"], json!({"file_path": "/etc/hostname"}));
    assert_eq!(uses[2]["name"], "mcp__forge__spawn_agents");
    assert_eq!(
        uses[3]["id"], long,
        "ids Claude Code would accept as they are are kept"
    );
    assert_eq!(msgs[1]["message"]["stop_reason"], "tool_use");

    let id_ok = |id: &str| {
        !id.is_empty()
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    };
    let results = msgs[2]["message"]["content"].as_array().unwrap();
    assert_eq!(
        results.len(),
        4,
        "one result per call, in call order, in ONE entry"
    );
    for (u, r) in uses.iter().zip(results) {
        assert!(id_ok(u["id"].as_str().unwrap()));
        assert_eq!(r["type"], "tool_result");
        assert_eq!(
            r["tool_use_id"], u["id"],
            "the result carries the same sanitized id"
        );
    }
    assert_eq!(results[0]["content"], "a.rs\n");
    assert_eq!(
        results[2]["content"], "[no result recorded]",
        "an orphan call gets a placeholder"
    );
    assert!(
        !out.lines.concat().contains("answers nothing"),
        "a result with no call is dropped"
    );
    assert!(!out.lines.concat().contains("display copy"));
    assert!(!out.lines.concat().contains("[time] now"));
    assert!(!out.lines.concat().contains("keep going"));

    assert_eq!(msgs[3]["message"]["content"][0]["text"], "Done.");
    assert_eq!(
        msgs[4]["message"]["content"], "thanks\n\nand one more thing",
        "consecutive user rows become one entry"
    );
    let last = entries.last().unwrap();
    assert_eq!(last["type"], "last-prompt");
    assert_eq!(last["leafUuid"], msgs[4]["uuid"]);
}

#[test]
fn tool_ids_are_sanitized_deterministically_and_unique_per_turn() {
    assert_eq!(cc_tool_id("call_0", 4), cc_tool_id("call_0", 4));
    assert_ne!(
        cc_tool_id("call_0", 4),
        cc_tool_id("call_0", 9),
        "the same id reused in another turn differs"
    );
    assert_eq!(
        cc_tool_id("toolu_01ABCDEFGHIJKLMNOP", 4),
        "toolu_01ABCDEFGHIJKLMNOP"
    );
    for id in ["", "a b", "weird id/1", "é", "x"] {
        let out = cc_tool_id(id, 3);
        assert!(
            !out.is_empty()
                && out
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-'),
            "{out}"
        );
    }
}

#[test]
fn a_compaction_summary_becomes_a_boundary_and_summary_entry() {
    let rows = vec![row(5, Role::User, "next"), assistant_with(6, "ok", vec![])];
    let out = build_entries(
        &ctx(),
        Some("leafleaf"),
        None,
        Some("We built the thing."),
        &rows,
    );
    let entries = parse(&out.lines);
    let boundary = &entries[0];
    assert_eq!(boundary["type"], "system");
    assert_eq!(boundary["subtype"], "compact_boundary");
    assert_eq!(boundary["parentUuid"], Value::Null);
    assert_eq!(boundary["logicalParentUuid"], "leafleaf");
    let summary = &entries[1];
    assert_eq!(summary["isCompactSummary"], true);
    assert_eq!(summary["isVisibleInTranscriptOnly"], true);
    assert_eq!(summary["parentUuid"], boundary["uuid"]);
    assert!(summary["message"]["content"]
        .as_str()
        .unwrap()
        .starts_with("This session is being continued"));
    assert!(summary["message"]["content"]
        .as_str()
        .unwrap()
        .ends_with("We built the thing."));
    assert_eq!(entries[2]["parentUuid"], summary["uuid"]);
    assert_eq!(entries[2]["message"]["content"], "next");
}

fn forge_session(store: &Store) -> String {
    let id = store.create_session(CWD, "Default").unwrap();
    store
        .set_session_pinned_model(&id, Some("claude-cli::claude-haiku-5-5"))
        .unwrap();
    id
}

fn add(store: &Store, id: &str, role: Role, content: &str) {
    let seq = store.next_message_seq(id).unwrap();
    let model = (role == Role::Assistant).then_some("claude-cli::claude-haiku-5-5");
    store
        .add_message_full(id, seq, role, content, model, &[], None)
        .unwrap();
}

fn texts(store: &Store, id: &str) -> Vec<(Role, String)> {
    store
        .load_all_messages(id)
        .unwrap()
        .into_iter()
        .filter(|m| m.role != Role::System && m.visibility.is_llm())
        .map(|m| (m.role, m.content))
        .collect()
}

fn lines_of(path: &Path) -> Vec<Value> {
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

fn cc_texts(path: &Path) -> Vec<(String, String)> {
    lines_of(path)
        .into_iter()
        .filter(|e| {
            matches!(e["type"].as_str(), Some("user" | "assistant"))
                && e["isCompactSummary"].is_null()
        })
        .map(|e| {
            let c = &e["message"]["content"];
            let text = c
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| c[0]["text"].as_str().unwrap_or_default().to_string());
            (e["type"].as_str().unwrap().to_string(), text)
        })
        .collect()
}

fn leaf(path: &Path) -> String {
    lines_of(path)
        .iter()
        .rev()
        .find_map(|e| e["uuid"].as_str().map(str::to_string))
        .unwrap()
}

/// Lines a Claude Code session would add for one typed prompt and its answer, with an
/// attachment entry in between, the way real transcripts chain.
fn cc_turn(path: &Path, prompt: &str, answer: &str) {
    let parent = leaf(path);
    let (u, at, a) = (
        forge_types::new_id(),
        forge_types::new_id(),
        forge_types::new_id(),
    );
    let base = |kind: &str, uuid: &str, parent: &str| {
        json!({"type": kind, "uuid": uuid, "parentUuid": parent, "isSidechain": false, "entrypoint": "cli",
               "cwd": CWD, "timestamp": "2026-10-12T10:00:00.000Z", "userType": "external"})
    };
    let mut user = base("user", &u, &parent);
    user["message"] = json!({"role": "user", "content": prompt});
    let mut attach = base("attachment", &at, &u);
    attach["attachment"] = json!({"type": "date", "date": "2026-10-12"});
    let mut asst = base("assistant", &a, &at);
    asst["message"] = json!({"id": format!("msg_{a}"), "role": "assistant", "model": "claude-haiku-5-5",
        "content": [{"type": "text", "text": answer}]});
    let mut f = std::fs::OpenOptions::new().append(true).open(path).unwrap();
    for v in [
        user,
        attach,
        asst,
        json!({"type": "last-prompt", "lastPrompt": prompt, "leafUuid": a}),
        json!({"type": "cost-state", "totalCostUSD": "0.01"}),
    ] {
        writeln!(f, "{v}").unwrap();
    }
}

struct Rig {
    store: Store,
    projects: tempfile::TempDir,
    forge: String,
}

impl Rig {
    fn new() -> Self {
        let store = Store::open_in_memory().unwrap();
        let forge = forge_session(&store);
        add(&store, &forge, Role::User, "q1");
        add(&store, &forge, Role::Assistant, "a1");
        Rig {
            store,
            projects: tempfile::tempdir().unwrap(),
            forge,
        }
    }

    fn export(&self) -> sync::ExportReport {
        export_to_claude_in(&self.store, &self.forge, self.projects.path(), "9.9.9").unwrap()
    }

    fn cc_path(&self) -> PathBuf {
        PathBuf::from(
            self.store
                .cc_link_for_forge(&self.forge)
                .unwrap()
                .unwrap()
                .cc_path,
        )
    }

    fn mirror(&self) -> sync::MirrorReport {
        mirror_from_claude(&self.store, &self.cc_path(), "Default").unwrap()
    }
}

#[test]
fn the_first_export_creates_a_session_file_and_a_link() {
    let rig = Rig::new();
    let report = rig.export();
    let path = rig.cc_path();
    let expected_dir = rig.projects.path().join("-work-proj");
    assert_eq!(
        path,
        expected_dir.join(format!("{}.jsonl", report.cc_session_id))
    );
    assert!(
        cc_fs::is_interactive(&path),
        "what Forge writes is an interactive session"
    );
    assert_eq!(
        cc_texts(&path),
        [
            ("user".into(), "q1".into()),
            ("assistant".into(), "a1".into())
        ]
    );
    let link = rig.store.cc_link_for_forge(&rig.forge).unwrap().unwrap();
    assert_eq!(
        link.cc_synced_bytes as u64,
        std::fs::metadata(&path).unwrap().len()
    );
    assert_eq!(link.forge_synced_seq, 1);
    assert_eq!(link.cc_last_uuid.unwrap(), leaf(&path));
    assert!(std::fs::read_dir(&expected_dir).unwrap().all(|e| !e
        .unwrap()
        .file_name()
        .to_string_lossy()
        .ends_with(".tmp")));
}

#[test]
fn exporting_twice_with_nothing_new_changes_nothing() {
    let rig = Rig::new();
    rig.export();
    let path = rig.cc_path();
    let before = std::fs::read(&path).unwrap();
    let again = rig.export();
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert!(again.warning.is_none());
    let mirrored = rig.mirror();
    assert_eq!(
        mirrored.kind,
        Mirrored::Unchanged,
        "what Forge just wrote is not new to Forge"
    );
    assert_eq!(texts(&rig.store, &rig.forge).len(), 2);
}

#[test]
fn a_forge_turn_is_appended_after_the_current_leaf_and_only_once() {
    let rig = Rig::new();
    rig.export();
    let path = rig.cc_path();
    let leaf_before = leaf(&path);
    let count = |p: &Path| std::fs::read_to_string(p).unwrap().lines().count();
    let lines_before = count(&path);
    add(&rig.store, &rig.forge, Role::User, "q2");
    add(&rig.store, &rig.forge, Role::Assistant, "a2");
    rig.export();
    assert_eq!(
        count(&path),
        lines_before + 3,
        "user, assistant and the last-prompt marker"
    );
    let all = lines_of(&path);
    let q2 = all
        .iter()
        .find(|e| e["message"]["content"] == "q2")
        .unwrap();
    assert_eq!(q2["parentUuid"], leaf_before);
    rig.export();
    assert_eq!(count(&path), lines_before + 3);
    assert_eq!(
        cc_texts(&path)
            .iter()
            .map(|(_, t)| t.as_str())
            .collect::<Vec<_>>(),
        ["q1", "a1", "q2", "a2"]
    );
}

#[test]
fn switching_back_and_forth_with_turns_on_each_side_never_duplicates() {
    let rig = Rig::new();
    rig.export();
    let path = rig.cc_path();
    cc_turn(&path, "q2", "a2");
    assert_eq!(rig.mirror().kind, Mirrored::Appended(2));
    add(&rig.store, &rig.forge, Role::User, "q3");
    add(&rig.store, &rig.forge, Role::Assistant, "a3");
    rig.export();
    cc_turn(&path, "q4", "a4");
    let report = rig.mirror();
    assert_eq!(report.kind, Mirrored::Appended(2));
    assert!(report.warning.is_none());
    // Settle: nothing more moves either way.
    let bytes = std::fs::read(&path).unwrap();
    rig.export();
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert_eq!(rig.mirror().kind, Mirrored::Unchanged);

    let want: Vec<&str> = vec!["q1", "a1", "q2", "a2", "q3", "a3", "q4", "a4"];
    let forge: Vec<String> = texts(&rig.store, &rig.forge)
        .into_iter()
        .map(|(_, t)| t)
        .collect();
    assert_eq!(forge, want, "Forge holds every message once, in order");
    let cc: Vec<String> = cc_texts(&path).into_iter().map(|(_, t)| t).collect();
    assert_eq!(cc, want, "Claude Code holds every message once, in order");
}

#[test]
fn a_session_imported_from_claude_code_switches_back_incrementally() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(format!("{}.jsonl", forge_types::new_id()));
    let first = json!({"type": "user", "uuid": "u1", "parentUuid": null, "isSidechain": false,
        "entrypoint": "cli", "cwd": CWD, "timestamp": "2026-10-12T09:00:00.000Z",
        "message": {"role": "user", "content": "hello from claude"}});
    let second = json!({"type": "assistant", "uuid": "a1", "parentUuid": "u1", "isSidechain": false,
        "entrypoint": "cli", "cwd": CWD, "timestamp": "2026-10-12T09:00:01.000Z",
        "message": {"id": "m1", "role": "assistant", "model": "claude-haiku-5-5",
                    "content": [{"type": "text", "text": "hi"}]}});
    std::fs::write(&path, format!("{first}\n{second}\n")).unwrap();
    let store = Store::open_in_memory().unwrap();
    let imported = mirror_from_claude(&store, &path, "Default").unwrap();
    assert_eq!(imported.kind, Mirrored::Imported);
    let link = store
        .cc_link_for_forge(&imported.forge_id)
        .unwrap()
        .unwrap();
    assert_eq!(link.cc_last_uuid.as_deref(), Some("a1"));
    assert_eq!(link.forge_synced_seq, 1);
    cc_turn(&path, "more", "sure");
    let next = mirror_from_claude(&store, &path, "Default").unwrap();
    assert_eq!(
        (next.kind, next.forge_id.as_str()),
        (Mirrored::Appended(2), imported.forge_id.as_str())
    );
    assert_eq!(texts(&store, &imported.forge_id).len(), 4);
    // And the way back appends only what Forge gained, after Claude Code's own leaf.
    add(&store, &imported.forge_id, Role::User, "from forge");
    let projects = tempfile::tempdir().unwrap();
    export_to_claude_in(&store, &imported.forge_id, projects.path(), "9.9.9").unwrap();
    assert_eq!(cc_texts(&path).last().unwrap().1, "from forge");
    assert_eq!(cc_texts(&path).len(), 5);
}

#[test]
fn when_both_sides_changed_each_direction_warns_and_keeps_every_turn() {
    let rig = Rig::new();
    rig.export();
    let path = rig.cc_path();
    cc_turn(&path, "cc-q", "cc-a");
    add(&rig.store, &rig.forge, Role::User, "forge-q");
    let report = rig.export();
    assert!(report
        .warning
        .as_deref()
        .unwrap()
        .contains("both sides changed"));
    let cc: Vec<String> = cc_texts(&path).into_iter().map(|(_, t)| t).collect();
    assert_eq!(cc, ["q1", "a1", "cc-q", "cc-a", "forge-q"]);
    let all = lines_of(&path);
    let forge_entry = all
        .iter()
        .find(|e| e["message"]["content"] == "forge-q")
        .unwrap();
    let cc_leaf = all
        .iter()
        .find(|e| e["message"]["content"][0]["text"] == "cc-a")
        .unwrap();
    assert_eq!(
        forge_entry["parentUuid"], cc_leaf["uuid"],
        "Forge's turn hangs off Claude Code's real leaf"
    );

    // The same divergence met from the Claude Code side.
    let rig = Rig::new();
    rig.export();
    cc_turn(&rig.cc_path(), "cc-q", "cc-a");
    add(&rig.store, &rig.forge, Role::User, "forge-q");
    let mirrored = rig.mirror();
    assert_eq!(mirrored.kind, Mirrored::Appended(2));
    assert!(mirrored
        .warning
        .as_deref()
        .unwrap()
        .contains("both sides changed"));
    let forge: Vec<String> = texts(&rig.store, &rig.forge)
        .into_iter()
        .map(|(_, t)| t)
        .collect();
    assert_eq!(forge, ["q1", "a1", "forge-q", "cc-q", "cc-a"]);
}

#[test]
fn a_compaction_on_the_claude_code_side_compacts_forge_the_same_way() {
    let rig = Rig::new();
    rig.export();
    let path = rig.cc_path();
    let logical = leaf(&path);
    let mut f = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap();
    let lines = [
        json!({"type": "system", "subtype": "compact_boundary", "uuid": "b1", "parentUuid": null,
               "logicalParentUuid": logical, "isSidechain": false, "compactMetadata": {"trigger": "manual"}}),
        json!({"type": "user", "uuid": "s1", "parentUuid": "b1", "isCompactSummary": true, "isSidechain": false,
               "timestamp": "2026-10-12T11:00:00.000Z",
               "message": {"role": "user", "content": "This session is being continued.\n\nSummary:\nwe did q1"}}),
        json!({"type": "user", "uuid": "u9", "parentUuid": "s1", "isSidechain": false,
               "timestamp": "2026-10-12T11:00:01.000Z", "message": {"role": "user", "content": "after"}}),
    ];
    for l in lines {
        writeln!(f, "{l}").unwrap();
    }
    drop(f);
    assert_eq!(
        rig.mirror().kind,
        Mirrored::Appended(2),
        "the summary row and what followed"
    );
    let live = rig.store.load_messages(&rig.forge).unwrap();
    assert_eq!(live.len(), 2, "summary + what followed");
    assert!(live[0].content.ends_with("we did q1"));
    assert_eq!(live[1].content, "after");
    assert_eq!(
        rig.store.load_all_messages(&rig.forge).unwrap().len(),
        4,
        "older turns stay in history"
    );
    let bytes = std::fs::read(&path).unwrap();
    rig.export();
    assert_eq!(
        std::fs::read(&path).unwrap(),
        bytes,
        "the summary is not echoed back"
    );
}

#[test]
fn a_forge_compaction_since_the_last_sync_appends_a_boundary_and_the_live_rows() {
    let rig = Rig::new();
    rig.export();
    add(&rig.store, &rig.forge, Role::User, "q2");
    add(&rig.store, &rig.forge, Role::Assistant, "a2");
    rig.store
        .compact_session_store(&rig.forge, "Forge summary of q1.", 2)
        .unwrap();
    add(&rig.store, &rig.forge, Role::User, "q3");
    let path = rig.cc_path();
    let leaf_before = leaf(&path);
    rig.export();
    let all = lines_of(&path);
    let boundary = all
        .iter()
        .find(|e| e["subtype"] == "compact_boundary")
        .unwrap();
    assert_eq!(boundary["logicalParentUuid"], leaf_before);
    let summary = all.iter().find(|e| e["isCompactSummary"] == true).unwrap();
    assert!(summary["message"]["content"]
        .as_str()
        .unwrap()
        .contains("Forge summary of q1."));
    let after: Vec<String> = all
        .iter()
        .skip_while(|e| e["isCompactSummary"] != true)
        .skip(1)
        .filter(|e| e["uuid"].is_string())
        .map(|e| {
            let c = &e["message"]["content"];
            c.as_str()
                .map(str::to_string)
                .unwrap_or_else(|| c[0]["text"].as_str().unwrap().to_string())
        })
        .collect();
    assert_eq!(
        after,
        ["q2", "a2", "q3"],
        "the live rows follow the summary"
    );
    let bytes = std::fs::read(&path).unwrap();
    rig.export();
    assert_eq!(
        std::fs::read(&path).unwrap(),
        bytes,
        "and the summary is only emitted once"
    );
}

#[test]
fn a_rewound_claude_code_session_is_reimported_as_a_new_forge_session() {
    let rig = Rig::new();
    rig.export();
    let path = rig.cc_path();
    let first_user = lines_of(&path)
        .iter()
        .find(|e| e["type"] == "user")
        .unwrap()["uuid"]
        .as_str()
        .unwrap()
        .to_string();
    let branch = json!({"type": "user", "uuid": "rewound", "parentUuid": first_user, "isSidechain": false,
        "timestamp": "2026-10-12T12:00:00.000Z", "message": {"role": "user", "content": "a different path"}});
    let mut f = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap();
    writeln!(f, "{branch}").unwrap();
    drop(f);
    let report = rig.mirror();
    assert_eq!(report.kind, Mirrored::Reimported);
    assert_ne!(report.forge_id, rig.forge);
    assert!(report.warning.unwrap().contains("rewound"));
    let link = rig
        .store
        .cc_link_for_cc(
            &rig.store
                .cc_link_for_forge(&report.forge_id)
                .unwrap()
                .unwrap()
                .cc_session_id,
        )
        .unwrap()
        .unwrap();
    assert_eq!(
        link.forge_session_id, report.forge_id,
        "the link follows the new session"
    );
    assert!(rig.store.cc_link_for_forge(&rig.forge).unwrap().is_none());
    assert_eq!(
        texts(&rig.store, &report.forge_id).last().unwrap().1,
        "a different path"
    );
}

#[test]
fn a_half_written_last_line_is_left_for_the_next_sync() {
    let rig = Rig::new();
    rig.export();
    let path = rig.cc_path();
    let mut f = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap();
    write!(f, "{{\"type\":\"user\",\"uuid\":\"half\",\"parentU").unwrap();
    drop(f);
    assert_eq!(rig.mirror().kind, Mirrored::Unchanged);
    let link = rig.store.cc_link_for_forge(&rig.forge).unwrap().unwrap();
    assert!(
        (link.cc_synced_bytes as u64) < std::fs::metadata(&path).unwrap().len(),
        "the partial line is not consumed"
    );
}

#[test]
fn one_session_links_to_at_most_one_other() {
    let rig = Rig::new();
    rig.export();
    let first = rig.store.cc_link_for_forge(&rig.forge).unwrap().unwrap();
    let other = forge_session(&rig.store);
    add(&rig.store, &other, Role::User, "x");
    let mut stolen = first.clone();
    stolen.forge_session_id = other.clone();
    rig.store.write_cc_link(&stolen).unwrap();
    assert!(rig.store.cc_link_for_forge(&rig.forge).unwrap().is_none());
    assert_eq!(
        rig.store
            .cc_link_for_cc(&first.cc_session_id)
            .unwrap()
            .unwrap()
            .forge_session_id,
        other
    );
}

fn args(session: Option<&str>, to: Option<SwitchTarget>) -> SwitchArgs {
    SwitchArgs {
        session: session.map(String::from),
        to,
        dry_run: false,
        no_launch: false,
        force: false,
    }
}

#[test]
fn naming_a_session_picks_its_tool_and_the_opposite_destination() {
    let rig = Rig::new();
    let projects = rig.projects.path();
    let prefix = &rig.forge[..8];
    let (src, to) = resolve(&rig.store, &args(Some(prefix), None), CWD, projects).unwrap();
    assert_eq!(
        (src, to),
        (
            Source::Forge {
                id: rig.forge.clone()
            },
            SwitchTarget::Claude
        )
    );
    assert!(
        resolve(
            &rig.store,
            &args(Some(prefix), Some(SwitchTarget::Forge)),
            CWD,
            projects
        )
        .is_err(),
        "unlinked: nothing to switch"
    );
    assert!(resolve(&rig.store, &args(Some("nope-nothing"), None), CWD, projects).is_err());

    rig.export();
    let path = rig.cc_path();
    let (src, to) = resolve(&rig.store, &args(path.to_str(), None), CWD, projects).unwrap();
    assert!(matches!(src, Source::Claude { .. }));
    assert_eq!(to, SwitchTarget::Forge);
    // `--to claude` with the Claude Code id of a linked pair means: bring the Forge side across.
    let (src, to) = resolve(
        &rig.store,
        &args(path.to_str(), Some(SwitchTarget::Claude)),
        CWD,
        projects,
    )
    .unwrap();
    assert_eq!(
        (src, to),
        (
            Source::Forge {
                id: rig.forge.clone()
            },
            SwitchTarget::Claude
        )
    );
}

#[test]
fn with_no_argument_the_newer_side_of_this_directory_is_carried_over() {
    let store = Store::open_in_memory().unwrap();
    let projects = tempfile::tempdir().unwrap();
    assert!(resolve(&store, &args(None, None), CWD, projects.path()).is_err());

    let forge = forge_session(&store);
    add(&store, &forge, Role::User, "only forge");
    let (src, _) = resolve(&store, &args(None, None), CWD, projects.path()).unwrap();
    assert_eq!(src, Source::Forge { id: forge.clone() });

    let dir = projects.path().join(cc_fs::slug(CWD));
    std::fs::create_dir_all(&dir).unwrap();
    let cc = dir.join("11111111-2222-4333-8444-555555555555.jsonl");
    std::fs::write(
        &cc,
        r#"{"type":"user","uuid":"u","entrypoint":"cli","message":{"role":"user","content":"hi"}}"#
            .to_string()
            + "\n",
    )
    .unwrap();
    let set_mtime = |secs: i64| {
        let t = if secs >= 0 {
            std::time::SystemTime::now() + std::time::Duration::from_secs(secs as u64)
        } else {
            std::time::SystemTime::now() - std::time::Duration::from_secs((-secs) as u64)
        };
        std::fs::File::options()
            .write(true)
            .open(&cc)
            .unwrap()
            .set_modified(t)
            .unwrap();
    };
    set_mtime(600);
    let (src, to) = resolve(&store, &args(None, None), CWD, projects.path()).unwrap();
    assert!(
        matches!(src, Source::Claude { .. }),
        "Claude Code was used last"
    );
    assert_eq!(to, SwitchTarget::Forge);
    set_mtime(-3600);
    let (src, to) = resolve(&store, &args(None, None), CWD, projects.path()).unwrap();
    assert_eq!(
        (src, to),
        (Source::Forge { id: forge.clone() }, SwitchTarget::Claude),
        "Forge was used last"
    );
    let (src, _) = resolve(
        &store,
        &args(None, Some(SwitchTarget::Forge)),
        CWD,
        projects.path(),
    )
    .unwrap();
    assert!(
        matches!(src, Source::Claude { .. }),
        "--to forge always takes the Claude Code side"
    );
}

#[test]
fn a_linked_pair_goes_the_way_the_new_turns_are() {
    let rig = Rig::new();
    let projects = rig.projects.path();
    rig.export();
    let path = rig.cc_path();
    let pick = || {
        resolve(&rig.store, &args(None, None), CWD, projects)
            .unwrap()
            .0
    };
    // Nothing new on either side: you are where the last switch put you, so move on from there.
    assert!(matches!(pick(), Source::Claude { .. }));
    add(&rig.store, &rig.forge, Role::User, "forge turn");
    assert_eq!(
        pick(),
        Source::Forge {
            id: rig.forge.clone()
        }
    );
    rig.export();
    cc_turn(&path, "claude turn", "answer");
    assert!(matches!(pick(), Source::Claude { .. }));
    // Forge's own system notes are not turns.
    rig.mirror();
    rig.store
        .add_ui_note(
            &rig.forge,
            rig.store.next_message_seq(&rig.forge).unwrap(),
            Role::System,
            "note",
        )
        .unwrap();
    assert!(
        matches!(pick(), Source::Forge { .. }),
        "after mirroring into Forge the last target was Forge"
    );
}

#[test]
fn launch_arguments_carry_mode_and_model_across() {
    let l = Launch::claude(
        "cc-id",
        Some("claude-haiku-5-5"),
        "AcceptEdits",
        "/work/proj",
    );
    assert_eq!(l.program, PathBuf::from("claude"));
    assert_eq!(
        l.args,
        [
            "--resume",
            "cc-id",
            "--model",
            "claude-haiku-5-5",
            "--permission-mode",
            "acceptEdits"
        ]
    );
    assert_eq!(l.dir, Some(PathBuf::from("/work/proj")));
    let l = Launch::claude("cc-id", None, "Bypass", "/w");
    assert_eq!(
        l.args,
        ["--resume", "cc-id", "--dangerously-skip-permissions"]
    );
    assert_eq!(
        Launch::claude("c", None, "Default", "/w").args,
        ["--resume", "c"]
    );
    let f = Launch::forge("fid", Some("bypassPermissions"), "/w").unwrap();
    assert_eq!(f.args, ["chat", "--resume", "fid", "--mode", "bypass"]);
    assert_eq!(
        Launch::forge("fid", Some("default"), "/w").unwrap().args,
        ["chat", "--resume", "fid"]
    );
    assert_eq!(
        Launch::forge("fid", Some("plan"), "/w")
            .unwrap()
            .args
            .last()
            .unwrap(),
        "plan"
    );
}

#[test]
fn a_non_claude_pin_is_noted_and_a_claude_pin_becomes_the_model_flag() {
    let rig = Rig::new();
    let (launch, note) = claude_launch(&rig.store, &rig.forge, "cc").unwrap();
    assert!(launch
        .args
        .windows(2)
        .any(|w| w == ["--model", "claude-haiku-5-5"]));
    assert!(note.is_none());
    rig.store
        .set_session_pinned_model(&rig.forge, Some("opencode::muse"))
        .unwrap();
    let (launch, note) = claude_launch(&rig.store, &rig.forge, "cc").unwrap();
    assert!(!launch.args.contains(&"--model".to_string()));
    assert!(note.unwrap().contains("opencode::muse"));
}
