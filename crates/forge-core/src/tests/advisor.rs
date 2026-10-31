//! `[harness.advisor]` through real turns: one review per turn at DONE, one re-drive, and every
//! failure mode finishing the turn as it would have without the flag.

use super::*;
use crate::advisor::{
    parse_advice, redrive_text, review_input, Advice, Note, Outcome, ADVISOR_MARKER,
    ADVISOR_SYSTEM, MAX_OUTPUT_TOKENS,
};

const MODEL: &str = "direct::scripted";

#[derive(Clone)]
enum Advisor {
    Reply(&'static str),
    Fail,
    Sleep,
}

struct Scripted {
    main: Vec<(String, Vec<forge_types::ToolCall>)>,
    advisor: Advisor,
    main_calls: Arc<std::sync::atomic::AtomicUsize>,
    advisor_calls: Arc<std::sync::atomic::AtomicUsize>,
    requests: Arc<Mutex<Vec<Vec<Message>>>>,
    advisor_usage: forge_types::Usage,
}

#[async_trait::async_trait]
impl Provider for Scripted {
    async fn complete(
        &self,
        _model: &str,
        messages: &[Message],
        _tools: &[ToolSpec],
        _on_event: &mut forge_provider::EventSink<'_>,
    ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
        use std::sync::atomic::Ordering::SeqCst;
        let is_advisor = messages
            .first()
            .is_some_and(|m| m.content.starts_with(ADVISOR_MARKER));
        let (content, tool_calls, usage) = if is_advisor {
            self.advisor_calls.fetch_add(1, SeqCst);
            match self.advisor {
                Advisor::Reply(text) => (text.to_string(), vec![], self.advisor_usage),
                Advisor::Fail => {
                    return Err(forge_provider::ProviderError::Unavailable("503".into()))
                }
                Advisor::Sleep => {
                    tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                    (String::new(), vec![], self.advisor_usage)
                }
            }
        } else {
            let n = self.main_calls.fetch_add(1, SeqCst);
            self.requests.lock().unwrap().push(messages.to_vec());
            let (c, t) = self
                .main
                .get(n)
                .or_else(|| self.main.last())
                .cloned()
                .unwrap_or_default();
            (c, t, forge_types::Usage::default())
        };
        Ok(forge_provider::ModelResponse {
            reasoning: String::new(),
            reasoning_items: Vec::new(),
            content,
            tool_calls,
            usage,
            quotas: Vec::new(),
        })
    }
}

struct Run {
    session: Session,
    store: Arc<Store>,
    events: Arc<Mutex<Vec<PresenterEvent>>>,
    main_calls: Arc<std::sync::atomic::AtomicUsize>,
    advisor_calls: Arc<std::sync::atomic::AtomicUsize>,
    requests: Arc<Mutex<Vec<Vec<Message>>>>,
    _dir: tempfile::TempDir,
}

fn write(path: &str) -> (String, Vec<forge_types::ToolCall>) {
    (
        String::new(),
        vec![forge_types::ToolCall {
            id: forge_types::new_id(),
            name: "write_file".into(),
            args: serde_json::json!({ "path": path, "content": "hello" }),
        }],
    )
}

fn say(text: &str) -> (String, Vec<forge_types::ToolCall>) {
    (text.to_string(), Vec::new())
}

const OK: &str = r#"{"verdict":"ok","notes":[],"next_step":""}"#;
const BLOCKER: &str = r#"Here you go: {"verdict":"blocker","notes":[{"severity":"blocker","text":"out.txt was never read back"}],"next_step":"cat out.txt"}"#;

fn run(
    main: Vec<(String, Vec<forge_types::ToolCall>)>,
    advisor: Advisor,
    configure: impl FnOnce(&mut Config),
    model: &str,
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
    configure(&mut config);
    let main_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let advisor_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let requests: Arc<Mutex<Vec<Vec<Message>>>> = Arc::default();
    let provider = Scripted {
        main,
        advisor,
        main_calls: main_calls.clone(),
        advisor_calls: advisor_calls.clone(),
        requests: requests.clone(),
        advisor_usage: forge_types::Usage {
            input_tokens: 900,
            output_tokens: 80,
            ..forge_types::Usage::default()
        },
    };
    let capture = CapturePresenter {
        attended: true,
        ..Default::default()
    };
    let events = capture.events.clone();
    let store = Arc::new(Store::open_in_memory().unwrap());
    let session = Session::start(
        store.clone(),
        Arc::new(provider),
        Arc::new(FixedRouter {
            model: model.into(),
            fallbacks: vec![],
        }),
        ToolRegistry::with_core_tools_in(dir.path()),
        Box::new(capture),
        config,
        dir.path().to_str().unwrap(),
    )
    .unwrap();
    Run {
        session,
        store,
        events,
        main_calls,
        advisor_calls,
        requests,
        _dir: dir,
    }
}

fn on(c: &mut Config) {
    c.harness.advisor.enabled = true;
}

impl Run {
    fn advisor_calls(&self) -> usize {
        self.advisor_calls.load(std::sync::atomic::Ordering::SeqCst)
    }
    fn main_calls(&self) -> usize {
        self.main_calls.load(std::sync::atomic::Ordering::SeqCst)
    }
    fn warnings(&self, needle: &str) -> usize {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter(|e| matches!(e, PresenterEvent::Warning(w) if w.contains(needle)))
            .count()
    }
    fn progress(&self, needle: &str) -> usize {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter(|e| matches!(e, PresenterEvent::Progress(p) if p.contains(needle)))
            .count()
    }
    /// Input tokens the store attributes to the advisor's side call (the main script reports none).
    fn side_call_input_tokens(&self) -> u64 {
        self.store
            .usage_by_provider_for_session(self.session.session_id())
            .unwrap()
            .iter()
            .map(|u| u.input_tokens)
            .sum()
    }
    fn nudges(&self) -> Vec<String> {
        self.session
            .transcript
            .iter()
            .filter(|m| m.role == Role::User && m.kind == forge_types::MessageKind::Nudge)
            .map(|m| m.content.clone())
            .collect()
    }
}

#[tokio::test]
async fn off_by_default_makes_no_advisor_call() {
    let mut r = run(
        vec![write("out.txt"), say("Done.")],
        Advisor::Reply(BLOCKER),
        |_| {},
        MODEL,
    );
    r.session.run_turn("write out.txt").await.unwrap();
    assert_eq!(r.advisor_calls(), 0);
    assert_eq!(r.main_calls(), 2);
    assert_eq!(r.side_call_input_tokens(), 0);
}

#[tokio::test]
async fn a_blocker_sends_the_turn_back_once() {
    let mut r = run(
        vec![write("out.txt"), say("Done."), say("Checked, fine.")],
        Advisor::Reply(BLOCKER),
        on,
        MODEL,
    );
    r.session.run_turn("write out.txt").await.unwrap();
    assert_eq!(r.advisor_calls(), 1, "the re-drive's final is not reviewed");
    assert_eq!(r.main_calls(), 3);
    let nudges = r.nudges();
    assert_eq!(nudges.len(), 1);
    assert!(nudges[0].contains("BLOCKER: out.txt was never read back"));
    assert!(nudges[0].contains("Suggested check: cat out.txt"));
    let last = r.requests.lock().unwrap().last().unwrap().clone();
    assert!(last
        .iter()
        .any(|m| m.role == Role::User && m.kind == forge_types::MessageKind::Nudge));
    assert_eq!(
        r.side_call_input_tokens(),
        900,
        "the review is priced into the store"
    );
    assert_eq!(r.warnings("advisor: blocker"), 1);
}

#[tokio::test]
async fn an_ok_verdict_finishes_without_a_redrive() {
    let mut r = run(
        vec![write("out.txt"), say("Done.")],
        Advisor::Reply(OK),
        on,
        MODEL,
    );
    r.session.run_turn("write out.txt").await.unwrap();
    assert_eq!((r.advisor_calls(), r.main_calls()), (1, 2));
    assert!(r.nudges().is_empty());
    assert_eq!(r.side_call_input_tokens(), 900);
    assert_eq!(r.progress("advisor: no concerns"), 1);
}

#[tokio::test]
async fn a_turn_that_changed_nothing_is_not_reviewed() {
    let mut r = run(
        vec![say("It is a parser.")],
        Advisor::Reply(BLOCKER),
        on,
        MODEL,
    );
    r.session.run_turn("what is this repo").await.unwrap();
    assert_eq!(r.advisor_calls(), 0);
}

#[tokio::test]
async fn a_bridge_or_subscription_model_is_never_the_reviewer() {
    for model in ["claude-cli::sonnet", "codex-oauth::gpt-5"] {
        let r = run(vec![say("x")], Advisor::Reply(OK), on, model);
        assert_eq!(r.session.advisor_model(model), None, "{model}");
    }
    let r = run(vec![say("x")], Advisor::Reply(OK), on, MODEL);
    assert_eq!(r.session.advisor_model(MODEL).as_deref(), Some(MODEL));
}

#[tokio::test]
async fn a_configured_bridge_model_falls_back_to_the_active_model() {
    let r = run(
        vec![say("x")],
        Advisor::Reply(OK),
        |c| {
            on(c);
            c.harness.advisor.model = "claude-cli::sonnet".into();
        },
        MODEL,
    );
    assert_eq!(r.session.advisor_model(MODEL).as_deref(), Some(MODEL));
}

#[tokio::test]
async fn a_provider_error_publishes_the_original_answer() {
    let mut r = run(
        vec![write("out.txt"), say("Done.")],
        Advisor::Fail,
        on,
        MODEL,
    );
    r.session.run_turn("write out.txt").await.unwrap();
    assert_eq!((r.advisor_calls(), r.main_calls()), (1, 2));
    assert!(r.nudges().is_empty());
    assert_eq!(r.warnings("advisor unavailable"), 1);
    assert!(
        !r.session.provider_readiness().health.is_benched(MODEL),
        "an advisor failure never benches a model"
    );
}

#[tokio::test]
async fn a_timeout_still_completes_the_turn() {
    let mut r = run(
        vec![write("out.txt"), say("Done.")],
        Advisor::Sleep,
        |c| {
            on(c);
            c.harness.advisor.timeout_secs = 1;
        },
        MODEL,
    );
    r.session.run_turn("write out.txt").await.unwrap();
    assert_eq!(r.warnings("advisor unavailable: timeout"), 1);
    assert_eq!(r.main_calls(), 2);
}

#[tokio::test]
async fn an_unparsable_reply_does_not_redrive() {
    let mut r = run(
        vec![write("out.txt"), say("Done.")],
        Advisor::Reply("looks fine to me"),
        on,
        MODEL,
    );
    r.session.run_turn("write out.txt").await.unwrap();
    assert!(r.nudges().is_empty());
    assert_eq!(r.warnings("advisor unavailable: unparsed"), 1);
}

#[tokio::test]
async fn the_next_turn_is_reviewed_again() {
    let mut r = run(
        vec![
            write("a.txt"),
            say("Done."),
            write("b.txt"),
            say("Done again."),
        ],
        Advisor::Reply(OK),
        on,
        MODEL,
    );
    r.session.run_turn("write a.txt").await.unwrap();
    r.session.run_turn("write b.txt").await.unwrap();
    assert_eq!(r.advisor_calls(), 2);
}

#[test]
fn parse_advice_handles_fences_prose_nits_and_garbage() {
    let fenced = format!("```json\n{BLOCKER}\n```");
    let a = parse_advice(&fenced).unwrap();
    assert_eq!(a.notes.len(), 1);
    assert!(a.notes[0].blocker);
    assert_eq!(a.next_step, "cat out.txt");

    let ok_with_notes = r#"{"verdict":"ok","notes":[{"severity":"blocker","text":"x"}]}"#;
    assert!(parse_advice(ok_with_notes).unwrap().notes.is_empty());

    let nits = r#"{"verdict":"concern","notes":[{"severity":"nit","text":"x"},{"severity":"weird","text":"y"}]}"#;
    assert!(parse_advice(nits).unwrap().notes.is_empty());

    let many: String = format!(
        r#"{{"verdict":"concern","notes":[{}]}}"#,
        (0..9)
            .map(|i| format!(
                r#"{{"severity":"concern","text":"n{i} {}"}}"#,
                "x".repeat(400)
            ))
            .collect::<Vec<_>>()
            .join(",")
    );
    let a = parse_advice(&many).unwrap();
    assert_eq!(a.notes.len(), 4);
    assert!(a.notes.iter().all(|n| n.text.chars().count() <= 240));

    assert!(parse_advice("nothing here").is_none());
    assert!(parse_advice(r#"{"verdict":"maybe"}"#).is_none());
}

#[test]
fn review_input_is_bounded_ordered_and_tagged() {
    let mut t = vec![
        Message::user("TASK ".repeat(5_000)),
        Message::new(Role::System, "pinned block").with_kind(forge_types::MessageKind::Pinned),
    ];
    for i in 0..400 {
        t.push(Message::new(
            Role::Tool,
            format!("result {i} {}", "y".repeat(5_000)),
        ));
    }
    t.push(Message::new(Role::Assistant, "final words"));
    let input = review_input(&t, "final words", None);
    assert!(input.len() < 130_000, "{}", input.len());
    assert!(input.contains("earlier messages omitted"));
    assert!(input.contains("result 399") && !input.contains("result 0 "));
    assert!(input.contains("none recorded in this pass"));
    assert!(!input.contains("pinned block"));
    for tag in ["<task>", "<activity>", "<checks>", "<answer>"] {
        assert!(input.contains(tag));
    }
    assert_eq!(
        input.matches("final words").count(),
        1,
        "answer is not duplicated"
    );
    let with = review_input(&t, "final words", Some("cargo test failed"));
    assert!(with.contains("cargo test failed"));
}

#[test]
fn review_input_excludes_earlier_turns() {
    let t = vec![
        Message::user("old task"),
        Message::new(Role::Assistant, "old work"),
        Message::user("new task"),
        Message::new(Role::Assistant, "new work"),
    ];
    let input = review_input(&t, "answer", None);
    assert!(input.contains("new task") && input.contains("new work"));
    assert!(!input.contains("old work"));
}

#[test]
fn labels_and_prompt_constants() {
    assert_eq!(Outcome::Redrive.label(), "advisor/done:redrive");
    assert_eq!(Outcome::Timeout.label(), "advisor/done:timeout");
    assert!(ADVISOR_SYSTEM.starts_with(ADVISOR_MARKER));
    assert_eq!(MAX_OUTPUT_TOKENS, 8_192);
    let text = redrive_text(&Advice {
        notes: vec![Note {
            blocker: false,
            text: "check X".into(),
        }],
        next_step: String::new(),
    });
    assert!(text.contains("CONCERN: check X") && !text.contains("Suggested check"));
}

#[tokio::test]
async fn an_empty_answer_is_never_reviewed() {
    let mut r = run(vec![write("out.txt")], Advisor::Reply(BLOCKER), on, MODEL);
    r.session.run_turn("write out.txt").await.unwrap();
    let mut pre = None;
    let redrive = r
        .session
        .advisor_review_done("  \n", MODEL, 1, None, &mut pre)
        .await;
    assert!(!redrive);
    assert_eq!(
        r.advisor_calls(),
        0,
        "no call for a stop that has no answer"
    );
    assert!(!r.session.advisor_reviewed, "the shot is not spent either");
}

#[tokio::test]
async fn a_benched_active_model_never_reviews() {
    let r = run(vec![say("x")], Advisor::Reply(OK), on, MODEL);
    assert_eq!(r.session.advisor_model(MODEL).as_deref(), Some(MODEL));
    r.store
        .bench_model(
            MODEL,
            chrono::Utc::now().timestamp() + 600,
            "empty response",
        )
        .unwrap();
    assert_eq!(r.session.advisor_model(MODEL), None);
}
