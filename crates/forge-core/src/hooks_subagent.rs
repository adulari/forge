//! `SubagentStart` (Claude-Code parity, docs/features/hooks.md): fires for each child agent as it
//! is spawned, with `agent_id` (the child session) and `agent_type` (what the matcher filters on).
//! It cannot stop the child; the `additionalContext` it returns is added to the child's task, so
//! the child reads it before it starts.

use std::path::Path;

use forge_config::{HookConfig, HookEvent};

use crate::hooks_events::{run_event_hooks, EventMatch};

/// The child's task with what the `SubagentStart` hooks asked to add.
pub(crate) async fn subagent_start_task(
    hooks: &[HookConfig],
    parent_id: &str,
    child_id: &str,
    agent_type: &str,
    cwd: &Path,
    task: String,
) -> String {
    if !hooks.iter().any(|h| h.event == HookEvent::SubagentStart) {
        return task;
    }
    let out = run_event_hooks(
        hooks,
        HookEvent::SubagentStart,
        parent_id,
        EventMatch::Value(agent_type),
        serde_json::json!({
            "agent_id": child_id,
            "agent_type": agent_type,
            "cwd": cwd.display().to_string(),
        }),
    )
    .await;
    for failure in &out.failures {
        tracing::warn!(%failure, agent_type, "SubagentStart hook failed");
    }
    if out.context.is_empty() {
        task
    } else {
        format!("{task}\n\n{}", out.context.join("\n\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[tokio::test]
    async fn the_hook_context_joins_the_task_of_the_agent_type_it_matches() {
        let hook = HookConfig {
            cc_compat: true,
            matcher: Some("reviewer".into()),
            ..HookConfig::new(
                HookEvent::SubagentStart,
                r#"grep -q '"agent_id":"child-1"' && echo '{"hookSpecificOutput":{"hookEventName":"SubagentStart","additionalContext":"follow docs/STYLE.md"}}'"#,
            )
        };
        let hooks = [hook];
        let cwd = std::env::temp_dir();
        let task = subagent_start_task(&hooks, "p", "child-1", "reviewer", &cwd, "review".into());
        assert_eq!(task.await, "review\n\nfollow docs/STYLE.md");
        let other = subagent_start_task(&hooks, "p", "child-1", "builder", &cwd, "build".into());
        assert_eq!(other.await, "build");
    }
}
