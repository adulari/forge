//! Running one tool call on behalf of a child.
//!
//! A child runs in parallel with its siblings and owns no interactive surface, so the parent's
//! `invoke_tool` contract does not transfer. An `Ask` decision goes to the context's
//! [`ChildAsker`](super::ChildAsker), which hosts the question on the parent's prompt; with no
//! asker it resolves to **Deny** with a reason the parent model can report. The safety denylist
//! still applies. Everything else about the child's loop lives with the loop.

use forge_tools::ToolRegistry;
use forge_types::{PermissionDecision, SideEffect, Usage};

use super::{rewrite_args_for_root, AgentCtx, ChildAnswer, ChildQuestion};
use crate::{permission, CoreError};

/// Run one subagent tool call through the permission gate. An `Ask` decision is put to the
/// context's asker (the parent's prompt); with none it is refused and says why. The safety
/// denylist always applies. `offered` is the child's toolset: a call outside it is refused.
pub(super) async fn execute_tool(
    ctx: &AgentCtx,
    registry: &ToolRegistry,
    offered: &[String],
    child: &str,
    msg_id: &str,
    call: &forge_types::ToolCall,
) -> Result<String, CoreError> {
    // A child's model fills in optional properties like any other: judge and run the call it meant.
    let args = registry.normalize_args(&call.name, &call.args);
    let args_json = serde_json::to_string(&args)?;
    let tool = registry
        .get(&call.name)
        .filter(|_| offered.contains(&call.name));
    let Some(tool) = tool else {
        let result = format!(
            "error: tool '{}' is not available to this subagent",
            call.name
        );
        ctx.store
            .record_tool_call(msg_id, &call.name, &args_json, &result, "n/a", "error")?;
        return Ok(result);
    };
    let side_effect = tool.side_effect();
    let mut refusal = None;
    let allowed = match permission::decide_in(
        ctx.mode,
        side_effect,
        &call.name,
        &args,
        &ctx.rules,
        Some(ctx.worktree_root.as_deref().unwrap_or(&ctx.repo_root)),
    ) {
        PermissionDecision::Allow => true,
        PermissionDecision::Deny => {
            refusal = Some("permission denied by policy".to_string());
            false
        }
        PermissionDecision::Ask => match &ctx.asker {
            Some(asker) => {
                let answer = asker(ChildQuestion {
                    child: child.to_string(),
                    tool: call.name.clone(),
                    side_effect,
                    args: args.clone(),
                })
                .await;
                match answer {
                    ChildAnswer::Allow => true,
                    ChildAnswer::Deny(note) => {
                        refusal = Some(match note {
                            Some(note) => format!(
                                "permission denied by the user: {}. The user said: {note}",
                                call.name
                            ),
                            None => format!(
                                "permission denied by the user: {}. Do not retry it; report what you would have done.",
                                call.name
                            ),
                        });
                        false
                    }
                }
            }
            None => {
                refusal = Some(format!(
                    "permission denied: `{}` needs approval in the `{}` temper and no one can be \
                     asked here (headless run). The tool exists; the session must run with a \
                     permissive temper (auto-edit or full) or an allow rule for it.",
                    call.name,
                    ctx.mode.key()
                ));
                false
            }
        },
    };
    let (result, status) = if allowed {
        // A daemon may host sessions rooted anywhere. Never let a child's relative path fall back
        // to the daemon process cwd: read-only children use the session repo, isolated writers use
        // their worktree, and shell receives an explicit cwd in both cases.
        let root = ctx.worktree_root.as_deref().unwrap_or(&ctx.repo_root);
        let mut effective_args = rewrite_args_for_root(&args, root);
        // An isolated child's worktree must not be bypassed by absolute paths of the checkout it
        // was branched from: those would edit the very tree the isolation protects.
        if ctx.worktree_root.is_some() {
            effective_args = forge_tools::remap_origin_paths(
                &effective_args,
                &ctx.repo_root,
                root,
                side_effect == SideEffect::ReadOnly,
            );
        }
        match tool.run(&effective_args).await {
            Ok(out) => (out, "ok"),
            Err(e) => (format!("error: {e}"), "error"),
        }
    } else {
        (
            refusal.unwrap_or_else(|| "permission denied by policy".to_string()),
            "error",
        )
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
