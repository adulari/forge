//! Plain line-based presenter for non-interactive use (scripting, pipes, CI).

use std::io::{IsTerminal, Write};

use forge_types::SideEffect;
use forge_types::{
    Answer, ConfirmOutcome, Presenter, PresenterEvent, QChoice, Question, NO_ANSWER,
};

use crate::answer::resolve_answer;
use crate::render;

/// A line of plain output. On a terminal the text is re-glyphed onto the active glyph set (the
/// ASCII tier folds everything to ASCII); piped output stays byte-for-byte what it always was.
macro_rules! out {
    () => { println!() };
    ($($arg:tt)*) => { println!("{}", fold_for_terminal(&format!($($arg)*))) };
}

/// Like [`out!`] without the trailing newline.
macro_rules! out_inline {
    ($($arg:tt)*) => { print!("{}", fold_for_terminal(&format!($($arg)*))) };
}

fn fold_for_terminal(text: &str) -> String {
    if std::io::stdout().is_terminal() {
        crate::theme::glyphs::normalise(text)
    } else {
        text.to_string()
    }
}

/// Plain line-based renderer for non-interactive use.
pub struct HeadlessPresenter {
    /// When false (e.g. piped, non-tty), confirmations default to deny (safe).
    interactive: bool,
    /// Whether stdout can carry ANSI styling. Reasoning is only distinguishable from the answer
    /// when it can be dimmed, so a piped/redirected stdout drops it instead of interleaving a
    /// model's private thinking into the answer text (a bridged claude turn on a thinking model
    /// otherwise printed its whole scratchpad as the reply).
    styled: bool,
    /// A reasoning delta was the last thing written, so the answer needs a line break first.
    reasoning_open: bool,
    /// The newest accounting line, printed once when the turn ends. `Cost` now arrives after
    /// every model call (so the TUI statusline is live); a line per call would flood plain output.
    pending_cost: Option<String>,
    /// A streamed answer line is still open (no trailing newline yet), so a warning must start
    /// on a fresh line instead of being glued onto the model's text.
    mid_line: bool,
}

impl Default for HeadlessPresenter {
    fn default() -> Self {
        Self::new(std::io::stdin().is_terminal())
    }
}

impl HeadlessPresenter {
    pub fn new(interactive: bool) -> Self {
        Self {
            interactive,
            styled: std::io::stdout().is_terminal()
                && std::env::var("TERM").map_or(true, |t| t != "dumb"),
            reasoning_open: false,
            pending_cost: None,
            mid_line: false,
        }
    }

    /// Override ANSI styling (tests, and callers that know their sink).
    pub fn with_styling(mut self, styled: bool) -> Self {
        self.styled = styled;
        self
    }

    /// Close an open reasoning run so answer text starts on its own line.
    fn end_reasoning(&mut self) {
        if self.reasoning_open {
            out!();
            self.reasoning_open = false;
        }
    }
}

impl Presenter for HeadlessPresenter {
    fn consumes_suggestions(&self) -> bool {
        false
    }

    fn emit(&mut self, event: PresenterEvent) {
        match event {
            PresenterEvent::SessionStarted { id } => {
                out!("● session {id}");
            }
            PresenterEvent::Routing {
                tier,
                model,
                rationale,
                ..
            } => {
                let model = crate::display_model(&model);
                out!("⚒ mesh → [{tier}] {model}  ({rationale})");
            }
            // Interactive surfaces use this exact request boundary for their live heartbeat.
            // Headless already printed the route and streams the next provider event directly.
            PresenterEvent::ProviderRequest { .. } => {}
            // Content-free heartbeat used only by interactive progress surfaces.
            PresenterEvent::ProviderProgress => {}
            PresenterEvent::ProviderToolInput { .. } | PresenterEvent::ProviderThinking { .. } => {}
            PresenterEvent::AssistantText(text) => {
                self.end_reasoning();
                self.mid_line = false;
                out!("\n{text}");
            }
            PresenterEvent::AssistantDelta(delta) => {
                self.end_reasoning();
                if !delta.is_empty() {
                    self.mid_line = !delta.ends_with('\n');
                }
                print!("{delta}");
                let _ = std::io::stdout().flush();
            }
            PresenterEvent::Reasoning(delta) => {
                // Dim so reasoning is visually distinct from the answer; without styling it is
                // indistinguishable, so it is dropped rather than merged into the answer.
                if self.styled {
                    print!(
                        "{}",
                        crate::theme::paint(crate::theme::t().palette.fg_muted, &delta)
                    );
                    let _ = std::io::stdout().flush();
                    self.reasoning_open = true;
                }
            }
            PresenterEvent::AssistantDone => {
                self.end_reasoning();
                out!();
            }
            PresenterEvent::Steered(text) => {
                match forge_types::parse_harness_notice(&text) {
                    Some(forge_types::HarnessNotice::Job(items)) => {
                        for item in items {
                            match item {
                                forge_types::JobNotice::Dropped(n) => {
                                    out!("  ⚙ {n} earlier job notice(s) dropped");
                                }
                                forge_types::JobNotice::Exit(exit) => {
                                    out!("  ⚙ {}", exit.summary());
                                    for l in &exit.tail {
                                        out!("      {l}");
                                    }
                                }
                            }
                        }
                        return;
                    }
                    Some(notice) => {
                        if let Some(line) = notice.headline(100) {
                            out!("  {line}");
                            return;
                        }
                    }
                    None => {}
                }
                out!("  ⚡ steer: {text}");
            }
            PresenterEvent::Warning(msg) => {
                if std::mem::take(&mut self.mid_line) {
                    out!();
                }
                out!("  ⚠ {msg}");
            }
            PresenterEvent::Error(msg) => {
                // Red + distinct glyph so a hard failure can't be mistaken for the yellow warning.
                let line = fold_for_terminal(&format!("  ✖ {msg}"));
                if self.styled && std::env::var_os("NO_COLOR").is_none() {
                    eprintln!(
                        "{}",
                        crate::theme::paint(crate::theme::t().palette.err, &line)
                    );
                } else {
                    eprintln!("{line}");
                }
            }
            PresenterEvent::ModelSearch { model, retrying } => {
                // Headless has no animated indicator; a concise dim line keeps the failover
                // record. A same-model retry must not claim a switch is happening (a pin pins).
                let text = if retrying {
                    format!("  · {model} unavailable — retrying…")
                } else {
                    format!("  · {model} unavailable — finding another model…")
                };
                if self.styled {
                    out!(
                        "{}",
                        crate::theme::paint(crate::theme::t().palette.fg_muted, &text)
                    );
                } else {
                    out!("{text}");
                }
            }
            PresenterEvent::ToolStart { name, args } => {
                out!("  ↳ {name}({args})");
            }
            PresenterEvent::ToolResult {
                name, ok, summary, ..
            } => {
                let mark = if ok { "✓" } else { "✗" };
                out!("  {mark} {name}: {summary}");
            }
            // Headless prints only the summary, so a script needs the path to read the rest.
            PresenterEvent::ToolOutput { output, .. } => {
                out!("    full output: {} ({} lines)", output.path, output.lines);
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
            PresenterEvent::SubagentStart { agent, task, .. } => {
                out!("  ⤷ spawn [{agent}]: {task}");
            }
            // Live per-child deltas are for the interactive TUI row; the line-based renderer
            // stays quiet and shows the final SubagentResult.
            PresenterEvent::SubagentProgress { .. } => {}
            PresenterEvent::SubagentResult {
                agent,
                ok,
                summary,
                cost_usd,
                ..
            } => {
                let mark = if ok { "✓" } else { "✗" };
                out!("  {mark} agent [{agent}] (${cost_usd:.4}): {summary}");
            }
            PresenterEvent::Diff(diff) => {
                // Plain unified-diff text for scripting/pipes (no ANSI).
                out_inline!("{}", render::diff_to_plain(&diff));
                let _ = std::io::stdout().flush();
            }
            PresenterEvent::AssayProgress(msg) => {
                out!("  {msg}");
            }
            PresenterEvent::AssayCriticRow(row) => {
                use forge_types::AssayCriticStatus;
                let status = match &row.status {
                    AssayCriticStatus::Queued => "queued".to_string(),
                    AssayCriticStatus::Done { candidates } => {
                        let model = row.model.as_deref().unwrap_or("?");
                        format!("done ({candidates}) [{model}] ${:.4}", row.cost_usd)
                    }
                    AssayCriticStatus::Skipped { reason } => format!("skipped ({reason})"),
                };
                out!("  {} — {status}", row.lens);
            }
            PresenterEvent::AssayVerifying { candidates } => {
                out!("  ⚖ verifying {candidates} candidate(s)…");
            }
            PresenterEvent::AssayReport(report) => {
                out_inline!("{}", render::assay_report_plain(&report));
                let _ = std::io::stdout().flush();
            }
            PresenterEvent::Tasks(tasks) => {
                let done = tasks
                    .iter()
                    .filter(|t| t.status == forge_types::TodoStatus::Done)
                    .count();
                out!("  tasks ({done}/{} done):", tasks.len());
                for t in &tasks {
                    out!("    {} {}", t.status.marker(), t.title);
                }
            }
            PresenterEvent::McpStatus(servers) => {
                if servers.is_empty() {
                    out!("  no MCP servers configured");
                } else {
                    out!("  MCP servers ({} configured)", servers.len());
                    for s in &servers {
                        let detail = s
                            .detail
                            .as_deref()
                            .map(|d| format!("  {d}"))
                            .unwrap_or_default();
                        out!(
                            "    {} {} {} — {} tools · {} resources · {} prompts{detail}",
                            s.name,
                            s.status,
                            s.transport,
                            s.tools,
                            s.resources,
                            s.prompts
                        );
                    }
                }
            }
            PresenterEvent::ContextInjected {
                symbols,
                files,
                tokens,
            } => {
                out!("  ⌬ lattice → injected {symbols} symbols · {files} files (~{tokens} tok)");
            }
            PresenterEvent::AuxiliaryRequest { model, purpose } => {
                out!("  ◇ {purpose} via {model}…");
            }
            PresenterEvent::AuxiliaryProgress { .. } => {}
            PresenterEvent::ShellDiagnosis {
                command,
                diagnosis,
                fix,
            } => {
                out!("  ⚠ shell failed: {command}");
                for line in diagnosis.lines() {
                    out!("    {line}");
                }
                if let Some(cmd) = fix {
                    out!("    fix: {cmd}");
                }
            }
            PresenterEvent::BtwAnswer {
                question, answer, ..
            } => {
                out!("  ◈ btw: {question}\n{answer}");
            }
            PresenterEvent::Recap { text } => {
                out!("  ※ recap  {text}");
            }
            PresenterEvent::Progress(text) => {
                out!("  ⋯ {text}");
            }
            // Ghost-text input suggestions are a TUI-only affordance (dim placeholder + Tab
            // accept in an interactive input box); headless has no input box to show it in.
            PresenterEvent::SuggestionReady { .. } => {}
            // The final answer was already streamed via AssistantText; Done only flushes the
            // turn's final accounting line.
            PresenterEvent::Done { .. } => {
                if let Some(line) = self.pending_cost.take() {
                    out!("{line}");
                }
            }
            // Real-time quota updates/pace are for the TUI overlay/statusline; headless ignores them.
            PresenterEvent::QuotaUpdate { .. } => {}
            PresenterEvent::QuotaPace { .. } => {}
            PresenterEvent::CustomWidgetOutput { .. } => {}
            PresenterEvent::CompactionStarted { auto } => {
                out!("  ⟳ compacting{}…", if auto { " (auto)" } else { "" });
            }
            PresenterEvent::CompactionFinished { before, after } => {
                out!("  ⟳ compacted {before} → {after} messages");
            }
            PresenterEvent::PlanProposed(plan) => {
                out!("  ⬡ PLAN  {}", plan.title.trim());
                for (i, step) in plan.steps.iter().enumerate() {
                    out!("    {:>2}. {}", i + 1, step.title.trim());
                    let d = step.detail.trim();
                    if !d.is_empty() {
                        out!("        {d}");
                    }
                }
                if let Some(n) = plan
                    .notes
                    .as_deref()
                    .map(str::trim)
                    .filter(|n| !n.is_empty())
                {
                    out!("    ⚠ {n}");
                }
            }
            PresenterEvent::Temper(_) => {}
            PresenterEvent::Effort(_) => {}
            PresenterEvent::WorkflowStarted { name } => match name {
                Some(n) => out!("  ⛓ workflow '{n}' started"),
                None => out!("  ⛓ workflow started"),
            },
            PresenterEvent::WorkflowPhase { title } => out!("  ▶ phase: {title}"),
            PresenterEvent::WorkflowLog(msg) => out!("  💬 {msg}"),
            PresenterEvent::WorkflowFinished { ok, summary } => {
                let mark = if ok { "✓" } else { "⚠" };
                out!("  {mark} workflow finished: {summary}");
            }
        }
    }

    fn confirm(&mut self, tool: &str, side_effect: SideEffect) -> ConfirmOutcome {
        if !self.interactive {
            out!("  ⚠ denying {tool} ({side_effect:?}) — non-interactive session");
            return ConfirmOutcome::Deny;
        }
        out_inline!("  ⚠ allow {tool} ({side_effect:?})? [y/a=always/N] ");
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        if std::io::stdin().read_line(&mut line).is_err() {
            return ConfirmOutcome::Deny;
        }
        match line.trim() {
            "a" | "A" | "always" => ConfirmOutcome::AlwaysAllow,
            "y" | "Y" | "yes" => ConfirmOutcome::Allow,
            _ => ConfirmOutcome::Deny,
        }
    }

    fn ask(&mut self, question: &str, options: &[QChoice], allow_other: bool) -> String {
        if !self.interactive {
            return NO_ANSWER.to_string();
        }
        // Re-prompt a couple of times on invalid input, then give up gracefully.
        for _ in 0..3 {
            out!("\n❓ {question}");
            for (i, o) in options.iter().enumerate() {
                if o.description.is_empty() {
                    out!("  {}) {}", i + 1, o.label);
                } else {
                    out!("  {}) {} — {}", i + 1, o.label, o.description);
                }
            }
            if allow_other {
                out_inline!("  choose a number, or type your own answer: ");
            } else {
                out_inline!("  choose a number: ");
            }
            let _ = std::io::stdout().flush();
            let mut line = String::new();
            if std::io::stdin().read_line(&mut line).is_err() {
                return NO_ANSWER.to_string();
            }
            if let Some(ans) = resolve_answer(&line, options, allow_other) {
                return ans;
            }
        }
        NO_ANSWER.to_string()
    }

    fn ask_form(&mut self, questions: &[Question]) -> Vec<Answer> {
        if !self.interactive {
            return Vec::new();
        }
        let total = questions.len();
        questions
            .iter()
            .enumerate()
            .map(|(i, q)| {
                if total > 1 {
                    let head = if q.header.is_empty() {
                        String::new()
                    } else {
                        format!(" · {}", q.header)
                    };
                    out!("\n❓ question {} of {total}{head}", i + 1);
                }
                let selected = ask_one_headless(q);
                let note = if q.allow_note && !selected.is_empty() {
                    out_inline!("  note (Enter to skip): ");
                    let _ = std::io::stdout().flush();
                    let mut line = String::new();
                    if std::io::stdin().read_line(&mut line).is_ok() {
                        Some(line.trim().to_string()).filter(|t| !t.is_empty())
                    } else {
                        None
                    }
                } else {
                    None
                };
                let mut answer = selected;
                answer.note = note;
                answer
            })
            .collect()
    }

    fn is_attended(&self) -> bool {
        self.interactive
    }

    fn read_line(&mut self) -> Option<String> {
        if self.interactive {
            out_inline!("› ");
            let _ = std::io::stdout().flush();
        }
        let mut line = String::new();
        match std::io::stdin().read_line(&mut line) {
            Ok(0) | Err(_) => None, // EOF or read error -> end
            Ok(_) => Some(line),
        }
    }
}

/// One question on a line-oriented terminal: numbers pick options (`1,3` on a multi-select),
/// anything else is the free-text answer when allowed. Three tries, then no answer.
fn ask_one_headless(q: &Question) -> Answer {
    for _ in 0..3 {
        out!("\n❓ {}", q.text);
        for (i, o) in q.options.iter().enumerate() {
            if o.description.is_empty() {
                out!("  {}) {}", i + 1, o.label);
            } else {
                out!("  {}) {} — {}", i + 1, o.label, o.description);
            }
        }
        let how = match (q.multi, q.allow_other) {
            (true, true) => "numbers separated by commas, or type your own answer: ",
            (true, false) => "numbers separated by commas: ",
            (false, true) => "choose a number, or type your own answer: ",
            (false, false) => "choose a number: ",
        };
        out_inline!("  {how}");
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        if std::io::stdin().read_line(&mut line).is_err() {
            return Answer::default();
        }
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        let picks: Option<Vec<usize>> = t
            .split(',')
            .map(|p| p.trim().parse::<usize>().ok())
            .map(|n| n.filter(|n| *n >= 1 && *n <= q.options.len()))
            .collect();
        match picks {
            Some(ns) if !ns.is_empty() && (q.multi || ns.len() == 1) => {
                let mut seen = std::collections::BTreeSet::new();
                return Answer {
                    selected: ns
                        .into_iter()
                        .filter(|n| seen.insert(*n))
                        .map(|n| q.options[n - 1].label.clone())
                        .collect(),
                    other: None,
                    note: None,
                };
            }
            _ if q.allow_other => {
                return Answer {
                    selected: Vec::new(),
                    other: Some(t.to_string()),
                    note: None,
                };
            }
            _ => {}
        }
    }
    Answer::default()
}

#[cfg(test)]
mod ask_tests {
    use super::*;

    fn opts() -> Vec<QChoice> {
        vec![
            QChoice {
                label: "Postgres".into(),
                description: "relational".into(),
            },
            QChoice {
                label: "SQLite".into(),
                description: String::new(),
            },
        ]
    }

    #[test]
    fn a_number_picks_that_option() {
        assert_eq!(
            resolve_answer("2", &opts(), true).as_deref(),
            Some("SQLite")
        );
        assert_eq!(
            resolve_answer(" 1 ", &opts(), false).as_deref(),
            Some("Postgres")
        );
    }

    #[test]
    fn free_text_allowed_only_when_open() {
        assert_eq!(
            resolve_answer("use mysql", &opts(), true).as_deref(),
            Some("use mysql")
        );
        assert_eq!(resolve_answer("use mysql", &opts(), false), None);
    }

    #[test]
    fn out_of_range_number_is_invalid() {
        assert_eq!(resolve_answer("9", &opts(), false), None);
        // ...but a free-text fallback accepts it as text when open.
        assert_eq!(resolve_answer("9", &opts(), true).as_deref(), Some("9"));
    }

    #[test]
    fn non_interactive_headless_returns_the_sentinel() {
        let mut p = HeadlessPresenter::new(false);
        assert_eq!(p.ask("which db?", &opts(), true), NO_ANSWER);
    }

    #[test]
    fn unstyled_headless_drops_reasoning_instead_of_merging_it_into_the_answer() {
        let mut p = HeadlessPresenter::new(false).with_styling(false);
        p.emit(PresenterEvent::Reasoning("private thoughts".into()));
        assert!(
            !p.reasoning_open,
            "nothing was written, so nothing to close"
        );
    }

    #[test]
    fn styled_headless_breaks_the_line_between_reasoning_and_the_answer() {
        let mut p = HeadlessPresenter::new(false).with_styling(true);
        p.emit(PresenterEvent::Reasoning("private thoughts".into()));
        assert!(p.reasoning_open);
        p.emit(PresenterEvent::AssistantDelta("OK".into()));
        assert!(!p.reasoning_open);
    }
}
