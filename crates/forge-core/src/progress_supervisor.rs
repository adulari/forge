//! The session side of progress supervision: feed each finished tool step to the ledger and act
//! on its verdict, delivering the one nudge or ending the turn cleanly.
//!
//! The judgement is [`crate::progress_ledger`]; this file is the part that touches the
//! transcript, the presenter and the working tree. A stalled turn is stopped with
//! [`StopReason::Stalled`], never the success path: the edits are kept, the person is told why and
//! how to go on, and the next turn's model is told what it was stopped on.

use super::*;
use crate::loop_progress::CallObservation;
use crate::progress_ledger::Verdict;
use crate::progress_report::StallReport;
use forge_config::ProgressGuardMode;

/// What a stall verdict asks of the step loop.
pub(crate) struct StallStop {
    /// The final text of the turn. Empty when the loop should have the model write a status.
    pub(crate) final_text: String,
    /// Whether the stop is as hard as a spend ceiling: no status call, no re-drive.
    pub(crate) hard: bool,
}

impl Session {
    /// Tracked tasks marked Done so far.
    pub(crate) fn done_task_count(&self) -> usize {
        self.tasks
            .iter()
            .filter(|t| matches!(t.status, forge_types::TodoStatus::Done))
            .count()
    }

    /// Whether no person is watching this turn: nobody to read a warning or answer a prompt. A
    /// person who chose `Bypass` is still watching, so the permission mode is not asked.
    pub(crate) fn turn_is_headless(&self) -> bool {
        !self.presenter.is_attended()
    }

    /// Give the progress ledger the tree a pass of the turn starts from, which is what it judges
    /// what shell-driven steps did to the tree against.
    pub(crate) fn rebase_progress_tree(&mut self, fingerprint: Option<u64>) {
        let state = fingerprint
            .or_else(|| crate::working_tree::progress_fingerprint(self.workspace_root()));
        self.ledger.rebase_tree(state);
    }

    /// Judge the step that just finished. A nudge is delivered into the transcript after the
    /// step's tool results, so message ordering stays valid; `Some` means the turn must end now.
    pub(crate) fn supervise_step(
        &mut self,
        narration: &str,
        observations: &[CallObservation],
        task_closed: bool,
        unresolved_checks: Option<&str>,
    ) -> Option<StallStop> {
        let headless = self.turn_is_headless();
        let mesh = &self.config.mesh;
        let (idle_nudge, idle_stop, mode) = (
            mesh.idle_nudge_steps,
            mesh.idle_stop_steps,
            mesh.progress_guard,
        );
        self.ledger.configure(idle_nudge, idle_stop, headless);
        self.ledger
            .set_repeat_interrupt(self.harness_flags.repeat_interrupt);
        let root = self.workspace_root().to_path_buf();
        let verdict = self
            .ledger
            .observe_step(narration, observations, task_closed, &mut || {
                crate::working_tree::progress_fingerprint(&root)
            });
        self.repeat_interrupt();
        // Carried into the NEXT request's sampling (see `model_request`).
        self.repetition_pressure = self.ledger.pressure();
        match verdict {
            Verdict::Fine => None,
            Verdict::Nudge(report) => {
                self.deliver_stall_nudge(&report, unresolved_checks);
                None
            }
            Verdict::Stop(report) => self.settle_stop(&report, mode, headless),
        }
    }

    fn deliver_stall_nudge(&mut self, report: &StallReport, unresolved_checks: Option<&str>) {
        // A `minimal` profile sends no nudge, but the ledger still counts, so a model that keeps
        // finding nothing new is stopped on the same schedule.
        if !self.advisory_nudges_on() {
            return;
        }
        self.presenter
            .emit(forge_types::PresenterEvent::Warning(report.nudge_notice()));
        let open_tasks = crate::nudge_policy::open_titles(&self.tasks).len();
        let text = report.nudge_text(unresolved_checks, open_tasks);
        let _ = self.push_note(forge_types::MessageKind::LoopGuard, text);
    }

    fn settle_stop(
        &mut self,
        report: &StallReport,
        mode: ProgressGuardMode,
        headless: bool,
    ) -> Option<StallStop> {
        match mode {
            ProgressGuardMode::Enforce => {}
            ProgressGuardMode::Warn => {
                self.presenter.emit(forge_types::PresenterEvent::Warning(
                    report.would_stop_text(),
                ));
                self.ledger.suppress_stop();
                return None;
            }
            ProgressGuardMode::Off => {
                self.ledger.suppress_stop();
                return None;
            }
        }
        self.ledger.stopped();
        let _ = self.push_note(forge_types::MessageKind::LoopGuard, report.persisted_note());
        Some(self.abort_for_stall(&report.stop_headline(), headless))
    }

    /// `forge mcp-serve`'s ledger stopped the bridged turn that just streamed, if it did: the model
    /// has answered, as it was told to, so that answer is the turn's result. The parent does not
    /// re-drive a plan whose calls were going nowhere; `continue` starts it afresh.
    pub(crate) fn take_bridge_stall(
        &mut self,
        reply: &str,
    ) -> Result<Option<StallStop>, CoreError> {
        let headless = self.turn_is_headless();
        let Some(headline) = self.bridge_stall.take() else {
            return Ok(None);
        };
        let headline = headline.as_str();
        let _ = self.push_note(
            forge_types::MessageKind::LoopGuard,
            format!(
                "Forge stopped your previous turn. {headline} Do not resume the same calls: \
                 change approach, or say what is blocking you."
            ),
        );
        let mut stop = self.abort_for_stall(headline, headless);
        if !headless {
            stop.final_text = reply.to_string();
            self.publish_terminal_answer(reply)?;
        }
        Ok(Some(stop))
    }

    /// End the turn for a stall. An attended turn gets a warning and a status the model writes;
    /// a headless one has nobody to read a warning, so it fails with the uncommitted work
    /// preserved and the way to resume, like every other unattended abort.
    fn abort_for_stall(&mut self, headline: &str, headless: bool) -> StallStop {
        self.turn_forced_stop = Some(StopReason::Stalled);
        if !headless {
            self.presenter
                .emit(forge_types::PresenterEvent::Warning(format!(
                    "{headline}\nSend `continue`, or tell it what to try."
                )));
            return StallStop {
                final_text: String::new(),
                hard: false,
            };
        }
        let work = self.preserve_uncommitted_work();
        let text = format!("ERROR: {headline}\n{work}\nResume: forge run --continue");
        self.presenter
            .emit(forge_types::PresenterEvent::Error(text.clone()));
        self.turn_hard_guard_abort = true;
        StallStop {
            final_text: text,
            hard: true,
        }
    }
}
