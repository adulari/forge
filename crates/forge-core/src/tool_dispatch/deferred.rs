//! The direct path's deferred tool registry (docs/features/deferred-tools.md): the lean advertised
//! surface, the `tool_call` unwrap that every loop counter and gate keys on, and `tool_search`.
//! The batch-eligibility check lives here too, because it must judge the unwrapped call.

use super::*;
use crate::deferred_tools;

impl Session {
    fn lean_tools(&self) -> bool {
        deferred_tools::lean_enabled_with(
            self.config.mesh.direct_lean,
            std::env::var(deferred_tools::DIRECT_LEAN_ENV)
                .ok()
                .as_deref(),
        )
    }

    fn is_direct_core(&self, name: &str) -> bool {
        deferred_tools::is_core(name)
            || deferred_tools::DIRECT_CORE_EXTRA.contains(&name)
            || self.config.mesh.lean_core_extra.iter().any(|n| n == name)
    }

    /// `full` split into the advertised core and the deferred catalog, with the external-MCP
    /// aliases (`mcp_search_tools`/`mcp_call`) in neither: `tool_search`/`tool_call` cover them.
    /// The flag says whether external MCP servers are connected.
    fn deferred_partition(&self, full: Vec<ToolSpec>) -> (Vec<ToolSpec>, Vec<ToolSpec>, bool) {
        let alias = |s: &ToolSpec| deferred_tools::MCP_ALIASES.contains(&s.name.as_str());
        let external = full.iter().any(alias);
        let full: Vec<ToolSpec> = full.into_iter().filter(|s| !alias(s)).collect();
        let (core, deferred) = deferred_tools::partition_by(full, |n| self.is_direct_core(n));
        (core, deferred, external)
    }

    /// What the model is shown: `full` unchanged when the lean surface is off, else the slimmed
    /// core plus `tool_search`/`tool_call`, sorted for a byte-stable prefix.
    pub(crate) fn advertised_tool_specs(&self, full: Vec<ToolSpec>) -> Vec<ToolSpec> {
        if !self.lean_tools() {
            return full;
        }
        let (core, deferred, external) = self.deferred_partition(full);
        let mut specs = deferred_tools::lean_surface(core, &deferred, external);
        specs.sort_by(|left, right| left.name.cmp(&right.name));
        specs
    }

    /// The call `call` really is: a `tool_call{name, arguments}` becomes the inner call (same id,
    /// so the transcript pairing holds), routed through `mcp_call` when it names an external
    /// `server__tool`. Anything else, including a malformed `tool_call`, comes back unchanged.
    pub(crate) fn resolve_deferred_call(
        &self,
        call: &forge_types::ToolCall,
    ) -> forge_types::ToolCall {
        let Some(Ok(inner)) = deferred_tools::unwrap_tool_call(call) else {
            return call.clone();
        };
        self.route_external_mcp(inner)
    }

    /// `inner` unchanged unless it names an external `server__tool` the registry does not hold;
    /// that becomes an `mcp_call`. Shared with `run_code`, whose scripts call tools by bare name.
    pub(crate) fn route_external_mcp(&self, inner: forge_types::ToolCall) -> forge_types::ToolCall {
        let external = inner.name.contains("__")
            && self.tools.get(&inner.name).is_none()
            && self
                .mcp
                .as_ref()
                .is_some_and(|m| m.knows_tool(forge_mcp::MCP_CALL));
        if !external {
            return inner;
        }
        forge_types::ToolCall {
            id: inner.id,
            name: forge_mcp::MCP_CALL.to_string(),
            args: serde_json::json!({ "name": inner.name, "arguments": inner.args }),
        }
    }

    /// `tool_search` over the deferred catalog (plus connected MCP servers), or the error for a
    /// `tool_call` that names nothing runnable. A valid `tool_call` never gets here: it was
    /// unwrapped before dispatch.
    pub(super) async fn invoke_deferred_meta(
        &mut self,
        msg_id: &str,
        call: &forge_types::ToolCall,
    ) -> Result<String, CoreError> {
        let args_json = serde_json::to_string(&call.args)?;
        self.presenter.emit(PresenterEvent::ToolStart {
            name: call.name.clone(),
            args: args_json.clone(),
        });
        let (result, ok) = match deferred_tools::unwrap_tool_call(call) {
            Some(Err(error)) => (error, false),
            Some(Ok(inner)) => (format!("error: unknown tool '{}'", inner.name), false),
            None => (self.search_deferred(call).await, true),
        };
        self.presenter.emit(PresenterEvent::ToolResult {
            name: call.name.clone(),
            ok,
            summary: summarize(&result),
            detail: crate::tool_detail(&result),
        });
        self.store.record_tool_call(
            msg_id,
            &call.name,
            &args_json,
            &result,
            "allowed",
            if ok { "ok" } else { "error" },
        )?;
        Ok(result)
    }

    /// Takes `&mut self` only so the returned future stays `Send` (`Session` is not `Sync`).
    async fn search_deferred(&mut self, call: &forge_types::ToolCall) -> String {
        let query = call
            .args
            .get("query")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let (_, deferred, external) = self.deferred_partition(self.full_tool_specs());
        let builtin = (!deferred_tools::matches(&deferred, query).is_empty() || !external)
            .then(|| deferred_tools::search(&deferred, query));
        let mcp = match (self.mcp.clone(), external) {
            (Some(mcp), true) => {
                let out = mcp
                    .call(
                        forge_mcp::MCP_SEARCH_TOOLS,
                        &serde_json::json!({ "query": query }),
                    )
                    .await;
                (out.ok && !out.text.trim().is_empty()).then(|| {
                    format!(
                        "## MCP server tools (run with tool_call{{\"name\":\"server__tool\",\"arguments\":{{…}}}})\n{}",
                        out.text.trim()
                    )
                })
            }
            _ => None,
        };
        match (builtin, mcp) {
            (Some(b), Some(m)) => format!("{b}\n\n{m}"),
            (Some(b), None) => b,
            (None, Some(m)) => m,
            (None, None) => deferred_tools::search(&deferred, query),
        }
    }

    /// Whether `call` is side-effect-free and safe to run concurrently in a batch: not a core-owned
    /// virtual tool (those mutate session state / prompt the user), not an external MCP tool,
    /// present in the registry, and ReadOnly — or a `shell` call that provably only reads
    /// ([`permission::is_read_only_shell_call`]), so independent probes stop running one by one.
    pub(crate) fn is_concurrent_readonly(&self, call: &forge_types::ToolCall) -> bool {
        let name = call.name.as_str();
        // The batch path skips the per-call task-scope gate, so a tool the scope forbids must take
        // the serial path, which refuses it properly.
        if self
            .task_scope
            .as_ref()
            .is_some_and(|scope| !scope.permits_tool(name))
        {
            return false;
        }
        if name == "shell" {
            // Only when the broker would allow it outright: the batch path never prompts, so a
            // shell line that would ask must go through the serial path's confirmation instead.
            return self.tools.get(name).is_some()
                && permission::is_read_only_shell_call(&call.args)
                && permission::decide(
                    self.mode,
                    forge_types::SideEffect::Shell,
                    name,
                    &call.args,
                    &self.rules,
                ) == PermissionDecision::Allow;
        }
        if name == subagent::SPAWN_AGENTS_TOOL
            || name == workflow::RUN_WORKFLOW_TOOL
            || name == ASK_USER_TOOL
            || name == UPDATE_TASKS_TOOL
            || name == PRESENT_PLAN_TOOL
            || name == USE_SKILL_TOOL
            || name == REMEMBER_TOOL
            || name == fleet::MESSAGE_SESSION_TOOL
            || name == fleet::LIST_SESSIONS_TOOL
            || name == crate::dispatch::DISPATCH_SESSIONS_TOOL
            || name == heartbeat::MANAGE_HEARTBEATS_TOOL
            || name == wakeup::SCHEDULE_WAKEUP_TOOL
            || name == push_notify::PUSH_NOTIFICATION_TOOL
        {
            return false;
        }
        if self.mcp.as_ref().is_some_and(|m| m.knows_tool(name)) {
            return false;
        }
        self.tools
            .get(name)
            .map(|t| t.side_effect() == forge_types::SideEffect::ReadOnly)
            .unwrap_or(false)
    }
}
