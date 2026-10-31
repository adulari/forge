//! `[harness]` loop and stop behaviours borrowed from oh-my-pi and Factory Droid, each behind its
//! own flag (default off) so a benchmark round can A/B them: `completion_contract`,
//! `stop_retries`, `thinking_loop_breaker`, `repeat_interrupt`. A flag that is on wins over a
//! `minimal` harness profile; the profile only silences the advisory nudges that have no flag.

use crate::stall_guard::{opening_words, similar};

/// Retries of an announced-but-undone reply, an empty reply or a text-only tool call.
pub(crate) const STOP_RETRY_MAX: usize = 3;

pub(crate) const UNEXPECTED_STOP_NUDGE: &str =
    "Call the next concrete tool now. Do not repeat prior analysis.";

pub(crate) const EMPTY_STOP_NUDGE: &str = "Stopped without actionable output; the task is \
incomplete. Call the next concrete tool now, or give the final answer.";

pub(crate) const THINKING_LOOP_NUDGE: &str = "Act, don't re-plan: make the next tool call.";

/// Consecutive identical calls with identical results before the interrupt.
pub(crate) const REPEAT_INTERRUPT_AT: u32 = 5;

/// Appended to the end of the base system prompt: static text, so the cached prefix stays stable.
pub(crate) const COMPLETION_CONTRACT: &str = "\n\n<critical>\n\
- Do not yield while actionable work remains. Each response must advance the task; a complete \
deliverable is the only stopping condition.\n\
- Act instead of asking when the tools or repo can answer.\n\
- Non-trivial work needs a smoke run before you yield: run the changed path and observe the \
result. Passing tests alone are not proof.\n\
- Bug fix: reproduce the failure first, confirm it is gone after.\n\
- No stubs, placeholders or \"MVP\" versions, and no silently reduced scope.\n\
- Never mention token or time budgets.\n\
</critical>";

pub(crate) fn repeat_interrupt_text(label: &str) -> String {
    format!(
        "You have made the same call {REPEAT_INTERRUPT_AT} times in a row and got the same result \
         each time: {label}. Do not call it with these arguments again; change approach."
    )
}

/// Openings of the tool-free replies since the model last ran a tool.
#[derive(Debug, Default)]
pub(crate) struct ThinkingLoop {
    openings: Vec<Vec<String>>,
    fired: usize,
}

/// Interrupts per stretch of tool-free replies.
const THINKING_LOOP_MAX: usize = 2;

impl ThinkingLoop {
    /// Record a tool-free reply; `true` when it restates an earlier one from this stretch and the
    /// interrupt budget is not spent.
    pub(crate) fn repeats(&mut self, text: &str) -> bool {
        let words = opening_words(text);
        let repeated = self.openings.iter().any(|prev| similar(prev, &words));
        self.openings.push(words);
        if self.openings.len() > 4 {
            self.openings.remove(0);
        }
        if repeated && self.fired < THINKING_LOOP_MAX {
            self.fired += 1;
            return true;
        }
        false
    }

    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }
}

impl crate::Session {
    /// Re-issue requests for a tool call written as text.
    pub(crate) fn toolcall_repair_max(&self) -> usize {
        if self.harness_flags.stop_retries {
            STOP_RETRY_MAX
        } else {
            2
        }
    }

    /// `[harness] thinking_loop_breaker`: push the interrupt when `text` restates an earlier
    /// tool-free reply of this stretch. Returns `true` when the turn should continue.
    ///
    /// Left alone for a tool call written as text (it has its own repair note) and for CLI-bridge
    /// turns (their cost ceiling and anti-spiral checks decide whether to re-drive): a generic
    /// "act now" nudge would pre-empt both and add re-drives they never count.
    pub(crate) fn thinking_loop_nudge(
        &mut self,
        text: &str,
        tracker: &mut ThinkingLoop,
        model: &str,
    ) -> bool {
        if !self.harness_flags.thinking_loop_breaker
            || forge_provider::is_cli_bridge(model)
            || forge_provider::looks_like_unexecuted_tool_call(text)
            || !tracker.repeats(text)
        {
            return false;
        }
        self.presenter.emit(crate::PresenterEvent::Warning(
            "model is restating its plan without acting — telling it to act".to_string(),
        ));
        let _ = self.push_nudge(THINKING_LOOP_NUDGE);
        true
    }

    /// `[harness] repeat_interrupt`: after the ledger counted [`REPEAT_INTERRUPT_AT`] identical
    /// calls with identical results, name the call to the model. Repeats that return new results
    /// never reach the count, so polling is unaffected.
    pub(crate) fn repeat_interrupt(&mut self) {
        if !self.harness_flags.repeat_interrupt {
            return;
        }
        if let Some(label) = self.ledger.repeat_label(REPEAT_INTERRUPT_AT) {
            self.presenter.emit(crate::PresenterEvent::Warning(format!(
                "same call repeated {REPEAT_INTERRUPT_AT} times with the same result: {label}"
            )));
            let _ = self.push_note(
                forge_types::MessageKind::LoopGuard,
                repeat_interrupt_text(&label),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contract_is_short_and_free_of_budget_pressure() {
        let words = COMPLETION_CONTRACT.split_whitespace().count();
        assert!(words < 120, "{words} words");
        let lower = COMPLETION_CONTRACT.to_lowercase();
        assert!(!lower.contains("efficient"));
        assert!(lower.contains("smoke run") && lower.contains("reproduce"));
    }

    #[test]
    fn thinking_loop_fires_on_a_restated_plan_only_twice() {
        let plan = "I will first inspect the parser module and then rewrite the tokenizer";
        let mut t = ThinkingLoop::default();
        assert!(!t.repeats(plan));
        assert!(t.repeats(plan));
        assert!(t.repeats(plan));
        assert!(!t.repeats(plan), "budget spent");
        t.reset();
        assert!(!t.repeats(plan));
    }

    #[test]
    fn different_replies_do_not_trip_the_thinking_loop() {
        let mut t = ThinkingLoop::default();
        assert!(!t.repeats("I will inspect the parser module before touching anything else"));
        assert!(!t.repeats("The build failed because the linker could not find libssl anywhere"));
    }
}
