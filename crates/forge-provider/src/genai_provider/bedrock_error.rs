//! Bedrock exception types.
//!
//! Bedrock names a failure by its exception type (`throttlingException`, `validationException`, …),
//! carried in the `:exception-type` stream header or the `x-amzn-ErrorType` HTTP header. The vendored
//! genai adapter folds it into the body as `{"error":{"type":..,"message":..}}`; this reads it back.
//! The message alone cannot tell the cases apart: a throttle, an outage and an over-long prompt all
//! arrive as a bare `{"message": "..."}`.

use super::*;

/// The Bedrock exception type of a structured error, lower-cased, when it is one this module knows.
pub(super) fn exception_kind(err: &serde_json::Value) -> Option<String> {
    let ty = err.get("type")?.as_str()?;
    let kind = ty
        .split(':')
        .next()
        .unwrap_or(ty)
        .trim()
        .to_ascii_lowercase();
    matches!(
        kind.as_str(),
        "throttlingexception"
            | "serviceunavailableexception"
            | "internalserverexception"
            | "modelstreamerrorexception"
            | "modelnotreadyexception"
            | "modeltimeoutexception"
            | "validationexception"
    )
    .then_some(kind)
}

/// Classify a known Bedrock exception. `kind` comes from [`exception_kind`].
pub(super) fn classify(
    kind: &str,
    message: String,
    retry_after: Option<std::time::Duration>,
) -> ProviderError {
    match kind {
        "throttlingexception" => ProviderError::RateLimited {
            message,
            retry_after,
        },
        "validationexception" => {
            // Never retried as-is. An over-long INPUT ("Input is too long for requested model")
            // still reaches the compact-and-retry path, because `ProviderError::is_context_overflow`
            // sniffs a `Request` message for that wording; every other validation failure is a
            // malformed request that fails identically on a retry, so it ends the turn.
            ProviderError::Request(message)
        }
        _ => ProviderError::Unavailable(message),
    }
}

/// Fold the `x-amzn-ErrorType` header of a non-streaming HTTP failure into its body, so the typed
/// classification above sees the same shape a streamed exception has. A body without the header (a
/// proxy's HTML error page, say) is returned untouched.
pub(super) fn body_with_error_type(header: Option<&str>, body: &str) -> String {
    let Some(ty) = header
        .map(|h| h.split(':').next().unwrap_or(h).trim())
        .filter(|t| !t.is_empty())
    else {
        return body.to_string();
    };
    let message = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| {
            ["message", "Message"]
                .iter()
                .find_map(|k| v.get(*k).and_then(|m| m.as_str()).map(str::to_string))
        })
        .unwrap_or_else(|| body.trim().to_string());
    serde_json::json!({ "error": { "type": ty, "message": message } }).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use genai::{adapter::AdapterKind, ModelIden};

    fn stream_error(ty: &str, message: &str) -> genai::Error {
        genai::Error::ChatResponse {
            model_iden: ModelIden::new(AdapterKind::BedrockApi, "global.moonshotai.kimi-k3"),
            body: serde_json::json!({"error": {"type": ty, "message": message}}),
        }
    }

    fn http_error(status: u16, header: Option<&str>, body: &str) -> genai::Error {
        genai::Error::HttpError {
            status: reqwest::StatusCode::from_u16(status).unwrap(),
            canonical_reason: String::new(),
            body: body_with_error_type(header, body),
        }
    }

    #[test]
    fn throttling_exception_is_a_rate_limit_whatever_the_prose() {
        let e = classify_genai_error(&stream_error(
            "throttlingException",
            "Too many tokens, please wait before trying again.",
        ));
        assert!(e.is_rate_limited(), "{e:?}");
        assert!(e.is_token_throttle());
        let e = classify_genai_error(&stream_error("throttlingException", "Slow down."));
        assert!(e.is_rate_limited(), "{e:?}");
        assert!(!e.is_token_throttle());
    }

    #[test]
    fn outage_exceptions_are_unavailable() {
        for ty in [
            "serviceUnavailableException",
            "internalServerException",
            "modelStreamErrorException",
            "modelNotReadyException",
        ] {
            let e = classify_genai_error(&stream_error(ty, "try again later"));
            assert!(matches!(e, ProviderError::Unavailable(_)), "{ty}: {e:?}");
        }
    }

    #[test]
    fn validation_exception_overflow_compacts_and_the_rest_ends_the_turn() {
        let long = classify_genai_error(&stream_error(
            "validationException",
            "Input is too long for requested model.",
        ));
        assert!(matches!(long, ProviderError::Request(_)), "{long:?}");
        assert!(long.is_context_overflow());
        let other = classify_genai_error(&stream_error(
            "validationException",
            "toolConfig.tools.0.toolSpec.name: invalid characters",
        ));
        assert!(matches!(other, ProviderError::Request(_)), "{other:?}");
        assert!(!other.is_context_overflow());
    }

    #[test]
    fn http_error_type_header_reaches_the_classifier() {
        let body = r#"{"message":"Too many tokens, please wait before trying again."}"#;
        let e = classify_genai_error(&http_error(
            429,
            Some("ThrottlingException:http://internal.amazon.com/coral/com.amazon.bedrock/"),
            body,
        ));
        assert!(e.is_token_throttle(), "{e:?}");
        let e = classify_genai_error(&http_error(
            503,
            Some("ServiceUnavailableException"),
            r#"{"message":"Bedrock is unable to process your request."}"#,
        ));
        assert!(matches!(e, ProviderError::Unavailable(_)), "{e:?}");
        // No header: the plain status mapping still applies and nothing is rewritten.
        let e = classify_genai_error(&http_error(429, None, body));
        assert!(e.is_rate_limited(), "{e:?}");
        assert_eq!(body_with_error_type(None, body), body);
    }

    #[test]
    fn profile_blip_stays_unavailable_on_every_path() {
        let msg = "ResourceNotFoundException: Inference Profile ARN not found";
        let e = classify_genai_error(&stream_error("resourceNotFoundException", msg));
        assert!(matches!(e, ProviderError::Unavailable(_)), "{e:?}");
        let e = classify_genai_error(&http_error(404, Some("ResourceNotFoundException"), msg));
        assert!(matches!(e, ProviderError::Unavailable(_)), "{e:?}");
    }

    #[test]
    fn a_request_id_containing_429_is_not_a_rate_limit() {
        // Bedrock stream failure with no usable type: the id carries `429`, the prose says nothing
        // about limits. It used to bench the pinned turn through six backoffs.
        let text = r#"{"message":"The model stream ended unexpectedly. Request ID: 6f1b429e-3c52-4d0a-9b7e-0a4291c3d8aa"}"#;
        let e = classify_text(text, text.into());
        assert!(matches!(e, ProviderError::Unavailable(_)), "{e:?}");
    }

    #[test]
    fn counts_and_units_containing_429_are_not_a_rate_limit() {
        for text in [
            "stream closed after 429 tokens",
            "received 1,429 bytes before reset",
            "reset after 429ms",
            "payload 429 bytes",
            "version 4.29 of the codec",
            "id req-429-abc",
            "ts 20260429T101500",
        ] {
            assert!(!has_status_429(&text.to_lowercase()), "{text}");
            assert!(
                matches!(
                    classify_text(text, text.into()),
                    ProviderError::Unavailable(_)
                ),
                "{text}"
            );
        }
    }

    #[test]
    fn a_standalone_429_status_still_counts() {
        for text in [
            "HTTP 429",
            "status: 429",
            "status=429",
            r#"{"code":429}"#,
            "429 Too Many Requests",
            "got a 429.",
            "error (429): slow down",
            "Request failed with status code 429",
        ] {
            assert!(has_status_429(&text.to_lowercase()), "{text}");
            assert!(classify_text(text, text.into()).is_rate_limited(), "{text}");
        }
    }

    #[test]
    fn quota_needs_exceeded_or_limit_wording() {
        let unrelated = "model stream error: could not read the quota settings file";
        assert!(matches!(
            classify_text(unrelated, unrelated.into()),
            ProviderError::Unavailable(_)
        ));
        for text in [
            "You exceeded your current quota",
            "quota exhausted for this key",
            "the daily quota limit was reached",
            "Quota exceeded for metric",
        ] {
            assert!(classify_text(text, text.into()).is_rate_limited(), "{text}");
        }
    }

    #[test]
    fn untyped_prose_needs_a_limit_word_to_read_as_a_rate_limit() {
        let text = r#"{"message":"Too many tokens, please wait before trying again."}"#;
        // The body has no `type`, so only the prose is left to read: `too many tokens` alone names
        // no limit word, and it must not be guessed into one.
        let e = classify_text(text, text.into());
        assert!(matches!(e, ProviderError::Unavailable(_)), "{e:?}");
        let e = classify_text("ThrottlingException: Too many requests", "x".into());
        assert!(e.is_rate_limited(), "{e:?}");
    }
}
