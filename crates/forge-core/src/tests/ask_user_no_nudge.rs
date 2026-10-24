//! A turn that asked the user a question ends on the user's answer: the empty-diff nudge must not
//! re-drive it with "implement the fix now" (found driving the desktop app: answering "No" to
//! "rename the function?" was followed by a forced implementation pass).

use super::*;

struct AskThenAcceptProvider {
    calls: std::sync::atomic::AtomicUsize,
}

#[async_trait::async_trait]
impl Provider for AskThenAcceptProvider {
    async fn complete(
        &self,
        _model: &str,
        messages: &[Message],
        _tools: &[ToolSpec],
        _on_event: &mut forge_provider::EventSink<'_>,
    ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
        use forge_provider::ModelResponse;
        use forge_types::{new_id, ToolCall, Usage};
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let answered = messages.iter().any(|m| m.role == Role::Tool);
        Ok(ModelResponse {
            reasoning: String::new(),
            reasoning_items: Vec::new(),
            content: if answered {
                "Understood, leaving it as is.".into()
            } else {
                String::new()
            },
            tool_calls: if answered {
                vec![]
            } else {
                vec![ToolCall {
                    id: new_id(),
                    name: "ask_user".into(),
                    args: serde_json::json!({
                        "question": "Rename add() to sum()?",
                        "options": [{"label": "No"}, {"label": "Yes"}]
                    }),
                }]
            },
            usage: Usage::default(),
            quotas: Vec::new(),
        })
    }
}

#[tokio::test]
async fn a_turn_that_asked_the_user_is_not_nudged_to_implement() {
    let dir = clean_git_repo();
    let provider = Arc::new(AskThenAcceptProvider {
        calls: std::sync::atomic::AtomicUsize::new(0),
    });
    let calls = Arc::clone(&provider);
    let router = Arc::new(FixedRouter {
        model: "m::x".into(),
        fallbacks: vec![],
    });
    let (_store, mut session) = fixed_session(provider, router);
    session.config.recap.enabled = false;
    session.config.suggest.enabled = false;
    session.config.mesh.auto_memory = false;
    session.workspace = WorkspaceContext::new(&dir).unwrap();
    session.set_expect_code_change(true);
    let answer = session.run_turn("fix the naming in calc.py").await.unwrap();
    assert_eq!(answer, "Understood, leaving it as is.");
    assert_eq!(
        calls.calls.load(std::sync::atomic::Ordering::SeqCst),
        2,
        "the question and the reply to the answer, nothing more"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
