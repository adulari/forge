//! `forge api` — an OpenAI-compatible HTTP server backed by Forge's model mesh.
//!
//! Run Forge as your application's AI backend: start this server and point ANY OpenAI-compatible
//! client (its `base_url`) at `http://<host>:<port>/v1`. Every `POST /v1/chat/completions` is
//! routed through Forge's mesh — task-tier model selection, cross-provider failover, subscription
//! quota-spread and cost tracking — instead of a single hard-wired model. Swapping one base URL
//! turns a single-model integration into a multi-model, self-healing one.
//!
//! The surface is deliberately a strict subset of OpenAI's, so existing SDKs work unchanged:
//! - `GET  /v1/models`            — the models the mesh can route to (plus the `auto` sentinel).
//! - `POST /v1/chat/completions`  — one chat completion; `stream:true` yields OpenAI SSE chunks.
//! - `GET  /health`              — liveness probe (`{"status":"ok"}`), no auth.
//!
//! Model selection: pass `"model":"auto"` (or `"mesh"`, or omit it) to let the mesh pick per
//! request; pass a concrete Forge id (e.g. `"anthropic::claude-opus-4-8"`, `"groq::llama-3.3-70b"`)
//! to pin it — the mesh still fails over to alternatives if the pinned model is down.
//!
//! Auth: if `--api-key <KEY>` (or `FORGE_API_KEY`) is set, requests must carry
//! `Authorization: Bearer <KEY>`; otherwise the server is open (intended for loopback / a trusted
//! private network — put a reverse proxy in front for TLS + public exposure).
//!
//! This is an opt-in subcommand: nothing here runs unless `forge api` is invoked, and it changes
//! no default Forge behavior.

use std::sync::Arc;

use anyhow::{Context, Result};
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;
use forge_mesh::{BudgetState, Router as MeshRouter};
use forge_provider::{CompletionOptions, Provider, StreamEvent, ToolSpec};
use forge_types::{EffortLevel, Message, ModelHealth, ProjectContext, Role, SubscriptionQuota};
use serde::Deserialize;

/// Default listen port for `forge api`. Distinct from `forge serve`'s 7420 so both can run at once.
const DEFAULT_PORT: u16 = 8787;

/// Shared HTTP state for the API server.
struct ApiState {
    provider: Arc<dyn Provider>,
    router: Arc<dyn MeshRouter>,
    pricing: forge_mesh::pricing::Pricing,
    /// Currently-benched models (skipped by routing); empty in `--mock`.
    health: ModelHealth,
    /// Models advertised by `GET /v1/models` (catalog when discovered, else configured candidates).
    models: Vec<String>,
    /// When `Some`, requests must present this as a bearer token.
    api_key: Option<String>,
}

pub(crate) async fn api_serve_cmd(
    host: Option<String>,
    port: Option<u16>,
    api_key: Option<String>,
    mock: bool,
) -> Result<()> {
    let config = forge_config::load().unwrap_or_default();
    let port = port.unwrap_or(DEFAULT_PORT);
    let host = host.unwrap_or_else(|| "127.0.0.1".to_string());
    let api_key = api_key
        .or_else(|| std::env::var("FORGE_API_KEY").ok())
        .filter(|k| !k.is_empty());

    // Auto-discovery catalog (same cache-first path as `forge run`), so the mesh routes across every
    // usable model rather than only the configured defaults. Skipped for the offline mock.
    let catalog = if !mock && config.mesh.auto_discover {
        crate::cli::commands::models::load_cached_catalog()
    } else {
        None
    };

    let ctx_windows = crate::open_store()
        .ok()
        .and_then(|s| s.all_model_contexts().ok())
        .unwrap_or_default();
    let (provider, router) = crate::cli::commands::models::build_provider_and_router(
        &config,
        mock,
        None, // no global pin — the per-request `model` field decides
        catalog.clone(),
        ctx_windows,
        std::collections::HashMap::new(),
    );

    let health = if mock {
        ModelHealth::default()
    } else {
        crate::open_store()
            .ok()
            .and_then(|s| s.current_benched().ok())
            .unwrap_or_default()
    };

    let models = advertised_models(&config, catalog.as_ref());
    let state = Arc::new(ApiState {
        provider,
        router,
        pricing: forge_mesh::pricing::Pricing::from_config(&config),
        health,
        models,
        api_key: api_key.clone(),
    });

    let app = api_router(state);
    let listener = tokio::net::TcpListener::bind((host.as_str(), port))
        .await
        .with_context(|| format!("binding {host}:{port} — is another server on that port?"))?;
    let addr = listener.local_addr()?;

    println!("⚒ forge api — OpenAI-compatible endpoint backed by the mesh");
    println!("  listening on http://{addr}");
    println!("  base_url:   http://{addr}/v1");
    println!(
        "  auth:       {}",
        if api_key.is_some() {
            "Bearer <FORGE_API_KEY> required"
        } else {
            "open (loopback / trusted network only)"
        }
    );
    println!("  models:     GET http://{addr}/v1/models  ·  use \"auto\" to let the mesh route");

    axum::serve(listener, app).await?;
    Ok(())
}

/// The API route table. Extracted so tests drive the real router without binding a socket.
fn api_router(state: Arc<ApiState>) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/v1/models", get(list_models))
        .route("/v1/chat/completions", post(chat_completions))
        .fallback(|| async { (StatusCode::NOT_FOUND, "Not Found").into_response() })
        .with_state(state)
}

/// The models `GET /v1/models` advertises: the discovered catalog when present, else the union of
/// configured candidates across tiers. Always includes the `auto` sentinel (mesh routing).
fn advertised_models(
    config: &forge_config::Config,
    catalog: Option<&forge_mesh::ModelCatalog>,
) -> Vec<String> {
    let mut models: Vec<String> = Vec::new();
    if let Some(cat) = catalog.filter(|c| !c.is_empty()) {
        models.extend(cat.models().iter().cloned());
    } else {
        for tier in [
            forge_types::TaskTier::Trivial,
            forge_types::TaskTier::Standard,
            forge_types::TaskTier::Complex,
        ] {
            models.extend(config.candidates_for(tier));
        }
    }
    models.sort();
    models.dedup();
    models.insert(0, "auto".to_string());
    models
}

// ---------------------------------------------------------------------------
// Auth
// ---------------------------------------------------------------------------

/// `None` when the request is authorized (no key configured, or a matching bearer token); otherwise
/// `Some(error_response)` to return. `Option` (not `Result`) keeps the large `Response` off a
/// hot-path return type (clippy::result_large_err).
fn check_auth(state: &ApiState, headers: &HeaderMap) -> Option<Response> {
    let expected = state.api_key.as_deref()?;
    let presented = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim);
    if presented == Some(expected) {
        None
    } else {
        Some(openai_error(
            StatusCode::UNAUTHORIZED,
            "invalid_request_error",
            "missing or invalid API key",
        ))
    }
}

// ---------------------------------------------------------------------------
// Request / response types (an OpenAI-compatible subset)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct ChatCompletionRequest {
    #[serde(default)]
    model: Option<String>,
    messages: Vec<IncomingMessage>,
    #[serde(default)]
    stream: bool,
    #[serde(default)]
    temperature: Option<f32>,
    /// OpenAI's reasoning-effort hint (`low`/`medium`/`high`); also accepts Forge's `xhigh`.
    #[serde(default)]
    reasoning_effort: Option<String>,
    /// Advertised tools (OpenAI function shape). Forwarded to the model; any `tool_calls` it makes
    /// come back in the response (the client runs its own tool loop, as with the OpenAI API).
    #[serde(default)]
    tools: Vec<serde_json::Value>,
}

#[derive(Deserialize)]
struct IncomingMessage {
    role: String,
    /// String, or an array of content parts (vision/multi-part) — only text parts are used.
    #[serde(default)]
    content: serde_json::Value,
    #[serde(default)]
    tool_call_id: Option<String>,
    #[serde(default)]
    tool_calls: Vec<serde_json::Value>,
}

/// Flatten OpenAI message `content` (string | array-of-parts | null) into plain text.
fn content_text(content: &serde_json::Value) -> String {
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
fn to_forge_messages(msgs: &[IncomingMessage]) -> Vec<Message> {
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
fn parse_incoming_tool_call(v: &serde_json::Value) -> Option<forge_types::ToolCall> {
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
fn to_tool_specs(tools: &[serde_json::Value]) -> Vec<ToolSpec> {
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

/// The routing prompt: the concatenation of the user turns (what the mesh classifies difficulty on).
fn routing_prompt(msgs: &[Message]) -> String {
    let user: Vec<&str> = msgs
        .iter()
        .filter(|m| m.role == Role::User)
        .map(|m| m.content.as_str())
        .collect();
    if user.is_empty() {
        msgs.last().map(|m| m.content.clone()).unwrap_or_default()
    } else {
        user.join("\n")
    }
}

/// `"auto"` / `"mesh"` / empty / missing ⇒ no pin (mesh routes). Anything else pins that model.
fn model_pin(model: &Option<String>) -> Option<String> {
    match model.as_deref() {
        None | Some("") | Some("auto") | Some("mesh") | Some("default") => None,
        Some(m) => Some(m.to_string()),
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn health() -> Response {
    axum::Json(serde_json::json!({"status": "ok"})).into_response()
}

async fn list_models(State(state): State<Arc<ApiState>>, headers: HeaderMap) -> Response {
    if let Some(resp) = check_auth(&state, &headers) {
        return resp;
    }
    let created = now_unix();
    let data: Vec<serde_json::Value> = state
        .models
        .iter()
        .map(|id| {
            serde_json::json!({
                "id": id,
                "object": "model",
                "created": created,
                "owned_by": "forge",
            })
        })
        .collect();
    axum::Json(serde_json::json!({"object": "list", "data": data})).into_response()
}

/// The outcome of one (possibly failed-over) completion.
struct Completed {
    model: String,
    response: forge_provider::ModelResponse,
    rationale: String,
}

async fn chat_completions(
    State(state): State<Arc<ApiState>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    if let Some(resp) = check_auth(&state, &headers) {
        return resp;
    }
    let req: ChatCompletionRequest = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(e) => {
            return openai_error(
                StatusCode::BAD_REQUEST,
                "invalid_request_error",
                &format!("malformed request body: {e}"),
            )
        }
    };
    if req.messages.is_empty() {
        return openai_error(
            StatusCode::BAD_REQUEST,
            "invalid_request_error",
            "`messages` must not be empty",
        );
    }

    let messages = to_forge_messages(&req.messages);
    let tools = to_tool_specs(&req.tools);
    let pin = model_pin(&req.model);
    let effort = req.reasoning_effort.as_deref().and_then(EffortLevel::parse);
    let opts = CompletionOptions {
        effort,
        temperature: req.temperature,
        checkpoint: None,
    };

    // Ask the mesh which model to use (and its ordered failover chain).
    let decision = {
        let prompt = routing_prompt(&messages);
        let router = state
            .router
            .route(
                &prompt,
                BudgetState::default(),
                &state.health,
                &SubscriptionQuota::default(),
                effort,
                &ProjectContext::default(),
            )
            .await;
        // A per-request pin overrides the mesh's primary pick but keeps the failover chain.
        match pin {
            Some(p) => {
                let mut chain: Vec<String> = std::iter::once(router.model.clone())
                    .chain(router.fallbacks.clone())
                    .filter(|m| m != &p)
                    .collect();
                chain.insert(0, p);
                chain
            }
            None => std::iter::once(router.model.clone())
                .chain(router.fallbacks.clone())
                .collect(),
        }
    };

    if req.stream {
        stream_completion(state, messages, tools, opts, decision)
    } else {
        match run_completion(&state, &messages, &tools, &opts, &decision).await {
            Ok(done) => non_stream_response(&state, done).into_response(),
            Err(msg) => openai_error(StatusCode::BAD_GATEWAY, "api_error", &msg),
        }
    }
}

/// Run the completion down the failover chain (non-streaming): the first model that succeeds wins;
/// a retryable failure moves to the next; a permanent one aborts. `chain` is never empty.
async fn run_completion(
    state: &ApiState,
    messages: &[Message],
    tools: &[ToolSpec],
    opts: &CompletionOptions,
    chain: &[String],
) -> Result<Completed, String> {
    let mut last_err = String::from("no model was available to route to");
    for model in chain {
        let mut sink = |_ev: StreamEvent| {};
        match state
            .provider
            .complete_with(model, messages, tools, opts, &mut sink)
            .await
        {
            Ok(response) => {
                return Ok(Completed {
                    model: model.clone(),
                    response,
                    rationale: format!("routed to {model}"),
                })
            }
            Err(e) if e.is_retryable() => {
                last_err = format!("{model}: {e}");
                continue;
            }
            Err(e) => return Err(format!("{model}: {e}")),
        }
    }
    Err(last_err)
}

fn non_stream_response(state: &ApiState, done: Completed) -> impl IntoResponse {
    let usage = &done.response.usage;
    let cost = state.pricing.cost_for_usage(&done.model, usage);
    let (message, finish_reason) = assistant_message_json(&done.response);
    let body = serde_json::json!({
        "id": completion_id(),
        "object": "chat.completion",
        "created": now_unix(),
        "model": done.model,
        "choices": [{
            "index": 0,
            "message": message,
            "finish_reason": finish_reason,
        }],
        "usage": {
            "prompt_tokens": usage.input_tokens,
            "completion_tokens": usage.output_tokens,
            "total_tokens": usage.total_tokens(),
        },
        // Forge extension: routing/cost visibility (ignored by strict OpenAI clients).
        "x_forge": {
            "routed_model": done.model,
            "rationale": done.rationale,
            "cost_usd": cost,
        },
    });
    axum::Json(body)
}

/// Build the OpenAI `message` object + `finish_reason` for a completed response.
fn assistant_message_json(
    resp: &forge_provider::ModelResponse,
) -> (serde_json::Value, &'static str) {
    if resp.wants_tools() {
        let tool_calls: Vec<serde_json::Value> = resp
            .tool_calls
            .iter()
            .map(|c| {
                serde_json::json!({
                    "id": c.id,
                    "type": "function",
                    "function": {
                        "name": c.name,
                        "arguments": serde_json::to_string(&c.args).unwrap_or_else(|_| "{}".into()),
                    },
                })
            })
            .collect();
        (
            serde_json::json!({
                "role": "assistant",
                "content": if resp.content.is_empty() { serde_json::Value::Null } else { serde_json::Value::String(resp.content.clone()) },
                "tool_calls": tool_calls,
            }),
            "tool_calls",
        )
    } else {
        (
            serde_json::json!({"role": "assistant", "content": resp.content}),
            "stop",
        )
    }
}

// ---------------------------------------------------------------------------
// Streaming (Server-Sent Events, OpenAI `chat.completion.chunk` shape)
// ---------------------------------------------------------------------------

fn stream_completion(
    state: Arc<ApiState>,
    messages: Vec<Message>,
    tools: Vec<ToolSpec>,
    opts: CompletionOptions,
    chain: Vec<String>,
) -> Response {
    let (tx, rx) =
        tokio::sync::mpsc::unbounded_channel::<Result<axum::body::Bytes, std::io::Error>>();
    let id = completion_id();
    let created = now_unix();

    tokio::spawn(async move {
        let send =
            |tx: &tokio::sync::mpsc::UnboundedSender<Result<axum::body::Bytes, std::io::Error>>,
             frame: String| {
                let _ = tx.send(Ok(axum::body::Bytes::from(frame)));
            };

        let mut last_err = String::from("no model was available to route to");
        let mut succeeded = false;
        for model in &chain {
            // Whether THIS attempt has already streamed any text to the client — once it has, a
            // later failure can't be transparently failed-over (the client saw partial output).
            let emitted = Arc::new(std::sync::atomic::AtomicBool::new(false));
            // Role chunk first (OpenAI clients expect an opening delta with the role).
            send(
                &tx,
                sse_chunk(
                    &id,
                    created,
                    model,
                    serde_json::json!({"role": "assistant"}),
                    None,
                ),
            );

            let result = {
                let emitted = emitted.clone();
                let tx = tx.clone();
                let id = id.clone();
                let sink_model = model.clone();
                let mut sink = move |ev: StreamEvent| {
                    if let StreamEvent::Text(t) = ev {
                        if t.is_empty() {
                            return;
                        }
                        emitted.store(true, std::sync::atomic::Ordering::Relaxed);
                        let _ = tx.send(Ok(axum::body::Bytes::from(sse_chunk(
                            &id,
                            created,
                            &sink_model,
                            serde_json::json!({"content": t}),
                            None,
                        ))));
                    }
                };
                state
                    .provider
                    .complete_with(model, &messages, &tools, &opts, &mut sink)
                    .await
            };

            match result {
                Ok(response) => {
                    // Emit any tool calls the model requested, then the terminal chunk.
                    let (_, finish_reason) = assistant_message_json(&response);
                    if response.wants_tools() {
                        let tool_calls: Vec<serde_json::Value> = response
                            .tool_calls
                            .iter()
                            .enumerate()
                            .map(|(i, c)| {
                                serde_json::json!({
                                    "index": i,
                                    "id": c.id,
                                    "type": "function",
                                    "function": {
                                        "name": c.name,
                                        "arguments": serde_json::to_string(&c.args).unwrap_or_else(|_| "{}".into()),
                                    },
                                })
                            })
                            .collect();
                        send(
                            &tx,
                            sse_chunk(
                                &id,
                                created,
                                model,
                                serde_json::json!({"tool_calls": tool_calls}),
                                None,
                            ),
                        );
                    }
                    send(
                        &tx,
                        sse_chunk(
                            &id,
                            created,
                            model,
                            serde_json::json!({}),
                            Some(finish_reason),
                        ),
                    );
                    succeeded = true;
                    break;
                }
                Err(e)
                    if e.is_retryable() && !emitted.load(std::sync::atomic::Ordering::Relaxed) =>
                {
                    // Nothing streamed yet — transparently try the next model.
                    last_err = format!("{model}: {e}");
                    continue;
                }
                Err(e) => {
                    // Either a permanent error, or a failure after partial output: report and stop.
                    last_err = format!("{model}: {e}");
                    break;
                }
            }
        }

        if !succeeded {
            send(&tx, sse_error(&last_err));
        }
        send(&tx, "data: [DONE]\n\n".to_string());
    });

    let stream = futures::stream::unfold(rx, |mut rx| async move {
        rx.recv().await.map(|item| (item, rx))
    });
    Response::builder()
        .status(StatusCode::OK)
        .header(axum::http::header::CONTENT_TYPE, "text/event-stream")
        .header(axum::http::header::CACHE_CONTROL, "no-cache")
        .header("x-accel-buffering", "no")
        .body(axum::body::Body::from_stream(stream))
        .unwrap()
}

/// One `data: {chat.completion.chunk}\n\n` SSE frame.
fn sse_chunk(
    id: &str,
    created: i64,
    model: &str,
    delta: serde_json::Value,
    finish_reason: Option<&str>,
) -> String {
    let chunk = serde_json::json!({
        "id": id,
        "object": "chat.completion.chunk",
        "created": created,
        "model": model,
        "choices": [{
            "index": 0,
            "delta": delta,
            "finish_reason": finish_reason,
        }],
    });
    format!("data: {chunk}\n\n")
}

fn sse_error(message: &str) -> String {
    let err = serde_json::json!({"error": {"message": message, "type": "api_error"}});
    format!("data: {err}\n\n")
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn completion_id() -> String {
    format!("chatcmpl-{}", forge_types::new_id())
}

/// An OpenAI-shaped error response (`{"error": {"message","type"}}`) with the given status.
fn openai_error(status: StatusCode, kind: &str, message: &str) -> Response {
    (
        status,
        axum::Json(serde_json::json!({
            "error": {
                "message": message,
                "type": kind,
            }
        })),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower::util::ServiceExt;

    fn mock_state() -> Arc<ApiState> {
        let config = forge_config::Config::default();
        let (provider, router) = crate::cli::commands::models::build_provider_and_router(
            &config,
            true,
            None,
            None,
            std::collections::HashMap::new(),
            std::collections::HashMap::new(),
        );
        Arc::new(ApiState {
            provider,
            router,
            pricing: forge_mesh::pricing::Pricing::from_config(&config),
            health: ModelHealth::default(),
            models: advertised_models(&config, None),
            api_key: None,
        })
    }

    fn post_chat(body: serde_json::Value) -> axum::http::Request<axum::body::Body> {
        axum::http::Request::post("/v1/chat/completions")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(body.to_string()))
            .unwrap()
    }

    async fn body_bytes(resp: Response) -> Vec<u8> {
        axum::body::to_bytes(resp.into_body(), 1 << 20)
            .await
            .unwrap()
            .to_vec()
    }

    #[tokio::test]
    async fn chat_completion_returns_openai_shaped_body() {
        let router = api_router(mock_state());
        let resp = router
            .oneshot(post_chat(serde_json::json!({
                "model": "auto",
                "messages": [{"role": "user", "content": "mock:code please show me a snippet"}],
            })))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let v: serde_json::Value = serde_json::from_slice(&body_bytes(resp).await).unwrap();
        assert_eq!(v["object"], "chat.completion");
        assert_eq!(v["choices"][0]["message"]["role"], "assistant");
        assert_eq!(v["choices"][0]["finish_reason"], "stop");
        let content = v["choices"][0]["message"]["content"].as_str().unwrap();
        assert!(
            content.contains("```rust"),
            "mock code answer streamed: {content}"
        );
        assert!(v["usage"]["total_tokens"].as_u64().unwrap() > 0);
        // Forge routing extension is present.
        assert!(v["x_forge"]["routed_model"].is_string());
    }

    #[tokio::test]
    async fn tool_calls_surface_with_tool_calls_finish_reason() {
        let router = api_router(mock_state());
        // The default mock turn (no special keyword) requests a `read_file` tool call.
        let resp = router
            .oneshot(post_chat(serde_json::json!({
                "messages": [{"role": "user", "content": "look at the project"}],
                "tools": [{"type": "function", "function": {"name": "read_file", "description": "read", "parameters": {"type": "object"}}}],
            })))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let v: serde_json::Value = serde_json::from_slice(&body_bytes(resp).await).unwrap();
        assert_eq!(v["choices"][0]["finish_reason"], "tool_calls");
        let calls = v["choices"][0]["message"]["tool_calls"].as_array().unwrap();
        assert_eq!(calls[0]["function"]["name"], "read_file");
        assert_eq!(calls[0]["type"], "function");
    }

    #[tokio::test]
    async fn streaming_emits_sse_chunks_then_done() {
        let router = api_router(mock_state());
        let resp = router
            .oneshot(post_chat(serde_json::json!({
                "model": "auto",
                "stream": true,
                "messages": [{"role": "user", "content": "mock:code snippet"}],
            })))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok()),
            Some("text/event-stream")
        );
        let text = String::from_utf8(body_bytes(resp).await).unwrap();
        // Role opener, at least one content delta, a terminal finish_reason, and the DONE sentinel.
        assert!(text.contains("chat.completion.chunk"));
        assert!(text.contains("\"role\":\"assistant\""));
        assert!(text.contains("\"content\":"));
        assert!(text.contains("\"finish_reason\":\"stop\""));
        assert!(text.trim_end().ends_with("data: [DONE]"));
    }

    #[tokio::test]
    async fn list_models_includes_auto_sentinel() {
        let router = api_router(mock_state());
        let resp = router
            .oneshot(
                axum::http::Request::get("/v1/models")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let v: serde_json::Value = serde_json::from_slice(&body_bytes(resp).await).unwrap();
        assert_eq!(v["object"], "list");
        let ids: Vec<&str> = v["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["id"].as_str().unwrap())
            .collect();
        assert!(ids.contains(&"auto"), "auto sentinel advertised: {ids:?}");
    }

    #[tokio::test]
    async fn empty_messages_is_a_400() {
        let router = api_router(mock_state());
        let resp = router
            .oneshot(post_chat(
                serde_json::json!({"model": "auto", "messages": []}),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let v: serde_json::Value = serde_json::from_slice(&body_bytes(resp).await).unwrap();
        assert_eq!(v["error"]["type"], "invalid_request_error");
    }

    #[tokio::test]
    async fn bearer_auth_is_enforced_when_a_key_is_set() {
        let mut st = mock_state();
        Arc::get_mut(&mut st).unwrap().api_key = Some("secret-key".to_string());
        let router = api_router(st);
        // No token → 401.
        let resp = router
            .clone()
            .oneshot(post_chat(serde_json::json!({
                "model": "auto",
                "messages": [{"role": "user", "content": "mock:code"}],
            })))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        // Correct token → 200.
        let ok = router
            .oneshot(
                axum::http::Request::post("/v1/chat/completions")
                    .header("content-type", "application/json")
                    .header("authorization", "Bearer secret-key")
                    .body(axum::body::Body::from(
                        serde_json::json!({
                            "model": "auto",
                            "messages": [{"role": "user", "content": "mock:code"}],
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(ok.status(), StatusCode::OK);
    }

    #[test]
    fn content_parts_flatten_to_text() {
        let v = serde_json::json!([
            {"type": "text", "text": "hello "},
            {"type": "text", "text": "world"},
        ]);
        assert_eq!(content_text(&v), "hello world");
        assert_eq!(content_text(&serde_json::json!("plain")), "plain");
        assert_eq!(content_text(&serde_json::Value::Null), "");
    }

    #[test]
    fn model_pin_treats_auto_as_unpinned() {
        assert_eq!(model_pin(&Some("auto".into())), None);
        assert_eq!(model_pin(&Some("mesh".into())), None);
        assert_eq!(model_pin(&None), None);
        assert_eq!(model_pin(&Some("".into())), None);
        assert_eq!(
            model_pin(&Some("groq::llama-3.3-70b".into())),
            Some("groq::llama-3.3-70b".into())
        );
    }
}
