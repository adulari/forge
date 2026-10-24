//! Progress supervision for a CLI bridge turn, from inside `forge mcp-serve`.
//!
//! A bridged model (claude, codex) runs its own tool loop in a subprocess, so the parent session
//! sees one response per subprocess run and none of the individual calls: the per-step ledger
//! never runs there. Every call does land in `mcp-serve`, with its full result, and that is where
//! this ledger sits. It watches the same evidence as the direct loop (a new write, a closed task,
//! a passing check, a repeated call with an identical result, consecutive failures, steps that
//! show nothing new) with the narration and idle lanes off: each call is one step, with no prose
//! to compare and no person-visible clock to measure against.
//!
//! A nudge is prefixed to the result of the call that earned it, so the model reads it in-band.
//! A stop replaces that result, and every later call's, with a refusal that tells the model to
//! make no more tool calls and answer; the caller also reports it to the parent through the sink
//! so the parent ends the turn as stalled instead of re-driving a plan that is going nowhere.

use crate::completion::{classify_tool, VerificationLedger};
use crate::loop_progress::CallObservation;
use crate::progress_ledger::{ProgressLedger, Verdict};
use forge_config::ProgressGuardMode;
use std::path::Path;

/// What one bridged call earned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BridgeVerdict {
    Fine,
    /// Text to put in front of this call's result.
    Nudge(String),
    /// The turn is over. `refusal` replaces this and every later result; `headline` is what the
    /// person is told.
    Stop {
        refusal: String,
        headline: String,
    },
}

pub struct BridgeProgress {
    ledger: ProgressLedger,
    verification: VerificationLedger,
    mode: ProgressGuardMode,
    turn: Option<i64>,
    stopped: Option<String>,
    tasks_done: usize,
}

impl Default for BridgeProgress {
    fn default() -> Self {
        Self::new(ProgressGuardMode::Enforce)
    }
}

impl BridgeProgress {
    pub fn new(mode: ProgressGuardMode) -> Self {
        Self {
            ledger: ProgressLedger::for_bridge(),
            verification: VerificationLedger::default(),
            mode,
            turn: None,
            stopped: None,
            tasks_done: 0,
        }
    }

    /// The refusal for any call made after the turn was stopped.
    pub fn refusal(&self) -> Option<&str> {
        self.stopped.as_deref()
    }

    /// A new user turn restarts the ledger: a long-lived bridge process serves many.
    fn follow_turn(&mut self, turn: Option<i64>) {
        if turn != self.turn {
            self.turn = turn;
            self.ledger.begin_turn();
            self.verification = VerificationLedger::default();
            self.stopped = None;
            self.tasks_done = 0;
        }
    }

    /// Whether the turn the caller is in was already stopped, after following the turn change.
    pub fn stopped_in(&mut self, turn: Option<i64>) -> Option<&str> {
        self.follow_turn(turn);
        self.stopped.as_deref()
    }

    /// Judge one finished call. `tasks_done` is the count of Done tasks after an `update_tasks`
    /// call (`None` for every other call): a rise is a closed task.
    #[allow(clippy::too_many_arguments)]
    pub fn observe_call(
        &mut self,
        turn: Option<i64>,
        tool: &str,
        args: &serde_json::Value,
        ok: bool,
        mutating: bool,
        result: &str,
        tasks_done: Option<usize>,
        root: &Path,
    ) -> BridgeVerdict {
        self.follow_turn(turn);
        if self.stopped.is_some() {
            return BridgeVerdict::Fine;
        }
        let call = forge_types::ToolCall {
            id: String::new(),
            name: tool.to_string(),
            args: args.clone(),
        };
        let verification = classify_tool(tool, &args.to_string());
        let flipped = self.verification.observe_result(verification, ok, result);
        let observation = CallObservation::for_call(&call, ok, mutating, result)
            .with_check(verification.check_family(), flipped);
        let task_closed = tasks_done.is_some_and(|done| {
            let closed = done > self.tasks_done;
            self.tasks_done = done;
            closed
        });
        let verdict = self
            .ledger
            .observe_step("", &[observation], task_closed, &mut || {
                crate::working_tree::progress_fingerprint(root)
            });
        match verdict {
            Verdict::Fine => BridgeVerdict::Fine,
            Verdict::Nudge(report) => BridgeVerdict::Nudge(
                report.nudge_text(self.verification.unresolved_summary().as_deref(), 0),
            ),
            Verdict::Stop(report) => match self.mode {
                ProgressGuardMode::Enforce => {
                    self.ledger.stopped();
                    let headline = report.stop_headline();
                    let refusal = format!(
                        "Forge stopped this turn: {headline} Make no more tool calls; reply now \
                         with a short summary of what you found and what is still open."
                    );
                    self.stopped = Some(refusal.clone());
                    BridgeVerdict::Stop { refusal, headline }
                }
                ProgressGuardMode::Warn => {
                    self.ledger.suppress_stop();
                    BridgeVerdict::Nudge(report.would_stop_text())
                }
                ProgressGuardMode::Off => {
                    self.ledger.suppress_stop();
                    BridgeVerdict::Fine
                }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(
        p: &mut BridgeProgress,
        turn: i64,
        tool: &str,
        args: serde_json::Value,
        ok: bool,
        result: &str,
    ) -> BridgeVerdict {
        p.observe_call(
            Some(turn),
            tool,
            &args,
            ok,
            tool == "write_file",
            result,
            None,
            Path::new("/nonexistent-forge-bridge-root"),
        )
    }

    fn same_read(p: &mut BridgeProgress, turn: i64) -> BridgeVerdict {
        run(
            p,
            turn,
            "read_file",
            serde_json::json!({"path": "a.rs"}),
            true,
            "fn a() {}\nfn b() {}\nfn c() {}\nfn d() {}",
        )
    }

    #[test]
    fn twelve_identical_calls_are_nudged_then_stopped_and_later_calls_are_refused() {
        let mut p = BridgeProgress::new(ProgressGuardMode::Enforce);
        let verdicts: Vec<BridgeVerdict> = (0..8).map(|_| same_read(&mut p, 1)).collect();
        let nudge = verdicts
            .iter()
            .position(|v| matches!(v, BridgeVerdict::Nudge(_)))
            .expect("a nudge");
        assert_eq!(nudge, 2, "the third identical call: {verdicts:?}");
        let BridgeVerdict::Nudge(text) = &verdicts[nudge] else {
            unreachable!()
        };
        assert!(
            text.contains("ran 3 times with identical results"),
            "{text}"
        );
        let stop = verdicts
            .iter()
            .position(|v| matches!(v, BridgeVerdict::Stop { .. }))
            .expect("a stop");
        assert_eq!(stop, 4, "{verdicts:?}");
        let BridgeVerdict::Stop { refusal, headline } = &verdicts[stop] else {
            unreachable!()
        };
        assert!(refusal.contains("Make no more tool calls"), "{refusal}");
        assert!(headline.starts_with("Stopped:"), "{headline}");
        assert_eq!(p.refusal(), Some(refusal.as_str()));
        assert_eq!(
            same_read(&mut p, 1),
            BridgeVerdict::Fine,
            "later calls are refused by the caller; the ledger no longer judges"
        );
    }

    #[test]
    fn a_new_turn_lifts_the_refusal_but_a_repeat_of_the_stopped_call_is_stopped_again() {
        let mut p = BridgeProgress::new(ProgressGuardMode::Enforce);
        for _ in 0..6 {
            same_read(&mut p, 1);
        }
        assert!(p.refusal().is_some());
        assert!(p.stopped_in(Some(2)).is_none());
        assert!(p.refusal().is_none());
        let other = run(
            &mut p,
            2,
            "read_file",
            serde_json::json!({"path": "b.rs"}),
            true,
            "fn other() {}\nfn more() {}",
        );
        assert_eq!(other, BridgeVerdict::Fine);
        assert!(
            matches!(same_read(&mut p, 2), BridgeVerdict::Stop { .. }),
            "the call the last turn was stopped on, again, right after `continue`"
        );
    }

    #[test]
    fn a_productive_run_of_edits_and_reads_is_never_judged() {
        let mut p = BridgeProgress::new(ProgressGuardMode::Enforce);
        for i in 0..200 {
            let v = if i % 2 == 0 {
                run(
                    &mut p,
                    1,
                    "write_file",
                    serde_json::json!({"path": format!("f{i}.rs"), "content": "x"}),
                    true,
                    "wrote",
                )
            } else {
                run(
                    &mut p,
                    1,
                    "read_file",
                    serde_json::json!({"path": format!("f{i}.rs")}),
                    true,
                    &format!("unique contents {i}\nline two {i}\nline three {i}"),
                )
            };
            assert_eq!(v, BridgeVerdict::Fine, "step {i}");
        }
    }

    #[test]
    fn consecutive_failures_are_nudged_at_three_and_stopped_at_six() {
        let mut p = BridgeProgress::new(ProgressGuardMode::Enforce);
        let verdicts: Vec<BridgeVerdict> = (0..8)
            .map(|i| {
                run(
                    &mut p,
                    1,
                    "edit_file",
                    serde_json::json!({"path": "a.rs", "n": i}),
                    false,
                    "error: no match for old_string",
                )
            })
            .collect();
        assert!(
            matches!(verdicts[2], BridgeVerdict::Nudge(_)),
            "{verdicts:?}"
        );
        assert!(
            matches!(verdicts[5], BridgeVerdict::Stop { .. }),
            "{verdicts:?}"
        );
    }

    #[test]
    fn warn_and_off_never_refuse() {
        for mode in [ProgressGuardMode::Warn, ProgressGuardMode::Off] {
            let mut p = BridgeProgress::new(mode);
            for _ in 0..12 {
                let v = same_read(&mut p, 1);
                assert!(!matches!(v, BridgeVerdict::Stop { .. }), "{mode:?}");
            }
            assert!(p.refusal().is_none());
        }
    }

    #[test]
    fn a_closed_task_is_progress() {
        let mut p = BridgeProgress::new(ProgressGuardMode::Enforce);
        for i in 0..4 {
            same_read(&mut p, 1);
            let _ = i;
        }
        let v = p.observe_call(
            Some(1),
            "update_tasks",
            &serde_json::json!({}),
            true,
            false,
            "task list updated",
            Some(1),
            Path::new("/nonexistent-forge-bridge-root"),
        );
        assert_eq!(v, BridgeVerdict::Fine);
        for _ in 0..2 {
            assert_eq!(same_read(&mut p, 1), BridgeVerdict::Fine);
        }
    }

    fn shell_call(
        p: &mut BridgeProgress,
        root: &Path,
        command: &str,
        ok: bool,
        result: &str,
    ) -> BridgeVerdict {
        p.observe_call(
            Some(1),
            "shell",
            &serde_json::json!({ "command": command }),
            ok,
            false,
            result,
            None,
            root,
        )
    }

    #[test]
    fn a_bridged_wait_on_a_running_job_or_a_slept_poll_is_never_a_loop() {
        let root = Path::new("/nonexistent-forge-bridge-root");
        let mut p = BridgeProgress::new(ProgressGuardMode::Enforce);
        for _ in 0..40 {
            let v = p.observe_call(
                Some(1),
                "shell_job",
                &serde_json::json!({"action": "status", "id": 7}),
                true,
                false,
                "job 7 is running\n  command: cargo build\n  log: /tmp/j.log",
                None,
                root,
            );
            assert_eq!(v, BridgeVerdict::Fine);
        }
        let mut p = BridgeProgress::new(ProgressGuardMode::Enforce);
        for _ in 0..40 {
            let v = shell_call(
                &mut p,
                root,
                "sleep 60; gh run view 9",
                true,
                "shell: exit 0 in 60004ms\n\nin progress",
            );
            assert!(!matches!(v, BridgeVerdict::Stop { .. }), "{v:?}");
        }
        assert!(
            p.refusal().is_none(),
            "a bridge never stops a wait: it cannot tell if anyone is watching"
        );
    }

    #[test]
    fn searches_that_find_nothing_are_not_a_failure_even_in_parallel_batches() {
        let root = Path::new("/nonexistent-forge-bridge-root");
        let mut p = BridgeProgress::new(ProgressGuardMode::Enforce);
        for i in 0..12 {
            let v = shell_call(
                &mut p,
                root,
                &format!("rg needle_{i} src"),
                false,
                "shell: exit 1 in 4ms",
            );
            assert!(!matches!(v, BridgeVerdict::Stop { .. }), "call {i}: {v:?}");
        }
    }

    /// A file rewritten into states it already was in is stopped, one that keeps reaching new
    /// states is not, whatever the period.
    #[test]
    fn a_bridged_file_that_flips_between_known_states_is_stopped_but_one_that_grows_is_not() {
        for period in [2usize, 3, 4] {
            let dir = tempfile::tempdir().unwrap();
            let mut p = BridgeProgress::new(ProgressGuardMode::Enforce);
            std::fs::write(dir.path().join("f.txt"), "state 0").unwrap();
            // A first call establishes the baseline: the bridge has no earlier state to compare.
            shell_call(&mut p, dir.path(), "true", true, "shell: exit 0 in 3ms");
            let mut stopped = None;
            for i in 1..=60 {
                std::fs::write(dir.path().join("f.txt"), format!("state {}", i % period)).unwrap();
                let command = format!("sed -i s/x/y/ f.txt # {}", i % period);
                if let BridgeVerdict::Stop { .. } =
                    shell_call(&mut p, dir.path(), &command, true, "shell: exit 0 in 3ms")
                {
                    stopped = Some(i);
                    break;
                }
            }
            assert!(
                stopped.is_some_and(|i| i <= 40),
                "period {period}: {stopped:?}"
            );
        }
        let dir = tempfile::tempdir().unwrap();
        let mut p = BridgeProgress::new(ProgressGuardMode::Enforce);
        shell_call(&mut p, dir.path(), "true", true, "shell: exit 0 in 3ms");
        for i in 1..=120 {
            std::fs::write(dir.path().join("log.txt"), "x".repeat(i)).unwrap();
            let v = shell_call(
                &mut p,
                dir.path(),
                "echo x >> log.txt",
                true,
                "shell: exit 0 in 3ms",
            );
            assert_eq!(v, BridgeVerdict::Fine, "call {i}");
        }
    }
}
