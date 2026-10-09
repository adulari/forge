//! `--output-format json`: one final result object, shaped like `claude -p --output-format json`.

use std::io::Write;
use std::time::Instant;

use forge_types::{SideEffect, StopReason};

use crate::{ConfirmOutcome, Presenter, PresenterEvent, QChoice, NO_ANSWER};

/// Buffers a whole turn and prints a single Claude-Code-shaped `result` object when it ends, so
/// scripts written against `claude -p --output-format json` can drive `forge run` unchanged.
/// Nothing else reaches stdout; warnings and errors are folded into the object or kept on stderr.
pub struct JsonResultPresenter {
    out: Box<dyn Write + Send>,
    started: Instant,
    session_id: String,
    turns: u64,
    cost_usd: f64,
    input: u64,
    cached: Option<u64>,
    output: u64,
    /// Session totals restored before this invocation's first model call (`--continue`/`--resume`).
    baseline: Option<(f64, u64, u64, u64)>,
    last_error: Option<String>,
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
            started: Instant::now(),
            session_id: String::new(),
            turns: 0,
            cost_usd: 0.0,
            input: 0,
            cached: None,
            output: 0,
            baseline: None,
            last_error: None,
        }
    }

    fn finish(&mut self, final_text: &str, stop_reason: StopReason) {
        let subtype = match stop_reason {
            StopReason::FinalAnswer => "success",
            StopReason::MaxSteps => "error_max_turns",
            _ => "error_during_execution",
        };
        let is_error = stop_reason != StopReason::FinalAnswer;
        let result = if final_text.trim().is_empty() {
            self.last_error.clone().unwrap_or_default()
        } else {
            final_text.to_string()
        };
        let (base_usd, base_in, base_cached, base_out) = self.baseline.unwrap_or_default();
        let value = serde_json::json!({
            "type": "result",
            "subtype": subtype,
            "is_error": is_error,
            "duration_ms": self.started.elapsed().as_millis() as u64,
            "num_turns": self.turns.max(1),
            "result": result,
            "stop_reason": stop_reason.as_str(),
            "session_id": self.session_id,
            "total_cost_usd": (self.cost_usd - base_usd).max(0.0),
            "usage": {
                "input_tokens": self.input.saturating_sub(base_in),
                "cache_read_input_tokens": self.cached.unwrap_or(0).saturating_sub(base_cached),
                "output_tokens": self.output.saturating_sub(base_out)
            }
        });
        if serde_json::to_writer(&mut self.out, &value).is_ok() {
            let _ = self.out.write_all(b"\n");
            let _ = self.out.flush();
        }
    }
}

impl Presenter for JsonResultPresenter {
    fn emit(&mut self, event: PresenterEvent) {
        match event {
            PresenterEvent::SessionStarted { id } => self.session_id = id,
            PresenterEvent::Routing { .. } => self.turns += 1,
            PresenterEvent::Cost {
                session_total_usd,
                session_in,
                session_cached_in,
                session_out,
                ..
            } if self.turns == 0 && self.baseline.is_none() => {
                self.baseline = Some((
                    session_total_usd,
                    session_in,
                    session_cached_in.unwrap_or(0),
                    session_out,
                ));
            }
            PresenterEvent::Cost {
                session_total_usd,
                session_in,
                session_cached_in,
                session_out,
                ..
            } => {
                self.cost_usd = session_total_usd;
                self.input = session_in;
                self.cached = session_cached_in;
                self.output = session_out;
            }
            PresenterEvent::Error(msg) => {
                eprintln!("error: {msg}");
                self.last_error = Some(msg);
            }
            PresenterEvent::Warning(msg) => eprintln!("warning: {msg}"),
            PresenterEvent::Done {
                final_text,
                stop_reason,
            } => self.finish(&final_text, stop_reason),
            _ => {}
        }
    }

    fn confirm(&mut self, _tool: &str, _side_effect: SideEffect) -> ConfirmOutcome {
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
        assert_eq!(r["usage"]["input_tokens"], 100);
        assert_eq!(r["usage"]["cache_read_input_tokens"], 40);
        assert_eq!(r["usage"]["output_tokens"], 7);
        assert!(r["duration_ms"].is_u64());
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
        assert_eq!(r["usage"]["input_tokens"], 40);
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
