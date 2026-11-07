//! The `recall` tool through real turns: a session compacted three times, whose model then reads
//! back a decision that every summary folded away.

use super::*;
use crate::compaction_episodes::EPISODES_HEADING;

struct Recaller {
    tool_names: Mutex<Vec<Vec<String>>>,
    recall_results: Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl Provider for Recaller {
    async fn complete(
        &self,
        _model: &str,
        messages: &[Message],
        tools: &[ToolSpec],
        _on_event: &mut forge_provider::EventSink<'_>,
    ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
        self.tool_names
            .lock()
            .unwrap()
            .push(tools.iter().map(|t| t.name.clone()).collect());
        let last = messages
            .iter()
            .rev()
            .find(|m| m.role != Role::System)
            .unwrap();
        let mut response = forge_provider::ModelResponse {
            content: "Noted, carrying on with that.".into(),
            ..Default::default()
        };
        if last.role == Role::Tool {
            self.recall_results
                .lock()
                .unwrap()
                .push(last.content.clone());
            if last.content.contains("folded message(s) match") {
                response.content = String::new();
                response.tool_calls = vec![forge_types::ToolCall {
                    id: forge_types::new_id(),
                    name: "recall".into(),
                    args: serde_json::json!({"action": "read", "from_seq": 0, "to_seq": 3}),
                }];
            }
        } else if last.content.contains("WHAT DID WE DECIDE")
            && tools.iter().any(|t| t.name == "recall")
        {
            response.content = String::new();
            response.tool_calls = vec![forge_types::ToolCall {
                id: forge_types::new_id(),
                name: "recall".into(),
                args: serde_json::json!({"action": "search", "query": "uploader retry policy"}),
            }];
        } else if last.content.contains("Choose the retry policy") {
            response.content = "Settled: exponential backoff capped at 30s.".into();
        }
        Ok(response)
    }

    async fn complete_with(
        &self,
        model: &str,
        messages: &[Message],
        tools: &[ToolSpec],
        _opts: &forge_provider::CompletionOptions,
        on_event: &mut forge_provider::EventSink<'_>,
    ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
        if !messages[0].content.contains("<summary>") {
            return self.complete(model, messages, tools, on_event).await;
        }
        Ok(forge_provider::ModelResponse {
            content: format!(
                "<summary>\n1. Primary request and intent\nupload work\n2. Key technical concepts\n{}\n\
                 3. Files and code\nnone\n</summary>",
                "detail ".repeat(120)
            ),
            ..Default::default()
        })
    }
}

fn recall_session() -> (tempfile::TempDir, Session, Arc<Recaller>) {
    let dir = tempfile::tempdir().unwrap();
    let mut config = Config {
        permission_mode: PermissionMode::Bypass,
        ..Config::default()
    };
    config.recap.enabled = false;
    config.suggest.enabled = false;
    config.mesh.auto_memory = false;
    config.mesh.verify_completeness = false;
    config.git.commit_nudge = false;
    let provider = Arc::new(Recaller {
        tool_names: Mutex::default(),
        recall_results: Mutex::default(),
    });
    let session = Session::start(
        Arc::new(Store::open_in_memory().unwrap()),
        provider.clone(),
        Arc::new(FixedRouter {
            model: "routed::session".into(),
            fallbacks: vec![],
        }),
        ToolRegistry::with_core_tools_in(dir.path()),
        Box::new(CapturePresenter {
            attended: true,
            ..Default::default()
        }),
        config,
        dir.path().to_str().unwrap(),
    )
    .unwrap();
    (dir, session, provider)
}

async fn filler_turns(session: &mut Session, label: &str, count: usize) {
    for n in 0..count {
        session
            .run_turn(&format!("{label} step {n}: keep going"))
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn a_model_recalls_a_decision_that_three_compactions_folded_away() {
    let (_dir, mut session, provider) = recall_session();
    session
        .run_turn("Choose the retry policy for the uploader")
        .await
        .unwrap();
    filler_turns(&mut session, "alpha", 6).await;
    let advertised = |session: &Session| -> Vec<String> {
        session.tool_specs().into_iter().map(|s| s.name).collect()
    };
    let before = advertised(&session);
    assert!(before.contains(&"read_file".to_string()), "{before:?}");
    assert!(
        !before.contains(&"recall".to_string()),
        "a session with no compaction pays nothing for the tool"
    );

    for label in ["alpha", "beta", "gamma"] {
        let (before, after) = session.compact(false).await.unwrap();
        assert!(after < before, "{label}: {before} -> {after}");
        filler_turns(&mut session, label, 6).await;
    }
    assert!(
        !session
            .transcript
            .iter()
            .any(|m| m.content.contains("exponential backoff")),
        "the decision is out of the model's context"
    );

    assert!(advertised(&session).contains(&"recall".to_string()));
    session
        .run_turn("WHAT DID WE DECIDE about the retry policy?")
        .await
        .unwrap();
    let results = provider.recall_results.lock().unwrap().clone();
    assert!(
        results
            .iter()
            .any(|r| r.contains("exponential backoff capped at 30s")),
        "{results:?}"
    );
    assert!(provider
        .tool_names
        .lock()
        .unwrap()
        .last()
        .unwrap()
        .contains(&"recall".to_string()));
}

#[tokio::test]
async fn each_summary_carries_the_episode_index_forward() {
    let (_dir, mut session, _provider) = recall_session();
    session
        .run_turn("Choose the retry policy for the uploader")
        .await
        .unwrap();
    filler_turns(&mut session, "alpha", 6).await;
    session.compact(false).await.unwrap();
    filler_turns(&mut session, "beta", 6).await;
    session.compact(false).await.unwrap();

    let summary = session.transcript[0].content.clone();
    let (_, index) = summary
        .split_once(EPISODES_HEADING)
        .expect("an episode index");
    let lines: Vec<&str> = index.lines().filter(|l| l.starts_with("- seq ")).collect();
    assert!(
        (2..=crate::compaction_episodes::MAX_EPISODE_LINES).contains(&lines.len()),
        "{index}"
    );
    assert!(
        lines[0].starts_with("- seq 0-")
            && lines[0].contains("Choose the retry policy for the uploader"),
        "the first compaction's episode survives the second: {index}"
    );
    assert!(
        index.contains("`recall`"),
        "the index says how to read what it lists"
    );
    assert!(
        summary.find(EPISODES_HEADING)
            < summary.find(crate::compaction_summary::USER_MESSAGES_HEADING),
        "the index sits before the verbatim section so both round-trip"
    );
}
