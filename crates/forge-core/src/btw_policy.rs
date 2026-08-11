//! `/btw <question>` (alias `/side`) — an inline side question that never enters the session
//! transcript (docs/features/side-questions.md).
//!
//! This mirrors the "side call" shape already used by `auxiliary_policy.rs`
//! (recap/suggest/memory/shell-diagnose) and `compaction_policy.rs::compact`: route one cheap
//! trivial-tier model call and hand the provider's raw messages straight to
//! `self.provider.complete_with`, never touching `self.transcript` and never calling
//! `self.store.add_message` / `record_side_call_usage`. Unlike those callers, `/btw` doesn't even
//! anchor a usage/cost row in the message table — the answer exists only for the duration of this
//! call and the `PresenterEvent::BtwAnswer` it emits.
//!
//! Deliberately stateless: each `/btw` call is independent. Forge does not keep a running
//! side-conversation the way prime-agent's `/btw` does — see docs/features/side-questions.md for
//! why that's an intentional simplification, not an oversight. Tests live alongside the other
//! `Session::start`-based tests in `lib.rs`'s `mod tests` (same convention as `compact`).

use super::*;

impl Session {
    const BTW_SYSTEM: &'static str = "You are answering a quick side question asked out-of-band \
via /btw during a coding session. Answer directly and concisely. This exchange is NOT part of the \
main conversation, will not be remembered, and does not see any prior turn — treat the question as \
the entire context you have.";

    /// Answer a `/btw`/`/side` question with one best-effort trivial-tier model call. Never fails
    /// the caller: on budget exhaustion, no available model, or a provider error, it emits a
    /// `PresenterEvent::Warning` instead of a `BtwAnswer` and returns.
    pub async fn ask_btw(&mut self, question: &str) {
        let question = question.trim();
        if question.is_empty() {
            self.presenter.emit(PresenterEvent::Warning(
                "usage: /btw <question>".to_string(),
            ));
            return;
        }
        let budget = BudgetState {
            spent_today_usd: self.store.spend_today_usd().unwrap_or(0.0),
            daily_cap_usd: self.config.mesh.daily_budget_usd,
            spent_week_usd: self.store.spend_this_week_usd().unwrap_or(0.0),
            weekly_cap_usd: self.config.mesh.weekly_budget_usd,
            spent_month_usd: self.store.spend_this_month_usd().unwrap_or(0.0),
            monthly_cap_usd: self.config.mesh.monthly_cap_usd,
            warn_fraction: self.config.mesh.warn_threshold,
            min_context_tokens: None,
        };
        if budget.status() == BudgetStatus::Exhausted {
            self.presenter.emit(PresenterEvent::Warning(
                "/btw skipped — budget exhausted".to_string(),
            ));
            return;
        }
        let readiness = self.provider_readiness();
        let health = readiness.health;
        let quota = readiness.quota;
        let decision = self
            .router
            .route_hinted(
                question,
                false,
                budget,
                &health,
                &quota,
                Some(TaskTier::Trivial),
                self.pinned_effort,
                &self.project,
            )
            .await;
        let Some(model) = self.post_turn_auxiliary_model(&decision) else {
            self.presenter.emit(PresenterEvent::Warning(
                "/btw skipped — no non-bridge model available".to_string(),
            ));
            return;
        };
        let messages = [
            Message::system(Self::BTW_SYSTEM),
            Message::user(question.to_string()),
        ];
        let mut on_event = |_: StreamEvent| {};
        let completion_opts = Self::auxiliary_completion_options(&self.id, "btw");
        match self
            .provider
            .complete_with(&model, &messages, &[], &completion_opts, &mut on_event)
            .await
        {
            Ok(r) => {
                self.presenter.emit(PresenterEvent::BtwAnswer {
                    question: question.to_string(),
                    answer: r.content,
                    model,
                    cost_usd: Some(r.usage.cost_usd),
                });
            }
            Err(e) => {
                self.presenter
                    .emit(PresenterEvent::Warning(format!("/btw failed: {e}")));
            }
        }
    }
}
