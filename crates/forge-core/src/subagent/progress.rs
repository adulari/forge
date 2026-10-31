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
        structured: None,
    }
}

/// The child is judged like the parent: nudged once, then stopped when it stops getting anywhere.
/// A child has no idle stop of its own: whether a person watches it is the parent's business, and
/// a child that really loops is stopped by the same lanes as the parent. It is held to its agent
/// type's `maxTurns` where that is set.
pub(super) struct ChildSupervisor {
    ledger: ProgressLedger,
    guard: ProgressGuardMode,
    root: std::path::PathBuf,
}

impl ChildSupervisor {
    /// `root` is the directory the child works in, which is where its shell commands leave their
    /// mark.
    pub(super) fn new(config: &Config, root: std::path::PathBuf) -> Self {
        let mut ledger = ProgressLedger::default();
        ledger.configure(config.mesh.idle_nudge_steps, 0, false);
        ledger.rebase_tree(crate::working_tree::progress_fingerprint(&root));
        Self {
            ledger,
            guard: config.mesh.progress_guard,
            root,
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
        let root = &self.root;
        let verdict = self
            .ledger
            .observe_step(narration, observations, false, &mut || {
                crate::working_tree::progress_fingerprint(root)
            });
        match verdict {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loop_progress::CallObservation;

    /// A child that converts a file per step with the same silent command.
    fn migrate(files_change: bool) -> Option<usize> {
        let dir = tempfile::tempdir().unwrap();
        let mut child = ChildSupervisor::new(&Config::default(), dir.path().to_path_buf());
        let mut transcript = vec![Message::tool_result("c", "ok".to_string())];
        let mut sink = |_: StreamEvent| {};
        for n in 0..40 {
            if files_change {
                std::fs::write(dir.path().join(format!("out_{n}.txt")), format!("{n}")).unwrap();
            }
            let same = {
                let call = forge_types::ToolCall {
                    id: format!("c{n}"),
                    name: "shell".into(),
                    args: serde_json::json!({ "command": "./convert-next.sh" }),
                };
                CallObservation::for_call(&call, true, false, "shell: exit 0 in 3ms")
            };
            if child
                .observe(
                    "Converting the next one.",
                    &[same],
                    &mut transcript,
                    &mut sink,
                )
                .is_some()
            {
                return Some(n + 1);
            }
        }
        None
    }

    #[test]
    fn a_child_whose_shell_commands_change_the_tree_is_never_stopped() {
        assert_eq!(migrate(true), None);
    }

    #[test]
    fn a_child_whose_shell_commands_change_nothing_is_stopped() {
        assert!(migrate(false).is_some_and(|step| step <= 8));
    }

    #[test]
    fn a_child_has_no_idle_stop_but_is_still_nudged() {
        let dir = tempfile::tempdir().unwrap();
        let mut child = ChildSupervisor::new(&Config::default(), dir.path().to_path_buf());
        let mut transcript = vec![Message::tool_result("c", "ok".to_string())];
        let mut sink = |_: StreamEvent| {};
        let mut stopped = false;
        for n in 0..300 {
            let call = forge_types::ToolCall {
                id: format!("c{n}"),
                name: "read_file".into(),
                args: serde_json::json!({ "path": format!("f{n}.rs") }),
            };
            let read = CallObservation::for_call(
                &call,
                true,
                false,
                &format!("fn item_{n}() {{}}\nfn other_{n}() {{}}"),
            );
            stopped |= child
                .observe(&format!("reading {n}"), &[read], &mut transcript, &mut sink)
                .is_some();
        }
        assert!(!stopped, "300 steps of new reads");
        assert!(
            transcript[0].content.contains("Progress check from Forge"),
            "the idle nudge was appended to the last tool result"
        );
    }
}
