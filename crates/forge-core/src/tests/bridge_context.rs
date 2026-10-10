//! A CLI bridge runs its own tool loop, so its context grows with every file read and build log
//! while Forge's transcript (what compaction measures) stays small.
//!
//! Observed 2026-10-10 in a 40-turn `claude-cli::haiku` daemon session: claude's own window held
//! 650k tokens per request by turn 19 while Forge's estimate read 18k, so Forge never compacted and
//! every request re-read the whole history.

use super::*;

/// Answers every call with a short prose reply and reports `fill` as the bridge's live context.
struct ReportingBridge {
    fill: Arc<std::sync::atomic::AtomicU64>,
}

#[async_trait::async_trait]
impl Provider for ReportingBridge {
    async fn complete(
        &self,
        _model: &str,
        _messages: &[Message],
        _tools: &[ToolSpec],
        _on_event: &mut forge_provider::EventSink<'_>,
    ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
        Ok(forge_provider::ModelResponse {
            reasoning: String::new(),
            reasoning_items: Vec::new(),
            content: "Understood; here is a reply that is long enough to count as an answer."
                .to_string(),
            tool_calls: Vec::new(),
            usage: forge_types::Usage::default(),
            quotas: Vec::new(),
        })
    }

    fn context_fill(&self, _model: &str, _owner: &str) -> Option<u64> {
        Some(self.fill.load(std::sync::atomic::Ordering::SeqCst))
    }
}

fn session(fill: &Arc<std::sync::atomic::AtomicU64>) -> Session {
    let mut config = Config::default();
    config.recap.enabled = false;
    config.suggest.enabled = false;
    config.mesh.auto_memory = false;
    Session::start(
        Arc::new(Store::open_in_memory().unwrap()),
        Arc::new(ReportingBridge {
            fill: Arc::clone(fill),
        }),
        Arc::new(FixedRouter {
            model: "claude-cli::opus".into(),
            fallbacks: vec![],
        }),
        ToolRegistry::with_core_tools_in(test_workspace()),
        Box::new(CapturePresenter {
            attended: true,
            ..Default::default()
        }),
        config,
        test_workspace().to_str().expect("workspace path is UTF-8"),
    )
    .unwrap()
}

fn summarized(session: &Session) -> bool {
    session
        .transcript
        .iter()
        .any(|m| m.content.starts_with("[Earlier conversation summarized"))
}

#[tokio::test]
async fn a_bridge_whose_own_context_is_huge_triggers_compaction_despite_a_small_transcript() {
    let fill = Arc::new(std::sync::atomic::AtomicU64::new(10_000));
    let mut session = session(&fill);
    for n in 0..6 {
        session
            .run_turn(&format!("question number {n}"))
            .await
            .unwrap();
    }
    assert!(
        !summarized(&session),
        "a small bridge context must not compact"
    );
    assert!(session.estimated_transcript_tokens() < 5_000);

    fill.store(900_000, std::sync::atomic::Ordering::SeqCst);
    // The bridge reports its fill with each reply, so compaction acts on it at the next check.
    session.run_turn("one more question").await.unwrap();
    session.run_turn("and another").await.unwrap();

    assert!(
        summarized(&session),
        "the bridge held 900k tokens but Forge only counted its own ~{} and never compacted",
        session.estimated_transcript_tokens()
    );
}

#[tokio::test]
async fn the_gauge_reports_the_bridge_context_not_the_transcript_estimate() {
    let fill = Arc::new(std::sync::atomic::AtomicU64::new(150_000));
    let mut session = session(&fill);
    session.run_turn("a question").await.unwrap();
    assert_eq!(session.context_pressure_tokens(), 150_000);
}
