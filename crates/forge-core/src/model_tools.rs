//! Direct model tool execution and loop-guard policy.

use super::*;

impl Session {
    /// Run one model response's tool calls through the permission broker and record what each
    /// returned. Returns whether a `PostToolBatch` hook ended the turn. Judging whether the turn
    /// is getting anywhere is the progress ledger's job, after the step (`progress_supervisor`).
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn execute_model_tool_step(
        &mut self,
        msg_id: &str,
        calls: &[forge_types::ToolCall],
        tools_ran: &std::sync::Arc<std::sync::atomic::AtomicU64>,
        inspect_ran: &std::sync::Arc<std::sync::atomic::AtomicU64>,
        mutations_ran: &std::sync::Arc<std::sync::atomic::AtomicU64>,
        verification_ledger: &std::sync::Arc<std::sync::Mutex<VerificationLedger>>,
        observations: &mut Vec<crate::loop_progress::CallObservation>,
    ) -> Result<bool, CoreError> {
        crate::turn_trace::mark("tools start");
        // A `tool_call{name, arguments}` IS its inner call for every counter below: the phantom-edit
        // gate (`mutations_ran`), the verification ledger, the progress observations, the
        // env-fight cap and batch eligibility. Ids are kept, so the tool results still pair with
        // the assistant message the model sent.
        let resolved: Vec<forge_types::ToolCall> = calls
            .iter()
            .map(|call| self.resolve_model_call(call))
            .collect();
        let calls = resolved.as_slice();

        // Count the tools the DIRECT path is about to run, so the completion-verification gate's
        // progress + inspection signals work for direct models. The stream sink only increments
        // these for tools the PROVIDER surfaces as `ToolStarted` events — which the bridge does
        // (its tool loop runs inside one `complete()`), but a direct genai provider does NOT: it
        // returns tool calls in `calls` and the loop executes them here. Without this,
        // `inspect_ran` stays 0 on the direct path and the gate can't tell an inspection from a
        // bare "done" claim. Bridge turns return an empty `tool_calls` (their tools ran inside the
        // subprocess), so this adds nothing for them — no double counting. `update_tasks`/
        // `present_plan` are bookkeeping, not inspections (same rule as the stream sink).
        let mut ledgers = crate::tool_outcome::TurnLedgers {
            tools_ran,
            inspect_ran,
            mutations_ran,
            verification_ledger,
            observations,
        };
        for call in calls {
            ledgers.count_started(&call.name);
        }

        // Fast path: when the model batched several independent side-effect-free calls (and no
        // tool hooks are configured), run them CONCURRENTLY instead of one-at-a-time — a direct
        // latency win on multi-file reads/searches. Mixed or tool-hook-bearing batches take the
        // serial path below unchanged. Only Pre/PostToolUse hooks run per call; requiring NO hooks
        // at all disabled this path for anyone with just a session-start or prompt hook.
        let tool_hooks = self.config.hooks.iter().any(|h| {
            matches!(
                h.event,
                forge_config::HookEvent::PreToolUse
                    | forge_config::HookEvent::PostToolUse
                    | forge_config::HookEvent::PostToolUseFailure
            )
        });
        // `PostToolBatch` sees every call of the step with the result the model reads.
        let batch_hooks = self.has_hooks_for(forge_config::HookEvent::PostToolBatch);
        let mut batch: Vec<(&forge_types::ToolCall, String)> = Vec::new();
        let concurrent_batch =
            calls.len() >= 2 && !tool_hooks && calls.iter().all(|c| self.is_concurrent_readonly(c));
        if concurrent_batch {
            let classified = self.run_readonly_batch(msg_id, calls).await?;
            // A read-only shell call that failed gets the same diagnosis as on the serial path;
            // its hint is delivered with the batch's other hints below.
            if self.config.shell.explain_errors {
                for (call, (_, _, result)) in calls.iter().zip(&classified) {
                    if call.name == "shell" && shell_command_did_fail(result) {
                        if let Some((command, output)) =
                            crate::shell_batch::failing_command(&call.args, result)
                        {
                            self.diagnose_shell_error(&command, &output).await;
                        }
                    }
                }
            }
            for (call, (_, kind, result)) in calls.iter().zip(classified) {
                if batch_hooks {
                    batch.push((call, result.clone()));
                }
                let verification = completion::classify_tool(&call.name, &call.args.to_string());
                let flipped = ledgers.verification_ledger.lock().unwrap().observe_result(
                    verification,
                    kind.is_none(),
                    &result,
                );
                ledgers.observations.push(
                    crate::loop_progress::CallObservation::for_call(
                        call,
                        kind.is_none(),
                        false,
                        &result,
                    )
                    .with_check(verification.check_family(), flipped),
                );
            }
            // Deliver any queued system hints (the serial path does this per call): a hint left
            // undelivered here would reach the model a step late, if at all.
            let hints: Vec<String> = std::mem::take(&mut self.pending_hints);
            for hint in hints {
                let _ = self.push_note(forge_types::MessageKind::Hint, hint);
            }
        } else {
            // Execute each requested tool through the permission broker, serially.
            for call in calls {
                let result = if call.name == crate::codemode::RUN_CODE_TOOL
                    && self.config.codemode.enabled
                {
                    // Boxed: the script pump nests `invoke_tool` one level deeper, which pushed the
                    // turn future past rustc's layout depth limit in the CLI's bench driver.
                    Box::pin(self.run_code(msg_id, call, &mut ledgers)).await?
                } else {
                    self.invoke_tool(msg_id, call).await?
                };
                self.observe_tool_outcome(call, &result, &mut ledgers);
                if batch_hooks {
                    batch.push((call, result.clone()));
                }
                let seq = self.next_seq();
                self.store.add_message_full(
                    &self.id,
                    seq,
                    Role::Tool,
                    &result,
                    None,
                    &[],
                    Some(&call.id),
                )?;
                self.transcript.push(Message::tool_result(&call.id, result));
                // Drain any system hints queued by side-call diagnostics (e.g. shell error
                // interceptor) so the model sees them after the failing tool result.
                let hints: Vec<String> = std::mem::take(&mut self.pending_hints);
                for hint in hints {
                    let _ = self.push_note(forge_types::MessageKind::Hint, hint);
                }
            }
        }

        crate::turn_trace::mark("tools done");
        if self.fire_post_tool_batch(&batch).await {
            return Ok(true);
        }

        Ok(false)
    }
}
