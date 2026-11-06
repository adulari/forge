//! Approvals learn: a command shape allowed three times in a project, and never refused, is
//! offered as a project rule at the next prompt; accepting writes it through the always-allow
//! plumbing and ends the prompts.

use super::*;
use forge_types::{ConfirmOutcome, ConfirmReply, RuleSuggestion};

#[derive(Clone, Default)]
struct Answering {
    replies: Arc<Mutex<std::collections::VecDeque<ConfirmReply>>>,
    asked: Arc<Mutex<usize>>,
    offers: Arc<Mutex<Vec<RuleSuggestion>>>,
}

impl Answering {
    fn new(replies: Vec<ConfirmReply>) -> Self {
        Self {
            replies: Arc::new(Mutex::new(replies.into())),
            ..Self::default()
        }
    }
    fn asked(&self) -> usize {
        *self.asked.lock().unwrap()
    }
    fn offers(&self) -> Vec<RuleSuggestion> {
        self.offers.lock().unwrap().clone()
    }
}

impl Presenter for Answering {
    fn emit(&mut self, _event: PresenterEvent) {}
    fn confirm(&mut self, tool: &str, side_effect: SideEffect) -> ConfirmOutcome {
        self.confirm_reply(tool, side_effect).outcome
    }
    fn confirm_reply(&mut self, _tool: &str, _side_effect: SideEffect) -> ConfirmReply {
        *self.asked.lock().unwrap() += 1;
        self.replies
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| ConfirmOutcome::Deny.into())
    }
    fn offer_rule(&mut self, suggestion: &RuleSuggestion) -> bool {
        self.offers.lock().unwrap().push(suggestion.clone());
        true
    }
    fn ask(&mut self, _q: &str, _o: &[forge_types::QChoice], _other: bool) -> String {
        String::new()
    }
    fn read_line(&mut self) -> Option<String> {
        None
    }
}

/// The trust-store override is process-wide, so these tests take turns.
static SERIAL: Mutex<()> = Mutex::new(());

struct Fixture {
    _serial: std::sync::MutexGuard<'static, ()>,
    dir: tempfile::TempDir,
    _trust_store: tempfile::TempDir,
    session: Session,
    msg_id: String,
}

impl Fixture {
    fn new(presenter: Answering) -> Self {
        let serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let trust_store = tempfile::tempdir().unwrap();
        // A trusted project writes its rule to `.forge/config.toml`; the store override keeps the
        // test off the real trust list.
        forge_config::project_trust::override_store_path(Some(
            trust_store.path().join("trusted-projects.json"),
        ));
        forge_config::project_trust::trust(dir.path()).unwrap();
        let config = Config {
            permission_mode: PermissionMode::Default,
            ..Config::default()
        };
        let session = Session::start(
            Arc::new(Store::open_in_memory().unwrap()),
            Arc::new(MockProvider),
            Arc::new(HeuristicRouter::new(config.clone())),
            ToolRegistry::with_core_tools_in(dir.path()),
            Box::new(presenter),
            config,
            dir.path().to_str().unwrap(),
        )
        .unwrap();
        let msg_id = session
            .store
            .add_message(session.session_id(), 0, Role::User, "go", None)
            .unwrap();
        Self {
            _serial: serial,
            dir,
            _trust_store: trust_store,
            session,
            msg_id,
        }
    }

    async fn shell(&mut self, command: &str) -> String {
        let call = forge_types::ToolCall {
            id: forge_types::new_id(),
            name: "shell".into(),
            args: serde_json::json!({ "command": command }),
        };
        self.session.invoke_tool(&self.msg_id, &call).await.unwrap()
    }

    fn project_config(&self) -> String {
        std::fs::read_to_string(self.dir.path().join(".forge").join("config.toml"))
            .unwrap_or_default()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        forge_config::project_trust::override_store_path(None);
    }
}

fn allows(n: usize) -> Vec<ConfirmReply> {
    (0..n).map(|_| ConfirmOutcome::Allow.into()).collect()
}

#[tokio::test]
async fn the_fourth_prompt_offers_the_rule_and_accepting_it_ends_the_prompts() {
    let presenter = Answering::new(
        allows(3)
            .into_iter()
            .chain([ConfirmReply::accepting_rule()])
            .collect(),
    );
    let mut fx = Fixture::new(presenter.clone());

    for n in 0..3 {
        fx.shell(&format!("touch a{n}")).await;
        assert!(
            presenter.offers().is_empty(),
            "no offer before three allows"
        );
    }
    fx.shell("touch a3").await;
    assert_eq!(
        presenter.offers(),
        vec![RuleSuggestion {
            rule: "touch *".into(),
            allows: 3
        }]
    );
    assert!(
        fx.project_config().contains("\"touch *\""),
        "{}",
        fx.project_config()
    );

    fx.shell("touch a4").await;
    assert_eq!(presenter.asked(), 4, "the fifth call runs without a prompt");
    assert!(fx.dir.path().join("a4").exists());
}

#[tokio::test]
async fn a_deny_resets_the_count_and_a_declined_offer_waits_for_three_more() {
    let mut replies = allows(2);
    replies.push(ConfirmOutcome::Deny.into());
    replies.extend(allows(3));
    replies.push(ConfirmOutcome::Allow.into());
    replies.extend(allows(2));
    replies.push(ConfirmOutcome::Allow.into());
    let presenter = Answering::new(replies);
    let mut fx = Fixture::new(presenter.clone());

    for n in 0..6 {
        fx.shell(&format!("touch b{n}")).await;
    }
    assert!(
        presenter.offers().is_empty(),
        "two allows, a deny, then three allows: the deny reset the count, so the sixth prompt \
         (the third since) is not yet offered"
    );
    fx.shell("touch b6").await;
    assert_eq!(presenter.offers().len(), 1, "three allows since the deny");
    for n in 7..10 {
        fx.shell(&format!("touch b{n}")).await;
    }
    assert_eq!(
        presenter.offers().len(),
        1,
        "the offer was ignored, so the next prompts must earn it again"
    );
    assert!(fx.project_config().is_empty());
}

#[tokio::test]
async fn a_dangerous_or_compound_command_is_never_offered() {
    let presenter = Answering::new(allows(8));
    let mut fx = Fixture::new(presenter.clone());
    for n in 0..4 {
        fx.shell(&format!("touch c{n} && touch d{n}")).await;
    }
    for n in 0..4 {
        fx.shell(&format!("curl http://127.0.0.1:9/{n}")).await;
    }
    assert!(presenter.offers().is_empty(), "{:?}", presenter.offers());
}

#[tokio::test]
async fn a_shape_an_existing_rule_covers_is_not_offered() {
    let presenter = Answering::new(allows(8));
    let mut fx = Fixture::new(presenter.clone());
    fx.session.rules.push(forge_types::PermissionRule {
        tool: "shell".into(),
        patterns: vec!["touch *".into()],
        decision: forge_types::PermissionDecision::Ask,
        source: forge_types::RuleSource::Configured,
        reason: None,
    });
    for n in 0..5 {
        fx.shell(&format!("touch e{n}")).await;
    }
    assert_eq!(presenter.asked(), 5);
    assert!(presenter.offers().is_empty());
}
