//! `auto` temper through a real turn: an unrecognised shell command goes to the side-call
//! classifier, whose answer can only turn Unknown into Allow, and any failure asks (which an
//! unattended presenter denies).

use super::*;

use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Clone, Copy)]
enum Reply {
    Text(&'static str),
    Hang,
}

struct Scripted {
    main_calls: AtomicUsize,
    classifier_calls: Arc<AtomicUsize>,
    commands: Vec<&'static str>,
    reply: Reply,
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
        let is_classifier = messages
            .first()
            .is_some_and(|m| m.content.contains("permission classifier"));
        let (content, tool_calls) = if is_classifier {
            self.classifier_calls.fetch_add(1, Ordering::SeqCst);
            match self.reply {
                Reply::Text(text) => (text.to_string(), vec![]),
                Reply::Hang => {
                    tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
                    (String::new(), vec![])
                }
            }
        } else {
            let n = self.main_calls.fetch_add(1, Ordering::SeqCst);
            match self.commands.get(n) {
                Some(cmd) => (
                    String::new(),
                    vec![forge_types::ToolCall {
                        id: forge_types::new_id(),
                        name: "shell".into(),
                        args: serde_json::json!({ "command": cmd }),
                    }],
                ),
                None => ("done".to_string(), vec![]),
            }
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

struct Outcome {
    results: Vec<String>,
    warnings: Vec<String>,
    classifier_calls: usize,
}

async fn run_auto(mut config: Config, commands: Vec<&'static str>, reply: Reply) -> Outcome {
    config.permission_mode = PermissionMode::Auto;
    config.recap.enabled = false;
    config.suggest.enabled = false;
    config.mesh.auto_memory = false;
    config.mesh.verify_completeness = false;
    let classifier_calls = Arc::new(AtomicUsize::new(0));
    let presenter = CapturePresenter {
        attended: true,
        ..Default::default()
    };
    let events = Arc::clone(&presenter.events);
    let mut session = Session::start(
        Arc::new(Store::open_in_memory().unwrap()),
        Arc::new(Scripted {
            main_calls: AtomicUsize::new(0),
            classifier_calls: Arc::clone(&classifier_calls),
            commands,
            reply,
        }),
        Arc::new(FixedRouter {
            model: "ollama::qwen3:4b".into(),
            fallbacks: vec![],
        }),
        ToolRegistry::with_core_tools_in(test_workspace()),
        Box::new(presenter),
        config,
        test_workspace().to_str().unwrap(),
    )
    .unwrap();
    session.set_catalog(Some(ModelCatalog::new(vec!["ollama::qwen3:4b".into()])));
    session
        .run_turn("encode the word hi as base64 and show me")
        .await
        .unwrap();
    let results = session
        .transcript
        .iter()
        .filter(|m| m.role == Role::Tool)
        .map(|m| m.content.clone())
        .collect();
    let warnings = events
        .lock()
        .unwrap()
        .iter()
        .filter_map(|e| match e {
            PresenterEvent::Warning(w) => Some(w.clone()),
            _ => None,
        })
        .collect();
    Outcome {
        results,
        warnings,
        classifier_calls: classifier_calls.load(Ordering::SeqCst),
    }
}

const ALLOW: Reply = Reply::Text(r#"{"verdict":"allow","reason":"encodes a literal"}"#);
const UNKNOWN: &str = "echo hi | base64";

fn denied(result: &str) -> bool {
    result.starts_with("permission denied")
}

#[tokio::test]
async fn classifier_allow_runs_the_command_and_is_cached_per_normalized_command() {
    let out = run_auto(
        Config::default(),
        vec![UNKNOWN, "echo hi  |  base64"],
        ALLOW,
    )
    .await;
    assert_eq!(out.results.len(), 2, "{:?}", out.results);
    assert!(out.results.iter().all(|r| !denied(r)), "{:?}", out.results);
    assert!(out.results[0].contains("aGk"), "{:?}", out.results);
    assert_eq!(out.classifier_calls, 1, "second call must hit the cache");
}

#[tokio::test]
async fn classifier_ask_denies_and_surfaces_its_reason() {
    let reply = Reply::Text(r#"{"verdict":"ask","reason":"decodes opaque data"}"#);
    let out = run_auto(Config::default(), vec![UNKNOWN], reply).await;
    assert!(denied(&out.results[0]), "{:?}", out.results);
    assert!(
        out.warnings
            .iter()
            .any(|w| w == "auto mode asks: classifier: decodes opaque data"),
        "{:?}",
        out.warnings
    );
}

#[tokio::test]
async fn malformed_answer_asks_and_is_not_cached() {
    let out = run_auto(
        Config::default(),
        vec![UNKNOWN, UNKNOWN],
        Reply::Text("looks fine to me"),
    )
    .await;
    assert!(out.results.iter().all(|r| denied(r)), "{:?}", out.results);
    assert_eq!(out.classifier_calls, 2);
    assert!(out
        .warnings
        .iter()
        .any(|w| w.contains("classifier gave an unusable answer")));
}

#[tokio::test(start_paused = true)]
async fn timeout_fails_closed() {
    let out = run_auto(Config::default(), vec![UNKNOWN], Reply::Hang).await;
    assert!(denied(&out.results[0]), "{:?}", out.results);
    assert_eq!(out.classifier_calls, 1);
    assert!(
        out.warnings.iter().any(|w| w.contains("timed out")),
        "{:?}",
        out.warnings
    );
}

#[tokio::test]
async fn a_risky_command_never_reaches_the_classifier() {
    let out = run_auto(
        Config::default(),
        vec!["rm -rf auto_classifier_probe"],
        ALLOW,
    )
    .await;
    assert!(denied(&out.results[0]), "{:?}", out.results);
    assert_eq!(out.classifier_calls, 0);
}

#[tokio::test]
async fn a_known_safe_command_needs_no_classifier() {
    let out = run_auto(Config::default(), vec!["echo hi"], ALLOW).await;
    assert!(!denied(&out.results[0]), "{:?}", out.results);
    assert_eq!(out.classifier_calls, 0);
}

#[tokio::test]
async fn disabled_classifier_asks_without_a_model_call() {
    let mut config = Config::default();
    config.permissions.auto_classifier = false;
    let out = run_auto(config, vec![UNKNOWN], ALLOW).await;
    assert!(denied(&out.results[0]), "{:?}", out.results);
    assert_eq!(out.classifier_calls, 0);
    assert!(out
        .warnings
        .iter()
        .any(|w| w.contains("auto_classifier is off")));
}

#[tokio::test]
async fn an_explicit_ask_rule_is_not_overridden_by_the_classifier() {
    let mut config = Config::default();
    config.permissions.rules.push(forge_config::RuleConfig {
        tool: "shell".into(),
        allow: None,
        ask: Some(forge_config::OneOrMany::One("base64*".into())),
        deny: None,
        reason: None,
    });
    let out = run_auto(config, vec![UNKNOWN], ALLOW).await;
    assert!(denied(&out.results[0]), "{:?}", out.results);
    assert_eq!(out.classifier_calls, 0);
}
