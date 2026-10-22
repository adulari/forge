//! NDJSON (`stream-json`) presenter — the non-interactive machine-readable surface.
//!
//! Event shapes follow `claude -p --output-format stream-json`: a `system/init` with the session's
//! metadata, one `assistant` event per finished message (deltas only as `stream_event` lines under
//! `--include-partial-messages`), `user` events carrying tool results, and a final `result` with
//! usage, cost, duration and turn count. Forge adds `system/routing` (which model the mesh picked),
//! `system/usage` (running cost after every model call), `system/warning`, `system/error` and
//! `system/tool_output`; Claude Code consumers ignore system subtypes they do not know.

use std::collections::VecDeque;
use std::io::Write;
use std::sync::{Arc, Mutex};

use forge_types::{SideEffect, StopReason};
use serde_json::{json, Value};

use crate::result_totals::ResultTotals;
use crate::stream_json_message::{MessageAssembler, Out};
use crate::usage_source::UsageSlot;
use crate::{ConfirmOutcome, Presenter, PresenterEvent, QChoice, NO_ANSWER};

/// Session metadata for the `system/init` event, known only once the session is built.
#[derive(Clone, Debug, Default)]
pub struct StreamInit {
    pub cwd: String,
    /// The pinned model, or `auto` when the mesh routes per turn.
    pub model: String,
    /// Claude Code's permission-mode spelling (`default`, `acceptEdits`, ...).
    pub permission_mode: String,
    pub tools: Vec<String>,
    /// `(name, status)` per configured MCP server.
    pub mcp_servers: Vec<(String, String)>,
    pub slash_commands: Vec<String>,
    pub skills: Vec<String>,
    pub forge_version: String,
}

/// Hand-off for a deferred `system/init`: the caller fills it after building the session, and the
/// presenter writes `init` (then everything it held back) on its next event.
#[derive(Clone, Default)]
pub struct StreamInitSlot(Arc<Mutex<Option<StreamInit>>>);

impl StreamInitSlot {
    pub fn fill(&self, init: StreamInit) {
        if let Ok(mut slot) = self.0.lock() {
            *slot = Some(init);
        }
    }

    fn take(&self) -> Option<StreamInit> {
        self.0.lock().ok().and_then(|mut slot| slot.take())
    }
}

enum Init {
    /// Write a minimal `init` as soon as the session starts.
    Immediate,
    /// Hold every line until the slot is filled (or the turn ends).
    Deferred(StreamInitSlot),
    Sent,
}

/// NDJSON presenter. Non-interactive: confirmations deny (recorded as `permission_denials`) and
/// questions get no answer. Run with `--mode bypass` / `--mode accept-edits` for autonomous use.
pub struct StreamJsonPresenter {
    out: Box<dyn Write + Send>,
    totals: ResultTotals,
    message: MessageAssembler,
    init: Init,
    held: Vec<Value>,
    next_tool_id: u64,
    pending_tool_ids: VecDeque<(String, String)>,
    /// `--verbose`: also emit Claude Code's `system/status` events.
    verbose: bool,
    /// The totals of the last `system/usage` event, so an unchanged total is not sent twice.
    last_usage: Option<(u64, u64, Option<u64>, u64)>,
}

impl Default for StreamJsonPresenter {
    fn default() -> Self {
        Self::with_writer(Box::new(std::io::stdout()))
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
            totals: ResultTotals::new(),
            message: MessageAssembler::new(false),
            init: Init::Immediate,
            held: Vec::new(),
            next_tool_id: 0,
            pending_tool_ids: VecDeque::new(),
            verbose: false,
            last_usage: None,
        }
    }

    /// `--verbose`: add the diagnostic events Claude Code's verbose stream carries (a
    /// `system/status` with `requesting` before every model call).
    pub fn with_verbose(mut self, verbose: bool) -> Self {
        self.verbose = verbose;
        self
    }

    /// Fill `usage.cache_creation_input_tokens` and `modelUsage` from the session's store (see
    /// [`UsageSlot`]); without one they come from the turn's cost events alone.
    pub fn with_usage_slot(mut self, slot: UsageSlot) -> Self {
        self.totals.set_usage_slot(slot);
        self
    }

    /// Also emit `stream_event` partials (`--include-partial-messages`).
    pub fn with_partial_messages(mut self, partials: bool) -> Self {
        self.message = MessageAssembler::new(partials);
        self
    }

    /// Defer `system/init` until the returned slot is filled with the session's metadata.
    pub fn with_deferred_init(mut self) -> (Self, StreamInitSlot) {
        let slot = StreamInitSlot::default();
        self.init = Init::Deferred(slot.clone());
        (self, slot)
    }

    fn write(&mut self, value: &Value) {
        if serde_json::to_writer(&mut self.out, value).is_ok() {
            let _ = self.out.write_all(b"\n");
            let _ = self.out.flush();
        }
    }

    /// Write one event, or hold it while a deferred `init` is still pending.
    fn line(&mut self, mut value: Value) {
        if let Some(map) = value.as_object_mut() {
            map.insert("session_id".into(), json!(self.totals.session_id()));
        }
        self.release_init(false);
        if matches!(self.init, Init::Sent | Init::Immediate) {
            self.write(&value);
        } else {
            self.held.push(value);
        }
    }

    fn release_init(&mut self, force: bool) {
        let Init::Deferred(slot) = &self.init else {
            return;
        };
        let info = match slot.take() {
            Some(info) => info,
            None if force => StreamInit::default(),
            None => return,
        };
        self.init = Init::Sent;
        let init = self.init_value(Some(info));
        self.write(&init);
        for held in std::mem::take(&mut self.held) {
            self.write(&held);
        }
    }

    fn init_value(&self, info: Option<StreamInit>) -> Value {
        let mut value = json!({
            "type": "system", "subtype": "init", "session_id": self.totals.session_id(),
            "uuid": forge_types::new_id(),
        });
        let cwd = std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        let Some(info) = info else {
            value["cwd"] = json!(cwd);
            return value;
        };
        let mcp: Vec<Value> = info
            .mcp_servers
            .iter()
            .map(|(name, status)| json!({ "name": name, "status": status }))
            .collect();
        let fields = json!({
            "cwd": if info.cwd.is_empty() { cwd } else { info.cwd },
            "model": info.model,
            "permissionMode": info.permission_mode,
            "tools": info.tools,
            "mcp_servers": mcp,
            "slash_commands": info.slash_commands,
            "skills": info.skills,
            "plugins": [],
            "output_style": "default",
            "forge_version": info.forge_version,
        });
        if let (Some(map), Some(extra)) = (value.as_object_mut(), fields.as_object()) {
            map.extend(extra.clone());
        }
        value
    }

    fn envelope(&self, kind: &str, message: Value) -> Value {
        json!({
            "type": kind, "message": message, "parent_tool_use_id": null,
            "uuid": forge_types::new_id(),
        })
    }

    fn send(&mut self, out: Vec<Out>) {
        for item in out {
            let value = match item {
                Out::Assistant(message) => self.envelope("assistant", message),
                Out::Partial(event) => json!({
                    "type": "stream_event", "event": event, "parent_tool_use_id": null,
                    "uuid": forge_types::new_id(),
                }),
            };
            self.line(value);
        }
    }

    fn system(&mut self, subtype: &str, fields: Value) {
        let mut value = json!({ "type": "system", "subtype": subtype });
        if let (Some(map), Some(extra)) = (value.as_object_mut(), fields.as_object()) {
            map.extend(extra.clone());
        }
        self.line(value);
    }

    fn finish_turn(&mut self, final_text: &str, stop_reason: StopReason) {
        let usage = self.totals.last_call_usage();
        let mut out = self.message.flush(&usage);
        // A turn whose answer never streamed (a bridge, a harness notice) still ends with the
        // answer as an assistant message, which is where Claude Code consumers read it.
        if !self.message.answered() && !final_text.trim().is_empty() {
            let message = self
                .message
                .message(vec![json!({ "type": "text", "text": final_text })], &usage);
            out.push(Out::Assistant(message));
        }
        self.send(out);
        self.release_init(true);
        let result = self.totals.result(final_text, stop_reason);
        self.line(result);
        self.totals.finish_turn();
        self.message.tool_finished();
    }
}

impl Presenter for StreamJsonPresenter {
    fn emit(&mut self, event: PresenterEvent) {
        self.totals.observe(&event);
        match event {
            PresenterEvent::SessionStarted { .. } => {
                if matches!(self.init, Init::Immediate) {
                    let init = self.init_value(None);
                    self.init = Init::Sent;
                    self.write(&init);
                }
            }
            PresenterEvent::Routing {
                tier,
                model,
                rationale,
                ..
            } => {
                self.message.set_model(&model);
                self.system(
                    "routing",
                    json!({ "tier": tier, "model": model, "rationale": rationale }),
                );
            }
            PresenterEvent::ProviderRequest { model, .. } => {
                if self.verbose {
                    self.system("status", json!({ "status": "requesting" }));
                }
                let usage = self.totals.last_call_usage();
                let out = self.message.begin_call(&model, &usage);
                self.send(out);
            }
            PresenterEvent::AssistantText(text) | PresenterEvent::AssistantDelta(text) => {
                let out = self.message.text(&text);
                self.send(out);
            }
            PresenterEvent::Reasoning(delta) => {
                let out = self.message.thinking(&delta);
                self.send(out);
            }
            PresenterEvent::ToolStart { name, args } => {
                // `args` is a JSON string; embed the parsed value when possible (CC `tool_use.input`).
                let input = serde_json::from_str::<Value>(&args).unwrap_or(Value::String(args));
                self.next_tool_id += 1;
                let id = format!("toolu_{:04}", self.next_tool_id);
                self.pending_tool_ids.push_back((name.clone(), id.clone()));
                let block = json!({ "type": "tool_use", "id": id, "name": name, "input": input });
                let usage = self.totals.last_call_usage();
                let out = self.message.tool_use(block, &usage);
                self.send(out);
            }
            PresenterEvent::ToolResult {
                name,
                ok,
                summary,
                detail,
            } => {
                let at = self.pending_tool_ids.iter().position(|(n, _)| *n == name);
                let id = at
                    .and_then(|i| self.pending_tool_ids.remove(i))
                    .map(|(_, id)| id);
                let content = detail.filter(|d| !d.trim().is_empty()).unwrap_or(summary);
                let message = json!({ "role": "user", "content": [ {
                    "type": "tool_result", "tool_use_id": id, "tool_name": name,
                    "is_error": !ok, "content": content
                } ] });
                let value = self.envelope("user", message);
                self.line(value);
                self.message.tool_finished();
            }
            PresenterEvent::ToolOutput { name, output } => self.system(
                "tool_output",
                json!({ "tool_name": name, "path": output.path, "lines": output.lines,
                        "bytes": output.bytes }),
            ),
            PresenterEvent::Cost {
                session_total_usd,
                session_in,
                session_cached_in,
                session_out,
                ..
            } => {
                let usage = self.totals.last_call_usage();
                let out = self.message.flush(&usage);
                self.send(out);
                // A model call's cost and the end of its turn both report the same running total.
                let key = (
                    session_total_usd.to_bits(),
                    session_in,
                    session_cached_in,
                    session_out,
                );
                if self.last_usage.replace(key) != Some(key) {
                    self.system(
                        "usage",
                        json!({
                            "total_cost_usd": session_total_usd,
                            "usage": { "input_tokens": session_in,
                                       "cached_input_tokens": session_cached_in,
                                       "output_tokens": session_out }
                        }),
                    );
                }
            }
            PresenterEvent::Warning(msg) => self.system("warning", json!({ "message": msg })),
            PresenterEvent::Error(msg) => self.system("error", json!({ "message": msg })),
            PresenterEvent::Done {
                final_text,
                stop_reason,
            } => self.finish_turn(&final_text, stop_reason),
            // Other events are not part of the CC stream-json surface; intentionally ignored.
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

impl Drop for StreamJsonPresenter {
    /// A run that died between session start and its first turn still owes its consumer the
    /// `init` and whatever was held behind it.
    fn drop(&mut self) {
        if matches!(self.init, Init::Deferred(_)) && !self.totals.session_id().is_empty() {
            self.release_init(true);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Default)]
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

    fn run(verbose: bool) -> Vec<Value> {
        let buf = Buf::default();
        let mut p = StreamJsonPresenter::with_writer(Box::new(buf.clone())).with_verbose(verbose);
        for e in [
            PresenterEvent::SessionStarted { id: "s-1".into() },
            PresenterEvent::Routing {
                effort: None,
                tier: "trivial".into(),
                model: "m".into(),
                rationale: "r".into(),
            },
            PresenterEvent::ProviderRequest {
                model: "m".into(),
                step: 0,
            },
            PresenterEvent::AssistantText("ok".into()),
            cost(),
            cost(),
            PresenterEvent::Done {
                final_text: "ok".into(),
                stop_reason: StopReason::FinalAnswer,
            },
        ] {
            p.emit(e);
        }
        let text = String::from_utf8(buf.0.lock().unwrap().clone()).unwrap();
        text.lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    fn kinds(events: &[Value]) -> Vec<String> {
        events
            .iter()
            .map(|e| {
                format!(
                    "{}/{}",
                    e["type"].as_str().unwrap(),
                    e["subtype"].as_str().unwrap_or("")
                )
            })
            .collect()
    }

    #[test]
    fn verbose_adds_claude_codes_status_events_and_nothing_else() {
        let quiet = kinds(&run(false));
        let verbose = kinds(&run(true));
        assert!(!quiet.contains(&"system/status".to_string()), "{quiet:?}");
        assert_eq!(verbose.iter().filter(|k| *k == "system/status").count(), 1);
        let without_status: Vec<_> = verbose
            .into_iter()
            .filter(|k| k != "system/status")
            .collect();
        assert_eq!(without_status, quiet);
    }

    #[test]
    fn an_unchanged_running_total_is_reported_once() {
        let events = run(false);
        let usage = events.iter().filter(|e| e["subtype"] == "usage").count();
        assert_eq!(usage, 1, "{:?}", kinds(&events));
        assert_eq!(events.last().unwrap()["type"], "result");
    }
}
