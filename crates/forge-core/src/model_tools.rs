//! Direct model tool execution and loop-guard policy.

use super::*;

impl Session {
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn execute_model_tool_step(
        &mut self,
        msg_id: &str,
        calls: &[forge_types::ToolCall],
        tools_ran: &std::sync::Arc<std::sync::atomic::AtomicU64>,
        inspect_ran: &std::sync::Arc<std::sync::atomic::AtomicU64>,
        verification_ledger: &std::sync::Arc<std::sync::Mutex<VerificationLedger>>,
        last_tool_sig: &mut Option<u64>,
        repeat_count: &mut usize,
        recent_sigs: &mut std::collections::VecDeque<u64>,
        doom_nudged: &mut bool,
        failure_counts: &mut std::collections::HashMap<(String, ErrorCategory), usize>,
        failure_nudged: &mut bool,
    ) -> Result<bool, CoreError> {
        // Doom-loop guard: if the model emits the exact same tool call(s) several steps running,
        // it's stuck (re-reading the same file, retrying an identical failing edit). Identical
        // args yield identical results, so halt with a clear message instead of burning the
        // remaining step budget + tokens.
        const DOOM_LOOP_THRESHOLD: usize = 3;
        // Sliding-window size for the oscillation guard. 6 holds three full A,B cycles, so an
        // A,B,A,B,A,B alternation surfaces the same signature THRESHOLD× and trips the guard,
        // while leaving room for legitimate progress (distinct calls don't accumulate).
        const DOOM_OSC_WINDOW: usize = 6;
        let sig = tool_batch_signature(calls);
        if *last_tool_sig == Some(sig) {
            *repeat_count += 1;
        } else {
            *repeat_count = 0;
            *last_tool_sig = Some(sig);
        }
        // Oscillation count: how many of the last DOOM_OSC_WINDOW steps had THIS signature.
        // Catches the non-consecutive loop the `repeat_count` reset blinds us to.
        recent_sigs.push_back(sig);
        if recent_sigs.len() > DOOM_OSC_WINDOW {
            recent_sigs.pop_front();
        }
        let osc_count = recent_sigs.iter().filter(|&&s| s == sig).count();
        // Break-out reset: clear the one-shot `doom_nudged` latch when the model changes course,
        // so a *later* genuine loop in the same turn earns its own nudge-before-halt cycle
        // instead of being hard-halted off a stale latch. `osc_count == 1` (this signature is
        // alone in the recent window) is the signal. Do NOT also wipe the window here: on a
        // strict A,B,A,B alternation every step's signature is "new" to a freshly-cleared window,
        // so clearing pinned `osc_count` at 1 forever and the guard NEVER fired — the model ran
        // to the step cap instead of halting (regression the doom_loop test now covers). The
        // window is bounded and slides on its own (`pop_front` above), so a broken-out model's
        // stale loop signatures age out naturally, while a true A,B,A,B spiral accumulates to
        // `DOOM_LOOP_THRESHOLD`.
        if osc_count == 1 {
            *doom_nudged = false;
        }
        // Distinguish the two loop shapes so the warning isn't misleading: a true A,A,A repeat
        // vs an A,B,A,B oscillation (where the model did NOT repeat the *same* call back-to-back).
        let is_oscillation =
            osc_count >= DOOM_LOOP_THRESHOLD && *repeat_count + 1 < DOOM_LOOP_THRESHOLD;
        if *repeat_count + 1 >= DOOM_LOOP_THRESHOLD || osc_count >= DOOM_LOOP_THRESHOLD {
            if !*doom_nudged {
                // First time: don't kill the turn. Tell it the loop won't make progress and to
                // switch approach — a weaker model usually breaks out of the rut. Queue the nudge
                // so it lands AFTER this step's tool results (valid message ordering); fall
                // through to execute, then re-check next step.
                *doom_nudged = true;
                self.presenter.emit(PresenterEvent::Warning(
                    if is_oscillation {
                        "model is alternating between the same tool calls in a loop (A→B→A \
                     pattern) — nudging it to break out before stopping"
                    } else {
                        "model repeated the same tool call — nudging it to change approach \
                     before stopping"
                    }
                    .to_string(),
                ));
                self.pending_hints.push(
                    "You've now cycled through the same tool calls several times — the results \
                 will not change. Stop repeating this pattern and take a DIFFERENT approach \
                 (another tool, different arguments, or a different file). If the task is \
                 genuinely complete, say so plainly or mark it Done with update_tasks. Do \
                 not issue that same cycle of calls again."
                        .to_string(),
                );
            } else {
                // Still looping after the nudge → truly stuck; halt with a clear message.
                self.presenter.emit(PresenterEvent::Error(
                    if is_oscillation {
                        "the model kept alternating between the same tool calls after a nudge — \
                     stopping to avoid a loop"
                    } else {
                        "the model kept repeating the same tool call after a nudge — stopping \
                     to avoid a loop"
                    }
                    .to_string(),
                ));
                return Ok(true);
            }
        }

        // Count the tools the DIRECT path is about to run, so the completion-verification gate's
        // progress + inspection signals work for direct models. The stream sink only increments
        // these for tools the PROVIDER surfaces as `ToolStarted` events — which the bridge does
        // (its tool loop runs inside one `complete()`), but a direct genai provider does NOT: it
        // returns tool calls in `calls` and the loop executes them here. Without this,
        // `inspect_ran` stays 0 on the direct path and the gate can't tell an inspection from a
        // bare "done" claim. Bridge turns return an empty `tool_calls` (their tools ran inside the
        // subprocess), so this adds nothing for them — no double counting. `update_tasks`/
        // `present_plan` are bookkeeping, not inspections (same rule as the stream sink).
        for call in calls {
            tools_ran.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if !call.name.ends_with("update_tasks") && !call.name.ends_with("present_plan") {
                inspect_ran.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        }

        // Fast path: when the model batched several independent side-effect-free calls (and no
        // hooks are configured), run them CONCURRENTLY instead of one-at-a-time — a direct
        // latency win on multi-file reads/searches. Mixed or hook-bearing batches take the
        // serial path below unchanged.
        let concurrent_batch = calls.len() >= 2
            && self.config.hooks.is_empty()
            && calls.iter().all(|c| self.is_concurrent_readonly(&c.name));
        if concurrent_batch {
            // Feed the failure-loop guard the same way the serial path does, so a concurrent
            // batch that keeps failing the same way (different args each step) is caught instead
            // of burning the budget to the step cap.
            let classified = self.run_readonly_batch(msg_id, calls).await?;
            for (call, (name, kind)) in calls.iter().zip(classified) {
                verification_ledger.lock().unwrap().observe(
                    completion::classify_tool(&call.name, &call.args.to_string()),
                    kind.is_none(),
                );
                match kind {
                    Some(k) => *failure_counts.entry((name, k)).or_insert(0) += 1,
                    None => {
                        failure_counts.retain(|(nm, _), _| nm != &name);
                        // A genuine tool success = the model recovered; clear the one-shot
                        // failure-loop latch so a *later* distinct failure loop earns its own
                        // nudge-before-halt instead of an immediate halt off a stale latch.
                        *failure_nudged = false;
                    }
                }
            }
            // Deliver any queued system hints (e.g. the doom-loop "change approach" nudge) — the
            // serial path does this per call; without it here the nudge sits undelivered and the
            // model is halted next step "after a nudge" it never actually saw.
            let hints: Vec<String> = std::mem::take(&mut self.pending_hints);
            for hint in hints {
                let hseq = self.next_seq();
                let _ = self
                    .store
                    .add_message(&self.id, hseq, Role::System, &hint, None);
                self.transcript.push(Message::system(hint));
            }
        } else {
            // Execute each requested tool through the permission broker, serially.
            for call in calls {
                let result = self.invoke_tool(msg_id, call).await?;
                let failure = classify_tool_failure(&result);
                verification_ledger.lock().unwrap().observe(
                    completion::classify_tool(&call.name, &call.args.to_string()),
                    failure.is_none(),
                );
                match failure {
                    Some(kind) => {
                        *failure_counts.entry((call.name.clone(), kind)).or_insert(0) += 1;
                    }
                    // A success on this tool means progress — clear its failure streaks so an
                    // earlier rough patch doesn't later trip the guard after the model recovered.
                    None => {
                        failure_counts.retain(|(nm, _), _| nm != &call.name);
                        // Also clear the one-shot failure-loop latch: a genuine success means
                        // the model recovered, so a *later* distinct failure loop in the same
                        // turn should get its own nudge-before-halt, not an immediate halt.
                        *failure_nudged = false;
                    }
                }
                // Env-fight spend cap (quality guards wave 4, fix 4): shell commands that look
                // like environment provisioning after a failure are venv archaeology — the
                // SWE-bench turns that burned minutes on host-python/repo-era mismatches. Allow
                // one alternate recovery attempt, then tell the model once per turn to stop.
                // Delivered via pending_hints so it lands right after the threshold result.
                if self.config.mesh.env_fight_nudge && call.name == "shell" {
                    if let Some(cmd) = call.args.get("command").and_then(|v| v.as_str()) {
                        if is_env_setup_command(cmd)
                            && self.env_fight.observe(shell_command_failed(&result))
                        {
                            self.presenter.emit(PresenterEvent::Warning(format!(
                                "environment setup/build spend cap reached after a failure \
                             ({ENV_FIGHT_ATTEMPT_THRESHOLD} attempts) — blocking further \
                             provisioning this turn"
                            )));
                            self.pending_hints.push(ENV_FIGHT_NUDGE.to_string());
                        }
                    }
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
                    let hseq = self.next_seq();
                    let _ = self
                        .store
                        .add_message(&self.id, hseq, Role::System, &hint, None);
                    self.transcript.push(Message::system(hint));
                }
            }
        }

        // Failure-loop guard: a tool that keeps failing the SAME way (across differing args) is
        // making no progress and burning the step/token budget — invisible to the identical-call
        // doom-loop above. Two-stage like that guard: nudge a change of approach once, then halt
        // if it persists. (BOTH the serial path and the concurrent read-only batch populate
        // `failure_counts`, so a batch failing the same way every step is caught here too.)
        const FAILURE_LOOP_THRESHOLD: usize = 3;
        if let Some((tool, kind, n)) = failure_counts
            .iter()
            .filter(|(_, &c)| c >= FAILURE_LOOP_THRESHOLD)
            .max_by_key(|(_, &c)| c)
            .map(|((nm, k), &c)| (nm.clone(), *k, c))
        {
            if !*failure_nudged {
                *failure_nudged = true;
                self.presenter.emit(PresenterEvent::Warning(format!(
                    "`{tool}` failed {n}× the same way ({}) — nudging a change of approach",
                    kind.label()
                )));
                let nudge = format!(
                    "Your `{tool}` calls keep failing with the same kind of error ({}). \
                 Repeating the same approach won't change that. Diagnose the root cause \
                 first (re-read the file / inspect the actual state), then take a DIFFERENT \
                 approach — or if you're genuinely blocked, say so plainly. Do not keep \
                 retrying the same way.",
                    kind.label()
                );
                let nseq = self.next_seq();
                let _ = self
                    .store
                    .add_message(&self.id, nseq, Role::System, &nudge, None);
                self.transcript.push(Message::system(nudge));
                // Fresh slate after the nudge: only halt if it loops AGAIN, and don't let a
                // stale pre-nudge streak trip the halt when the model is now trying something new.
                failure_counts.clear();
            } else {
                self.presenter.emit(PresenterEvent::Warning(format!(
                    "`{tool}` kept failing ({}) after a nudge — stopping to avoid a wasted loop",
                    kind.label()
                )));
                return Ok(true);
            }
        }

        Ok(false)
    }
}
