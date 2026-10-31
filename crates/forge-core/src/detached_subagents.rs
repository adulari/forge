//! Detached-subagent session methods: listing, cancelling, and delivering the results of children
//! that finished since the last turn boundary.
//!
//! Split out of `orchestration.rs` so that file stays inside the architecture-size limit for a file
//! the baseline does not already track (800 implementation lines). These three belong together
//! anyway — they are the `spawn_agents(detached: true)` lifecycle seen from the parent session,
//! whereas `orchestration.rs` owns the in-turn fan-out.

use super::*;

/// The presenter event for one subagent lifecycle step, shared by blocking and detached spawns so
/// both reach the agents panel (TUI rows, the remote snapshot's `subagents`) the same way.
pub(crate) fn lifecycle_event(ev: subagent::Lifecycle) -> PresenterEvent {
    match ev {
        subagent::Lifecycle::Start {
            id,
            agent,
            task,
            model,
        } => PresenterEvent::SubagentStart {
            id: id.to_string(),
            agent: agent.to_string(),
            task: task.to_string(),
            model: Some(model.to_string()),
            phase: None,
        },
        subagent::Lifecycle::Progress { id, snippet } => PresenterEvent::SubagentProgress {
            id: id.to_string(),
            snippet: snippet.to_string(),
        },
        subagent::Lifecycle::Done {
            id,
            agent,
            ok,
            summary,
            cost_usd,
        } => PresenterEvent::SubagentResult {
            id: id.to_string(),
            agent: agent.to_string(),
            ok,
            summary: summary.to_string(),
            cost_usd,
        },
    }
}

impl Session {
    /// Stop every detached child `parent_session` (this session, or the one just replaced by a
    /// fresh one) still has running: abort the task held in this
    /// process and settle its row. Used before the session is deleted, so no task keeps writing
    /// to rows that are about to vanish. Returns how many were cancelled.
    pub fn cancel_all_detached(&self, parent_session: &str) -> usize {
        let Ok(children) = self.store.list_detached_children(parent_session) else {
            return 0;
        };
        let mut cancelled = 0;
        for child in children
            .iter()
            .filter(|c| c.status == forge_store::DetachedChildStatus::Running)
        {
            self.detached_registry.abort(&child.child_id);
            if matches!(self.store.cancel_detached_child(&child.child_id), Ok(true)) {
                cancelled += 1;
            }
        }
        cancelled
    }

    /// Where detached children report their lifecycle after the admitting turn has ended: the
    /// presenter's background sink. `None` on a surface without one (plain headless output),
    /// where the labeled result message at the next turn boundary is the only report.
    pub(crate) fn detached_sink(&self) -> Option<subagent::DetachedSink> {
        let sink = std::sync::Mutex::new(self.presenter.recap_sink()?);
        Some(Arc::new(move |ev: subagent::Lifecycle| {
            sink.lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .emit(lifecycle_event(ev));
        }))
    }

    /// Handle a `list_subagents` call: report every detached child spawned this session (RFC
    /// retained-async-subagents), with live status.
    pub(crate) fn list_subagents(
        &mut self,
        msg_id: &str,
        call: &forge_types::ToolCall,
    ) -> Result<String, CoreError> {
        let args_json = serde_json::to_string(&call.args)?;
        let children = self.store.list_detached_children(&self.id)?;
        let result = subagent::format_subagent_list(&children);
        self.store
            .record_tool_call(msg_id, &call.name, &args_json, &result, "allowed", "ok")?;
        Ok(result)
    }

    /// Handle a `cancel_subagent` call: stop a still-running detached child (RFC
    /// retained-async-subagents). Best-effort in-process abort plus the durable store transition;
    /// a child that already finished (or was never detached) is reported, not errored.
    pub(crate) fn cancel_subagent(
        &mut self,
        msg_id: &str,
        call: &forge_types::ToolCall,
    ) -> Result<String, CoreError> {
        let args_json = serde_json::to_string(&call.args)?;
        let fail = |result: String, store: &Store| -> Result<String, CoreError> {
            store.record_tool_call(msg_id, &call.name, &args_json, &result, "allowed", "error")?;
            Ok(result)
        };
        let address = call
            .args
            .get("agent")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if address.is_empty() {
            return fail(
                "error: cancel_subagent needs `agent` (name or id prefix)".into(),
                &self.store,
            );
        }
        let children = self.store.list_detached_children(&self.id)?;
        let Some(child) = subagent::resolve_detached_address(&children, &address) else {
            let known: Vec<String> = children
                .iter()
                .map(|c| format!("{} ({})", c.name, &c.child_id[..c.child_id.len().min(8)]))
                .collect();
            return fail(
                format!(
                    "error: no detached agent matches '{address}'. Detached agents this \
                     session: [{}]",
                    known.join(", ")
                ),
                &self.store,
            );
        };
        if child.status != forge_store::DetachedChildStatus::Running {
            let result = format!(
                "'{}' ({}) is already {} — nothing to cancel",
                child.name,
                &child.child_id[..child.child_id.len().min(8)],
                child.status.as_str()
            );
            self.store
                .record_tool_call(msg_id, &call.name, &args_json, &result, "allowed", "ok")?;
            return Ok(result);
        }
        let child_id = child.child_id.clone();
        let name = child.name.clone();
        // Best-effort: abort the in-memory task if it's running in THIS process, then flip the
        // durable row regardless (also correct if the task is running in a different process —
        // the store transition is what `list_subagents`/delivery actually consult).
        self.detached_registry.abort(&child_id);
        self.store.cancel_detached_child(&child_id)?;
        // The aborted task never reports Done itself, so settle its panel row here.
        self.presenter.emit(PresenterEvent::SubagentResult {
            id: child_id.clone(),
            agent: name.clone(),
            ok: false,
            summary: "cancelled".to_string(),
            cost_usd: self.store.session_cost(&child_id).unwrap_or(0.0),
        });
        let result = format!(
            "cancelled '{name}' ({})",
            &child_id[..child_id.len().min(8)]
        );
        self.store
            .record_tool_call(msg_id, &call.name, &args_json, &result, "allowed", "ok")?;
        Ok(result)
    }

    /// Deliver any of this session's detached children that finished since the last turn boundary
    /// (RFC retained-async-subagents, ADR-0004): inject each as a labeled system message into the
    /// transcript (persisted + pushed live), then mark it delivered so it isn't repeated. Called
    /// at the top of every `run_turn_with`; a no-op when nothing finished meanwhile.
    pub(crate) fn deliver_pending_detached_results(&mut self) -> Result<(), CoreError> {
        let pending = self.store.undelivered_detached_children(&self.id)?;
        for child in pending {
            let short = &child.child_id[..child.child_id.len().min(8)];
            let label = format!(
                "[detached agent '{}' ({short}) {}]\n{}",
                child.name,
                child.status.as_str(),
                child
                    .result_ref
                    .as_deref()
                    .unwrap_or("(no result recorded)"),
            );
            // Settles a panel row nothing else did: a child whose process ended before it
            // reported (a bridge turn, a restart). A row already settled ignores the repeat.
            self.presenter.emit(PresenterEvent::SubagentResult {
                id: child.child_id.clone(),
                agent: child.name.clone(),
                ok: child.status == forge_store::DetachedChildStatus::Done,
                summary: subagent::summary(child.result_ref.as_deref().unwrap_or("")),
                cost_usd: self.store.session_cost(&child.child_id).unwrap_or(0.0),
            });
            self.presenter.emit(PresenterEvent::Warning(format!(
                "detached agent '{}' {} — result delivered",
                child.name,
                child.status.as_str()
            )));
            let seq = self.next_seq();
            self.store
                .add_message(&self.id, seq, Role::System, &label, None)?;
            self.transcript.push(Message::system(&label));
            self.store.mark_detached_delivered(&child.child_id)?;
        }
        Ok(())
    }
}
