//! The `run_code` pump: runs the script on the session task and serves its tool requests.
//!
//! The script future owns no session state; its `__tool` host function sends each request down a
//! channel and awaits a oneshot reply. The pump `select!`s over that channel, the script future and
//! the deadline, the same shape as `workflow::run`. Each request runs to completion through
//! `invoke_tool` inside the loop body, so the deadline can only fire between inner calls: an
//! in-flight call (its snapshot, write, audit row, or an open permission prompt) is never
//! cancelled. Time spent on a permission prompt does not count against the budget.

use std::time::{Duration, Instant};

use forge_workflow::{HostFunction, HostResult, Limits};
use serde_json::Value;
use tokio::sync::{mpsc, oneshot};

use super::catalog::{self, Resolved, ToolDirectory};
use super::prompt_clock::{self, PromptClock};
use super::{InnerCall, ScriptEnd, ScriptReport};
use crate::tool_outcome::TurnLedgers;
use crate::{classify_tool_failure, CoreError, PresenterEvent, Session};

/// How long a script gets to unwind (catch the deadline error, return partial results) after its
/// deadline before the pump drops it.
const DEADLINE_GRACE: Duration = Duration::from_secs(2);

/// Engine limits for a codemode script: a tighter heap and sync slice than workflows, since a
/// script's real work happens in its tool calls.
const LIMITS: Limits = Limits {
    memory_bytes: 64 * 1024 * 1024,
    max_sync_ms: 5_000,
};

struct ToolReq {
    name: String,
    args: Value,
    reply: oneshot::Sender<HostResult>,
}

fn tool_host_fn(tx: mpsc::UnboundedSender<ToolReq>) -> HostFunction {
    HostFunction::new("__tool", move |args: Vec<Value>| {
        let tx = tx.clone();
        async move {
            let mut args = args.into_iter();
            let name = args
                .next()
                .and_then(|v| v.as_str().map(str::to_string))
                .unwrap_or_default();
            let call_args = args
                .next()
                .unwrap_or_else(|| Value::Object(Default::default()));
            let (reply, answer) = oneshot::channel();
            tx.send(ToolReq {
                name,
                args: call_args,
                reply,
            })
            .map_err(|_| "run_code has already finished".to_string())?;
            answer
                .await
                .map_err(|_| "run_code stopped before this call ran".to_string())?
        }
    })
}

/// Per-run state the pump threads through each inner call.
struct Run {
    calls: Vec<InnerCall>,
    hints: Vec<String>,
    max_calls: usize,
    /// `[harness] script_parallel`: requests that arrive together may run together.
    parallel: bool,
}

impl Session {
    /// Run one `run_code` call: the script, every inner call through `invoke_tool` and the shared
    /// bookkeeping, then the rendered result. Only this outer result enters the transcript.
    pub(crate) async fn run_code(
        &mut self,
        msg_id: &str,
        call: &forge_types::ToolCall,
        ledgers: &mut TurnLedgers<'_>,
    ) -> Result<String, CoreError> {
        let args_json = serde_json::to_string(&call.args)?;
        if let Some(scope) = self
            .task_scope
            .as_ref()
            .filter(|scope| !scope.permits_tool(&call.name))
        {
            let result = format!(
                "permission denied by task scope {}: `{}` is unavailable for {:?}",
                scope.audit_digest(),
                call.name,
                scope.contract.intent()
            );
            self.store
                .record_tool_call(msg_id, &call.name, &args_json, &result, "denied", "error")?;
            return Ok(result);
        }
        let (code, timeout_secs) = match super::script_from_args(&call.args, &self.config.codemode)
        {
            Ok(parsed) => parsed,
            Err(msg) => {
                let result = format!("error: {msg}");
                self.store.record_tool_call(
                    msg_id, &call.name, &args_json, &result, "allowed", "error",
                )?;
                return Ok(result);
            }
        };
        self.presenter.emit(PresenterEvent::ToolStart {
            name: call.name.clone(),
            args: code.clone(),
        });

        let started = Instant::now();
        let (clock, slot) = prompt_clock::install(&mut self.presenter);
        let mut run = Run {
            calls: Vec::new(),
            hints: Vec::new(),
            max_calls: self.config.codemode.max_calls,
            parallel: self.config.harness.script_parallel,
        };
        let parallel = run.parallel;
        let pump = self.pump(
            msg_id,
            call,
            &code,
            Duration::from_secs(timeout_secs),
            &clock,
            &mut run,
            ledgers,
        );
        let pumped = if parallel {
            super::with_parallel_inner_cap(pump).await
        } else {
            pump.await
        };
        prompt_clock::uninstall(&mut self.presenter, &slot);
        let (end, logs, dropped_logs, deadline_hit) = pumped?;
        let saved: Vec<(usize, String)> = if parallel {
            (1..=run.calls.len())
                .filter_map(|n| {
                    self.kept_outputs
                        .get(&format!("{}.{n}", call.id))
                        .map(|path| (n, path.clone()))
                })
                .collect()
        } else {
            Vec::new()
        };

        let report = ScriptReport {
            end,
            logs,
            dropped_logs,
            calls: run.calls,
            deadline_hit,
            timeout_secs,
            elapsed: started.elapsed(),
            saved,
        };
        let ok = report.succeeded();
        let result = crate::tool_output::fit_for_model(
            super::render(&report, self.config.codemode.result_max_chars),
            None,
        );
        // Each hint was already appended to its own call's value; deliver it once more after the
        // outer result only when the script did not return it (LSP blocks run to thousands of
        // tokens, so a returned hint must not enter the transcript twice).
        for hint in run.hints {
            if !result.contains(hint.as_str()) && !self.pending_hints.contains(&hint) {
                self.pending_hints.push(hint);
            }
        }
        self.presenter.emit(PresenterEvent::ToolResult {
            name: call.name.clone(),
            ok,
            summary: crate::summarize(&result),
            detail: crate::tool_detail(&result),
        });
        self.store.record_tool_call(
            msg_id,
            &call.name,
            &args_json,
            &result,
            "allowed",
            if ok { "ok" } else { "error" },
        )?;
        Ok(result)
    }

    #[allow(clippy::too_many_arguments)]
    async fn pump(
        &mut self,
        msg_id: &str,
        outer: &forge_types::ToolCall,
        code: &str,
        budget: Duration,
        clock: &PromptClock,
        run: &mut Run,
        ledgers: &mut TurnLedgers<'_>,
    ) -> Result<(ScriptEnd, Vec<String>, u64, bool), CoreError> {
        let (tx, mut rx) = mpsc::unbounded_channel::<ToolReq>();
        let script = super::wrap_with(code, run.parallel);
        let script_fut =
            forge_workflow::run_script_with_limits(vec![tool_host_fn(tx)], &script, LIMITS);
        tokio::pin!(script_fut);

        let started = Instant::now();
        let used = || started.elapsed().saturating_sub(clock.waited());
        let mut channel_open = true;
        let mut grace_until: Option<Instant> = None;
        let finished = loop {
            let wake = match grace_until {
                Some(until) => until.saturating_duration_since(Instant::now()),
                None => budget.saturating_sub(used()),
            };
            tokio::select! {
                biased;
                req = rx.recv(), if channel_open => {
                    let Some(req) = req else {
                        channel_open = false;
                        continue;
                    };
                    // The dispatch point: the only place the deadline refuses work.
                    if grace_until.is_some() || used() >= budget {
                        grace_until.get_or_insert_with(|| Instant::now() + DEADLINE_GRACE);
                        let _ = req.reply.send(Err(format!(
                            "run_code deadline exceeded ({}s); no further tool calls run",
                            budget.as_secs()
                        )));
                        continue;
                    }
                    let mut batch = vec![req];
                    if run.parallel {
                        // A script that awaits several calls at once (Promise.all) has queued
                        // them all by now; take them together.
                        let room = run.max_calls.saturating_sub(run.calls.len()).max(1);
                        while batch.len() < room {
                            match rx.try_recv() {
                                Ok(next) => batch.push(next),
                                Err(_) => break,
                            }
                        }
                    }
                    self.dispatch_batch(msg_id, outer, batch, run, ledgers).await?;
                }
                res = &mut script_fut => break Some(res),
                _ = tokio::time::sleep(wake) => {
                    if grace_until.is_some() {
                        break None;
                    }
                    // A prompt may have paused the clock since `wake` was computed; only a budget
                    // that is really spent starts the grace period.
                    if used() >= budget {
                        grace_until = Some(Instant::now() + DEADLINE_GRACE);
                    }
                }
            }
        };
        let deadline_hit = grace_until.is_some();
        // Requests still queued when the script ended were never awaited; they do not run.
        while let Ok(req) = rx.try_recv() {
            let _ = req
                .reply
                .send(Err("run_code has already finished".to_string()));
        }
        Ok(match finished {
            Some(Ok(value)) => {
                let (end, logs, dropped) = super::parse_script_value(value);
                (end, logs, dropped, deadline_hit)
            }
            Some(Err(engine)) => (
                ScriptEnd::Failed(super::engine_error(&engine)),
                Vec::new(),
                0,
                deadline_hit,
            ),
            None => (ScriptEnd::Expired, Vec::new(), 0, true),
        })
    }

    /// Serve requests that arrived together. Read-only calls that need no prompt and no hook run
    /// concurrently through the shared read-only batch path; anything else, or a single request,
    /// takes the one-at-a-time path in arrival order.
    async fn dispatch_batch(
        &mut self,
        msg_id: &str,
        outer: &forge_types::ToolCall,
        batch: Vec<ToolReq>,
        run: &mut Run,
        ledgers: &mut TurnLedgers<'_>,
    ) -> Result<(), CoreError> {
        if let Some(inners) = self.concurrent_inner_calls(outer, &batch, run) {
            for inner in &inners {
                ledgers.count_started(&inner.name);
            }
            let classified = self.run_readonly_calls(msg_id, &inners, false).await?;
            for ((req, inner), (_, kind, result)) in batch.into_iter().zip(inners).zip(classified) {
                self.observe_tool_outcome(&inner, &result, ledgers);
                run.calls.push(InnerCall {
                    name: inner.name,
                    ok: kind.is_none(),
                });
                let _ = req.reply.send(if super::throws(&result) {
                    Err(result)
                } else {
                    Ok(Value::String(result))
                });
            }
            return Ok(());
        }
        for req in batch {
            let answer = self
                .dispatch_inner(msg_id, outer, req.name, req.args, run, ledgers)
                .await?;
            let _ = req.reply.send(answer);
        }
        Ok(())
    }

    /// The resolved inner calls when EVERY request in `batch` may run concurrently; `None` sends
    /// the whole batch down the serial path (which prompts, snapshots and runs hooks per call).
    fn concurrent_inner_calls(
        &self,
        outer: &forge_types::ToolCall,
        batch: &[ToolReq],
        run: &Run,
    ) -> Option<Vec<forge_types::ToolCall>> {
        if batch.len() < 2 || run.calls.len() + batch.len() > run.max_calls {
            return None;
        }
        let tool_hooks = self.config.hooks.iter().any(|h| {
            matches!(
                h.event,
                forge_config::HookEvent::PreToolUse
                    | forge_config::HookEvent::PostToolUse
                    | forge_config::HookEvent::PostToolUseFailure
            )
        });
        if tool_hooks {
            return None;
        }
        let mut inners = Vec::with_capacity(batch.len());
        for (i, req) in batch.iter().enumerate() {
            if req.name == super::SEARCH_CALL {
                return None;
            }
            let Resolved::Call(name, args) = catalog::resolve(&req.name, req.args.clone(), self)
            else {
                return None;
            };
            let inner = self.route_external_mcp(forge_types::ToolCall {
                id: format!("{}.{}", outer.id, run.calls.len() + i + 1),
                name,
                args,
            });
            let inner = self.resolve_model_call(&inner);
            if !self.is_concurrent_readonly(&inner) {
                return None;
            }
            inners.push(inner);
        }
        Some(inners)
    }

    /// Serve one script request. `$search` reads the directory; every tool call goes through
    /// `invoke_tool` and `observe_tool_outcome` exactly as a call the model made itself.
    async fn dispatch_inner(
        &mut self,
        msg_id: &str,
        outer: &forge_types::ToolCall,
        name: String,
        args: Value,
        run: &mut Run,
        ledgers: &mut TurnLedgers<'_>,
    ) -> Result<HostResult, CoreError> {
        if name == super::SEARCH_CALL {
            let query = args
                .get("query")
                .and_then(Value::as_str)
                .unwrap_or_default();
            return Ok(Ok(catalog::search(&ToolDirectory::catalog(self), query)));
        }
        if run.calls.len() >= run.max_calls {
            return Ok(Err(format!(
                "run_code tool-call limit reached ({}); return what you have",
                run.max_calls
            )));
        }
        let (name, args) = match catalog::resolve(&name, args, self) {
            Resolved::Call(name, args) => (name, args),
            Resolved::Denied(msg) => return Ok(Err(msg)),
        };
        let inner = self.route_external_mcp(forge_types::ToolCall {
            id: format!("{}.{}", outer.id, run.calls.len() + 1),
            name,
            args,
        });
        ledgers.count_started(&inner.name);
        let hints_before = self.pending_hints.len();
        let result = self.invoke_tool(msg_id, &inner).await?;
        self.observe_tool_outcome(&inner, &result, ledgers);
        let hints: Vec<String> = self.pending_hints.drain(hints_before..).collect();
        let value = super::with_hints(&result, &hints);
        run.hints.extend(hints);
        run.calls.push(InnerCall {
            name: inner.name,
            ok: classify_tool_failure(&result).is_none(),
        });
        Ok(if super::throws(&result) {
            Err(value)
        } else {
            Ok(Value::String(value))
        })
    }
}
