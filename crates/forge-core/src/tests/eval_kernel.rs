//! The persistent `python` kernel tool on a real session: advertising, the permission posture,
//! the safety floor, failure classification and the verification ledger (docs/features/eval-kernel.md).

use super::*;
use forge_types::{ConfirmOutcome, ConfirmReply};

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

fn python3_available() -> bool {
    std::env::var_os("PATH")
        .is_some_and(|p| std::env::split_paths(&p).any(|d| d.join("python3").is_file()))
}

struct Fixture {
    dir: std::path::PathBuf,
    session: Session,
    msg_id: String,
}

impl Fixture {
    fn new(
        mode: PermissionMode,
        kernel: bool,
        rules: Vec<forge_types::PermissionRule>,
    ) -> (Self, Answering) {
        Self::with_replies(mode, kernel, rules, Vec::new())
    }

    fn with_replies(
        mode: PermissionMode,
        kernel: bool,
        rules: Vec<forge_types::PermissionRule>,
        replies: Vec<ConfirmReply>,
    ) -> (Self, Answering) {
        let dir = std::env::temp_dir().join(format!("forge-evalk-{}", forge_types::new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let presenter = Answering::new(replies);
        let config = Config {
            permission_mode: mode,
            ..Config::default()
        };
        let mut tools = ToolRegistry::with_core_tools_in(&dir);
        if kernel {
            tools.register(Box::new(forge_tools::PythonTool::new()));
        }
        let mut session = Session::start(
            Arc::new(Store::open_in_memory().unwrap()),
            Arc::new(MockProvider),
            Arc::new(HeuristicRouter::new(config.clone())),
            tools,
            Box::new(presenter.clone()),
            config,
            dir.to_str().unwrap(),
        )
        .unwrap();
        session.rules.extend(rules);
        let msg_id = session
            .store
            .add_message(session.session_id(), 0, Role::User, "go", None)
            .unwrap();
        (
            Self {
                dir,
                session,
                msg_id,
            },
            presenter,
        )
    }

    async fn python(&mut self, code: &str) -> String {
        let call = forge_types::ToolCall {
            id: forge_types::new_id(),
            name: "python".into(),
            args: serde_json::json!({ "code": code }),
        };
        self.session.invoke_tool(&self.msg_id, &call).await.unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn names(session: &Session) -> Vec<String> {
    session
        .full_tool_specs()
        .into_iter()
        .map(|s| s.name)
        .collect()
}

#[tokio::test]
async fn flag_off_means_no_tool_and_an_unknown_tool_error() {
    let (mut fx, _) = Fixture::new(PermissionMode::Bypass, false, Vec::new());
    assert!(!names(&fx.session).iter().any(|n| n == "python"));
    assert!(!fx
        .session
        .transcript_with_preamble_and_tools("direct::m", &fx.session.tool_specs())
        .iter()
        .any(|m| m.content.contains("python (persistent kernel)")));
    let out = fx.python("1").await;
    assert!(out.contains("unknown tool"), "{out}");
}

#[test]
fn flag_on_advertises_it_on_the_lean_surface_with_guidance_once() {
    let (fx, _) = Fixture::new(PermissionMode::Bypass, true, Vec::new());
    let lean = fx.session.tool_specs();
    assert!(lean.iter().any(|s| s.name == "python"), "lean surface");
    let count = |model: &str| {
        fx.session
            .transcript_with_preamble_and_tools(model, &lean)
            .iter()
            .filter(|m| m.content.contains("python (persistent kernel)"))
            .count()
    };
    assert_eq!(count("direct::m"), 1);
    assert_eq!(count("claude-cli::opus"), 0, "never for a CLI bridge");
}

#[test]
fn plan_mode_and_observational_scope_hide_it() {
    let (fx, _) = Fixture::new(PermissionMode::Plan, true, Vec::new());
    assert!(!names(&fx.session).iter().any(|n| n == "python"));
    let scope = TaskScope::for_test(
        "audit the current implementation",
        TaskIntent::ReadOnlyReview,
        PermissionMode::Bypass,
        1,
        None,
    );
    assert!(!scope.permits_tool("python"));
}

#[tokio::test]
async fn default_mode_asks_and_always_or_session_answers_are_only_once() {
    if !python3_available() {
        return;
    }
    let replies = vec![
        ConfirmOutcome::AlwaysAllow.into(),
        ConfirmOutcome::AllowSession.into(),
        ConfirmOutcome::Allow.into(),
    ];
    let (mut fx, presenter) =
        Fixture::with_replies(PermissionMode::Default, true, Vec::new(), replies);
    for _ in 0..3 {
        assert!(fx.python("1 + 1").await.starts_with("python: ok"));
    }
    assert_eq!(presenter.asked(), 3, "every cell asks again");
    assert!(
        !fx.dir.join(".forge").join("config.toml").exists(),
        "nothing is written to project config"
    );
}

#[tokio::test]
async fn a_deny_rule_for_python_denies() {
    let rule = forge_types::PermissionRule {
        tool: "python".into(),
        patterns: vec!["*".into()],
        decision: forge_types::PermissionDecision::Deny,
        source: forge_types::RuleSource::Configured,
        reason: None,
    };
    let (mut fx, _) = Fixture::new(PermissionMode::Bypass, true, vec![rule]);
    let out = fx.python("1").await;
    assert!(out.to_lowercase().contains("denied"), "{out}");
}

#[tokio::test]
async fn bypass_floor_refuses_privilege_and_home_deletion_but_not_data_strings() {
    let (mut fx, _) = Fixture::new(PermissionMode::Bypass, true, Vec::new());
    let sudo = fx.python("!sudo apt install x").await;
    assert!(
        sudo.contains("privileged") || sudo.contains("sudo"),
        "{sudo}"
    );
    assert!(!sudo.starts_with("python:"), "{sudo}");
    let rmtree = fx
        .python("import shutil, os\nshutil.rmtree(os.path.expanduser('~'))")
        .await;
    assert!(rmtree.contains("safety floor"), "{rmtree}");
    if python3_available() {
        for ok in [
            "open('/etc/passwd').read()[:0]",
            "df = {'su': 1}\ndf['su']",
            "s = \"rm -rf /\"\nlen(s)",
        ] {
            let out = fx.python(ok).await;
            assert!(out.starts_with("python: ok"), "{ok}: {out}");
        }
    }
}

#[tokio::test]
async fn a_quote_opened_on_a_magic_line_cannot_hide_the_next_one() {
    let (mut fx, _) = Fixture::new(PermissionMode::Bypass, true, Vec::new());
    for cell in [
        "!echo '''\n!rm -rf ~",
        "!echo \"\"\"\n!rm -rf ~",
        "!echo '''\n!/usr/bin/sudo -n true",
    ] {
        let out = fx.python(cell).await;
        assert!(!out.starts_with("python:"), "{cell}: {out}");
        assert!(
            out.contains("safety floor") || out.contains("Hand it to the user"),
            "{cell}: {out}"
        );
    }
}

#[test]
fn failure_classification_and_ledger() {
    assert!(classify_tool_failure("python: error KeyError (cell 3)\ntb").is_some());
    assert!(classify_tool_failure("python: ok (cell 3, 0.1s)").is_none());
    let args = |code: &str| serde_json::json!({ "code": code }).to_string();
    use crate::completion::{classify_tool, VerificationObservation as V};
    assert_eq!(
        classify_tool("python", &args("df.to_csv('o.csv')")),
        V::Mutation
    );
    assert_eq!(classify_tool("python", &args("!pytest -q")), V::Mutation);
    assert_eq!(classify_tool("python", &args("df.describe()")), V::Generic);
    let call = forge_types::ToolCall {
        id: "1".into(),
        name: "python".into(),
        args: serde_json::json!({ "code": "# c\nimport pandas as pd\n1" }),
    };
    assert_eq!(
        crate::loop_progress::call_label(&call),
        "python import pandas as pd"
    );
}

#[tokio::test]
async fn error_cells_are_not_shown_as_ok_and_run_code_can_call_python() {
    if !python3_available() {
        return;
    }
    let (mut fx, _) = Fixture::new(PermissionMode::Bypass, true, Vec::new());
    let out = fx.python("1 / 0").await;
    assert!(out.starts_with("python: error ZeroDivisionError"), "{out}");
    let call = forge_types::ToolCall {
        id: forge_types::new_id(),
        name: crate::codemode::RUN_CODE_TOOL.into(),
        args: serde_json::json!({ "code": "const r = await tools.python({code: '1+1'}); return r;" }),
    };
    let counter = || Arc::new(std::sync::atomic::AtomicU64::new(0));
    let (ran, inspect, mutated) = (counter(), counter(), counter());
    let verification = Arc::new(Mutex::new(VerificationLedger::default()));
    let mut observations = Vec::new();
    fx.session
        .execute_model_tool_step(
            &fx.msg_id,
            &[call],
            &ran,
            &inspect,
            &mutated,
            &verification,
            &mut observations,
        )
        .await
        .unwrap();
    let inner = fx
        .session
        .transcript
        .iter()
        .rev()
        .find(|m| m.role == Role::Tool)
        .map(|m| m.content.clone())
        .unwrap_or_default();
    assert!(
        inner.contains("python: ok") && inner.contains("Out: 2"),
        "{inner}"
    );
}
