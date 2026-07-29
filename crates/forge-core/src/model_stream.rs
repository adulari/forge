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
    build_fight: &std::sync::Arc<std::sync::atomic::AtomicU64>,
    verification: &std::sync::Arc<std::sync::Mutex<VerificationLedger>>,
    pending_observations: &std::sync::Arc<
        std::sync::Mutex<
            std::collections::HashMap<String, std::collections::VecDeque<VerificationObservation>>,
        >,
    >,
    tools_unavailable: &std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    act.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    match ev {
        StreamEvent::ProviderActivity => presenter.emit(PresenterEvent::ProviderProgress),
        StreamEvent::Text(t) => {
            if !suppress_assistant_text {
                presenter.emit(PresenterEvent::AssistantDelta(t));
            }
        }
        StreamEvent::Reasoning(t) => presenter.emit(PresenterEvent::Reasoning(t)),
        StreamEvent::ToolStarted { name, args } => {
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
            presenter.emit(PresenterEvent::ToolStart { name, args })
        }
        StreamEvent::ToolFinished { name, ok, summary } => {
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
            verification.lock().unwrap().observe(observation, ok);
            presenter.emit(PresenterEvent::ToolResult { name, ok, summary })
        }
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
    }
}
