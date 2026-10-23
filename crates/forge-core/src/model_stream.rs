//! Provider stream-event projection into session telemetry and presenter events.

use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) fn handle_stream_event(
    ev: StreamEvent,
    presenter: &mut dyn Presenter,
    suppress_assistant_text: bool,
    in_plan_mode: bool,
    proposed_plan: &mut Option<forge_types::PlanProposal>,
    act: &std::sync::Arc<std::sync::atomic::AtomicU64>,
    active: &std::sync::Arc<std::sync::atomic::AtomicU64>,
    tools: &std::sync::Arc<std::sync::atomic::AtomicU64>,
    inspects: &std::sync::Arc<std::sync::atomic::AtomicU64>,
    // `mutations`: successful state-changing calls a CLI bridge made this loop. The bridge runs its
    // tool loop in a subprocess, so this stream is the ONLY place its writes are observable —
    // without it a bridge turn always looks like it mutated nothing.
    mutations: &std::sync::Arc<std::sync::atomic::AtomicU64>,
    build_fight: &std::sync::Arc<std::sync::atomic::AtomicU64>,
    verification: &std::sync::Arc<std::sync::Mutex<VerificationLedger>>,
    pending_observations: &std::sync::Arc<
        std::sync::Mutex<
            std::collections::HashMap<String, std::collections::VecDeque<VerificationObservation>>,
        >,
    >,
    tools_unavailable: &std::sync::Arc<std::sync::atomic::AtomicBool>,
    bridge_trace: &std::sync::Arc<std::sync::Mutex<crate::bridge_trace::BridgeTrace>>,
) {
    if act.fetch_add(1, std::sync::atomic::Ordering::Relaxed) == 0 {
        crate::turn_trace::first_event(match &ev {
            StreamEvent::Text(_) => "text",
            StreamEvent::Reasoning(_) => "reasoning",
            StreamEvent::ProviderThinking { .. } => "thinking",
            StreamEvent::ProviderActivity => "activity",
            _ => "other",
        });
    }
    match ev {
        StreamEvent::ProviderActivity => presenter.emit(PresenterEvent::ProviderProgress),
        StreamEvent::ProviderToolInput { name, chars } => {
            presenter.emit(PresenterEvent::ProviderToolInput { name, chars })
        }
        StreamEvent::ProviderThinking { tokens } => {
            presenter.emit(PresenterEvent::ProviderThinking { tokens })
        }
        StreamEvent::Text(t) => {
            crate::turn_trace::text_delta();
            if !suppress_assistant_text {
                presenter.emit(PresenterEvent::AssistantDelta(t));
            }
        }
        StreamEvent::Reasoning(t) => presenter.emit(PresenterEvent::Reasoning(t)),
        StreamEvent::ToolStarted { name, args } => {
            bridge_trace.lock().unwrap().started(&name, &args);
            active.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            tools.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            // Bookkeeping tools don't count as a real inspection — the
            // verification gate needs an actual state CHECK (read/shell/…).
            if !name.ends_with("update_tasks") && !name.ends_with("present_plan") {
                inspects.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            // Bridge-aware env/build-fight tracking (wave 5, fix 2): a bridge's
            // shell tools surface here, not in `resp.tool_calls`, so this is the
            // only place the build/provision-command pattern is observable.
            if is_env_setup_command(&bridge_tool_command(&args)) {
                build_fight.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            pending_observations
                .lock()
                .unwrap()
                .entry(name.clone())
                .or_default()
                .push_back(completion::classify_tool(&name, &args));
            presenter.emit(PresenterEvent::ToolStart {
                name: forge_tool_name(name),
                args,
            })
        }
        StreamEvent::ToolFinished { name, ok, summary } => {
            bridge_trace.lock().unwrap().finished(&name, ok, &summary);
            let _ = active.fetch_update(
                std::sync::atomic::Ordering::Relaxed,
                std::sync::atomic::Ordering::Relaxed,
                |count| Some(count.saturating_sub(1)),
            );
            let observation = pending_observations
                .lock()
                .unwrap()
                .get_mut(&name)
                .and_then(std::collections::VecDeque::pop_front)
                .unwrap_or(VerificationObservation::Generic);
            verification
                .lock()
                .unwrap()
                .observe_result(observation, ok, &summary);
            if ok && completion::tool_name_mutates(&name) {
                mutations.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            // A CLI bridge reports only a summary line for its own tool calls — there is no raw
            // output to hand a surface, so an expandable card here stays header-only.
            presenter.emit(PresenterEvent::ToolResult {
                name: forge_tool_name(name),
                ok,
                summary,
                detail: None,
            })
        }
        StreamEvent::ModelCalls(calls) => bridge_trace.lock().unwrap().add_model_calls(calls),
        // `mcp-serve`'s ledger stopped the turn; remembered until the response is recorded.
        StreamEvent::Stall { headline } => bridge_trace.lock().unwrap().stalled(headline),
        StreamEvent::SubagentStarted { id, agent, task } => {
            presenter.emit(PresenterEvent::SubagentStart {
                id,
                agent,
                task,
                model: None,
                phase: None,
            })
        }
        StreamEvent::SubagentProgress { id, snippet } => {
            presenter.emit(PresenterEvent::SubagentProgress { id, snippet })
        }
        StreamEvent::SubagentFinished {
            id,
            agent,
            ok,
            summary,
            cost_usd,
        } => presenter.emit(PresenterEvent::SubagentResult {
            id,
            agent,
            ok,
            summary,
            cost_usd,
        }),
        // A bridged turn's `update_tasks` (tailed from the sink): surface the
        // list live so the sticky panel updates during the turn. The parent's
        // post-turn store reload (below) keeps `self.tasks` authoritative.
        StreamEvent::Tasks(tasks) => presenter.emit(PresenterEvent::Tasks(tasks)),
        // A bridged turn's `present_plan`: in planning mode, render the
        // card now and stash it for the turn's approval flow (picked up
        // via the outcome). Ignored outside Plan mode (stray proposal).
        StreamEvent::Plan(plan) => {
            if in_plan_mode {
                presenter.emit(PresenterEvent::PlanProposed(plan.clone()));
                *proposed_plan = Some(plan);
            }
        }
        // The bridge's `mcp-serve` tool server failed to start this turn (wave 7):
        // the model's write tools were never exposed. Latch it for the toolless-
        // bridge classification in `run_turn`. Deliberately does NOT emit a
        // presenter event — interactive turns stay behaviourally unchanged; only
        // headless `expect_code_change` runs act on it (classify + retry).
        StreamEvent::ToolsUnavailable { reason: _ } => {
            tools_unavailable.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        // A bridged tool hit an `Ask` decision in `mcp-serve`, which is waiting for an answer.
        // Ask exactly as a native tool would — this presenter is the TUI or the daemon's remote
        // surface — and hand the decision back through the answer file.
        StreamEvent::PermissionRequest {
            tool,
            side_effect,
            answer_path,
            args,
            diff,
        } => {
            // Same order as a native write: the diff first, then the question about it.
            if let Some(diff) = diff {
                presenter.emit(PresenterEvent::Diff(*diff));
            }
            let args = args.unwrap_or(serde_json::Value::Null);
            // One word, and on a deny the user's note on the lines after it (the relay's refusal
            // carries it to the bridged model).
            let reply = permission::confirm_call(presenter, &tool, &args, side_effect);
            let answer = match (reply.outcome, reply.note) {
                (forge_types::ConfirmOutcome::AlwaysAllow, _) => "always".to_string(),
                (forge_types::ConfirmOutcome::AllowSession, _) => "session".to_string(),
                (forge_types::ConfirmOutcome::Allow, _) => "allow".to_string(),
                (forge_types::ConfirmOutcome::Deny, Some(note)) => format!("deny\n{note}"),
                (forge_types::ConfirmOutcome::Deny, None) => "deny".to_string(),
            };
            if let Err(error) = std::fs::write(&answer_path, answer) {
                presenter.emit(PresenterEvent::Warning(format!(
                    "could not hand the {tool} permission answer to the bridge: {error}"
                )));
            }
        }
    }
}

/// A bridge reports Forge's own tools under claude's MCP namespace (`mcp__forge__edit_file`).
/// Surfaces key cards, headlines and the permission prompt on the tool's own name, so a bridged
/// card is named exactly like a native one; other MCP servers keep their namespace.
fn forge_tool_name(name: String) -> String {
    match name.strip_prefix("mcp__forge__") {
        Some(tool) if !tool.is_empty() => tool.to_string(),
        _ => name,
    }
}

#[cfg(test)]
mod tests {
    use super::forge_tool_name;

    #[test]
    fn bridged_forge_tools_carry_their_native_names() {
        assert_eq!(forge_tool_name("mcp__forge__edit_file".into()), "edit_file");
        assert_eq!(forge_tool_name("edit_file".into()), "edit_file");
        assert_eq!(
            forge_tool_name("mcp__helm__search".into()),
            "mcp__helm__search"
        );
        assert_eq!(forge_tool_name("mcp__forge__".into()), "mcp__forge__");
    }
}
