//! A child agent's share of progress supervision: its own ledger, its step limit, and what the
//! parent is handed back when the child is stopped.

use super::SubagentOutcome;
use crate::progress_ledger::{ProgressLedger, Verdict};
use forge_config::{Config, ProgressGuardMode};
use forge_provider::StreamEvent;
use forge_types::Message;

/// The explicit cap on a child's model calls: the agent type's `maxTurns`, never above
/// `[mesh] max_steps` when that is set. `None` means unlimited; a child that stops getting
/// anywhere is stopped by its own progress ledger instead.
pub(crate) fn step_limit(agent_max_turns: Option<usize>, mesh_max_steps: usize) -> Option<usize> {
    match (agent_max_turns, mesh_max_steps) {
        (Some(n), 0) => Some(n.max(1)),
        (Some(n), cap) => Some(n.min(cap).max(1)),
        (None, 0) => None,
        (None, cap) => Some(cap),
    }
}

pub(super) fn step_label(step: usize, limit: Option<usize>) -> String {
    match limit {
        Some(limit) => format!("{}/{limit}", step + 1),
        None => (step + 1).to_string(),
    }
}

pub(super) fn no_answer_error(limit: Option<usize>) -> String {
    match limit {
        Some(limit) => format!(
            "error: subagent produced no usable final answer (blank output, or it hit the \
             {limit}-step limit without answering)"
        ),
        None => "error: subagent produced no usable final answer (blank output)".to_string(),
    }
}

/// A stalled child is not a clean answer, but what it found is not thrown away: the parent gets
/// the child's last words and the reason it was stopped.
pub(super) fn stalled_outcome(last_words: String, headline: &str) -> SubagentOutcome {
    let partial = last_words.trim();
    let final_text = if partial.is_empty() {
        format!("error: subagent {}", headline.to_lowercase())
    } else {
        format!("{partial}\n\n[{headline}]")
    };
    SubagentOutcome {
        final_text,
        ok: false,
    }
}

/// The child is judged like the parent: nudged once, then stopped when it stops getting anywhere.
/// Nobody watches a child, so it gets the unattended idle stop.
pub(super) struct ChildSupervisor {
    ledger: ProgressLedger,
    guard: ProgressGuardMode,
}

impl ChildSupervisor {
    pub(super) fn new(config: &Config) -> Self {
        let mut ledger = ProgressLedger::default();
        ledger.configure(
            config.mesh.idle_nudge_steps,
            config.mesh.idle_stop_steps,
            true,
        );
        Self {
            ledger,
            guard: config.mesh.progress_guard,
        }
    }

    /// Judge one finished step. A nudge is appended to the last tool result, because a system
    /// message in the middle of a child's transcript is not accepted by every provider. `Some`
    /// is the headline of the stop that must end the child.
    pub(super) fn observe(
        &mut self,
        narration: &str,
        observations: &[crate::loop_progress::CallObservation],
        transcript: &mut [Message],
        on_delta: &mut (dyn FnMut(StreamEvent) + Send),
    ) -> Option<String> {
        match self.ledger.observe_step(narration, observations, false) {
            Verdict::Fine => None,
            Verdict::Nudge(report) => {
                if let Some(last) = transcript.last_mut() {
                    last.content
                        .push_str(&format!("\n\n{}", report.nudge_text(None, 0)));
                }
                None
            }
            Verdict::Stop(report) => match self.guard {
                ProgressGuardMode::Enforce => Some(report.stop_headline()),
                ProgressGuardMode::Warn => {
                    on_delta(StreamEvent::Reasoning(report.would_stop_text()));
                    self.ledger.suppress_stop();
                    None
                }
                ProgressGuardMode::Off => {
                    self.ledger.suppress_stop();
                    None
                }
            },
        }
    }
}
