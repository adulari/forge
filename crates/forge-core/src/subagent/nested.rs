//! A child agent delegating further: one level deeper through the same orchestrator.

use super::{orchestrate, parse_requests, AgentCtx, BudgetState, Lifecycle};

/// A child agent delegating further: recurse one level deeper. Grandchildren are not surfaced
/// to the live UI (no-op lifecycle) — their results roll up into this child's tool result.
/// `Box::pin` breaks the orchestrate→run_subagent→orchestrate async-recursion type cycle.
/// Returns a **boxed** (concrete, non-opaque) future so it does not participate in the
/// orchestrate→run_subagent async-`impl Future` opaque-type cycle; `+ Send` asserts Send at the
/// boundary. Owned args so the future is `'static`.
pub(super) fn run_nested_spawn(
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
