//! Layer 2 — HTTP contract tests for `GenAiProvider` (FR-3). They point genai at a local
//! `httpmock` server via a service-target resolver (loopback endpoint + dummy auth), so the
//! *real* genai adapter builds and parses real HTTP/SSE — but no API key is used and no byte
//! leaves the machine. This exercises the streaming/usage/tool-call branches that the
//! `MockProvider` tests cannot reach.

use forge_provider::{
    CompletionOptions, GenAiProvider, Provider, ProviderError, StreamEvent, ToolSpec,
};
use forge_types::Message;
use genai::resolver::{AuthData, Endpoint};
use genai::{Client, ServiceTarget};
use httpmock::prelude::*;
use serde_json::json;

/// Build a genai client whose every request is redirected to `base` with a throwaway key.
fn client_pointed_at(base: String) -> Client {
    Client::builder()
        .with_service_target_resolver_fn(move |mut t: ServiceTarget| {
            t.endpoint = Endpoint::from_owned(format!("{base}/"));
            t.auth = AuthData::from_single("test-key");
            Ok::<ServiceTarget, genai::resolver::Error>(t)
        })
        .build()
}

const SSE_CT: &str = "text/event-stream";

#[tokio::test]
async fn streaming_accumulates_deltas_and_usage() {
    let server = MockServer::start_async().await;
    let body = concat!(
        "data: {\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"Hel\"},\"finish_reason\":null}]}\n\n",
        "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"lo\"},\"finish_reason\":null}]}\n\n",
        "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
        "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":5,\"completion_tokens\":2,\"total_tokens\":7}}\n\n",
        "data: [DONE]\n\n",
    );
    let _m = server.mock(|when, then| {
        when.method(POST).path("/chat/completions");
        then.status(200).header("content-type", SSE_CT).body(body);
    });

    let provider = GenAiProvider::with_client(client_pointed_at(server.base_url()));
    let mut sink = String::new();
    let res = provider
        .complete(
            "openai::gpt-4o-mini",
            &[Message::user("hi")],
            &[],
            &mut |ev| {
                if let StreamEvent::Text(t) = ev {
                    sink.push_str(&t)
                }
            },
        )
        .await
        .expect("complete should succeed against the mock");

    assert_eq!(sink, "Hello", "deltas streamed to the sink in order");
    assert_eq!(res.content, "Hello", "content is the concatenation");
    assert_eq!(res.usage.input_tokens, 5);
    assert_eq!(res.usage.output_tokens, 2);
    assert!(res.tool_calls.is_empty());
}

#[tokio::test]
async fn tool_call_is_translated() {
    let server = MockServer::start_async().await;
    let body = concat!(
        "data: {\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"tool_calls\":[{\"index\":0,\"id\":\"call_abc\",\"type\":\"function\",\"function\":{\"name\":\"read_file\",\"arguments\":\"{\\\"path\\\":\\\"\"}}]},\"finish_reason\":null}]}\n\n",
        "data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"Cargo.toml\"}}]},\"finish_reason\":null}]}\n\n",
        "data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"\\\"}\"}}]},\"finish_reason\":null}]}\n\n",
        "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n",
        "data: [DONE]\n\n",
    );
    let _m = server.mock(|when, then| {
        when.method(POST).path("/chat/completions");
        then.status(200).header("content-type", SSE_CT).body(body);
    });

    let tools = [ToolSpec {
        name: "read_file".into(),
        description: "read".into(),
        schema: json!({"type":"object","properties":{"path":{"type":"string"}}}),
    }];
    let provider = GenAiProvider::with_client(client_pointed_at(server.base_url()));
    let mut provider_activity = 0usize;
    let res = provider
        .complete(
            "openai::gpt-4o-mini",
            &[Message::user("read it")],
            &tools,
            &mut |event| {
                if event == StreamEvent::ProviderActivity {
                    provider_activity += 1;
                }
            },
        )
        .await
        .expect("complete should succeed");

    assert!(res.wants_tools(), "a tool call was requested");
    assert_eq!(res.tool_calls.len(), 1);
    let call = &res.tool_calls[0];
    assert_eq!(call.id, "call_abc");
    assert_eq!(call.name, "read_file");
    assert_eq!(call.args["path"], "Cargo.toml");
    assert!(
        provider_activity >= 3,
        "every buffered tool-argument delta should keep the caller's watchdog alive"
    );
}

#[tokio::test]
async fn http_500_maps_to_unavailable_for_failover() {
    // A 5xx is a transient provider problem → classified Unavailable (retryable) so the mesh
    // benches the model and fails over (docs/features/mesh-routing.md), not a hard Request failure.
    let server = MockServer::start_async().await;
    let _m = server.mock(|when, then| {
        when.method(POST).path("/chat/completions");
        then.status(500).body("boom");
    });

    let provider = GenAiProvider::with_client(client_pointed_at(server.base_url()));
    let err = provider
        .complete(
            "openai::gpt-4o-mini",
            &[Message::user("hi")],
            &[],
            &mut |_| {},
        )
        .await
        .expect_err("a 500 must surface as an error");
    assert!(err.is_retryable(), "5xx should be retryable: {err:?}");
    assert!(matches!(err, ProviderError::Unavailable(_)), "got {err:?}");
}

#[tokio::test]
async fn strict_openai_gateway_retries_without_prompt_cache_key() {
    let server = MockServer::start_async().await;
    let rejected = server.mock(|when, then| {
        when.method(POST)
            .path("/chat/completions")
            .body_includes("prompt_cache_key");
        then.status(400)
            .header("content-type", "application/json")
            .json_body(json!({
                "error": {
                    "type": "invalid_request_error",
                    "message": "Unsupported parameter: prompt_cache_key"
                }
            }));
    });
    let accepted = server.mock(|when, then| {
        when.method(POST)
            .path("/chat/completions")
            .body_excludes("prompt_cache_key");
        then.status(200).header("content-type", SSE_CT).body(concat!(
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"ok\"},\"finish_reason\":null}]}\n\n",
            "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":5,\"completion_tokens\":1,\"total_tokens\":6}}\n\n",
            "data: [DONE]\n\n",
        ));
    });

    let provider = GenAiProvider::with_client(client_pointed_at(server.base_url()));
    let result = provider
        .complete_with(
            "openai::strict-compatible-model",
            &[Message::user("hi")],
            &[],
            &CompletionOptions {
                frequency_penalty: None,
                presence_penalty: None,
                prompt_cache_key: Some("forge-session-1".into()),
                ..Default::default()
            },
            &mut |_| {},
        )
        .await;
    let response = result.unwrap_or_else(|error| {
        panic!(
            "unsupported cache field should degrade to the compatible request; error={error:?}, rejected_calls={}, accepted_calls={}",
            rejected.calls(),
            accepted.calls()
        )
    });

    assert_eq!(response.content, "ok");
    assert_eq!(rejected.calls(), 1);
    assert_eq!(accepted.calls(), 1);
}

/// A loopback server that answers the Nth request with the Nth scripted SSE body (the last one
/// repeats) and records every request body — what `httpmock` cannot do, since identical retries
/// match the same mock.
async fn scripted_sse_server(
    bodies: Vec<&'static str>,
) -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let recorded = seen.clone();
    tokio::spawn(async move {
        let mut n = 0usize;
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                return;
            };
            let mut buf = Vec::new();
            let mut chunk = [0u8; 8192];
            let body_start = loop {
                let read = sock.read(&mut chunk).await.unwrap_or(0);
                if read == 0 {
                    break None;
                }
                buf.extend_from_slice(&chunk[..read]);
                if let Some(at) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    break Some(at + 4);
                }
            };
            let Some(start) = body_start else { continue };
            let head = String::from_utf8_lossy(&buf[..start]).to_lowercase();
            let want = head
                .lines()
                .find_map(|l| l.strip_prefix("content-length:"))
                .and_then(|v| v.trim().parse::<usize>().ok())
                .unwrap_or(0);
            while buf.len() < start + want {
                let read = sock.read(&mut chunk).await.unwrap_or(0);
                if read == 0 {
                    break;
                }
                buf.extend_from_slice(&chunk[..read]);
            }
            recorded
                .lock()
                .unwrap()
                .push(String::from_utf8_lossy(&buf[start..]).into_owned());
            let body = bodies[n.min(bodies.len() - 1)];
            n += 1;
            let reply = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: {SSE_CT}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = sock.write_all(reply.as_bytes()).await;
            let _ = sock.shutdown().await;
        }
    });
    (base, seen)
}

/// A reply that is blank — a role-only delta, then `stop`, as Kimi K3 sent 40 times in 60 days —
/// is retried once with the identical request instead of reaching the agent loop's nudge path.
const BLANK_STOP: &str = concat!(
    "data: {\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"\"},\"finish_reason\":null}]}\n\n",
    "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
    "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":9,\"completion_tokens\":3,\"total_tokens\":12}}\n\n",
    "data: [DONE]\n\n",
);
const ANSWER: &str = concat!(
    "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"done\"},\"finish_reason\":null}]}\n\n",
    "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
    "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":9,\"completion_tokens\":1,\"total_tokens\":10}}\n\n",
    "data: [DONE]\n\n",
);

fn one_tool() -> [ToolSpec; 1] {
    [ToolSpec {
        name: "read_file".into(),
        description: "read".into(),
        schema: json!({"type":"object","properties":{"path":{"type":"string"}}}),
    }]
}

#[tokio::test]
async fn a_blank_reply_with_tools_on_offer_is_resampled_once() {
    let (base, seen) = scripted_sse_server(vec![BLANK_STOP, ANSWER]).await;
    let provider = GenAiProvider::with_client(client_pointed_at(base));
    let res = provider
        .complete(
            "openai::gpt-4o-mini",
            &[Message::user("go")],
            &one_tool(),
            &mut |_| {},
        )
        .await
        .expect("the retry answers");
    assert_eq!(res.content, "done");
    let bodies = seen.lock().unwrap();
    assert_eq!(bodies.len(), 2, "exactly one retry");
    assert_eq!(bodies[0], bodies[1], "the retry is the identical request");
    assert_eq!(
        res.usage.output_tokens, 4,
        "the blank attempt's spend is kept"
    );
}

#[tokio::test]
async fn a_model_that_stays_blank_is_handed_back_after_one_retry() {
    let (base, seen) = scripted_sse_server(vec![BLANK_STOP]).await;
    let provider = GenAiProvider::with_client(client_pointed_at(base));
    let res = provider
        .complete(
            "openai::gpt-4o-mini",
            &[Message::user("go")],
            &one_tool(),
            &mut |_| {},
        )
        .await
        .expect("still a response; the agent loop owns the nudge");
    assert!(res.content.is_empty() && res.tool_calls.is_empty());
    assert_eq!(seen.lock().unwrap().len(), 2, "no retry storm");
}

#[tokio::test]
async fn a_blank_reply_without_tools_is_not_retried() {
    let (base, seen) = scripted_sse_server(vec![BLANK_STOP, ANSWER]).await;
    let provider = GenAiProvider::with_client(client_pointed_at(base));
    let res = provider
        .complete(
            "openai::gpt-4o-mini",
            &[Message::user("go")],
            &[],
            &mut |_| {},
        )
        .await
        .unwrap();
    assert!(res.content.is_empty());
    assert_eq!(seen.lock().unwrap().len(), 1);
}

/// The nudge that re-drives a stopped model must be the LAST thing in the request. Hoisted to the
/// system block it left the request ending on the model's own reply, and moved every byte after it,
/// so the provider's prefix cache read 9-46% of input instead of 96-98%.
#[tokio::test]
async fn a_mid_transcript_nudge_is_sent_last_and_in_place() {
    let (base, seen) = scripted_sse_server(vec![ANSWER]).await;
    let provider = GenAiProvider::with_client(client_pointed_at(base));
    let transcript = [
        Message::system("base prompt"),
        Message::user("fix it"),
        Message::assistant("All done."),
        Message::system("You ended your reply, but tasks on your list are NOT yet Done."),
    ];
    provider
        .complete("openai::gpt-4o-mini", &transcript, &[], &mut |_| {})
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_str(&seen.lock().unwrap()[0]).unwrap();
    let sent = body["messages"].as_array().unwrap();
    let roles: Vec<&str> = sent.iter().map(|m| m["role"].as_str().unwrap()).collect();
    assert_eq!(
        roles,
        ["system", "user", "assistant", "user"],
        "got {roles:?}"
    );
    assert!(sent[3]["content"]
        .as_str()
        .unwrap()
        .contains("NOT yet Done"));
}
