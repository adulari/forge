//! The shell error interceptor's model call (shell-error-interceptor.md), kept off the turn.
//!
//! The call used to run inline: a failed command's result waited for a trivial-tier completion
//! before the model saw it. On a DeepSeek pin that was 2.7-3.1 s in 29 of 52 bench tasks, mostly
//! for a line the model did not need: a test run's verdict restated, `git grep` finding nothing,
//! the session's own model re-reading the output it was about to read anyway. Now:
//!
//! - a failure whose output is already the answer gets no call: a check's verdict (tests, build,
//!   lint, type check), a failed assertion, a search or comparison that matched nothing;
//! - an unattended session whose side calls run on its own pinned model gets no call either: the
//!   only reader would be that model, which reads the failure itself on its next step;
//! - everything else runs on a detached task and the tool result returns at once. The diagnosis
//!   reaches the model only when it is ready by the next request (arriving later it would describe
//!   a command the model has already acted on) and the screen whenever it lands: at the next step
//!   while the turn runs, straight away on a channel-backed surface once the turn is over.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use tokio::sync::mpsc;

use super::*;
use crate::compaction_policy::record_model_failure_in;
use crate::post_turn_calls::SideModelInputs;
use crate::spend_guard::priced;

/// The routing prompt the diagnosis is classified under (it only steers the tier hints).
const DIAGNOSE_ROUTE: &str = "explain a shell error";

/// One finished diagnosis on its way back to the session.
pub(crate) struct Diagnosis {
    /// The step boundary that was next when the command failed: the hint is for that one only.
    boundary: u64,
    event: PresenterEvent,
    hint: String,
}

/// Where detached diagnoses land, and what the session needs to deliver them.
pub(crate) struct ShellDiagnoses {
    tx: mpsc::UnboundedSender<Diagnosis>,
    rx: mpsc::UnboundedReceiver<Diagnosis>,
    boundary: u64,
    /// The turn is running and drains the inbox at its next step. Otherwise a diagnosis that lands
    /// goes straight to a channel-backed surface.
    turn_open: Arc<AtomicBool>,
}

impl Default for ShellDiagnoses {
    fn default() -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self {
            tx,
            rx,
            boundary: 0,
            turn_open: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl ShellDiagnoses {
    /// The transcript was rewritten: a diagnosis still in flight describes a removed command.
    pub(crate) fn forget(&mut self) {
        self.boundary += 1;
        while self.rx.try_recv().is_ok() {}
    }
}

/// Whether the failed command's own output already says what a diagnosis would: a verification
/// run that reported its verdict, a failed assertion, or a search/comparison that found nothing
/// (exit 1 is that tool's "no", not an error).
pub(crate) fn diagnosis_adds_nothing(command: &str, result: &str) -> bool {
    if crate::completion::verification_family(command).is_some() {
        return true;
    }
    let lower = result.to_ascii_lowercase();
    if lower.contains("assertionerror")
        || lower.contains("assertion failed")
        || (lower.contains("assertion `") && lower.contains("failed"))
    {
        return true;
    }
    exit_code(result) == Some(1) && last_stage_answers_no(command)
}

fn exit_code(result: &str) -> Option<i32> {
    let header = result.lines().next()?;
    let rest = header.strip_prefix("shell: exit ")?;
    rest.split_whitespace().next()?.parse().ok()
}

/// The program of the command's last stage is one whose exit 1 means "no match" or "differs".
fn last_stage_answers_no(command: &str) -> bool {
    let command = command.replace(">&", ">").replace("&>", ">");
    let last = command
        .rsplit(['|', ';', '&', '\n'])
        .map(str::trim)
        .find(|stage| !stage.is_empty())
        .unwrap_or("");
    let mut words = last
        .split_whitespace()
        .skip_while(|word| word.contains('=') || *word == "!" || *word == "command");
    match words.next() {
        Some("grep" | "egrep" | "fgrep" | "rg" | "ag" | "ack" | "diff" | "cmp" | "test" | "[") => {
            true
        }
        Some("git") => matches!(words.next(), Some("grep" | "diff")),
        _ => false,
    }
}

/// The diagnosis completion, with everything it needs owned so it can run off the turn.
struct DiagnoseCall {
    provider: Arc<dyn Provider>,
    store: Arc<Store>,
    pricing: Pricing,
    session_id: String,
    cooldown: std::time::Duration,
    idle: std::time::Duration,
    command: String,
    result: String,
}

impl DiagnoseCall {
    /// The cause and the optional fix command, or `None` when the model failed, timed out or said
    /// nothing. Bounded by [`SHELL_DIAGNOSE_MAX_SECS`] so a stalled provider does not leave the
    /// task behind for the rest of the session.
    async fn run(self, model: &str) -> Option<(String, Option<String>)> {
        let messages = [
            Message::system(SHELL_DIAGNOSE_SYSTEM),
            Message::user(format!(
                "Command:\n{}\n\nResult:\n{}",
                self.command, self.result
            )),
        ];
        let options = Session::auxiliary_completion_options(&self.session_id, "shell-diagnose");
        let activity = AtomicU64::new(0);
        let mut sink = |event: StreamEvent| {
            if let StreamEvent::Text(delta) | StreamEvent::Reasoning(delta) = event {
                if !delta.is_empty() {
                    activity.fetch_add(1, Ordering::Relaxed);
                }
            }
        };
        let completion = self
            .provider
            .complete_with(model, &messages, &[], &options, &mut sink);
        let response = tokio::time::timeout(
            std::time::Duration::from_secs(SHELL_DIAGNOSE_MAX_SECS),
            stream_with_idle_timeout(completion, &activity, None, self.idle),
        )
        .await;
        let response = match response {
            Ok(Ok(response)) => response,
            Ok(Err(error)) => {
                record_model_failure_in(&self.store, model, &error, self.cooldown);
                return None;
            }
            Err(_) => {
                tracing::debug!("shell diagnosis timed out after {SHELL_DIAGNOSE_MAX_SECS}s");
                return None;
            }
        };
        if let Err(error) = self.store.record_side_call_usage_for(
            &self.session_id,
            "shell/diagnose",
            Some(model),
            &priced(&self.pricing, model, &response.usage),
        ) {
            tracing::warn!(error = %error, "failed to persist shell diagnosis usage");
        }
        parse_diagnosis(&response.content)
    }
}

/// Cause on the first non-empty line, an optional `FIX: <cmd>` line after it.
fn parse_diagnosis(content: &str) -> Option<(String, Option<String>)> {
    let mut cause = String::new();
    let mut fix = None;
    for line in content.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if let Some(cmd) = line.strip_prefix("FIX: ") {
            fix = Some(cmd.trim().to_string());
        } else if cause.is_empty() {
            cause = line.to_string();
        }
    }
    if cause.is_empty() {
        cause = content.trim().to_string();
    }
    (!cause.is_empty()).then_some((cause, fix))
}

impl Session {
    /// On a failed shell command, explain the likely cause and a fix (see the module docs for
    /// when no model call is made and how the answer is delivered). Returns at once.
    pub(crate) async fn diagnose_shell_error(&mut self, command: &str, result: &str) {
        // Common patterns need no model call, and their answer is ready now.
        if let Some(cached) = pattern_diagnose(result) {
            self.pending_hints
                .push(format!("[shell diagnosis] {cached}"));
            self.presenter.emit(PresenterEvent::ShellDiagnosis {
                command: command.to_string(),
                diagnosis: cached.to_string(),
                fix: None,
            });
            return;
        }
        if diagnosis_adds_nothing(command, result) || self.diagnosis_reaches_no_one() {
            return;
        }
        crate::turn_trace::mark("shell diagnose: detached");
        let configured_idle = self.config.mesh.stream_idle_timeout_secs;
        let call = DiagnoseCall {
            provider: Arc::clone(&self.provider),
            store: Arc::clone(&self.store),
            pricing: self.pricing.clone(),
            session_id: self.id.clone(),
            cooldown: std::time::Duration::from_secs(self.config.mesh.failover_cooldown_secs),
            idle: std::time::Duration::from_secs(if configured_idle == 0 {
                SHELL_DIAGNOSE_MAX_SECS
            } else {
                configured_idle.min(SHELL_DIAGNOSE_MAX_SECS)
            }),
            command: command.to_string(),
            result: result.to_string(),
        };
        let inputs: SideModelInputs = self.side_model_inputs();
        let inbox = self.shell_diagnoses.tx.clone();
        let boundary = self.shell_diagnoses.boundary;
        let turn_open = Arc::clone(&self.shell_diagnoses.turn_open);
        let sink = self.presenter.recap_sink();
        tokio::spawn(async move {
            let Some(model) = inputs.decide(DIAGNOSE_ROUTE).await else {
                return;
            };
            let command = call.command.clone();
            let Some((cause, fix)) = call.run(&model).await else {
                return;
            };
            let hint = match &fix {
                Some(f) => format!("[shell diagnosis] {cause}  fix: {f}"),
                None => format!("[shell diagnosis] {cause}"),
            };
            let event = PresenterEvent::ShellDiagnosis {
                command,
                diagnosis: cause,
                fix,
            };
            match sink {
                Some(mut sink) if !turn_open.load(Ordering::Acquire) => sink.emit(event),
                _ => {
                    let _ = inbox.send(Diagnosis {
                        boundary,
                        event,
                        hint,
                    });
                }
            }
        });
    }

    /// An unattended session whose side calls stay on its own pinned model: the diagnosis would
    /// be that same model reading the same output, for no one but itself.
    fn diagnosis_reaches_no_one(&self) -> bool {
        // The surface, not the permission mode: a bypass-mode TUI still has someone watching.
        !self.presenter.is_attended()
            && !self.config.mesh.pin_failover
            && (self
                .pinned_model
                .as_ref()
                .is_some_and(|pin| !pin.is_empty())
                || self.router.pin().is_some())
    }

    /// At a step boundary: show the diagnoses that landed and hand the model the ones made for
    /// this very request. Never waits.
    pub(crate) fn deliver_shell_diagnoses(&mut self) {
        self.shell_diagnoses
            .turn_open
            .store(true, Ordering::Release);
        let current = self.shell_diagnoses.boundary;
        self.shell_diagnoses.boundary += 1;
        while let Ok(diagnosis) = self.shell_diagnoses.rx.try_recv() {
            self.presenter.emit(diagnosis.event);
            if diagnosis.boundary == current {
                let _ = self.push_note(forge_types::MessageKind::Hint, diagnosis.hint);
            }
        }
    }

    /// The turn is over: diagnoses still on their way go straight to a channel-backed surface
    /// when they land. One that landed after the last step reaches that surface now; a plain
    /// surface has already printed the turn's result, so it is not given one after it.
    pub(crate) fn close_shell_diagnoses(&mut self) {
        self.shell_diagnoses
            .turn_open
            .store(false, Ordering::Release);
        self.shell_diagnoses.boundary += 1;
        let mut sink = self.presenter.recap_sink();
        while let Ok(diagnosis) = self.shell_diagnoses.rx.try_recv() {
            if let Some(sink) = sink.as_mut() {
                sink.emit(diagnosis.event);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_verdict_or_a_no_match_needs_no_explanation() {
        let fail = "shell: exit 1 in 40ms\n";
        assert!(diagnosis_adds_nothing(
            "python -m pytest -q",
            "shell: exit 1 in 2s\n2 failed"
        ));
        assert!(diagnosis_adds_nothing(
            "cargo build",
            "shell: exit 101 in 9s"
        ));
        assert!(diagnosis_adds_nothing(
            "python3 check.py",
            "shell: exit 1 in 30ms\nTraceback\nAssertionError: 3 != 4"
        ));
        assert!(diagnosis_adds_nothing(
            "git grep -n Customer -- store api",
            fail
        ));
        assert!(diagnosis_adds_nothing("cd src && grep -rn foo .", fail));
        assert!(diagnosis_adds_nothing("diff -u a.txt b.txt", fail));
        assert!(diagnosis_adds_nothing("LC_ALL=C rg needle", fail));
        assert!(diagnosis_adds_nothing("grep -c TODO src/lib.rs 2>&1", fail));
    }

    #[test]
    fn a_broken_command_still_gets_one() {
        assert!(!diagnosis_adds_nothing(
            "python3 -c 'import throttle'",
            "shell: exit 1 in 30ms\nModuleNotFoundError: No module named 'throttle'"
        ));
        assert!(!diagnosis_adds_nothing(
            "grep -rn foo missing/",
            "shell: exit 2 in 5ms\ngrep: missing/: No such file"
        ));
        assert!(!diagnosis_adds_nothing(
            "grep foo a.txt | xargs rm",
            "shell: exit 1 in 5ms"
        ));
        assert!(!diagnosis_adds_nothing(
            "git add store/user.go && git commit -m rename",
            "shell: exit 128 in 9ms\nfatal: pathspec 'store/user.go' did not match any files"
        ));
    }

    #[test]
    fn the_reply_is_split_into_cause_and_fix() {
        assert_eq!(
            parse_diagnosis("The file is missing.\nFIX: touch a.txt\n"),
            Some(("The file is missing.".into(), Some("touch a.txt".into())))
        );
        assert_eq!(
            parse_diagnosis("  only a cause  "),
            Some(("only a cause".into(), None))
        );
        assert_eq!(parse_diagnosis("   "), None);
    }
}
