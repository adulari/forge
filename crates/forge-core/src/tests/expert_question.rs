//! The expert answering an `ask_user` question for an unattended session: when it fires, when
//! the human wins, and every case where it must stay out of it.

use super::*;
use crate::expert::EXPERT_MARKER;
use forge_types::{Answer, ExpertOffer, FormWait, QChoice, Question};
use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};

const CHEAP: &str = "direct::scripted";
const EXPERT: &str = "anthropic::expert-test";
const PICK_SQLITE: &str = r#"{"needs_human":false,"reason":"the repo already ships SQLite","answers":[{"selected":["SQLite"]}]}"#;

struct AskingProvider {
    ask_args: serde_json::Value,
    asks: usize,
    expert_reply: &'static str,
    expert_calls: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl Provider for AskingProvider {
    async fn complete(
        &self,
        _model: &str,
        messages: &[Message],
        _tools: &[ToolSpec],
        _on_event: &mut forge_provider::EventSink<'_>,
    ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
        let system = messages.first().map(|m| m.content.as_str()).unwrap_or("");
        let (content, tool_calls) = if system.starts_with(EXPERT_MARKER) {
            self.expert_calls.fetch_add(1, SeqCst);
            (self.expert_reply.to_string(), Vec::new())
        } else if messages.iter().filter(|m| m.role == Role::Tool).count() < self.asks {
            (
                String::new(),
                vec![forge_types::ToolCall {
                    id: forge_types::new_id(),
                    name: "ask_user".into(),
                    args: self.ask_args.clone(),
                }],
            )
        } else {
            ("Carrying on.".to_string(), Vec::new())
        };
        Ok(forge_provider::ModelResponse {
            reasoning: String::new(),
            reasoning_items: Vec::new(),
            content,
            tool_calls,
            usage: forge_types::Usage::default(),
            quotas: Vec::new(),
        })
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Human {
    /// Nobody answers inside the grace period; the form parks.
    Absent,
    /// A person answers inside the grace period.
    AnswersFirst,
    /// A person answers while the expert is still thinking.
    AnswersDuringTheCall,
}

#[derive(Clone)]
struct Surface {
    human: Human,
    events: Arc<Mutex<Vec<PresenterEvent>>>,
    plain_asks: Arc<AtomicUsize>,
    parks: Arc<AtomicUsize>,
    offers: Arc<AtomicUsize>,
    waits: Arc<AtomicUsize>,
}

fn human_answer() -> Vec<Answer> {
    vec![Answer {
        selected: vec!["Postgres".into()],
        ..Answer::default()
    }]
}

impl Presenter for Surface {
    fn emit(&mut self, event: PresenterEvent) {
        self.events.lock().unwrap().push(event);
    }
    fn confirm(&mut self, _: &str, _: forge_types::SideEffect) -> forge_types::ConfirmOutcome {
        forge_types::ConfirmOutcome::Deny
    }
    fn ask(&mut self, _: &str, _: &[QChoice], _: bool) -> String {
        self.plain_asks.fetch_add(1, SeqCst);
        "Postgres".into()
    }
    fn read_line(&mut self) -> Option<String> {
        None
    }
    fn is_attended(&self) -> bool {
        false
    }
    fn ask_form_parking(&mut self, _: &[Question], _: std::time::Duration) -> FormWait {
        self.parks.fetch_add(1, SeqCst);
        match self.human {
            Human::AnswersFirst => FormWait::Answered(human_answer()),
            _ => FormWait::Parked,
        }
    }
    fn offer_expert_answer(&mut self, _: &[Answer], _: &str, _: &str) -> ExpertOffer {
        self.offers.fetch_add(1, SeqCst);
        match self.human {
            Human::AnswersDuringTheCall => ExpertOffer::Human(human_answer()),
            _ => ExpertOffer::Applied,
        }
    }
    fn await_parked_form(&mut self) -> Vec<Answer> {
        self.waits.fetch_add(1, SeqCst);
        human_answer()
    }
}

struct Run {
    session: Session,
    surface: Surface,
    store: Arc<Store>,
    expert_calls: Arc<AtomicUsize>,
    _dir: tempfile::TempDir,
}

fn question_args(text: &str) -> serde_json::Value {
    serde_json::json!({"questions": [{
        "question": text,
        "options": [{"label": "Postgres"}, {"label": "SQLite"}]
    }]})
}

fn run(
    human: Human,
    ask_args: serde_json::Value,
    asks: usize,
    expert_reply: &'static str,
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
    config.mesh.context_windows.insert(CHEAP.into(), 1_000_000);
    config.harness.expert.model = EXPERT.into();
    config.harness.expert.answer_questions = forge_config::expert::AnswerQuestions::Unattended;
    config.harness.expert.answer_after_secs = 5;
    configure(&mut config);
    let store = Arc::new(Store::open_in_memory().unwrap());
    let expert_calls = Arc::new(AtomicUsize::new(0));
    let provider = AskingProvider {
        ask_args,
        asks,
        expert_reply,
        expert_calls: expert_calls.clone(),
    };
    let surface = Surface {
        human,
        events: Arc::default(),
        plain_asks: Arc::default(),
        parks: Arc::default(),
        offers: Arc::default(),
        waits: Arc::default(),
    };
    let session = Session::start(
        store.clone(),
        Arc::new(provider),
        Arc::new(FixedRouter {
            model: CHEAP.into(),
            fallbacks: vec![],
        }),
        ToolRegistry::with_core_tools_in(dir.path()),
        Box::new(surface.clone()),
        config,
        dir.path().to_str().unwrap(),
    )
    .unwrap();
    Run {
        session,
        surface,
        store,
        expert_calls,
        _dir: dir,
    }
}

impl Run {
    fn tool_results(&self) -> Vec<String> {
        self.store
            .load_messages(self.session.session_id())
            .unwrap()
            .into_iter()
            .filter(|m| m.role == Role::Tool)
            .map(|m| m.content)
            .collect()
    }
    fn consults(&self) -> Vec<(String, String)> {
        self.surface
            .events
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
    fn count(&self, counter: &Arc<AtomicUsize>) -> usize {
        counter.load(SeqCst)
    }
}

#[tokio::test]
async fn an_unattended_unanswered_question_is_answered_by_the_expert_and_says_so() {
    let mut r = run(
        Human::Absent,
        question_args("Which database should the cache use?"),
        1,
        PICK_SQLITE,
        |_| {},
    );
    r.session.run_turn("add a cache").await.unwrap();
    assert_eq!(r.count(&r.expert_calls), 1);
    assert_eq!(r.count(&r.surface.offers), 1);
    let result = r.tool_results().remove(0);
    assert!(result.starts_with("SQLite"), "{result}");
    assert!(
        result.contains(
            "Answered by expert (anthropic::expert-test): SQLite — the repo already ships SQLite"
        ),
        "{result}"
    );
    let consults = r.consults();
    assert_eq!(consults.len(), 1);
    assert_eq!(consults[0].0, "question");
    assert!(consults[0]
        .1
        .starts_with("Answered by expert (anthropic::expert-test): SQLite"));
    let row = r
        .store
        .last_closed_turn(r.session.session_id())
        .unwrap()
        .unwrap();
    let detail: serde_json::Value =
        serde_json::from_str(row.detail_json.as_deref().unwrap()).unwrap();
    assert_eq!(detail["expert_triggers"], serde_json::json!(["question"]));
}

#[tokio::test]
async fn a_human_who_answers_inside_the_grace_period_means_the_expert_is_never_asked() {
    let mut r = run(
        Human::AnswersFirst,
        question_args("Which database should the cache use?"),
        1,
        PICK_SQLITE,
        |_| {},
    );
    r.session.run_turn("add a cache").await.unwrap();
    assert_eq!(r.count(&r.expert_calls), 0);
    assert_eq!(r.tool_results(), vec!["Postgres".to_string()]);
}

#[tokio::test]
async fn an_expert_answer_that_lands_after_the_human_answered_is_dropped() {
    let mut r = run(
        Human::AnswersDuringTheCall,
        question_args("Which database should the cache use?"),
        1,
        PICK_SQLITE,
        |_| {},
    );
    r.session.run_turn("add a cache").await.unwrap();
    assert_eq!(r.count(&r.expert_calls), 1);
    assert_eq!(r.tool_results(), vec!["Postgres".to_string()]);
    assert!(r.consults()[0].1.contains("dropped"));
}

#[tokio::test]
async fn nothing_changes_while_the_setting_is_off() {
    let mut r = run(
        Human::Absent,
        question_args("Which database should the cache use?"),
        1,
        PICK_SQLITE,
        |c| c.harness.expert.answer_questions = forge_config::expert::AnswerQuestions::Off,
    );
    r.session.run_turn("add a cache").await.unwrap();
    assert_eq!(r.count(&r.surface.parks), 0);
    assert_eq!(r.count(&r.surface.plain_asks), 1);
    assert_eq!(r.count(&r.expert_calls), 0);
}

#[tokio::test]
async fn no_expert_model_means_no_parking() {
    let mut r = run(
        Human::Absent,
        question_args("Which database should the cache use?"),
        1,
        PICK_SQLITE,
        |c| c.harness.expert.model = String::new(),
    );
    r.session.run_turn("add a cache").await.unwrap();
    assert_eq!(r.count(&r.surface.parks), 0);
    assert_eq!(r.count(&r.expert_calls), 0);
}

#[tokio::test]
async fn a_question_marked_requires_human_is_never_delegated() {
    let args = serde_json::json!({"questions": [{
        "question": "Which database should the cache use?",
        "options": [{"label": "Postgres"}, {"label": "SQLite"}],
        "requires_human": true
    }]});
    let mut r = run(Human::Absent, args, 1, PICK_SQLITE, |_| {});
    r.session.run_turn("add a cache").await.unwrap();
    assert_eq!(r.count(&r.surface.parks), 0);
    assert_eq!(r.count(&r.expert_calls), 0);
}

#[tokio::test]
async fn a_question_about_money_or_destruction_is_never_delegated() {
    let mut r = run(
        Human::Absent,
        question_args("Delete the production database and recreate it?"),
        1,
        PICK_SQLITE,
        |_| {},
    );
    r.session.run_turn("clean up").await.unwrap();
    assert_eq!(r.count(&r.surface.parks), 0);
    assert_eq!(r.count(&r.expert_calls), 0);
}

#[tokio::test]
async fn an_expert_that_says_the_human_must_decide_leaves_the_question_waiting() {
    let mut r = run(
        Human::Absent,
        question_args("Which database should the cache use?"),
        1,
        r#"{"needs_human":true,"reason":"a matter of taste","answers":[]}"#,
        |_| {},
    );
    r.session.run_turn("add a cache").await.unwrap();
    assert_eq!(r.count(&r.expert_calls), 1);
    assert_eq!(r.count(&r.surface.offers), 0);
    assert_eq!(r.count(&r.surface.waits), 1);
    assert_eq!(r.tool_results(), vec!["Postgres".to_string()]);
}

#[tokio::test]
async fn an_exhausted_budget_falls_back_to_waiting_for_the_human() {
    let mut r = run(
        Human::Absent,
        question_args("Which database should the cache use?"),
        2,
        PICK_SQLITE,
        |c| c.harness.expert.max_consults_per_turn = 1,
    );
    r.session.run_turn("add a cache").await.unwrap();
    assert_eq!(
        r.count(&r.expert_calls),
        1,
        "the second question is not asked"
    );
    assert_eq!(r.count(&r.surface.offers), 1);
    assert_eq!(r.count(&r.surface.waits), 1);
    let results = r.tool_results();
    assert!(results[0].contains("Answered by expert"));
    assert_eq!(results[1], "Postgres");
}
