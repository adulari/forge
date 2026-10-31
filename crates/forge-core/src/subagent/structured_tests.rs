//! `submit_result` end to end through the real child loop, with a scripted provider.

use std::sync::Mutex;

use serde_json::{json, Value};

use forge_provider::{CompletionOptions, EventSink, ModelResponse, ProviderError};
use forge_types::{ToolCall, Usage};

use super::structured::ResultStatus;
use super::*;

#[derive(Clone)]
struct Reply {
    content: String,
    calls: Vec<(&'static str, Value)>,
}

fn prose(text: &str) -> Reply {
    Reply {
        content: text.into(),
        calls: Vec::new(),
    }
}

fn calls(calls: Vec<(&'static str, Value)>) -> Reply {
    Reply {
        content: String::new(),
        calls,
    }
}

fn submit(args: Value) -> Reply {
    calls(vec![("submit_result", args)])
}

fn read_manifest() -> Reply {
    calls(vec![("read_file", json!({"path": "Cargo.toml"}))])
}

#[derive(Debug, Clone)]
struct Seen {
    tools: Vec<String>,
    with_options: bool,
    last_role: Role,
    last_content: String,
    model: String,
}

struct Scripted {
    replies: Mutex<std::collections::VecDeque<Reply>>,
    wrap_reply: Mutex<Option<Reply>>,
    bad_models: Vec<&'static str>,
    seen: Mutex<Vec<Seen>>,
}

impl Scripted {
    fn new(replies: Vec<Reply>) -> Arc<Self> {
        Arc::new(Self {
            replies: Mutex::new(replies.into()),
            wrap_reply: Mutex::new(None),
            bad_models: Vec::new(),
            seen: Mutex::new(Vec::new()),
        })
    }

    fn with_wrap_reply(self: Arc<Self>, reply: Reply) -> Arc<Self> {
        *self.wrap_reply.lock().unwrap() = Some(reply);
        self
    }

    fn respond(
        &self,
        model: &str,
        messages: &[Message],
        tools: &[ToolSpec],
        with_options: bool,
    ) -> Result<ModelResponse, ProviderError> {
        let last = messages.last().unwrap();
        self.seen.lock().unwrap().push(Seen {
            tools: tools.iter().map(|t| t.name.clone()).collect(),
            with_options,
            last_role: last.role,
            last_content: last.content.clone(),
            model: model.to_string(),
        });
        if self.bad_models.contains(&model) {
            return Err(ProviderError::RateLimited {
                message: "429".into(),
                retry_after: Some(std::time::Duration::from_secs(30)),
            });
        }
        let only_submit = tools.len() == 1 && tools[0].name == "submit_result";
        let reply = if only_submit && self.wrap_reply.lock().unwrap().is_some() {
            self.wrap_reply.lock().unwrap().clone()
        } else {
            self.replies.lock().unwrap().pop_front()
        }
        .unwrap_or_else(|| prose("nothing more to say"));
        let n = self.seen.lock().unwrap().len();
        Ok(ModelResponse {
            reasoning: String::new(),
            reasoning_items: Vec::new(),
            content: reply.content,
            tool_calls: reply
                .calls
                .into_iter()
                .enumerate()
                .map(|(i, (name, args))| ToolCall {
                    id: format!("c{n}-{i}"),
                    name: name.into(),
                    args,
                })
                .collect(),
            usage: Usage::default(),
            quotas: Vec::new(),
        })
    }
}

#[async_trait::async_trait]
impl Provider for Scripted {
    async fn complete(
        &self,
        model: &str,
        messages: &[Message],
        tools: &[ToolSpec],
        _on_event: &mut EventSink<'_>,
    ) -> Result<ModelResponse, ProviderError> {
        self.respond(model, messages, tools, false)
    }

    async fn complete_with(
        &self,
        model: &str,
        messages: &[Message],
        tools: &[ToolSpec],
        _opts: &CompletionOptions,
        _on_event: &mut EventSink<'_>,
    ) -> Result<ModelResponse, ProviderError> {
        self.respond(model, messages, tools, true)
    }
}

struct Pick {
    model: &'static str,
    fallbacks: Vec<String>,
}

#[async_trait::async_trait]
impl Router for Pick {
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
            model: self.model.into(),
            rationale: "test".into(),
            fallbacks: self.fallbacks.clone(),
            pinned: false,
            unroutable: false,
        }
    }
}

fn line_schema() -> Value {
    json!({
        "type": "object",
        "properties": {"line": {"type": "integer"}, "verdict": {"type": "string"}},
        "required": ["line"]
    })
}

struct Ran {
    out: SubagentOutcome,
    seen: Vec<Seen>,
    stored: Vec<(Role, String)>,
}

struct Setup {
    provider: Arc<Scripted>,
    model: &'static str,
    fallbacks: Vec<String>,
    schema: Option<Value>,
    flag: bool,
    max_turns: Option<usize>,
    effort: bool,
}

impl Setup {
    fn new(provider: Arc<Scripted>) -> Self {
        Self {
            provider,
            model: "good::model",
            fallbacks: Vec::new(),
            schema: Some(line_schema()),
            flag: true,
            max_turns: None,
            effort: false,
        }
    }

    async fn run(self) -> Ran {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let child = store
            .create_child_session(".", "default", "parent")
            .unwrap();
        let mut config = Config::default();
        config.harness.subagent_output_schema = self.flag;
        let ctx = AgentCtx {
            provider: self.provider.clone(),
            router: Arc::new(Pick {
                model: self.model,
                fallbacks: self.fallbacks.clone(),
            }),
            store: Arc::clone(&store),
            pricing: Pricing::from_config(&config),
            config,
            mode: PermissionMode::default(),
            rules: Vec::new(),
            depth: 1,
            max_depth: 1,
            agents: Arc::new(HashMap::new()),
            worktree_root: None,
            repo_root: std::path::PathBuf::from("."),
            effective_pin: None,
            inherit_pin: true,
            asker: None,
        };
        let agent = ResolvedAgent {
            name: "general".into(),
            task: "do the thing".into(),
            system_prompt: "you are a subagent".into(),
            max_turns: self.max_turns,
            effort: self.effort.then_some(forge_types::EffortLevel::High),
            output_schema: self.schema,
            ..Default::default()
        };
        let mut sink = |_: StreamEvent| {};
        let decision = route_child(&ctx, &agent, BudgetState::default()).await;
        let out = run_subagent(
            &ctx,
            &child,
            &agent,
            decision,
            BudgetState::default(),
            &mut sink,
        )
        .await
        .unwrap();
        let stored = store
            .load_messages(&child)
            .unwrap()
            .into_iter()
            .map(|m| (m.role, m.content))
            .collect();
        let seen = self.provider.seen.lock().unwrap().clone();
        Ran { out, seen, stored }
    }
}

fn status(ran: &Ran) -> ResultStatus {
    ran.out.structured.as_ref().expect("structured").status
}

fn tool_messages(ran: &Ran) -> Vec<&str> {
    ran.stored
        .iter()
        .filter(|(r, _)| *r == Role::Tool)
        .map(|(_, c)| c.as_str())
        .collect()
}

#[tokio::test]
async fn valid_submit_on_the_first_try() {
    let ran = Setup::new(Scripted::new(vec![submit(json!({"line": 4}))]))
        .run()
        .await;
    assert_eq!(status(&ran), ResultStatus::Valid);
    assert!(ran.out.ok);
    assert_eq!(ran.seen.len(), 1);
    assert_eq!(tool_messages(&ran), ["accepted"]);
    assert_eq!(ran.out.final_text, r#"{"line":4}"#);
    assert!(ran.seen[0].tools.contains(&"submit_result".to_string()));
}

#[tokio::test]
async fn invalid_then_valid_reports_the_path_and_counts_attempts() {
    let ran = Setup::new(Scripted::new(vec![
        submit(json!({"line": "x"})),
        submit(json!({"line": 9})),
    ]))
    .run()
    .await;
    assert_eq!(status(&ran), ResultStatus::Valid);
    assert_eq!(ran.out.structured.as_ref().unwrap().attempts, 2);
    assert_eq!(ran.seen.len(), 2);
    assert_eq!(ran.seen[1].last_role, Role::Tool);
    assert!(
        ran.seen[1]
            .last_content
            .contains("$.line: expected integer"),
        "{}",
        ran.seen[1].last_content
    );
}

#[tokio::test]
async fn always_invalid_stops_after_exactly_three_submits() {
    let bad = || submit(json!({"line": "x"}));
    let ran = Setup::new(Scripted::new(vec![bad(), bad(), bad(), bad(), bad()]))
        .run()
        .await;
    assert_eq!(status(&ran), ResultStatus::Invalid);
    assert!(!ran.out.ok);
    assert_eq!(ran.seen.len(), 3);
    let result = ran.out.structured.unwrap();
    assert_eq!(result.attempts, 3);
    assert!(result.raw.is_some() && !result.errors.is_empty());
    assert!(!ran.out.final_text.contains("Stopped:"));
}

#[tokio::test]
async fn prose_with_json_is_accepted_and_prose_without_is_nudged() {
    let ran = Setup::new(Scripted::new(vec![prose("Here: {\"line\": 3} done")]))
        .run()
        .await;
    assert_eq!(status(&ran), ResultStatus::Valid);

    let ran = Setup::new(Scripted::new(vec![
        prose("I looked around."),
        submit(json!({"line": 1})),
    ]))
    .run()
    .await;
    assert_eq!(status(&ran), ResultStatus::Valid);
    assert_eq!(ran.seen.len(), 2);
    assert_eq!(ran.seen[1].last_role, Role::User);
    assert!(ran.seen[1]
        .last_content
        .contains("without calling submit_result"));
    assert!(ran
        .stored
        .iter()
        .any(|(r, c)| *r == Role::User && c.contains("without calling submit_result")));
}

#[tokio::test]
async fn step_limit_gets_one_submit_only_wrap_up() {
    let mut setup = Setup::new(
        Scripted::new(vec![read_manifest(), read_manifest()])
            .with_wrap_reply(submit(json!({"line": 2}))),
    );
    setup.max_turns = Some(2);
    setup.effort = true;
    let ran = setup.run().await;
    assert_eq!(status(&ran), ResultStatus::Partial);
    assert!(!ran.out.ok);
    assert_eq!(ran.seen.len(), 3);
    let wrap = &ran.seen[2];
    assert_eq!(wrap.tools, ["submit_result"]);
    assert!(!wrap.with_options, "effort is cleared for the wrap-up");
    assert!(wrap.last_content.contains("step budget is used up"));
}

#[tokio::test]
async fn prose_or_empty_wrap_ups_are_invalid_with_raw() {
    for wrap in [prose("I ran out of time but it seems fine."), prose("")] {
        let mut setup = Setup::new(Scripted::new(vec![read_manifest()]).with_wrap_reply(wrap));
        setup.max_turns = Some(1);
        let ran = setup.run().await;
        assert_eq!(status(&ran), ResultStatus::Invalid);
        assert_eq!(ran.seen.len(), 2);
        assert!(!ran.out.ok);
    }
    let mut setup =
        Setup::new(Scripted::new(vec![read_manifest()]).with_wrap_reply(prose("seems fine to me")));
    setup.max_turns = Some(1);
    let raw = setup.run().await.out.structured.unwrap().raw;
    assert_eq!(raw.as_deref(), Some("seems fine to me"));
}

#[tokio::test]
async fn a_progress_guard_stop_leads_to_a_wrap_up() {
    let replies = (0..40).map(|_| read_manifest()).collect();
    let ran = Setup::new(Scripted::new(replies).with_wrap_reply(submit(json!({"line": 5}))))
        .run()
        .await;
    assert_eq!(status(&ran), ResultStatus::Partial);
    let last = ran.seen.last().unwrap();
    assert_eq!(last.tools, ["submit_result"]);
    assert!(last.last_content.contains("stopped making progress"));
    assert!(ran.seen.len() < 12, "{} calls", ran.seen.len());
}

#[tokio::test]
async fn a_failed_last_tool_call_before_a_valid_submit_is_still_ok() {
    let ran = Setup::new(Scripted::new(vec![
        calls(vec![(
            "read_file",
            json!({"path": "/definitely/not/here-xyz"}),
        )]),
        submit(json!({"line": 1})),
    ]))
    .run()
    .await;
    assert_eq!(status(&ran), ResultStatus::Valid);
    assert!(ran.out.ok);
}

#[tokio::test]
async fn calls_after_an_accepted_submit_are_skipped_and_persisted() {
    let ran = Setup::new(Scripted::new(vec![calls(vec![
        ("submit_result", json!({"line": 1})),
        ("read_file", json!({"path": "Cargo.toml"})),
    ])]))
    .run()
    .await;
    assert_eq!(status(&ran), ResultStatus::Valid);
    assert_eq!(
        tool_messages(&ran),
        ["accepted", "skipped: result already submitted"]
    );
    assert_eq!(ran.seen.len(), 1);
}

#[tokio::test]
async fn a_bridged_model_runs_as_a_text_child() {
    let mut setup = Setup::new(Scripted::new(vec![prose("plain answer")]));
    setup.model = "claude-cli::sonnet";
    let ran = setup.run().await;
    assert_eq!(status(&ran), ResultStatus::Text);
    assert_eq!(ran.out.final_text, "plain answer");
    assert!(ran.seen[0].tools.iter().all(|t| t != "submit_result"));
    assert!(ran.out.structured.unwrap().notes[0].contains("bridged"));
}

#[tokio::test]
async fn failover_to_a_bridged_model_drops_the_contract() {
    let provider = Arc::new(Scripted {
        replies: Mutex::new(vec![prose("prose from the bridge")].into()),
        wrap_reply: Mutex::new(None),
        bad_models: vec!["good::model"],
        seen: Mutex::new(Vec::new()),
    });
    let mut setup = Setup::new(provider);
    setup.fallbacks = vec!["claude-cli::sonnet".into()];
    let ran = setup.run().await;
    assert_eq!(status(&ran), ResultStatus::Text);
    assert_eq!(ran.out.final_text, "prose from the bridge");
    assert!(ran.out.structured.unwrap().notes[0].contains("failover"));
    let after = ran.seen.last().unwrap();
    assert_eq!(after.model, "claude-cli::sonnet");
    assert!(after.tools.iter().all(|t| t != "submit_result"));
}

#[tokio::test]
async fn flag_off_ignores_the_schema_entirely() {
    let mut setup = Setup::new(Scripted::new(vec![prose("just prose")]));
    setup.flag = false;
    let ran = setup.run().await;
    assert!(ran.out.structured.is_none());
    assert_eq!(ran.out.final_text, "just prose");
    assert!(ran.seen[0].tools.iter().all(|t| t != "submit_result"));
    assert_eq!(ran.stored[0].0, Role::User);
}

#[tokio::test]
async fn no_schema_with_the_flag_on_changes_nothing() {
    let mut setup = Setup::new(Scripted::new(vec![prose("just prose")]));
    setup.schema = None;
    let ran = setup.run().await;
    assert!(ran.out.structured.is_none());
    assert_eq!(ran.out.final_text, "just prose");
    assert!(ran.seen[0].tools.iter().all(|t| t != "submit_result"));
}

#[tokio::test]
async fn contract_only_adds_submit_result_to_the_toolset() {
    let with = Setup::new(Scripted::new(vec![submit(json!({"line": 1}))]))
        .run()
        .await;
    let mut setup = Setup::new(Scripted::new(vec![prose("x")]));
    setup.schema = None;
    let without = setup.run().await;
    let mut offered = with.seen[0].tools.clone();
    offered.retain(|t| t != "submit_result");
    assert_eq!(offered, without.seen[0].tools);
}

fn orchestrate_ctx(provider: Arc<Scripted>, flag: bool) -> AgentCtx {
    let mut config = Config::default();
    config.harness.subagent_output_schema = flag;
    AgentCtx {
        provider,
        router: Arc::new(Pick {
            model: "good::model",
            fallbacks: Vec::new(),
        }),
        store: Arc::new(Store::open_in_memory().unwrap()),
        pricing: Pricing::from_config(&config),
        config,
        mode: PermissionMode::default(),
        rules: Vec::new(),
        depth: 0,
        max_depth: 0,
        agents: Arc::new(HashMap::new()),
        worktree_root: None,
        repo_root: std::path::PathBuf::from("."),
        effective_pin: None,
        inherit_pin: true,
        asker: None,
    }
}

fn request(task: &str, schema: Option<Value>) -> AgentRequest {
    AgentRequest {
        agent: "general-purpose".into(),
        task: task.into(),
        output_schema: schema,
    }
}

#[tokio::test]
async fn a_batch_with_a_schema_returns_a_json_envelope() {
    let ctx = orchestrate_ctx(Scripted::new(vec![submit(json!({"line": 7}))]), true);
    let mut events = Vec::new();
    let mut sink = |e: Lifecycle| {
        if let Lifecycle::Done { summary, .. } = e {
            events.push(summary.to_string());
        }
    };
    let (text, ok) = orchestrate(
        &ctx,
        "parent",
        vec![request("find it", Some(line_schema()))],
        BudgetState::default(),
        1,
        &mut sink,
    )
    .await
    .unwrap();
    assert!(ok);
    let v: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["agents"][0]["status"], "valid");
    assert_eq!(v["agents"][0]["data"]["line"], 7);
    assert_eq!(events, ["✓ valid · line=7"]);
}

#[tokio::test]
async fn a_bad_schema_returns_an_error_and_creates_no_child_session() {
    let ctx = orchestrate_ctx(Scripted::new(vec![]), true);
    let mut sink = |_: Lifecycle| {};
    let (text, ok) = orchestrate(
        &ctx,
        "parent",
        vec![request("t", Some(json!({"$ref": "#/x"})))],
        BudgetState::default(),
        1,
        &mut sink,
    )
    .await
    .unwrap();
    assert!(!ok);
    assert!(
        text.starts_with("error: output_schema for agent 1"),
        "{text}"
    );
    assert!(ctx.store.child_sessions("parent").unwrap().is_empty());
}

#[tokio::test]
async fn flag_off_batches_stay_labelled_prose() {
    let ctx = orchestrate_ctx(Scripted::new(vec![prose("child said hi")]), false);
    let mut sink = |_: Lifecycle| {};
    let (text, ok) = orchestrate(
        &ctx,
        "parent",
        vec![request("t", Some(line_schema()))],
        BudgetState::default(),
        1,
        &mut sink,
    )
    .await
    .unwrap();
    assert!(ok);
    assert_eq!(text, "[agent 1: general-purpose]\nchild said hi");
}

#[tokio::test]
async fn flag_on_without_a_schema_stays_labelled_prose() {
    let ctx = orchestrate_ctx(Scripted::new(vec![prose("child said hi")]), true);
    let mut sink = |_: Lifecycle| {};
    let (text, _) = orchestrate(
        &ctx,
        "parent",
        vec![request("t", None)],
        BudgetState::default(),
        1,
        &mut sink,
    )
    .await
    .unwrap();
    assert_eq!(text, "[agent 1: general-purpose]\nchild said hi");
}

#[test]
fn advertised_spec_differs_only_by_the_output_schema_delta() {
    let plain = spawn_agents_spec(4);
    let mut flagged = plain.clone();
    structured::advertise_output_schema(&mut flagged);
    assert!(flagged.schema["properties"]["output_schema"].is_object());
    assert!(
        flagged.schema["properties"]["agents"]["items"]["properties"]["output_schema"].is_object()
    );
    let mut stripped = flagged.clone();
    stripped.schema["properties"]
        .as_object_mut()
        .unwrap()
        .remove("output_schema");
    stripped.schema["properties"]["agents"]["items"]["properties"]
        .as_object_mut()
        .unwrap()
        .remove("output_schema");
    assert_eq!(stripped.schema, plain.schema);
    assert!(flagged.description.starts_with(&plain.description));
}
