//! Last-resort answer for a turn a loop guard ended with nothing to show.
//!
//! The narration-stall, doom-loop and failure-loop guards (and an empty-response dead end) stop a
//! turn mid-investigation. They used to leave `final_text` empty, so a headless caller (`forge run
//! --output-format json`, MCP `forge_chat`, a subagent) received `""` as the result of a turn that
//! had in fact gathered evidence — observed live when a plan-mode model gathered a directory
//! listing over twelve steps, was halted for repeating itself, and reported nothing. One
//! tool-less completion over the gathered transcript turns that evidence into an answer; if even
//! that fails, the last thing the model said, or a plain statement of what happened, is better
//! than silence.

use super::*;

/// Prefix of every placeholder answer. The turn classifier treats text that starts with it as "no
/// answer" so a turn that only got a placeholder is still reported as `NoOutput`, not a success.
pub(crate) const NO_ANSWER_NOTICE: &str = "The turn ended before the model wrote an answer";

pub(crate) fn is_no_answer_notice(text: &str) -> bool {
    text.starts_with(NO_ANSWER_NOTICE)
}

const FINALIZE_PROMPT: &str = "Stop investigating. Using ONLY what the tool results above \
already show, write your final answer to the user's request now. Do not call tools and do not \
announce further steps. If something is still unknown, say what, in one line.";

impl Session {
    /// Best-effort non-empty answer for a turn that ended without one. Never returns blank text.
    pub(crate) async fn answer_for_halted_turn(&mut self, model: &str, halted: bool) -> String {
        if let Some(text) = self.finalize_from_evidence(model).await {
            return text;
        }
        let why = if halted {
            " (stopped by a loop guard)"
        } else {
            ""
        };
        let last = self
            .last_substantive_assistant_text()
            .map(|t| format!(" Its last message was: {t}"))
            .unwrap_or_default();
        format!(
            "{NO_ANSWER_NOTICE}{why}, and a final summary could not be produced.{last} Its tool \
             results are in the transcript; send `continue` to let it try again."
        )
    }

    async fn finalize_from_evidence(&mut self, model: &str) -> Option<String> {
        let mut msgs = self.transcript_for(model);
        msgs.push(Message::user(FINALIZE_PROMPT));
        let opts = CompletionOptions {
            effort: Some(EffortLevel::Low),
            prompt_cache_key: Some(format!("{}:finalize", self.id)),
            ..Self::auxiliary_completion_options(&self.id, "finalize")
        };
        let idle = std::time::Duration::from_secs(self.config.mesh.stream_idle_timeout_secs);
        let activity = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
        let act = std::sync::Arc::clone(&activity);
        let mut sink = |_: StreamEvent| {
            act.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        };
        let fut = self
            .provider
            .complete_with(model, &msgs, &[], &opts, &mut sink);
        let mut resp = stream_with_idle_timeout(fut, &activity, None, idle)
            .await
            .ok()?;
        if resp.content.trim().is_empty() || !resp.tool_calls.is_empty() {
            return None;
        }
        resp.usage.cost_usd = self.pricing.cost_for_usage(model, &resp.usage);
        let seq = self.next_seq();
        if let Ok(msg_id) = self.store.add_message_full(
            &self.id,
            seq,
            Role::Assistant,
            &resp.content,
            Some(model),
            &[],
            None,
        ) {
            let _ = self
                .store
                .record_usage(&self.id, &msg_id, &resp.usage, Some(model));
        }
        self.presenter
            .emit(PresenterEvent::AssistantDelta(resp.content.clone()));
        Some(resp.content)
    }

    /// The model's latest non-blank prose since the user's last message (narration like "I'll run
    /// wc -l" is thin, but it is the model's own account of where it got to).
    fn last_substantive_assistant_text(&self) -> Option<String> {
        self.transcript
            .iter()
            .rev()
            .take_while(|m| m.role != Role::User)
            .find(|m| m.role == Role::Assistant && !m.content.trim().is_empty())
            .map(|m| m.content.trim().to_string())
    }
}

impl Session {
    /// End-of-loop bookkeeping: a step-ceiling abort, publishing a hard stop's reason into the
    /// history, and a last-resort answer for a halted turn that did work but wrote nothing.
    /// `ceiling` is `(hit_step_cap, unattended, step_limit)`, `flags` is
    /// `(halted_by_loop_guard, hard_guard_abort)`.
    pub(crate) async fn close_out_turn_text(
        &mut self,
        mut final_text: String,
        active_model: &str,
        ceiling: (bool, bool, Option<usize>),
        flags: (bool, bool),
        tools_ran: u64,
    ) -> Result<(String, bool, bool), CoreError> {
        let (hit_step_cap, unattended, step_limit) = ceiling;
        let (mut halted_by_loop_guard, mut hard_guard_abort) = flags;
        if let (true, true, Some(limit)) = (hit_step_cap, unattended, step_limit) {
            final_text = self.abort_for_step_ceiling(limit);
            halted_by_loop_guard = true;
            hard_guard_abort = true;
        }

        // A hard stop's reason (spend or token cap, run budget, headless stall, unfinished-task
        // abort) used to travel only in the turn's result and a live event, so a session read
        // later ended on a tool call or a tool result with no word of why. Keep it in the history.
        if hard_guard_abort && !final_text.trim().is_empty() {
            self.publish_terminal_answer(&final_text.clone())?;
        }

        // A loop guard (or an unrecoverable empty reply) can end a turn that did real work with
        // nothing to say; never hand that back as the result.
        if final_text.trim().is_empty()
            && !hard_guard_abort
            && !hit_step_cap
            && !self.past_turn_deadline()
            && tools_ran > 0
        {
            final_text = self
                .answer_for_halted_turn(active_model, halted_by_loop_guard)
                .await;
            self.publish_terminal_answer(&final_text.clone())?;
        }

        Ok((final_text, halted_by_loop_guard, hard_guard_abort))
    }
}
