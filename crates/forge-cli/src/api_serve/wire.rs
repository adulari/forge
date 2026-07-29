//! Translating the OpenAI wire format into Forge's domain (and the routing text back out).
//!
//! The whole point of the API server is that an existing OpenAI-compatible client works unchanged,
//! so this module absorbs the shapes those clients actually send: content that is either a string
//! or a content-part array, tool calls with stringified JSON arguments, `response_format` in two
//! spellings, `model` as `auto`/`mesh`/a concrete id. It also derives the text the mesh classifies
//! on — bounded, and reduced to structure for machine-generated payloads, so one enormous pasted
//! document cannot dominate routing.

use forge_mesh::RoutingContext;
use forge_provider::{ResponseFormat, ToolSpec};
use forge_types::{Message, Role};
use serde::Deserialize;
use sha2::{Digest as _, Sha256};

#[derive(Deserialize)]
pub(super) struct ChatCompletionRequest {
    #[serde(default)]
    pub(super) model: Option<String>,
    pub(super) messages: Vec<IncomingMessage>,
    #[serde(default)]
    pub(super) stream: bool,
    /// OpenAI streaming option. When requested, Forge emits a final usage-only chunk, including
    /// provider-reported cached prompt tokens.
    #[serde(default)]
    pub(super) stream_options: Option<StreamOptions>,
    #[serde(default)]
    pub(super) temperature: Option<f32>,
    /// OpenAI's reasoning-effort hint (`low`/`medium`/`high`); also accepts Forge's `xhigh`.
    #[serde(default)]
    pub(super) reasoning_effort: Option<String>,
    /// OpenAI prompt-cache routing key. Forwarded unchanged when supplied; when omitted Forge
    /// derives a privacy-preserving stable key from the standing conversation prefix.
    #[serde(default)]
    pub(super) prompt_cache_key: Option<String>,
    /// Deprecated OpenAI end-user identifier, still accepted as stable cache-key input.
    #[serde(default)]
    pub(super) user: Option<String>,
    /// Advertised tools (OpenAI function shape). Forwarded to the model; any `tool_calls` it makes
    /// come back in the response (the client runs its own tool loop, as with the OpenAI API).
    #[serde(default)]
    pub(super) tools: Vec<serde_json::Value>,
    /// OpenAI structured-output request (`{"type":"json_object"}` or `{"type":"json_schema",…}`).
    /// Forwarded to the provider's JSON mode so a caller asking for JSON actually gets JSON.
    #[serde(default)]
    pub(super) response_format: Option<serde_json::Value>,
}

#[derive(Deserialize, Default)]
pub(super) struct StreamOptions {
    #[serde(default)]
    pub(super) include_usage: bool,
}

/// Stable cache identity for stateless `/v1/chat/completions` callers. SDKs that expose OpenAI's
/// `prompt_cache_key` retain full control. Older clients get an automatic SHA-256 key derived from
/// the model/user plus the leading system messages and first conversation message — the portion
/// that remains unchanged as a client resends a growing transcript. No prompt text leaks into the
/// provider-visible key.
pub(super) fn api_prompt_cache_key(req: &ChatCompletionRequest) -> String {
    if let Some(explicit) = req
        .prompt_cache_key
        .as_deref()
        .map(str::trim)
        .filter(|key| !key.is_empty())
    {
        return explicit.to_string();
    }

    let mut hash = Sha256::new();
    hash.update(b"forge-api-cache-v1\0");
    hash.update(req.model.as_deref().unwrap_or("auto").as_bytes());
    hash.update(b"\0");
    if let Some(user) = req.user.as_deref() {
        hash.update(user.as_bytes());
    }
    hash.update(b"\0");

    let mut included_conversation_message = false;
    for message in &req.messages {
        let standing = matches!(message.role.as_str(), "system" | "developer");
        if !standing && included_conversation_message {
            break;
        }
        hash.update(message.role.as_bytes());
        hash.update(b"\0");
        hash.update(content_text(&message.content).as_bytes());
        hash.update(b"\0");
        if !standing {
            included_conversation_message = true;
        }
    }

    let digest = hash.finalize();
    let short = digest[..16]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("forge-api-{short}")
}

#[derive(Deserialize)]
pub(super) struct IncomingMessage {
    pub(super) role: String,
    /// String, or an array of content parts (vision/multi-part) — only text parts are used.
    #[serde(default)]
    pub(super) content: serde_json::Value,
    #[serde(default)]
    pub(super) tool_call_id: Option<String>,
    #[serde(default)]
    pub(super) tool_calls: Vec<serde_json::Value>,
}

/// Flatten OpenAI message `content` (string | array-of-parts | null) into plain text.
pub(super) fn content_text(content: &serde_json::Value) -> String {
    match content {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(parts) => parts
            .iter()
            .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    }
}

/// Convert the incoming OpenAI messages into Forge's transcript type.
pub(super) fn to_forge_messages(msgs: &[IncomingMessage]) -> Vec<Message> {
    msgs.iter()
        .map(|m| {
            let text = content_text(&m.content);
            let role = match m.role.as_str() {
                "system" | "developer" => Role::System,
                "assistant" => Role::Assistant,
                "tool" => Role::Tool,
                _ => Role::User,
            };
            let mut msg = Message::new(role, text);
            msg.tool_call_id = m.tool_call_id.clone();
            if role == Role::Assistant && !m.tool_calls.is_empty() {
                msg.tool_calls = m
                    .tool_calls
                    .iter()
                    .filter_map(parse_incoming_tool_call)
                    .collect();
            }
            msg
        })
        .collect()
}

/// Parse an OpenAI assistant `tool_calls[]` entry into a Forge `ToolCall`.
pub(super) fn parse_incoming_tool_call(v: &serde_json::Value) -> Option<forge_types::ToolCall> {
    let f = v.get("function")?;
    let name = f.get("name")?.as_str()?.to_string();
    let args = match f.get("arguments") {
        Some(serde_json::Value::String(s)) => {
            serde_json::from_str(s).unwrap_or(serde_json::json!({}))
        }
        Some(other) => other.clone(),
        None => serde_json::json!({}),
    };
    Some(forge_types::ToolCall {
        id: v
            .get("id")
            .and_then(|i| i.as_str())
            .unwrap_or("")
            .to_string(),
        name,
        args,
    })
}

/// Convert advertised OpenAI tools into Forge `ToolSpec`s.
pub(super) fn to_tool_specs(tools: &[serde_json::Value]) -> Vec<ToolSpec> {
    tools
        .iter()
        .filter_map(|t| {
            let f = t.get("function").unwrap_or(t);
            let name = f.get("name")?.as_str()?.to_string();
            Some(ToolSpec {
                name,
                description: f
                    .get("description")
                    .and_then(|d| d.as_str())
                    .unwrap_or("")
                    .to_string(),
                schema: f
                    .get("parameters")
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({"type": "object"})),
            })
        })
        .collect()
}

const ROUTING_TOOL_RESULT_CHARS: usize = 2_000;
const ROUTING_TOOL_RESULTS: usize = 3;

pub(super) fn bounded_routing_text(text: &str, max_chars: usize) -> String {
    let mut chars = text.trim().chars();
    let mut bounded: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        bounded.push('…');
    }
    bounded
}

pub(super) fn structured_routing_text(text: &str) -> String {
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Role-aware activity for mesh classification: the last user task plus bounded tool results
/// produced after it. Standing system/developer instructions and older independent user turns are
/// intentionally excluded.
pub(super) fn routing_prompt(msgs: &[Message]) -> String {
    let Some(user_index) = msgs.iter().rposition(|message| message.role == Role::User) else {
        return msgs
            .iter()
            .rev()
            .find(|message| message.role == Role::Tool)
            .map(|message| bounded_routing_text(&message.content, ROUTING_TOOL_RESULT_CHARS))
            .unwrap_or_default();
    };

    // Remove paragraph separators after role extraction so the opt-in legacy
    // `classifier_activity_focused` compatibility mode cannot re-split structured API input.
    let mut prompt = structured_routing_text(&msgs[user_index].content);
    for result in msgs[user_index + 1..]
        .iter()
        .filter(|message| message.role == Role::Tool)
        .take(ROUTING_TOOL_RESULTS)
    {
        prompt.push_str("\nTOOL RESULT:\n");
        prompt.push_str(&bounded_routing_text(
            &result.content,
            ROUTING_TOOL_RESULT_CHARS,
        ));
    }
    prompt
}

/// Bounded prior user/assistant context for genuinely referential turns such as "continue".
/// [`RoutingContext`] excludes ordinary system messages and only uses this history when the last
/// user turn actually depends on it.
pub(super) fn routing_context(msgs: &[Message]) -> RoutingContext {
    msgs.iter()
        .rposition(|message| message.role == Role::User)
        .map(|user_index| RoutingContext::from_messages(&msgs[..user_index]))
        .unwrap_or_default()
}

/// `"auto"` / `"mesh"` / empty / missing ⇒ no pin (mesh routes). Anything else pins that model.
pub(super) fn model_pin(model: &Option<String>) -> Option<String> {
    match model.as_deref() {
        None | Some("") | Some("auto") | Some("mesh") | Some("default") => None,
        Some(m) => Some(m.to_string()),
    }
}

/// Resolve the request's `model` into an optional hard pin. An explicit model is honored EXACTLY as
/// `forge run --model <id>` honors it — dispatched straight to its provider, bypassing mesh
/// classification — so external OpenAI-compatible consumers get deterministic per-request model
/// selection. Returns:
/// - `Ok(None)` for the mesh sentinels (`auto`/`mesh`/…) — let the mesh classify + route.
/// - `Ok(Some(id))` when `id` is a routable model this server can dispatch to (advertised, OR any
///   model whose provider has a usable key — the [`forge_mesh::pin_is_dispatchable`] rule the CLI
///   pin path uses). Crucially this is NOT gated on the advertised catalog: a caller must be able to
///   pin any model their key reaches, even one auto-discovery didn't enumerate (a completion-only
///   provider, or a model newer than the cached catalog). That un-advertised-but-valid case is the
///   gap #509 missed — it "worked" only for models that happened to be in the discovery catalog.
/// - `Err(msg)` when `id` is unroutable — a bare `provider::` prefix, a task-specific endpoint
///   (translation/embedding/…), or an unknown provider — so the caller returns a clean 4xx instead
///   of silently mesh-routing the request to some unrelated model, panicking, or 500ing.
pub(super) fn resolve_pin(
    models: &[String],
    model: &Option<String>,
) -> Result<Option<String>, String> {
    let Some(pin) = model_pin(model) else {
        return Ok(None);
    };
    let provider = forge_config::provider_of(&pin);
    if provider.is_empty() || pin.trim_end().ends_with("::") {
        return Err(format!(
            "model '{pin}' is not a valid `provider::model` id — GET /v1/models for the routable \
             ids, or use \"auto\" to let the mesh pick"
        ));
    }
    if !forge_mesh::catalog::is_routable(&pin) {
        return Err(format!(
            "model '{pin}' is a task-specific endpoint (translation/embedding/TTS/image), not a \
             chat model — it can't answer /v1/chat/completions; GET /v1/models for the routable ids"
        ));
    }
    // An advertised model is dispatchable by definition (covers keyless/offline setups). Otherwise
    // honor it iff its provider is one we know AND has a usable key — mirroring the CLI `--model`
    // pin, which routes to any keyed provider regardless of the discovery catalog. A truly unknown
    // provider (a typo) or a keyed provider with no key configured is rejected here, not dispatched.
    if models.iter().any(|m| m == &pin)
        || (forge_config::is_known_provider(provider) && forge_mesh::pin_is_dispatchable(&pin))
    {
        Ok(Some(pin))
    } else {
        Err(format!(
            "model '{pin}' can't be routed on this server — unknown provider, or no API key \
             configured for '{provider}'. GET /v1/models for the routable ids, or use \"auto\"."
        ))
    }
}

/// Strip a surrounding Markdown code fence (```` ```json … ``` ````) from `s`, returning the inner
/// text. Providers vary: OpenAI honors JSON mode natively, but others (e.g. Gemini via a plain
/// `json_object` request, several OpenRouter models) ignore it and wrap the JSON in a fence. The
/// OpenAI `json_object` contract promises the `content` IS parseable JSON, so when the caller asked
/// for JSON we unwrap the fence to honor it. A no-op when there's no fence.
pub(super) fn unfence_json(s: &str) -> String {
    let t = s.trim();
    let Some(rest) = t.strip_prefix("```") else {
        return s.to_string();
    };
    // Drop an optional language tag on the opening fence (```json / ```JSON / ```).
    let rest = rest
        .strip_prefix("json")
        .or_else(|| rest.strip_prefix("JSON"))
        .unwrap_or(rest);
    let inner = rest.trim_start_matches('\n');
    match inner.rfind("```") {
        Some(end) => inner[..end].trim().to_string(),
        None => s.to_string(),
    }
}

/// Parse an OpenAI `response_format` object into a provider-neutral [`ResponseFormat`]. Unknown or
/// `{"type":"text"}` shapes map to `None` (plain text, the default).
pub(super) fn parse_response_format(v: &Option<serde_json::Value>) -> Option<ResponseFormat> {
    let obj = v.as_ref()?;
    match obj.get("type").and_then(|t| t.as_str()) {
        Some("json_object") => Some(ResponseFormat::JsonObject),
        Some("json_schema") => {
            // OpenAI nests the schema under `json_schema: {name, schema}`.
            let spec = obj.get("json_schema")?;
            let name = spec
                .get("name")
                .and_then(|n| n.as_str())
                .unwrap_or("response")
                .to_string();
            let schema = spec.get("schema").cloned().unwrap_or(serde_json::json!({}));
            Some(ResponseFormat::JsonSchema { name, schema })
        }
        _ => None,
    }
}
