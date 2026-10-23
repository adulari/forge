//! The Ask temper through a real session: provably read-only shell runs with no prompt, every
//! other shell call still asks, and what outranks the auto-allow (deny rules, hooks, the opt-out)
//! still does. The decision itself is table-tested in `permission/ask_shell/tests.rs`; this is the
//! native loop's seam, where a prompt would show.

use super::*;
use forge_config::{HookConfig, HookEvent};

/// Counts the prompts the session raises and answers every one of them "no".
#[derive(Clone, Default)]
struct PromptCounter {
    asked: Arc<Mutex<Vec<String>>>,
}

impl Presenter for PromptCounter {
    fn emit(&mut self, _event: PresenterEvent) {}
    fn confirm(&mut self, tool: &str, _side_effect: SideEffect) -> forge_types::ConfirmOutcome {
        self.asked.lock().unwrap().push(tool.to_string());
        forge_types::ConfirmOutcome::Deny
    }
    fn ask(&mut self, _q: &str, options: &[forge_types::QChoice], _allow_other: bool) -> String {
        options.first().map(|o| o.label.clone()).unwrap_or_default()
    }
    fn read_line(&mut self) -> Option<String> {
        None
    }
    fn is_attended(&self) -> bool {
        true
    }
}

struct Ask {
    session: Session,
    asked: Arc<Mutex<Vec<String>>>,
    root: std::path::PathBuf,
    _dir: tempfile::TempDir,
}

fn ask_session(configure: impl FnOnce(&mut Config)) -> Ask {
    ask_session_trusting(true, configure)
}

/// A session in a workspace the user did (`true`) or did not (`false`) trust with `forge trust`,
/// on this test's thread only.
fn ask_session_trusting(trusted: bool, configure: impl FnOnce(&mut Config)) -> Ask {
    crate::permission::stand_in_for_trust(Some(std::rc::Rc::new(move |_: &std::path::Path| {
        trusted
    })));
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    std::fs::write(root.join("calc.py"), "def add(a, b):\n    return a - b\n").unwrap();
    std::fs::write(root.join(".env"), "API_KEY=hunter2\n").unwrap();
    let git = root.join(".git");
    std::fs::create_dir_all(git.join("objects")).unwrap();
    std::fs::create_dir_all(git.join("refs")).unwrap();
    std::fs::write(git.join("HEAD"), "ref: refs/heads/main\n").unwrap();
    std::fs::write(git.join("config"), "[core]\n\tbare = false\n").unwrap();
    let mut config = Config {
        permission_mode: forge_types::PermissionMode::Default,
        ..Config::default()
    };
    configure(&mut config);
    let presenter = PromptCounter::default();
    let asked = presenter.asked.clone();
    let session = Session::start(
        Arc::new(Store::open_in_memory().unwrap()),
        Arc::new(MockProvider),
        Arc::new(HeuristicRouter::new(config.clone())),
        ToolRegistry::with_core_tools_in(&root),
        Box::new(presenter),
        config,
        root.to_str().unwrap(),
    )
    .unwrap();
    Ask {
        session,
        asked,
        root,
        _dir: dir,
    }
}

impl Ask {
    async fn shell(&mut self, args: serde_json::Value) -> String {
        let sid = self.session.session_id().to_string();
        let msg = self
            .session
            .store
            .add_message(&sid, 0, Role::User, "x", None)
            .unwrap();
        let call = forge_types::ToolCall {
            id: forge_types::new_id(),
            name: "shell".into(),
            args,
        };
        self.session.invoke_tool(&msg, &call).await.unwrap()
    }

    async fn run(&mut self, command: &str) -> String {
        self.shell(serde_json::json!({ "command": command })).await
    }

    fn prompts(&self) -> usize {
        self.asked.lock().unwrap().len()
    }
}

#[tokio::test]
async fn read_only_shell_runs_without_a_prompt_and_a_writer_still_asks() {
    let mut ask = ask_session(|_| {});

    let listing = ask.run("ls").await;
    assert!(listing.contains("calc.py"), "{listing}");
    let shown = ask.run("cat calc.py | head -1").await;
    assert!(shown.contains("def add"), "{shown}");
    let counted = ask
        .shell(serde_json::json!({ "commands": ["wc -l calc.py", "git status --short"] }))
        .await;
    assert!(counted.contains("calc.py"), "{counted}");
    assert_eq!(ask.prompts(), 0, "read-only calls must not prompt");

    let refused = ask.run("touch ask_probe.txt").await;
    assert_eq!(ask.prompts(), 1, "{refused}");
    assert!(!ask.root.join("ask_probe.txt").exists());

    let chained = ask.run("ls; touch ask_probe.txt").await;
    assert_eq!(ask.prompts(), 2, "{chained}");
    assert!(!ask.root.join("ask_probe.txt").exists());
}

#[tokio::test]
async fn a_secret_read_is_refused_by_the_rules_not_run_and_not_asked() {
    let mut ask = ask_session(|_| {});
    let denied = ask.run("cat .env").await;
    assert!(
        denied.starts_with("permission denied by policy"),
        "{denied}"
    );
    assert!(!denied.contains("hunter2"));
    // A reader the deny verbs do not list is not auto-allowed past the secret rules: it asks.
    let sneaky = ask.run("wc -c .env").await;
    assert_eq!(ask.prompts(), 1, "{sneaky}");
}

#[tokio::test]
async fn a_configured_deny_rule_beats_the_auto_allow() {
    let mut ask = ask_session(|config| {
        config.permissions.rules.push(forge_config::RuleConfig {
            tool: "shell".into(),
            allow: None,
            ask: None,
            deny: Some(forge_config::OneOrMany::One("ls*".into())),
            reason: None,
        });
    });
    let denied = ask.run("ls").await;
    assert!(denied.starts_with("permission denied"), "{denied}");
    assert_eq!(ask.prompts(), 0);
}

#[tokio::test]
async fn a_configured_ask_rule_beats_the_auto_allow() {
    let mut ask = ask_session(|config| {
        config.permissions.rules.push(forge_config::RuleConfig {
            tool: "shell".into(),
            allow: None,
            ask: Some(forge_config::OneOrMany::One("ls*".into())),
            deny: None,
            reason: None,
        });
    });
    ask.run("ls").await;
    assert_eq!(ask.prompts(), 1);
}

#[tokio::test]
async fn the_setting_restores_the_prompt() {
    let mut ask = ask_session(|config| config.permissions.auto_allow_read_only_shell = false);
    ask.run("ls").await;
    assert_eq!(ask.prompts(), 1);
}

#[cfg(unix)]
#[tokio::test]
async fn a_pre_tool_use_hook_can_still_block_an_auto_allowed_command() {
    let mut ask = ask_session(|config| {
        config.hooks = vec![HookConfig {
            cc_compat: true,
            timeout_secs: 10,
            ..HookConfig::new(HookEvent::PreToolUse, "echo 'hook says no' >&2; exit 2")
        }];
    });
    let blocked = ask.run("git status").await;
    assert!(blocked.contains("hook says no"), "{blocked}");
    assert_eq!(ask.prompts(), 0);
}

#[tokio::test]
async fn other_tempers_do_not_change() {
    for (mode, prompts) in [
        (forge_types::PermissionMode::AcceptEdits, 0),
        (forge_types::PermissionMode::Bypass, 0),
    ] {
        let mut ask = ask_session(|config| config.permission_mode = mode);
        ask.run("touch tempers_probe.txt").await;
        assert_eq!(ask.prompts(), prompts, "{mode:?}");
        assert!(ask.root.join("tempers_probe.txt").exists(), "{mode:?}");
    }
    let mut plan = ask_session(|config| config.permission_mode = forge_types::PermissionMode::Plan);
    let denied = plan.run("touch tempers_probe.txt").await;
    assert!(denied.starts_with("permission denied"), "{denied}");
    assert_eq!(plan.prompts(), 0);
    assert!(plan.run("ls").await.contains("calc.py"));
}

/// The trust gate through the native loop's seam: git is the one auto-allowed binary a repository
/// can influence, so in a project nobody trusted it keeps its prompt while everything else that
/// only reads still runs, and trust withdrawn between two calls applies to the second.
#[tokio::test]
async fn git_asks_in_an_untrusted_project_and_runs_in_a_trusted_one() {
    let mut untrusted = ask_session_trusting(false, |_| {});
    let listing = untrusted.run("ls").await;
    assert!(listing.contains("calc.py"), "{listing}");
    untrusted.run("cat calc.py | head -1").await;
    assert_eq!(untrusted.prompts(), 0, "non-git reads need no trust");
    untrusted.run("git status --short").await;
    assert_eq!(untrusted.prompts(), 1, "git in an untrusted project asks");
    untrusted.run("ls && git log -1").await;
    assert_eq!(untrusted.prompts(), 2);

    let mut trusted = ask_session_trusting(true, |_| {});
    trusted.run("git status --short").await;
    trusted.run("git log -1").await;
    assert_eq!(trusted.prompts(), 0, "git in a trusted project does not");

    crate::permission::stand_in_for_trust(Some(std::rc::Rc::new(|_: &std::path::Path| false)));
    trusted.run("git status --short").await;
    assert_eq!(
        trusted.prompts(),
        1,
        "trust withdrawn applies to the next call"
    );
    trusted.run("ls").await;
    assert_eq!(trusted.prompts(), 1);
}
