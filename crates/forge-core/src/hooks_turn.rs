//! Turn-level hooks (Claude-Code parity, docs/features/hooks.md): `StopFailure` when a turn ends
//! on a provider error instead of an answer, the delivery of finished `async` hooks at the start
//! of a turn, and `PostToolBatch` after each step's tool calls.

use forge_config::HookEvent;
use forge_provider::ProviderError;
use forge_types::{LoopOutcome, TaskTier};

use crate::hooks_events::{run_event_hooks, EventMatch};
use crate::{CoreError, PresenterEvent, Session};

tokio::task_local! {
    /// Set while a turn runs, so a turn started from inside one (plan approval re-runs the build
    /// turn) neither re-delivers async hook output nor reports one failure twice.
    static IN_TURN: ();
}

impl Session {
    /// Like [`Session::run_turn`], but first prepends `guidance` (an invoked command's or
    /// skill's methodology) as persisted system messages, and biases routing with an optional
    /// `tier_override` (the command/skill `tier:` hint). `run_turn(p)` is exactly
    /// `run_turn_with(p, &[], None)` — the agent loop, tools, permission broker, pricing and
    /// persistence are otherwise unchanged. Output of `async` hooks that finished since the last
    /// turn joins the context first; a turn that fails on a provider error fires `StopFailure`.
    pub async fn run_turn_with(
        &mut self,
        prompt: &str,
        guidance: &[String],
        tier_override: Option<TaskTier>,
    ) -> Result<LoopOutcome, CoreError> {
        // Boxed: the turn body is the largest future in the crate, and every caller (bench, serve,
        // the TUI) would otherwise inline its layout and overflow rustc's query depth.
        if IN_TURN.try_with(|_| ()).is_ok() {
            return Box::pin(self.run_turn_body(prompt, guidance, tier_override)).await;
        }
        for text in crate::hooks_async::take(&self.id) {
            self.inject_hook_context(&text);
        }
        forge_reporting::event("turn_start", None);
        let started = std::time::Instant::now();
        let result = IN_TURN
            .scope(
                (),
                Box::pin(self.run_turn_body(prompt, guidance, tier_override)),
            )
            .await;
        forge_reporting::event(
            if result.is_ok() {
                "turn_end"
            } else {
                "turn_error"
            },
            Some(u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)),
        );
        self.close_shell_diagnoses();
        if let Err(error) = &result {
            crate::error_report::report_turn_error(error);
            self.fire_stop_failure(error).await;
        }
        result
    }

    /// `StopFailure` (observe-only: Claude Code ignores its output and exit code).
    async fn fire_stop_failure(&mut self, error: &CoreError) {
        let Some(kind) = stop_failure_kind(error) else {
            return;
        };
        if !self.has_hooks_for(HookEvent::StopFailure) {
            return;
        }
        let details = error.to_string();
        run_event_hooks(
            &self.config.hooks,
            HookEvent::StopFailure,
            &self.id,
            EventMatch::Value(kind),
            serde_json::json!({
                "error": kind,
                "error_details": details,
                "last_assistant_message": format!("API Error: {details}"),
                "cwd": self.workspace.display(),
            }),
        )
        .await;
    }

    /// `PostToolBatch` once the step's calls all resolved, before the next model request.
    /// `additionalContext` joins the conversation; a block (`decision: "block"`, `continue:
    /// false`, exit 2) ends the turn, and its reason stays in the conversation. Returns whether
    /// the turn must end.
    pub(crate) async fn fire_post_tool_batch(
        &mut self,
        batch: &[(&forge_types::ToolCall, String)],
    ) -> bool {
        if batch.is_empty() || !self.has_hooks_for(HookEvent::PostToolBatch) {
            return false;
        }
        let tool_calls: Vec<serde_json::Value> = batch
            .iter()
            .map(|(call, result)| {
                let (name, input) = crate::hooks_cc::forge_tool_to_cc(&call.name, &call.args);
                serde_json::json!({
                    "tool_name": name, "tool_input": input, "tool_use_id": call.id,
                    "tool_response": result,
                })
            })
            .collect();
        let runner = self.llm_hook_runner().await;
        let out = crate::hooks_llm::with_runner(
            runner,
            run_event_hooks(
                &self.config.hooks,
                HookEvent::PostToolBatch,
                &self.id,
                EventMatch::Any,
                serde_json::json!({ "tool_calls": tool_calls, "cwd": self.workspace.display() }),
            ),
        )
        .await;
        for note in out.notes {
            self.presenter
                .emit(PresenterEvent::Warning(format!("⎇ hook: {note}")));
        }
        for failure in out.failures {
            self.presenter
                .emit(PresenterEvent::Warning(format!("⎇ hook error: {failure}")));
        }
        for text in out.context {
            let _ = self.push_note(forge_types::MessageKind::Hint, text);
        }
        let Some(reason) = out.blocked.or(out.halt) else {
            return false;
        };
        self.presenter.emit(PresenterEvent::Warning(format!(
            "a PostToolBatch hook ended the turn: {reason}"
        )));
        let _ = self.push_note(
            forge_types::MessageKind::Hint,
            format!("[PostToolBatch hook] {reason}"),
        );
        true
    }
}

/// Claude Code's `StopFailure` error type for a failed turn, or `None` when it did not fail on the
/// provider.
pub(crate) fn stop_failure_kind(error: &CoreError) -> Option<&'static str> {
    match error {
        CoreError::Provider(e) => Some(provider_failure_kind(e)),
        CoreError::NoHealthyModel {
            reason, last_error, ..
        } => Some(match *reason {
            "rate-limited" => "rate_limit",
            "unavailable" => overloaded_or_server(last_error),
            "auth failed" => "authentication_failed",
            "request error" => "invalid_request",
            "not available on this account" => "model_not_found",
            r if r.starts_with("unsupported") => billing_or_invalid(last_error),
            _ => "unknown",
        }),
        _ => None,
    }
}

fn provider_failure_kind(e: &ProviderError) -> &'static str {
    match e {
        ProviderError::RateLimited { .. } => "rate_limit",
        ProviderError::Unavailable(message) => overloaded_or_server(message),
        ProviderError::Auth(_) => "authentication_failed",
        ProviderError::Capability(message) => billing_or_invalid(message),
        ProviderError::NoModelAccess(_) => "model_not_found",
        ProviderError::Request(_) => "invalid_request",
    }
}

fn overloaded_or_server(message: &str) -> &'static str {
    let m = message.to_ascii_lowercase();
    if m.contains("overloaded") || m.contains("529") {
        "overloaded"
    } else {
        "server_error"
    }
}

fn billing_or_invalid(message: &str) -> &'static str {
    let m = message.to_ascii_lowercase();
    if ["402", "credit", "billing", "afford", "payment"]
        .iter()
        .any(|k| m.contains(k))
    {
        "billing_error"
    } else {
        "invalid_request"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_failures_map_to_claude_codes_error_types() {
        let kind = |e: ProviderError| stop_failure_kind(&CoreError::Provider(e));
        assert_eq!(
            kind(ProviderError::RateLimited {
                message: "429".into(),
                retry_after: None
            }),
            Some("rate_limit")
        );
        assert_eq!(
            kind(ProviderError::Auth("401".into())),
            Some("authentication_failed")
        );
        assert_eq!(
            kind(ProviderError::Unavailable("529 overloaded".into())),
            Some("overloaded")
        );
        assert_eq!(
            kind(ProviderError::Capability(
                "HTTP 402: requires more credits".into()
            )),
            Some("billing_error")
        );
        assert_eq!(
            stop_failure_kind(&CoreError::NoHealthyModel {
                verdict: "v".into(),
                model: "m".into(),
                reason: "not available on this account",
                last_error: "x".into(),
            }),
            Some("model_not_found")
        );
        assert_eq!(stop_failure_kind(&CoreError::Internal("x".into())), None);
    }
}
