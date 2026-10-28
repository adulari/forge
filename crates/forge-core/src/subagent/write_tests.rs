//! A child that may write: its toolset, the temper bounds, and what happens to an `Ask`.

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::json;

use forge_provider::{EventSink, ModelResponse, ProviderError};
use forge_types::{SideEffect, ToolCall, Usage};

use super::*;

/// Writes `out.txt` on its first step, then reports the tool result it was given.
struct WriterProvider {
    steps: AtomicUsize,
    dir: PathBuf,
    offered: std::sync::Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl Provider for WriterProvider {
    async fn complete(
        &self,
        _model: &str,
        messages: &[Message],
        tools: &[ToolSpec],
        _on_event: &mut EventSink<'_>,
    ) -> Result<ModelResponse, ProviderError> {
        *self.offered.lock().unwrap() = tools.iter().map(|t| t.name.clone()).collect();
        let first = self.steps.fetch_add(1, Ordering::SeqCst) == 0;
        let (content, tool_calls) = if first {
            let call = ToolCall {
                id: "c1".into(),
                name: "write_file".into(),
                args: json!({"path": self.dir.join("out.txt"), "content": "hi\n"}),
            };
            (String::new(), vec![call])
        } else {
            let seen = messages
                .iter()
                .rev()
                .find(|m| m.role == Role::Tool)
                .map(|m| m.content.clone())
                .unwrap_or_default();
            (format!("tool said: {seen}"), vec![])
        };
        Ok(ModelResponse {
            reasoning: String::new(),
            reasoning_items: Vec::new(),
            content,
            tool_calls,
            usage: Usage::default(),
            quotas: Vec::new(),
        })
    }
}

struct One;
#[async_trait::async_trait]
impl Router for One {
    async fn route(
        &self,
        _p: &str,
        _i: bool,
        _b: BudgetState,
        _h: &forge_types::ModelHealth,
        _q: &forge_types::SubscriptionQuota,
        _e: Option<forge_types::EffortLevel>,
        _project: &forge_types::ProjectContext,
    ) -> RoutingDecision {
        RoutingDecision {
            effort: None,
            tier: TaskTier::Standard,
            model: "openai::gpt-test".into(),
            rationale: "test".into(),
            fallbacks: vec![],
            pinned: false,
            unroutable: false,
        }
    }
}

struct Run {
    out: String,
    wrote: bool,
    offered: Vec<String>,
}

async fn run(agent: &str, mode: PermissionMode, asker: Option<ChildAsker>) -> Run {
    let dir = tempfile::tempdir().unwrap();
    let provider = Arc::new(WriterProvider {
        steps: AtomicUsize::new(0),
        dir: dir.path().to_path_buf(),
        offered: std::sync::Mutex::new(Vec::new()),
    });
    let config = Config::default();
    let ctx = AgentCtx {
        provider: provider.clone(),
        router: Arc::new(One),
        store: Arc::new(Store::open_in_memory().unwrap()),
        pricing: Pricing::from_config(&config),
        config,
        mode,
        rules: Vec::new(),
        depth: 0,
        max_depth: 0,
        agents: Arc::new(forge_config::load_agents_layered(
            dir.path(),
            ".forge/agents",
        )),
        worktree_root: None,
        repo_root: dir.path().to_path_buf(),
        effective_pin: None,
        inherit_pin: true,
        asker,
    };
    let requests = vec![AgentRequest {
        agent: agent.into(),
        task: "write out.txt".into(),
    }];
    let mut sink = |_: Lifecycle| {};
    let (out, _) = orchestrate(
        &ctx,
        "parent",
        requests,
        BudgetState::default(),
        1,
        &mut sink,
    )
    .await
    .unwrap();
    let offered = provider.offered.lock().unwrap().clone();
    Run {
        out,
        wrote: dir.path().join("out.txt").exists(),
        offered,
    }
}

fn answering(answer: ChildAnswer, asked: Arc<std::sync::Mutex<Vec<ChildQuestion>>>) -> ChildAsker {
    Arc::new(move |q| {
        asked.lock().unwrap().push(q);
        let answer = answer.clone();
        Box::pin(async move { answer })
    })
}

#[tokio::test]
async fn an_unknown_type_is_general_purpose_and_writes_under_bypass() {
    let r = run("no-such-type", PermissionMode::Bypass, None).await;
    assert!(r.wrote, "{}", r.out);
    for tool in ["write_file", "edit_file", "shell"] {
        assert!(
            r.offered.iter().any(|t| t == tool),
            "{tool}: {:?}",
            r.offered
        );
    }
}

#[tokio::test]
async fn a_plan_parent_stops_a_children_writes() {
    let r = run("general-purpose", PermissionMode::Plan, None).await;
    assert!(!r.wrote);
    assert!(r.out.contains("permission denied"), "{}", r.out);
}

#[tokio::test]
async fn explore_stays_read_only_even_under_bypass() {
    let r = run("explore", PermissionMode::Bypass, None).await;
    assert!(!r.wrote);
    assert!(!r.offered.iter().any(|t| t == "write_file"));
    assert!(!r.offered.iter().any(|t| t == "edit_file"));
}

#[tokio::test]
async fn ask_without_an_asker_says_nobody_can_be_asked() {
    let r = run("general-purpose", PermissionMode::Default, None).await;
    assert!(!r.wrote);
    assert!(r.out.contains("no one can be asked"), "{}", r.out);
    assert!(r.out.contains("`default` temper"), "{}", r.out);
}

#[tokio::test]
async fn ask_with_an_asker_runs_the_call_when_approved_and_names_the_child() {
    let asked = Arc::new(std::sync::Mutex::new(Vec::new()));
    let asker = answering(ChildAnswer::Allow, Arc::clone(&asked));
    let r = run("general-purpose", PermissionMode::Default, Some(asker)).await;
    assert!(r.wrote, "{}", r.out);
    let asked = asked.lock().unwrap();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].tool, "write_file");
    assert_eq!(asked[0].side_effect, SideEffect::Write);
    assert!(asked[0].child.starts_with("general-purpose-"));
}

#[tokio::test]
async fn ask_with_an_asker_refuses_the_call_when_declined() {
    let asked = Arc::new(std::sync::Mutex::new(Vec::new()));
    let asker = answering(ChildAnswer::Deny(Some("not now".into())), asked);
    let r = run("general-purpose", PermissionMode::Default, Some(asker)).await;
    assert!(!r.wrote);
    assert!(r.out.contains("denied by the user"), "{}", r.out);
    assert!(r.out.contains("not now"), "{}", r.out);
}

#[tokio::test]
async fn types_resolve_to_the_expected_write_capability() {
    let r = run("general-purpose", PermissionMode::Bypass, None).await;
    assert!(r.wrote);
    let agents = forge_config::load_agents_layered(Path::new("/nonexistent-forge-root"), ".x");
    let explore = resolve(
        &AgentRequest {
            agent: "explore".into(),
            task: "t".into(),
        },
        &agents,
    );
    let registry = ToolRegistry::with_core_tools();
    assert!(!is_write_capable(&explore, &registry));
    let general = resolve(
        &AgentRequest {
            agent: "whatever".into(),
            task: "t".into(),
        },
        &agents,
    );
    assert_eq!(general.name, "whatever");
    assert!(is_write_capable(&general, &registry));
}
