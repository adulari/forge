//! What an invoked skill's frontmatter does to the turn it starts, through real turns: its
//! `allowed-tools` pre-approve a command for that turn only, `disallowed-tools` take a tool away,
//! a model-loaded skill grants mid-turn, and `context: fork` hands the turn to a subagent.

use super::*;
use crate::turn_grant::{ForkTurn, TurnGrant};
use forge_types::{PermissionDecision, RuleSource, ToolCall};

/// Plays one scripted list of tool calls per turn, one call per model step, then answers `done`.
/// A task carrying the fork marker is the `context: fork` child's, and it answers `CHILD_ANSWER`.
struct Script {
    turns: Vec<Vec<ToolCall>>,
    started: std::sync::atomic::AtomicUsize,
}

impl Script {
    fn new(turns: Vec<Vec<(&str, serde_json::Value)>>) -> Self {
        let turns = turns
            .into_iter()
            .map(|calls| {
                calls
                    .into_iter()
                    .map(|(name, args)| ToolCall {
                        id: forge_types::new_id(),
                        name: name.into(),
                        args,
                    })
                    .collect()
            })
            .collect();
        Self {
            turns,
            started: std::sync::atomic::AtomicUsize::new(0),
        }
    }
}

#[async_trait::async_trait]
impl Provider for Script {
    async fn complete(
        &self,
        _model: &str,
        messages: &[Message],
        _tools: &[ToolSpec],
        _on_event: &mut forge_provider::EventSink<'_>,
    ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
        let respond = |content: &str, call: Option<&ToolCall>| forge_provider::ModelResponse {
            content: content.into(),
            tool_calls: call.cloned().into_iter().collect(),
            ..Default::default()
        };
        if messages
            .iter()
            .any(|m| m.role == Role::User && m.content.contains("FORK_BODY_MARKER"))
        {
            return Ok(respond("CHILD_ANSWER", None));
        }
        let steps = messages
            .iter()
            .rev()
            .take_while(|m| m.role != Role::User)
            .filter(|m| m.role == Role::Tool)
            .count();
        let turn = if steps == 0 {
            self.started
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        } else {
            self.started.load(std::sync::atomic::Ordering::SeqCst) - 1
        };
        Ok(match self.turns.get(turn).and_then(|t| t.get(steps)) {
            Some(call) => respond("", Some(call)),
            None => respond("done", None),
        })
    }
}

fn quiet_config() -> Config {
    let mut config = Config {
        permission_mode: PermissionMode::Default,
        ..Config::default()
    };
    config.recap.enabled = false;
    config.suggest.enabled = false;
    config.mesh.auto_memory = false;
    config.mesh.verify_completeness = false;
    config
}

fn rule_config(pattern: &str, decision: PermissionDecision) -> forge_config::RuleConfig {
    let one = || Some(forge_config::OneOrMany::One(pattern.to_string()));
    forge_config::RuleConfig {
        tool: "shell".into(),
        allow: (decision == PermissionDecision::Allow).then(one).flatten(),
        ask: (decision == PermissionDecision::Ask).then(one).flatten(),
        deny: (decision == PermissionDecision::Deny).then(one).flatten(),
        reason: None,
    }
}

fn scratch(name: &str) -> std::path::PathBuf {
    let dir =
        std::env::temp_dir().join(format!("forge-turn-grant-{name}-{}", forge_types::new_id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn session_in(dir: &std::path::Path, provider: Arc<dyn Provider>, config: Config) -> Session {
    Session::start(
        Arc::new(Store::open_in_memory().unwrap()),
        provider,
        Arc::new(FixedRouter {
            model: "claude-cli::opus".into(),
            fallbacks: vec![],
        }),
        ToolRegistry::with_core_tools_in(dir),
        // `confirm` denies, so anything that needs approval fails the test loudly.
        Box::new(CapturePresenter {
            attended: true,
            ..Default::default()
        }),
        config,
        dir.to_str().unwrap(),
    )
    .unwrap()
}

fn allow_mkdir() -> TurnGrant {
    let rules = forge_config::cc_rules::rules_for_spec(
        "Bash(mkdir *)",
        PermissionDecision::Allow,
        RuleSource::Turn,
        "allowed-tools of test",
    );
    TurnGrant {
        label: "test".into(),
        rules,
        ..TurnGrant::default()
    }
}

fn tool_results(session: &Session) -> Vec<String> {
    session
        .transcript
        .iter()
        .filter(|m| m.role == Role::Tool)
        .map(|m| m.content.clone())
        .collect()
}

#[tokio::test]
async fn allowed_tools_pre_approve_a_command_for_one_turn_only() {
    let dir = scratch("allow");
    let provider = Script::new(vec![
        vec![("shell", serde_json::json!({ "command": "mkdir first" }))],
        vec![("shell", serde_json::json!({ "command": "mkdir second" }))],
    ]);
    let mut session = session_in(&dir, Arc::new(provider), quiet_config());

    session.grant_next_turn(allow_mkdir());
    session.run_turn("make a directory").await.unwrap();
    assert!(dir.join("first").is_dir(), "the grant approved the command");
    assert!(
        session
            .permission_rules()
            .iter()
            .all(|r| r.source != RuleSource::Turn),
        "the grant ended with its turn"
    );

    session.run_turn("make another directory").await.unwrap();
    assert!(
        !dir.join("second").exists(),
        "no grant, so approval was needed"
    );
    assert!(
        tool_results(&session)
            .last()
            .is_some_and(|r| r.starts_with("permission denied")),
        "{:?}",
        tool_results(&session)
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn a_configured_deny_still_beats_a_skills_allow() {
    let dir = scratch("deny");
    let provider = Script::new(vec![vec![(
        "shell",
        serde_json::json!({ "command": "mkdir blocked" }),
    )]]);
    let mut config = quiet_config();
    config
        .permissions
        .rules
        .push(rule_config("mkdir *", PermissionDecision::Deny));
    let mut session = session_in(&dir, Arc::new(provider), config);
    session.grant_next_turn(allow_mkdir());
    session.run_turn("make a directory").await.unwrap();
    assert!(!dir.join("blocked").exists());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn turn_rules_sit_ahead_of_configured_ones_so_a_configured_ask_wins_a_tie() {
    let dir = scratch("tie");
    let provider = Script::new(vec![]);
    let mut config = quiet_config();
    config
        .permissions
        .rules
        .push(rule_config("mkdir *", PermissionDecision::Ask));
    let mut session = session_in(&dir, Arc::new(provider), config);
    session.grant_next_turn(allow_mkdir());
    session.begin_turn_grant();
    let args = serde_json::json!({ "command": "mkdir x" });
    let decision = crate::permission::decide(
        PermissionMode::Default,
        SideEffect::Shell,
        "shell",
        &args,
        session.permission_rules(),
    );
    assert_eq!(decision, PermissionDecision::Ask);
    session.end_turn_grant();
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_disallowed_tool_leaves_the_tool_surface_for_that_turn() {
    let dir = scratch("surface");
    let provider = Script::new(vec![]);
    let mut session = session_in(&dir, Arc::new(provider), quiet_config());
    let names = |s: &Session| {
        s.tool_specs()
            .into_iter()
            .map(|t| t.name)
            .collect::<Vec<_>>()
    };
    assert!(names(&session).iter().any(|n| n == "ask_user"));

    session.grant_next_turn(TurnGrant {
        label: "test".into(),
        rules: forge_config::cc_rules::rules_for_spec(
            "AskUserQuestion",
            PermissionDecision::Deny,
            RuleSource::Turn,
            "disallowed-tools of test",
        ),
        ..TurnGrant::default()
    });
    session.begin_turn_grant();
    assert!(!names(&session).iter().any(|n| n == "ask_user"));
    session.end_turn_grant();
    assert!(names(&session).iter().any(|n| n == "ask_user"));
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn a_skill_the_model_loads_grants_its_allowed_tools_for_the_rest_of_the_turn() {
    let dir = scratch("use-skill");
    std::fs::create_dir_all(dir.join("skills/maker")).unwrap();
    std::fs::write(
        dir.join("skills/maker/SKILL.md"),
        "---\nname: maker\ndescription: makes dirs\nallowed-tools: Bash(mkdir *)\n---\nMake what was asked.",
    )
    .unwrap();
    std::fs::create_dir_all(dir.join("skills/hidden")).unwrap();
    std::fs::write(
        dir.join("skills/hidden/SKILL.md"),
        "---\nname: hidden\ndescription: user only\ndisable-model-invocation: true\n---\nSecret.",
    )
    .unwrap();
    let catalog = forge_skills::Catalog::load(&forge_skills::Sources {
        commands: vec![],
        skills: vec![forge_skills::ScopedDir {
            scope: forge_skills::Scope::User,
            path: dir.join("skills"),
        }],
    });
    let provider = Script::new(vec![vec![
        (USE_SKILL_TOOL, serde_json::json!({ "name": "hidden" })),
        (USE_SKILL_TOOL, serde_json::json!({ "name": "maker" })),
        ("shell", serde_json::json!({ "command": "mkdir made" })),
    ]]);
    let mut session = session_in(&dir, Arc::new(provider), quiet_config());
    session.set_skills(Some(Arc::new(catalog)));
    session.run_turn("use the maker skill").await.unwrap();

    let results = tool_results(&session);
    assert!(
        results[0].contains("hidden") && !results[0].contains("Secret."),
        "the model cannot load a disable-model-invocation skill: {results:?}"
    );
    assert!(results[1].contains("Make what was asked."), "{results:?}");
    assert!(
        dir.join("made").is_dir(),
        "the skill's allow covered mkdir: {results:?}"
    );
    assert!(session
        .permission_rules()
        .iter()
        .all(|r| r.source != RuleSource::Turn));
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn context_fork_runs_the_skill_as_a_subagent_and_keeps_its_body_out_of_the_transcript() {
    let dir = scratch("fork");
    let provider = Script::new(vec![]);
    let mut session = session_in(&dir, Arc::new(provider), quiet_config());
    session.grant_next_turn(TurnGrant {
        label: "skill pr".into(),
        fork: Some(ForkTurn {
            agent: "explore".into(),
            model: None,
            label: "/pr 12".into(),
        }),
        ..TurnGrant::default()
    });
    let outcome = session
        .run_turn_with("PR 12", &["FORK_BODY_MARKER: summarize.".into()], None)
        .await
        .unwrap();
    assert_eq!(outcome.text, "CHILD_ANSWER");
    let transcript: Vec<(Role, String)> = session
        .transcript
        .iter()
        .map(|m| (m.role, m.content.clone()))
        .collect();
    assert!(
        transcript.contains(&(Role::User, "/pr 12".into())),
        "{transcript:?}"
    );
    assert!(
        transcript
            .iter()
            .all(|(_, text)| !text.contains("FORK_BODY_MARKER")),
        "the skill body went to the subagent, not this transcript: {transcript:?}"
    );
    assert!(transcript.contains(&(Role::Assistant, "CHILD_ANSWER".into())));
    std::fs::remove_dir_all(&dir).ok();
}

/// Records how many tools each request carried.
struct ToolCount(Arc<Mutex<Vec<usize>>>);

#[async_trait::async_trait]
impl Provider for ToolCount {
    async fn complete(
        &self,
        _model: &str,
        _messages: &[Message],
        tools: &[ToolSpec],
        _on_event: &mut forge_provider::EventSink<'_>,
    ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
        self.0.lock().unwrap().push(tools.len());
        Ok(forge_provider::ModelResponse {
            content: "done".into(),
            ..Default::default()
        })
    }
}

/// `/skill zz` carries the skill's methodology as guidance and only the typed argument as the
/// prompt; classified by "zz" alone the turn is trivial chat and the skill had no tools to use.
#[tokio::test]
async fn a_skill_turn_keeps_its_tools_when_the_typed_argument_is_short() {
    let dir = scratch("skill-tools");
    let counts = Arc::new(Mutex::new(Vec::new()));
    let mut session = session_in(
        &dir,
        Arc::new(ToolCount(Arc::clone(&counts))),
        quiet_config(),
    );
    session.run_turn_with("zz", &[], None).await.unwrap();
    assert_eq!(
        counts.lock().unwrap().last(),
        Some(&0),
        "plain chat stays tool-free"
    );

    let mut session = session_in(
        &dir,
        Arc::new(ToolCount(Arc::clone(&counts))),
        quiet_config(),
    );
    session
        .run_turn_with("zz", &["Run the checks.".into()], None)
        .await
        .unwrap();
    assert!(
        counts.lock().unwrap().last().is_some_and(|n| *n > 0),
        "a skill's turn is offered the tools: {:?}",
        counts.lock().unwrap()
    );
    std::fs::remove_dir_all(&dir).ok();
}
