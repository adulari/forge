//! The fresh-worker hand-off through a real session: the successor row, the predecessor's turn
//! ending `handed_off`, and the brief read from git, the contract and the task list.

use super::*;
use forge_types::goal_contract::{Assertion, AssertionResult, GoalContract};

use crate::goal_relay::{RelayReason, RelayRequest};

struct Quiet;

#[async_trait::async_trait]
impl Provider for Quiet {
    async fn complete(
        &self,
        _model: &str,
        _messages: &[Message],
        _tools: &[ToolSpec],
        _on_event: &mut forge_provider::EventSink<'_>,
    ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
        Ok(forge_provider::ModelResponse {
            content: "done".into(),
            ..Default::default()
        })
    }
}

fn run(dir: &std::path::Path, args: &[&str]) {
    let ok = std::process::Command::new("git")
        .current_dir(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@t")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@t")
        .status()
        .unwrap()
        .success();
    assert!(ok, "git {args:?}");
}

#[tokio::test]
async fn a_hand_off_creates_the_successor_and_ends_the_predecessor_turn() {
    let dir = tempfile::tempdir().unwrap();
    run(dir.path(), &["init", "-q"]);
    std::fs::write(dir.path().join("a.txt"), "one").unwrap();
    run(dir.path(), &["add", "a.txt"]);
    run(dir.path(), &["commit", "-q", "-m", "feat: first slice"]);
    let store = Arc::new(Store::open_in_memory().unwrap());
    let mut config = Config {
        permission_mode: PermissionMode::Bypass,
        ..Config::default()
    };
    config.mesh.auto_memory = false;
    let mut session = Session::start(
        store.clone(),
        Arc::new(Quiet),
        Arc::new(FixedRouter {
            model: "ollama::relay".into(),
            fallbacks: vec![],
        }),
        ToolRegistry::with_core_tools_in(dir.path()),
        Box::new(CapturePresenter::default()),
        config,
        dir.path().to_str().unwrap(),
    )
    .unwrap();
    let id = session.id().to_string();
    store
        .set_session_pinned_model(&id, Some("opencode::muse"))
        .unwrap();
    let turn = store.open_turn(&id, 0).unwrap();
    store.close_turn(turn, "final_answer", None).unwrap();
    store
        .set_tasks(
            &id,
            &[forge_types::TodoItem {
                title: "wire the cli".into(),
                status: forge_types::TodoStatus::InProgress,
                assignee: None,
            }],
        )
        .unwrap();
    let mut contract = GoalContract::new(vec![Assertion {
        id: 1,
        text: "a.txt says two".into(),
        command: "cat a.txt".into(),
        expect: String::new(),
        contains: Some("two".into()),
    }]);
    contract.approved = true;
    contract.record(
        vec![AssertionResult {
            id: 1,
            passed: false,
            proof: "one".into(),
            reason: "output lacks two".into(),
        }],
        1,
    );

    let hand_off = session
        .hand_off_goal(&RelayRequest {
            goal: "ship it".into(),
            started_unix: 0,
            reason: RelayReason::Wedged,
            detail: "assertion 1 still fails".into(),
            contract: Some(contract),
            max_relays: 3,
            envelope: None,
            tokens_used: 10,
            iteration: 4,
        })
        .await
        .unwrap();

    let link = store
        .relay_predecessor(&hand_off.successor_id)
        .unwrap()
        .unwrap();
    assert_eq!(link.predecessor_id, id);
    assert_eq!(link.reason, "wedged");
    assert_eq!(
        store.session_cwd(&hand_off.successor_id).unwrap(),
        store.session_cwd(&id).unwrap()
    );
    assert_eq!(
        store
            .session_pinned_model(&hand_off.successor_id)
            .unwrap()
            .as_deref(),
        Some("opencode::muse")
    );
    assert_eq!(
        store
            .last_closed_turn(&id)
            .unwrap()
            .unwrap()
            .end_reason
            .as_deref(),
        Some("handed_off")
    );
    let brief = &hand_off.brief;
    assert!(brief.contains("feat: first slice"), "{brief}");
    assert!(brief.contains("FAIL 1. a.txt says two"), "{brief}");
    assert!(brief.contains("output lacks two"), "{brief}");
    assert!(brief.contains("wire the cli"), "{brief}");
    assert_eq!(
        store
            .relay_brief(&hand_off.successor_id)
            .unwrap()
            .as_deref(),
        Some(brief.as_str())
    );
}

fn quiet_session(dir: &std::path::Path, store: Arc<Store>) -> Session {
    let mut config = Config {
        permission_mode: PermissionMode::Bypass,
        ..Config::default()
    };
    config.mesh.auto_memory = false;
    Session::start(
        store,
        Arc::new(Quiet),
        Arc::new(FixedRouter {
            model: "ollama::relay".into(),
            fallbacks: vec![],
        }),
        ToolRegistry::with_core_tools_in(dir),
        Box::new(CapturePresenter::default()),
        config,
        dir.to_str().unwrap(),
    )
    .unwrap()
}

#[tokio::test]
async fn a_milestone_hand_off_briefs_the_next_milestone_and_is_not_a_failure_relay() {
    let dir = tempfile::tempdir().unwrap();
    run(dir.path(), &["init", "-q"]);
    let store = Arc::new(Store::open_in_memory().unwrap());
    let mut session = quiet_session(dir.path(), store.clone());
    let id = session.id().to_string();
    let reply = r#"{"milestones":[
      {"title":"lexer","assertions":[{"assertion":"lexer exists","check":"test -f lexer.rs"},{"assertion":"lexer tested","check":"test -f lexer_test.rs"}]},
      {"title":"parser","assertions":[{"assertion":"parser exists","check":"test -f parser.rs"},{"assertion":"parser tested","check":"test -f parser_test.rs"}]}]}"#;
    let mut contract = forge_types::goal_contract::mission::parse_mission(reply, 6)
        .unwrap()
        .into_contract()
        .unwrap();
    contract.approved = true;
    let pass = |id| AssertionResult {
        id,
        passed: true,
        proof: String::new(),
        reason: String::new(),
    };
    contract.record(vec![pass(1), pass(2)], 1);
    assert!(contract.advance_milestone());

    let hand_off = session
        .hand_off_goal(&RelayRequest {
            goal: "ship the parser".into(),
            started_unix: 0,
            reason: RelayReason::Milestone,
            detail: "milestone 1 passed".into(),
            contract: Some(contract),
            max_relays: 3,
            envelope: None,
            tokens_used: 10,
            iteration: 2,
        })
        .await
        .unwrap();

    let brief = &hand_off.brief;
    assert!(
        brief.contains("Mission: you are on milestone 2/2 - parser"),
        "{brief}"
    );
    assert!(
        brief.contains("finished its milestone (milestone 1 passed)"),
        "{brief}"
    );
    assert!(
        brief.contains("(hand-off 1)"),
        "no cap is quoted for a milestone: {brief}"
    );
    assert!(
        brief.contains("Goal contract: milestone 2/2: 0/2 pass"),
        "{brief}"
    );
    assert!(brief.contains("PASS 1. lexer exists"), "{brief}");
    assert!(brief.contains("NOT RUN 3. parser exists"), "{brief}");
    let link = store
        .relay_predecessor(&hand_off.successor_id)
        .unwrap()
        .unwrap();
    assert_eq!(link.reason, "milestone");
    assert_eq!(
        RelayReason::parse("milestone"),
        Some(RelayReason::Milestone)
    );

    // A milestone hand-off does not use up the failure cap; a wedge does.
    let older = store.create_session("/x", "default").unwrap();
    store
        .record_relay(&older, &id, "milestone", "", "")
        .unwrap();
    assert_eq!(session.relay_depth(), 0);
    let oldest = store.create_session("/x", "default").unwrap();
    store
        .record_relay(&oldest, &older, "wedged", "", "")
        .unwrap();
    assert_eq!(session.relay_depth(), 1);
}
