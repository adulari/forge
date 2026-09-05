//! NDJSON (`stream-json`) presenter — the non-interactive machine-readable surface.

use std::io::Write;

use forge_types::SideEffect;

use crate::{ConfirmOutcome, Presenter, PresenterEvent, QChoice, NO_ANSWER};

/// NDJSON (`stream-json`) presenter: emits one JSON object per line on stdout, mirroring Claude
/// Code's `--output-format stream-json` event shapes as closely as practical so existing CC
/// stream-json integrations (editors, agent SDKs) can consume Forge's output. Non-interactive:
/// confirmations deny (safe default) and prompts return no answer. Run with `--mode bypass` /
/// `--mode accept-edits` for autonomous tool use.
pub struct StreamJsonPresenter {
    out: Box<dyn Write + Send>,
    session_id: String,
}

impl Default for StreamJsonPresenter {
    fn default() -> Self {
        Self {
            out: Box::new(std::io::stdout()),
            session_id: String::new(),
        }
    }
}

impl StreamJsonPresenter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Build a presenter that writes into an arbitrary sink (used by tests to capture NDJSON).
    pub fn with_writer(out: Box<dyn Write + Send>) -> Self {
        Self {
            out,
            session_id: String::new(),
        }
    }

    /// Write one event as a single compact JSON line, flushing so consumers see it live.
    fn line(&mut self, value: serde_json::Value) {
        if serde_json::to_writer(&mut self.out, &value).is_ok() {
            let _ = self.out.write_all(b"\n");
            let _ = self.out.flush();
        }
    }
}

impl Presenter for StreamJsonPresenter {
    fn emit(&mut self, event: PresenterEvent) {
        let sid = self.session_id.clone();
        match event {
            PresenterEvent::SessionStarted { id } => {
                self.session_id = id.clone();
                self.line(serde_json::json!({
                    "type": "system", "subtype": "init", "session_id": id
                }));
            }
            PresenterEvent::Routing {
                tier,
                model,
                rationale,
                ..
            } => self.line(serde_json::json!({
                "type": "system", "subtype": "routing", "session_id": sid,
                "tier": tier, "model": model, "rationale": rationale
            })),
            PresenterEvent::AssistantText(text) | PresenterEvent::AssistantDelta(text) => self
                .line(serde_json::json!({
                    "type": "assistant", "session_id": sid,
                    "message": { "role": "assistant",
                                 "content": [ { "type": "text", "text": text } ] }
                })),
            PresenterEvent::Reasoning(delta) => self.line(serde_json::json!({
                "type": "assistant", "session_id": sid,
                "message": { "role": "assistant",
                             "content": [ { "type": "thinking", "thinking": delta } ] }
            })),
            PresenterEvent::ToolStart { name, args } => {
                // `args` is a JSON string; embed the parsed value when possible (CC `tool_use.input`).
                let input = serde_json::from_str::<serde_json::Value>(&args)
                    .unwrap_or(serde_json::Value::String(args));
                self.line(serde_json::json!({
                    "type": "assistant", "session_id": sid,
                    "message": { "role": "assistant",
                                 "content": [ { "type": "tool_use", "name": name, "input": input } ] }
                }));
            }
            PresenterEvent::ToolResult { name, ok, summary } => self.line(serde_json::json!({
                "type": "user", "session_id": sid,
                "message": { "role": "user",
                             "content": [ { "type": "tool_result", "tool_name": name,
                                            "is_error": !ok, "content": summary } ] }
            })),
            PresenterEvent::Cost {
                session_total_usd,
                session_in,
                session_cached_in,
                session_out,
                ..
            } => self.line(serde_json::json!({
                "type": "system", "subtype": "usage", "session_id": sid,
                "total_cost_usd": session_total_usd,
                "usage": {
                    "input_tokens": session_in,
                    "cached_input_tokens": session_cached_in,
                    "output_tokens": session_out
                }
            })),
            PresenterEvent::Warning(msg) => self.line(serde_json::json!({
                "type": "system", "subtype": "warning", "session_id": sid, "message": msg
            })),
            PresenterEvent::Error(msg) => self.line(serde_json::json!({
                "type": "system", "subtype": "error", "session_id": sid, "message": msg
            })),
            PresenterEvent::Done {
                final_text,
                stop_reason,
            } => self.line(serde_json::json!({
                "type": "result", "subtype": "success", "session_id": sid,
                "result": final_text, "stop_reason": format!("{stop_reason:?}")
            })),
            // Other events are not part of the CC stream-json surface; intentionally ignored.
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
}
