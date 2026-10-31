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
        if self.config.harness.stop_retries {
            STOP_RETRY_MAX
        } else {
            2
        }
    }

    /// A direct model wrote a tool call as TEXT and nothing ran: ask it to make the call, bounded.
    /// `true` means the turn should continue; at the bound the markup is stripped and the reply
    /// finishes the turn with a loud warning.
    pub(crate) fn repair_text_tool_call(
        &mut self,
        resp: &mut forge_provider::ModelResponse,
        nudges: &mut usize,
    ) -> bool {
        // Honest-failure guard: a direct model wrote a tool call as TEXT (e.g.
        // `<invoke>`/`default_api:` markup) instead of invoking it, and neither the
        // provider nor the text-recovery pass turned it into a real call — so NOTHING
        // ran. Accepting this as the final answer is how a turn "succeeds" while having
        // merged no PR and pushed no tag. Detect it and nudge the model to actually
        // call the tool (bounded); never silently accept narrated tool calls.
        if forge_provider::looks_like_unexecuted_tool_call(&resp.content) {
            let max_nudges = self.toolcall_repair_max();
            if *nudges < max_nudges {
                *nudges += 1;
                self.tools_advertised_once = true;
                self.presenter.emit(crate::PresenterEvent::Warning(format!(
                    "model wrote a tool call as text instead of invoking it — nothing ran; asking it to retry ({}/{max_nudges})",
                    *nudges
                )));
                let nudge = "Your last message contained a tool call written as TEXT \
                             (`<invoke …>`, a DeepSeek DSML block or `default_api:` \
                             syntax), or a block that was cut off or malformed. That \
                             tool DID NOT run — text is not a tool call. Make the call \
                             through the function-calling interface instead. Do not paste \
                             tool-call markup into your message.";
                let _ = self.push_note(forge_types::MessageKind::Continue, nudge);
                return true;
            }
            // Retries exhausted: do NOT pretend it worked. Surface it loudly so the user
            // knows the turn's actions never executed, then end (can't loop forever).
            self.presenter.emit(crate::PresenterEvent::Warning(
                "model kept emitting tool calls as text that never executed — the turn did NOT complete its actions"
                    .to_string(),
            ));
            resp.content = forge_provider::strip_tool_call_markup(&resp.content);
        }
        false
    }

    /// `[harness] thinking_loop_breaker`: push the interrupt when `text` restates an earlier
    /// tool-free reply of this stretch. Returns `true` when the turn should continue.
    pub(crate) fn thinking_loop_nudge(&mut self, text: &str, tracker: &mut ThinkingLoop) -> bool {
        if !self.config.harness.thinking_loop_breaker || !tracker.repeats(text) {
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
        if !self.config.harness.repeat_interrupt {
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
