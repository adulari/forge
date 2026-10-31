//! Subagent orchestration (RFC subagent-orchestration): the `spawn_agents` tool lets the
//! top-level model delegate subtasks to **child agents**, each in its own isolated context
//! and **routed independently through the Model Mesh** — so a Complex parent can fan out
//! cheap Trivial children.
//!
//! `spawn_agents` is a *virtual tool*: it is advertised to the parent model but is not a
//! `forge_tools::Tool` (it needs the provider/router/store, which ordinary tools can't reach).
//! [`orchestrate`] is the presenter-agnostic driver; [`Session`](crate::Session) calls it for a
//! native API turn (lifecycle → TUI events) and `forge mcp-serve` calls it for a CLI-bridge
//! (claude/codex) turn (RFC subagent-orchestration Phase 3).
//!
//! Children run **concurrently** (bounded by `max_concurrency`), each as a persisted child
//! session linked to the parent. A child's toolset comes from its agent type (default:
//! read-only `read_file`/`list_dir`/`search`). Recursive delegation is disabled by default and
//! bounded by `mesh.subagents.max_depth` when explicitly enabled. Named agent types load from
//! `.forge/agents/*.md` (system prompt + optional tool subset + optional pinned tier);
//! unknown/inline agents use the built-in `general-purpose` type (every tool) and are mesh-routed.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use forge_config::{AgentDef, Config};
use forge_mesh::pricing::Pricing;
use forge_mesh::{BudgetState, Router, RoutingDecision};
use forge_provider::{Provider, StreamEvent, ToolSpec};
use forge_store::Store;
use forge_tools::ToolRegistry;
use forge_types::{Message, PermissionMode, PermissionRule, Role, TaskTier};

use crate::{worktree, CoreError};

/// The virtual tool name the parent model calls to delegate subtasks.
pub const SPAWN_AGENTS_TOOL: &str = "spawn_agents";

/// The built-in agent type a request with no (or an unknown) `agent` resolves to: every tool, the
/// parent's temper.
pub const GENERAL_PURPOSE_AGENT: &str = "general-purpose";

const SUBAGENT_SYSTEM: &str = "You are a focused Forge subagent working on ONE delegated \
    subtask. Do the task with the tools you have, editing files and running commands when it \
    calls for it, and return a concise, self-contained answer — just the result the parent \
    agent needs, no preamble. If a tool call is refused, say so plainly in your answer.";

/// A permission question a child raises when its tool call needs approval.
#[derive(Debug, Clone)]
pub struct ChildQuestion {
    /// The child's display name, so the prompt can say who is asking.
    pub child: String,
    pub tool: String,
    pub side_effect: forge_types::SideEffect,
    pub args: serde_json::Value,
}

/// The parent surface's answer to a [`ChildQuestion`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChildAnswer {
    Allow,
    /// With the note the user typed, if any.
    Deny(Option<String>),
}

/// How a child reaches someone who can approve a call the permission gate says to ask about. The
/// parent turn is blocked inside its `spawn_agents` call, so its own prompt can host the question.
/// Absent for headless contexts, where the call is refused with a reason instead.
pub type ChildAsker = Arc<
    dyn Fn(
            ChildQuestion,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ChildAnswer> + Send>>
        + Send
        + Sync,
>;

mod detached;
pub use detached::{
    format_admission, format_subagent_list, resolve_detached_address, spawn_detached,
    DetachedRegistry, DetachedSink,
};
mod execution;
pub use execution::sum_usage;
use execution::{execute_tool, tool_result_failed};

mod progress;
mod requests;
mod resume_repair;
pub(crate) mod structured;
#[cfg(test)]
mod structured_tests;
#[cfg(test)]
mod write_tests;
pub use requests::{
    cancel_subagent_spec, effective_tools, find_agent, is_write_capable, list_subagents_spec,
    narrowed_mode, parse_detached_flag, parse_requests, resolve, resolve_child_address,
    rewrite_args_for_root, send_to_agent_spec, spawn_agents_spec, spawn_agents_spec_for,
    spawn_agents_spec_with, system_prompt_with_skills, AdmissionHandle, AgentRequest,
    ResolvedAgent, CANCEL_SUBAGENT_TOOL, LIST_SUBAGENTS_TOOL, SEND_TO_AGENT_TOOL,
};

/// Shared, cheaply-cloneable machinery a subagent needs — the same backends the parent uses.
#[derive(Clone)]
pub struct AgentCtx {
    pub provider: Arc<dyn Provider>,
    pub router: Arc<dyn Router>,
    pub store: Arc<Store>,
    pub config: Config,
    pub pricing: Pricing,
    pub mode: PermissionMode,
    pub rules: Vec<PermissionRule>,
    /// Nesting level of *this* context's children (0 = the top-level turn's children). Children
    /// may themselves spawn iff `depth < max_depth` (RFC subagent-orchestration Phase 3c).
    pub depth: usize,
    pub max_depth: usize,
    /// Loaded agent types, shared so a recursing child can resolve named agents too.
    pub agents: Arc<HashMap<String, AgentDef>>,
    /// When worktree isolation is active for this child, the root of its isolated worktree.
    /// `None` for the top-level context and for read-only children.
    pub worktree_root: Option<PathBuf>,
    /// The git repo root for this session (used to create/merge worktrees).
    pub repo_root: PathBuf,
    /// The effective pin set for this turn's children: the session `/model` pin if set, else the
    /// router's own `--model` pin set. `None` = no pin, children route via the mesh independently.
    /// Used by [`route_child`] to decide whether (and which) subagents inherit the parent's pin.
    pub effective_pin: Option<Vec<String>>,
    /// Whether children inherit `effective_pin` at all: the session's `/subagents free|pinned`
    /// override resolved against `mesh.subagents.inherit_pin`. Resolved once by the parent rather
    /// than read back off `config` here, so the runtime command and the config default meet in ONE
    /// place and a child can never disagree with the session that spawned it.
    pub inherit_pin: bool,
    /// Where a child's `Ask` decision is sent. `None` = nobody can be asked: the call is refused.
    pub asker: Option<ChildAsker>,
}

/// The result of running one child agent.
#[derive(Debug, Clone)]
pub struct SubagentOutcome {
    pub final_text: String,
    pub ok: bool,
    /// Set when the parent asked for a schema-validated result.
    pub(crate) structured: Option<structured::StructuredResult>,
}

/// Run one child agent to completion against `child_id` (a persisted child session): route the
/// task independently, run the model↔tool loop with read-only tools, persist messages + usage
/// to the child session (so its cost rolls into the shared budget), and return the answer.
/// Route one child's task through the mesh (deterministic, no model API call): an agent type may
/// pin a tier; otherwise the task is routed around currently-benched models. Used both inside
/// [`run_subagent`] and by [`orchestrate`] up front, so the live panel can show the child's model
/// the moment it starts (not only when it finishes).
pub async fn route_child(
    ctx: &AgentCtx,
    agent: &ResolvedAgent,
    budget: BudgetState,
) -> RoutingDecision {
    if let Some(model) = agent.pinned_model.as_deref() {
        // A hard model pin (e.g. one /duel candidate) bypasses the mesh entirely and wins over any
        // session/router pin. /duel pins each candidate to a specific model in order to COMPARE
        // them; letting an inherited session pin override it would collapse every duel candidate
        // onto one model and make the feature meaningless. So this explicit per-child pin always
        // takes precedence over the effective session pin.
        return RoutingDecision {
            effort: None,
            tier: agent.tier.unwrap_or(TaskTier::Standard),
            model: model.to_string(),
            rationale: "pinned by the agent type or /duel".to_string(),
            fallbacks: Vec::new(),
            // A duel candidate is a hard, explicit pin: switching models mid-duel would
            // invalidate the comparison, so the pinned rate-limit backoff applies here too.
            pinned: true,
            unroutable: false,
        };
    }
    // When a pin is active for the turn and subagent inheritance is on (the default), the child
    // is restricted to the EXACT same model set as the parent — ranked within the set by the mesh
    // (so a set of two or more keeps an intra-set failover chain, and a single pin is strict with
    // no cross-model fallback). An explicit user pin beats an agent-type tier default.
    // `agent.pinned_model` (handled above) is the one exception.
    if let Some(pin) = ctx.effective_pin.as_deref() {
        if ctx.inherit_pin {
            let readiness = crate::readiness::ProviderReadiness::snapshot(&ctx.config, &ctx.store);
            let health = readiness.health;
            let quota = readiness.quota;
            let project = crate::project_context::compute(&ctx.repo_root);
            return ctx
                .router
                .route_with_pin_set(
                    pin,
                    &agent.task,
                    false,
                    budget,
                    &health,
                    &quota,
                    None,
                    &project,
                )
                .await;
        }
    }
    match agent
        .tier
        .and_then(|t| ctx.config.model_for(t).map(|m| (t, m)))
    {
        Some((tier, model)) => RoutingDecision {
            effort: None,
            tier,
            model: model.to_string(),
            rationale: format!("pinned by agent type '{}'", agent.name),
            fallbacks: Vec::new(),
            // An agent-type tier default, not an explicit user pin — normal failover rules.
            pinned: false,
            unroutable: false,
        },
        // Route around benched models too (docs/features/mesh-routing.md): a child still avoids a
        // model the parent just rate-limited. When pin inheritance is disabled, route through the
        // mesh WITHOUT the router's own pin so the child's normal failover chain is intact instead
        // of being forced onto the parent's `--model`.
        None => {
            let readiness = crate::readiness::ProviderReadiness::snapshot(&ctx.config, &ctx.store);
            let health = readiness.health;
            let quota = readiness.quota;
            let project = crate::project_context::compute(&ctx.repo_root);
            if ctx.inherit_pin {
                ctx.router
                    .route(&agent.task, false, budget, &health, &quota, None, &project)
                    .await
            } else {
                ctx.router
                    .route_unpinned(&agent.task, false, budget, &health, &quota, None, &project)
                    .await
            }
        }
    }
}

pub async fn run_subagent(
    ctx: &AgentCtx,
    child_id: &str,
    agent: &ResolvedAgent,
    // Routed ONCE by the caller (orchestrate) — that's the model its per-provider concurrency permit
    // was acquired for. Re-routing here could pick a DIFFERENT provider (if another child benched the
    // first in between), so the child would hold provider A's permit while hammering provider B,
    // silently bypassing B's cap. Take the decision as a parameter to keep route↔permit consistent.
    decision: RoutingDecision,
    budget: BudgetState,
    on_delta: &mut (dyn FnMut(StreamEvent) + Send),
) -> Result<SubagentOutcome, CoreError> {
    let task = agent.task.as_str();
    // `decision` is a parameter (routed once by the caller); no second route_child here.
    let system = child_system_prompt(ctx, agent);
    let transcript = vec![Message::system(&system), Message::user(task)];
    ctx.store.add_message(child_id, 0, Role::User, task, None)?;
    run_agent_loop(
        ctx, child_id, agent, decision, budget, transcript, 1, on_delta,
    )
    .await
}

/// Continue a previously-run child agent with a follow-up message (persistent subagents,
/// gap-analysis #12): the child's transcript is rebuilt from its persisted session — nothing
/// was retained in memory, so this works across parent turns AND across a resumed parent
/// session — the follow-up is appended, and the same model↔tool loop runs again. The system
/// prompt is re-resolved from the agent definition (it was never persisted), so a named agent
/// type keeps its persona; an ad-hoc child gets the default subagent persona back.
pub async fn resume_subagent(
    ctx: &AgentCtx,
    child_id: &str,
    agent: &ResolvedAgent,
    follow_up: &str,
    decision: RoutingDecision,
    budget: BudgetState,
    on_delta: &mut (dyn FnMut(StreamEvent) + Send),
) -> Result<SubagentOutcome, CoreError> {
    resume_repair::close_dangling_tool_calls(&ctx.store, child_id)?;
    let stored = ctx.store.load_messages(child_id)?;
    let mut transcript = Vec::with_capacity(stored.len() + 2);
    transcript.push(Message::system(child_system_prompt(ctx, agent)));
    transcript.extend(stored.into_iter().map(resume_repair::replay_message));
    transcript.push(Message::user(follow_up));
    let seq = ctx.store.next_message_seq(child_id)?;
    ctx.store
        .add_message(child_id, seq, Role::User, follow_up, None)?;
    run_agent_loop(
        ctx,
        child_id,
        agent,
        decision,
        budget,
        transcript,
        seq + 1,
        on_delta,
    )
    .await
}

fn child_system_prompt(ctx: &AgentCtx, agent: &ResolvedAgent) -> String {
    system_prompt_with_skills(agent, &ctx.repo_root, ctx.config.commands.trust_project)
}

/// The child model↔tool loop shared by a fresh [`run_subagent`] and a [`resume_subagent`]
/// follow-up: same toolset resolution, same failover chain, same persistence — only the
/// starting transcript and message-seq offset differ.
#[allow(clippy::too_many_arguments)]
async fn run_agent_loop(
    ctx: &AgentCtx,
    child_id: &str,
    agent: &ResolvedAgent,
    decision: RoutingDecision,
    budget: BudgetState,
    mut transcript: Vec<Message>,
    seq_start: i64,
    on_delta: &mut (dyn FnMut(StreamEvent) + Send),
) -> Result<SubagentOutcome, CoreError> {
    // The agent type may widen the toolset beyond read-only; writes/shell still pass the
    // permission gate (where Ask→Deny in a child) under the agent's temper, which can only be
    // stricter than the parent's. The child can itself spawn subagents only while there is depth
    // budget left (RFC subagent-orchestration Phase 3c) and its type does not disallow it.
    let narrowed = AgentCtx {
        mode: narrowed_mode(ctx.mode, agent.mode),
        ..ctx.clone()
    };
    let ctx = &narrowed;
    let child_label = format!("{}-{}", agent.name, &child_id[..child_id.len().min(4)]);
    let full = ToolRegistry::with_core_tools();
    let can_recurse = ctx.depth < ctx.max_depth
        && !agent
            .disallowed_tools
            .iter()
            .any(|t| t == SPAWN_AGENTS_TOOL);
    let allowed = effective_tools(agent, &full);
    let mut specs: Vec<ToolSpec> = allowed
        .iter()
        .filter_map(|name| full.get(name))
        .map(|t| ToolSpec {
            name: t.name().to_string(),
            description: t.description().to_string(),
            schema: t.schema(),
        })
        .collect();
    if can_recurse {
        let mut spec = spawn_agents_spec(ctx.config.mesh.subagents.max_agents);
        if ctx.config.harness.subagent_output_schema {
            structured::advertise_output_schema(&mut spec);
        }
        specs.push(spec);
    }
    let (mut contract, mut schema_note) = structured::attach(
        agent.output_schema.as_ref(),
        ctx.config.harness.subagent_output_schema,
        &decision.model,
        &mut specs,
        &mut transcript[0],
    );

    let mut seq: i64 = seq_start;
    let mut next_seq = || {
        let n = seq;
        seq += 1;
        n
    };

    let mut final_text = String::new();
    let mut ok = true;

    // Failover (docs/features/mesh-routing.md): subagents fail over down the routed chain too, so a
    // child whose model rate-limits/stalls doesn't kill the whole spawn.
    let failover_enabled = ctx.config.mesh.failover;
    let default_cooldown = std::time::Duration::from_secs(ctx.config.mesh.failover_cooldown_secs);
    let stream_idle = std::time::Duration::from_secs(ctx.config.mesh.stream_idle_timeout_secs);
    let mut chain = decision.fallbacks.clone().into_iter();
    let mut active_model = decision.model.clone();

    let step_limit = progress::step_limit(agent.max_turns, ctx.config.mesh.max_steps);
    let mut supervisor = progress::ChildSupervisor::new(
        &ctx.config,
        ctx.worktree_root
            .clone()
            .unwrap_or_else(|| ctx.repo_root.clone()),
    );
    let mut stalled: Option<String> = None;
    'steps: for step in 0usize.. {
        if contract
            .as_ref()
            .is_some_and(structured::Contract::wrap_spent)
        {
            break;
        }
        if step_limit.is_some_and(|limit| step >= limit) {
            let armed = contract.as_mut().is_some_and(|c| {
                c.wrap_armed() || c.arm_wrap_up(structured::WrapReason::StepLimit)
            });
            if !armed {
                break;
            }
        }
        let wrap_specs = structured::wrap_up_specs(
            contract.as_ref(),
            &mut transcript,
            &ctx.store,
            child_id,
            &mut next_seq,
        )?;
        let wrapping = !wrap_specs.is_empty();
        // Forward the child's streamed deltas so the orchestrator can show live per-child
        // activity (RFC subagent-orchestration Phase 3b), with transparent failover.
        let mut resp = loop {
            // A model that failed over to a bridge cannot be handed `submit_result`.
            if let Some(note) =
                structured::drop_for_bridge(&mut contract, &mut specs, &active_model)
            {
                schema_note = Some(note);
            }
            if wrapping && contract.is_none() {
                // The wrap-up was the child's last call; a bridged model would get the full toolset.
                break 'steps;
            }
            let step_specs: &[ToolSpec] = if wrapping { &wrap_specs } else { &specs };
            on_delta(StreamEvent::Reasoning(format!(
                "model step {}: waiting for `{active_model}`",
                progress::step_label(step, step_limit)
            )));
            let activity = Arc::new(std::sync::atomic::AtomicU64::new(0));
            let sink_activity = Arc::clone(&activity);
            let mut sink = |ev: StreamEvent| {
                sink_activity.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                on_delta(ev);
            };
            // The agent type's `effort`, resolved against the routed model's own ladder.
            let opts = forge_provider::CompletionOptions {
                effort: agent.effort.filter(|_| !wrapping).and_then(|level| {
                    forge_types::effort::resolve_id(&active_model, Some(level)).sent
                }),
                ..Default::default()
            };
            let completion = if opts.effort.is_some() {
                ctx.provider
                    .complete_with(&active_model, &transcript, step_specs, &opts, &mut sink)
            } else {
                ctx.provider
                    .complete(&active_model, &transcript, step_specs, &mut sink)
            };
            let idle = crate::effective_stream_idle(&*ctx.provider, &active_model, stream_idle);
            let first_event = if idle > stream_idle {
                idle
            } else {
                std::time::Duration::from_secs(ctx.config.mesh.first_event_timeout_secs)
            };
            match crate::stream_with_first_event_timeout(
                completion,
                &activity,
                None,
                idle,
                first_event,
            )
            .await
            {
                Ok(r) => break r,
                Err(e) if failover_enabled && e.is_retryable() => {
                    let _ = ctx.store.bench_for(
                        &active_model,
                        e.cooldown(default_cooldown),
                        e.reason(),
                    );
                    match chain.next() {
                        Some(next) => {
                            on_delta(StreamEvent::Reasoning(format!(
                                "provider `{active_model}` failed ({e}); failing over to `{next}`"
                            )));
                            tracing::debug!("subagent failover {active_model} -> {next}: {e}");
                            active_model = next;
                            continue;
                        }
                        None => return Err(e.into()),
                    }
                }
                Err(e) => return Err(e.into()),
            }
        };
        // `cost_for_usage`, not the bare rate: it bills a subscription (codex) call at $0 and
        // prices cache hits at their discounted rate, exactly like the parent turn.
        resp.usage.cost_usd = ctx.pricing.cost_for_usage(&active_model, &resp.usage);
        if let Some(c) = contract.as_mut().filter(|_| wrapping) {
            c.spend_wrap_up();
        }

        transcript.push(Message::assistant_tool_calls(
            &resp.content,
            resp.tool_calls.clone(),
        ));
        let msg_id = ctx.store.add_message_full(
            child_id,
            next_seq(),
            Role::Assistant,
            &resp.content,
            Some(&active_model),
            &resp.tool_calls,
            None,
        )?;
        if step == 0 {
            ctx.store
                .record_routing(&msg_id, decision.tier, &active_model, &decision.rationale)?;
        }
        ctx.store
            .record_usage(child_id, &msg_id, &resp.usage, Some(&active_model))?;

        if !resp.wants_tools() {
            let Some(c) = contract.as_mut() else {
                final_text = resp.content;
                break;
            };
            match c.on_text(&resp.content) {
                structured::Submit::Rejected(nudge) => {
                    structured::push_nudge(
                        &mut transcript,
                        &ctx.store,
                        child_id,
                        next_seq(),
                        &nudge,
                    )?;
                    continue;
                }
                structured::Submit::Accepted | structured::Submit::Exhausted(_) => {
                    final_text = resp.content;
                    break;
                }
            }
        }

        let mut observations = Vec::new();
        let mut submit_seen = false;
        let mut finished = false;
        for call in &resp.tool_calls {
            if let Some(c) = contract.as_mut() {
                let taken = structured::Turn {
                    msg_id: &msg_id,
                    wrapping,
                    finished,
                    submit_seen: &mut submit_seen,
                };
                if let Some((text, done)) = structured::intercept(c, &ctx.store, call, taken)? {
                    finished |= done;
                    ctx.store.add_message_full(
                        child_id,
                        next_seq(),
                        Role::Tool,
                        &text,
                        None,
                        &[],
                        Some(&call.id),
                    )?;
                    transcript.push(Message::tool_result(&call.id, text));
                    continue;
                }
            }
            on_delta(StreamEvent::Reasoning(format!(
                "model step {}: running tool `{}`",
                progress::step_label(step, step_limit),
                call.name
            )));
            let result = if call.name == SPAWN_AGENTS_TOOL && can_recurse {
                // The child delegates further: recurse one level deeper (bounded by max_depth).
                // Grandchildren aren't shown in the live panel — they roll up into this result.
                run_nested_spawn(ctx.clone(), child_id.to_string(), call.args.clone(), budget).await
            } else {
                execute_tool(ctx, &full, &allowed, &child_label, &msg_id, call).await?
            };
            // Last outcome wins, not a permanent latch: a child that hits one failing tool call
            // but RECOVERS with a later successful one is not a failed child. (Seen live in a
            // workflow run: an agent's first read_file errored on a bad path, it recovered by
            // reading the right file and answering well — the old latch still marked it ✗, while
            // siblings that failed without ever touching a tool showed ✓.)
            let failed = tool_result_failed(&result);
            ok = !failed;
            let writes = full
                .get(&call.name)
                .is_some_and(|tool| tool.side_effect() == forge_types::SideEffect::Write);
            observations.push(crate::loop_progress::CallObservation::for_call(
                call, !failed, writes, &result,
            ));
            on_delta(StreamEvent::Reasoning(format!(
                "model step {}: tool `{}` {}",
                progress::step_label(step, step_limit),
                call.name,
                if failed {
                    "failed; continuing"
                } else {
                    "finished"
                }
            )));
            ctx.store.add_message_full(
                child_id,
                next_seq(),
                Role::Tool,
                &result,
                None,
                &[],
                Some(&call.id),
            )?;
            transcript.push(Message::tool_result(&call.id, result));
        }
        if finished || wrapping {
            final_text = resp.content.clone();
            break;
        }
        if submit_seen && observations.is_empty() {
            continue;
        }
        if let Some(headline) = supervisor.observe(
            &resp.content,
            &observations,
            &mut transcript,
            &mut *on_delta,
        ) {
            final_text = resp.content.clone();
            stalled = Some(headline);
            if contract
                .as_mut()
                .is_some_and(|c| c.arm_wrap_up(structured::WrapReason::Stalled))
            {
                continue;
            }
            break;
        }
    }

    if let Some(contract) = contract {
        return Ok(structured::outcome_from(contract, schema_note));
    }
    if let Some(headline) = stalled {
        return Ok(structured::with_note(
            progress::stalled_outcome(final_text, &headline),
            schema_note,
            true,
        ));
    }

    // The model produced no usable final answer — either it kept calling tools until the step
    // cap, or it answered with a blank/whitespace-only string (`is_empty()` alone missed "\n").
    // Don't report that as an empty SUCCESS — the parent model would assemble a blank
    // `[agent N]` block and proceed as if the child finished its task.
    if final_text.trim().is_empty() {
        ok = false;
        final_text = progress::no_answer_error(step_limit);
    }

    Ok(structured::with_note(
        SubagentOutcome {
            final_text,
            ok,
            structured: None,
        },
        schema_note,
        false,
    ))
}

/// A child agent delegating further: recurse one level deeper. Grandchildren are not surfaced
/// to the live UI (no-op lifecycle) — their results roll up into this child's tool result.
/// `Box::pin` breaks the orchestrate→run_subagent→orchestrate async-recursion type cycle.
/// Returns a **boxed** (concrete, non-opaque) future so it does not participate in the
/// orchestrate→run_subagent async-`impl Future` opaque-type cycle; `+ Send` asserts Send at the
/// boundary. Owned args so the future is `'static`.
fn run_nested_spawn(
    ctx: AgentCtx,
    parent_id: String,
    args: serde_json::Value,
    budget: BudgetState,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = String> + Send>> {
    Box::pin(async move {
        let max = ctx.config.mesh.subagents.max_agents;
        let requests = match parse_requests(&args, max) {
            Ok(r) => r,
            Err(e) => return format!("error: {e}"),
        };
        let deeper = AgentCtx {
            depth: ctx.depth + 1,
            // Nested spawns start without a pre-allocated worktree; orchestrate will create
            // one per child if worktree_isolation is enabled.
            worktree_root: None,
            ..ctx.clone()
        };
        let concurrency = ctx.config.mesh.subagents.max_concurrency;
        let mut noop = |_: Lifecycle| {};
        match orchestrate(
            &deeper,
            &parent_id,
            requests,
            budget,
            concurrency,
            &mut noop,
        )
        .await
        {
            Ok((combined, _)) => combined,
            Err(e) => format!("error: nested subagents failed: {e}"),
        }
    })
}

/// A message from a child task to the orchestrator's drain loop: a live activity delta, or the
/// final completion.
enum ChildMsg {
    Progress { index: usize, snippet: String },
    Done(ChildDone),
}

/// One subagent's completion, sent from its task back to the orchestrator's drain loop.
struct ChildDone {
    index: usize,
    id: String,
    agent: String,
    text: String,
    ok: bool,
    cost: f64,
    structured: Option<structured::StructuredResult>,
}

/// A subagent lifecycle event, surfaced to whatever is driving the orchestration (the TUI
/// presenter for a native turn, or a headless logger for the CLI-bridge `mcp-serve` path).
pub enum Lifecycle<'a> {
    Start {
        id: &'a str,
        agent: &'a str,
        task: &'a str,
        /// The model this child routed to (shown in the live panel from the moment it starts).
        model: &'a str,
    },
    /// A live activity snippet (streamed text/reasoning) from a still-running child.
    Progress { id: &'a str, snippet: &'a str },
    Done {
        id: &'a str,
        agent: &'a str,
        ok: bool,
        summary: &'a str,
        cost_usd: f64,
    },
}

/// Run a batch of subagents concurrently (bounded by `max_concurrency`) under `parent_id`,
/// reporting each child's lifecycle through `on_event`, and return the combined labeled result
/// plus whether all succeeded. Presenter-agnostic so both [`crate::Session`] (TUI events) and
/// `forge mcp-serve` (the CLI-bridge path) reuse one orchestrator (RFC subagent-orchestration).
#[allow(clippy::too_many_arguments)]
pub async fn orchestrate(
    ctx: &AgentCtx,
    parent_id: &str,
    requests: Vec<AgentRequest>,
    budget: BudgetState,
    max_concurrency: usize,
    on_event: &mut (dyn FnMut(Lifecycle) + Send),
) -> Result<(String, bool), CoreError> {
    use tokio::sync::{mpsc, Mutex, Semaphore};

    let wants = match structured::preflight(&ctx.config.harness, &requests) {
        Ok(wants) => wants,
        Err(message) => return Ok((message, false)),
    };
    let mode_label = format!("{:?}", ctx.mode);
    let n = requests.len();
    let sem = Arc::new(Semaphore::new(max_concurrency.max(1)));
    let (tx, mut rx) = mpsc::unbounded_channel::<ChildMsg>();
    // Indexed by request position (not push order) so a mid-batch skip below can't desync `ids`
    // from the `index` values already-spawned children send back over `tx`.
    let mut ids: Vec<String> = vec![String::new(); n];
    let mut all_ok = true;

    // Tokio does NOT cascade-cancel tasks spawned inside an aborted task. Without this guard, a
    // user interrupt (Esc) aborts the turn — dropping this function's future — while every
    // already-spawned child keeps running to completion in the background: still calling models
    // (spend), still executing write/shell tools, still merging worktree branches back into the
    // shared tree, all AFTER the user believed the operation was cancelled. Holding the handles
    // in a drop-guard aborts the children the moment this future is dropped; on normal
    // completion the tasks are already finished and `abort()` is a no-op.
    struct AbortChildrenOnDrop(Vec<tokio::task::JoinHandle<()>>);
    impl Drop for AbortChildrenOnDrop {
        fn drop(&mut self) {
            for handle in &self.0 {
                handle.abort();
            }
        }
    }
    let mut child_tasks = AbortChildrenOnDrop(Vec::with_capacity(n));

    // Serialize merge-back across concurrently-finishing children so `git apply` doesn't race on
    // the index. One merge at a time is fine: the patch itself is generated from the branch diff,
    // not the index, so ordering is deterministic.
    let merge_lock: Arc<Mutex<()>> = Arc::new(Mutex::new(()));

    let isolation_enabled = ctx.config.mesh.subagents.worktree_isolation;
    // Emit a one-time warning when isolation is requested but the cwd is not a git repo.
    let repo_is_git = if isolation_enabled {
        let ok = worktree::is_git_repo(&ctx.repo_root);
        if !ok {
            tracing::warn!(
                "worktree_isolation is enabled but {:?} is not a git repo — \
                 running without worktree isolation",
                ctx.repo_root
            );
        }
        ok
    } else {
        false
    };

    let full_registry = ToolRegistry::with_core_tools();

    // Per-provider concurrency sub-cap: a burst of children all routed to ONE provider (a
    // claude/codex subscription bridge, or a single metered key) would otherwise run in parallel up
    // to the global `max_concurrency` and hammer that one quota. Each child also acquires a permit
    // from its provider's semaphore (sized by `max_per_provider`), so same-provider fan-out is
    // throttled while different providers still run in parallel. `0` disables the sub-cap.
    let max_per_provider = ctx.config.mesh.subagents.max_per_provider;
    let mut provider_sems: std::collections::HashMap<String, Arc<Semaphore>> =
        std::collections::HashMap::new();

    // Create each child session + announce Start up front (so a UI shows the whole batch as
    // running immediately), then spawn the work bounded by a concurrency permit.
    for (i, req) in requests.into_iter().enumerate() {
        let mut resolved = resolve(&req, &ctx.agents);
        // A `?` here would abort the whole function — dropping `rx` and silently discarding the
        // results (and spend) of every sibling child already `tokio::spawn`ed above. Skip just
        // this request instead so the rest of the batch still runs and reports.
        let child_cwd = ctx.repo_root.to_string_lossy();
        let child_id = match ctx
            .store
            .create_child_session(&child_cwd, &mode_label, parent_id)
        {
            Ok(id) => id,
            Err(e) => {
                tracing::warn!(
                    "create_child_session failed for subagent request {i}: {e} — skipping this \
                     agent, other children continue"
                );
                all_ok = false;
                continue;
            }
        };
        // Name the child session after its resolved agent — the address `send_to_agent` resolves
        // for follow-ups. Best-effort: an unnamed child is still reachable by id prefix.
        let _ = ctx.store.set_session_title(&child_id, &resolved.name);
        // Route up front (deterministic, no API call) so the panel shows the model immediately AND
        // the child runs against the SAME model its provider permit is acquired for (routed once).
        let decision = route_child(ctx, &resolved, budget).await;
        let child_model = decision.model.clone();
        on_event(Lifecycle::Start {
            id: &child_id,
            agent: &resolved.name,
            task: &resolved.task,
            model: &child_model,
        });
        resolved.task = crate::hooks_subagent::subagent_start_task(
            &ctx.config.hooks,
            parent_id,
            &child_id,
            &resolved.name,
            &ctx.repo_root,
            std::mem::take(&mut resolved.task),
        )
        .await;
        ids[i] = child_id.clone();

        // Resolve this child's provider semaphore (get-or-create), sized by `max_per_provider`.
        // Done on the orchestrator (the routing model is already known) so the task just acquires it.
        let provider_sem = if max_per_provider > 0 {
            let provider = forge_config::provider_of(&child_model).to_string();
            Some(Arc::clone(provider_sems.entry(provider).or_insert_with(
                || Arc::new(Semaphore::new(max_per_provider)),
            )))
        } else {
            None
        };

        // Decide whether this child gets an isolated worktree. We create the WorktreeGuard here
        // (on the orchestrator task, before spawning) so any creation error is visible immediately
        // and doesn't kill an already-running child task.
        // An agent type with `isolation: worktree` is isolated even when the config is off.
        let wants_worktree = (isolation_enabled && repo_is_git)
            || (resolved.worktree && worktree::is_git_repo(&ctx.repo_root));
        let maybe_guard = if wants_worktree && is_write_capable(&resolved, &full_registry) {
            let repo_root = ctx.repo_root.clone();
            let cid = child_id.clone();
            match tokio::task::spawn_blocking(move || {
                worktree::WorktreeGuard::create(&repo_root, &cid)
            })
            .await
            {
                Ok(Ok(g)) => Some(g),
                Ok(Err(e)) => {
                    tracing::warn!(
                        "worktree create failed for {child_id}: {e} — running without isolation"
                    );
                    None
                }
                Err(e) => {
                    tracing::warn!(
                            "worktree create task failed for {child_id}: {e} — running without isolation"
                        );
                    None
                }
            }
        } else {
            None
        };

        // Build the child context, injecting the worktree root when applicable.
        let child_ctx = AgentCtx {
            worktree_root: maybe_guard.as_ref().map(|g| g.path().to_path_buf()),
            ..ctx.clone()
        };

        let tx = tx.clone();
        let sem = Arc::clone(&sem);
        let merge_lock = Arc::clone(&merge_lock);
        let repo_root = ctx.repo_root.clone();
        child_tasks.0.push(tokio::spawn(async move {
            // Acquire the provider sub-cap FIRST (block here without holding a global permit, so a
            // saturated provider can't head-of-line-block children bound for OTHER providers), then
            // the global concurrency permit. Both are held for the child's lifetime.
            let _provider_permit = match provider_sem {
                Some(ps) => ps.acquire_owned().await.ok(),
                None => None,
            };
            let _permit = sem.acquire_owned().await;
            let _host_slot = crate::host_limits::acquire_helper_slot().await;
            // Forward streamed text/reasoning as live progress for this child's UI row.
            let mut on_delta = |ev: StreamEvent| {
                let snippet = match ev {
                    StreamEvent::Text(t) | StreamEvent::Reasoning(t) => t,
                    _ => return,
                };
                let _ = tx.send(ChildMsg::Progress { index: i, snippet });
            };
            let outcome = run_subagent(
                &child_ctx,
                &child_id,
                &resolved,
                decision,
                budget,
                &mut on_delta,
            )
            .await;
            let (mut text, mut ok, mut structured) = match outcome {
                Ok(out) => (out.final_text, out.ok, out.structured),
                Err(e) => (format!("error: subagent failed: {e}"), false, None),
            };

            // Merge the child's worktree changes back into the main tree (serialized).
            if let Some(guard) = maybe_guard {
                let branch = guard.branch().to_string();
                // Snapshot the child's UNCOMMITTED edits onto its branch first —
                // merge_worktree_back diffs HEAD..branch, and a child never commits on its own,
                // so without this every edit was silently dropped ("clean" merge, work lost).
                let commit_res = {
                    let path = guard.path().to_path_buf();
                    tokio::task::spawn_blocking(move || worktree::commit_worktree(&path)).await
                };
                match commit_res {
                    Ok(Ok(_)) => {}
                    Ok(Err(e)) => {
                        tracing::warn!("worktree snapshot failed for {child_id}: {e}");
                        structured::note_or_text(
                            &mut text,
                            &mut structured,
                            format!("\n[worktree snapshot failed: {e}]"),
                        );
                        ok = false;
                    }
                    Err(e) => {
                        tracing::warn!("worktree snapshot task failed for {child_id}: {e}");
                        structured::note_or_text(
                            &mut text,
                            &mut structured,
                            format!("\n[worktree snapshot task failed: {e}]"),
                        );
                        ok = false;
                    }
                }
                // Hold the merge lock for the duration of the git apply so concurrent finishers
                // don't race on the index. Drop guard AFTER merge so the branch still exists.
                let _lock = merge_lock.lock().await;
                let merge_res = {
                    let repo_root = repo_root.clone();
                    let branch = branch.clone();
                    tokio::task::spawn_blocking(move || {
                        worktree::merge_worktree_back(&repo_root, &branch)
                    })
                    .await
                };
                match merge_res {
                    Ok(Ok(report)) if report.conflicted_files.is_empty() => {
                        // Clean merge — nothing to add.
                    }
                    Ok(Ok(report)) => {
                        let conflicts = report.conflicted_files.join(", ");
                        structured::note_or_text(
                            &mut text,
                            &mut structured,
                            format!("\n[worktree merge conflicts in: {conflicts}]"),
                        );
                        ok = false;
                    }
                    Ok(Err(e)) => {
                        tracing::warn!("merge_worktree_back failed for {child_id}: {e}");
                        structured::note_or_text(
                            &mut text,
                            &mut structured,
                            format!("\n[worktree merge failed: {e}]"),
                        );
                        ok = false;
                    }
                    Err(e) => {
                        tracing::warn!("merge_worktree_back task failed for {child_id}: {e}");
                        structured::note_or_text(
                            &mut text,
                            &mut structured,
                            format!("\n[worktree merge task failed: {e}]"),
                        );
                        ok = false;
                    }
                }
                // Drop the guard now (removes worktree dir + branch).
                drop(guard);
            }

            let cost = child_ctx.store.session_cost(&child_id).unwrap_or(0.0);
            let _ = tx.send(ChildMsg::Done(ChildDone {
                index: i,
                id: child_id,
                agent: resolved.name,
                text,
                ok,
                cost,
                structured,
            }));
        }));
    }
    drop(tx); // close the channel once every task holds its own clone

    let mut slots: Vec<Option<(String, String, Option<structured::StructuredResult>)>> =
        vec![None; n];
    while let Some(msg) = rx.recv().await {
        match msg {
            ChildMsg::Progress { index, snippet } => {
                on_event(Lifecycle::Progress {
                    id: &ids[index],
                    snippet: &snippet,
                });
            }
            ChildMsg::Done(done) => {
                all_ok &= done.ok;
                let one_line = match &done.structured {
                    Some(result) => structured::preview(result),
                    None => summary(&done.text),
                };
                on_event(Lifecycle::Done {
                    id: &done.id,
                    agent: &done.agent,
                    ok: done.ok,
                    summary: &one_line,
                    cost_usd: done.cost,
                });
                slots[done.index] = Some((done.agent, done.text, done.structured));
            }
        }
    }

    let mut combined = String::new();
    let mut rows = Vec::with_capacity(n);
    for (i, slot) in slots.into_iter().enumerate() {
        let (agent, text, result) = match slot {
            Some(triple) => triple,
            // The child's task ended without ever sending Done — it panicked (or was aborted).
            // That MUST count as a failure: `all_ok` feeds the parent's tool-call status, and a
            // batch with a crashed child is not "ok" just because its siblings succeeded.
            None => {
                all_ok = false;
                (
                    "?".into(),
                    "error: no result (the agent crashed before reporting)".into(),
                    None,
                )
            }
        };
        combined.push_str(&format!("[agent {}: {}]\n{}\n\n", i + 1, agent, text));
        rows.push(structured::row_for(i + 1, agent, &text, result));
    }
    if wants.iter().any(|w| *w) {
        return Ok((structured::envelope(&rows), all_ok));
    }
    Ok((combined.trim_end().to_string(), all_ok))
}

/// First non-empty line of a result, truncated — a one-line summary for lifecycle events.
pub(crate) fn summary(text: &str) -> String {
    let line = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    line.chars().take(120).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_requests_reads_agent_and_task_with_inline_default() {
        let args = json!({"agents": [
            {"agent": "reviewer", "task": "review the diff"},
            {"task": "find all call sites of foo"}
        ]});
        let reqs = parse_requests(&args, 8).unwrap();
        assert_eq!(reqs.len(), 2);
        assert_eq!(reqs[0].agent, "reviewer");
        assert_eq!(reqs[1].agent, "general-purpose"); // omitted agent → general-purpose
        assert_eq!(reqs[1].task, "find all call sites of foo");
    }

    #[test]
    fn parse_requests_caps_at_max_agents() {
        let agents: Vec<_> = (0..20).map(|i| json!({"task": format!("t{i}")})).collect();
        let err = parse_requests(&json!({ "agents": agents }), 3).unwrap_err();
        assert!(err.contains("at most 3"), "{err}");
        let few: Vec<_> = (0..3).map(|i| json!({"task": format!("t{i}")})).collect();
        assert_eq!(
            parse_requests(&json!({ "agents": few }), 3).unwrap().len(),
            3
        );
    }

    #[test]
    fn parse_requests_rejects_empty_or_taskless() {
        assert!(parse_requests(&json!({"agents": []}), 8).is_err());
        assert!(parse_requests(&json!({"agents": [{"agent": "x"}]}), 8).is_err());
        assert!(parse_requests(&json!({"nope": 1}), 8).is_err());
    }

    #[test]
    fn resolve_applies_named_agent_type_and_defaults_unknown() {
        let mut agents = HashMap::new();
        agents.insert(
            "reviewer".to_string(),
            AgentDef {
                name: "reviewer".into(),
                description: "d".into(),
                tools: vec!["read_file".into()],
                tier: Some(TaskTier::Complex),
                pinned_model: Some("p::m".into()),
                system_prompt: "You review.".into(),
                ..Default::default()
            },
        );
        let named = resolve(
            &AgentRequest {
                agent: "reviewer".into(),
                task: "look at the diff".into(),
                ..Default::default()
            },
            &agents,
        );
        assert_eq!(named.system_prompt, "You review.");
        assert_eq!(named.tools, vec!["read_file"]);
        assert_eq!(named.tier, Some(TaskTier::Complex));
        assert_eq!(named.pinned_model.as_deref(), Some("p::m"));

        let unknown = resolve(
            &AgentRequest {
                agent: "general".into(),
                task: "t".into(),
                ..Default::default()
            },
            &agents,
        );
        assert!(unknown.tools.is_empty()); // → every tool
        assert_eq!(unknown.tier, None); // → mesh-routed
        assert_eq!(unknown.system_prompt, SUBAGENT_SYSTEM);
    }

    #[test]
    fn spawn_spec_lists_agent_types_and_accepts_subagent_type() {
        let mut agents = HashMap::new();
        agents.insert(
            "reviewer".to_string(),
            AgentDef {
                name: "reviewer".into(),
                description: "Reviews diffs".into(),
                tools: vec!["read_file".into()],
                tier: None,
                pinned_model: None,
                system_prompt: String::new(),
                ..Default::default()
            },
        );
        let with = spawn_agents_spec_with(4, &agents);
        assert!(with
            .description
            .contains("- reviewer: Reviews diffs [read_file]"));
        assert!(!spawn_agents_spec_with(4, &HashMap::new())
            .description
            .contains("Available `agent` types"));
        let reqs = parse_requests(
            &serde_json::json!({ "agents": [{ "subagent_type": "reviewer", "task": "t" }] }),
            4,
        )
        .unwrap();
        assert_eq!(reqs[0].agent, "reviewer");
    }

    #[tokio::test]
    async fn pinned_model_bypasses_the_router_entirely() {
        // A /duel candidate must run the EXACT model the arena picked, not whatever the mesh
        // would independently route the task to (FixedRouter here would pick "mesh::pick").
        let store = Arc::new(Store::open_in_memory().unwrap());
        let provider: Arc<dyn Provider> = Arc::new(FlakyProvider {
            bad: std::collections::HashSet::new(),
        });
        let router: Arc<dyn Router> = Arc::new(FixedRouter {
            model: "mesh::pick".into(),
            fallbacks: vec![],
        });
        let ctx = ctx_with(provider, router, store);
        let agent = ResolvedAgent {
            name: "duel:provx".into(),
            task: "implement the thing".into(),
            system_prompt: "s".into(),
            tools: Vec::new(),
            tier: None,
            pinned_model: Some("provx::model".into()),
            ..Default::default()
        };
        let decision = route_child(&ctx, &agent, BudgetState::default()).await;
        assert_eq!(decision.model, "provx::model");
        assert!(decision.rationale.contains("duel"));
        assert!(decision.fallbacks.is_empty());
    }

    // --- Subagent pin inheritance (docs/features/mesh-routing.md): a pin active on the session
    // (`--model` or in-session `/model`) must propagate to every subagent strictly, unless
    // `mesh.subagents.inherit_pin` is disabled. ---

    #[tokio::test]
    async fn inherit_pin_true_with_router_pin_forces_child_onto_the_pin() {
        // Router-pin inheritance (default `inherit_pin = true`): the child gets the exact pinned
        // model, `pinned == true`, and NO cross-model fallback chain — the same strict-pin the
        // parent gets.
        let router: Arc<dyn Router> = Arc::new(PinnedAwareRouter {
            pinned: Some("pin::model".into()),
            mesh_model: "mesh::model".into(),
            mesh_fallbacks: vec!["mesh::fallback".into()],
        });
        let ctx = ctx_with_pin(Arc::clone(&router), Some(vec!["pin::model".into()]), true);
        let decision = route_child(&ctx, &plain_agent(), BudgetState::default()).await;
        assert_eq!(decision.model, "pin::model");
        assert!(decision.pinned, "inherited pin must be flagged pinned");
        assert!(
            decision.fallbacks.is_empty(),
            "an inherited pin must have NO cross-model fallback chain"
        );
    }

    #[tokio::test]
    async fn inherit_pin_true_with_session_pin_forces_child_onto_the_pin() {
        // The regression that proves `/model` (the session pin, distinct from the router's `--model`)
        // now propagates to children. The router has NO pin; only the effective session pin is set.
        let router: Arc<dyn Router> = Arc::new(PinnedAwareRouter {
            pinned: None,
            mesh_model: "mesh::model".into(),
            mesh_fallbacks: vec!["mesh::fallback".into()],
        });
        let ctx = ctx_with_pin(Arc::clone(&router), Some(vec!["session::pin".into()]), true);
        let decision = route_child(&ctx, &plain_agent(), BudgetState::default()).await;
        assert_eq!(decision.model, "session::pin");
        assert!(decision.pinned);
        assert!(decision.fallbacks.is_empty());
    }

    #[tokio::test]
    async fn inherit_pin_true_user_pin_beats_agent_type_tier_default() {
        // An explicit user pin must override an agent-type tier default (which would otherwise
        // resolve via `config.model_for(tier)`). The agent has a Complex tier; the pin wins.
        let router: Arc<dyn Router> = Arc::new(PinnedAwareRouter {
            pinned: Some("user::pin".into()),
            mesh_model: "mesh::model".into(),
            mesh_fallbacks: vec!["mesh::fallback".into()],
        });
        let ctx = ctx_with_pin(Arc::clone(&router), Some(vec!["user::pin".into()]), true);
        let mut agent = plain_agent();
        agent.tier = Some(TaskTier::Complex);
        let decision = route_child(&ctx, &agent, BudgetState::default()).await;
        assert_eq!(decision.model, "user::pin");
        assert!(decision.pinned);
    }

    #[tokio::test]
    async fn inherit_pin_true_duel_pin_wins_over_session_pin() {
        // `/duel` pins each candidate to a specific model to COMPARE them; letting an inherited
        // session pin override it would collapse every duel candidate onto one model and make the
        // feature meaningless. So `agent.pinned_model` still wins over the session pin.
        let router: Arc<dyn Router> = Arc::new(PinnedAwareRouter {
            pinned: None,
            mesh_model: "mesh::model".into(),
            mesh_fallbacks: Vec::new(),
        });
        let ctx = ctx_with_pin(Arc::clone(&router), Some(vec!["session::pin".into()]), true);
        let mut agent = plain_agent();
        agent.pinned_model = Some("duel::candidate".into());
        let decision = route_child(&ctx, &agent, BudgetState::default()).await;
        assert_eq!(decision.model, "duel::candidate");
        assert!(decision.pinned);
    }

    #[tokio::test]
    async fn inherit_pin_false_defeats_the_router_level_pin() {
        // With `inherit_pin = false` the child must route through the mesh INDEPENDENTLY with its
        // normal failover chain intact — even though the router itself still holds the pin. This is
        // the case that proves the pin-free routing path (`route_unpinned`) works: skipping the new
        // code path alone would NOT be enough, because `ctx.router.route(...)` would still return
        // the pinned model.
        let router: Arc<dyn Router> = Arc::new(PinnedAwareRouter {
            pinned: Some("pin::model".into()),
            mesh_model: "mesh::model".into(),
            mesh_fallbacks: vec!["mesh::fallback".into()],
        });
        let ctx = ctx_with_pin(Arc::clone(&router), Some(vec!["pin::model".into()]), false);
        let decision = route_child(&ctx, &plain_agent(), BudgetState::default()).await;
        assert_ne!(decision.model, "pin::model");
        assert_eq!(decision.model, "mesh::model");
        assert!(!decision.pinned);
        assert!(
            !decision.fallbacks.is_empty(),
            "a mesh-routed child must keep its failover chain"
        );
    }

    #[tokio::test]
    async fn nothing_pinned_routes_via_the_mesh_as_usual() {
        // No pin anywhere: the child routes through the mesh exactly as before (its model + full
        // failover chain).
        let router: Arc<dyn Router> = Arc::new(PinnedAwareRouter {
            pinned: None,
            mesh_model: "mesh::model".into(),
            mesh_fallbacks: vec!["mesh::fallback".into()],
        });
        let ctx = ctx_with_pin(Arc::clone(&router), None, true);
        let decision = route_child(&ctx, &plain_agent(), BudgetState::default()).await;
        assert_eq!(decision.model, "mesh::model");
        assert!(!decision.pinned);
        assert!(!decision.fallbacks.is_empty());
    }

    #[tokio::test]
    async fn inherit_pin_true_with_a_two_model_set_restricts_the_child_to_the_set() {
        // With `inherit_pin = true` (default) a child is restricted to the SAME pin SET as the
        // parent: the model is one of the members and the fallback chain stays inside the set.
        let router: Arc<dyn Router> = Arc::new(PinnedAwareRouter {
            pinned: None,
            mesh_model: "mesh::model".into(),
            mesh_fallbacks: vec!["mesh::fallback".into()],
        });
        let set = vec!["a::model".to_string(), "b::model".to_string()];
        let ctx = ctx_with_pin(Arc::clone(&router), Some(set.clone()), true);
        let decision = route_child(&ctx, &plain_agent(), BudgetState::default()).await;
        assert!(
            set.contains(&decision.model),
            "child must be in the set: {}",
            decision.model
        );
        assert!(decision.pinned);
        assert!(
            decision.fallbacks.iter().all(|m| set.contains(m)),
            "child chain must stay inside the set: {:?}",
            decision.fallbacks
        );
        assert_eq!(
            decision.fallbacks.len(),
            1,
            "only the other member is a fallback"
        );
    }

    #[tokio::test]
    async fn inherit_pin_false_routes_the_child_outside_the_set() {
        // With `inherit_pin = false` a child routes via the full mesh — it may pick a model
        // OUTSIDE the parent's set (here the mesh's unrelated model).
        let router: Arc<dyn Router> = Arc::new(PinnedAwareRouter {
            pinned: None,
            mesh_model: "mesh::model".into(),
            mesh_fallbacks: vec!["mesh::fallback".into()],
        });
        let set = vec!["a::model".to_string(), "b::model".to_string()];
        let ctx = ctx_with_pin(Arc::clone(&router), Some(set.clone()), false);
        let decision = route_child(&ctx, &plain_agent(), BudgetState::default()).await;
        assert_eq!(
            decision.model, "mesh::model",
            "child routes via the full mesh"
        );
        assert!(!set.contains(&decision.model));
        assert!(!decision.pinned);
        assert!(!decision.fallbacks.is_empty());
    }

    #[test]
    fn an_agent_without_tools_gets_every_tool_but_spawn_agents() {
        let registry = ToolRegistry::with_core_tools();
        let agent = ResolvedAgent::default();
        let tools = effective_tools(&agent, &registry);
        for name in ["read_file", "write_file", "edit_file", "shell"] {
            assert!(tools.iter().any(|t| t == name), "{name} missing: {tools:?}");
        }
        assert!(!tools.iter().any(|t| t == SPAWN_AGENTS_TOOL));
    }

    #[test]
    fn spawn_tool_description_keeps_single_coding_tasks_direct() {
        let description = spawn_agents_spec(8).description;

        assert!(description.contains("at least two child results"));
        assert!(description.contains("Do not use for routine repository exploration"));
        assert!(description.contains("direct tools share context and are faster"));
    }

    // --- Subagent failover (docs/features/mesh-routing.md): a child whose model rate-limits must
    // fail over down its chain, not die — the bug the user hit ("run a testing task" → the
    // spawned child 429'd). ---

    struct FlakyProvider {
        bad: std::collections::HashSet<String>,
    }
    #[async_trait::async_trait]
    impl Provider for FlakyProvider {
        async fn complete(
            &self,
            model: &str,
            _messages: &[forge_types::Message],
            _tools: &[ToolSpec],
            _on_event: &mut forge_provider::EventSink<'_>,
        ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
            if self.bad.contains(model) {
                return Err(forge_provider::ProviderError::RateLimited {
                    message: "429".into(),
                    retry_after: Some(std::time::Duration::from_secs(30)),
                });
            }
            Ok(forge_provider::ModelResponse {
                reasoning: String::new(),
                reasoning_items: Vec::new(),
                content: "child done".into(),
                tool_calls: vec![],
                usage: forge_types::Usage::default(),
                quotas: Vec::new(),
            })
        }
    }

    struct FixedRouter {
        model: String,
        fallbacks: Vec<String>,
    }
    #[async_trait::async_trait]
    impl Router for FixedRouter {
        async fn route(
            &self,
            _p: &str,
            _has_images: bool,
            _b: BudgetState,
            _h: &forge_types::ModelHealth,
            _q: &forge_types::SubscriptionQuota,
            _effort: Option<forge_types::EffortLevel>,
            _project: &forge_types::ProjectContext,
        ) -> RoutingDecision {
            RoutingDecision {
                effort: None,
                tier: TaskTier::Standard,
                model: self.model.clone(),
                rationale: "test".into(),
                fallbacks: self.fallbacks.clone(),
                pinned: false,
                unroutable: false,
            }
        }
    }

    fn ctx_with(
        provider: Arc<dyn Provider>,
        router: Arc<dyn Router>,
        store: Arc<Store>,
    ) -> AgentCtx {
        let config = Config::default();
        let pricing = Pricing::from_config(&config);
        AgentCtx {
            provider,
            router,
            store,
            config,
            pricing,
            mode: PermissionMode::default(),
            rules: Vec::new(),
            depth: 1,
            max_depth: 2,
            agents: Arc::new(HashMap::new()),
            worktree_root: None,
            repo_root: std::path::PathBuf::from("."),
            effective_pin: None,
            inherit_pin: true,
            asker: None,
        }
    }

    /// A router that reports a pin through [`Router::pin`] and returns the pinned model from
    /// `route` (simulating the router holding the pin), but a distinct mesh model with a full
    /// fallback chain from `route_unpinned` (simulating what a child should get when pin
    /// inheritance is disabled). Lets a test prove `inherit_pin = false` genuinely defeats the
    /// router-level pin rather than merely skipping a new code path.
    struct PinnedAwareRouter {
        pinned: Option<String>,
        mesh_model: String,
        mesh_fallbacks: Vec<String>,
    }
    #[async_trait::async_trait]
    impl Router for PinnedAwareRouter {
        async fn route(
            &self,
            _p: &str,
            _has_images: bool,
            _b: BudgetState,
            _h: &forge_types::ModelHealth,
            _q: &forge_types::SubscriptionQuota,
            _effort: Option<forge_types::EffortLevel>,
            _project: &forge_types::ProjectContext,
        ) -> RoutingDecision {
            match &self.pinned {
                Some(pin) => RoutingDecision {
                    effort: None,
                    tier: TaskTier::Standard,
                    model: pin.clone(),
                    rationale: "pinned via --model".into(),
                    fallbacks: Vec::new(),
                    pinned: true,
                    unroutable: false,
                },
                None => RoutingDecision {
                    effort: None,
                    tier: TaskTier::Standard,
                    model: self.mesh_model.clone(),
                    rationale: "mesh".into(),
                    fallbacks: self.mesh_fallbacks.clone(),
                    pinned: false,
                    unroutable: false,
                },
            }
        }
        async fn route_unpinned(
            &self,
            _p: &str,
            _has_images: bool,
            _b: BudgetState,
            _h: &forge_types::ModelHealth,
            _q: &forge_types::SubscriptionQuota,
            _effort: Option<forge_types::EffortLevel>,
            _project: &forge_types::ProjectContext,
        ) -> RoutingDecision {
            RoutingDecision {
                effort: None,
                tier: TaskTier::Standard,
                model: self.mesh_model.clone(),
                rationale: "mesh".into(),
                fallbacks: self.mesh_fallbacks.clone(),
                pinned: false,
                unroutable: false,
            }
        }
        async fn route_with_pin_set(
            &self,
            pin: &[String],
            _p: &str,
            _has_images: bool,
            _b: BudgetState,
            _h: &forge_types::ModelHealth,
            _q: &forge_types::SubscriptionQuota,
            _effort: Option<forge_types::EffortLevel>,
            _project: &forge_types::ProjectContext,
        ) -> RoutingDecision {
            // A single pinned model: strict, no cross-model fallback. A set: ranked within the
            // set, chain confined to the set.
            let model = pin.first().cloned().unwrap_or_default();
            RoutingDecision {
                effort: None,
                tier: TaskTier::Standard,
                model,
                rationale: "inherited pin".into(),
                fallbacks: if pin.len() > 1 {
                    pin[1..].to_vec()
                } else {
                    Vec::new()
                },
                pinned: true,
                unroutable: false,
            }
        }
        fn pin(&self) -> Option<&[String]> {
            self.pinned.as_ref().map(std::slice::from_ref)
        }
    }

    /// Build a ctx with a configured effective pin and `inherit_pin`, for pin-inheritance tests.
    fn ctx_with_pin(
        router: Arc<dyn Router>,
        effective_pin: Option<Vec<String>>,
        inherit_pin: bool,
    ) -> AgentCtx {
        let mut ctx = ctx_with(
            Arc::new(FlakyProvider {
                bad: std::collections::HashSet::new(),
            }),
            router,
            Arc::new(Store::open_in_memory().unwrap()),
        );
        ctx.effective_pin = effective_pin;
        ctx.config.mesh.subagents.inherit_pin = inherit_pin;
        ctx.inherit_pin = inherit_pin;
        ctx
    }

    fn plain_agent() -> ResolvedAgent {
        ResolvedAgent {
            name: "general".into(),
            task: "some task".into(),
            system_prompt: "s".into(),
            tools: Vec::new(),
            tier: None,
            pinned_model: None,
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn subagent_fails_over_when_its_model_rate_limits() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let child = store
            .create_child_session(".", "default", "parent")
            .unwrap();
        let provider = Arc::new(FlakyProvider {
            bad: ["bad::model".to_string()].into_iter().collect(),
        });
        let router = Arc::new(FixedRouter {
            model: "bad::model".into(),
            fallbacks: vec!["good::model".into()],
        });
        let ctx = ctx_with(provider, router, Arc::clone(&store));
        let agent = ResolvedAgent {
            name: "general".into(),
            task: "run a testing task".into(),
            system_prompt: "you are a subagent".into(),
            tools: Vec::new(),
            tier: None,
            pinned_model: None,
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
        .expect("subagent must recover via failover, not error");
        assert!(out.ok, "subagent succeeded on the fallback");
        assert_eq!(out.final_text, "child done");
        assert!(
            store.current_benched().unwrap().is_benched("bad::model"),
            "the rate-limited model was benched"
        );
    }

    struct StalledThenGoodProvider;

    #[async_trait::async_trait]
    impl Provider for StalledThenGoodProvider {
        async fn complete(
            &self,
            model: &str,
            _messages: &[forge_types::Message],
            _tools: &[ToolSpec],
            _on_event: &mut forge_provider::EventSink<'_>,
        ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
            if model == "stalled::model" {
                std::future::pending().await
            } else {
                Ok(forge_provider::ModelResponse {
                    reasoning: String::new(),
                    reasoning_items: Vec::new(),
                    content: "child recovered after stall".into(),
                    tool_calls: vec![],
                    usage: forge_types::Usage::default(),
                    quotas: Vec::new(),
                })
            }
        }
    }

    #[tokio::test]
    async fn subagent_idle_stream_watchdog_fails_over_instead_of_hanging() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let child = store
            .create_child_session(".", "default", "parent")
            .unwrap();
        let router = Arc::new(FixedRouter {
            model: "stalled::model".into(),
            fallbacks: vec!["good::model".into()],
        });
        let mut ctx = ctx_with(
            Arc::new(StalledThenGoodProvider),
            router,
            Arc::clone(&store),
        );
        ctx.config.mesh.stream_idle_timeout_secs = 1;
        let agent = ResolvedAgent {
            name: "general".into(),
            task: "inspect the package".into(),
            system_prompt: "you are a subagent".into(),
            tools: Vec::new(),
            tier: None,
            pinned_model: None,
            ..Default::default()
        };
        let progress = Arc::new(std::sync::Mutex::new(Vec::new()));
        let captured = Arc::clone(&progress);
        let mut sink = move |event: StreamEvent| {
            if let StreamEvent::Reasoning(text) = event {
                captured.lock().unwrap().push(text);
            }
        };
        let decision = route_child(&ctx, &agent, BudgetState::default()).await;
        let out = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            run_subagent(
                &ctx,
                &child,
                &agent,
                decision,
                BudgetState::default(),
                &mut sink,
            ),
        )
        .await
        .expect("subagent watchdog must bound a silent provider")
        .expect("subagent must recover through its fallback");
        assert!(out.ok);
        assert_eq!(out.final_text, "child recovered after stall");
        assert!(
            progress
                .lock()
                .unwrap()
                .iter()
                .any(|line| line.contains("waiting for `stalled::model`")),
            "the live activity row should identify the provider being awaited"
        );
        assert!(
            progress
                .lock()
                .unwrap()
                .iter()
                .any(|line| line.contains("failing over to `good::model`")),
            "the live activity row should explain the automatic failover"
        );
        assert!(
            store
                .current_benched()
                .unwrap()
                .is_benched("stalled::model"),
            "the stalled model must be benched"
        );
    }

    /// Emits: (0) a `read_file` on a nonexistent path → "error:" tool result, (1) a `read_file`
    /// on a real path → success, (2) a final answer — the recover-after-one-bad-call shape.
    struct RecoveringToolProvider {
        step: std::sync::atomic::AtomicUsize,
    }
    #[async_trait::async_trait]
    impl Provider for RecoveringToolProvider {
        async fn complete(
            &self,
            _model: &str,
            _messages: &[forge_types::Message],
            _tools: &[ToolSpec],
            _on_event: &mut forge_provider::EventSink<'_>,
        ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
            let step = self.step.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let tool_calls = match step {
                0 => vec![forge_types::ToolCall {
                    id: "c0".into(),
                    name: "read_file".into(),
                    args: serde_json::json!({"path": "/definitely/not/here-xyz"}),
                }],
                1 => vec![forge_types::ToolCall {
                    id: "c1".into(),
                    name: "read_file".into(),
                    args: serde_json::json!({"path": "Cargo.toml"}),
                }],
                _ => vec![],
            };
            let content = if tool_calls.is_empty() {
                "recovered fine".to_string()
            } else {
                String::new()
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

    /// Regression (seen live in a workflow run): the old `ok` latch flipped false on ANY failing
    /// tool call and never reset, so a child whose first read errored on a bad path but which
    /// recovered with a successful read + good answer was still shown as ✗ — while siblings that
    /// failed without ever touching a tool showed ✓. Last outcome wins now.
    #[tokio::test]
    async fn a_child_that_recovers_from_one_failing_tool_call_is_not_marked_failed() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let child = store
            .create_child_session(".", "default", "parent")
            .unwrap();
        let provider = Arc::new(RecoveringToolProvider {
            step: std::sync::atomic::AtomicUsize::new(0),
        });
        let router = Arc::new(FixedRouter {
            model: "good::model".into(),
            fallbacks: vec![],
        });
        let ctx = ctx_with(provider, router, Arc::clone(&store));
        let agent = ResolvedAgent {
            name: "general".into(),
            task: "read the manifest".into(),
            system_prompt: "you are a subagent".into(),
            tools: Vec::new(),
            tier: None,
            pinned_model: None,
            ..Default::default()
        };
        let progress = Arc::new(std::sync::Mutex::new(Vec::new()));
        let captured = Arc::clone(&progress);
        let mut sink = move |event: StreamEvent| {
            if let StreamEvent::Reasoning(text) = event {
                captured.lock().unwrap().push(text);
            }
        };
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
        assert_eq!(out.final_text, "recovered fine");
        assert!(
            out.ok,
            "one failed tool call followed by a successful one is a recovery, not a failed child"
        );
        let progress = progress.lock().unwrap();
        assert!(
            progress
                .iter()
                .any(|line| line.contains("running tool `read_file`")),
            "the live activity row should identify child tools while they run"
        );
        assert!(
            progress
                .iter()
                .any(|line| line.contains("tool `read_file` failed; continuing")),
            "recoverable child tool failures should be visible"
        );
        assert!(
            progress
                .iter()
                .any(|line| line.contains("tool `read_file` finished")),
            "successful child tool recovery should be visible"
        );
    }

    #[test]
    fn a_childs_step_limit_is_explicit_and_never_below_one() {
        assert_eq!(progress::step_limit(None, 0), None);
        assert_eq!(progress::step_limit(None, 100), Some(100));
        assert_eq!(progress::step_limit(Some(7), 0), Some(7));
        assert_eq!(progress::step_limit(Some(7), 3), Some(3));
        assert_eq!(progress::step_limit(Some(7), 100), Some(7));
        assert_eq!(progress::step_limit(Some(0), 0), Some(1));
    }

    /// Reads the same file with the same arguments `loop_steps` times, then answers.
    struct LoopingChild {
        calls: std::sync::atomic::AtomicUsize,
        loop_steps: usize,
    }
    #[async_trait::async_trait]
    impl Provider for LoopingChild {
        async fn complete(
            &self,
            _model: &str,
            _messages: &[forge_types::Message],
            _tools: &[ToolSpec],
            _on_event: &mut forge_provider::EventSink<'_>,
        ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
            let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let looping = n < self.loop_steps;
            Ok(forge_provider::ModelResponse {
                reasoning: String::new(),
                reasoning_items: Vec::new(),
                content: if looping {
                    "Still reading the manifest.".into()
                } else {
                    "the manifest names the crate".into()
                },
                tool_calls: if looping {
                    vec![forge_types::ToolCall {
                        id: format!("c{n}"),
                        name: "read_file".into(),
                        args: serde_json::json!({"path": "Cargo.toml"}),
                    }]
                } else {
                    vec![]
                },
                usage: forge_types::Usage::default(),
                quotas: Vec::new(),
            })
        }
    }

    async fn run_looping_child(
        loop_steps: usize,
        guard: forge_config::ProgressGuardMode,
    ) -> (SubagentOutcome, usize) {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let child = store
            .create_child_session(".", "default", "parent")
            .unwrap();
        let provider = Arc::new(LoopingChild {
            calls: std::sync::atomic::AtomicUsize::new(0),
            loop_steps,
        });
        let router = Arc::new(FixedRouter {
            model: "good::model".into(),
            fallbacks: vec![],
        });
        let mut ctx = ctx_with(provider.clone(), router, Arc::clone(&store));
        ctx.config.mesh.progress_guard = guard;
        let agent = ResolvedAgent {
            name: "general".into(),
            task: "read the manifest".into(),
            system_prompt: "you are a subagent".into(),
            tools: Vec::new(),
            tier: None,
            pinned_model: None,
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
        (
            out,
            provider.calls.load(std::sync::atomic::Ordering::SeqCst),
        )
    }

    #[tokio::test]
    async fn a_child_that_stops_getting_anywhere_is_stopped_and_keeps_what_it_said() {
        let (out, calls) = run_looping_child(50, forge_config::ProgressGuardMode::Enforce).await;
        assert!(!out.ok, "a stalled child is not a clean answer");
        assert!(
            out.final_text.contains("Still reading the manifest."),
            "{}",
            out.final_text
        );
        assert!(out.final_text.contains("Stopped:"), "{}", out.final_text);
        assert!(calls <= 6, "{calls} model calls");
    }

    #[tokio::test]
    async fn a_child_is_never_stopped_with_the_progress_guard_off() {
        let (out, calls) = run_looping_child(8, forge_config::ProgressGuardMode::Off).await;
        assert!(out.ok);
        assert_eq!(out.final_text, "the manifest names the crate");
        assert_eq!(calls, 9);
    }

    #[tokio::test]
    async fn a_child_with_no_step_limit_at_all_runs_to_its_answer() {
        // `mesh.max_steps` defaults to none; a child must not be capped at one step by it.
        let (out, _) = run_looping_child(2, forge_config::ProgressGuardMode::Enforce).await;
        assert!(out.ok);
        assert_eq!(out.final_text, "the manifest names the crate");
    }

    #[test]
    fn is_write_capable_detects_write_tools() {
        let registry = ToolRegistry::with_core_tools();

        // An agent with write_file in its toolset is write-capable.
        let write_agent = ResolvedAgent {
            name: "writer".into(),
            task: "t".into(),
            system_prompt: "s".into(),
            tools: vec!["write_file".into()],
            tier: None,
            pinned_model: None,
            ..Default::default()
        };
        assert!(is_write_capable(&write_agent, &registry));

        // An agent with only read_file is not.
        let read_agent = ResolvedAgent {
            name: "reader".into(),
            task: "t".into(),
            system_prompt: "s".into(),
            tools: vec!["read_file".into()],
            tier: None,
            pinned_model: None,
            ..Default::default()
        };
        assert!(!is_write_capable(&read_agent, &registry));

        // An agent with shell is write-capable.
        let shell_agent = ResolvedAgent {
            name: "sheller".into(),
            task: "t".into(),
            system_prompt: "s".into(),
            tools: vec!["shell".into()],
            tier: None,
            pinned_model: None,
            ..Default::default()
        };
        assert!(is_write_capable(&shell_agent, &registry));

        // An agent with empty tools inherits every tool, so it is write-capable.
        let default_agent = ResolvedAgent {
            name: "general".into(),
            task: "t".into(),
            system_prompt: "s".into(),
            tools: Vec::new(),
            tier: None,
            pinned_model: None,
            ..Default::default()
        };
        assert!(is_write_capable(&default_agent, &registry));
    }

    #[test]
    fn rewrite_args_for_worktree_rewrites_relative_path() {
        let root = std::path::Path::new("/work/tree");
        let args = json!({"path": "src/main.rs", "content": "fn main() {}"});
        let rewritten = rewrite_args_for_root(&args, root);
        // Compute the expected path the same way the impl joins it, so the assertion holds on both
        // unix ("/work/tree/src/main.rs") and Windows (backslash separators).
        let expected = root.join("src/main.rs").to_string_lossy().into_owned();
        assert_eq!(rewritten["path"].as_str().unwrap(), expected);
        // content field is untouched
        assert_eq!(rewritten["content"].as_str().unwrap(), "fn main() {}");
    }

    #[test]
    fn rewrite_args_for_worktree_leaves_absolute_path_alone() {
        let root = std::path::Path::new("/work/tree");
        let args = json!({"path": "/absolute/path/file.rs"});
        let rewritten = rewrite_args_for_root(&args, root);
        assert_eq!(
            rewritten["path"].as_str().unwrap(),
            "/absolute/path/file.rs"
        );
    }

    #[test]
    fn rewrite_args_for_worktree_injects_cwd_when_absent() {
        let root = std::path::Path::new("/work/tree");
        let args = json!({"cmd": "cargo test"});
        let rewritten = rewrite_args_for_root(&args, root);
        assert_eq!(rewritten["cwd"].as_str().unwrap(), "/work/tree");
    }

    #[test]
    fn rewrite_args_for_worktree_rewrites_relative_cwd() {
        let root = std::path::Path::new("/work/tree");
        let args = json!({"cmd": "ls", "cwd": "subdir"});
        let rewritten = rewrite_args_for_root(&args, root);
        let expected = root.join("subdir").to_string_lossy().into_owned();
        assert_eq!(rewritten["cwd"].as_str().unwrap(), expected);
    }

    #[test]
    fn rewrite_args_for_worktree_leaves_absolute_cwd_alone() {
        let root = std::path::Path::new("/work/tree");
        let args = json!({"cmd": "ls", "cwd": "/other/dir"});
        let rewritten = rewrite_args_for_root(&args, root);
        assert_eq!(rewritten["cwd"].as_str().unwrap(), "/other/dir");
    }

    #[test]
    fn child_tool_args_are_rooted_even_without_worktree_isolation() {
        let root = std::path::Path::new("/session/repo");
        let args = json!({"path": "alpha.txt", "paths": ["beta.txt"]});
        let rewritten = rewrite_args_for_root(&args, root);
        assert_eq!(
            rewritten["path"].as_str().unwrap(),
            root.join("alpha.txt").to_string_lossy()
        );
        assert_eq!(
            rewritten["paths"][0].as_str().unwrap(),
            root.join("beta.txt").to_string_lossy()
        );
        assert_eq!(rewritten["cwd"].as_str().unwrap(), root.to_string_lossy());
    }

    #[test]
    fn labelled_batched_read_error_is_a_failed_tool_result() {
        assert!(tool_result_failed(
            "===== alpha.txt =====\n[error: io error: No such file or directory]"
        ));
        assert!(!tool_result_failed(
            "===== alpha.txt =====\nalpha has three words"
        ));
    }

    // --- Provider-aware fan-out cap (competitor-gap #5): a burst of children all routed to ONE
    // provider must not run more than `max_per_provider` at once, so a single subscription/key
    // quota isn't hammered in parallel even when the global concurrency cap is higher. ---

    /// Records the peak number of `complete` calls in flight at once.
    struct ConcurrencyProbe {
        active: Arc<std::sync::atomic::AtomicUsize>,
        peak: Arc<std::sync::atomic::AtomicUsize>,
    }
    #[async_trait::async_trait]
    impl Provider for ConcurrencyProbe {
        async fn complete(
            &self,
            _model: &str,
            _messages: &[forge_types::Message],
            _tools: &[ToolSpec],
            _on_event: &mut forge_provider::EventSink<'_>,
        ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
            use std::sync::atomic::Ordering::SeqCst;
            let now = self.active.fetch_add(1, SeqCst) + 1;
            self.peak.fetch_max(now, SeqCst);
            // Hold the slot long enough that, without the cap, all children would overlap.
            tokio::time::sleep(std::time::Duration::from_millis(40)).await;
            self.active.fetch_sub(1, SeqCst);
            Ok(forge_provider::ModelResponse {
                reasoning: String::new(),
                reasoning_items: Vec::new(),
                content: "child done".into(),
                tool_calls: vec![],
                usage: forge_types::Usage::default(),
                quotas: Vec::new(),
            })
        }
    }

    #[tokio::test]
    async fn per_provider_cap_throttles_same_provider_fanout() {
        use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
        let store = Arc::new(Store::open_in_memory().unwrap());
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let provider = Arc::new(ConcurrencyProbe {
            active: Arc::clone(&active),
            peak: Arc::clone(&peak),
        });
        // Every child routes to the SAME provider (`openai::`), so they share one provider permit.
        let router = Arc::new(FixedRouter {
            model: "openai::gpt-test".into(),
            fallbacks: vec![],
        });
        let mut ctx = ctx_with(provider, router, store);
        ctx.config.mesh.subagents.max_per_provider = 1; // serialize same-provider children
        let requests: Vec<_> = (0..4)
            .map(|i| AgentRequest {
                agent: "general".into(),
                task: format!("t{i}"),
                ..Default::default()
            })
            .collect();
        let mut sink = |_: Lifecycle| {};
        // Global cap 8 (>= the 4 children): only the PER-PROVIDER cap can hold them back.
        let (_out, ok) = orchestrate(
            &ctx,
            "parent",
            requests,
            BudgetState::default(),
            8,
            &mut sink,
        )
        .await
        .unwrap();
        assert!(ok, "all children should succeed");
        assert_eq!(
            peak.load(SeqCst),
            1,
            "max_per_provider=1 must serialize children sharing a provider (peak in-flight)"
        );
    }

    /// Answers instantly with a whitespace-only string — `is_empty()` alone let this through as
    /// a SUCCESSFUL child ("\n" is not empty), handing the parent a blank agent block marked ok.
    struct WhitespaceProvider;
    #[async_trait::async_trait]
    impl Provider for WhitespaceProvider {
        async fn complete(
            &self,
            _model: &str,
            _messages: &[forge_types::Message],
            _tools: &[ToolSpec],
            _on_event: &mut forge_provider::EventSink<'_>,
        ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
            Ok(forge_provider::ModelResponse {
                reasoning: String::new(),
                reasoning_items: Vec::new(),
                content: "\n  \n".into(),
                tool_calls: vec![],
                usage: forge_types::Usage::default(),
                quotas: Vec::new(),
            })
        }
    }

    #[tokio::test]
    async fn a_whitespace_only_answer_is_a_failure_not_a_blank_success() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let child = store
            .create_child_session(".", "default", "parent")
            .unwrap();
        let router = Arc::new(FixedRouter {
            model: "openai::gpt-test".into(),
            fallbacks: vec![],
        });
        let ctx = ctx_with(Arc::new(WhitespaceProvider), router, store);
        let resolved = resolve(
            &AgentRequest {
                agent: "general".into(),
                task: "t".into(),
                ..Default::default()
            },
            &ctx.agents,
        );
        let decision = route_child(&ctx, &resolved, BudgetState::default()).await;
        let mut on_delta = |_: StreamEvent| {};
        let out = run_subagent(
            &ctx,
            &child,
            &resolved,
            decision,
            BudgetState::default(),
            &mut on_delta,
        )
        .await
        .unwrap();
        assert!(!out.ok, "blank output must not be a successful child");
        assert!(
            out.final_text.contains("no usable final answer"),
            "explains itself: {}",
            out.final_text
        );
    }

    /// A provider that panics mid-turn — the child's spawned task dies without ever sending
    /// `Done`, which must surface as a failed batch, not a silent "ok".
    struct PanickingProvider;
    #[async_trait::async_trait]
    impl Provider for PanickingProvider {
        async fn complete(
            &self,
            _model: &str,
            _messages: &[forge_types::Message],
            _tools: &[ToolSpec],
            _on_event: &mut forge_provider::EventSink<'_>,
        ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
            panic!("simulated provider crash");
        }
    }

    #[tokio::test]
    async fn a_panicked_child_fails_the_batch_not_silent_ok() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let router = Arc::new(FixedRouter {
            model: "openai::gpt-test".into(),
            fallbacks: vec![],
        });
        let ctx = ctx_with(Arc::new(PanickingProvider), router, store);
        let requests = vec![AgentRequest {
            agent: "general".into(),
            task: "t".into(),
            ..Default::default()
        }];
        let mut sink = |_: Lifecycle| {};
        let (out, ok) = orchestrate(
            &ctx,
            "parent",
            requests,
            BudgetState::default(),
            8,
            &mut sink,
        )
        .await
        .unwrap();
        assert!(!ok, "a crashed child must not report a successful batch");
        assert!(
            out.contains("error: no result"),
            "placeholder present: {out}"
        );
    }

    /// A provider whose completions count — used to prove that dropping `orchestrate`'s future
    /// (what a turn abort does) actually cancels in-flight children instead of leaking them to
    /// run (and spend, and write) in the background.
    struct SlowCountingProvider {
        completed: Arc<std::sync::atomic::AtomicUsize>,
    }
    #[async_trait::async_trait]
    impl Provider for SlowCountingProvider {
        async fn complete(
            &self,
            _model: &str,
            _messages: &[forge_types::Message],
            _tools: &[ToolSpec],
            _on_event: &mut forge_provider::EventSink<'_>,
        ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            self.completed
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(forge_provider::ModelResponse {
                reasoning: String::new(),
                reasoning_items: Vec::new(),
                content: "child done".into(),
                tool_calls: vec![],
                usage: forge_types::Usage::default(),
                quotas: Vec::new(),
            })
        }
    }

    #[tokio::test]
    async fn dropping_orchestrate_cancels_inflight_children() {
        use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
        let store = Arc::new(Store::open_in_memory().unwrap());
        let completed = Arc::new(AtomicUsize::new(0));
        let provider = Arc::new(SlowCountingProvider {
            completed: Arc::clone(&completed),
        });
        let router = Arc::new(FixedRouter {
            model: "openai::gpt-test".into(),
            fallbacks: vec![],
        });
        let ctx = ctx_with(provider, router, store);
        let requests: Vec<_> = (0..3)
            .map(|i| AgentRequest {
                agent: "general".into(),
                task: format!("t{i}"),
                ..Default::default()
            })
            .collect();

        // Poll orchestrate long enough for the children to spawn and start their slow model
        // calls, then DROP it — exactly what `JoinHandle::abort()` on the turn task does.
        {
            let mut sink = |_: Lifecycle| {};
            let fut = orchestrate(
                &ctx,
                "parent",
                requests,
                BudgetState::default(),
                8,
                &mut sink,
            );
            let _ = tokio::time::timeout(std::time::Duration::from_millis(100), fut).await;
            // timeout dropped the future here
        }

        // Give any leaked child ample time to finish its 300ms sleep and increment the counter.
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        assert_eq!(
            completed.load(SeqCst),
            0,
            "children must be aborted when the orchestrating turn is dropped, not keep \
             running (and spending / writing) in the background"
        );
    }

    #[tokio::test]
    async fn per_provider_cap_disabled_lets_same_provider_fanout_run_parallel() {
        use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
        let store = Arc::new(Store::open_in_memory().unwrap());
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let provider = Arc::new(ConcurrencyProbe {
            active: Arc::clone(&active),
            peak: Arc::clone(&peak),
        });
        let router = Arc::new(FixedRouter {
            model: "openai::gpt-test".into(),
            fallbacks: vec![],
        });
        let mut ctx = ctx_with(provider, router, store);
        ctx.config.mesh.subagents.max_per_provider = 0; // disabled → only the global cap applies
        let requests: Vec<_> = (0..4)
            .map(|i| AgentRequest {
                agent: "general".into(),
                task: format!("t{i}"),
                ..Default::default()
            })
            .collect();
        let mut sink = |_: Lifecycle| {};
        let (_out, ok) = orchestrate(
            &ctx,
            "parent",
            requests,
            BudgetState::default(),
            8,
            &mut sink,
        )
        .await
        .unwrap();
        assert!(ok);
        assert!(
            peak.load(SeqCst) > 1,
            "with the per-provider cap off and a high global cap, same-provider children run in parallel"
        );
    }

    // --- Retained async subagents (RFC retained-async-subagents): detached admission returns
    // handles immediately (without waiting for the child), the child still runs to completion
    // and lands its result in the durable registry, and depth-1 stays structural. ---

    #[tokio::test]
    async fn spawn_detached_admits_immediately_and_returns_handles() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let parent = store.create_session(".", "default").unwrap();
        let provider = Arc::new(FlakyProvider {
            bad: std::collections::HashSet::new(),
        });
        let router = Arc::new(FixedRouter {
            model: "openai::gpt-test".into(),
            fallbacks: vec![],
        });
        let mut ctx = ctx_with(provider, router, Arc::clone(&store));
        // `ctx_with` defaults to depth 1 (most tests in this module exercise a CHILD's own
        // context); detached admission is a top-level-only entry point.
        ctx.depth = 0;
        let registry = DetachedRegistry::new();
        let requests = vec![AgentRequest {
            agent: "general".into(),
            task: "investigate something".into(),
            ..Default::default()
        }];

        let handles = spawn_detached(
            &ctx,
            &registry,
            &parent,
            requests,
            BudgetState::default(),
            None,
        )
        .await
        .expect("detached admission must not wait for the child to finish");

        assert_eq!(handles.len(), 1);
        assert_eq!(handles[0].model, "openai::gpt-test");
        assert_eq!(handles[0].name, "general");
        assert!(
            !handles[0].child_id.is_empty(),
            "admission handle carries a real child id"
        );

        // The registry row exists immediately — admission does not wait on the background task.
        let rows = store.list_detached_children(&parent).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].child_id, handles[0].child_id);

        // The background task eventually finishes and writes its result to the durable row.
        for _ in 0..100 {
            if store.list_detached_children(&parent).unwrap()[0].status
                != forge_store::DetachedChildStatus::Running
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let rows = store.list_detached_children(&parent).unwrap();
        assert_eq!(rows[0].status, forge_store::DetachedChildStatus::Done);
        assert_eq!(rows[0].result_ref.as_deref(), Some("child done"));
    }

    #[tokio::test]
    async fn spawn_detached_reports_its_lifecycle_to_the_sink_after_the_call_returns() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let parent = store.create_session(".", "default").unwrap();
        let provider = Arc::new(FlakyProvider {
            bad: std::collections::HashSet::new(),
        });
        let router = Arc::new(FixedRouter {
            model: "openai::gpt-test".into(),
            fallbacks: vec![],
        });
        let mut ctx = ctx_with(provider, router, Arc::clone(&store));
        ctx.depth = 0;
        let registry = DetachedRegistry::new();
        let seen: Arc<std::sync::Mutex<Vec<String>>> = Arc::default();
        let sink_seen = Arc::clone(&seen);
        let sink_store = Arc::clone(&store);
        let sink_parent = parent.clone();
        let sink: DetachedSink = Arc::new(move |ev: Lifecycle| {
            let line = match ev {
                Lifecycle::Start {
                    id, agent, model, ..
                } => format!("start {agent} {model} {}", id.len()),
                Lifecycle::Progress { .. } => return,
                Lifecycle::Done {
                    agent, ok, summary, ..
                } => {
                    // The store already holds the result when the panel hears about it, so
                    // `list_subagents` never lags the UI.
                    let row = &sink_store.list_detached_children(&sink_parent).unwrap()[0];
                    format!("done {agent} {ok} {summary} {}", row.status.as_str())
                }
            };
            sink_seen.lock().unwrap().push(line);
        });
        let requests = vec![AgentRequest {
            agent: "general".into(),
            task: "investigate something".into(),
            ..Default::default()
        }];

        let handles = spawn_detached(
            &ctx,
            &registry,
            &parent,
            requests,
            BudgetState::default(),
            Some(sink),
        )
        .await
        .unwrap();
        let id_len = handles[0].child_id.len();
        assert_eq!(
            seen.lock().unwrap().first().cloned(),
            Some(format!("start general openai::gpt-test {id_len}")),
            "Start is reported at admission"
        );

        for _ in 0..200 {
            if seen.lock().unwrap().len() == 2 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert_eq!(
            seen.lock().unwrap().get(1).cloned(),
            Some("done general true child done done".to_string())
        );
        assert!(
            !registry.is_running(&handles[0].child_id),
            "a finished child leaves the in-process registry"
        );
    }

    #[tokio::test]
    async fn spawn_detached_rejects_detached_grandchildren() {
        // Structural depth guard: a context already one level deep (as a spawned child's own
        // AgentCtx would be) must not be able to admit ITS OWN detached children.
        let store = Arc::new(Store::open_in_memory().unwrap());
        let parent = store.create_session(".", "default").unwrap();
        let provider = Arc::new(FlakyProvider {
            bad: std::collections::HashSet::new(),
        });
        let router = Arc::new(FixedRouter {
            model: "m".into(),
            fallbacks: vec![],
        });
        let mut ctx = ctx_with(provider, router, Arc::clone(&store));
        ctx.depth = 1;
        let registry = DetachedRegistry::new();
        let requests = vec![AgentRequest {
            agent: "general".into(),
            task: "t".into(),
            ..Default::default()
        }];

        let err = spawn_detached(
            &ctx,
            &registry,
            &parent,
            requests,
            BudgetState::default(),
            None,
        )
        .await
        .expect_err("depth > 0 must refuse detached admission");
        assert!(
            err.to_string().contains("depth guard") || err.to_string().contains("grandchildren"),
            "error should explain the depth guard: {err}"
        );
        assert!(
            store.list_detached_children(&parent).unwrap().is_empty(),
            "a refused admission must not create any child session or registry row"
        );
    }

    #[test]
    fn resolve_detached_address_matches_name_then_id_prefix() {
        let a = forge_store::DetachedChild {
            child_id: "aaaaaaaa1111".into(),
            parent_session: "p".into(),
            name: "researcher".into(),
            model: "m".into(),
            status: forge_store::DetachedChildStatus::Running,
            result_ref: None,
            created_at: 0,
            finished_at: None,
        };
        let b = forge_store::DetachedChild {
            child_id: "bbbbbbbb2222".into(),
            parent_session: "p".into(),
            name: "writer".into(),
            model: "m".into(),
            status: forge_store::DetachedChildStatus::Running,
            result_ref: None,
            created_at: 1,
            finished_at: None,
        };
        let children = vec![a, b];
        assert_eq!(
            resolve_detached_address(&children, "writer")
                .unwrap()
                .child_id,
            "bbbbbbbb2222"
        );
        assert_eq!(
            resolve_detached_address(&children, "aaaaaaaa")
                .unwrap()
                .child_id,
            "aaaaaaaa1111"
        );
        assert!(resolve_detached_address(&children, "nope").is_none());
    }

    #[test]
    fn format_admission_lists_every_handle() {
        let handles = vec![
            AdmissionHandle {
                child_id: "abcdefgh1234".into(),
                name: "researcher".into(),
                model: "openai::gpt-test".into(),
            },
            AdmissionHandle {
                child_id: "ijklmnop5678".into(),
                name: "writer".into(),
                model: "anthropic::claude-test".into(),
            },
        ];
        let text = format_admission(&handles);
        assert!(text.contains("2 detached agent"));
        assert!(text.contains("researcher"));
        assert!(text.contains("writer"));
        assert!(text.contains("abcdefgh"));
        assert!(text.contains("list_subagents"));
        assert!(text.contains("cancel_subagent"));
    }
}
