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

const GATEWAY_503: &str = r#"{"error":{"message":"Gateway is temporarily busy. The gateway cannot accept this request at the moment because its capacity is limited. Wait briefly and retry. Follow the Retry-After header when one is provided. (request id: 20260930abc)","type":"new_api_error","code":"system_cpu_overloaded","details":{"source":"gateway","category":"gateway_overloaded","retryable":true}}}"#;

#[test]
fn a_gateway_overload_is_busy_not_unavailable_and_keeps_its_text() {
    let err = classify_status(503, "HTTP 503".into(), GATEWAY_503, None);
    let ProviderError::Overloaded {
        status,
        reason,
        message,
        ..
    } = &err
    else {
        panic!("expected Overloaded, got {err:?}");
    };
    assert_eq!(*status, 503);
    assert_eq!(reason, "gateway_overloaded");
    assert!(message.contains("Wait briefly and retry"), "{message}");
    assert!(!message.contains("request id"), "{message}");
}

#[test]
fn overload_signals_are_recognised_on_any_5xx() {
    let cases: &[(u16, &str, Option<u64>)] = &[
        (
            529,
            r#"{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#,
            None,
        ),
        (503, "upstream connect error", None),
        (
            502,
            r#"{"error":{"message":"try later","retryable":true}}"#,
            None,
        ),
        (
            500,
            r#"{"error":{"message":"x","code":"server_busy"}}"#,
            None,
        ),
        (500, "Internal server error", Some(3)),
    ];
    for (code, body, retry_after) in cases {
        let err = classify_status(
            *code,
            "HTTP".into(),
            body,
            retry_after.map(std::time::Duration::from_secs),
        );
        assert!(err.is_overloaded(), "{code} {body}: {err:?}");
        assert_eq!(
            err.retry_after(),
            retry_after.map(std::time::Duration::from_secs)
        );
    }
}

#[test]
fn a_plain_500_stays_a_candidate_for_the_rejection_heuristic() {
    for code in [500, 502, 504] {
        let err = classify_status(code, "HTTP".into(), "Internal server error", None);
        assert!(
            matches!(err, ProviderError::Unavailable(_)),
            "{code}: {err:?}"
        );
    }
}

/// Texts captured from the Terminal-Bench run through ModelGateway (2026-10-31): the 503 that
/// ended pinned turns as "not available on this account", and its 502 sibling that already
/// retried. Plus the generic gateway phrasings the same fix covers.
#[test]
fn gateway_routing_failures_are_transient_never_a_dead_model() {
    let no_route_body = r#"{"error":{"message":"No provider route is available. The gateway could not find an available provider route for this model and request. Try again shortly, select another model or contact support. (request id: 2026abc)","type":"new_api_error","code":"model_not_available"}}"#;
    let cases: &[(u16, &str, &str, &str)] = &[
        (503, no_route_body, "HTTP error", "model_not_available"),
        (404, no_route_body, "HTTP error", "provider route"),
        (
            502,
            r#"{"error":{"message":"Every provider route failed. The gateway tried every available provider route for this model, and each one returned an error. Retry shortly or choose another model.","details":{"category":"provider_routes_exhausted"}}}"#,
            "HTTP error",
            "provider_routes_exhausted",
        ),
        (
            503,
            "503 No provider route",
            "503 No provider route",
            "no provider route",
        ),
        (
            504,
            "upstream unavailable",
            "upstream unavailable",
            "upstream unavailable",
        ),
        (502, "", "provider unavailable", "provider unavailable"),
        (
            503,
            "no healthy upstream",
            "no healthy upstream",
            "no healthy upstream",
        ),
        (
            503,
            "",
            "No available provider for model",
            "no available provider",
        ),
    ];
    for (status, body, message, _) in cases {
        let e = classify_status(*status, (*message).into(), body, None);
        let ProviderError::Overloaded { reason, .. } = &e else {
            panic!("{status} {body} {message} -> {e:?}");
        };
        assert!(!reason.is_empty());
        assert!(!e.is_permanent(), "{status} {body}");
    }
    let ProviderError::Overloaded { status, reason, .. } =
        classify_status(503, "HTTP error".into(), no_route_body, None)
    else {
        unreachable!()
    };
    assert_eq!((status, reason.as_str()), (503, "no provider route"));
}

#[test]
fn a_streamed_gateway_routing_failure_is_busy_too() {
    for text in [
        "API Error: 503 No provider route",
        "stream error: no provider route is available",
        "503 provider unavailable",
    ] {
        let e = classify_text(text, text.into());
        assert!(e.is_overloaded(), "{text} -> {e:?}");
    }
}

#[test]
fn the_model_owners_refusal_and_auth_and_billing_stay_permanent() {
    let cases: &[(u16, &str)] = &[
        (
            404,
            r#"{"error":{"message":"The model `x` does not exist or you do not have access to it.","code":"model_not_found"}}"#,
        ),
        (
            400,
            r#"{"error":{"message":"The requested model is not available on this account.","code":"model_not_available"}}"#,
        ),
        (
            401,
            r#"{"error":{"message":"Invalid API key. No provider route for anonymous callers."}}"#,
        ),
        (
            402,
            r#"{"error":{"message":"Insufficient balance; provider route disabled"}}"#,
        ),
    ];
    for (status, body) in cases {
        let e = classify_status(*status, "HTTP error".into(), body, None);
        assert!(e.is_permanent(), "{status} {body} -> {e:?}");
    }
}
