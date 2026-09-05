//! Plain line-based presenter for non-interactive use (scripting, pipes, CI).

use std::io::{IsTerminal, Write};

use forge_types::SideEffect;
use forge_types::{ConfirmOutcome, Presenter, PresenterEvent, QChoice, NO_ANSWER};

use crate::answer::resolve_answer;
use crate::render;

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
            styled: std::io::stdout().is_terminal(),
            reasoning_open: false,
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
            println!();
            self.reasoning_open = false;
        }
    }
}

impl Presenter for HeadlessPresenter {
    fn emit(&mut self, event: PresenterEvent) {
        match event {
            PresenterEvent::SessionStarted { id } => {
                println!("● session {id}");
            }
            PresenterEvent::Routing {
                tier,
                model,
                rationale,
                ..
            } => {
                let model = crate::display_model(&model);
                println!("⚒ mesh → [{tier}] {model}  ({rationale})");
            }
            // Interactive surfaces use this exact request boundary for their live heartbeat.
            // Headless already printed the route and streams the next provider event directly.
            PresenterEvent::ProviderRequest { .. } => {}
            // Content-free heartbeat used only by interactive progress surfaces.
            PresenterEvent::ProviderProgress => {}
            PresenterEvent::AssistantText(text) => {
                self.end_reasoning();
                println!("\n{text}");
            }
            PresenterEvent::AssistantDelta(delta) => {
                self.end_reasoning();
                print!("{delta}");
                let _ = std::io::stdout().flush();
            }
            PresenterEvent::Reasoning(delta) => {
                // Dim so reasoning is visually distinct from the answer; without styling it is
                // indistinguishable, so it is dropped rather than merged into the answer.
                if self.styled {
                    print!("\x1b[2m{delta}\x1b[0m");
                    let _ = std::io::stdout().flush();
                    self.reasoning_open = true;
                }
            }
            PresenterEvent::AssistantDone => {
                self.end_reasoning();
                println!();
            }
            PresenterEvent::Warning(msg) => {
                println!("  ⚠ {msg}");
            }
            PresenterEvent::Error(msg) => {
                // Red + distinct glyph so a hard failure can't be mistaken for the yellow ⚠.
                eprintln!("\x1b[31m  ✖ {msg}\x1b[0m");
            }
            PresenterEvent::ModelSearch { model, retrying } => {
                // Headless has no animated indicator; a concise dim line keeps the failover
                // record. A same-model retry must not claim a switch is happening (a pin pins).
                if retrying {
                    println!("\x1b[2m  · {model} unavailable — retrying…\x1b[0m");
                } else {
                    println!("\x1b[2m  · {model} unavailable — finding another model…\x1b[0m");
                }
            }
            PresenterEvent::ToolStart { name, args } => {
                println!("  ↳ {name}({args})");
            }
            PresenterEvent::ToolResult { name, ok, summary } => {
                let mark = if ok { "✓" } else { "✗" };
                println!("  {mark} {name}: {summary}");
            }
            PresenterEvent::Cost {
                session_total_usd,
                session_in,
                session_out,
                ..
            } => {
                println!(
                    "  $ session total: ${session_total_usd:.4} · ↑{session_in} ↓{session_out} tok"
                );
            }
            PresenterEvent::SubagentStart { agent, task, .. } => {
                println!("  ⤷ spawn [{agent}]: {task}");
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
                println!("  {mark} agent [{agent}] (${cost_usd:.4}): {summary}");
            }
            PresenterEvent::Diff(diff) => {
                // Plain unified-diff text for scripting/pipes (no ANSI).
                print!("{}", render::diff_to_plain(&diff));
                let _ = std::io::stdout().flush();
            }
            PresenterEvent::AssayProgress(msg) => {
                println!("  {msg}");
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
                println!("  {} — {status}", row.lens);
            }
            PresenterEvent::AssayVerifying { candidates } => {
                println!("  ⚖ verifying {candidates} candidate(s)…");
            }
            PresenterEvent::AssayReport(report) => {
                print!("{}", render::assay_report_plain(&report));
                let _ = std::io::stdout().flush();
            }
            PresenterEvent::Tasks(tasks) => {
                let done = tasks
                    .iter()
                    .filter(|t| t.status == forge_types::TodoStatus::Done)
                    .count();
                println!("  tasks ({done}/{} done):", tasks.len());
                for t in &tasks {
                    println!("    {} {}", t.status.marker(), t.title);
                }
            }
            PresenterEvent::McpStatus(servers) => {
                if servers.is_empty() {
                    println!("  no MCP servers configured");
                } else {
                    println!("  MCP servers ({} configured)", servers.len());
                    for s in &servers {
                        let detail = s
                            .detail
                            .as_deref()
                            .map(|d| format!("  {d}"))
                            .unwrap_or_default();
                        println!(
                            "    {} {} {} — {} tools · {} resources · {} prompts{detail}",
                            s.name, s.status, s.transport, s.tools, s.resources, s.prompts
                        );
                    }
                }
            }
            PresenterEvent::ContextInjected {
                symbols,
                files,
                tokens,
            } => {
                println!(
                    "  ⌬ lattice → injected {symbols} symbols · {files} files (~{tokens} tok)"
                );
            }
            PresenterEvent::AuxiliaryRequest { model, purpose } => {
                println!("  ◇ {purpose} via {model}…");
            }
            PresenterEvent::AuxiliaryProgress { .. } => {}
            PresenterEvent::ShellDiagnosis {
                command,
                diagnosis,
                fix,
            } => {
                println!("  ⚠ shell failed: {command}");
                for line in diagnosis.lines() {
                    println!("    {line}");
                }
                if let Some(cmd) = fix {
                    println!("    fix: {cmd}");
                }
            }
            PresenterEvent::BtwAnswer {
                question, answer, ..
            } => {
                println!("  ◈ btw: {question}\n{answer}");
            }
            PresenterEvent::Recap { text } => {
                println!("  ※ recap  {text}");
            }
            // Ghost-text input suggestions are a TUI-only affordance (dim placeholder + Tab
            // accept in an interactive input box); headless has no input box to show it in.
            PresenterEvent::SuggestionReady { .. } => {}
            // The final answer was already streamed via AssistantText; Done is a
            // lifecycle marker, so the headless renderer needs no extra output here.
            PresenterEvent::Done { .. } => {}
            // Real-time quota updates/pace are for the TUI overlay/statusline; headless ignores them.
            PresenterEvent::QuotaUpdate { .. } => {}
            PresenterEvent::QuotaPace { .. } => {}
            PresenterEvent::CustomWidgetOutput { .. } => {}
            PresenterEvent::CompactionStarted { auto } => {
                println!("  ⟳ compacting{}…", if auto { " (auto)" } else { "" });
            }
            PresenterEvent::CompactionFinished { before, after } => {
                println!("  ⟳ compacted {before} → {after} messages");
            }
            PresenterEvent::PlanProposed(plan) => {
                println!("  ⬡ PLAN  {}", plan.title.trim());
                for (i, step) in plan.steps.iter().enumerate() {
                    println!("    {:>2}. {}", i + 1, step.title.trim());
                    let d = step.detail.trim();
                    if !d.is_empty() {
                        println!("        {d}");
                    }
                }
                if let Some(n) = plan
                    .notes
                    .as_deref()
                    .map(str::trim)
                    .filter(|n| !n.is_empty())
                {
                    println!("    ⚠ {n}");
                }
            }
            PresenterEvent::Temper(_) => {}
            PresenterEvent::Effort(_) => {}
            PresenterEvent::WorkflowStarted { name } => match name {
                Some(n) => println!("  ⛓ workflow '{n}' started"),
                None => println!("  ⛓ workflow started"),
            },
            PresenterEvent::WorkflowPhase { title } => println!("  ▶ phase: {title}"),
            PresenterEvent::WorkflowLog(msg) => println!("  💬 {msg}"),
            PresenterEvent::WorkflowFinished { ok, summary } => {
                let mark = if ok { "✓" } else { "⚠" };
                println!("  {mark} workflow finished: {summary}");
            }
        }
    }

    fn confirm(&mut self, tool: &str, side_effect: SideEffect) -> ConfirmOutcome {
        if !self.interactive {
            println!("  ⚠ denying {tool} ({side_effect:?}) — non-interactive session");
            return ConfirmOutcome::Deny;
        }
        print!("  ⚠ allow {tool} ({side_effect:?})? [y/a=always/N] ");
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
            println!("\n❓ {question}");
            for (i, o) in options.iter().enumerate() {
                if o.description.is_empty() {
                    println!("  {}) {}", i + 1, o.label);
                } else {
                    println!("  {}) {} — {}", i + 1, o.label, o.description);
                }
            }
            if allow_other {
                print!("  choose a number, or type your own answer: ");
            } else {
                print!("  choose a number: ");
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

    fn is_attended(&self) -> bool {
        self.interactive
    }

    fn read_line(&mut self) -> Option<String> {
        if self.interactive {
            print!("› ");
            let _ = std::io::stdout().flush();
        }
        let mut line = String::new();
        match std::io::stdin().read_line(&mut line) {
            Ok(0) | Err(_) => None, // EOF or read error -> end
            Ok(_) => Some(line),
        }
    }
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
