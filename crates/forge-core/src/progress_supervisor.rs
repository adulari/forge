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

    /// Judge the step that just finished. A nudge is delivered into the transcript after the
    /// step's tool results, so message ordering stays valid; `Some` means the turn must end now.
    pub(crate) fn supervise_step(
        &mut self,
        narration: &str,
        observations: &[CallObservation],
        task_closed: bool,
        unresolved_checks: Option<&str>,
    ) -> Option<StallStop> {
        let unattended = self.turn_is_unattended();
        let mesh = &self.config.mesh;
        let (idle_nudge, idle_stop, mode) = (
            mesh.idle_nudge_steps,
            mesh.idle_stop_steps,
            mesh.progress_guard,
        );
        self.ledger.configure(idle_nudge, idle_stop, unattended);
        let verdict = self
            .ledger
            .observe_step(narration, observations, task_closed);
        // Carried into the NEXT request's sampling (see `model_request`).
        self.repetition_pressure = self.ledger.pressure();
        match verdict {
            Verdict::Fine => None,
            Verdict::Nudge(report) => {
                self.deliver_stall_nudge(&report, unresolved_checks);
                None
            }
            Verdict::Stop(report) => self.settle_stop(&report, mode, unattended),
        }
    }

    fn deliver_stall_nudge(&mut self, report: &StallReport, unresolved_checks: Option<&str>) {
        self.ledger
            .record_nudge_tree(crate::completeness::worktree_fingerprint(
                self.workspace_root(),
            ));
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
        unattended: bool,
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
        // Shell-driven work (a silent `sed -i`, a `git commit`) looks stale to the ledger. If the
        // tree moved since the nudge, the turn did something: start over instead of stopping.
        if self.ledger.has_nudge_tree() {
            let now = crate::completeness::worktree_fingerprint(self.workspace_root());
            if self.ledger.tree_changed_since_nudge(now) {
                self.ledger.veto_stop("the working tree changed");
                return None;
            }
        }
        self.ledger.stopped();
        let _ = self.push_note(forge_types::MessageKind::LoopGuard, report.persisted_note());
        Some(self.abort_for_stall(report, unattended))
    }

    /// End the turn for a stall. An attended turn gets a warning and a status the model writes;
    /// an unattended one has nobody to read a warning, so it fails with the uncommitted work
    /// preserved and the way to resume, like every other unattended abort.
    fn abort_for_stall(&mut self, report: &StallReport, unattended: bool) -> StallStop {
        self.turn_forced_stop = Some(StopReason::Stalled);
        if !unattended {
            self.presenter
                .emit(forge_types::PresenterEvent::Warning(report.stop_text()));
            return StallStop {
                final_text: String::new(),
                hard: false,
            };
        }
        let work = self.preserve_uncommitted_work();
        let text = format!(
            "ERROR: {}\n{work}\nResume: forge run --continue",
            report.stop_headline()
        );
        self.presenter
            .emit(forge_types::PresenterEvent::Error(text.clone()));
        self.turn_hard_guard_abort = true;
        StallStop {
            final_text: text,
            hard: true,
        }
    }
}
