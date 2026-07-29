//! Running one tool call on behalf of a child, with nobody to ask.
//!
//! A subagent is headless by construction: it runs in parallel with its siblings and has no
//! interactive surface, so the parent's `invoke_tool` contract does not transfer. Here an `Ask`
//! decision resolves to **Deny** rather than blocking on a prompt that can never be answered, no
//! presenter events are emitted, and the safety denylist still applies. Everything else about the
//! child's loop lives with the loop.

use forge_tools::ToolRegistry;
use forge_types::{PermissionDecision, Usage};

use super::{rewrite_args_for_root, AgentCtx};
use crate::{permission, CoreError};

/// Run one subagent tool call through the permission gate (headless). Differs from the parent's
/// `invoke_tool` in that there is no interactive surface: an `Ask` decision resolves to **Deny**
/// (a parallel/headless child can't prompt), and no presenter events are emitted. The safety
/// denylist always applies. Unknown / non-read-only tools are refused.
pub(super) async fn execute_tool(
    ctx: &AgentCtx,
    registry: &ToolRegistry,
    msg_id: &str,
    call: &forge_types::ToolCall,
) -> Result<String, CoreError> {
    let args_json = serde_json::to_string(&call.args)?;
    let Some(tool) = registry.get(&call.name) else {
        let result = format!("error: tool '{}' is not available to subagents", call.name);
        ctx.store
            .record_tool_call(msg_id, &call.name, &args_json, &result, "n/a", "error")?;
        return Ok(result);
    };
    let side_effect = tool.side_effect();
    let allowed =
        match permission::decide(ctx.mode, side_effect, &call.name, &call.args, &ctx.rules) {
            PermissionDecision::Allow => true,
            // No interactive surface in a subagent → Ask becomes Deny (safe default).
            PermissionDecision::Deny | PermissionDecision::Ask => false,
        };
    let (result, status) = if allowed {
        // A daemon may host sessions rooted anywhere. Never let a child's relative path fall back
        // to the daemon process cwd: read-only children use the session repo, isolated writers use
        // their worktree, and shell receives an explicit cwd in both cases.
        let root = ctx.worktree_root.as_deref().unwrap_or(&ctx.repo_root);
        let effective_args = rewrite_args_for_root(&call.args, root);
        match tool.run(&effective_args).await {
            Ok(out) => (out, "ok"),
            Err(e) => (format!("error: {e}"), "error"),
        }
    } else {
        ("permission denied by policy".to_string(), "error")
    };
    ctx.store.record_tool_call(
        msg_id,
        &call.name,
        &args_json,
        &result,
        if allowed { "allowed" } else { "denied" },
        status,
    )?;
    Ok(result)
}

/// Child tools normally return failures as `error: ...`, but batched `read_file` preserves a
/// labelled per-file result (`===== file =====\n[error: ...]`). Treat both wire shapes as failed
/// so a workflow cannot paint a child green after its only read actually failed.
pub(super) fn tool_result_failed(result: &str) -> bool {
    let lower = result.to_ascii_lowercase();
    lower.starts_with("error:")
        || lower.starts_with("permission denied")
        || lower
            .lines()
            .any(|line| line.trim_start().starts_with("[error:"))
}

/// Sum the token/cost usage of a list of usages (helper for rollups).
pub fn sum_usage(items: impl IntoIterator<Item = Usage>) -> Usage {
    items.into_iter().fold(Usage::default(), |mut acc, u| {
        acc.input_tokens += u.input_tokens;
        acc.output_tokens += u.output_tokens;
        acc.cost_usd += u.cost_usd;
        acc
    })
}
