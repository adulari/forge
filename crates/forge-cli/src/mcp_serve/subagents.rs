//! Subagents on the CLI-bridge path.
//!
//! On the direct path a subagent turn reports through a presenter; here the caller is claude or
//! codex running its own loop, so `spawn_agents` and `send_to_agent` have to reproduce the same
//! semantics with no presenter at all — resolving a child among the parent session's persisted
//! children, rebuilding its transcript, and reporting progress out of band through the sink. This
//! module owns that bridge-shaped half; the shared orchestration itself stays in forge-core.

use rmcp::model::{CallToolResult, ContentBlock};
use serde_json::Value;

use forge_core::subagent::{self, AgentCtx};
use forge_mesh::BudgetState;

use super::ForgeMcp;

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
}

impl ForgeMcp {
    /// Bridge-side `send_to_agent`: resolve the child among the parent session's persisted
    /// children, rebuild its transcript, and continue it — the same semantics as the direct
    /// path's `Session::send_to_agent`, reported through the out-of-band sink instead of a
    /// presenter.
    pub(super) async fn handle_send_to_agent(&self, args: &Value) -> CallToolResult {
        let Some(s) = &self.subagents else {
            return CallToolResult::error(vec![ContentBlock::text(
                "send_to_agent is not available here",
            )]);
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
        let budget = BudgetState {
            spent_today_usd: s.ctx.store.spend_today_usd().unwrap_or(0.0),
            daily_cap_usd: self.config.mesh.daily_budget_usd,
            spent_week_usd: s.ctx.store.spend_this_week_usd().unwrap_or(0.0),
            weekly_cap_usd: self.config.mesh.weekly_budget_usd,
            spent_month_usd: s.ctx.store.spend_this_month_usd().unwrap_or(0.0),
            monthly_cap_usd: self.config.mesh.monthly_cap_usd,
            warn_fraction: self.config.mesh.warn_threshold,
            min_context_tokens: None,
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
        let Some(s) = &self.subagents else {
            return CallToolResult::error(vec![ContentBlock::text(
                "spawn_agents is not available here",
            )]);
        };
        let requests = match subagent::parse_requests(args, s.max_agents) {
            Ok(r) => r,
            Err(msg) => {
                return CallToolResult::error(vec![ContentBlock::text(format!("error: {msg}"))])
            }
        };

        let budget = BudgetState {
            spent_today_usd: s.ctx.store.spend_today_usd().unwrap_or(0.0),
            daily_cap_usd: self.config.mesh.daily_budget_usd,
            spent_week_usd: s.ctx.store.spend_this_week_usd().unwrap_or(0.0),
            weekly_cap_usd: self.config.mesh.weekly_budget_usd,
            spent_month_usd: s.ctx.store.spend_this_month_usd().unwrap_or(0.0),
            monthly_cap_usd: self.config.mesh.monthly_cap_usd,
            warn_fraction: self.config.mesh.warn_threshold,
            min_context_tokens: None,
        };

        // Detached admission (RFC retained-async-subagents), bridge parity with the direct path's
        // `Session::spawn_agents`: admit immediately, return admission handles without waiting.
        if subagent::parse_detached_flag(args) {
            return match subagent::spawn_detached(
                &s.ctx,
                &s.detached_registry,
                &s.parent_id,
                requests,
                budget,
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
        let mut sink = std::env::var(forge_provider::SUBAGENT_SINK_ENV)
            .ok()
            .and_then(|p| {
                std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(p)
                    .ok()
            });
        let mut write = move |v: serde_json::Value| {
            if let Some(f) = sink.as_mut() {
                use std::io::Write;
                let _ = writeln!(f, "{v}");
                let _ = f.flush();
            }
        };
        let mut on_event = |ev: subagent::Lifecycle| match ev {
            subagent::Lifecycle::Start {
                id,
                agent,
                task,
                model,
            } => {
                write(
                    serde_json::json!({"k":"start","id":id,"agent":agent,"task":task,"model":model}),
                );
            }
            subagent::Lifecycle::Progress { id, snippet } => {
                write(serde_json::json!({"k":"progress","id":id,"snippet":snippet}));
            }
            subagent::Lifecycle::Done {
                id,
                agent,
                ok,
                summary,
                cost_usd,
            } => {
                write(
                    serde_json::json!({"k":"done","id":id,"agent":agent,"ok":ok,"summary":summary,"cost":cost_usd}),
                );
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
        let Some(s) = &self.subagents else {
            return CallToolResult::error(vec![ContentBlock::text(
                "list_subagents is not available here",
            )]);
        };
        let children = s
            .ctx
            .store
            .list_detached_children(&s.parent_id)
            .unwrap_or_default();
        CallToolResult::success(vec![ContentBlock::text(subagent::format_subagent_list(
            &children,
        ))])
    }

    /// Bridge-side `cancel_subagent`: stop a still-running detached child, mirroring the direct
    /// path's `Session::cancel_subagent` semantics (best-effort in-process abort + the durable
    /// store transition; already-finished is reported, not an error).
    pub(super) async fn handle_cancel_subagent(&self, args: &Value) -> CallToolResult {
        let Some(s) = &self.subagents else {
            return CallToolResult::error(vec![ContentBlock::text(
                "cancel_subagent is not available here",
            )]);
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
            .list_detached_children(&s.parent_id)
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
