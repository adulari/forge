//! Detached (background) subagents (RFC retained-async-subagents): admission, the in-process
//! registry of running children, and the text the parent model reads back.
//!
//! A detached child outlives the turn that admitted it, so its lifecycle cannot flow through the
//! turn's `&mut` presenter the way [`super::orchestrate`]'s does. The caller hands
//! [`spawn_detached`] a [`DetachedSink`] instead — a `Send + Sync` callback that outlives the turn
//! (the session's background presenter sink, or `mcp-serve`'s out-of-band sink file) — and every
//! child reports Start, Progress and Done through it, so the agents panel shows it like any other.

use std::collections::HashMap;
use std::sync::Arc;

use forge_mesh::BudgetState;
use forge_provider::StreamEvent;

use super::{
    resolve, route_child, run_subagent, summary, AdmissionHandle, AgentCtx, AgentRequest, Lifecycle,
};
use crate::CoreError;

/// Where a detached child reports its lifecycle once the admitting turn has moved on.
pub type DetachedSink = Arc<dyn Fn(Lifecycle<'_>) + Send + Sync>;

/// In-memory handles for currently-running detached children, so `cancel_subagent` can actually
/// abort a live task in THIS process (the durable half — the `detached_child` store row — is
/// what makes cancellation/status correct even when the task is NOT in this process, e.g. after
/// a daemon restart; see [`forge_store::Store::reconcile_running_detached_children`]). Cheaply
/// cloneable so both the direct path (held on `Session`) and the CLI-bridge path (held on
/// `SubagentSupport`) can share one registry across every `spawn_agents`/`cancel_subagent` call
/// in the same process. A child removes its own entry when it finishes.
#[derive(Clone, Default)]
pub struct DetachedRegistry(Arc<std::sync::Mutex<HashMap<String, tokio::task::AbortHandle>>>);

impl DetachedRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, tokio::task::AbortHandle>> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Whether `child_id` is still running in this process.
    pub fn is_running(&self, child_id: &str) -> bool {
        self.lock().contains_key(child_id)
    }

    /// Ids of every child still running in this process.
    pub fn running_ids(&self) -> Vec<String> {
        self.lock().keys().cloned().collect()
    }

    /// Best-effort abort of the in-memory task for `child_id`, if this process holds it. Returns
    /// whether a handle was found — NOT whether the task was still running (aborting an
    /// already-completed task is a harmless no-op on tokio's side). The caller still consults the
    /// store row for the authoritative status.
    pub fn abort(&self, child_id: &str) -> bool {
        match self.lock().remove(child_id) {
            Some(handle) => {
                handle.abort();
                true
            }
            None => false,
        }
    }
}

/// Admit `requests` as DETACHED children under `parent_id`: create + register each one
/// immediately (persisted `running` row + a live tokio task), then return their admission
/// handles WITHOUT waiting for any of them to finish (RFC retained-async-subagents).
///
/// Deliberately does NOT hold the spawned tasks' `JoinHandle`s the way [`super::orchestrate`]'s
/// `AbortChildrenOnDrop` guard does — that guard exists so a CANCELLED TURN cancels its blocking
/// fan-out; a detached child's entire purpose is to outlive the turn (and the future) that
/// admitted it. Its `AbortHandle` still goes into `registry` so an explicit `cancel_subagent`
/// can stop it.
///
/// Each child's lifecycle goes to `sink`: Start at admission, Progress while it streams, and Done
/// once its result is in the store — store first, so `list_subagents` never lags the panel.
///
/// Structural depth guard: detached spawning is refused below the top level (`ctx.depth > 0`).
/// In practice a child never reaches here today — recursive `spawn_agents` calls route through
/// `run_nested_spawn`, which always runs blocking `orchestrate` and never parses `detached` —
/// but this makes the "children cannot spawn detached grandchildren" rule explicit and testable
/// independent of that call-site wiring.
pub async fn spawn_detached(
    ctx: &AgentCtx,
    registry: &DetachedRegistry,
    parent_id: &str,
    requests: Vec<AgentRequest>,
    budget: BudgetState,
    sink: Option<DetachedSink>,
) -> Result<Vec<AdmissionHandle>, CoreError> {
    if ctx.depth > 0 {
        return Err(CoreError::Internal(
            "detached subagents cannot themselves spawn detached grandchildren (depth guard)"
                .to_string(),
        ));
    }
    let mode_label = format!("{:?}", ctx.mode);
    let mut handles = Vec::with_capacity(requests.len());
    for req in requests {
        let mut resolved = resolve(&req, &ctx.agents);
        // Detached children return prose; a schema is not enforced for them.
        resolved.output_schema = None;
        let name = resolved.name.clone();
        let shown_task = resolved.task.clone();
        let child_cwd = ctx.repo_root.to_string_lossy();
        let child_id = ctx
            .store
            .create_child_session(&child_cwd, &mode_label, parent_id)?;
        let _ = ctx.store.set_session_title(&child_id, &name);
        resolved.task = crate::hooks_subagent::subagent_start_task(
            &ctx.config.hooks,
            parent_id,
            &child_id,
            &name,
            &ctx.repo_root,
            std::mem::take(&mut resolved.task),
        )
        .await;
        // Routed up front, same as the blocking path: deterministic, no API call, and the
        // admission handle can report the model immediately.
        let decision = route_child(ctx, &resolved, budget).await;
        let model = decision.model.clone();
        ctx.store
            .create_detached_child(&child_id, parent_id, &name, &model)?;
        if let Some(sink) = &sink {
            sink(Lifecycle::Start {
                id: &child_id,
                agent: &name,
                task: &shown_task,
                model: &model,
            });
        }

        let child_ctx = ctx.clone();
        let run_child_id = child_id.clone();
        let child_sink = sink.clone();
        let child_registry = registry.clone();
        // Hold the registry lock across the spawn so a child that finishes instantly cannot
        // remove its entry before it was inserted (no await happens while it is held).
        let mut running = registry.lock();
        let task = tokio::spawn(async move {
            let mut on_delta = |ev: StreamEvent| {
                let (StreamEvent::Text(snippet) | StreamEvent::Reasoning(snippet)) = ev else {
                    return;
                };
                if let Some(sink) = &child_sink {
                    sink(Lifecycle::Progress {
                        id: &run_child_id,
                        snippet: &snippet,
                    });
                }
            };
            let outcome = run_subagent(
                &child_ctx,
                &run_child_id,
                &resolved,
                decision,
                budget,
                &mut on_delta,
            )
            .await;
            let (text, ok) = match outcome {
                Ok(out) => (out.final_text, out.ok),
                Err(e) => (format!("error: subagent failed: {e}"), false),
            };
            let _ = child_ctx
                .store
                .finish_detached_child(&run_child_id, ok, &text);
            child_registry.lock().remove(&run_child_id);
            if let Some(sink) = &child_sink {
                let cost = child_ctx.store.session_cost(&run_child_id).unwrap_or(0.0);
                sink(Lifecycle::Done {
                    id: &run_child_id,
                    agent: &resolved.name,
                    ok,
                    summary: &summary(&text),
                    cost_usd: cost,
                });
            }
        });
        running.insert(child_id.clone(), task.abort_handle());
        drop(running);
        handles.push(AdmissionHandle {
            child_id,
            name,
            model,
        });
    }
    Ok(handles)
}

/// Resolve a `cancel_subagent`/status-lookup address against a parent's detached children: exact
/// name match wins (most-recent-first tiebreak), then an id-prefix match — the same resolution
/// order [`super::resolve_child_address`] uses for `send_to_agent`, for a consistent addressing
/// scheme across both subagent families.
pub fn resolve_detached_address<'a>(
    children: &'a [forge_store::DetachedChild],
    address: &str,
) -> Option<&'a forge_store::DetachedChild> {
    if let Some(found) = children.iter().rev().find(|c| c.name == address) {
        return Some(found);
    }
    let mut prefix_matches = children.iter().filter(|c| c.child_id.starts_with(address));
    match (prefix_matches.next(), prefix_matches.next()) {
        (Some(found), None) => Some(found),
        _ => None,
    }
}

/// Render the admission handles from a detached `spawn_agents` call as the tool result text the
/// parent model reads.
pub fn format_admission(handles: &[AdmissionHandle]) -> String {
    let mut out = format!(
        "admitted {} detached agent(s) — running in the background:\n",
        handles.len()
    );
    for h in handles {
        let short = &h.child_id[..h.child_id.len().min(8)];
        out.push_str(&format!("- {} ({short}) on {}\n", h.name, h.model));
    }
    out.push_str(
        "Results are delivered automatically as a labeled message once ready. Use \
         list_subagents to check status meanwhile, or cancel_subagent to stop one early.",
    );
    out
}

/// Render a parent's detached children (from [`forge_store::Store::list_detached_children`]) as
/// `list_subagents`' tool result text.
pub fn format_subagent_list(children: &[forge_store::DetachedChild]) -> String {
    if children.is_empty() {
        return "no detached agents spawned this session".to_string();
    }
    let mut out = String::new();
    for c in children {
        let short = &c.child_id[..c.child_id.len().min(8)];
        out.push_str(&format!(
            "- {} ({short}) [{}] model={}",
            c.name,
            c.status.as_str(),
            c.model
        ));
        if c.status.is_finished() {
            if let Some(result) = &c.result_ref {
                out.push_str(&format!(" — {}", summary(result)));
            }
        }
        out.push('\n');
    }
    out.trim_end().to_string()
}
