//! Expert on call through real turns: the stall trigger before a nudge or stop, the DONE review
//! sent to the expert, the `consult_expert` tool, the opt-in rules for a subscription expert and
//! the budget that bounds it.

use super::*;
use crate::expert::{consult_expert_spec, meter, status_line, EXPERT_MARKER};
use forge_types::goal_envelope::{EnvelopeCheck, EnvelopeSpec};

const CHEAP: &str = "direct::scripted";
const EXPERT: &str = "anthropic::expert-test";
const CODEX_EXPERT: &str = "codex-cli::expert-test";
const ADVICE: &str = r#"{"diagnosis":"you keep reading lines of big.txt that hold nothing","next_step":"grep for TODO in src/ instead"}"#;

type Plan = Box<dyn Fn(usize) -> Option<(String, Vec<forge_types::ToolCall>)> + Send + Sync>;

struct Scripted {
    plan: Plan,
    expert_replies: Vec<&'static str>,
    main_calls: Arc<std::sync::atomic::AtomicUsize>,
    expert_calls: Arc<Mutex<Vec<(String, String)>>>,
    store: Arc<Store>,
    bump_quota_to: Option<f64>,
}

#[async_trait::async_trait]
impl Provider for Scripted {
    async fn complete(
        &self,
        model: &str,
        messages: &[Message],
        _tools: &[ToolSpec],
        _on_event: &mut forge_provider::EventSink<'_>,
    ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
        use std::sync::atomic::Ordering::SeqCst;
        let system = messages.first().map(|m| m.content.as_str()).unwrap_or("");
        let is_expert =
            system.starts_with(EXPERT_MARKER) || system.starts_with(crate::advisor::ADVISOR_MARKER);
        let (content, tool_calls) = if is_expert {
            let n = {
                let mut calls = self.expert_calls.lock().unwrap();
                calls.push((model.to_string(), system.chars().take(40).collect()));
                calls.len() - 1
            };
            if let Some(pct) = self.bump_quota_to {
                let _ = self.store.record_codex_quota(&forge_types::QuotaHint {
                    provider: "codex-cli".into(),
                    window: "weekly".into(),
                    status: forge_types::QuotaStatus::Ok,
                    resets_at: Some(chrono::Utc::now().timestamp() + 100_000),
                    fraction_used: Some(pct / 100.0),
                });
            }
            let reply = self
                .expert_replies
                .get(n)
                .or_else(|| self.expert_replies.last())
                .copied()
                .unwrap_or(ADVICE);
            (reply.to_string(), Vec::new())
        } else {
            let n = self.main_calls.fetch_add(1, SeqCst);
            (self.plan)(n).unwrap_or(("All done.".to_string(), Vec::new()))
        };
        Ok(forge_provider::ModelResponse {
            reasoning: String::new(),
            reasoning_items: Vec::new(),
            content,
            tool_calls,
            usage: forge_types::Usage {
                input_tokens: if is_expert { 700 } else { 0 },
                ..forge_types::Usage::default()
            },
            quotas: Vec::new(),
        })
    }
}

fn call(name: &str, args: serde_json::Value) -> forge_types::ToolCall {
    forge_types::ToolCall {
        id: forge_types::new_id(),
        name: name.into(),
        args,
    }
}

fn novel_read(n: usize) -> Option<(String, Vec<forge_types::ToolCall>)> {
    Some((
        format!("Reading the next stretch ({n}) to see what else is in there."),
        vec![call(
            "read_file",
            serde_json::json!({"path": "big.txt", "start_line": n + 1, "end_line": n + 1}),
        )],
    ))
}

struct Run {
    session: Session,
    store: Arc<Store>,
    events: Arc<Mutex<Vec<PresenterEvent>>>,
    main_calls: Arc<std::sync::atomic::AtomicUsize>,
    expert_calls: Arc<Mutex<Vec<(String, String)>>>,
    _dir: tempfile::TempDir,
}

fn run(
    plan: Plan,
    expert_replies: Vec<&'static str>,
    bump_quota_to: Option<f64>,
    configure: impl FnOnce(&mut Config),
) -> Run {
    let dir = tempfile::tempdir().unwrap();
    let mut config = Config {
        permission_mode: PermissionMode::Bypass,
        ..Config::default()
    };
    config.recap.enabled = false;
    config.suggest.enabled = false;
    config.mesh.auto_memory = false;
    config.mesh.verify_completeness = false;
    config.shell.explain_errors = false;
    config.mesh.idle_nudge_steps = 6;
    config.mesh.idle_stop_steps = 12;
    config.mesh.context_windows.insert(CHEAP.into(), 1_000_000);
    configure(&mut config);
    let store = Arc::new(Store::open_in_memory().unwrap());
    let main_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let expert_calls: Arc<Mutex<Vec<(String, String)>>> = Arc::default();
    let provider = Scripted {
        plan,
        expert_replies,
        main_calls: main_calls.clone(),
        expert_calls: expert_calls.clone(),
        store: store.clone(),
        bump_quota_to,
    };
    let capture = CapturePresenter {
        attended: false,
        ..Default::default()
    };
    let events = capture.events.clone();
    let session = Session::start(
        store.clone(),
        Arc::new(provider),
        Arc::new(FixedRouter {
            model: CHEAP.into(),
            fallbacks: vec![],
        }),
        ToolRegistry::with_core_tools_in(dir.path()),
        Box::new(capture),
        config,
        dir.path().to_str().unwrap(),
    )
    .unwrap();
    let text: String = (0..400)
        .map(|i| format!("unique line number {i}\n"))
        .collect();
    std::fs::write(dir.path().join("big.txt"), text).unwrap();
    Run {
        session,
        store,
        events,
        main_calls,
        expert_calls,
        _dir: dir,
    }
}

fn with_expert(c: &mut Config) {
    c.harness.expert.model = EXPERT.into();
}

impl Run {
    fn expert_models(&self) -> Vec<String> {
        self.expert_calls
            .lock()
            .unwrap()
            .iter()
            .map(|(m, _)| m.clone())
            .collect()
    }
    fn warnings(&self, needle: &str) -> usize {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter(|e| matches!(e, PresenterEvent::Warning(w) if w.contains(needle)))
            .count()
    }
    fn consult_events(&self) -> Vec<(String, String)> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                PresenterEvent::ExpertConsult {
                    trigger, summary, ..
                } => Some((trigger.clone(), summary.clone())),
                _ => None,
            })
            .collect()
    }
    fn guard_notes(&self) -> Vec<String> {
        self.session
            .transcript
            .iter()
            .filter(|m| m.kind == forge_types::MessageKind::LoopGuard)
            .map(|m| m.content.clone())
            .collect()
    }
    fn nudges(&self) -> Vec<String> {
        self.session
            .transcript
            .iter()
            .filter(|m| m.role == Role::User && m.kind == forge_types::MessageKind::Nudge)
            .map(|m| m.content.clone())
            .collect()
    }
    fn turn_detail(&self) -> serde_json::Value {
        let row = self
            .store
            .last_closed_turn(self.session.session_id())
            .unwrap()
            .unwrap();
        serde_json::from_str(row.detail_json.as_deref().unwrap()).unwrap()
    }
}

#[tokio::test]
async fn no_expert_configured_changes_nothing_about_a_stall() {
    let mut r = run(Box::new(novel_read), vec![], None, |_| {});
    let out = r.session.run_turn("read everything").await.unwrap();
    assert_eq!(out.stop_reason, StopReason::Stalled);
    assert!(r.expert_models().is_empty());
    assert!(r.consult_events().is_empty());
    assert!(r.turn_detail().get("expert_consults").is_none());
}

#[tokio::test]
async fn the_nudge_carries_the_experts_diagnosis_and_the_consult_is_recorded() {
    let mut r = run(Box::new(novel_read), vec![ADVICE], None, |c| {
        with_expert(c);
        c.harness.expert.max_consults_per_turn = 1;
    });
    r.session.run_turn("read everything").await.unwrap();
    assert_eq!(
        r.expert_models(),
        vec![EXPERT.to_string()],
        "the cap is one"
    );
    let notes = r.guard_notes();
    let nudge = notes
        .iter()
        .find(|n| n.contains("Progress check from Forge"))
        .expect("the supervisor's nudge");
    assert!(nudge.contains("Diagnosis: you keep reading lines of big.txt"));
    assert!(nudge.contains("Next step: grep for TODO in src/ instead"));
    let consults = r.consult_events();
    assert_eq!(consults.len(), 1);
    assert_eq!(consults[0].0, "stall");
    let detail = r.turn_detail();
    assert_eq!(detail["expert_consults"], 1);
    assert_eq!(detail["expert_model"], EXPERT);
    assert_eq!(detail["expert_triggers"], serde_json::json!(["stall"]));
}

#[tokio::test]
async fn a_stop_is_deferred_once_when_the_expert_gave_a_next_step() {
    let mut baseline = run(Box::new(novel_read), vec![], None, |_| {});
    baseline.session.run_turn("read everything").await.unwrap();
    let mut r = run(Box::new(novel_read), vec![ADVICE], None, with_expert);
    let out = r.session.run_turn("read everything").await.unwrap();
    assert_eq!(
        out.stop_reason,
        StopReason::Stalled,
        "deferred, not cancelled"
    );
    assert_eq!(
        r.expert_models().len(),
        2,
        "one at the nudge, one at the stop"
    );
    assert!(
        r.main_calls.load(std::sync::atomic::Ordering::SeqCst)
            > baseline
                .main_calls
                .load(std::sync::atomic::Ordering::SeqCst),
        "the model got more steps to try the expert's next step"
    );
    let nudges = r.nudges();
    assert_eq!(nudges.len(), 1);
    assert!(nudges[0].contains("Next step: grep for TODO in src/ instead"));
    assert_eq!(r.warnings("the stop is deferred once"), 1);
    assert_eq!(r.turn_detail()["expert_consults"], 2);
}

#[tokio::test]
async fn an_unparsable_reply_is_recorded_and_the_stop_goes_ahead_on_schedule() {
    let mut baseline = run(Box::new(novel_read), vec![], None, |_| {});
    baseline.session.run_turn("read everything").await.unwrap();
    let mut r = run(
        Box::new(novel_read),
        vec!["I think you should try harder."],
        None,
        with_expert,
    );
    let out = r.session.run_turn("read everything").await.unwrap();
    assert_eq!(out.stop_reason, StopReason::Stalled);
    assert_eq!(
        r.main_calls.load(std::sync::atomic::Ordering::SeqCst),
        baseline
            .main_calls
            .load(std::sync::atomic::Ordering::SeqCst),
        "no next step, no deferral"
    );
    assert!(r
        .consult_events()
        .iter()
        .all(|(_, summary)| summary.contains("no usable reply (unparsed)")));
    assert!(r.nudges().is_empty());
    assert_eq!(r.turn_detail()["expert_consults"], 2);
}

#[tokio::test]
async fn an_expert_that_is_the_turns_own_model_is_never_asked_about_a_stall() {
    let mut r = run(Box::new(novel_read), vec![ADVICE], None, |c| {
        c.harness.expert.model = CHEAP.into();
    });
    r.session.run_turn("read everything").await.unwrap();
    assert!(r.expert_models().is_empty());
}

#[tokio::test]
async fn the_stall_trigger_can_be_switched_off() {
    let mut r = run(Box::new(novel_read), vec![ADVICE], None, |c| {
        with_expert(c);
        c.harness.expert.triggers.stall = false;
    });
    r.session.run_turn("read everything").await.unwrap();
    assert!(r.expert_models().is_empty());
}

#[tokio::test]
async fn a_subscription_expert_without_a_budget_is_refused_and_told_once() {
    let mut r = run(Box::new(novel_read), vec![ADVICE], None, |c| {
        c.harness.expert.model = "codex-oauth::gpt-5.6-sol".into();
    });
    r.session.run_turn("read everything").await.unwrap();
    assert!(r.expert_models().is_empty());
    assert_eq!(
        r.warnings("expert unavailable: codex-oauth::gpt-5.6-sol needs a budget: set harness.expert.budget"),
        1
    );
}

#[tokio::test]
async fn a_spent_budget_refuses_the_next_consult_with_a_note() {
    // The window reads 90% when the first consult is checked; the expert's own call lifts it to
    // 96%, which is past a 5% slice.
    let mut r = run(Box::new(novel_read), vec![ADVICE], Some(96.0), |c| {
        c.harness.expert.model = CODEX_EXPERT.into();
        c.harness.expert.budget = "codex:5%w".into();
    });
    let _ = r.store.record_codex_quota(&forge_types::QuotaHint {
        provider: "codex-cli".into(),
        window: "weekly".into(),
        status: forge_types::QuotaStatus::Ok,
        resets_at: Some(chrono::Utc::now().timestamp() + 100_000),
        fraction_used: Some(0.90),
    });
    r.session.run_turn("read everything").await.unwrap();
    assert_eq!(r.expert_models().len(), 1, "the second consult is refused");
    assert_eq!(
        r.warnings("expert not consulted: the expert budget (codex:5%w) is spent"),
        1
    );
}

#[tokio::test]
async fn the_done_review_goes_to_the_expert_model_without_the_advisor_flag() {
    let blocker = r#"{"verdict":"blocker","notes":[{"severity":"blocker","text":"out.txt was never read back"}],"next_step":"cat out.txt"}"#;
    let plan: Plan = Box::new(|n| match n {
        0 => Some((
            String::new(),
            vec![call(
                "write_file",
                serde_json::json!({"path": "out.txt", "content": "hello"}),
            )],
        )),
        _ => None,
    });
    let mut r = run(plan, vec![blocker], None, with_expert);
    r.session.run_turn("write out.txt").await.unwrap();
    assert_eq!(r.expert_models(), vec![EXPERT.to_string()]);
    assert_eq!(r.nudges().len(), 1);
    assert!(r.nudges()[0].contains("BLOCKER: out.txt was never read back"));
    assert_eq!(r.consult_events()[0].0, "done");
    assert_eq!(
        r.turn_detail()["expert_triggers"],
        serde_json::json!(["done"])
    );
}

#[tokio::test]
async fn the_tool_is_advertised_only_with_a_usable_expert() {
    let r = run(Box::new(novel_read), vec![], None, |_| {});
    assert!(!r
        .session
        .tool_specs()
        .iter()
        .any(|s| s.name == CONSULT_EXPERT_TOOL));
    let r = run(Box::new(novel_read), vec![], None, with_expert);
    assert!(r
        .session
        .tool_specs()
        .iter()
        .any(|s| s.name == CONSULT_EXPERT_TOOL));
    let r = run(Box::new(novel_read), vec![], None, |c| {
        with_expert(c);
        c.harness.expert.triggers.tool = false;
    });
    assert!(!r
        .session
        .tool_specs()
        .iter()
        .any(|s| s.name == CONSULT_EXPERT_TOOL));
    assert_eq!(consult_expert_spec().name, CONSULT_EXPERT_TOOL);
}

#[tokio::test]
async fn the_model_can_ask_the_expert_and_reads_the_answer() {
    let plan: Plan = Box::new(|n| match n {
        0 => Some((
            "I am unsure how to proceed.".into(),
            vec![call(
                CONSULT_EXPERT_TOOL,
                serde_json::json!({"question": "which file holds the parser?"}),
            )],
        )),
        _ => None,
    });
    let mut r = run(plan, vec![ADVICE], None, with_expert);
    r.session.run_turn("find the parser").await.unwrap();
    assert_eq!(r.expert_models(), vec![EXPERT.to_string()]);
    let answer = r
        .session
        .transcript
        .iter()
        .find(|m| {
            m.role == Role::Tool
                && m.content
                    .contains("Expert (anthropic::expert-test) answers.")
        })
        .expect("the tool result");
    assert!(answer
        .content
        .contains("Next step: grep for TODO in src/ instead"));
    assert_eq!(
        r.turn_detail()["expert_triggers"],
        serde_json::json!(["tool"])
    );
}

#[tokio::test]
async fn the_per_turn_cap_refuses_the_tool_after_it_is_spent() {
    let plan: Plan = Box::new(|n| {
        (n < 3).then(|| {
            (
                "Asking again.".into(),
                vec![call(
                    CONSULT_EXPERT_TOOL,
                    serde_json::json!({"question": format!("attempt {n}")}),
                )],
            )
        })
    });
    let mut r = run(plan, vec![ADVICE], None, |c| {
        with_expert(c);
        c.harness.expert.max_consults_per_turn = 1;
    });
    r.session.run_turn("find the parser").await.unwrap();
    assert_eq!(r.expert_models().len(), 1);
    assert!(r.session.transcript.iter().any(|m| m.role == Role::Tool
        && m.content.contains(
            "The expert was not consulted: this turn already used its 1 expert consults"
        )));
}

#[test]
fn the_meter_counts_a_window_slice_and_refuses_when_it_is_spent() {
    let spec = EnvelopeSpec::parse("codex:5%w", None).unwrap();
    let now = 1_000_000;
    let window = |pct: f64| forge_store::SubscriptionWindow {
        provider: "codex-oauth".into(),
        window_kind: "weekly".into(),
        status: "ok".into(),
        resets_at: Some(now + 100_000),
        fraction: Some(pct / 100.0),
        updated_at: now,
    };
    let mut env = None;
    assert_eq!(
        meter(&mut env, &spec, &[window(40.0)], 0.0, now),
        EnvelopeCheck::Within
    );
    assert_eq!(
        meter(&mut env, &spec, &[window(42.1)], 0.0, now),
        EnvelopeCheck::Within
    );
    assert_eq!(
        status_line("codex-oauth::gpt-5.6-sol", 1, env.as_ref()),
        "expert: codex-oauth::gpt-5.6-sol · 1 consult · 2.1% of weekly window"
    );
    assert_eq!(
        meter(&mut env, &spec, &[window(45.2)], 0.0, now),
        EnvelopeCheck::Spent
    );
    let mut blind = None;
    assert_eq!(
        meter(&mut blind, &spec, &[], 0.0, now),
        EnvelopeCheck::Unverified,
        "no reading: the per-turn cap still bounds it"
    );
    assert_eq!(
        status_line("codex-oauth::gpt-5.6-sol", 2, blind.as_ref()),
        "expert: codex-oauth::gpt-5.6-sol · 2 consults · usage unread"
    );
}

#[test]
fn a_dollar_budget_counts_the_experts_own_spend() {
    let spec = EnvelopeSpec::parse("$1", None).unwrap();
    let mut env = None;
    assert_eq!(meter(&mut env, &spec, &[], 0.0, 0), EnvelopeCheck::Within);
    assert_eq!(meter(&mut env, &spec, &[], 0.4, 0), EnvelopeCheck::Within);
    assert_eq!(meter(&mut env, &spec, &[], 1.1, 0), EnvelopeCheck::Spent);
    assert_eq!(
        status_line("anthropic::sonnet", 3, env.as_ref()),
        "expert: anthropic::sonnet · 3 consults · $1.10"
    );
}

#[test]
fn only_a_meterable_subscription_with_a_matching_budget_resolves() {
    use crate::expert::resolve_config;
    let cfg = |model: &str, budget: &str| forge_config::expert::ExpertConfig {
        model: model.into(),
        budget: budget.into(),
        ..Default::default()
    };
    assert!(resolve_config(&cfg("", "")).unwrap().is_none());
    assert!(resolve_config(&cfg("anthropic::sonnet", ""))
        .unwrap()
        .is_some());
    assert!(resolve_config(&cfg("codex-oauth::gpt-5.6-sol", "")).is_err());
    assert!(
        resolve_config(&cfg("codex-oauth::gpt-5.6-sol", "codex:5%w"))
            .unwrap()
            .is_some()
    );
    assert!(resolve_config(&cfg("codex-oauth::gpt-5.6-sol", "5%w"))
        .unwrap()
        .is_some());
    assert!(resolve_config(&cfg("codex-oauth::gpt-5.6-sol", "claude:5%w")).is_err());
    assert!(resolve_config(&cfg("claude-cli::opus", "claude:10%5h"))
        .unwrap()
        .is_some());
    assert!(
        resolve_config(&cfg("kimi::k3", "codex:5%w")).is_err(),
        "no usage meter"
    );
    assert!(resolve_config(&cfg("anthropic::sonnet", "lots")).is_err());
    assert!(resolve_config(&cfg("anthropic::sonnet", "$2"))
        .unwrap()
        .is_some());
}

#[test]
fn a_typo_in_the_provider_or_a_window_budget_on_an_api_model_says_what_to_do() {
    use crate::expert::resolve_config;
    let cfg = |model: &str, budget: &str| forge_config::expert::ExpertConfig {
        model: model.into(),
        budget: budget.into(),
        ..Default::default()
    };
    let typo = resolve_config(&cfg("anthrpic::sonnet", "")).unwrap_err();
    assert!(
        typo.contains("'anthrpic' is not a known provider"),
        "{typo}"
    );
    assert!(typo.contains("forge models"), "{typo}");
    let wrong = resolve_config(&cfg("anthropic::sonnet", "codex:5%w")).unwrap_err();
    assert!(wrong.contains("billed per token"), "{wrong}");
    assert!(wrong.contains("\"$2\""), "{wrong}");
}

#[tokio::test]
async fn an_unusable_expert_is_reported_at_the_first_turn_not_at_the_first_stall() {
    let mut r = run(Box::new(|_| None), vec![], None, |c| {
        c.harness.expert.model = "anthrpic::sonnet".into();
    });
    r.session.run_turn("say hi").await.unwrap();
    assert_eq!(
        r.warnings("expert unavailable: 'anthrpic' is not a known provider"),
        1
    );
    r.session.run_turn("say hi again").await.unwrap();
    assert_eq!(r.warnings("expert unavailable"), 1, "told once per session");
}

#[tokio::test]
async fn privacy_mode_blocking_the_expert_is_said_not_silently_skipped() {
    let mut r = run(Box::new(|_| None), vec![], None, |c| {
        with_expert(c);
        c.privacy.zero_retention = true;
        c.privacy.retains.insert("direct".into(), false);
    });
    r.session.run_turn("say hi").await.unwrap();
    assert_eq!(
        r.warnings("expert unavailable: privacy mode blocks anthropic::expert-test"),
        1
    );
}

#[tokio::test]
async fn a_refused_consult_expert_call_is_shown_to_the_owner_too() {
    let plan: Plan = Box::new(|n| {
        (n < 4).then(|| {
            (
                "Asking again.".into(),
                vec![call(
                    CONSULT_EXPERT_TOOL,
                    serde_json::json!({"question": format!("attempt {n}")}),
                )],
            )
        })
    });
    let mut r = run(plan, vec![ADVICE], None, |c| {
        with_expert(c);
        c.harness.expert.max_consults_per_turn = 2;
    });
    r.session.run_turn("find the parser").await.unwrap();
    assert_eq!(r.expert_models().len(), 2);
    assert_eq!(
        r.warnings("expert not consulted: this turn already used its 2 expert consults"),
        2,
        "attempts three and four"
    );
}

#[tokio::test]
async fn an_expert_done_review_shows_one_line_with_its_severity_and_no_advisor_echo() {
    let blocker = r#"{"verdict":"blocker","notes":[{"severity":"blocker","text":"out.txt was never read back"}],"next_step":"cat out.txt"}"#;
    let plan: Plan = Box::new(|n| match n {
        0 => Some((
            String::new(),
            vec![call(
                "write_file",
                serde_json::json!({"path": "out.txt", "content": "hello"}),
            )],
        )),
        _ => None,
    });
    let mut r = run(plan, vec![blocker], None, with_expert);
    r.session.run_turn("write out.txt").await.unwrap();
    let consults = r.consult_events();
    assert_eq!(consults.len(), 1);
    assert_eq!(
        consults[0].1,
        "blocker: out.txt was never read back Next: cat out.txt"
    );
    assert_eq!(
        r.warnings("advisor:"),
        0,
        "the consult line is the one line"
    );
}

#[tokio::test]
async fn the_consult_line_carries_the_next_step_the_model_was_told() {
    let mut r = run(Box::new(novel_read), vec![ADVICE], None, |c| {
        with_expert(c);
        c.harness.expert.max_consults_per_turn = 1;
    });
    r.session.run_turn("read everything").await.unwrap();
    let consults = r.consult_events();
    assert_eq!(consults.len(), 1);
    assert!(
        consults[0].1.contains("you keep reading lines of big.txt")
            && consults[0]
                .1
                .contains("Next: grep for TODO in src/ instead"),
        "{}",
        consults[0].1
    );
}

#[tokio::test]
async fn a_bridged_models_question_is_answered_from_the_stored_transcript_within_the_cap() {
    let store = Arc::new(Store::open_in_memory().unwrap());
    let id = store.create_session(".", "Bypass").unwrap();
    store
        .add_message(&id, 0, Role::User, "fix the parser in src/parse.rs", None)
        .unwrap();
    let expert_calls: Arc<Mutex<Vec<(String, String)>>> = Arc::default();
    let provider: Arc<dyn Provider> = Arc::new(Scripted {
        plan: Box::new(|_| None),
        expert_replies: vec![ADVICE],
        main_calls: Arc::default(),
        expert_calls: expert_calls.clone(),
        store: store.clone(),
        bump_quota_to: None,
    });
    let mut config = Config::default();
    config.harness.expert.model = EXPERT.into();
    config.harness.expert.max_consults_per_turn = 1;
    let mut bridge = crate::BridgeExpert::default();
    let answer = bridge
        .consult(
            &provider,
            &store,
            &config,
            &id,
            Some(3),
            "where is the bug?",
        )
        .await;
    assert!(
        answer.contains("Next step: grep for TODO in src/ instead"),
        "{answer}"
    );
    assert_eq!(expert_calls.lock().unwrap().len(), 1);
    assert_eq!(
        store
            .count_side_calls_since(&id, "expert/tool:", 0)
            .unwrap(),
        1
    );
    let capped = bridge
        .consult(&provider, &store, &config, &id, Some(3), "again?")
        .await;
    assert!(
        capped.contains("this turn already used its 1 expert consults"),
        "{capped}"
    );
    assert_eq!(expert_calls.lock().unwrap().len(), 1, "no second call");
    let next_turn = bridge
        .consult(&provider, &store, &config, &id, Some(9), "a new turn")
        .await;
    assert!(next_turn.contains("Expert (anthropic::expert-test) answers."));
    config.harness.expert.model = String::new();
    let none = crate::BridgeExpert::default()
        .consult(&provider, &store, &config, &id, Some(1), "q")
        .await;
    assert!(none.contains("No expert is available"));
}

#[test]
fn a_stored_turn_detail_reads_back_as_the_consults_it_recorded() {
    let (n, who) = crate::expert_of_detail(Some(
        r#"{"expert_consults":3,"expert_model":"m","expert_triggers":["stall","stall","tool"]}"#,
    ));
    assert_eq!(n, Some(3));
    assert_eq!(who.as_deref(), Some("m (stall, tool)"));
    assert_eq!(
        crate::expert_of_detail(Some(r#"{"model":"m"}"#)),
        (None, None)
    );
    assert_eq!(crate::expert_of_detail(None), (None, None));
}

#[tokio::test]
async fn an_unreadable_window_allows_only_a_few_consults_per_session() {
    let plan: Plan = Box::new(|n| {
        (n < 5).then(|| {
            (
                "Asking again.".into(),
                vec![call(
                    CONSULT_EXPERT_TOOL,
                    serde_json::json!({"question": format!("attempt {n}")}),
                )],
            )
        })
    });
    let mut r = run(plan, vec![ADVICE], None, |c| {
        c.harness.expert.model = CODEX_EXPERT.into();
        c.harness.expert.budget = "codex:5%w".into();
        c.harness.expert.max_consults_per_turn = 10;
    });
    r.session.run_turn("find the parser").await.unwrap();
    assert_eq!(
        r.expert_models().len(),
        3,
        "no reading: three consults, then refused"
    );
    assert!(r.session.transcript.iter().any(|m| m.role == Role::Tool
        && m.content
            .contains("the expert budget (codex:5%w) cannot be checked")
        && m.content.contains("forge doctor")));
}

#[tokio::test]
async fn consults_a_bridged_model_made_are_folded_in_with_a_line_that_says_what_they_were() {
    let mut r = run(Box::new(|_| None), vec![], None, with_expert);
    r.session.expert.prompt_seq = 0;
    for _ in 0..2 {
        r.store
            .record_side_call_usage_for(
                r.session.session_id(),
                "expert/tool:advice",
                Some(EXPERT),
                &forge_types::Usage::default(),
            )
            .unwrap();
    }
    r.session.fold_bridge_consults();
    let consults = r.consult_events();
    assert_eq!(consults.len(), 1);
    assert_eq!(consults[0].0, "tool");
    assert_eq!(
        consults[0].1,
        "the bridged model asked the expert 2 times; its answers are in that model's tool results"
    );
    assert_eq!(r.session.expert.turn_consults, 2);
}
