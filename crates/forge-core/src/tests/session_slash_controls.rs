//! `/context`, `/add-dir` and `/permissions` against a real session.

use super::*;
use forge_types::{PermissionDecision, PermissionMode};

fn session_in(dir: &std::path::Path, config: Config) -> Session {
    Session::start(
        Arc::new(Store::open_in_memory().unwrap()),
        Arc::new(MockProvider),
        Arc::new(HeuristicRouter::new(config.clone())),
        ToolRegistry::with_core_tools_in(dir),
        Box::new(HeadlessPresenter::default()),
        config,
        dir.to_str().unwrap(),
    )
    .unwrap()
}

#[test]
fn the_context_report_counts_each_part_of_the_request() {
    let dir = tempfile::tempdir().unwrap();
    let mut session = session_in(dir.path(), Config::default());
    let empty = session.context_report();
    let tokens = |report: &crate::context_report::ContextReport, label: &str| {
        report
            .categories
            .iter()
            .find(|c| c.label == label)
            .map(|c| (c.tokens, c.items))
            .unwrap()
    };
    assert!(tokens(&empty, "System prompt").0 > 0);
    let (tool_tokens, tool_count) = tokens(&empty, "Tool definitions");
    assert!(tool_tokens > 0 && tool_count > 0);
    assert_eq!(tokens(&empty, "Your messages"), (0, 0));

    session
        .transcript
        .push(Message::user("summarise the parser module"));
    session.transcript.push(Message::assistant("It tokenises."));
    let report = session.context_report();
    let (user_tokens, user_messages) = tokens(&report, "Your messages");
    assert!(user_tokens > 0);
    assert_eq!(user_messages, 1);
    assert_eq!(tokens(&report, "Assistant replies").1, 1);
    assert_eq!(
        report.total(),
        report.categories.iter().map(|c| c.tokens).sum::<usize>()
    );
    assert!(report.total() > empty.total());
}

#[test]
fn add_dir_extends_the_roots_the_file_tools_and_auto_edit_trust() {
    let workspace = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    let config = Config {
        permission_mode: PermissionMode::AcceptEdits,
        tools: forge_config::ToolsConfig {
            confine_to_workspace: true,
            ..Default::default()
        },
        ..Config::default()
    };
    let mut session = session_in(workspace.path(), config);
    let target = other.path().join("notes.md");
    let args = serde_json::json!({ "path": target.to_str().unwrap() });
    assert!(session.writes_beyond_workspace(&args));

    let added = session.add_dir(other.path()).unwrap();
    assert_eq!(added, other.path().canonicalize().unwrap());
    assert!(session
        .added_dirs()
        .contains(&added.to_string_lossy().into_owned()));
    assert!(!session.writes_beyond_workspace(&args));
    // Adding it again changes nothing.
    session.add_dir(other.path()).unwrap();
    assert_eq!(session.added_dirs().len(), 1);

    assert!(session.add_dir(&workspace.path().join("missing")).is_err());
    std::fs::write(workspace.path().join("file.txt"), "x").unwrap();
    assert!(session.add_dir(std::path::Path::new("file.txt")).is_err());
}

#[test]
fn a_session_rule_joins_the_rules_in_force() {
    let dir = tempfile::tempdir().unwrap();
    let mut session = session_in(dir.path(), Config::default());
    let call = serde_json::json!({ "command": "cargo build --release" });
    let decide = |session: &Session| {
        crate::permission::decide(
            PermissionMode::Default,
            forge_types::SideEffect::Shell,
            "shell",
            &call,
            session.permission_rules(),
        )
    };
    assert_eq!(decide(&session), PermissionDecision::Ask);
    let before = session.permission_rules().len();
    session.add_session_rule(
        "shell".into(),
        vec!["cargo build *".into()],
        PermissionDecision::Allow,
    );
    assert_eq!(session.permission_rules().len(), before + 1);
    assert_eq!(decide(&session), PermissionDecision::Allow);
    assert_eq!(
        session.permission_rules().last().unwrap().reason.as_deref(),
        Some(crate::session_slash_controls::SESSION_RULE_REASON)
    );
}
