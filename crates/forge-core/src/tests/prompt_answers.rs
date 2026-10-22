//! Answers at the runtime permission prompt: "this session" allows without saving a rule, and a
//! note typed with the answer reaches the model in the tool result.

use super::*;
use forge_types::{ConfirmOutcome, ConfirmReply};

/// Hands out scripted replies in order and counts how often it was asked.
#[derive(Clone, Default)]
struct Answering {
    replies: Arc<Mutex<std::collections::VecDeque<ConfirmReply>>>,
    asked: Arc<Mutex<usize>>,
}

impl Answering {
    fn new(replies: Vec<ConfirmReply>) -> Self {
        Self {
            replies: Arc::new(Mutex::new(replies.into())),
            asked: Arc::default(),
        }
    }
    fn asked(&self) -> usize {
        *self.asked.lock().unwrap()
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
    fn ask(&mut self, _q: &str, _o: &[forge_types::QChoice], _other: bool) -> String {
        String::new()
    }
    fn read_line(&mut self) -> Option<String> {
        None
    }
}

struct Fixture {
    dir: std::path::PathBuf,
    session: Session,
    msg_id: String,
}

impl Fixture {
    fn new(presenter: Answering) -> Self {
        let dir = std::env::temp_dir().join(format!("forge-answers-{}", forge_types::new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let config = Config {
            permission_mode: PermissionMode::Default,
            ..Config::default()
        };
        let session = Session::start(
            Arc::new(Store::open_in_memory().unwrap()),
            Arc::new(MockProvider),
            Arc::new(HeuristicRouter::new(config.clone())),
            ToolRegistry::with_core_tools_in(&dir),
            Box::new(presenter),
            config,
            dir.to_str().unwrap(),
        )
        .unwrap();
        let msg_id = session
            .store
            .add_message(session.session_id(), 0, Role::User, "go", None)
            .unwrap();
        Self {
            dir,
            session,
            msg_id,
        }
    }

    async fn write(&mut self, name: &str) -> String {
        let call = forge_types::ToolCall {
            id: forge_types::new_id(),
            name: "write_file".into(),
            args: serde_json::json!({ "path": self.dir.join(name), "content": "x" }),
        };
        self.session.invoke_tool(&self.msg_id, &call).await.unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[tokio::test]
async fn allow_for_this_session_stops_asking_and_saves_nothing() {
    let presenter = Answering::new(vec![ConfirmOutcome::AllowSession.into()]);
    let mut fx = Fixture::new(presenter.clone());

    fx.write("a.txt").await;
    fx.write("b.txt").await;

    assert_eq!(presenter.asked(), 1, "the second write is covered");
    assert!(fx.dir.join("a.txt").exists() && fx.dir.join("b.txt").exists());
    assert!(
        !fx.dir.join(".forge").join("config.toml").exists(),
        "a session allow is never written to the project config"
    );
}

#[tokio::test]
async fn a_note_on_deny_is_the_reason_the_model_reads() {
    let presenter = Answering::new(vec![ConfirmReply::with_note(
        ConfirmOutcome::Deny,
        "write it under docs/ instead",
    )]);
    let mut fx = Fixture::new(presenter);

    let result = fx.write("a.txt").await;

    assert!(result.contains("declined at the prompt"), "{result}");
    assert!(
        result.contains("The user said: write it under docs/ instead"),
        "{result}"
    );
    assert!(!fx.dir.join("a.txt").exists());
}

#[tokio::test]
async fn a_note_on_allow_follows_the_tool_output() {
    let presenter = Answering::new(vec![ConfirmReply::with_note(
        ConfirmOutcome::Allow,
        "keep it short",
    )]);
    let mut fx = Fixture::new(presenter.clone());

    let result = fx.write("a.txt").await;

    assert!(fx.dir.join("a.txt").exists());
    assert!(
        result.trim_end().ends_with("added: keep it short]"),
        "{result}"
    );
    fx.write("b.txt").await;
    assert_eq!(presenter.asked(), 2, "allow once still asks next time");
}
