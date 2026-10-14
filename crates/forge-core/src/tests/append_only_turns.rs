//! Turn-start context through real turns: what changed is appended, nothing already sent is
//! rewritten, and the cache instrument sees it that way.

use super::*;

/// Answers every call with text, reporting a well-cached prompt, and records the tool count of
/// each request.
struct CachedAnswer {
    tools_seen: Arc<Mutex<Vec<usize>>>,
}

#[async_trait::async_trait]
impl Provider for CachedAnswer {
    async fn complete(
        &self,
        _model: &str,
        _messages: &[Message],
        tools: &[ToolSpec],
        _on_event: &mut forge_provider::EventSink<'_>,
    ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
        self.tools_seen.lock().unwrap().push(tools.len());
        Ok(forge_provider::ModelResponse {
            reasoning: String::new(),
            reasoning_items: Vec::new(),
            content: "done".into(),
            tool_calls: vec![],
            usage: forge_types::Usage {
                input_tokens: 10_000,
                output_tokens: 5,
                cached_input_tokens: Some(9_500),
                cost_usd: 0.0,
            },
            quotas: Vec::new(),
        })
    }
}

fn quiet_session(dir: &std::path::Path) -> (Arc<Store>, Session, Arc<Mutex<Vec<usize>>>) {
    let tools_seen = Arc::new(Mutex::new(Vec::new()));
    let provider = Arc::new(CachedAnswer {
        tools_seen: Arc::clone(&tools_seen),
    });
    let router = Arc::new(FixedRouter {
        model: "m::x".into(),
        fallbacks: vec![],
    });
    let (store, mut session) = fixed_session(provider, router);
    session.config.recap.enabled = false;
    session.config.suggest.enabled = false;
    session.config.mesh.auto_memory = false;
    session.config.git.commit_nudge = false;
    session.workspace = WorkspaceContext::new(dir).unwrap();
    session.cached_git_branch = current_git_branch(dir);
    (store, session, tools_seen)
}

fn system_lines<'a>(session: &'a Session, prefix: &str) -> Vec<&'a str> {
    session
        .transcript
        .iter()
        .filter(|m| m.role == Role::System && m.content.starts_with(prefix))
        .map(|m| m.content.as_str())
        .collect()
}

#[tokio::test]
async fn later_turns_append_only_what_changed() {
    let dir = clean_git_repo();
    let (store, mut session, _) = quiet_session(&dir);

    session.run_turn("what does f.txt hold?").await.unwrap();
    session.run_turn("and why?").await.unwrap();

    // The branch is said once, not on every turn, and not in the cache-anchored preamble.
    let branches = system_lines(&session, "[env] git_branch: ");
    assert_eq!(branches.len(), 1, "{branches:?}");
    assert!(session
        .system_preamble()
        .iter()
        .all(|m| !m.content.contains("git_branch")));

    // The long clock instruction is sent once; later turns only carry the time.
    let clocks = system_lines(&session, "[time] ");
    assert_eq!(clocks.len(), 2, "{clocks:?}");
    assert!(clock::is_full_line(clocks[0]));
    assert!(!clock::is_full_line(clocks[1]), "{}", clocks[1]);

    // A branch switch is appended the next turn.
    let status = std::process::Command::new("git")
        .args(["checkout", "-q", "-b", "feature/x"])
        .current_dir(&dir)
        .status()
        .unwrap();
    assert!(status.success());
    session.run_turn("and now?").await.unwrap();
    let branches = system_lines(&session, "[env] git_branch: ");
    assert_eq!(
        branches.last().copied(),
        Some("[env] git_branch: feature/x")
    );
    assert_eq!(branches.len(), 2);

    // Every call after the first reused the previous request whole.
    let stats = store.cache_stats(Some(&session.id), 0).unwrap();
    let model = stats.iter().find(|s| s.model == "m::x").unwrap();
    let labels: std::collections::HashMap<&str, u64> = model
        .labels
        .iter()
        .map(|(label, n)| (label.as_str(), *n))
        .collect();
    assert_eq!(labels.get("cold"), Some(&1), "{labels:?}");
    assert!(
        labels.get("append").copied().unwrap_or(0) >= 2,
        "{labels:?}"
    );
    assert!(!labels.contains_key("harness_rewrite"), "{labels:?}");
    assert_eq!(model.fingerprinted, model.calls);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Compaction can fold away every tool message; a trivial follow-up must not then drop the tools
/// (a full cache miss for the tool list) when the session has already used them.
#[tokio::test]
async fn tools_stay_advertised_after_their_messages_are_gone() {
    let dir = clean_git_repo();
    let (_store, mut session, tools_seen) = quiet_session(&dir);

    session.run_turn("read the file f.txt").await.unwrap();
    session
        .transcript
        .retain(|m| m.role != Role::Tool && m.tool_calls.is_empty());
    session.run_turn("thanks").await.unwrap();

    let seen = tools_seen.lock().unwrap().clone();
    assert!(seen.len() >= 2, "{seen:?}");
    assert!(seen[0] > 0, "the first turn advertised tools: {seen:?}");
    assert_eq!(
        seen.last(),
        seen.first(),
        "the trivial follow-up kept them: {seen:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
