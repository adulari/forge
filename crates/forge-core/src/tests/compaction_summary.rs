//! Compaction summary quality: the session's own model writes it, a long transcript is summarized
//! in parts, and the user's own words are appended verbatim by the harness.

use super::*;
use crate::compaction_summary::{SUMMARY_PREFIX, USER_MESSAGES_HEADING};
use std::collections::HashSet;

#[derive(Clone, Debug)]
struct Call {
    model: String,
    system: String,
    user: String,
    effort: Option<forge_types::EffortLevel>,
    max_output: Option<u32>,
}

/// Answers every model with a well-formed summary except those listed: `fail` models are rate
/// limited, `hang` models never answer, `unstructured` models answer with prose.
#[derive(Default)]
struct ScriptedProvider {
    calls: Mutex<Vec<Call>>,
    fail: HashSet<String>,
    hang: HashSet<String>,
    unstructured: HashSet<String>,
}

fn reply(n: usize) -> String {
    format!(
        "<analysis>scratch {n}</analysis>\n<summary>\n1. Primary request and intent\nREPLY-{n}\n\
         2. Key technical concepts\n{}\n3. Files and code\nnone\n</summary>",
        "detail ".repeat(120)
    )
}

impl ScriptedProvider {
    fn models(&self) -> Vec<String> {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .map(|c| c.model.clone())
            .collect()
    }
    fn calls(&self) -> Vec<Call> {
        self.calls.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl Provider for ScriptedProvider {
    async fn complete(
        &self,
        _model: &str,
        _messages: &[Message],
        _tools: &[ToolSpec],
        _on_event: &mut forge_provider::EventSink<'_>,
    ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
        Err(forge_provider::ProviderError::Request(
            "complete_with only".into(),
        ))
    }

    async fn complete_with(
        &self,
        model: &str,
        messages: &[Message],
        _tools: &[ToolSpec],
        opts: &forge_provider::CompletionOptions,
        _on_event: &mut forge_provider::EventSink<'_>,
    ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
        let n = {
            let mut calls = self.calls.lock().unwrap();
            calls.push(Call {
                model: model.to_string(),
                system: messages[0].content.clone(),
                user: messages[1].content.clone(),
                effort: opts.effort,
                max_output: opts.max_output_tokens,
            });
            calls.len()
        };
        if self.hang.contains(model) {
            std::future::pending::<()>().await;
        }
        if self.fail.contains(model) {
            return Err(forge_provider::ProviderError::RateLimited {
                message: "429".into(),
                retry_after: Some(std::time::Duration::from_secs(42)),
            });
        }
        let content = if self.unstructured.contains(model) {
            "word ".repeat(400)
        } else {
            reply(n)
        };
        Ok(forge_provider::ModelResponse {
            content,
            ..Default::default()
        })
    }
}

/// A router with a free shortlist, which `FixedRouter` does not have.
struct ShortlistRouter {
    model: String,
    fallbacks: Vec<String>,
    shortlist: Vec<String>,
}

#[async_trait::async_trait]
impl Router for ShortlistRouter {
    async fn route(
        &self,
        _prompt: &str,
        _has_images: bool,
        _budget: BudgetState,
        _health: &forge_types::ModelHealth,
        _quota: &forge_types::SubscriptionQuota,
        _effort: Option<forge_types::EffortLevel>,
        _project: &forge_types::ProjectContext,
    ) -> forge_mesh::RoutingDecision {
        forge_mesh::RoutingDecision {
            effort: None,
            tier: forge_types::TaskTier::Standard,
            model: self.model.clone(),
            rationale: "test".into(),
            fallbacks: self.fallbacks.clone(),
            pinned: false,
            unroutable: false,
        }
    }
    async fn route_with_pin_set(
        &self,
        pin: &[String],
        _p: &str,
        _has_images: bool,
        _b: BudgetState,
        _h: &forge_types::ModelHealth,
        _q: &forge_types::SubscriptionQuota,
        _effort: Option<forge_types::EffortLevel>,
        _project: &forge_types::ProjectContext,
    ) -> forge_mesh::RoutingDecision {
        forge_mesh::RoutingDecision {
            effort: None,
            tier: forge_types::TaskTier::Standard,
            model: pin.first().cloned().unwrap_or_else(|| self.model.clone()),
            rationale: "test".into(),
            fallbacks: self.fallbacks.clone(),
            pinned: true,
            unroutable: false,
        }
    }
    fn compact_candidates(&self) -> Vec<String> {
        self.shortlist.clone()
    }
}

fn names(models: &[&str]) -> Vec<String> {
    models.iter().map(|m| m.to_string()).collect()
}

fn session_with(provider: Arc<ScriptedProvider>, router: Arc<dyn Router>) -> (Arc<Store>, Session) {
    fixed_session(provider, router)
}

fn shortlist_router() -> Arc<ShortlistRouter> {
    Arc::new(ShortlistRouter {
        model: "routed::session".into(),
        fallbacks: names(&["routed::fallback"]),
        shortlist: names(&["free::shortlist"]),
    })
}

/// Everything fails except the free shortlist model, so the order models were tried in is the
/// chain order.
fn failing_all_but_free() -> ScriptedProvider {
    ScriptedProvider {
        fail: ["routed::session", "routed::fallback", "pin::x", "pin::y"]
            .into_iter()
            .map(String::from)
            .collect(),
        ..Default::default()
    }
}

fn push_requests(session: &mut Session, range: std::ops::Range<usize>) {
    for i in range {
        session
            .transcript
            .push(Message::user(format!("request {i}")));
        session
            .transcript
            .push(Message::assistant(format!("done {i}")));
    }
}

#[tokio::test]
async fn the_session_model_summarizes_first_then_its_fallbacks_then_the_free_shortlist() {
    let provider = Arc::new(failing_all_but_free());
    let (_store, mut session) = session_with(provider.clone(), shortlist_router());
    push_requests(&mut session, 0..8);
    session.compact(false).await.unwrap();
    assert_eq!(
        provider.models(),
        ["routed::session", "routed::fallback", "free::shortlist"]
    );
}

#[tokio::test]
async fn a_model_pin_is_the_session_model_and_the_routers_stale_pick_is_not_in_the_chain() {
    let provider = Arc::new(failing_all_but_free());
    let (_store, mut session) = session_with(provider.clone(), shortlist_router());
    session.pin_model(Some("pin::x,pin::y".into()));
    push_requests(&mut session, 0..8);
    session.compact(false).await.unwrap();
    assert_eq!(
        provider.models(),
        ["pin::x", "pin::y", "free::shortlist"],
        "the pin set, then the shortlist; never routed::session"
    );
}

#[tokio::test]
async fn the_model_the_last_turn_ran_on_is_the_session_model() {
    let provider = Arc::new(failing_all_but_free());
    let (_store, mut session) = session_with(provider.clone(), shortlist_router());
    session.route_affinity = Some(SessionAffinity {
        model: "pin::y".into(),
        tier: forge_types::TaskTier::Standard,
        code_heavy: false,
    });
    push_requests(&mut session, 0..8);
    session.compact(false).await.unwrap();
    assert_eq!(provider.models()[0], "pin::y");
}

#[tokio::test]
async fn compact_model_auto_keeps_the_free_shortlist_first() {
    let provider = Arc::new(failing_all_but_free());
    let (_store, mut session) = session_with(provider.clone(), shortlist_router());
    session.config.mesh.compact_model = forge_config::CompactModel::Auto;
    push_requests(&mut session, 0..8);
    session.compact(false).await.unwrap();
    assert_eq!(provider.models(), ["free::shortlist"]);
}

#[tokio::test]
async fn the_summary_runs_at_the_sessions_effort_with_a_20k_output_cap() {
    let provider = Arc::new(ScriptedProvider::default());
    let (store, mut session) = session_with(provider.clone(), shortlist_router());
    store.set_model_context("routed::session", 200_000).unwrap();
    session.set_effort(Some(forge_types::EffortLevel::High));
    push_requests(&mut session, 0..8);
    session.compact(false).await.unwrap();
    let call = &provider.calls()[0];
    assert_eq!(call.effort, Some(forge_types::EffortLevel::High));
    assert_eq!(call.max_output, Some(20_000));
}

#[tokio::test]
async fn the_analysis_is_stripped_before_the_summary_is_stored_and_the_user_words_follow() {
    let provider = Arc::new(ScriptedProvider::default());
    let (store, mut session) = session_with(provider, shortlist_router());
    push_requests(&mut session, 0..8);
    session.compact(false).await.unwrap();
    let summary = &session.transcript[0].content;
    assert!(summary.starts_with(SUMMARY_PREFIX));
    assert!(summary.contains("REPLY-1"), "{summary}");
    assert!(!summary.contains("scratch") && !summary.contains("<analysis>"));
    assert!(!summary.contains("<summary>"));
    assert!(summary.contains(USER_MESSAGES_HEADING));
    assert!(summary.contains("[user message #1]\nrequest 0"));
    let stored = store
        .load_messages(session.id())
        .unwrap()
        .into_iter()
        .next();
    assert!(
        stored.is_none() || !stored.unwrap().content.contains("scratch"),
        "nothing the scratchpad said reaches the store"
    );
}

#[tokio::test]
async fn harness_nudges_never_enter_the_verbatim_section_but_steers_do() {
    let provider = Arc::new(ScriptedProvider::default());
    let (store, mut session) = session_with(provider, shortlist_router());
    let sid = session.id().to_string();
    store
        .add_nudge_message(&sid, 0, "gate says fix lint")
        .unwrap();
    session.transcript.push(Message::user("fix the login bug"));
    session.transcript.push(Message::assistant("on it"));
    session
        .transcript
        .push(Message::user(crate::empty_reply::EMPTY_REPLY_NUDGE));
    session
        .transcript
        .push(Message::user("gate says fix lint").with_kind(forge_types::MessageKind::Nudge));
    session
        .transcript
        .push(Message::user("also keep the v1 route working"));
    push_requests(&mut session, 0..6);
    session.compact(false).await.unwrap();
    let summary = &session.transcript[0].content;
    let (_, block) = summary.split_once(USER_MESSAGES_HEADING).unwrap();
    assert!(block.contains("fix the login bug"));
    assert!(
        block.contains("also keep the v1 route working"),
        "a steer is real input"
    );
    assert!(!block.contains("Your last response was empty"));
    assert!(!block.contains("gate says fix lint"));
}

#[tokio::test]
async fn verbatim_messages_survive_chained_compactions_and_the_summarizer_never_sees_the_old_block()
{
    let provider = Arc::new(ScriptedProvider::default());
    let (_store, mut session) = session_with(provider.clone(), shortlist_router());
    push_requests(&mut session, 0..8);
    session.compact(false).await.unwrap();
    push_requests(&mut session, 8..16);
    session.compact(false).await.unwrap();
    let summary = &session.transcript[0].content;
    let (_, block) = summary.split_once(USER_MESSAGES_HEADING).unwrap();
    for needed in ["request 0", "request 3", "request 9", "request 12"] {
        assert!(block.contains(needed), "{needed} missing from {block}");
    }
    let numbers: Vec<usize> = block
        .lines()
        .filter_map(|l| {
            l.strip_prefix("[user message #")?
                .strip_suffix(']')?
                .parse()
                .ok()
        })
        .collect();
    assert_eq!(numbers.first(), Some(&1));
    assert!(numbers.windows(2).all(|w| w[1] == w[0] + 1), "{numbers:?}");

    let second = provider.calls().pop().unwrap();
    assert!(
        second.user.contains("REPLY-1"),
        "the earlier summary is folded into the new one"
    );
    assert!(
        !second.user.contains(USER_MESSAGES_HEADING),
        "the harness re-appends the block itself"
    );
}

#[tokio::test]
async fn the_summarizer_reads_the_stored_tool_result_not_the_pruned_head() {
    let provider = Arc::new(ScriptedProvider::default());
    let (store, mut session) = session_with(provider.clone(), shortlist_router());
    let sid = session.id().to_string();
    let full = format!(
        "{}NEEDLE_IN_THE_MIDDLE{}",
        "a".repeat(4_000),
        "b".repeat(4_000)
    );
    store
        .add_message_full(&sid, 0, Role::Tool, &full, None, &[], Some("c1"))
        .unwrap();
    session.transcript.push(Message::user("read the file"));
    session.transcript.push(Message::assistant_tool_calls(
        "",
        vec![forge_types::ToolCall {
            id: "c1".into(),
            name: "read_file".into(),
            args: serde_json::json!({"path": "src/lib.rs"}),
        }],
    ));
    session.transcript.push(Message::tool_result(
        "c1",
        format!("{}{}", "a".repeat(1_500), context_pipeline::PRUNE_MARKER),
    ));
    push_requests(&mut session, 0..6);
    session.compact(false).await.unwrap();
    assert!(provider.calls()[0].user.contains("NEEDLE_IN_THE_MIDDLE"));
}

#[tokio::test]
async fn one_giant_tool_result_cannot_dominate_the_summarizer_input() {
    let provider = Arc::new(ScriptedProvider::default());
    let (store, mut session) = session_with(provider.clone(), shortlist_router());
    let sid = session.id().to_string();
    let huge = format!("HEAD{}TAIL", "z".repeat(300_000));
    store
        .add_message_full(&sid, 0, Role::Tool, &huge, None, &[], Some("c1"))
        .unwrap();
    session.transcript.push(Message::user("run it"));
    session.transcript.push(Message::assistant_tool_calls(
        "",
        vec![forge_types::ToolCall {
            id: "c1".into(),
            name: "shell".into(),
            args: serde_json::json!({}),
        }],
    ));
    session
        .transcript
        .push(Message::tool_result("c1", "HEAD…[output trimmed]"));
    push_requests(&mut session, 0..6);
    session.compact(false).await.unwrap();
    let payload = &provider.calls()[0].user;
    assert!(
        payload.chars().count() < 20_000,
        "{}",
        payload.chars().count()
    );
    assert!(payload.contains("HEAD") && payload.contains("TAIL"));
    assert!(payload.contains("omitted from the middle"));
}

#[tokio::test]
async fn an_oversized_transcript_is_summarized_in_parts_with_a_rolling_summary() {
    let provider = Arc::new(ScriptedProvider::default());
    let (store, mut session) = session_with(provider.clone(), shortlist_router());
    store.set_model_context("routed::session", 8_000).unwrap();
    let filler = "alpha bravo charlie delta echo foxtrot ".repeat(60);
    for i in 0..30 {
        session
            .transcript
            .push(Message::user(format!("message {i} {filler}")));
    }
    session.compact(false).await.unwrap();

    let calls = provider.calls();
    assert!(calls.len() >= 3, "{} calls", calls.len());
    let budget = session.compact_input_budget("routed::session");
    let mut last_seen_in = 0usize;
    for i in 0..24 {
        let holders: Vec<usize> = calls
            .iter()
            .enumerate()
            .filter(|(_, c)| c.user.contains(&format!("message {i} ")))
            .map(|(index, _)| index)
            .collect();
        assert!(!holders.is_empty(), "message {i} was never summarized");
        assert!(holders[0] >= last_seen_in, "message {i} came out of order");
        last_seen_in = holders[0];
    }
    for (index, call) in calls.iter().enumerate() {
        assert!(
            tokens::count_message(&call.user) <= budget,
            "part {index} is over the window: {} > {budget}",
            tokens::count_message(&call.user)
        );
        if index == 0 {
            assert!(!call.user.contains("<previous_summary>"));
        } else {
            assert!(
                call.user.contains(&format!("REPLY-{index}")),
                "part {index} must receive the summary of the parts before it"
            );
        }
        assert!(call.system.contains("arrives in parts"));
    }
    let summary = &session.transcript[0].content;
    assert!(summary.contains(&format!("REPLY-{}", calls.len())));
    assert!(
        !summary.contains(&format!("REPLY-{}\n", calls.len() - 1)),
        "only the final rolling summary is kept"
    );
}

#[tokio::test]
async fn a_transcript_that_fits_goes_in_one_call_without_the_parts_wrapper() {
    let provider = Arc::new(ScriptedProvider::default());
    let (_store, mut session) = session_with(provider.clone(), shortlist_router());
    push_requests(&mut session, 0..8);
    session.compact(false).await.unwrap();
    let calls = provider.calls();
    assert_eq!(calls.len(), 1);
    assert!(!calls[0].user.contains("<conversation_part>"));
    assert!(!calls[0].system.contains("arrives in parts"));
}

#[tokio::test]
async fn compact_focus_reaches_the_prompt_once() {
    let provider = Arc::new(ScriptedProvider::default());
    let (_store, mut session) = session_with(provider.clone(), shortlist_router());
    push_requests(&mut session, 0..8);
    session.set_compact_focus("  keep the migration plan and every file path  ");
    session.compact(false).await.unwrap();
    assert!(provider.calls()[0]
        .system
        .contains("keep the migration plan and every file path"));

    push_requests(&mut session, 8..16);
    session.compact(false).await.unwrap();
    assert!(
        !provider.calls()[1].system.contains("migration plan"),
        "a focus applies to the one compaction it was given for"
    );
}

#[tokio::test]
async fn a_call_that_outruns_its_timeout_moves_to_the_next_model_without_benching_it() {
    let provider = Arc::new(ScriptedProvider {
        hang: ["slow::model".to_string()].into_iter().collect(),
        ..Default::default()
    });
    let router = Arc::new(FixedRouter {
        model: "slow::model".into(),
        fallbacks: vec!["good::model".into()],
    });
    let (store, mut session) = session_with(provider.clone(), router);
    session.compact_timeout = std::time::Duration::from_millis(100);
    push_requests(&mut session, 0..8);
    session.compact(false).await.unwrap();
    assert_eq!(provider.models(), ["slow::model", "good::model"]);
    assert!(session.transcript[0].content.contains("REPLY-2"));
    assert!(
        store.current_benched_report().unwrap().is_empty(),
        "a slow summary is not a health verdict on the session's own model"
    );
}

#[tokio::test]
async fn every_model_timing_out_leaves_the_transcript_untouched() {
    let provider = Arc::new(ScriptedProvider {
        hang: ["slow::model".to_string()].into_iter().collect(),
        ..Default::default()
    });
    let router = Arc::new(FixedRouter {
        model: "slow::model".into(),
        fallbacks: vec![],
    });
    let (_store, mut session) = session_with(provider, router);
    session.compact_timeout = std::time::Duration::from_millis(50);
    push_requests(&mut session, 0..8);
    let before = session.transcript.len();
    assert!(session.compact(false).await.is_err());
    assert_eq!(session.transcript.len(), before);
}

#[tokio::test]
async fn an_unstructured_reply_to_a_long_transcript_moves_to_the_next_model() {
    let provider = Arc::new(ScriptedProvider {
        unstructured: ["routed::session".to_string()].into_iter().collect(),
        ..Default::default()
    });
    let (store, mut session) = session_with(provider.clone(), shortlist_router());
    let filler = "alpha bravo charlie delta echo foxtrot ".repeat(60);
    for i in 0..30 {
        session
            .transcript
            .push(Message::user(format!("message {i} {filler}")));
    }
    session.compact(false).await.unwrap();
    assert_eq!(provider.models(), ["routed::session", "routed::fallback"]);
    assert!(session.transcript[0].content.contains("REPLY-2"));
    assert!(store.current_benched_report().unwrap().is_empty());
}
