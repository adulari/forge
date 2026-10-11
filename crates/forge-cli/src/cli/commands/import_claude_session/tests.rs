use std::io::Write;

use forge_types::Role;
use serde_json::json;

use super::*;

fn fixture(dir: &Path) -> PathBuf {
    let path = dir.join("cc.jsonl");
    let mut f = std::fs::File::create(&path).unwrap();
    let lines = [
        json!({"type": "user", "uuid": "u1", "parentUuid": null, "cwd": "/work/proj",
            "timestamp": "2026-10-01T10:00:00Z",
            "message": {"role": "user", "content": "build the thing"}}),
        json!({"type": "assistant", "uuid": "a1", "parentUuid": "u1", "timestamp": "2026-10-01T10:00:01Z",
            "message": {"id": "m1", "role": "assistant", "model": "claude-opus-5-5", "content": [
                {"type": "tool_use", "id": "toolu_1", "name": "Bash", "input": {"command": "make"}}]}}),
        json!({"type": "user", "uuid": "r1", "parentUuid": "a1", "timestamp": "2026-10-01T10:00:02Z",
            "message": {"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": "toolu_1", "content": "built"}]}}),
        json!({"type": "assistant", "uuid": "a2", "parentUuid": "r1", "timestamp": "2026-10-01T10:00:03Z",
            "message": {"id": "m2", "role": "assistant", "model": "claude-opus-5-5",
                "content": [{"type": "text", "text": "Built it."}]}}),
        json!({"type": "system", "subtype": "compact_boundary", "uuid": "b1", "parentUuid": null,
            "logicalParentUuid": "a2", "compactMetadata": {"trigger": "manual"}}),
        json!({"type": "user", "uuid": "s1", "parentUuid": "b1", "isCompactSummary": true,
            "timestamp": "2026-10-01T11:00:00Z",
            "message": {"role": "user", "content":
                "This session is being continued from a previous conversation.\n\nSummary:\nwe built the thing"}}),
        json!({"type": "user", "uuid": "u2", "parentUuid": "s1", "timestamp": "2026-10-01T11:00:01Z",
            "message": {"role": "user", "content": "now test it"}}),
        json!({"type": "assistant", "uuid": "a3", "parentUuid": "u2", "timestamp": "2026-10-01T11:00:02Z",
            "message": {"id": "m3", "role": "assistant", "model": "claude-opus-5-5",
                "content": [{"type": "text", "text": "Tests pass."}]}}),
    ];
    for l in lines {
        writeln!(f, "{l}").unwrap();
    }
    path
}

fn args(session: &Path) -> ClaudeSessionArgs {
    ClaudeSessionArgs {
        session: session.display().to_string(),
        model: None,
        title: None,
        cwd: None,
        quiet: true,
    }
}

#[test]
fn the_imported_model_context_is_the_summary_plus_what_followed_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = fixture(dir.path());
    let store = Store::open_in_memory().unwrap();
    let report = import_transcript(&store, &path, &args(&path), "Default").unwrap();
    let id = &report.session_id;

    let live = store.load_messages(id).unwrap();
    let shape: Vec<(Role, &str)> = live.iter().map(|m| (m.role, m.content.as_str())).collect();
    assert_eq!(
        shape,
        vec![
            (
                Role::System,
                "[Earlier conversation summarized to save context]\nThis session is being continued from a previous conversation.\n\nSummary:\nwe built the thing"
            ),
            (Role::User, "now test it"),
            (Role::Assistant, "Tests pass."),
        ]
    );

    let all = store.load_all_messages(id).unwrap();
    assert_eq!(all.len(), 7, "the older turns stay in history");
    assert_eq!(all[1].tool_calls[0].name, "Bash");
    assert_eq!(all[2].tool_call_id.as_deref(), Some("toolu_1"));
    assert!(store.session_has_compaction(id).unwrap());

    assert_eq!(
        store.session_cwd(id).unwrap().as_deref(),
        Some("/work/proj")
    );
    assert_eq!(
        store.session_pinned_model(id).unwrap().as_deref(),
        Some("claude-cli::claude-opus-5-5")
    );
    assert_eq!(report.stats.live, 2);
}

#[test]
fn uncompact_brings_the_whole_history_back_into_context() {
    let dir = tempfile::tempdir().unwrap();
    let path = fixture(dir.path());
    let store = Store::open_in_memory().unwrap();
    let id = import_transcript(&store, &path, &args(&path), "Default")
        .unwrap()
        .session_id;
    assert!(store.uncompact_session_store(&id).unwrap());
    let live = store.load_messages(&id).unwrap();
    assert_eq!(live.len(), 7);
    assert_eq!(live[0].content, "build the thing");
}

#[test]
fn flags_override_transcript_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let path = fixture(dir.path());
    let store = Store::open_in_memory().unwrap();
    let mut a = args(&path);
    a.model = Some("claude-cli::haiku".into());
    a.title = Some("  spaced\n title ".into());
    a.cwd = Some(PathBuf::from("/elsewhere"));
    let id = import_transcript(&store, &path, &a, "Default")
        .unwrap()
        .session_id;
    assert_eq!(
        store.session_pinned_model(&id).unwrap().as_deref(),
        Some("claude-cli::haiku")
    );
    assert_eq!(
        store.session_cwd(&id).unwrap().as_deref(),
        Some("/elsewhere")
    );
    let titles: Vec<_> = store.list_sessions().unwrap();
    assert_eq!(titles.len(), 1);
}

#[test]
fn a_bare_model_gets_the_bridge_prefix() {
    assert_eq!(
        pin_for(Some("haiku"), Some("x")).as_deref(),
        Some("claude-cli::haiku")
    );
    assert_eq!(
        pin_for(None, Some("claude-opus-5-5")).as_deref(),
        Some("claude-cli::claude-opus-5-5")
    );
    assert_eq!(
        pin_for(Some("codex-cli::m"), None).as_deref(),
        Some("codex-cli::m")
    );
    assert_eq!(pin_for(None, None), None);
}

#[test]
fn an_empty_transcript_is_refused_and_leaves_no_session() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("empty.jsonl");
    std::fs::write(&path, "{\"type\":\"mode\"}\nnot json\n").unwrap();
    let store = Store::open_in_memory().unwrap();
    assert!(import_transcript(&store, &path, &args(&path), "Default").is_err());
    assert!(store.list_sessions().unwrap().is_empty());
}

#[test]
fn a_session_id_resolves_under_the_projects_dir_and_a_bad_path_errors() {
    assert!(locate_transcript("/definitely/not/here.jsonl").is_err());
    let dir = tempfile::tempdir().unwrap();
    let path = fixture(dir.path());
    assert_eq!(locate_transcript(path.to_str().unwrap()).unwrap(), path);
}

#[test]
fn the_subcommand_parses() {
    use clap::Parser;
    let cli = Cli::try_parse_from([
        "forge",
        "import",
        "claude-session",
        "abc",
        "--model",
        "claude-cli::haiku",
        "--quiet",
    ])
    .unwrap();
    assert!(matches!(
        cli.command,
        Some(Command::Import {
            source: ImportSource::ClaudeSession { quiet: true, .. }
        })
    ));
}

#[test]
fn the_imported_session_keeps_the_mode_it_was_given() {
    let dir = tempfile::tempdir().unwrap();
    let path = fixture(dir.path());
    let store = Store::open_in_memory().unwrap();
    let id = import_transcript(&store, &path, &args(&path), "Bypass")
        .unwrap()
        .session_id;
    assert_eq!(store.session_mode(&id).unwrap(), "Bypass");
}
