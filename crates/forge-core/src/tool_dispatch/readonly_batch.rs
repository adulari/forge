//! Concurrent dispatch of a batch of read-only tool calls, with results appended in the
//! original order.

use super::*;

impl Session {
    /// Execute a batch of side-effect-free tool calls CONCURRENTLY, then append their results in the
    /// original order. When the model requests several independent reads/searches in one step,
    /// running them together (instead of serially) is a direct latency win — and safe because
    /// ReadOnly tools have no side effects, never prompt (permission resolves to Allow/Deny without
    /// asking), don't snapshot, and queue no hints. Only used when all calls qualify and no hooks
    /// are configured (PreToolUse/PostToolUse run on every call and must stay serial); otherwise the
    /// caller falls back to the serial [`invoke_tool`] path.
    /// Returns each call's `(name, failure_kind)` in original order so the caller can feed the
    /// failure-loop guard exactly as the serial path does — a concurrent batch that keeps failing the
    /// same way (e.g. two reads of ever-changing missing paths every step) must still be caught.
    pub(crate) async fn run_readonly_batch(
        &mut self,
        msg_id: &str,
        calls: &[forge_types::ToolCall],
    ) -> Result<Vec<(String, Option<ErrorCategory>, String)>, CoreError> {
        self.run_readonly_calls(msg_id, calls, true).await
    }

    /// [`Self::run_readonly_batch`], optionally without appending the results to the transcript:
    /// the inner calls of a `run_code` script report to the script, not to the model.
    pub(crate) async fn run_readonly_calls(
        &mut self,
        msg_id: &str,
        calls: &[forge_types::ToolCall],
        to_transcript: bool,
    ) -> Result<Vec<(String, Option<ErrorCategory>, String)>, CoreError> {
        struct Pending {
            id: String,
            name: String,
            args: serde_json::Value,
            args_json: String,
            allowed: bool,
        }
        // Phase 1 (serial): announce each call + resolve permission (pure; no prompt for ReadOnly).
        let mut pend = Vec::with_capacity(calls.len());
        for call in calls {
            let args_json = serde_json::to_string(&call.args)?;
            self.presenter.emit(PresenterEvent::ToolStart {
                name: call.name.clone(),
                args: args_json.clone(),
            });
            let allowed = matches!(
                permission::decide(
                    self.mode,
                    forge_types::SideEffect::ReadOnly,
                    &call.name,
                    &call.args,
                    &self.rules,
                ),
                PermissionDecision::Allow
            );
            pend.push(Pending {
                id: call.id.clone(),
                name: call.name.clone(),
                args: call.args.clone(),
                args_json,
                allowed,
            });
        }
        // Phase 2 (concurrent): run every allowed tool's `run()` together. Borrows `self.tools`
        // immutably for the duration of the join; no `&mut self` is touched until it completes.
        let results: Vec<(String, Option<String>, bool)> = {
            let tools = &self.tools;
            let futs = pend.iter().map(|p| async move {
                if !p.allowed {
                    return ("permission denied by policy".to_string(), None, false);
                }
                match tools.get(&p.name) {
                    Some(tool) => match tool.run_full(&p.args).await {
                        Ok(out) => (out.model, out.full, true),
                        Err(e) => (format!("error: {e}"), None, false),
                    },
                    None => (format!("error: unknown tool '{}'", p.name), None, false),
                }
            });
            let all = futures::future::join_all(futs);
            // One call owns the tail; several running at once would interleave theirs.
            if pend.len() == 1 && pend[0].allowed && pend[0].name == "shell" {
                stream_live_tail(self.presenter.as_mut(), "shell", all).await
            } else {
                all.await
            }
        };
        // Phase 3 (serial): surface + persist + append each result in the ORIGINAL order, so every
        // tool_call_id is answered in sequence. Also classify each result for the failure-loop guard.
        let mut classified = Vec::with_capacity(pend.len());
        for (p, (result, full, ok)) in pend.iter().zip(results) {
            let shown_ok = ok && !(p.name == "shell" && shell_command_did_fail(&result));
            self.emit_tool_card(&p.name, shown_ok, &result);
            self.record_test_proof(&p.name, &p.args, &result, full.as_deref());
            if ok {
                self.note_read_files(&p.name, &p.args);
            }
            let kept = self.surface_full_output(&p.name, &p.id, full.as_deref(), &result);
            let result = crate::tool_output::fit_for_model_capped(
                result,
                kept.as_deref(),
                full.is_none(),
                crate::codemode::inner_result_cap()
                    .unwrap_or(crate::tool_output::MODEL_RESULT_MAX_CHARS),
            );
            let result = if ok {
                let touched = crate::instructions::touched_paths(&p.name, &p.args, None);
                self.with_scoped_instructions(&touched, result)
            } else {
                result
            };
            self.store.record_tool_call(
                msg_id,
                &p.name,
                &p.args_json,
                &result,
                if p.allowed { "allowed" } else { "denied" },
                if ok { "ok" } else { "error" },
            )?;
            classified.push((
                p.name.clone(),
                classify_tool_failure(&result),
                result.clone(),
            ));
            if to_transcript {
                let seq = self.next_seq();
                self.store.add_message_full(
                    &self.id,
                    seq,
                    Role::Tool,
                    &result,
                    None,
                    &[],
                    Some(&p.id),
                )?;
                self.transcript.push(Message::tool_result(&p.id, result));
            }
        }
        Ok(classified)
    }
}
