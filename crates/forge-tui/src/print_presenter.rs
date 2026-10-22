//! `forge run -p`: Claude Code's print mode. stdout carries only the turn's final answer, so
//! `answer=$(forge run -p "...")` and byte-for-byte comparisons against `claude -p` work. The
//! trace (session, routing, narration, tool calls, cost) goes to stderr when a person is watching
//! it (stderr is a terminal) or `--verbose` asks for it, and never under `--quiet`; errors always
//! reach stderr.

use std::io::{IsTerminal, Write};

use forge_types::{SideEffect, StopReason};

use crate::answer::resolve_answer;
use crate::{render, ConfirmOutcome, Presenter, PresenterEvent, QChoice, NO_ANSWER};

pub struct PrintPresenter {
    out: Box<dyn Write + Send>,
    trace: Option<Box<dyn Write + Send>>,
    /// stdin is a terminal, so a confirmation can be asked (on stderr) instead of denied.
    interactive: bool,
    /// The assistant message being streamed. It becomes trace narration when a tool call follows
    /// it, and stays off stdout unless it turns out to be the final answer.
    narration: String,
    pending_cost: Option<String>,
}

impl PrintPresenter {
    /// `may_ask: false` (`--permission-mode dontAsk`) denies unapproved calls even at a terminal.
    pub fn new(verbose: bool, may_ask: bool) -> Self {
        let trace: Option<Box<dyn Write + Send>> = (verbose || std::io::stderr().is_terminal())
            .then(|| Box::new(std::io::stderr()) as Box<dyn Write + Send>);
        Self::with_writers(
            Box::new(std::io::stdout()),
            trace,
            may_ask && std::io::stdin().is_terminal(),
        )
    }

    /// `--quiet`: nothing but the answer, even at a terminal. Errors still reach stderr.
    pub fn without_trace(mut self) -> Self {
        self.trace = None;
        self
    }

    pub fn with_writers(
        out: Box<dyn Write + Send>,
        trace: Option<Box<dyn Write + Send>>,
        interactive: bool,
    ) -> Self {
        Self {
            out,
            trace,
            interactive,
            narration: String::new(),
            pending_cost: None,
        }
    }

    fn trace(&mut self, text: &str) {
        if let Some(trace) = self.trace.as_mut() {
            let _ = writeln!(trace, "{text}");
            let _ = trace.flush();
        }
    }

    /// Errors are never hidden: they go to the trace when there is one, else straight to stderr.
    fn error(&mut self, text: &str) {
        match self.trace.as_mut() {
            Some(trace) => {
                let _ = writeln!(trace, "  ✖ {text}");
                let _ = trace.flush();
            }
            None => eprintln!("error: {text}"),
        }
    }

    fn flush_narration(&mut self) {
        let text = std::mem::take(&mut self.narration);
        let text = text.trim();
        if !text.is_empty() {
            self.trace(text);
        }
    }

    /// A prompt for the person at the terminal; stdout stays reserved for the answer.
    fn prompt(&mut self, text: &str) -> Option<String> {
        eprint!("{text}");
        let _ = std::io::stderr().flush();
        let mut line = String::new();
        match std::io::stdin().read_line(&mut line) {
            Ok(0) | Err(_) => None,
            Ok(_) => Some(line),
        }
    }
}

impl Presenter for PrintPresenter {
    fn emit(&mut self, event: PresenterEvent) {
        match event {
            PresenterEvent::SessionStarted { id } => self.trace(&format!("● session {id}")),
            PresenterEvent::Routing {
                tier,
                model,
                rationale,
                ..
            } => {
                let model = crate::display_model(&model);
                self.trace(&format!("⚒ mesh → [{tier}] {model}  ({rationale})"));
            }
            PresenterEvent::AssistantText(text) | PresenterEvent::AssistantDelta(text) => {
                self.narration.push_str(&text);
            }
            PresenterEvent::ToolStart { name, args } => {
                self.flush_narration();
                self.trace(&format!("  ↳ {name}({args})"));
            }
            PresenterEvent::ToolResult {
                name, ok, summary, ..
            } => {
                let mark = if ok { "✓" } else { "✗" };
                self.trace(&format!("  {mark} {name}: {summary}"));
            }
            PresenterEvent::ToolOutput { output, .. } => self.trace(&format!(
                "    full output: {} ({} lines)",
                output.path, output.lines
            )),
            PresenterEvent::Warning(msg) => self.trace(&format!("  ⚠ {msg}")),
            PresenterEvent::Error(msg) => self.error(&msg),
            PresenterEvent::ModelSearch { model, retrying } => {
                let what = if retrying {
                    "retrying…"
                } else {
                    "finding another model…"
                };
                self.trace(&format!("  · {model} unavailable — {what}"));
            }
            PresenterEvent::SubagentStart { agent, task, .. } => {
                self.trace(&format!("  ⤷ spawn [{agent}]: {task}"));
            }
            PresenterEvent::SubagentResult {
                agent,
                ok,
                summary,
                cost_usd,
                ..
            } => {
                let mark = if ok { "✓" } else { "✗" };
                self.trace(&format!(
                    "  {mark} agent [{agent}] (${cost_usd:.4}): {summary}"
                ));
            }
            PresenterEvent::Diff(diff) => {
                let text = render::diff_to_plain(&diff);
                self.trace(text.trim_end());
            }
            PresenterEvent::Tasks(tasks) => {
                let done = tasks
                    .iter()
                    .filter(|t| t.status == forge_types::TodoStatus::Done)
                    .count();
                self.trace(&format!("  tasks ({done}/{} done)", tasks.len()));
            }
            PresenterEvent::CompactionFinished { before, after } => {
                self.trace(&format!("  ⟳ compacted {before} → {after} messages"));
            }
            PresenterEvent::PlanProposed(plan) => {
                self.trace(&format!("  ⬡ PLAN  {}", plan.title.trim()));
                for (i, step) in plan.steps.iter().enumerate() {
                    self.trace(&format!("    {:>2}. {}", i + 1, step.title.trim()));
                }
            }
            PresenterEvent::Cost {
                session_total_usd,
                session_in,
                session_out,
                ..
            } => {
                self.pending_cost = Some(format!(
                    "  $ session total: ${session_total_usd:.4} · ↑{session_in} ↓{session_out} tok"
                ));
            }
            PresenterEvent::Done {
                final_text,
                stop_reason,
            } => {
                // The streamed message was the answer itself: it goes to stdout, not the trace.
                self.narration.clear();
                if let Some(line) = self.pending_cost.take() {
                    self.trace(&line);
                }
                // A failed turn's text is the error the caller reports on stderr.
                if stop_reason.is_success() || stop_reason == StopReason::DaemonShutdown {
                    let answer = final_text.trim_end();
                    if !answer.is_empty() {
                        let _ = writeln!(self.out, "{answer}");
                        let _ = self.out.flush();
                    }
                }
            }
            _ => {}
        }
    }

    fn confirm(&mut self, tool: &str, side_effect: SideEffect) -> ConfirmOutcome {
        if !self.interactive {
            self.trace(&format!(
                "  ⚠ denying {tool} ({side_effect:?}) — non-interactive session"
            ));
            return ConfirmOutcome::Deny;
        }
        let question = format!("  ⚠ allow {tool} ({side_effect:?})? [y/a=always/N] ");
        match self.prompt(&question).as_deref().map(str::trim) {
            Some("a" | "A" | "always") => ConfirmOutcome::AlwaysAllow,
            Some("y" | "Y" | "yes") => ConfirmOutcome::Allow,
            _ => ConfirmOutcome::Deny,
        }
    }

    fn ask(&mut self, question: &str, options: &[QChoice], allow_other: bool) -> String {
        if !self.interactive {
            return NO_ANSWER.to_string();
        }
        let mut text = format!("\n❓ {question}\n");
        for (i, o) in options.iter().enumerate() {
            text.push_str(&format!("  {}) {}\n", i + 1, o.label));
        }
        text.push_str(if allow_other {
            "  choose a number, or type your own answer: "
        } else {
            "  choose a number: "
        });
        for _ in 0..3 {
            let Some(line) = self.prompt(&text) else {
                break;
            };
            if let Some(answer) = resolve_answer(&line, options, allow_other) {
                return answer;
            }
        }
        NO_ANSWER.to_string()
    }

    fn read_line(&mut self) -> Option<String> {
        None
    }

    /// Print mode is a one-shot: nobody is there to answer "paused — send `continue`".
    fn is_attended(&self) -> bool {
        false
    }

    fn consumes_recap(&self) -> bool {
        false
    }

    fn consumes_suggestions(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct Buf(Arc<Mutex<Vec<u8>>>);
    impl Write for Buf {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    impl Buf {
        fn text(&self) -> String {
            String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
        }
    }

    fn turn(p: &mut PrintPresenter, stop_reason: StopReason) {
        p.emit(PresenterEvent::SessionStarted { id: "s-1".into() });
        p.emit(PresenterEvent::AssistantDelta("Let me look.".into()));
        p.emit(PresenterEvent::ToolStart {
            name: "read_file".into(),
            args: "{}".into(),
        });
        p.emit(PresenterEvent::ToolResult {
            name: "read_file".into(),
            ok: true,
            summary: "ok".into(),
            detail: None,
        });
        p.emit(PresenterEvent::AssistantDelta("All ".into()));
        p.emit(PresenterEvent::AssistantDelta("good.".into()));
        p.emit(PresenterEvent::Done {
            final_text: "All good.".into(),
            stop_reason,
        });
    }

    #[test]
    fn stdout_holds_only_the_final_answer() {
        let (out, trace) = (Buf::default(), Buf::default());
        let mut p = PrintPresenter::with_writers(
            Box::new(out.clone()),
            Some(Box::new(trace.clone())),
            false,
        );
        turn(&mut p, StopReason::FinalAnswer);
        assert_eq!(out.text(), "All good.\n");
        let trace = trace.text();
        assert!(trace.contains("● session s-1"), "{trace}");
        assert!(
            trace.contains("Let me look."),
            "narration is trace: {trace}"
        );
        assert!(trace.contains("↳ read_file"), "{trace}");
        assert!(
            !trace.contains("All good."),
            "the answer is not repeated: {trace}"
        );
    }

    #[test]
    fn quiet_drops_the_trace_a_terminal_would_have_shown() {
        let (out, trace) = (Buf::default(), Buf::default());
        let mut p = PrintPresenter::with_writers(
            Box::new(out.clone()),
            Some(Box::new(trace.clone())),
            false,
        )
        .without_trace();
        turn(&mut p, StopReason::FinalAnswer);
        assert_eq!(out.text(), "All good.\n");
        assert_eq!(trace.text(), "");
    }

    #[test]
    fn without_a_trace_nothing_but_the_answer_is_written() {
        let out = Buf::default();
        let mut p = PrintPresenter::with_writers(Box::new(out.clone()), None, false);
        turn(&mut p, StopReason::FinalAnswer);
        assert_eq!(out.text(), "All good.\n");
    }

    #[test]
    fn a_failed_turn_leaves_stdout_empty() {
        let out = Buf::default();
        let mut p = PrintPresenter::with_writers(Box::new(out.clone()), None, false);
        turn(&mut p, StopReason::TasksUnfinished);
        assert_eq!(out.text(), "");
    }

    #[test]
    fn print_mode_is_unattended_and_skips_side_calls() {
        let mut p = PrintPresenter::with_writers(Box::new(Buf::default()), None, false);
        assert!(!p.is_attended());
        assert!(!p.consumes_recap() && !p.consumes_suggestions());
        assert_eq!(p.confirm("shell", SideEffect::Shell), ConfirmOutcome::Deny);
        assert_eq!(p.ask("which?", &[], true), NO_ANSWER);
    }
}
