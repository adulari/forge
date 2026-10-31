//! MCP elicitation: a server asking the user for input in the middle of a tool call.
//!
//! The client handler ([`crate::ForgeClientHandler`]) receives `elicitation/create` on the
//! connection's event loop, where no presenter is reachable. This module bridges the two halves:
//!
//! - the host opens an [`ElicitScope`] around an MCP tool call and drains the returned receiver
//!   while the call is in flight, putting each [`ElicitRequest`] to the user;
//! - the handler routes a request to the newest open scope that covers its server. With no scope
//!   open (a background reconnect, a headless run) the request is declined rather than parked.
//!
//! The JSON schema a server sends is mapped to the same question form `ask_user` uses
//! ([`ElicitForm::questions`]) and the answers are coerced back to typed JSON values
//! ([`ElicitForm::resolve`]): select, multi-select, bool, int and number with bounds, masked
//! strings for secrets, and a URL step.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use forge_types::{Answer, QChoice, Question};
use parking_lot::Mutex;
use serde_json::{Map, Value};
use tokio::sync::{mpsc, oneshot};

/// What the user did with an elicitation.
#[derive(Debug, Clone, PartialEq)]
pub enum ElicitReply {
    /// Submitted: the typed field values (empty for a URL step the user agreed to).
    Accept(Map<String, Value>),
    /// Refused to provide the input; the server's call carries on.
    Decline,
    /// Abandoned the whole operation.
    Cancel,
}

/// One request from a server, waiting for the host to answer on `reply`.
pub struct ElicitRequest {
    pub server: String,
    pub message: String,
    pub prompt: ElicitPrompt,
    pub reply: oneshot::Sender<ElicitReply>,
}

/// The two elicitation modes of the MCP spec.
#[derive(Debug, Clone, PartialEq)]
pub enum ElicitPrompt {
    Form(ElicitForm),
    /// The user finishes the step in a browser: Forge shows the address and asks for consent.
    Url(String),
}

/// A requested schema, one [`ElicitField`] per property in the server's order.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ElicitForm {
    pub fields: Vec<ElicitField>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ElicitField {
    pub key: String,
    pub title: String,
    pub description: Option<String>,
    pub required: bool,
    pub kind: FieldKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FieldKind {
    Text {
        secret: bool,
        min_len: Option<usize>,
        max_len: Option<usize>,
    },
    /// `(value, label)` pairs; `multi` for an array of enum values.
    Choice {
        options: Vec<(String, String)>,
        multi: bool,
    },
    Bool,
    Integer {
        min: Option<i64>,
        max: Option<i64>,
    },
    Number {
        min: Option<f64>,
        max: Option<f64>,
    },
}

fn looks_secret(key: &str, title: &str) -> bool {
    let hay = format!("{key} {title}").to_lowercase();
    [
        "password",
        "passwd",
        "secret",
        "token",
        "api key",
        "api_key",
        "apikey",
        "passphrase",
    ]
    .iter()
    .any(|needle| hay.contains(needle))
}

fn as_usize(v: Option<&Value>) -> Option<usize> {
    v.and_then(Value::as_u64).map(|n| n as usize)
}

/// `(value, label)` options from `enum` + `enumNames`, or from `oneOf`/`anyOf` of `{const, title}`.
fn choice_options(schema: &Value) -> Option<Vec<(String, String)>> {
    if let Some(values) = schema.get("enum").and_then(Value::as_array) {
        let names = schema.get("enumNames").and_then(Value::as_array);
        return Some(
            values
                .iter()
                .enumerate()
                .map(|(i, v)| {
                    let value = v.as_str().map_or_else(|| v.to_string(), str::to_string);
                    let label = names
                        .and_then(|n| n.get(i))
                        .and_then(Value::as_str)
                        .map_or_else(|| value.clone(), str::to_string);
                    (value, label)
                })
                .collect(),
        );
    }
    for key in ["oneOf", "anyOf"] {
        if let Some(items) = schema.get(key).and_then(Value::as_array) {
            let opts: Vec<(String, String)> = items
                .iter()
                .filter_map(|item| {
                    let value = item.get("const")?;
                    let value = value
                        .as_str()
                        .map_or_else(|| value.to_string(), str::to_string);
                    let label = item
                        .get("title")
                        .and_then(Value::as_str)
                        .map_or_else(|| value.clone(), str::to_string);
                    Some((value, label))
                })
                .collect();
            if !opts.is_empty() {
                return Some(opts);
            }
        }
    }
    None
}

fn field_kind(key: &str, title: &str, schema: &Value) -> FieldKind {
    let ty = schema
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("string");
    match ty {
        "boolean" => FieldKind::Bool,
        "integer" => FieldKind::Integer {
            min: schema.get("minimum").and_then(Value::as_i64),
            max: schema.get("maximum").and_then(Value::as_i64),
        },
        "number" => FieldKind::Number {
            min: schema.get("minimum").and_then(Value::as_f64),
            max: schema.get("maximum").and_then(Value::as_f64),
        },
        "array" => {
            let items = schema.get("items").unwrap_or(&Value::Null);
            FieldKind::Choice {
                options: choice_options(items).unwrap_or_default(),
                multi: true,
            }
        }
        _ => match choice_options(schema) {
            Some(options) => FieldKind::Choice {
                options,
                multi: false,
            },
            None => FieldKind::Text {
                secret: schema.get("format").and_then(Value::as_str) == Some("password")
                    || schema.get("writeOnly").and_then(Value::as_bool) == Some(true)
                    || looks_secret(key, title),
                min_len: as_usize(schema.get("minLength")),
                max_len: as_usize(schema.get("maxLength")),
            },
        },
    }
}

impl ElicitForm {
    /// Map a requested JSON schema (`{type: object, properties, required}`) to fields.
    pub fn from_schema(schema: &Value) -> Self {
        let required: Vec<&str> = schema
            .get("required")
            .and_then(Value::as_array)
            .map(|r| r.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        let fields = schema
            .get("properties")
            .and_then(Value::as_object)
            .map(|props| {
                props
                    .iter()
                    .map(|(key, prop)| {
                        let title = prop
                            .get("title")
                            .and_then(Value::as_str)
                            .unwrap_or(key)
                            .to_string();
                        ElicitField {
                            key: key.clone(),
                            kind: field_kind(key, &title, prop),
                            description: prop
                                .get("description")
                                .and_then(Value::as_str)
                                .map(str::to_string),
                            required: required.contains(&key.as_str()),
                            title,
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        Self { fields }
    }

    /// The form as `ask_user` questions. `errors[i]` (from a previous [`Self::resolve`]) is
    /// prefixed to question `i` so a rejected value is re-asked with the reason.
    pub fn questions(&self, errors: &[Option<String>]) -> Vec<Question> {
        self.fields
            .iter()
            .enumerate()
            .map(|(i, field)| {
                let mut text = field.title.clone();
                if let Some(desc) = field.description.as_deref().filter(|d| !d.is_empty()) {
                    text.push_str(" - ");
                    text.push_str(desc);
                }
                let hint = match &field.kind {
                    FieldKind::Integer { min, max } => bounds_hint(min, max),
                    FieldKind::Number { min, max } => bounds_hint(min, max),
                    FieldKind::Text {
                        min_len, max_len, ..
                    } => length_hint(*min_len, *max_len),
                    _ => String::new(),
                };
                text.push_str(&hint);
                if !field.required {
                    text.push_str(" (optional)");
                }
                if let Some(Some(err)) = errors.get(i) {
                    text = format!("{err}. {text}");
                }
                let (options, multi, allow_other, secret) = match &field.kind {
                    FieldKind::Choice { options, multi } => (
                        options
                            .iter()
                            .map(|(_, label)| QChoice {
                                label: label.clone(),
                                description: String::new(),
                            })
                            .collect(),
                        *multi,
                        false,
                        false,
                    ),
                    FieldKind::Bool => (
                        vec![
                            QChoice {
                                label: "Yes".into(),
                                description: String::new(),
                            },
                            QChoice {
                                label: "No".into(),
                                description: String::new(),
                            },
                        ],
                        false,
                        false,
                        false,
                    ),
                    FieldKind::Text { secret, .. } => (Vec::new(), false, true, *secret),
                    FieldKind::Integer { .. } | FieldKind::Number { .. } => {
                        (Vec::new(), false, true, false)
                    }
                };
                Question {
                    header: field.title.chars().take(16).collect(),
                    text,
                    options,
                    multi,
                    allow_other,
                    allow_note: false,
                    secret,
                }
            })
            .collect()
    }

    /// Coerce the user's answers to typed values. `Err` holds one reason per field that was
    /// rejected (`None` for fields that were fine) so the caller can re-ask just those.
    pub fn resolve(&self, answers: &[Answer]) -> Result<Map<String, Value>, Vec<Option<String>>> {
        let mut values = Map::new();
        let mut errors: Vec<Option<String>> = vec![None; self.fields.len()];
        for (i, field) in self.fields.iter().enumerate() {
            let empty = Answer::default();
            let answer = answers.get(i).unwrap_or(&empty);
            match coerce(field, answer) {
                Ok(Some(v)) => {
                    values.insert(field.key.clone(), v);
                }
                Ok(None) if field.required => errors[i] = Some("This field is required".into()),
                Ok(None) => {}
                Err(e) => errors[i] = Some(e),
            }
        }
        if errors.iter().any(Option::is_some) {
            Err(errors)
        } else {
            Ok(values)
        }
    }
}

fn bounds_hint<T: std::fmt::Display>(min: &Option<T>, max: &Option<T>) -> String {
    match (min, max) {
        (Some(lo), Some(hi)) => format!(" [{lo} to {hi}]"),
        (Some(lo), None) => format!(" [at least {lo}]"),
        (None, Some(hi)) => format!(" [at most {hi}]"),
        (None, None) => String::new(),
    }
}

fn length_hint(min: Option<usize>, max: Option<usize>) -> String {
    match (min, max) {
        (Some(lo), Some(hi)) => format!(" [{lo} to {hi} characters]"),
        (Some(lo), None) => format!(" [at least {lo} characters]"),
        (None, Some(hi)) => format!(" [at most {hi} characters]"),
        (None, None) => String::new(),
    }
}

/// `Ok(None)` is "left blank".
fn coerce(field: &ElicitField, answer: &Answer) -> Result<Option<Value>, String> {
    let typed = answer.other.as_deref().map(str::trim).unwrap_or("");
    match &field.kind {
        FieldKind::Text {
            min_len, max_len, ..
        } => {
            if typed.is_empty() {
                return Ok(None);
            }
            let len = typed.chars().count();
            if min_len.is_some_and(|m| len < m) || max_len.is_some_and(|m| len > m) {
                return Err(format!(
                    "Length must be {}",
                    length_hint(*min_len, *max_len).trim()
                ));
            }
            Ok(Some(Value::String(typed.to_string())))
        }
        FieldKind::Bool => match answer.selected.first().map(String::as_str) {
            Some("Yes") => Ok(Some(Value::Bool(true))),
            Some("No") => Ok(Some(Value::Bool(false))),
            _ => Ok(None),
        },
        FieldKind::Choice { options, multi } => {
            let picked: Vec<Value> = answer
                .selected
                .iter()
                .filter_map(|label| options.iter().find(|(_, l)| l == label))
                .map(|(v, _)| Value::String(v.clone()))
                .collect();
            if picked.is_empty() {
                Ok(None)
            } else if *multi {
                Ok(Some(Value::Array(picked)))
            } else {
                Ok(picked.into_iter().next())
            }
        }
        FieldKind::Integer { min, max } => {
            if typed.is_empty() {
                return Ok(None);
            }
            let n: i64 = typed
                .parse()
                .map_err(|_| format!("'{typed}' is not a whole number"))?;
            if min.is_some_and(|m| n < m) || max.is_some_and(|m| n > m) {
                return Err(format!("{n} is out of range{}", bounds_hint(min, max)));
            }
            Ok(Some(Value::from(n)))
        }
        FieldKind::Number { min, max } => {
            if typed.is_empty() {
                return Ok(None);
            }
            let n: f64 = typed
                .parse()
                .ok()
                .filter(|n: &f64| n.is_finite())
                .ok_or_else(|| format!("'{typed}' is not a number"))?;
            if min.is_some_and(|m| n < m) || max.is_some_and(|m| n > m) {
                return Err(format!("{n} is out of range{}", bounds_hint(min, max)));
            }
            Ok(Some(Value::from(n)))
        }
    }
}

/// What a person is doing for one scope: how many questions are waiting, and when the last was
/// answered. Per scope, so one session's unanswered question never suspends another's timeout.
#[derive(Default)]
struct ScopeState {
    waiting: AtomicU64,
    last_answered: Mutex<Option<std::time::Instant>>,
}

struct Scope {
    id: u64,
    /// The manager (one session's MCP connections) that opened the scope.
    owner: u64,
    server: String,
    tx: mpsc::UnboundedSender<ElicitRequest>,
    state: Arc<ScopeState>,
}

static SCOPES: Mutex<Vec<Scope>> = Mutex::new(Vec::new());
static NEXT_SCOPE: AtomicU64 = AtomicU64::new(1);
static NEXT_OWNER: AtomicU64 = AtomicU64::new(1);

/// How long after an answer a tool call is still given the benefit of its own timeout.
const ANSWER_GRACE: std::time::Duration = std::time::Duration::from_secs(10);

/// A fresh identity for one manager's connections.
pub(crate) fn new_owner_id() -> u64 {
    NEXT_OWNER.fetch_add(1, Ordering::Relaxed)
}

/// Whether a person is, or just was, answering a question `server` asked during a call that
/// `owner` made. That call's timeout must not fire while it is parked on the person: the time
/// they take is not the server's. Nobody else's call is affected.
pub(crate) fn user_is_answering(owner: u64, server: &str) -> bool {
    SCOPES
        .lock()
        .iter()
        .filter(|s| s.owner == owner && s.server == server)
        .any(|s| {
            s.state.waiting.load(Ordering::Relaxed) > 0
                || s.state
                    .last_answered
                    .lock()
                    .is_some_and(|at| at.elapsed() < ANSWER_GRACE)
        })
}

struct WaitingGuard(Arc<ScopeState>);

impl WaitingGuard {
    fn new(state: Arc<ScopeState>) -> Self {
        state.waiting.fetch_add(1, Ordering::Relaxed);
        Self(state)
    }
}

impl Drop for WaitingGuard {
    fn drop(&mut self) {
        self.0.waiting.fetch_sub(1, Ordering::Relaxed);
        *self.0.last_answered.lock() = Some(std::time::Instant::now());
    }
}

/// Keeps a scope open; dropping it closes the scope.
pub struct ElicitScope(u64);

impl Drop for ElicitScope {
    fn drop(&mut self) {
        SCOPES.lock().retain(|s| s.id != self.0);
    }
}

/// Start receiving elicitation requests that `server` sends during a call `owner` (a manager's
/// [`new_owner_id`]) makes, until the returned guard is dropped. There is no catch-all scope: a
/// request is delivered only to a scope for the connection that sent it.
pub(crate) fn open_scope(
    owner: u64,
    server: &str,
) -> (ElicitScope, mpsc::UnboundedReceiver<ElicitRequest>) {
    let (tx, rx) = mpsc::unbounded_channel();
    let id = NEXT_SCOPE.fetch_add(1, Ordering::Relaxed);
    SCOPES.lock().push(Scope {
        id,
        owner,
        server: server.to_string(),
        tx,
        state: Arc::default(),
    });
    (ElicitScope(id), rx)
}

impl crate::McpManager {
    /// Receive the elicitation requests `server` sends during calls this manager makes, until
    /// the guard drops. Requests from any other connection never arrive here.
    pub fn elicit_scope(
        &self,
        server: &str,
    ) -> (
        ElicitScope,
        tokio::sync::mpsc::UnboundedReceiver<ElicitRequest>,
    ) {
        open_scope(self.id, server)
    }
}

/// Put a request to the host and wait for the user. `owner` is the manager the sending
/// connection belongs to, or `None` for a pooled connection shared by several managers: then the
/// request goes to the one scope that covers the server, and is declined when more than one
/// session could be the one that triggered it. Declines when no scope matches or the host left.
pub(crate) async fn route(
    owner: Option<u64>,
    server: &str,
    message: String,
    prompt: ElicitPrompt,
) -> ElicitReply {
    let (reply, answer) = oneshot::channel();
    let request = ElicitRequest {
        server: server.to_string(),
        message,
        prompt,
        reply,
    };
    let chosen = {
        let scopes = SCOPES.lock();
        let mut candidates = scopes
            .iter()
            .filter(|s| s.server == server && owner.is_none_or(|o| o == s.owner));
        let newest = candidates.clone().next_back();
        let ambiguous =
            owner.is_none() && newest.is_some_and(|n| candidates.any(|s| s.owner != n.owner));
        newest
            .filter(|_| !ambiguous)
            .map(|s| (s.tx.clone(), Arc::clone(&s.state)))
    };
    let Some((tx, state)) = chosen else {
        return ElicitReply::Decline;
    };
    let _waiting = WaitingGuard::new(state);
    if tx.send(request).is_err() {
        return ElicitReply::Decline;
    }
    answer.await.unwrap_or(ElicitReply::Decline)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn form() -> ElicitForm {
        ElicitForm::from_schema(&json!({
            "type": "object",
            "properties": {
                "count": {"type": "integer", "title": "How many", "minimum": 1, "maximum": 10},
                "api_token": {"type": "string", "title": "Token"},
                "env": {"type": "string", "enum": ["dev", "prod"], "enumNames": ["Development", "Production"]},
                "tags": {"type": "array", "items": {"enum": ["a", "b", "c"]}},
                "dry": {"type": "boolean"},
                "note": {"type": "string", "maxLength": 5}
            },
            "required": ["count", "api_token"]
        }))
    }

    /// Answers laid out in the form's own field order (schema properties come back sorted by key).
    fn by_key(form: &ElicitForm, given: &[(&str, Answer)]) -> Vec<Answer> {
        form.fields
            .iter()
            .map(|f| {
                given
                    .iter()
                    .find(|(k, _)| *k == f.key)
                    .map(|(_, a)| a.clone())
                    .unwrap_or_default()
            })
            .collect()
    }

    fn typed(text: &str) -> Answer {
        Answer {
            other: Some(text.into()),
            ..Answer::default()
        }
    }

    fn picked(labels: &[&str]) -> Answer {
        Answer {
            selected: labels.iter().map(|l| l.to_string()).collect(),
            ..Answer::default()
        }
    }

    #[test]
    fn schema_maps_to_typed_fields_and_masks_secrets() {
        let f = form();
        assert_eq!(f.fields.len(), 6);
        let by_key = |k: &str| f.fields.iter().find(|x| x.key == k).unwrap();
        assert_eq!(
            by_key("count").kind,
            FieldKind::Integer {
                min: Some(1),
                max: Some(10)
            }
        );
        assert!(matches!(
            by_key("api_token").kind,
            FieldKind::Text { secret: true, .. }
        ));
        assert!(matches!(
            by_key("tags").kind,
            FieldKind::Choice { multi: true, .. }
        ));
        let qs = f.questions(&[]);
        let count_q = qs.iter().find(|q| q.header == "How many").unwrap();
        assert!(count_q.text.contains("[1 to 10]"), "{}", count_q.text);
        assert!(qs.iter().any(|q| q.secret), "token question is masked");
        assert_eq!(qs.iter().filter(|q| q.secret).count(), 1);
    }

    #[test]
    fn answers_coerce_to_typed_values() {
        let f = form();
        let answers = by_key(
            &f,
            &[
                ("count", typed("7")),
                ("api_token", typed("hunter2")),
                ("env", picked(&["Production"])),
                ("tags", picked(&["a", "c"])),
                ("dry", picked(&["Yes"])),
            ],
        );
        let v = f.resolve(&answers).unwrap();
        assert_eq!(v["count"], json!(7));
        assert_eq!(v["api_token"], json!("hunter2"));
        assert_eq!(v["env"], json!("prod"));
        assert_eq!(v["tags"], json!(["a", "c"]));
        assert_eq!(v["dry"], json!(true));
        assert!(!v.contains_key("note"));
    }

    #[test]
    fn bounds_required_and_type_errors_name_the_field() {
        let f = form();
        let at = |key: &str| f.fields.iter().position(|x| x.key == key).unwrap();
        let errs = f
            .resolve(&by_key(
                &f,
                &[("count", typed("11")), ("note", typed("toolong"))],
            ))
            .unwrap_err();
        assert!(errs[at("count")]
            .as_deref()
            .unwrap()
            .contains("out of range"));
        assert!(errs[at("api_token")]
            .as_deref()
            .unwrap()
            .contains("required"));
        assert!(errs[at("env")].is_none());
        assert!(errs[at("note")].is_some());
        let errs = f
            .resolve(&by_key(
                &f,
                &[("count", typed("abc")), ("api_token", typed("x"))],
            ))
            .unwrap_err();
        assert!(errs[at("count")]
            .as_deref()
            .unwrap()
            .contains("whole number"));
        let again = f.questions(&errs);
        assert!(again[at("count")]
            .text
            .starts_with("'abc' is not a whole number."));
    }

    fn url() -> ElicitPrompt {
        ElicitPrompt::Url("https://x".into())
    }

    #[tokio::test]
    async fn requests_reach_only_the_scope_of_the_connection_that_sent_them() {
        let (a, b) = (new_owner_id(), new_owner_id());
        assert_eq!(
            route(Some(a), "srv", "hi".into(), url()).await,
            ElicitReply::Decline,
            "no scope open"
        );
        let (_b_other, mut b_other_rx) = open_scope(b, "elsewhere");
        let (scope_a, mut a_rx) = open_scope(a, "srv");
        let (scope_b, mut b_rx) = open_scope(b, "srv");

        // A private connection of manager A reaches A's scope even though B's is newer.
        let task = tokio::spawn(route(Some(a), "srv", "hi".into(), url()));
        let req = a_rx.recv().await.unwrap();
        assert_eq!(req.message, "hi");
        assert!(b_rx.try_recv().is_err() && b_other_rx.try_recv().is_err());
        req.reply.send(ElicitReply::Cancel).unwrap();
        assert_eq!(task.await.unwrap(), ElicitReply::Cancel);

        // A pooled connection (owner unknown) shared by two sessions is ambiguous: declined,
        // and neither session sees the form.
        assert_eq!(
            route(None, "srv", "hi".into(), url()).await,
            ElicitReply::Decline
        );
        assert!(a_rx.try_recv().is_err() && b_rx.try_recv().is_err());

        // With only one session covering the server there is no ambiguity.
        drop(scope_a);
        let task = tokio::spawn(route(None, "srv", "hi".into(), url()));
        let req = b_rx.recv().await.unwrap();
        req.reply.send(ElicitReply::Decline).unwrap();
        assert_eq!(task.await.unwrap(), ElicitReply::Decline);
        drop(scope_b);
    }

    #[tokio::test]
    async fn a_pending_question_only_suspends_the_timeout_of_its_own_call() {
        let (a, b) = (new_owner_id(), new_owner_id());
        let (_sa, mut a_rx) = open_scope(a, "srv");
        let (_sb, _b_rx) = open_scope(b, "srv");
        assert!(!user_is_answering(a, "srv"));
        let task = tokio::spawn(route(Some(a), "srv", "hi".into(), url()));
        let req = a_rx.recv().await.unwrap();
        assert!(user_is_answering(a, "srv"));
        assert!(
            !user_is_answering(b, "srv"),
            "another session's call is unaffected"
        );
        assert!(!user_is_answering(a, "other"));
        req.reply.send(ElicitReply::Decline).unwrap();
        task.await.unwrap();
    }

    #[tokio::test]
    async fn a_server_elicitation_round_trips_through_a_real_connection() {
        let mgr = std::sync::Arc::new(
            crate::testsupport::manager_with_echo(&forge_config::McpConfig::default()).await,
        );
        let (_scope, mut rx) = mgr.elicit_scope("test");
        let call = {
            let mgr = std::sync::Arc::clone(&mgr);
            tokio::spawn(async move { mgr.call("test__ask", &json!({})).await })
        };
        let request = rx.recv().await.expect("the server asked");
        assert_eq!(request.message, "Need two things");
        let ElicitPrompt::Form(form) = &request.prompt else {
            panic!("expected a form");
        };
        let keys: Vec<&str> = form.fields.iter().map(|f| f.key.as_str()).collect();
        assert_eq!(keys, ["api_token", "count", "dry_run", "env"]);
        let questions = form.questions(&[]);
        assert!(
            questions
                .iter()
                .find(|q| q.text.starts_with("API token"))
                .unwrap()
                .secret
        );
        let answers: Vec<Answer> = form
            .fields
            .iter()
            .map(|f| match f.key.as_str() {
                "count" => typed("7"),
                "api_token" => typed("s3cret"),
                "env" => picked(&["prod"]),
                _ => picked(&["No"]),
            })
            .collect();
        let values = form.resolve(&answers).expect("valid");
        request.reply.send(ElicitReply::Accept(values)).unwrap();
        let outcome = call.await.unwrap();
        assert!(outcome.ok, "{}", outcome.text);
        assert!(
            outcome.text.contains("\"action\":\"accept\""),
            "{}",
            outcome.text
        );
        assert!(outcome.text.contains("\"count\":7"), "{}", outcome.text);
        assert!(
            outcome.text.contains("\"dry_run\":false"),
            "{}",
            outcome.text
        );

        // The scope is shared process state, so the no-listener case runs in the same test.
        drop(_scope);
        let outcome = mgr.call("test__ask", &json!({})).await;
        assert!(
            outcome.text.contains("\"action\":\"decline\""),
            "{}",
            outcome.text
        );
    }
}
