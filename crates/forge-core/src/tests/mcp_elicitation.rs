//! A server's elicitation reaches only a surface where a person answers it, names the asking
//! server on every question, and never leaves its target connection.

use super::*;
use forge_types::{Answer, Question};

#[derive(Clone)]
struct FormPresenter {
    can_answer: bool,
    asked: Arc<Mutex<Vec<Vec<Question>>>>,
}

impl Presenter for FormPresenter {
    fn emit(&mut self, _event: PresenterEvent) {}
    fn confirm(&mut self, _tool: &str, _side_effect: SideEffect) -> forge_types::ConfirmOutcome {
        forge_types::ConfirmOutcome::Allow
    }
    // Like `forge mcp agent`: auto-picks the first option, which would say "Yes" to a confirmation.
    fn ask(&mut self, _q: &str, options: &[forge_types::QChoice], _other: bool) -> String {
        options.first().map(|o| o.label.clone()).unwrap_or_default()
    }
    fn ask_form(&mut self, questions: &[Question]) -> Vec<Answer> {
        self.asked.lock().unwrap().push(questions.to_vec());
        questions
            .iter()
            .map(|q| {
                if q.text.contains("Count") {
                    Answer::from_text("4", &[])
                } else if q.text.contains("API token") {
                    Answer::from_text("s3cret", &[])
                } else {
                    Answer::default()
                }
            })
            .collect()
    }
    fn read_line(&mut self) -> Option<String> {
        None
    }
    fn can_answer_elicitation(&self) -> bool {
        self.can_answer
    }
}

async fn ask_through(can_answer: bool) -> (String, Vec<Vec<Question>>) {
    let mcp = forge_config::McpConfig {
        allow: forge_config::McpAllowlist {
            servers: vec!["test".into()],
            tools: vec!["test__ask".into()],
        },
        ..Default::default()
    };
    let config = Config {
        mcp: mcp.clone(),
        permission_mode: forge_types::PermissionMode::Bypass,
        ..Config::default()
    };
    let dir = tempfile::tempdir().unwrap();
    let presenter = FormPresenter {
        can_answer,
        asked: Arc::default(),
    };
    let asked = Arc::clone(&presenter.asked);
    let mut session = Session::start(
        Arc::new(Store::open_in_memory().unwrap()),
        Arc::new(MockProvider),
        Arc::new(HeuristicRouter::new(config.clone())),
        ToolRegistry::with_core_tools_in(dir.path()),
        Box::new(presenter),
        config,
        dir.path().to_str().unwrap(),
    )
    .unwrap();
    session.set_mcp(Some(Arc::new(
        forge_mcp::testsupport::manager_with_echo(&mcp).await,
    )));
    let msg_id = session
        .store
        .add_message(session.id(), 0, Role::User, "ask", None)
        .unwrap();
    let result = session
        .invoke_tool(
            &msg_id,
            &forge_types::ToolCall {
                id: "c".into(),
                name: forge_mcp::MCP_CALL.into(),
                args: serde_json::json!({"name": "test__ask", "arguments": {}}),
            },
        )
        .await
        .unwrap();
    let asked = asked.lock().unwrap().clone();
    (result, asked)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_surface_that_auto_answers_never_gets_the_form() {
    let (result, asked) = ask_through(false).await;
    assert!(asked.is_empty(), "form was put to a non-human surface");
    assert!(result.contains("\"action\":\"decline\""), "{result}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_human_surface_answers_and_every_question_names_the_server() {
    let (result, asked) = ask_through(true).await;
    assert!(result.contains("\"action\":\"accept\""), "{result}");
    assert!(result.contains("\"count\":4"), "{result}");
    let form = &asked[0];
    assert!(
        form.iter()
            .all(|q| q.text.starts_with("MCP server test asks")),
        "{form:?}"
    );
    assert!(form.iter().any(|q| q.secret));
}
