//! The accounting behind a Claude-Code-shaped `result` object, shared by the `json` and
//! `stream-json` presenters so both report the same numbers for the same run.
//!
//! A result describes the turn it closes. For `forge run` that is the whole invocation; with
//! `--input-format stream-json` each user message is a turn and gets its own result. Session totals
//! restored by `--continue`/`--resume` arrive before the first routing decision and become the
//! baseline, so `usage` and `total_cost_usd` report this run, not the conversation's history.

use std::time::{Duration, Instant};

use forge_types::{PresenterEvent, StopReason};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Totals {
    usd: f64,
    input: u64,
    cached: u64,
    output: u64,
}

impl Totals {
    fn since(self, base: Totals) -> Totals {
        Totals {
            usd: (self.usd - base.usd).max(0.0),
            input: self.input.saturating_sub(base.input),
            cached: self.cached.saturating_sub(base.cached),
            output: self.output.saturating_sub(base.output),
        }
    }
}

pub(crate) fn result_subtype(stop_reason: StopReason) -> &'static str {
    match stop_reason {
        StopReason::FinalAnswer => "success",
        StopReason::MaxSteps => "error_max_turns",
        StopReason::BudgetExhausted => "error_max_budget_usd",
        _ => "error_during_execution",
    }
}

pub(crate) struct ResultTotals {
    turn_started: Option<Instant>,
    session_id: String,
    routed: bool,
    model_calls: u64,
    last_step: Option<usize>,
    api_time: Duration,
    request_started: Option<Instant>,
    now: Totals,
    baseline: Totals,
    /// The newest per-call usage: the difference between the last two cost snapshots.
    last_call: Totals,
    last_error: Option<String>,
    denials: Vec<serde_json::Value>,
}

impl ResultTotals {
    pub(crate) fn new() -> Self {
        Self {
            turn_started: Some(Instant::now()),
            session_id: String::new(),
            routed: false,
            model_calls: 0,
            last_step: None,
            api_time: Duration::ZERO,
            request_started: None,
            now: Totals::default(),
            baseline: Totals::default(),
            last_call: Totals::default(),
            last_error: None,
            denials: Vec::new(),
        }
    }

    pub(crate) fn session_id(&self) -> &str {
        &self.session_id
    }

    pub(crate) fn observe(&mut self, event: &PresenterEvent) {
        match event {
            PresenterEvent::SessionStarted { id } => self.session_id = id.clone(),
            PresenterEvent::Routing { .. } => {
                self.routed = true;
                self.turn_started.get_or_insert_with(Instant::now);
            }
            PresenterEvent::ProviderRequest { step, .. } => {
                // A failover retry re-sends the same step; it is still one model turn.
                if self.last_step != Some(*step) {
                    self.model_calls += 1;
                    self.last_step = Some(*step);
                }
                self.request_started.get_or_insert_with(Instant::now);
            }
            PresenterEvent::Cost {
                session_total_usd,
                session_in,
                session_cached_in,
                session_out,
                ..
            } => {
                let totals = Totals {
                    usd: *session_total_usd,
                    input: *session_in,
                    cached: session_cached_in.unwrap_or(0),
                    output: *session_out,
                };
                if !self.routed && self.model_calls == 0 {
                    self.baseline = totals;
                } else {
                    self.last_call = totals.since(self.now);
                    if let Some(started) = self.request_started.take() {
                        self.api_time += started.elapsed();
                    }
                }
                self.now = totals;
            }
            PresenterEvent::Error(msg) => self.last_error = Some(msg.clone()),
            _ => {}
        }
    }

    /// A tool call the presenter refused (headless sessions cannot ask).
    pub(crate) fn record_denial(&mut self, tool: &str) {
        self.denials.push(serde_json::json!({
            "tool_name": tool,
            "tool_use_id": serde_json::Value::Null,
            "tool_input": {},
        }));
    }

    /// Usage of the most recent model call, shaped like an Anthropic `message.usage`.
    pub(crate) fn last_call_usage(&self) -> serde_json::Value {
        usage_json(self.last_call)
    }

    pub(crate) fn result(&self, final_text: &str, stop_reason: StopReason) -> serde_json::Value {
        let result = if final_text.trim().is_empty() {
            self.last_error.clone().unwrap_or_default()
        } else {
            final_text.to_string()
        };
        let spent = self.now.since(self.baseline);
        let duration = self.turn_started.map_or(Duration::ZERO, |t| t.elapsed());
        serde_json::json!({
            "type": "result",
            "subtype": result_subtype(stop_reason),
            "is_error": stop_reason != StopReason::FinalAnswer,
            "duration_ms": duration.as_millis() as u64,
            "duration_api_ms": self.api_time.as_millis() as u64,
            "num_turns": self.model_calls.max(1),
            "result": result,
            "stop_reason": stop_reason.as_str(),
            "session_id": self.session_id,
            "total_cost_usd": spent.usd,
            "usage": usage_json(spent),
            "permission_denials": self.denials,
            "uuid": forge_types::new_id(),
        })
    }

    /// Close the turn: the next result reports only what happens after this point.
    pub(crate) fn finish_turn(&mut self) {
        self.baseline = self.now;
        self.routed = false;
        self.model_calls = 0;
        self.last_step = None;
        self.api_time = Duration::ZERO;
        self.request_started = None;
        self.turn_started = None;
        self.last_error = None;
        self.denials.clear();
    }
}

/// Anthropic's `usage` shape. Its `input_tokens` excludes cache reads, which Forge counts inside
/// `input`, so a consumer summing `input_tokens + cache_read_input_tokens` (as Claude Code
/// integrations do) gets the real total instead of counting the cached prefix twice.
fn usage_json(t: Totals) -> serde_json::Value {
    serde_json::json!({
        "input_tokens": t.input.saturating_sub(t.cached),
        "cache_read_input_tokens": t.cached,
        "output_tokens": t.output,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cost(usd: f64, input: u64, cached: u64, output: u64) -> PresenterEvent {
        PresenterEvent::Cost {
            session_total_usd: usd,
            session_in: input,
            session_cached_in: Some(cached),
            session_out: output,
            context_tokens: input,
            context_limit: None,
        }
    }

    fn routing() -> PresenterEvent {
        PresenterEvent::Routing {
            effort: None,
            tier: "trivial".into(),
            model: "m".into(),
            rationale: "r".into(),
        }
    }

    fn request(step: usize) -> PresenterEvent {
        PresenterEvent::ProviderRequest {
            model: "m".into(),
            step,
        }
    }

    #[test]
    fn counts_model_calls_once_per_step_even_across_failover_retries() {
        let mut t = ResultTotals::new();
        for e in [
            routing(),
            request(0),
            request(0),
            cost(0.1, 10, 0, 1),
            request(1),
        ] {
            t.observe(&e);
        }
        let r = t.result("ok", StopReason::FinalAnswer);
        assert_eq!(r["num_turns"], 2);
    }

    #[test]
    fn each_turn_reports_only_its_own_spend() {
        let mut t = ResultTotals::new();
        for e in [
            cost(1.0, 100, 0, 10),
            routing(),
            request(0),
            cost(1.5, 160, 20, 14),
        ] {
            t.observe(&e);
        }
        let first = t.result("a", StopReason::FinalAnswer);
        assert_eq!(
            first["usage"]["input_tokens"], 40,
            "restored totals are the baseline; cache reads are reported apart"
        );
        assert_eq!(first["usage"]["cache_read_input_tokens"], 20);
        assert!((first["total_cost_usd"].as_f64().unwrap() - 0.5).abs() < 1e-9);
        assert_eq!(t.last_call_usage()["output_tokens"], 4);

        t.finish_turn();
        for e in [routing(), request(0), cost(2.0, 200, 20, 20)] {
            t.observe(&e);
        }
        let second = t.result("b", StopReason::FinalAnswer);
        assert_eq!(second["usage"]["input_tokens"], 40);
        assert_eq!(second["usage"]["output_tokens"], 6);
        assert_eq!(second["num_turns"], 1);
    }

    #[test]
    fn budget_stops_use_the_claude_code_budget_subtype() {
        assert_eq!(
            result_subtype(StopReason::BudgetExhausted),
            "error_max_budget_usd"
        );
        assert_eq!(result_subtype(StopReason::MaxSteps), "error_max_turns");
    }

    #[test]
    fn an_empty_answer_carries_the_last_error_and_denials_are_listed() {
        let mut t = ResultTotals::new();
        t.observe(&PresenterEvent::Error("provider down".into()));
        t.record_denial("shell");
        let r = t.result("", StopReason::NoOutput);
        assert_eq!(r["result"], "provider down");
        assert_eq!(r["is_error"], true);
        assert_eq!(r["permission_denials"][0]["tool_name"], "shell");
    }
}
