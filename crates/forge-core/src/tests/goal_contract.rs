//! The `/goal --contract` flow: drafting, approval, and the independent validator.

use super::*;
use crate::goal_contract::{ContractOutcome, GOAL_CONTRACT_MARKER};
use forge_types::goal_contract::{parse_contract, GoalContract};

const MODEL: &str = "ollama::goal-contract";

/// Answers the drafter with queued replies (the last repeats) and counts every other call.
struct DrafterProvider {
    replies: Mutex<Vec<String>>,
    drafts: Arc<Mutex<Vec<String>>>,
    other_calls: Arc<Mutex<usize>>,
}

#[async_trait::async_trait]
impl Provider for DrafterProvider {
    async fn complete(
        &self,
        _model: &str,
        messages: &[Message],
        _tools: &[ToolSpec],
        _on_event: &mut forge_provider::EventSink<'_>,
    ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
        let content = if messages
            .first()
            .is_some_and(|m| m.content.starts_with(GOAL_CONTRACT_MARKER))
        {
            self.drafts.lock().unwrap().push(
                messages
                    .last()
                    .map(|m| m.content.clone())
                    .unwrap_or_default(),
            );
            let mut replies = self.replies.lock().unwrap();
            if replies.len() > 1 {
                replies.remove(0)
            } else {
                replies[0].clone()
            }
        } else {
            *self.other_calls.lock().unwrap() += 1;
            "done".to_string()
        };
        Ok(forge_provider::ModelResponse {
            reasoning: String::new(),
            reasoning_items: Vec::new(),
            content,
            tool_calls: vec![],
            usage: forge_types::Usage {
                input_tokens: 300,
                output_tokens: 200,
                ..forge_types::Usage::default()
            },
            quotas: Vec::new(),
        })
    }
}

struct Harness {
    session: Session,
    events: Arc<Mutex<Vec<PresenterEvent>>>,
    drafts: Arc<Mutex<Vec<String>>>,
    other_calls: Arc<Mutex<usize>>,
    dir: tempfile::TempDir,
}

fn harness(replies: &[&str], ask_answer: &str, config: Config) -> Harness {
    harness_with(replies, ask_answer, config, true)
}

fn harness_with(replies: &[&str], ask_answer: &str, config: Config, attended: bool) -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let capture = CapturePresenter {
        attended,
        ask_answer: ask_answer.to_string(),
        ..CapturePresenter::default()
    };
    let events = capture.events.clone();
    let drafts: Arc<Mutex<Vec<String>>> = Arc::default();
    let other_calls: Arc<Mutex<usize>> = Arc::default();
    let mut session = Session::start(
        Arc::new(Store::open_in_memory().unwrap()),
        Arc::new(DrafterProvider {
            replies: Mutex::new(replies.iter().map(|r| r.to_string()).collect()),
            drafts: drafts.clone(),
            other_calls: other_calls.clone(),
        }),
        Arc::new(FixedRouter {
            model: MODEL.into(),
            fallbacks: vec![],
        }),
        ToolRegistry::with_core_tools_in(dir.path()),
        Box::new(capture),
        config,
        dir.path().to_str().unwrap(),
    )
    .unwrap();
    session.set_catalog(Some(ModelCatalog::new(vec![MODEL.into()])));
    Harness {
        session,
        events,
        drafts,
        other_calls,
        dir,
    }
}

const GOOD: &str = r#"{"assertions":[
  {"assertion":"notes.txt exists","check":"test -f notes.txt","expect":"exit 0"},
  {"assertion":"notes.txt says ready","check":"cat notes.txt","contains":"ready"},
  {"assertion":"no TODO left","check":"! grep -q TODO notes.txt","expect":"exit 0"}]}"#;

fn contract_of(reply: &str) -> GoalContract {
    let mut c = GoalContract::new(parse_contract(reply).unwrap());
    c.approved = true;
    c
}

#[tokio::test]
async fn an_approved_draft_becomes_the_contract_and_guides_the_worker() {
    let mut h = harness(&[GOOD], "Approve", Config::default());
    h.session.request_goal_contract("write notes.txt");
    h.session.resolve_goal_contract().await;
    let Some(ContractOutcome::Approved(contract)) = h.session.take_goal_contract_outcome() else {
        panic!("expected an approved contract");
    };
    assert_eq!(contract.assertions.len(), 3);
    assert!(contract.approved);
    let events = h.events.lock().unwrap();
    let card = events
        .iter()
        .find_map(|e| match e {
            PresenterEvent::PlanProposed(p) => Some(p.clone()),
            _ => None,
        })
        .expect("a card is shown for approval");
    assert_eq!(card.steps.len(), 3);
    assert!(card.steps[0].detail.contains("test -f notes.txt"));
    assert!(h
        .session
        .transcript
        .iter()
        .any(|m| m.content.contains("validation contract")));
    assert!(h.drafts.lock().unwrap()[0].contains("write notes.txt"));
}

#[tokio::test]
async fn cancel_declines_and_the_turn_never_reaches_the_model() {
    let mut h = harness(&[GOOD], "Cancel", Config::default());
    h.session.request_goal_contract("write notes.txt");
    let outcome = h.session.run_turn_with("go", &[], None).await.unwrap();
    assert!(outcome.text.contains("not approved"), "{}", outcome.text);
    assert_eq!(
        h.session.take_goal_contract_outcome(),
        Some(ContractOutcome::Declined)
    );
    assert_eq!(*h.other_calls.lock().unwrap(), 0, "no worker call was made");
}

#[tokio::test]
async fn typed_changes_redraft_with_the_feedback() {
    let mut h = harness(&[GOOD], "also check the license header", Config::default());
    h.session.request_goal_contract("write notes.txt");
    h.session.resolve_goal_contract().await;
    // The scripted answer never approves, so the revisions run out; each redraft saw the text.
    assert!(matches!(
        h.session.take_goal_contract_outcome(),
        Some(ContractOutcome::Failed(_))
    ));
    let drafts = h.drafts.lock().unwrap();
    assert!(drafts.len() >= 2);
    assert!(drafts[1].contains("also check the license header"));
}

#[tokio::test]
async fn bypass_accepts_without_asking() {
    let mut h = harness(&[GOOD], "Cancel", Config::default());
    h.session.set_temper(PermissionMode::Bypass);
    h.session.request_goal_contract("write notes.txt");
    h.session.resolve_goal_contract().await;
    assert!(matches!(
        h.session.take_goal_contract_outcome(),
        Some(ContractOutcome::Approved(_))
    ));
}

#[tokio::test]
async fn non_read_only_checks_are_redrafted_then_dropped() {
    let bad = r#"{"assertions":[
      {"assertion":"a","check":"test -f a"},
      {"assertion":"b","check":"test -f b"},
      {"assertion":"c","check":"rm -rf build"}]}"#;
    let mut h = harness(&[bad, GOOD], "Approve", Config::default());
    h.session.request_goal_contract("g");
    h.session.resolve_goal_contract().await;
    let Some(ContractOutcome::Approved(contract)) = h.session.take_goal_contract_outcome() else {
        panic!("second draft should have been accepted");
    };
    assert!(contract
        .assertions
        .iter()
        .all(|a| !a.command.contains("rm -rf")));
    assert!(h.drafts.lock().unwrap()[1].contains("rm -rf build"));
}

#[tokio::test]
async fn a_reply_without_a_contract_fails_the_goal_start() {
    let mut h = harness(&["I would rather not."], "Approve", Config::default());
    h.session.request_goal_contract("g");
    h.session.resolve_goal_contract().await;
    assert!(matches!(
        h.session.take_goal_contract_outcome(),
        Some(ContractOutcome::Failed(_))
    ));
}

#[tokio::test]
async fn the_validator_passes_what_holds_and_fails_what_does_not() {
    let mut h = harness(&[GOOD], "Approve", Config::default());
    std::fs::write(h.dir.path().join("notes.txt"), "ready\nTODO later\n").unwrap();
    let contract = contract_of(GOOD);
    let results = h.session.validate_goal_contract(&contract).await;
    assert_eq!(results.len(), 3);
    assert!(results[0].passed);
    assert!(results[1].passed, "{:?}", results[1]);
    assert!(!results[2].passed, "TODO is still in the file");
    assert!(
        results[2].reason.contains("exit 1"),
        "{}",
        results[2].reason
    );
    assert_eq!(
        *h.other_calls.lock().unwrap(),
        0,
        "the validator makes no model call"
    );
}

#[tokio::test]
async fn all_passing_checks_make_the_contract_pass() {
    let mut h = harness(&[GOOD], "Approve", Config::default());
    std::fs::write(h.dir.path().join("notes.txt"), "ready\n").unwrap();
    let mut contract = contract_of(GOOD);
    let results = h.session.validate_goal_contract(&contract).await;
    contract.record(results, 1);
    assert!(contract.all_passed(), "{:?}", contract.results);
}

#[tokio::test]
async fn a_missing_contains_text_fails_with_proof() {
    let mut h = harness(&[GOOD], "Approve", Config::default());
    std::fs::write(h.dir.path().join("notes.txt"), "pending\n").unwrap();
    let results = h.session.validate_goal_contract(&contract_of(GOOD)).await;
    assert!(!results[1].passed);
    assert!(results[1].reason.contains("does not contain \"ready\""));
    assert!(results[1].proof.contains("pending"));
}

#[tokio::test]
async fn a_check_that_outruns_its_timeout_fails_with_the_reason() {
    let mut config = Config::default();
    config.goal.contract_check_timeout_secs = 1;
    let mut h = harness(&[GOOD], "Approve", config);
    std::fs::write(h.dir.path().join("notes.txt"), "x\n").unwrap();
    let slow = r#"{"assertions":[
      {"assertion":"a","check":"tail -f notes.txt"},
      {"assertion":"b","check":"true"},
      {"assertion":"c","check":"true"}]}"#;
    let results = h.session.validate_goal_contract(&contract_of(slow)).await;
    assert!(!results[0].passed);
    assert!(
        results[0].reason.contains("timed out"),
        "{}",
        results[0].reason
    );
    assert!(results[1].passed && results[2].passed);
}

#[tokio::test]
async fn a_command_that_writes_is_refused_and_not_run() {
    let mut h = harness(&[GOOD], "Approve", Config::default());
    let writing = r#"{"assertions":[
      {"assertion":"a","check":"touch created.txt"},
      {"assertion":"b","check":"echo hi > also.txt"},
      {"assertion":"c","check":"true"}]}"#;
    let results = h
        .session
        .validate_goal_contract(&contract_of(writing))
        .await;
    assert!(!results[0].passed && results[0].reason.contains("refused"));
    assert!(!results[1].passed && results[1].reason.contains("refused"));
    assert!(!h.dir.path().join("created.txt").exists());
    assert!(!h.dir.path().join("also.txt").exists());
}

#[tokio::test]
async fn an_unattended_session_accepts_the_contract_it_cannot_ask_about() {
    let mut h = harness_with(&[GOOD], "Cancel", Config::default(), false);
    h.session.request_goal_contract("write notes.txt");
    h.session.resolve_goal_contract().await;
    assert!(matches!(
        h.session.take_goal_contract_outcome(),
        Some(ContractOutcome::Approved(_))
    ));
}
