//! `--output-format json`: one final result object, shaped like `claude -p --output-format json`.

use std::io::Write;

use forge_types::{SideEffect, StopReason};

use crate::result_totals::ResultTotals;
use crate::usage_source::UsageSlot;
use crate::{ConfirmOutcome, Presenter, PresenterEvent, QChoice, NO_ANSWER};

/// Buffers a whole turn and prints a single Claude-Code-shaped `result` object when it ends, so
/// scripts written against `claude -p --output-format json` can drive `forge run` unchanged.
/// Nothing else reaches stdout; warnings and errors are folded into the object or kept on stderr.
pub struct JsonResultPresenter {
    out: Box<dyn Write + Send>,
    totals: ResultTotals,
}

/// The terminal `result` line for a run that failed before (or without) the presenter's `Done`:
/// a machine consumer must see `is_error: true`, not an empty stdout it cannot tell from a crash.
/// It carries the same fields as a finished run's result, zeroed, as Claude Code's does.
pub fn error_result_line(message: &str) -> String {
    let value = serde_json::json!({
        "type": "result",
        "subtype": "error_during_execution",
        "is_error": true,
        "result": message,
        "stop_reason": "error",
        "uuid": forge_types::new_id(),
        "duration_ms": 0,
        "duration_api_ms": 0,
        "num_turns": 0,
        "total_cost_usd": 0.0,
        "usage": {
            "input_tokens": 0,
            "cache_creation_input_tokens": 0,
            "cache_read_input_tokens": 0,
            "output_tokens": 0,
        },
        "modelUsage": {},
        "permission_denials": [],
    });
    value.to_string()
}

impl Default for JsonResultPresenter {
    fn default() -> Self {
        Self::with_writer(Box::new(std::io::stdout()))
    }
}

impl JsonResultPresenter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_writer(out: Box<dyn Write + Send>) -> Self {
        Self {
            out,
            totals: ResultTotals::new(),
        }
    }

    /// Fill `usage.cache_creation_input_tokens` and `modelUsage` from the session's store (see
    /// [`UsageSlot`]); without one they come from the turn's cost events alone.
    pub fn with_usage_slot(mut self, slot: UsageSlot) -> Self {
        self.totals.set_usage_slot(slot);
        self
    }

    fn finish(&mut self, final_text: &str, stop_reason: StopReason) {
        let value = self.totals.result(final_text, stop_reason);
        if serde_json::to_writer(&mut self.out, &value).is_ok() {
            let _ = self.out.write_all(b"\n");
            let _ = self.out.flush();
        }
        self.totals.finish_turn();
    }
}

impl Presenter for JsonResultPresenter {
    fn emit(&mut self, event: PresenterEvent) {
        self.totals.observe(&event);
        match event {
            PresenterEvent::Error(msg) => eprintln!("error: {msg}"),
            PresenterEvent::Warning(msg) => eprintln!("warning: {msg}"),
            PresenterEvent::Done {
                final_text,
                stop_reason,
            } => self.finish(&final_text, stop_reason),
            _ => {}
        }
    }

    fn confirm(&mut self, tool: &str, _side_effect: SideEffect) -> ConfirmOutcome {
        self.totals.record_denial(tool);
        ConfirmOutcome::Deny
    }

    fn ask(&mut self, _question: &str, _options: &[QChoice], _allow_other: bool) -> String {
        NO_ANSWER.to_string()
    }

    fn read_line(&mut self) -> Option<String> {
        None
    }

    fn is_attended(&self) -> bool {
        false
    }

    fn consumes_recap(&self) -> bool {
        false
    }

    fn consumes_suggestions(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Clone)]
    struct Buf(Arc<Mutex<Vec<u8>>>);
    impl Write for Buf {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn run(events: Vec<PresenterEvent>) -> Vec<serde_json::Value> {
        let buf = Arc::new(Mutex::new(Vec::new()));
        let mut p = JsonResultPresenter::with_writer(Box::new(Buf(buf.clone())));
        for e in events {
            p.emit(e);
        }
        let text = String::from_utf8(buf.lock().unwrap().clone()).unwrap();
        text.lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    fn cost() -> PresenterEvent {
        PresenterEvent::Cost {
            session_total_usd: 0.25,
            session_in: 100,
            session_cached_in: Some(40),
            session_out: 7,
            context_tokens: 100,
            context_limit: None,
        }
    }

    #[test]
    fn emits_exactly_one_cc_shaped_result_object() {
        let out = run(vec![
            PresenterEvent::SessionStarted { id: "s-1".into() },
            PresenterEvent::Routing {
                effort: None,
                tier: "trivial".into(),
                model: "m".into(),
                rationale: "r".into(),
            },
            PresenterEvent::AssistantText("OK".into()),
            cost(),
            PresenterEvent::Done {
                final_text: "OK".into(),
                stop_reason: StopReason::FinalAnswer,
            },
        ]);
        assert_eq!(out.len(), 1, "no event stream, only the result: {out:?}");
        let r = &out[0];
        assert_eq!(r["type"], "result");
        assert_eq!(r["subtype"], "success");
        assert_eq!(r["is_error"], false);
        assert_eq!(r["result"], "OK");
        assert_eq!(r["session_id"], "s-1");
        assert_eq!(r["num_turns"], 1);
        assert_eq!(r["total_cost_usd"], 0.25);
        assert_eq!(
            r["usage"]["input_tokens"], 60,
            "uncached input, as Anthropic reports it"
        );
        assert_eq!(r["usage"]["cache_read_input_tokens"], 40);
        assert_eq!(r["usage"]["cache_creation_input_tokens"], 0);
        assert_eq!(r["usage"]["output_tokens"], 7);
        assert!(r["duration_ms"].is_u64());
        assert_eq!(r["modelUsage"]["m"]["inputTokens"], 60);
        assert_eq!(r["modelUsage"]["m"]["outputTokens"], 7);
        assert!(r["uuid"].is_string() && r["permission_denials"].is_array());
    }

    #[test]
    fn resumed_session_reports_this_invocation_not_session_totals() {
        let restored = PresenterEvent::Cost {
            session_total_usd: 0.10,
            session_in: 60,
            session_cached_in: Some(30),
            session_out: 5,
            context_tokens: 60,
            context_limit: None,
        };
        let out = run(vec![
            PresenterEvent::SessionStarted { id: "s-1".into() },
            restored,
            PresenterEvent::Routing {
                effort: None,
                tier: "trivial".into(),
                model: "m".into(),
                rationale: "r".into(),
            },
            cost(),
            PresenterEvent::Done {
                final_text: "OK".into(),
                stop_reason: StopReason::FinalAnswer,
            },
        ]);
        let r = &out[0];
        assert_eq!(r["usage"]["input_tokens"], 30);
        assert_eq!(r["usage"]["cache_read_input_tokens"], 10);
        assert_eq!(r["usage"]["output_tokens"], 2);
        assert!((r["total_cost_usd"].as_f64().unwrap() - 0.15).abs() < 1e-9);
    }

    #[test]
    fn incomplete_turn_is_an_error_result() {
        let out = run(vec![
            PresenterEvent::Error("provider down".into()),
            PresenterEvent::Done {
                final_text: String::new(),
                stop_reason: StopReason::NoOutput,
            },
        ]);
        assert_eq!(out[0]["is_error"], true);
        assert_eq!(out[0]["subtype"], "error_during_execution");
        assert_eq!(out[0]["result"], "provider down");
    }
}

#[cfg(test)]
mod error_line_tests {
    #[test]
    fn error_result_is_a_parseable_error_object() {
        let v: serde_json::Value = serde_json::from_str(&super::error_result_line("boom")).unwrap();
        assert_eq!(v["is_error"], true);
        assert_eq!(v["result"], "boom");
        assert_eq!(v["usage"]["cache_creation_input_tokens"], 0);
        assert!(v["uuid"].is_string() && v["modelUsage"].is_object());
        assert_eq!(v["permission_denials"], serde_json::json!([]));
    }
}
