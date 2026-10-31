//! Schema-validated child results (`[harness] subagent_output_schema`).
//!
//! The parent attaches an `output_schema` to a `spawn_agents` child. The child gets a virtual
//! `submit_result` tool whose parameters are that schema; every submission is checked with
//! `forge_tools::schema_check`, and an invalid one is sent back with path-precise errors. A batch
//! that asked for a schema returns one JSON envelope instead of labelled prose. Everything here is
//! inert while the flag is off.

use forge_config::HarnessConfig;
use forge_provider::ToolSpec;
use forge_tools::schema_check::{self, SchemaError};
use serde_json::{json, Value};

use super::{AgentRequest, SubagentOutcome};

pub(crate) const SUBMIT_RESULT_TOOL: &str = "submit_result";

const SCHEMA_RETRIES: u32 = 2;
const MAX_RESULT_BYTES: usize = 16384;
const MAX_ENVELOPE_BYTES: usize = 65536;
const RAW_ECHO_BYTES: usize = 1024;
const MAX_ERRORS_SHOWN: usize = 8;
const PREVIEW_CHARS: usize = 78;

const SPEC_NOTE: &str = "\nGive `output_schema` (JSON Schema with root type object; supported keywords: type, properties, required, additionalProperties, items, enum) when you need data back instead of prose. A top-level `output_schema` applies to every agent without its own. This call then returns JSON {\"agents\":[{\"status\":\"valid|partial|invalid|text|error\",\"data\":{...},\"errors\":[...]}]}. Prefer flat schemas with `required`.";
const SUBMIT_DESC: &str = "Return your final result to the parent agent. Arguments must match this schema. Call it once, when finished; the call ends your task.";
const CHILD_CONTRACT: &str = "\n\nYour parent needs a structured result. When the task is done, call `submit_result` once with arguments matching its schema; that call is your answer and ends your work. Use null or empty values where the schema allows for anything you could not determine. If submit_result returns errors, fix exactly those fields and call it again.";
const MISSING_SUBMIT_NUDGE: &str = "You ended without calling submit_result. Call it now with your result; its arguments must match the schema.";
pub(crate) const SKIPPED_AFTER_ACCEPT: &str = "skipped: result already submitted";
pub(crate) const SKIPPED_EXTRA_SUBMIT: &str =
    "skipped: only one submit_result per reply is checked";
pub(crate) const WRAP_UP_ONLY: &str = "error: only submit_result is available now";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResultStatus {
    Valid,
    Partial,
    Invalid,
    Text,
    Error,
}

impl ResultStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Valid => "valid",
            Self::Partial => "partial",
            Self::Invalid => "invalid",
            Self::Text => "text",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct StructuredResult {
    pub status: ResultStatus,
    pub data: Option<Value>,
    pub errors: Vec<String>,
    pub raw: Option<String>,
    pub text: Option<String>,
    pub attempts: u32,
    pub notes: Vec<String>,
    reason: Option<WrapReason>,
}

impl StructuredResult {
    fn bare(status: ResultStatus) -> Self {
        Self {
            status,
            data: None,
            errors: Vec::new(),
            raw: None,
            text: None,
            attempts: 0,
            notes: Vec::new(),
            reason: None,
        }
    }

    /// A child that ran as a normal text child although a schema was requested.
    pub(crate) fn text(text: &str, note: String) -> Self {
        Self {
            text: Some(text.to_string()),
            notes: vec![note],
            ..Self::bare(ResultStatus::Text)
        }
    }

    pub(crate) fn error(message: &str) -> Self {
        Self {
            errors: vec![message.to_string()],
            ..Self::bare(ResultStatus::Error)
        }
    }

    pub(crate) fn text_only(text: &str) -> Self {
        Self {
            text: Some(text.to_string()),
            ..Self::bare(ResultStatus::Text)
        }
    }

    /// What `final_text` consumers (workflow digest, duel, skill_fork) see.
    fn final_text(&self) -> String {
        match (&self.data, self.errors.first()) {
            (Some(data), _) => data.to_string(),
            (None, Some(first)) => format!("error: {first}"),
            _ => self.text.clone().unwrap_or_default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WrapReason {
    StepLimit,
    Stalled,
}

impl WrapReason {
    fn phrase(self) -> &'static str {
        match self {
            Self::StepLimit => "step budget is used up",
            Self::Stalled => "work stopped making progress",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::StepLimit => "step budget",
            Self::Stalled => "progress guard",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WrapUp {
    Idle,
    Armed(WrapReason),
    Spent(WrapReason),
}

pub(crate) enum Submit {
    Accepted,
    Rejected(String),
    Exhausted(String),
}

/// `wants_schema[i]` for each request. With the flag on, every requested schema must be in the
/// supported subset; the error names the agent so the parent can fix it before any child exists.
pub(crate) fn preflight(
    cfg: &HarnessConfig,
    requests: &[AgentRequest],
) -> Result<Vec<bool>, String> {
    if !cfg.subagent_output_schema {
        return Ok(vec![false; requests.len()]);
    }
    let mut wants = Vec::with_capacity(requests.len());
    for (i, req) in requests.iter().enumerate() {
        match &req.output_schema {
            Some(schema) => {
                schema_check::check_supported(schema).map_err(|e| {
                    format!(
                        "error: output_schema for agent {} is not supported: {e}",
                        i + 1
                    )
                })?;
                wants.push(true);
            }
            None => wants.push(false),
        }
    }
    Ok(wants)
}

/// Add `output_schema` to a `spawn_agents` spec (per agent and as a batch default).
pub(crate) fn advertise_output_schema(spec: &mut ToolSpec) {
    let prop = json!({
        "type": "object",
        "description": "optional JSON Schema for the result (root type object)"
    });
    if let Some(item_props) = spec
        .schema
        .pointer_mut("/properties/agents/items/properties")
        .and_then(Value::as_object_mut)
    {
        item_props.insert("output_schema".into(), prop.clone());
    }
    if let Some(props) = spec
        .schema
        .get_mut("properties")
        .and_then(Value::as_object_mut)
    {
        props.insert("output_schema".into(), prop);
    }
    spec.description.push_str(SPEC_NOTE);
}

/// A child's contract with its parent: one schema, a retry budget, and what has been seen.
pub(crate) struct Contract {
    schema: Value,
    retries_left: u32,
    attempts: u32,
    last_errors: Vec<String>,
    last_raw: Option<String>,
    accepted: Option<Value>,
    wrap: WrapUp,
}

impl Contract {
    /// `None` for a bridged model: it cannot reliably receive a virtual tool.
    fn new(schema: &Value, model: &str) -> Option<Self> {
        if forge_provider::is_cli_bridge(model) {
            return None;
        }
        Some(Self {
            schema: schema.clone(),
            retries_left: SCHEMA_RETRIES,
            attempts: 0,
            last_errors: Vec::new(),
            last_raw: None,
            accepted: None,
            wrap: WrapUp::Idle,
        })
    }

    pub(crate) fn tool_spec(&self) -> ToolSpec {
        ToolSpec {
            name: SUBMIT_RESULT_TOOL.to_string(),
            description: SUBMIT_DESC.to_string(),
            schema: self.schema.clone(),
        }
    }

    pub(crate) fn wrap_armed(&self) -> bool {
        matches!(self.wrap, WrapUp::Armed(_))
    }

    pub(crate) fn wrap_spent(&self) -> bool {
        matches!(self.wrap, WrapUp::Spent(_))
    }

    /// Allow exactly one more model call that can only submit. False if not applicable.
    pub(crate) fn arm_wrap_up(&mut self, reason: WrapReason) -> bool {
        if self.accepted.is_some() || self.wrap != WrapUp::Idle {
            return false;
        }
        self.wrap = WrapUp::Armed(reason);
        true
    }

    pub(crate) fn spend_wrap_up(&mut self) {
        if let WrapUp::Armed(r) = self.wrap {
            self.wrap = WrapUp::Spent(r);
        }
    }

    pub(crate) fn wrap_up_text(&self) -> String {
        let reason = match self.wrap {
            WrapUp::Armed(r) | WrapUp::Spent(r) => r.phrase(),
            WrapUp::Idle => "work is over",
        };
        format!(
            "Your {reason}. Call submit_result now with the best result you have; use null or empty values for anything unfinished. It is the only tool available."
        )
    }

    pub(crate) fn on_submit(&mut self, args: &Value) -> Submit {
        self.attempts += 1;
        let normalized = forge_tools::normalize_args(&self.schema, args);
        let value = schema_check::coerce(&self.schema, &normalized);
        self.last_raw = Some(args.to_string());
        self.judge(value)
    }

    pub(crate) fn on_text(&mut self, text: &str) -> Submit {
        self.last_raw = Some(text.to_string());
        match extract_json(text) {
            Some(found) => {
                self.attempts += 1;
                let value = schema_check::coerce(&self.schema, &found);
                self.judge(value)
            }
            None => self.reject(vec!["no JSON object in the reply".to_string()], false),
        }
    }

    fn judge(&mut self, value: Value) -> Submit {
        let mut errors: Vec<String> =
            schema_check::validate(&self.schema, &value, MAX_ERRORS_SHOWN)
                .into_iter()
                .map(|SchemaError { path, message }| format!("{path}: {message}"))
                .collect();
        if errors.is_empty() && value.to_string().len() > MAX_RESULT_BYTES {
            errors.push(format!(
                "$: result is larger than {MAX_RESULT_BYTES} bytes; return less"
            ));
        }
        if errors.is_empty() {
            self.accepted = Some(value);
            return Submit::Accepted;
        }
        self.reject(errors, true)
    }

    fn reject(&mut self, errors: Vec<String>, listed: bool) -> Submit {
        self.last_errors = errors.clone();
        let no_retry = self.retries_left == 0 || self.wrap_spent();
        let body = if listed {
            let lines: Vec<String> = errors.iter().map(|e| format!("- {e}")).collect();
            format!(
                "error: submit_result rejected, {} problem(s):\n{}",
                errors.len(),
                lines.join("\n")
            )
        } else {
            MISSING_SUBMIT_NUDGE.to_string()
        };
        if no_retry {
            return Submit::Exhausted(body);
        }
        self.retries_left -= 1;
        if listed {
            Submit::Rejected(format!(
                "{body}\nCall submit_result again with these fixed. Attempts left: {}.",
                self.retries_left + 1
            ))
        } else {
            Submit::Rejected(body)
        }
    }

    pub(crate) fn into_result(self, note: Option<String>) -> StructuredResult {
        let mut out = match self.accepted {
            Some(data) => {
                let (status, reason) = match self.wrap {
                    WrapUp::Spent(r) => (ResultStatus::Partial, Some(r)),
                    _ => (ResultStatus::Valid, None),
                };
                StructuredResult {
                    data: Some(data),
                    reason,
                    ..StructuredResult::bare(status)
                }
            }
            None => {
                let mut errors = self.last_errors;
                errors.truncate(MAX_ERRORS_SHOWN);
                if errors.is_empty() {
                    errors.push("the agent did not submit a result".to_string());
                }
                let raw = self
                    .last_raw
                    .filter(|r| !r.trim().is_empty())
                    .map(|r| truncate_bytes(&r, RAW_ECHO_BYTES));
                StructuredResult {
                    errors,
                    raw,
                    ..StructuredResult::bare(ResultStatus::Invalid)
                }
            }
        };
        out.attempts = self.attempts;
        out.notes.extend(note);
        out
    }
}

/// Wire a contract onto a child's specs and system prompt. A bridged model, a model that failed
/// over to one, or an unsupported schema leaves the child a plain text child with a note.
pub(super) fn attach(
    schema: Option<&Value>,
    enabled: bool,
    model: &str,
    specs: &mut Vec<ToolSpec>,
    system: &mut forge_types::Message,
) -> (Option<Contract>, Option<String>) {
    let Some(schema) = schema.filter(|_| enabled) else {
        return (None, None);
    };
    if let Err(e) = schema_check::check_supported(schema) {
        return (None, Some(format!("schema not enforced: {e}")));
    }
    match Contract::new(schema, model) {
        Some(c) => {
            specs.push(c.tool_spec());
            system.content.push_str(CHILD_CONTRACT);
            (Some(c), None)
        }
        None => (
            None,
            Some(format!("schema not enforced: {model} is a bridged model")),
        ),
    }
}

/// A child whose active model became a bridged model can no longer be handed `submit_result`.
pub(super) fn drop_for_bridge(
    contract: &mut Option<Contract>,
    specs: &mut Vec<ToolSpec>,
    model: &str,
) -> Option<String> {
    if contract.is_some() && forge_provider::is_cli_bridge(model) {
        *contract = None;
        specs.retain(|s| s.name != SUBMIT_RESULT_TOOL);
        return Some(format!("schema not enforced after failover to {model}"));
    }
    None
}

/// Check one `submit_result` call, record it like an unavailable-tool call (it has no side effect,
/// so the permission broker does not apply), and return the tool result plus whether it ended the
/// child's task.
pub(super) fn run_submit(
    contract: &mut Contract,
    store: &forge_store::Store,
    msg_id: &str,
    call: &forge_types::ToolCall,
) -> Result<(String, bool), crate::CoreError> {
    let args_json = serde_json::to_string(&call.args)?;
    let (text, done, status) = match contract.on_submit(&call.args) {
        Submit::Accepted => ("accepted".to_string(), true, "ok"),
        Submit::Rejected(message) => (message, false, "error"),
        Submit::Exhausted(message) => (message, true, "error"),
    };
    store.record_tool_call(msg_id, &call.name, &args_json, &text, "n/a", status)?;
    Ok((text, done))
}

/// Tell the child its budget is gone and only `submit_result` is left. Appended to the last tool
/// result in memory (the nudge convention); otherwise a persisted user message.
pub(super) fn inject_wrap_up(
    transcript: &mut Vec<forge_types::Message>,
    store: &forge_store::Store,
    child_id: &str,
    next_seq: &mut dyn FnMut() -> i64,
    text: &str,
) -> Result<(), crate::CoreError> {
    match transcript.last_mut() {
        Some(last) if last.role == forge_types::Role::Tool => {
            last.content.push_str(&format!("\n\n{text}"));
        }
        _ => push_nudge(transcript, store, child_id, next_seq(), text)?,
    }
    Ok(())
}

/// A worktree merge problem: a note on a structured result (forcing nothing about its status), or
/// a line of the text for a plain child.
pub(super) fn note_or_text(text: &mut String, result: &mut Option<StructuredResult>, line: String) {
    match result {
        Some(r) => r
            .notes
            .push(line.trim().trim_matches(['[', ']']).to_string()),
        None => text.push_str(&line),
    }
}

/// The envelope row for one child. A child that returned no structured result is a text row, or
/// an error row when its text is an error.
pub(super) fn row_for(
    index: usize,
    agent: String,
    text: &str,
    result: Option<StructuredResult>,
) -> Row {
    let result = result.unwrap_or_else(|| match text.strip_prefix("error: ") {
        Some(message) => StructuredResult::error(message),
        None => StructuredResult::text_only(text),
    });
    Row {
        index,
        agent,
        result,
    }
}

/// The `submit_result`-only spec for the wrap-up call, once armed (empty otherwise), after telling
/// the child why.
pub(super) fn wrap_up_specs(
    contract: Option<&Contract>,
    transcript: &mut Vec<forge_types::Message>,
    store: &forge_store::Store,
    child_id: &str,
    next_seq: &mut dyn FnMut() -> i64,
) -> Result<Vec<ToolSpec>, crate::CoreError> {
    let Some(c) = contract.filter(|c| c.wrap_armed()) else {
        return Ok(Vec::new());
    };
    inject_wrap_up(transcript, store, child_id, next_seq, &c.wrap_up_text())?;
    Ok(vec![c.tool_spec()])
}

/// A corrective user turn after a reply that never called `submit_result`.
pub(super) fn push_nudge(
    transcript: &mut Vec<forge_types::Message>,
    store: &forge_store::Store,
    child_id: &str,
    seq: i64,
    text: &str,
) -> Result<(), crate::CoreError> {
    transcript.push(forge_types::Message::user(text));
    store.add_message_kind(
        child_id,
        seq,
        forge_types::Role::User,
        text,
        forge_types::MessageKind::Nudge,
    )?;
    Ok(())
}

/// What a step looks like to [`intercept`].
pub(super) struct Turn<'a> {
    pub msg_id: &'a str,
    pub wrapping: bool,
    pub finished: bool,
    pub submit_seen: &'a mut bool,
}

/// The tool result for a call the contract answers itself: `submit_result` is checked, anything
/// after the task ended is skipped, and during the wrap-up only `submit_result` exists. `None`
/// means the call runs as a normal tool. The bool is whether the task is over.
pub(super) fn intercept(
    contract: &mut Contract,
    store: &forge_store::Store,
    call: &forge_types::ToolCall,
    turn: Turn<'_>,
) -> Result<Option<(String, bool)>, crate::CoreError> {
    Ok(if call.name == SUBMIT_RESULT_TOOL {
        if std::mem::replace(turn.submit_seen, true) {
            Some((SKIPPED_EXTRA_SUBMIT.to_string(), false))
        } else {
            Some(run_submit(contract, store, turn.msg_id, call)?)
        }
    } else if turn.finished {
        Some((SKIPPED_AFTER_ACCEPT.to_string(), false))
    } else if turn.wrapping {
        Some((WRAP_UP_ONLY.to_string(), false))
    } else {
        None
    })
}

pub(super) fn outcome_from(contract: Contract, note: Option<String>) -> SubagentOutcome {
    let result = contract.into_result(note);
    SubagentOutcome {
        final_text: result.final_text(),
        ok: result.status == ResultStatus::Valid,
        structured: Some(result),
    }
}

pub(super) fn with_note(mut outcome: SubagentOutcome, note: Option<String>) -> SubagentOutcome {
    if let Some(note) = note {
        outcome.structured = Some(StructuredResult::text(&outcome.final_text, note));
    }
    outcome
}

fn truncate_bytes(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

/// The JSON object in a reply: the whole trimmed text, a fenced block, or `{` to the last `}`.
pub(crate) fn extract_json(text: &str) -> Option<Value> {
    let as_object = |s: &str| {
        serde_json::from_str::<Value>(s.trim())
            .ok()
            .filter(Value::is_object)
    };
    if let Some(v) = as_object(text) {
        return Some(v);
    }
    if let Some(start) = text.find("```") {
        let after = &text[start + 3..];
        let body = after.strip_prefix("json").unwrap_or(after);
        if let Some(end) = body.find("```") {
            if let Some(v) = as_object(&body[..end]) {
                return Some(v);
            }
        }
    }
    let (first, last) = (text.find('{')?, text.rfind('}')?);
    (first < last)
        .then(|| as_object(&text[first..=last]))
        .flatten()
}

/// One line for the live panel: at most 78 chars.
pub(crate) fn preview(result: &StructuredResult) -> String {
    let line = match result.status {
        ResultStatus::Valid => format!("✓ valid · {}", data_glance(result.data.as_ref())),
        ResultStatus::Partial => format!(
            "◐ partial ({}) · {}",
            result.reason.map_or("wrap-up", WrapReason::label),
            data_glance(result.data.as_ref())
        ),
        ResultStatus::Invalid => format!(
            "✗ invalid ×{} · {}",
            result.attempts.max(1),
            result.errors.first().map_or("no result", String::as_str)
        ),
        ResultStatus::Text => format!(
            "text · {}",
            result
                .notes
                .first()
                .or(result.text.as_ref())
                .map_or("", String::as_str)
        ),
        ResultStatus::Error => format!(
            "✗ error · {}",
            result.errors.first().map_or("", String::as_str)
        ),
    };
    let one_line = line.lines().next().unwrap_or("");
    one_line.chars().take(PREVIEW_CHARS).collect()
}

fn data_glance(data: Option<&Value>) -> String {
    let Some(Value::Object(map)) = data else {
        return String::new();
    };
    map.iter()
        .map(|(k, v)| match v {
            Value::String(s) => {
                let shown: String = s.chars().take(16).collect();
                format!("{k}={shown:?}")
            }
            Value::Array(a) => format!("{k}[{}]", a.len()),
            Value::Object(_) => format!("{k}{{}}"),
            other => format!("{k}={other}"),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub(crate) struct Row {
    pub index: usize,
    pub agent: String,
    pub result: StructuredResult,
}

fn render_row(row: &Row, truncated: Option<&str>) -> String {
    let r = &row.result;
    let mut s = format!(
        "{{\"index\":{},\"agent\":{},\"status\":\"{}\",\"attempts\":{}",
        row.index,
        json!(row.agent),
        r.status.as_str(),
        r.attempts
    );
    let mut field = |key: &str, value: Value| {
        s.push_str(&format!(",\"{key}\":{value}"));
    };
    match (truncated, &r.data) {
        (Some(head), _) => field("data_truncated", json!(head)),
        (None, Some(data)) => field("data", data.clone()),
        _ => {}
    }
    if !r.errors.is_empty() {
        field("errors", json!(r.errors));
    }
    if let Some(raw) = &r.raw {
        field("raw", json!(raw));
    }
    if let Some(text) = &r.text {
        field("text", json!(text));
    }
    if !r.notes.is_empty() {
        field("notes", json!(r.notes));
    }
    s.push('}');
    s
}

/// The batch result: compact JSON, one row per line. Over 64 KB the largest `data` payloads are
/// replaced by their first 1 KB.
pub(crate) fn envelope(rows: &[Row]) -> String {
    let mut cut: Vec<Option<String>> = vec![None; rows.len()];
    let mut notes: Vec<String> = Vec::new();
    loop {
        let rendered: Vec<String> = rows
            .iter()
            .zip(&cut)
            .map(|(row, c)| render_row(row, c.as_deref()))
            .collect();
        let total: usize = rendered.iter().map(|r| r.len() + 2).sum();
        let biggest = rows
            .iter()
            .enumerate()
            .filter(|(i, row)| cut[*i].is_none() && row.result.data.is_some())
            .max_by_key(|(_, row)| row.result.data.as_ref().map_or(0, |d| d.to_string().len()))
            .map(|(i, _)| i);
        match biggest {
            Some(i) if total > MAX_ENVELOPE_BYTES => {
                let full = rows[i].result.data.as_ref().map(Value::to_string);
                cut[i] = full.map(|f| truncate_bytes(&f, RAW_ECHO_BYTES));
                notes.push(format!(
                    "agent {}: data truncated; full result in the child session (Ctrl+O)",
                    rows[i].index
                ));
            }
            _ => {
                return format!(
                    "{{\"agents\":[\n{}\n],\"notes\":{}}}",
                    rendered.join(",\n"),
                    json!(notes)
                );
            }
        }
    }
}

pub(crate) fn is_envelope(text: &str) -> bool {
    text.starts_with("{\"agents\":[\n") && text.ends_with("]}")
}

/// Append a batch-level note while keeping the envelope valid JSON.
pub(crate) fn add_note(envelope: &str, note: &str) -> String {
    let Some(body) = envelope.strip_suffix("]}") else {
        return format!("{envelope}\n\n{note}");
    };
    let sep = if body.ends_with('[') { "" } else { "," };
    format!("{body}{sep}{}]}}", json!(note))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(index: usize, result: StructuredResult) -> Row {
        Row {
            index,
            agent: "explore".into(),
            result,
        }
    }

    fn valid(data: Value) -> StructuredResult {
        StructuredResult {
            data: Some(data),
            attempts: 1,
            ..StructuredResult::bare(ResultStatus::Valid)
        }
    }

    #[test]
    fn extract_json_shapes() {
        assert!(extract_json(r#"{"a":1}"#).is_some());
        assert!(extract_json("here:\n```json\n{\"a\":1}\n```\nbye").is_some());
        assert!(extract_json("Result is {\"a\": 2} ok").is_some());
        assert!(extract_json("no json here").is_none());
        assert!(extract_json("[1,2]").is_none());
    }

    #[test]
    fn preview_is_one_short_line() {
        let long = valid(json!({"verdict":"pass","files":[1,2,3],"note":"x".repeat(500)}));
        let p = preview(&long);
        assert!(p.chars().count() <= 78, "{p}");
        assert!(p.starts_with("✓ valid · "), "{p}");
        assert!(
            p.contains("files[3]") && p.contains("verdict=\"pass\""),
            "{p}"
        );
        let partial = StructuredResult {
            reason: Some(WrapReason::StepLimit),
            ..valid(json!({"files":[1,2]}))
        };
        let partial = StructuredResult {
            status: ResultStatus::Partial,
            ..partial
        };
        assert_eq!(preview(&partial), "◐ partial (step budget) · files[2]");
        let bad = StructuredResult {
            errors: vec!["$.line: expected integer, got string".into()],
            attempts: 3,
            ..StructuredResult::bare(ResultStatus::Invalid)
        };
        assert!(preview(&bad).starts_with("✗ invalid ×3 · $.line"));
    }

    #[test]
    fn envelope_mixed_batch_is_json_with_aligned_indexes() {
        let rows = vec![
            row(1, valid(json!({"v":"pass"}))),
            row(
                2,
                StructuredResult {
                    errors: vec!["$.line: expected integer".into()],
                    raw: Some("{\"line\":\"x\"}".into()),
                    attempts: 3,
                    ..StructuredResult::bare(ResultStatus::Invalid)
                },
            ),
            row(3, StructuredResult::text_only("prose")),
            row(
                4,
                StructuredResult::error("no result (the agent crashed before reporting)"),
            ),
        ];
        let text = envelope(&rows);
        let v: Value = serde_json::from_str(&text).unwrap();
        let agents = v["agents"].as_array().unwrap();
        let statuses: Vec<&str> = agents
            .iter()
            .map(|a| a["status"].as_str().unwrap())
            .collect();
        assert_eq!(statuses, ["valid", "invalid", "text", "error"]);
        for (i, a) in agents.iter().enumerate() {
            assert_eq!(a["index"], i + 1);
        }
        assert_eq!(text.lines().count(), 6);
    }

    #[test]
    fn envelope_over_64kb_truncates_largest_rows() {
        let big = |n: usize| valid(json!({"blob": "z".repeat(n)}));
        let rows = vec![row(1, big(40_000)), row(2, big(30_000)), row(3, big(10))];
        let text = envelope(&rows);
        assert!(text.len() <= MAX_ENVELOPE_BYTES + 2048, "{}", text.len());
        let v: Value = serde_json::from_str(&text).unwrap();
        assert!(v["agents"][0].get("data_truncated").is_some());
        assert!(v["agents"][2].get("data").is_some());
        assert!(!v["notes"].as_array().unwrap().is_empty());
    }

    #[test]
    fn add_note_keeps_valid_json() {
        let env = envelope(&[row(1, valid(json!({"a":1})))]);
        let once = add_note(&env, "[subagent_stop hook] blocked");
        let twice = add_note(&once, "second");
        let v: Value = serde_json::from_str(&twice).unwrap();
        assert_eq!(v["notes"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn preflight_flag_off_never_errors() {
        let req = AgentRequest {
            output_schema: Some(json!({"$ref": "#/x"})),
            ..AgentRequest::default()
        };
        let off = HarnessConfig::default();
        assert_eq!(
            preflight(&off, std::slice::from_ref(&req)).unwrap(),
            vec![false]
        );
        let on = HarnessConfig {
            subagent_output_schema: true,
            ..HarnessConfig::default()
        };
        let err = preflight(&on, &[req]).unwrap_err();
        assert!(err.contains("agent 1"), "{err}");
        let ok = AgentRequest {
            output_schema: Some(json!({"type":"object"})),
            ..AgentRequest::default()
        };
        let none = AgentRequest::default();
        assert_eq!(preflight(&on, &[ok, none]).unwrap(), vec![true, false]);
    }

    #[test]
    fn contract_allows_exactly_three_submits() {
        let schema =
            json!({"type":"object","properties":{"n":{"type":"integer"}},"required":["n"]});
        let mut c = Contract::new(&schema, "deepseek::x").unwrap();
        assert!(
            matches!(c.on_submit(&json!({"n":"x"})), Submit::Rejected(m) if m.contains("$.n: expected integer"))
        );
        assert!(matches!(c.on_submit(&json!({})), Submit::Rejected(_)));
        assert!(matches!(
            c.on_submit(&json!({"n":"y"})),
            Submit::Exhausted(_)
        ));
        let r = c.into_result(None);
        assert_eq!((r.status, r.attempts), (ResultStatus::Invalid, 3));
        assert!(r.raw.is_some() && !r.errors.is_empty());
    }

    #[test]
    fn contract_coerces_and_accepts() {
        let schema =
            json!({"type":"object","properties":{"n":{"type":"integer"}},"required":["n"]});
        let mut c = Contract::new(&schema, "deepseek::x").unwrap();
        assert!(matches!(c.on_submit(&json!({"n":"7"})), Submit::Accepted));
        let r = c.into_result(None);
        assert_eq!(r.status, ResultStatus::Valid);
        assert_eq!(r.data, Some(json!({"n":7})));
    }
}
