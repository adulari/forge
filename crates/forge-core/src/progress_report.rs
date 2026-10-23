//! What the progress ledger tells the model and the person when a turn stops getting anywhere.
//!
//! A report names observations (what failed, what repeated, which check is still red), never a
//! step count alone: "you have taken 100 steps" says nothing about whether the turn is stuck,
//! while "`shell` failed 4 times (command not found: x)" says what to change.

/// Which signal fired.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Lane {
    /// The same opening sentence again and again while nothing new turns up.
    Narration,
    /// Consecutive steps that changed nothing and showed nothing new.
    Futile,
    /// Consecutive steps in which every call failed.
    Failure,
    /// The same calls with identical results.
    Cycle,
    /// A check that keeps failing with the same output however much is edited between runs.
    Check,
    /// A wait for something external that does not end.
    Waiting,
    /// Many steps without a file change, a closed task, a passing check or a steer.
    Idle,
    /// A resumed turn repeated what it was stopped on.
    Resumed,
}

impl Lane {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Lane::Narration => "narration",
            Lane::Futile => "futile",
            Lane::Failure => "failure",
            Lane::Cycle => "cycle",
            Lane::Check => "check",
            Lane::Waiting => "waiting",
            Lane::Idle => "idle",
            Lane::Resumed => "resumed",
        }
    }

    /// The stop reason in a sentence.
    fn reason(self) -> &'static str {
        match self {
            Lane::Narration => "repeating itself with nothing new",
            Lane::Futile => "no progress",
            Lane::Failure => "the same calls failing",
            Lane::Cycle => "the same calls returning the same results",
            Lane::Check => "the same check failing the same way after every edit",
            Lane::Waiting => "still waiting for something that is not ready",
            Lane::Idle => "no file change or finished task",
            Lane::Resumed => "the same steps again after a stop",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StallReport {
    pub(crate) lane: Lane,
    /// Steps in the stretch that tripped the lane.
    pub(crate) steps: u32,
    /// The last step that changed something, and what it was.
    pub(crate) last_progress: Option<(u32, String)>,
    /// A call that kept coming back with the same result, and how often.
    pub(crate) repeated: Option<(String, u32)>,
    /// A tool that kept failing, the failure's cause and how often.
    pub(crate) failing: Option<(String, String, u32)>,
    /// A check family that failed with the same output, and how many runs in a row.
    pub(crate) check: Option<(&'static str, u32)>,
}

impl StallReport {
    /// The observations as one `;`-separated line, or `None` when there are none to name.
    fn observed(&self, unresolved_checks: Option<&str>, open_tasks: usize) -> Option<String> {
        let mut parts = Vec::new();
        if let Some((tool, cause, times)) = &self.failing {
            parts.push(format!("`{tool}` failed {times} times ({cause})"));
        }
        if let Some((label, times)) = &self.repeated {
            parts.push(format!(
                "`{label}` ran {times} times with identical results"
            ));
        }
        if let Some((family, runs)) = self
            .check
            .filter(|(_, runs)| *runs >= 2 && self.lane != Lane::Check)
        {
            parts.push(format!(
                "`{family}` failed {runs} times with the same output"
            ));
        }
        if let Some(checks) = unresolved_checks {
            parts.push(format!("the `{checks}` check is still failing"));
        }
        match open_tasks {
            0 => {}
            1 => parts.push("1 task open".to_string()),
            n => parts.push(format!("{n} tasks open")),
        }
        (!parts.is_empty()).then(|| parts.join("; "))
    }

    fn last_progress_line(&self) -> Option<String> {
        self.last_progress
            .as_ref()
            .map(|(step, what)| format!("Last real progress: step {step} ({what})."))
    }

    /// The model-facing nudge, delivered once per episode as a `LoopGuard` note.
    pub(crate) fn nudge_text(&self, unresolved_checks: Option<&str>, open_tasks: usize) -> String {
        let observed = self.observed(unresolved_checks, open_tasks);
        let mut text = match self.lane {
            Lane::Narration => crate::stall_guard::STALL_NUDGE.to_string(),
            Lane::Idle => format!(
                "Progress check from Forge. {} steps without changing a file or closing a task. \
                 If this is an investigation, report what you found and what you still need; \
                 otherwise make the change.",
                self.steps
            ),
            Lane::Waiting => format!(
                "Progress check from Forge. You have waited {} steps for something that is still \
                 not ready. Tell the person what you are waiting for and for how long, and stop \
                 polling; if they say to keep waiting, poll again.",
                self.steps
            ),
            Lane::Check => format!(
                "Progress check from Forge. The same check has now failed {} times with the same \
                 output, so the edits in between are not changing what it reports. Read the \
                 failing output again, confirm the edits touch the code that fails, or say what \
                 is blocking you.",
                self.steps
            ),
            _ => format!(
                "Progress check from Forge. In your last {} steps nothing changed and nothing \
                 new turned up.",
                self.steps
            ),
        };
        if let Some(observed) = observed {
            text.push_str(&format!("\nObserved: {observed}."));
        }
        if let Some(line) = self.last_progress_line() {
            text.push_str(&format!("\n{line}"));
        }
        if !matches!(
            self.lane,
            Lane::Narration | Lane::Idle | Lane::Waiting | Lane::Check
        ) {
            text.push_str(
                "\nEither fix the cause, try a different approach, answer from what you have and \
                 say what is uncertain, or, if waiting on something external, say so and stop \
                 polling.",
            );
        }
        text
    }

    /// The one-line notice that goes with the nudge.
    pub(crate) fn nudge_notice(&self) -> String {
        // An attended turn is never stopped for taking long, so promising a stop would be false.
        let then = match self.lane {
            Lane::Idle | Lane::Waiting => "asking the model to report",
            _ => "nudging the model once before stopping",
        };
        format!(
            "progress check ({}): {} for {} steps — {then}",
            self.lane.as_str(),
            self.lane.reason(),
            self.steps
        )
    }

    /// Why the turn stopped and that its edits are kept.
    pub(crate) fn stop_headline(&self) -> String {
        let what = match &self.last_progress {
            Some((step, what)) => format!(" (last change: step {step}, {what})"),
            None => String::new(),
        };
        let span = match self.lane {
            Lane::Resumed => String::new(),
            Lane::Check => format!(" ({} runs)", self.steps),
            _ => format!(" for {} steps", self.steps),
        };
        format!(
            "Stopped: {}{span}{what}. Your edits are kept.",
            self.lane.reason()
        )
    }

    /// The same, for `progress_guard = "warn"`: where the turn would have stopped.
    pub(crate) fn would_stop_text(&self) -> String {
        format!(
            "progress_guard = warn: would have stopped here ({}: {} for {} steps); the turn goes on",
            self.lane.as_str(),
            self.lane.reason(),
            self.steps
        )
    }

    /// The note stored when a turn is stopped, so the next turn's model knows why and does not
    /// resume the calls that stalled it.
    pub(crate) fn persisted_note(&self) -> String {
        let evidence = self
            .check
            .filter(|_| self.lane == Lane::Check)
            .map(|(family, runs)| format!(": `{family}` failed {runs} times with the same output"))
            .or_else(|| {
                self.failing.as_ref().map(|(tool, cause, times)| {
                    format!(": `{tool}` failed {times} times ({cause})")
                })
            })
            .or_else(|| {
                self.repeated.as_ref().map(|(label, times)| {
                    format!(": `{label}` ran {times} times with identical results")
                })
            })
            .unwrap_or_default();
        let advice = if self.lane == Lane::Waiting {
            "If the person says to keep waiting, poll again; otherwise say what you were waiting for."
        } else {
            "Do not resume the same calls: change approach, or say what is blocking you."
        };
        format!(
            "Forge stopped your previous turn ({}: {}{evidence}). {advice}",
            self.lane.as_str(),
            self.lane.reason()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(lane: Lane) -> StallReport {
        StallReport {
            lane,
            steps: 4,
            last_progress: Some((31, "edit_file src/lib.rs".into())),
            repeated: Some(("read_file a.rs".into(), 3)),
            failing: Some(("shell".into(), "command not found: x".into(), 4)),
            check: Some(("test", 7)),
        }
    }

    #[test]
    fn the_nudge_names_what_was_observed_and_the_way_out() {
        let text = report(Lane::Cycle).nudge_text(Some("test"), 2);
        assert!(text.starts_with("Progress check from Forge. In your last 4 steps"));
        assert!(
            text.contains("`shell` failed 4 times (command not found: x)"),
            "{text}"
        );
        assert!(text.contains("`read_file a.rs` ran 3 times with identical results"));
        assert!(text.contains("the `test` check is still failing"));
        assert!(text.contains("2 tasks open"));
        assert!(text.contains("Last real progress: step 31 (edit_file src/lib.rs)."));
        assert!(text.contains("try a different approach"));
    }

    #[test]
    fn the_narration_nudge_keeps_its_wording_and_the_idle_one_is_gentler() {
        let narration = report(Lane::Narration).nudge_text(None, 0);
        assert!(narration.starts_with(crate::stall_guard::STALL_NUDGE));
        let idle = report(Lane::Idle).nudge_text(None, 0);
        assert!(idle.contains("4 steps without changing a file or closing a task"));
        assert!(idle.contains("report what you found"));
        assert!(!idle.contains("Either fix the cause"));
    }

    #[test]
    fn the_stop_says_why_and_that_edits_are_kept() {
        let text = report(Lane::Futile).stop_headline();
        assert!(text.starts_with("Stopped: no progress for 4 steps (last change: step 31"));
        assert!(text.ends_with("Your edits are kept."));
        let bare = StallReport {
            last_progress: None,
            ..report(Lane::Futile)
        };
        assert!(!bare.stop_headline().contains("last change"));
    }

    #[test]
    fn the_check_and_waiting_lanes_say_what_to_do_and_how_long_it_went_on() {
        let check = report(Lane::Check);
        let text = check.nudge_text(None, 0);
        assert!(
            text.contains("failed 4 times with the same output"),
            "{text}"
        );
        assert_eq!(
            text.matches("failed 7 times").count(),
            0,
            "the lane's own sentence is not repeated under Observed"
        );
        assert!(check
            .stop_headline()
            .contains("the same check failing the same way"));
        assert!(check.stop_headline().contains("(4 runs)"));
        assert!(check.persisted_note().contains("`test` failed 7 times"));

        let waiting = report(Lane::Waiting);
        assert!(waiting.nudge_text(None, 0).contains("waited 4 steps"));
        assert!(waiting
            .persisted_note()
            .contains("If the person says to keep waiting, poll again"));
        assert!(!waiting
            .persisted_note()
            .contains("Do not resume the same calls"));
    }

    #[test]
    fn a_resumed_stop_does_not_count_steps() {
        let text = report(Lane::Resumed).stop_headline();
        assert!(
            text.starts_with("Stopped: the same steps again after a stop (last change"),
            "{text}"
        );
    }

    #[test]
    fn the_persisted_note_forbids_resuming_the_same_calls() {
        let note = report(Lane::Failure).persisted_note();
        assert!(note.contains("Forge stopped your previous turn"));
        assert!(note.contains("`shell` failed 4 times"));
        assert!(note.contains("Do not resume the same calls"));
    }
}
