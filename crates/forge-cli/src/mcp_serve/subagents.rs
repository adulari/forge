//! Subagents on the CLI-bridge path.
//!
//! On the direct path a subagent turn reports through a presenter; here the caller is claude or
//! codex running its own loop, so `spawn_agents` and `send_to_agent` have to reproduce the same
//! semantics with no presenter at all — resolving a child among the parent session's persisted
//! children, rebuilding its transcript, and reporting progress out of band through the sink. This
//! module owns that bridge-shaped half; the shared orchestration itself stays in forge-core.

use rmcp::model::{CallToolResult, ContentBlock};
use serde_json::Value;
use std::fmt::Display;

use forge_core::subagent::{self, AgentCtx};
use forge_mesh::BudgetState;

use super::ForgeMcp;

fn load_budget(
    store: &forge_store::Store,
    config: &forge_config::Config,
) -> Result<BudgetState, String> {
    Ok(BudgetState {
        spent_today_usd: store
            .spend_today_usd()
            .map_err(|error| format!("reading daily spend from the audit store: {error}"))?,
        daily_cap_usd: config.mesh.daily_budget_usd,
        spent_week_usd: store
            .spend_this_week_usd()
            .map_err(|error| format!("reading weekly spend from the audit store: {error}"))?,
        weekly_cap_usd: config.mesh.weekly_budget_usd,
        spent_month_usd: store
            .spend_this_month_usd()
            .map_err(|error| format!("reading monthly spend from the audit store: {error}"))?,
        monthly_cap_usd: config.mesh.monthly_cap_usd,
        warn_fraction: config.mesh.warn_threshold,
        min_context_tokens: None,
    })
}

fn budget_error_message(error: impl Display) -> String {
    format!("error: subagent budget unavailable; refusing to route: {error}")
}

/// Everything a `spawn_agents` call needs, built once if subagents are enabled here. `ctx`
/// already carries the loaded agent types, the nesting depth, and `max_depth`.
pub(super) struct SubagentSupport {
    pub(super) ctx: AgentCtx,
    pub(super) parent_id: String,
    pub(super) max_agents: usize,
    pub(super) max_concurrency: usize,
    /// In-memory abort handles for this process's currently-running detached children (retained
    /// async subagents) — bridge-side counterpart of the direct path's `Session::detached_registry`.
    /// Built once alongside `ctx` so it's shared across every `spawn_agents`/`cancel_subagent`
    /// call this long-lived `forge mcp-serve` process handles.
    pub(super) detached_registry: subagent::DetachedRegistry,
    /// The session detached children belong to: the bridged Forge session that spawned this
    /// process when it says so (`FORGE_CHECKPOINT_SESSION`), so that session delivers their
    /// results at its next turn boundary and its `list_subagents` agrees with this one; this
    /// process's own `parent_id` otherwise.
    pub(super) detached_parent: String,
}

/// The bridged Forge session that spawned this process, if it names one this store knows.
fn bridging_session(store: &forge_store::Store) -> Option<String> {
    let id = std::env::var(forge_core::snapshot::ENV_SESSION).ok()?;
    let id = id.trim();
    store
        .session_times(id)
        .ok()
        .flatten()
        .map(|_| id.to_string())
}

/// The out-of-band sink the parent Forge process tails (`FORGE_SUBAGENT_SINK`), as a lifecycle
/// callback: one JSON line per event, so bridge-spawned children show natively in the parent's
/// agents panel (RFC subagent-orchestration Phase 3c). Shared with detached children, which keep
/// reporting after the spawning call returned. `None` when the bridge gave no sink.
fn lifecycle_sink() -> Option<subagent::DetachedSink> {
    let path = std::env::var(forge_provider::SUBAGENT_SINK_ENV).ok()?;
    let file = match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        Ok(f) => std::sync::Mutex::new(f),
        Err(error) => {
            tracing::warn!(
                path,
                %error,
                "mcp-serve: cannot open the subagent sink; the parent session will not see this \
                 spawn's lifecycle"
            );
            return None;
        }
    };
    Some(std::sync::Arc::new(move |ev: subagent::Lifecycle| {
        let line = match ev {
            subagent::Lifecycle::Start {
                id,
                agent,
                task,
                model,
            } => serde_json::json!({"k":"start","id":id,"agent":agent,"task":task,"model":model}),
            subagent::Lifecycle::Progress { id, snippet } => {
                serde_json::json!({"k":"progress","id":id,"snippet":snippet})
            }
            subagent::Lifecycle::Done {
                id,
                agent,
                ok,
                summary,
                cost_usd,
            } => {
                serde_json::json!({"k":"done","id":id,"agent":agent,"ok":ok,"summary":summary,"cost":cost_usd})
            }
        };
        use std::io::Write;
        let mut f = file
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Err(error) = writeln!(f, "{line}").and_then(|()| f.flush()) {
            tracing::warn!(
                path,
                %error,
                "mcp-serve: failed to append a subagent lifecycle event to the sink"
            );
        }
    }))
}

/// Subagent support for one `forge mcp-serve` process, built on the first call that needs it.
///
/// Building it means a provider + router, a duel-boost read and a parent session row in the
/// store: about 300 ms (debug build) that every bridge spawn used to pay before answering claude's
/// MCP handshake, and so before the turn's first token, whether or not the model ever delegated.
/// Only what the tool list shows (the agent types and the fan-out cap) is known up front.
pub(super) struct LazySubagents {
    pub(super) max_agents: usize,
    pub(super) agents: std::sync::Arc<std::collections::HashMap<String, forge_config::AgentDef>>,
    config: forge_config::Config,
    depth: usize,
    store: std::sync::Arc<forge_store::Store>,
    built: tokio::sync::OnceCell<Result<SubagentSupport, String>>,
    /// Created eagerly (it is just an empty map) so [`Self::interrupt_running`] can reach it
    /// after the server has consumed `ForgeMcp`.
    detached_registry: subagent::DetachedRegistry,
}

impl LazySubagents {
    /// `None` when subagents are off here or the nesting budget is spent.
    pub(super) fn new(
        config: &forge_config::Config,
        depth: usize,
        store: std::sync::Arc<forge_store::Store>,
    ) -> Option<Self> {
        if !config.mesh.subagents.enabled || depth > config.mesh.subagents.max_depth {
            return None;
        }
        let agents = std::sync::Arc::new(forge_config::load_agents_layered(
            &std::env::current_dir().unwrap_or_default(),
            &config.mesh.subagents.agents_dir,
        ));
        Some(Self {
            max_agents: config.mesh.subagents.max_agents,
            agents,
            config: config.clone(),
            depth,
            store,
            built: tokio::sync::OnceCell::new(),
            detached_registry: subagent::DetachedRegistry::new(),
        })
    }

    /// What [`Self::interrupt_running`] needs, detached from `self` so it outlives the server.
    pub(super) fn exit_handle(
        &self,
    ) -> (
        subagent::DetachedRegistry,
        std::sync::Arc<forge_store::Store>,
    ) {
        (
            self.detached_registry.clone(),
            std::sync::Arc::clone(&self.store),
        )
    }

    async fn get(&self) -> Result<&SubagentSupport, &str> {
        self.built
            .get_or_init(|| async { self.build().map_err(|error| format!("{error:#}")) })
            .await
            .as_ref()
            .map_err(String::as_str)
    }

    fn build(&self) -> anyhow::Result<SubagentSupport> {
        let config = &self.config;
        // Same global store the parent uses — NOT a relative `.forge/forge.db`, which is a
        // DIFFERENT file: the divergence created spurious empty sessions and broke the bridge task
        // round-trip.
        let store = std::sync::Arc::clone(&self.store);
        let repo_key = std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        let repo_boosts = store.duel_boosts(&repo_key).unwrap_or_default();
        let (provider, router) = crate::build_provider_and_router(
            config,
            false,
            None,
            None,
            Default::default(),
            repo_boosts,
        );
        let parent_id = store.create_session(".", &format!("{:?}", config.permission_mode))?;
        let ctx = AgentCtx {
            provider,
            router,
            store,
            config: config.clone(),
            pricing: forge_mesh::pricing::Pricing::from_config(config),
            mode: config.permission_mode,
            rules: config.permission_rules(),
            depth: self.depth,
            max_depth: config.mesh.subagents.max_depth,
            agents: std::sync::Arc::clone(&self.agents),
            worktree_root: None,
            repo_root: std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")),
            effective_pin: None,
            inherit_pin: config.mesh.subagents.inherit_pin,
            asker: super::permission_relay::child_asker(),
        };
        Ok(SubagentSupport {
            ctx,
            max_agents: config.mesh.subagents.max_agents,
            max_concurrency: config.mesh.subagents.max_concurrency,
            detached_registry: self.detached_registry.clone(),
            detached_parent: bridging_session(&self.store).unwrap_or_else(|| parent_id.clone()),
            parent_id,
        })
    }
}

/// Settle the detached children still running when this process exits: the bridge CLI that owned
/// it is gone, and so are their tasks. Marking them failed keeps `list_subagents` truthful and lets
/// the parent session deliver the outcome (and settle the panel row) at its next turn boundary.
pub(super) fn interrupt_running(
    (registry, store): (
        subagent::DetachedRegistry,
        std::sync::Arc<forge_store::Store>,
    ),
) {
    for child_id in registry.running_ids() {
        registry.abort(&child_id);
        let _ = store.finish_detached_child(
            &child_id,
            false,
            "error: interrupted — the bridge process running this detached agent exited before \
             it finished",
        );
    }
}

impl ForgeMcp {
    /// The subagent support to advertise: none while the parent thread has delegation off.
    pub(super) fn visible_subagents(&self) -> Option<&LazySubagents> {
        self.subagents
            .as_ref()
            .filter(|_| !forge_core::delegation_flag::is_off_from_env())
    }

    /// The built subagent support, or the tool error for `tool` when subagents are unavailable
    /// here or could not be set up.
    async fn subagent_support(&self, tool: &str) -> Result<&SubagentSupport, CallToolResult> {
        // `/delegation off` in the parent session withholds every delegation tool here too.
        if forge_core::delegation_flag::is_off_from_env() {
            return Err(CallToolResult::error(vec![ContentBlock::text(
                "delegation is switched off for this thread by the user; do the work yourself \
                 with your own tools",
            )]));
        }
        let Some(lazy) = &self.subagents else {
            return Err(CallToolResult::error(vec![ContentBlock::text(format!(
                "{tool} is not available here"
            ))]));
        };
        lazy.get().await.map_err(|error| {
            CallToolResult::error(vec![ContentBlock::text(format!(
                "error: {tool} unavailable: subagent setup failed: {error}"
            ))])
        })
    }

    /// Bridge-side `send_to_agent`: resolve the child among the parent session's persisted
    /// children, rebuild its transcript, and continue it — the same semantics as the direct
    /// path's `Session::send_to_agent`, reported through the out-of-band sink instead of a
    /// presenter.
    pub(super) async fn handle_send_to_agent(&self, args: &Value) -> CallToolResult {
        let s = match self.subagent_support("send_to_agent").await {
            Ok(s) => s,
            Err(refusal) => return refusal,
        };
        let address = args
            .get("agent")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let message = args
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if address.is_empty() || message.is_empty() {
            return CallToolResult::error(vec![ContentBlock::text(
                "error: send_to_agent needs both `agent` (name or id prefix) and `message`",
            )]);
        }
        let children = s
            .ctx
            .store
            .named_child_sessions(&s.parent_id)
            .unwrap_or_default();
        let Some((child_id, agent_name)) = subagent::resolve_child_address(&children, &address)
        else {
            let known: Vec<String> = children
                .iter()
                .map(|(id, t)| format!("{} ({})", t.as_deref().unwrap_or("unnamed"), &id[..8]))
                .collect();
            return CallToolResult::error(vec![ContentBlock::text(format!(
                "error: no child agent matches '{address}'. Children this session: [{}] — \
                 spawn one first with spawn_agents",
                known.join(", ")
            ))]);
        };
        let request = subagent::AgentRequest {
            agent: agent_name.clone(),
            task: message.clone(),
        };
        let resolved = subagent::resolve(&request, &s.ctx.agents);
        let budget = match load_budget(&s.ctx.store, &self.config) {
            Ok(budget) => budget,
            Err(error) => {
                return CallToolResult::error(vec![ContentBlock::text(budget_error_message(
                    error,
                ))]);
            }
        };
        let decision = subagent::route_child(&s.ctx, &resolved, budget).await;
        let mut on_delta = |_: forge_provider::StreamEvent| {};
        match subagent::resume_subagent(
            &s.ctx,
            &child_id,
            &resolved,
            &message,
            decision,
            budget,
            &mut on_delta,
        )
        .await
        {
            Ok(outcome) => {
                let label = if agent_name.is_empty() {
                    child_id[..8].to_string()
                } else {
                    agent_name
                };
                let text = format!("[{label}] {}", outcome.final_text);
                if outcome.ok {
                    CallToolResult::success(vec![ContentBlock::text(text)])
                } else {
                    CallToolResult::error(vec![ContentBlock::text(text)])
                }
            }
            Err(e) => CallToolResult::error(vec![ContentBlock::text(format!(
                "error: send_to_agent failed: {e}"
            ))]),
        }
    }

    pub(super) async fn handle_spawn_agents(&self, args: &Value) -> CallToolResult {
        let s = match self.subagent_support("spawn_agents").await {
            Ok(s) => s,
            Err(refusal) => return refusal,
        };
        let requests = match subagent::parse_requests(args, s.max_agents) {
            Ok(r) => r,
            Err(msg) => {
                return CallToolResult::error(vec![ContentBlock::text(format!("error: {msg}"))])
            }
        };

        let budget = match load_budget(&s.ctx.store, &self.config) {
            Ok(budget) => budget,
            Err(error) => {
                return CallToolResult::error(vec![ContentBlock::text(budget_error_message(
                    error,
                ))]);
            }
        };

        // Detached admission (RFC retained-async-subagents), bridge parity with the direct path's
        // `Session::spawn_agents`: admit immediately, return admission handles without waiting.
        if subagent::parse_detached_flag(args) {
            let detached_ctx = AgentCtx {
                asker: None,
                ..s.ctx.clone()
            };
            return match subagent::spawn_detached(
                &detached_ctx,
                &s.detached_registry,
                &s.detached_parent,
                requests,
                budget,
                lifecycle_sink(),
            )
            .await
            {
                Ok(handles) => CallToolResult::success(vec![ContentBlock::text(
                    subagent::format_admission(&handles),
                )]),
                Err(e) => CallToolResult::error(vec![ContentBlock::text(format!("error: {e}"))]),
            };
        }

        // Report subagent lifecycle to the out-of-band sink (if the bridge gave us one) so the
        // parent Forge TUI shows these children natively (RFC subagent-orchestration Phase 3c).
        let sink = lifecycle_sink();
        let mut on_event = |ev: subagent::Lifecycle| {
            if let Some(sink) = &sink {
                sink(ev);
            }
        };

        match subagent::orchestrate(
            &s.ctx,
            &s.parent_id,
            requests,
            budget,
            s.max_concurrency,
            &mut on_event,
        )
        .await
        {
            Ok((combined, _ok)) => CallToolResult::success(vec![ContentBlock::text(combined)]),
            Err(e) => {
                CallToolResult::error(vec![ContentBlock::text(format!("subagents failed: {e}"))])
            }
        }
    }

    /// Bridge-side `list_subagents`: report every detached child spawned this parent session.
    pub(super) async fn handle_list_subagents(&self) -> CallToolResult {
        let s = match self.subagent_support("list_subagents").await {
            Ok(s) => s,
            Err(refusal) => return refusal,
        };
        let children = s
            .ctx
            .store
            .list_detached_children(&s.detached_parent)
            .unwrap_or_default();
        CallToolResult::success(vec![ContentBlock::text(subagent::format_subagent_list(
            &children,
        ))])
    }

    /// Bridge-side `cancel_subagent`: stop a still-running detached child, mirroring the direct
    /// path's `Session::cancel_subagent` semantics (best-effort in-process abort + the durable
    /// store transition; already-finished is reported, not an error).
    pub(super) async fn handle_cancel_subagent(&self, args: &Value) -> CallToolResult {
        let s = match self.subagent_support("cancel_subagent").await {
            Ok(s) => s,
            Err(refusal) => return refusal,
        };
        let address = args
            .get("agent")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if address.is_empty() {
            return CallToolResult::error(vec![ContentBlock::text(
                "error: cancel_subagent needs `agent` (name or id prefix)",
            )]);
        }
        let children = s
            .ctx
            .store
            .list_detached_children(&s.detached_parent)
            .unwrap_or_default();
        let Some(child) = subagent::resolve_detached_address(&children, &address) else {
            let known: Vec<String> = children
                .iter()
                .map(|c| format!("{} ({})", c.name, &c.child_id[..c.child_id.len().min(8)]))
                .collect();
            return CallToolResult::error(vec![ContentBlock::text(format!(
                "error: no detached agent matches '{address}'. Detached agents this session: [{}]",
                known.join(", ")
            ))]);
        };
        if child.status != forge_store::DetachedChildStatus::Running {
            return CallToolResult::success(vec![ContentBlock::text(format!(
                "'{}' ({}) is already {} — nothing to cancel",
                child.name,
                &child.child_id[..child.child_id.len().min(8)],
                child.status.as_str()
            ))]);
        }
        let child_id = child.child_id.clone();
        let name = child.name.clone();
        s.detached_registry.abort(&child_id);
        if let Err(e) = s.ctx.store.cancel_detached_child(&child_id) {
            return CallToolResult::error(vec![ContentBlock::text(format!(
                "error: cancel_subagent failed: {e}"
            ))]);
        }
        CallToolResult::success(vec![ContentBlock::text(format!(
            "cancelled '{name}' ({})",
            &child_id[..child_id.len().min(8)]
        ))])
    }
}

#[cfg(test)]
mod tests {
    use super::budget_error_message;

    #[test]
    fn budget_failures_refuse_routing_with_actionable_text() {
        assert_eq!(
            budget_error_message("daily spend query failed"),
            "error: subagent budget unavailable; refusing to route: daily spend query failed"
        );
    }
}
