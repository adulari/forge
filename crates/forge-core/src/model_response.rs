//! Persistence and accounting for each completed model response.

use super::*;

impl Session {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn record_model_response(
        &mut self,
        step: usize,
        active_model: &str,
        decision: Option<&forge_mesh::RoutingDecision>,
        resp: &forge_provider::ModelResponse,
        tools_before: u64,
        tools_ran: &std::sync::Arc<std::sync::atomic::AtomicU64>,
        bridge_input_accum: &mut u64,
        empty_nudges: &mut usize,
    ) -> Result<(String, bool), CoreError> {
        let provisional_completion = !resp.wants_tools() && !resp.content.trim().is_empty();
        let mut assistant_message =
            Message::assistant_tool_calls(&resp.content, resp.tool_calls.clone());
        if provisional_completion {
            assistant_message = assistant_message.llm_only();
        }
        self.transcript.push(assistant_message);

        let seq = self.next_seq();
        let msg_id = if provisional_completion {
            self.store.add_llm_only_message_full(
                &self.id,
                seq,
                Role::Assistant,
                &resp.content,
                Some(active_model),
                &resp.tool_calls,
                None,
            )?
        } else {
            self.store.add_message_full(
                &self.id,
                seq,
                Role::Assistant,
                &resp.content,
                Some(active_model),
                &resp.tool_calls,
                None,
            )?
        };
        // A successful Codex OAuth response carries a backend-authoritative plan header.
        // Persist its short-lived observation even for a model-pinned turn (which has no
        // auto-routing decision) so the next process's mesh inspector sees the same account.
        if active_model.starts_with("codex-oauth::") {
            if let Some(plan) = forge_provider::fresh_live_codex_plan() {
                let _ = self.store.record_subscription_plan("codex-oauth", &plan);
            }
        }
        // Step-0 routing record and quota-hint persistence are only meaningful for the primary
        // turn (when we have a decision). The autofix re-run skips both.
        if let Some(d) = decision {
            if step == 0 {
                self.store
                    .record_routing(&msg_id, d.tier, active_model, &d.rationale)?;
            }
            // Quota-aware routing (L3): if a CLI bridge reported its subscription window this
            // turn, persist it so the next route() can demote/skip a near-limit subscription.
            for hint in &resp.quotas {
                let _ = self.store.record_quota(hint);
                // Push to the TUI so the /usage overlay updates in real-time.
                if let Some(f) = hint.fraction_used {
                    self.presenter
                        .emit(forge_types::PresenterEvent::QuotaUpdate {
                            provider: hint.provider.clone(),
                            window: hint.window.clone(),
                            fraction: f,
                        });
                }
                self.emit_quota_pace(hint);
            }
        }
        self.store.record_usage(&self.id, &msg_id, &resp.usage)?;
        // Accumulate this bridge completion's input toward the per-turn ceiling (wave 5, fix 1).
        if forge_provider::is_cli_bridge(active_model) {
            *bridge_input_accum = bridge_input_accum.saturating_add(resp.usage.input_tokens);
        }

        let bridge_tool_progress = forge_provider::is_cli_bridge(active_model)
            && tools_ran.load(std::sync::atomic::Ordering::Relaxed) > tools_before;

        if resp.wants_tools() || bridge_tool_progress {
            *empty_nudges = 0;
        }

        Ok((msg_id, bridge_tool_progress))
    }
}
