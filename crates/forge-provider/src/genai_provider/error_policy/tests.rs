use super::*;

/// Every provider's own way of saying "no such model" is a permanent no-access verdict, so a pinned
/// turn fails at once instead of retrying or waiting out an outage budget.
#[test]
fn each_providers_unknown_model_answer_is_permanent() {
    let cases: &[(u16, &str)] = &[
        (
            400,
            r#"{"error":{"message":"Model Not Exist","type":"invalid_request_error","code":"invalid_request_error"}}"#,
        ),
        (
            400,
            r#"{"object":"error","message":"Invalid model: foo","type":"invalid_request_error"}"#,
        ),
        (
            404,
            r#"{"error":{"code":404,"message":"models/gemini-9 is not found for API version v1beta, or is not supported for generateContent.","status":"NOT_FOUND"}}"#,
        ),
        (
            404,
            r#"{"error":{"message":"model \"qwen9\" not found, try pulling it first","type":"api_error"}}"#,
        ),
        (
            404,
            r#"{"error":{"message":"Unable to access model acme/x. Please visit the models page.","code":"model_not_available"}}"#,
        ),
        (
            404,
            r#"{"code":"Some requested entity was not found","error":"The model grok-9 does not exist or your team 1234 does not have access to it."}"#,
        ),
        (
            404,
            r#"{"error":{"code":"DeploymentNotFound","message":"The API deployment for this resource does not exist."}}"#,
        ),
    ];
    for (status, body) in cases {
        let e = classify_status(*status, "HTTP error".into(), body, None);
        assert!(
            matches!(e, ProviderError::NoModelAccess(_)),
            "{body} -> {e:?}"
        );
        assert!(e.is_permanent(), "{body}");
    }
}

#[test]
fn an_unrelated_client_error_is_still_not_a_dead_model() {
    for body in [
        r#"{"error":{"message":"Invalid JSON payload received."}}"#,
        r#"{"error":{"message":"messages: Field required"}}"#,
    ] {
        let e = classify_status(400, "HTTP error".into(), body, None);
        assert!(matches!(e, ProviderError::Request(_)), "{body} -> {e:?}");
    }
}

/// The stream wrapper's first line only names the model (`short` keeps one line), so the cause that
/// says nothing answered has to survive in the message.
#[test]
fn a_refused_connection_keeps_its_cause() {
    let cause = "error sending request for url (http://localhost:11434/v1/chat/completions)";
    let e = classify_text(cause, "'nonexistent-model-xyz (adapter: OpenAI)'.".into());
    let ProviderError::Unavailable(message) = &e else {
        panic!("expected Unavailable, got {e:?}");
    };
    assert!(message.contains("could not connect"), "{message}");
    assert!(message.contains("localhost:11434"), "{message}");
    assert!(
        !e.is_permanent(),
        "a dropped connection stays transient on its own"
    );
}

#[test]
fn a_dropped_stream_without_a_transport_cause_is_unchanged() {
    let e = classify_text("stream ended early", "stream dropped".into());
    assert!(
        matches!(&e, ProviderError::Unavailable(m) if m == "stream dropped"),
        "{e:?}"
    );
}

#[test]
fn only_ids_nothing_recognises_fall_back_to_local_ollama() {
    for id in ["nonexistent-model-xyz", "llama3.2:3b", "qwen3-coder"] {
        assert!(falls_back_to_local_ollama(id), "{id}");
    }
    for id in [
        "gpt-5.6-luna",
        "claude-sonnet-5",
        "gemini-3-pro",
        "deepseek-v4",
        "grok-5",
        "groq::llama-3.3-70b-versatile",
        "ollama::llama3.2",
    ] {
        assert!(!falls_back_to_local_ollama(id), "{id}");
    }
}

/// The live S11 defect: a bare unknown id went to the local Ollama default, nothing was listening,
/// and the pinned turn then waited a 5 s backoff before the second identical failure ended it.
#[test]
fn an_unrecognised_id_with_no_server_is_permanent_not_an_outage() {
    let refused = ProviderError::Unavailable(
        "could not connect: error sending request for url (http://localhost:11434/v1/chat/completions)"
            .into(),
    );
    let e = explain_unrecognized_model("nonexistent-model-xyz", refused);
    assert!(matches!(e, ProviderError::NoModelAccess(_)), "{e:?}");
    assert!(e.is_permanent());
    let text = e.to_string();
    assert!(text.contains("nonexistent-model-xyz"), "{text}");
    assert!(text.contains("no local Ollama answered"), "{text}");
}

#[test]
fn other_failures_pass_through_unchanged() {
    let refused = || {
        ProviderError::Unavailable("could not connect: error sending request for url (x)".into())
    };
    // A recognised id, an explicit `ollama::` pin, a real provider outage: all keep their class.
    for id in ["gpt-5.6-luna", "ollama::llama3.2", "groq::x"] {
        let e = explain_unrecognized_model(id, refused());
        assert!(matches!(e, ProviderError::Unavailable(_)), "{id}: {e:?}");
    }
    let bad_gateway = ProviderError::Unavailable("502 bad gateway".into());
    let e = explain_unrecognized_model("nonexistent-model-xyz", bad_gateway);
    assert!(matches!(e, ProviderError::Unavailable(_)), "{e:?}");
    let limited = ProviderError::RateLimited {
        message: "slow down".into(),
        retry_after: None,
    };
    let e = explain_unrecognized_model("nonexistent-model-xyz", limited);
    assert!(matches!(e, ProviderError::RateLimited { .. }), "{e:?}");
}
