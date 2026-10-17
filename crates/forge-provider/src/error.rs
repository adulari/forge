use std::time::Duration;

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    /// A non-retryable failure: bad request, malformed response, context-length, etc. It
    /// would fail the same way on any model, so the mesh must NOT fail over on it.
    #[error("provider request failed: {0}")]
    Request(String),
    /// Rate-limited / out of quota (HTTP 429, `RESOURCE_EXHAUSTED`). Retryable on another
    /// model; `retry_after` carries the server's cooldown when it told us one.
    #[error("rate limited: {message}")]
    RateLimited {
        message: String,
        retry_after: Option<Duration>,
    },
    /// The provider is down / the stream dropped (5xx, connection/timeout). Retryable.
    #[error("provider unavailable: {0}")]
    Unavailable(String),
    /// Authentication failed (HTTP 401/403) — the key is bad, missing, or lacks access. Failing
    /// over to *another* provider is correct, but the bad credential won't fix itself mid-session,
    /// so retrying THIS model auth-fails identically every turn (the per-turn failover churn). Like
    /// [`Capability`](Self::Capability) it's treated as PERMANENT: excluded on the long window +
    /// periodic re-probe (so it recovers automatically once the user fixes the key).
    #[error("provider auth failed: {0}")]
    Auth(String),
    /// A PERMANENT, model-specific incapability: this model can't serve Forge's (tool-using)
    /// turns at all — it rejects function calling, has no tool-supporting endpoint, mangles tool
    /// params, or the account can't afford it (HTTP 402 / "requires more credits"). Failing over
    /// to *another* model is correct, but retrying THIS one will fail identically every time, so
    /// the mesh excludes it (a long bench window) rather than benching it on a short cooldown.
    #[error("model unsupported: {0}")]
    Capability(String),
    /// This ACCOUNT cannot call this MODEL: the id was never valid, was decommissioned, or is
    /// gated behind a tier this key doesn't have (`model_not_found` — groq's "does not exist or
    /// you do not have access to it"). Permanent like [`Capability`](Self::Capability) and scoped
    /// the same way — only the model is excluded, since the provider's other models keep working —
    /// but a distinct variant because the cause and the fix differ: nothing about Forge's payload
    /// is wrong, the model simply isn't callable here. Previously classified as
    /// [`Request`](Self::Request), which is non-retryable AND never benched, so the mesh kept
    /// routing to a dead model turn after turn.
    #[error("model not available on this account: {0}")]
    NoModelAccess(String),
}

impl ProviderError {
    /// Whether the mesh should bench this model and fail over to another. True for
    /// rate-limit / unavailable / auth / capability / no-access; false for
    /// [`Request`](Self::Request) (would fail identically everywhere).
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::RateLimited { .. }
                | Self::Unavailable(_)
                | Self::Auth(_)
                | Self::Capability(_)
                | Self::NoModelAccess(_)
        )
    }

    /// Whether this failure is PERMANENT for the model: it will recur on every call, so the model
    /// should be *excluded* (a long bench window + periodic re-probe), not benched on the short
    /// transient cooldown. True for [`Capability`](Self::Capability) (the model can't serve
    /// tool-using turns), [`Auth`](Self::Auth) (the credential is bad/missing and won't fix
    /// itself mid-session), and [`NoModelAccess`](Self::NoModelAccess) (the account can't call
    /// this model id at all) — each fails identically on every turn otherwise.
    pub fn is_permanent(&self) -> bool {
        matches!(
            self,
            Self::Capability(_) | Self::Auth(_) | Self::NoModelAccess(_)
        )
    }

    /// Whether this is a rate-limit / quota-exhaustion failure (HTTP 429, `RESOURCE_EXHAUSTED`).
    /// Used by the failover loop to lazily skip the *same provider's* remaining chain entries
    /// after one of its models 429s — a rate limit is usually provider-wide, so the siblings would
    /// 429 too. Every other failure mode keeps strict mesh-rank failover order.
    pub fn is_rate_limited(&self) -> bool {
        matches!(self, Self::RateLimited { .. })
    }

    /// How long to bench the model: the server-provided `retry_after` when present,
    /// otherwise `default`.
    pub fn cooldown(&self, default: Duration) -> Duration {
        match self {
            Self::RateLimited {
                retry_after: Some(d),
                ..
            } => *d,
            _ => default,
        }
    }

    /// Heuristic: whether this failure is a context-length OVERFLOW (the prompt exceeded the
    /// model's window) rather than a genuine outage. Providers surface overflow inconsistently —
    /// often as a 4xx/5xx the generic classifier files under [`Unavailable`](Self::Unavailable) or
    /// [`Request`](Self::Request) — so we sniff the message. The correct response is to SHRINK the
    /// input (compact/trim) and retry the SAME model, not to bench a healthy model and fail over.
    pub fn is_context_overflow(&self) -> bool {
        let msg = match self {
            Self::Unavailable(m) | Self::Request(m) => m,
            Self::RateLimited { message, .. } => message,
            _ => return false,
        };
        let m = msg.to_lowercase();
        // A token-per-minute throttle ("Too many tokens, please wait before trying again") is a
        // RATE LIMIT that happens to name tokens, not an over-window prompt: the pinned-turn
        // backoff owns it and compacts only once waiting has demonstrably not helped.
        if self.is_token_throttle() {
            return false;
        }
        [
            "context length",
            "context window",
            "context_length",
            "maximum context",
            "maximum number of tokens",
            "too many tokens",
            "too many input tokens",
            "input is too long",
            "input token count",
            "reduce the length",
            "prompt is too long",
            "input is too large",
            "exceeds the maximum",
            "string too long",
            "request too large",
            "reduce your message size",
            "payload too large",
            "entity too large",
        ]
        .iter()
        .any(|k| m.contains(k))
    }

    /// Whether this is a rate limit on TOKENS (Bedrock's `Too many tokens, please wait before trying
    /// again`, OpenAI's `tokens per min (TPM)`) rather than on requests. Only a token throttle can be
    /// helped by sending less: a request-rate throttle clears with time alone.
    pub fn is_token_throttle(&self) -> bool {
        let Self::RateLimited { message, .. } = self else {
            return false;
        };
        let m = message.to_lowercase();
        // `request too large` is groq's over-the-per-request-cap wording; the overflow path owns it.
        !m.contains("request too large")
            && [
                "too many tokens",
                "tokens per min",
                "token rate limit",
                "(tpm)",
                "tpm limit",
            ]
            .iter()
            .any(|k| m.contains(k))
    }

    /// A short reason string for the health record / UI ("rate-limited (429)", …).
    pub fn reason(&self) -> &'static str {
        match self {
            Self::RateLimited { .. } => "rate-limited",
            Self::Unavailable(_) => "unavailable",
            Self::Auth(_) => "auth failed",
            Self::Request(_) => "request error",
            Self::Capability(_) => "unsupported (no tool calling / unaffordable)",
            Self::NoModelAccess(_) => "not available on this account",
        }
    }
}

#[cfg(test)]
mod overflow_413_tests {
    use super::*;

    #[test]
    fn a_token_throttle_is_a_rate_limit_not_an_overflow() {
        let throttle = ProviderError::RateLimited {
            message: "Too many tokens, please wait before trying again.".into(),
            retry_after: None,
        };
        assert!(throttle.is_token_throttle());
        assert!(!throttle.is_context_overflow());
        let requests = ProviderError::RateLimited {
            message: "Too many requests, please wait before trying again.".into(),
            retry_after: None,
        };
        assert!(!requests.is_token_throttle());
        assert!(!requests.is_context_overflow());
        let tpm = ProviderError::RateLimited {
            message: "Rate limit reached on tokens per min (TPM): Limit 10000, Used 9000".into(),
            retry_after: None,
        };
        assert!(tpm.is_token_throttle());
        // Over the PER-REQUEST cap is still an overflow, and a 400 "too many tokens" stays one.
        assert!(ProviderError::RateLimited {
            message: "Request too large for model on tokens per minute (TPM)".into(),
            retry_after: None,
        }
        .is_context_overflow());
        assert!(
            ProviderError::Request("too many tokens in the prompt".into()).is_context_overflow()
        );
        assert!(!ProviderError::Request("too many tokens".into()).is_token_throttle());
    }

    #[test]
    fn bedrock_input_too_long_wording_is_an_overflow() {
        for m in [
            "Input is too long for requested model.",
            "Too many input tokens. Max input tokens: 262144, request input token count: 300000",
        ] {
            assert!(
                ProviderError::Request(m.into()).is_context_overflow(),
                "{m}"
            );
        }
    }

    #[test]
    fn request_too_large_counts_as_context_overflow() {
        for m in [
            "Request too large for model: Limit 8000, Requested 8598",
            "please reduce your message size and try again",
            "413 Payload Too Large",
        ] {
            assert!(
                ProviderError::Request(m.into()).is_context_overflow(),
                "{m}"
            );
        }
    }
}
