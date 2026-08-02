//! Provider error classification, retry timing, and capability policy.

use super::*;

pub(super) fn classify_genai_error(err: &genai::Error) -> ProviderError {
    use genai::webc::Error as WebcError;
    match err {
        genai::Error::HttpError { status, body, .. } => classify_status(
            status.as_u16(),
            err.to_string(),
            body,
            parse_retry_after_body(body),
        ),
        genai::Error::WebModelCall { webc_error, .. }
        | genai::Error::WebAdapterCall { webc_error, .. } => match webc_error {
            WebcError::ResponseFailedStatus {
                status,
                body,
                headers,
            } => {
                // `Retry-After` header (delta-seconds), else the body's `retryDelay`.
                let retry_after = headers
                    .get("retry-after")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| {
                        let t = v.trim();
                        t.parse::<u64>()
                            .ok()
                            .map(std::time::Duration::from_secs)
                            .or_else(|| parse_secs(t).and_then(duration_from_secs))
                    })
                    .or_else(|| parse_retry_after_body(body));
                classify_status(status.as_u16(), err.to_string(), body, retry_after)
            }
            other => ProviderError::Unavailable(short(&other.to_string())),
        },
        // Streaming path: genai gives no typed HTTP status, only a string. Prefer a STRUCTURED read
        // when the cause is (or embeds) a JSON error body — classify on `error.code`/`error.status`/
        // `error.type` instead of substring-guessing — and fall back to text scanning otherwise.
        genai::Error::WebStream { cause, error, .. } => {
            // WebStream boxes the initial HTTP failure (including status + response body). Preserve
            // that typed error instead of collapsing it to a generic stream outage; cache-field
            // compatibility fallback and normal 4xx/429/5xx classification both depend on it.
            if let Some(inner) = error.downcast_ref::<genai::Error>() {
                classify_genai_error(inner)
            } else {
                parse_embedded_json(cause)
                    .as_ref()
                    .and_then(classify_error_body)
                    .unwrap_or_else(|| classify_text(cause, err.to_string()))
            }
        }
        // In-stream error event with a STRUCTURED JSON body — classify on its typed fields first.
        genai::Error::ChatResponse { body, .. } => classify_error_body(body)
            .unwrap_or_else(|| classify_text(&body.to_string(), err.to_string())),
        // A bad/truncated stream chunk — transient, worth trying elsewhere.
        genai::Error::StreamParse { .. } => ProviderError::Unavailable(short(&err.to_string())),
        other => {
            let s = other.to_string();
            // A genai "Resolver error" (adapter/auth couldn't be built — almost always a missing
            // API key) is PERMANENT for this turn: retrying dispatches the same keyless model and
            // fails identically. Class it as Auth so the mesh EXCLUDES it (long bench + periodic
            // re-probe) instead of surfacing the raw "Resolver error for model 'groq::…'" and, on
            // the last-resort path, re-benching it forever.
            if is_auth_config_failure(&s) {
                ProviderError::Auth(short(&s))
            } else {
                ProviderError::Request(short(&s))
            }
        }
    }
}

/// Classify a provider error from its STRUCTURED JSON body — the typed signal genai exposes on the
/// `ChatResponse` stream-error path (and that some providers embed in a `WebStream` cause) — instead
/// of substring-matching the stringified form. Reads the shapes real providers actually emit:
///   - a numeric HTTP-ish `error.code` (OpenAI/Gemini `429`) → reuse [`classify_status`] (most
///     reliable: the same code-based path the typed HTTP errors take);
///   - Google's `error.status` enum (`RESOURCE_EXHAUSTED` / `UNAUTHENTICATED` / `UNAVAILABLE` / …);
///   - OpenAI/Anthropic string `error.code` / `error.type` (`rate_limit_exceeded`,
///     `insufficient_quota`, `rate_limit_error`, `overloaded_error`, `authentication_error`, …).
///
/// Returns `None` when the body isn't one of these shapes, so the caller falls back to text scanning.
/// A provider tweaking its prose no longer silently breaks classification — the typed field still
/// carries the signal, and the per-provider contract tests assert each shape.
pub(super) fn classify_error_body(body: &serde_json::Value) -> Option<ProviderError> {
    let err = body.get("error").unwrap_or(body);
    let raw = body.to_string();
    let msg = err
        .get("message")
        .and_then(|m| m.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| raw.clone());
    // Permanent incapability / payment markers win first (mirrors `classify_status`' ordering), so a
    // "requires more credits" / "function calling not supported" body is EXCLUDED, not retried.
    // A throttling-flavoured quota (Alibaba `Throttling.AllocationQuota`) is TRANSIENT though —
    // route it to RateLimited before the capability markers, which would otherwise misread the
    // `insufficient_quota` marker it also carries.
    if is_throttling_failure(&raw) {
        return Some(ProviderError::RateLimited {
            message: short(&msg),
            retry_after: parse_retry_after_body(&raw),
        });
    }
    if is_capability_failure(&raw) {
        return Some(ProviderError::Capability(short(&msg)));
    }
    // 1. Numeric HTTP code — delegate to the shared status classifier (handles 402/429/401/5xx/…).
    if let Some(code) = err.get("code").and_then(json_status_code) {
        return Some(classify_status(
            code,
            msg,
            &raw,
            parse_retry_after_body(&raw),
        ));
    }
    let m = short(&msg);
    // 2. Google RPC status enum.
    if let Some(status) = err.get("status").and_then(|s| s.as_str()) {
        match status {
            "RESOURCE_EXHAUSTED" => {
                return Some(ProviderError::RateLimited {
                    message: m,
                    retry_after: parse_retry_after_body(&raw).filter(|_| !quota_is_exhausted(&raw)),
                })
            }
            "UNAUTHENTICATED" | "PERMISSION_DENIED" => return Some(ProviderError::Auth(m)),
            "UNAVAILABLE" | "INTERNAL" | "DEADLINE_EXCEEDED" => {
                return Some(ProviderError::Unavailable(m))
            }
            _ => {}
        }
    }
    // 3. OpenAI/Anthropic string code or type.
    let code_type = err
        .get("code")
        .and_then(|c| c.as_str())
        .or_else(|| err.get("type").and_then(|t| t.as_str()));
    if let Some(ct) = code_type {
        let l = ct.to_lowercase();
        // A throttling-flavoured quota (Alibaba `Throttling.AllocationQuota`) is TRANSIENT —
        // back off and retry, never a permanent capability. Checked before the `insufficient_quota`
        // marker below, which Alibaba's OpenAI-compatible error also carries.
        if is_throttling_failure(&l) {
            return Some(ProviderError::RateLimited {
                message: m,
                retry_after: parse_retry_after_body(&raw),
            });
        }
        if l.contains("rate_limit") || l.contains("resource_exhausted") || l.contains("overloaded")
        {
            return Some(ProviderError::RateLimited {
                message: m,
                retry_after: parse_retry_after_body(&raw),
            });
        }
        if l.contains("insufficient_quota") || l.contains("billing") || l.contains("payment") {
            return Some(ProviderError::Capability(m));
        }
        if l.contains("authentication")
            || l.contains("invalid_api_key")
            || l.contains("unauthorized")
            || l.contains("permission")
        {
            return Some(ProviderError::Auth(m));
        }
    }
    None
}

/// Read a JSON value as an HTTP status code: an integer (`429`) or a numeric string (`"429"`),
/// bounded to a plausible 1xx–5xx range so a random `code: 0` / `code: 20000` isn't misread.
pub(super) fn json_status_code(v: &serde_json::Value) -> Option<u16> {
    v.as_u64()
        .and_then(|n| u16::try_from(n).ok())
        .or_else(|| v.as_str().and_then(|s| s.trim().parse::<u16>().ok()))
        .filter(|c| (100..=599).contains(c))
}

/// Best-effort extract a JSON error body from a free-text stream `cause`: the cause is the whole
/// JSON, or has one embedded (`...Body: {…}`). Returns the parsed value when found.
pub(super) fn parse_embedded_json(cause: &str) -> Option<serde_json::Value> {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(cause.trim()) {
        return Some(v);
    }
    let start = cause.find('{')?;
    let end = cause.rfind('}')?;
    if end <= start {
        return None;
    }
    serde_json::from_str::<serde_json::Value>(&cause[start..=end]).ok()
}

/// Markers of a no-credentials / misconfigured-provider failure (genai's resolver couldn't build
/// the adapter, or the provider rejected an absent key). Treated as [`ProviderError::Auth`] —
/// permanent for the session, so the model is excluded rather than benched-and-retried.
pub(super) fn is_auth_config_failure(text: &str) -> bool {
    let l = text.to_lowercase();
    l.contains("resolver error")
        || l.contains("no auth")
        || l.contains("missing api key")
        || l.contains("no api key")
        || l.contains("api key not")
        || l.contains("requires an api key")
        || l.contains("requires api key")
}

/// A 429 whose quota is per-day or flat-out zero (a free-tier model that's disabled, like
/// Gemini's `limit: 0`). The server still hands back a tiny `retryDelay` (e.g. 7s), but retrying
/// in 7s just fails again and thrashes — so we drop that hint and let the longer default bench
/// apply. Genuine per-minute limits (no such marker) keep their short delay.
pub(super) fn quota_is_exhausted(s: &str) -> bool {
    let l = s.to_lowercase();
    l.contains("limit: 0") || l.contains("perday") || l.contains("per day") || l.contains("per-day")
}

/// Markers of a transient per-minute throughput throttle paraphrased as a quota error. Alibaba
/// Model Studio's OpenAI-compatible endpoint returns HTTP 429 with the error code
/// `Throttling.AllocationQuota` and a message like "Allocated quota exceeded, please increase
/// your quota limit" — the SAME `insufficient_quota` marker OpenAI uses for a genuine, permanent
/// out-of-credits condition. The two must be told apart: the throttle is transient (back-off +
/// retry the same model), while real credit exhaustion is permanent (exclude the model). The
/// discriminator is the `throttling` code and/or the "allocated quota exceeded" phrase — both
/// present in Alibaba's payload and absent from OpenAI's "you exceeded your current quota" body.
/// Checked BEFORE the capability markers so this never falls through to `Capability`.
pub(super) fn is_throttling_failure(text: &str) -> bool {
    let l = text.to_lowercase();
    l.contains("throttling") || l.contains("allocated quota exceeded")
}

/// Markers of a PERMANENT, model-specific incapability — this model can never serve Forge's
/// tool-using turns, or the account can't afford it. These errors recur identically on every
/// call, so the model is *excluded* rather than benched-and-retried (the source of the
/// "every model is failing" churn). Checked against the raw error body, which carries the
/// provider's real message even when the HTTP status is generic (400/404).
pub(super) fn is_capability_failure(text: &str) -> bool {
    let l = text.to_lowercase();
    // A throttling-flavoured quota (Alibaba `Throttling.AllocationQuota` / "allocated quota
    // exceeded") is TRANSIENT even though it carries the `insufficient_quota` marker below.
    // Exclude it here so callers route it to RateLimited (back-off + retry) instead of excluding
    // the model as permanently incapable.
    if is_throttling_failure(text) {
        return false;
    }
    // Standalone markers that unambiguously mean "this model can't serve us".
    const MARKERS: &[&str] = &[
        // OpenRouter: no provider endpoint exposes tool use for this model.
        "no endpoints found that support tool use",
        // OpenRouter / generic: feature explicitly unsupported.
        "does not support feature: function-calling",
        // MiniMax (via opencode_go): rejects our tool payload outright.
        "function name or parameters is empty",
        // Account can't afford the request (OpenRouter 402 free tier).
        "requires more credits",
        "can only afford",
        "insufficient credit",
        "insufficient_quota",
        // A 402 surfaced via the STREAMING path (no typed HTTP status) — e.g. SambaNova:
        // "A payment method is required to use `<model>`". Permanent for this key, so EXCLUDE the
        // model rather than benching + retrying it as a transient outage (the churn dogfooding hit).
        "payment required",
        "payment method is required",
        "payment_required",
        "payment method to continue",
        "add a payment method",
        // Gemini / Antigravity preview models are single-turn only: they reject Forge's multi-turn
        // agentic conversation with "Multiturn chat is not enabled for models/<name>". This is a
        // permanent per-model incapability (never recovers on retry), so EXCLUDE the model + fail
        // over to a capable one instead of failing the whole turn — mesh auto-rotation dogfooding
        // hit this: a session routed to antigravity-preview died with a hard turn failure.
        "multiturn chat is not enabled",
        "multi-turn chat is not enabled",
    ];
    if MARKERS.iter().any(|m| l.contains(m)) {
        return true;
    }
    // Tool/function-calling unsupported, robust to punctuation/wording: a tool-or-function term
    // co-occurring with a "not supported / does not support" phrase. Catches e.g.
    // "`tool calling` is not supported with this model" and "model does not support tool use".
    //
    // PROXIMITY-gated: the two terms must be NEAR each other (same clause), not merely both present
    // somewhere in the body. Anywhere-co-occurrence produced false positives — e.g. "tool use works
    // fine, but JSON/structured-output mode is not supported" would wrongly mark the model as
    // permanently incapable of tool calling and exclude it for a week.
    const TOOL_TERMS: &[&str] = &[
        "tool calling",
        "tool use",
        "tool_use",
        "tool calls",
        "function calling",
        "function-calling",
        "function call",
    ];
    const UNSUPPORTED_TERMS: &[&str] = &[
        "not supported",
        "does not support",
        "isn't supported",
        "unsupported",
    ];
    const PROXIMITY: usize = 60;
    let tool_positions: Vec<usize> = TOOL_TERMS
        .iter()
        .flat_map(|t| l.match_indices(t).map(|(i, _)| i))
        .collect();
    if tool_positions.is_empty() {
        return false;
    }
    UNSUPPORTED_TERMS.iter().any(|u| {
        l.match_indices(u).any(|(up, _)| {
            tool_positions.iter().any(|&tp| {
                let (lo, hi) = if tp <= up { (tp, up) } else { (up, tp) };
                hi - lo <= PROXIMITY
            })
        })
    })
}

/// Classify from an HTTP status code. `body` is the raw provider response (inspected for
/// capability markers that a generic 400/404 status hides); `message` is the shortened display
/// string for the UI.
pub(super) fn classify_status(
    code: u16,
    message: String,
    body: &str,
    retry_after: Option<std::time::Duration>,
) -> ProviderError {
    let exhausted = quota_is_exhausted(&message) || quota_is_exhausted(body);
    // Prefer the provider's JSON error message over genai's generic "HTTP error" wrapper. Besides
    // producing useful diagnostics, this preserves optional-field names such as
    // `prompt_cache_key`, which lets the caller safely retry a strict compatible endpoint without
    // that optimization.
    let provider_message = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|value| {
            let error = value.get("error").unwrap_or(&value);
            error
                .get("message")
                .or_else(|| value.get("message"))
                .or_else(|| error.get("detail"))
                .or_else(|| value.get("detail"))
                .and_then(|message| message.as_str())
                .map(str::to_string)
        });
    let message = short(provider_message.as_deref().unwrap_or(&message));
    // A permanent incapability (no tool support / unaffordable) regardless of status code: 402
    // is always "can't afford", and 400/404 bodies often carry "tool calling not supported".
    // A throttling-flavoured quota (Alibaba `Throttling.AllocationQuota` / "allocated quota
    // exceeded") is NOT that — it's a transient per-minute limit, so it must reach the 429 branch
    // below (RateLimited) rather than being excluded as permanently incapable.
    if code == 402
        || model_endpoint_is_missing(code, body)
        || is_capability_failure(body)
        || is_capability_failure(&message)
    {
        return ProviderError::Capability(message);
    }
    match code {
        429 => ProviderError::RateLimited {
            message,
            retry_after: retry_after.filter(|_| !exhausted),
        },
        401 | 403 => ProviderError::Auth(message),
        500..=599 => ProviderError::Unavailable(message),
        _ => ProviderError::Request(message),
    }
}

/// NVIDIA's hosted NIM catalog maps model ids to account-scoped NVCF function ids. Catalog churn
/// can leave a cached model pointing at a removed function; the endpoint then returns a 404 such
/// as `Function '<uuid>': Not found for account '<id>'`. That is a model-specific incapability,
/// not a malformed turn: bench this stale model and let the mesh try another candidate.
pub(super) fn model_endpoint_is_missing(code: u16, body: &str) -> bool {
    if code != 404 {
        return false;
    }
    let lower = body.to_ascii_lowercase();
    lower.contains("not found for account")
        && (lower.contains("function '")
            || lower.contains("function \"")
            || lower.contains("model"))
}

/// Classify from a free-text error (the streaming case, where genai gives no typed status).
pub(super) fn classify_text(text: &str, message: String) -> ProviderError {
    let lower = text.to_lowercase();
    let has = |needle: &str| lower.contains(needle);
    // retry_after is parsed from the full `text` before the message is shortened; a per-day /
    // zero quota drops the (useless) tiny delay so the longer default bench applies.
    let retry_after = parse_retry_after_body(text).filter(|_| !quota_is_exhausted(text));
    let message = short(&message);
    // Permanent incapability first — a streamed "tool calling is not supported" / "402 requires
    // more credits" must NOT be mistaken for a transient dropped stream (the misclassification
    // bug that benched-and-retried dead models forever).
    if is_capability_failure(text) {
        ProviderError::Capability(message)
    } else if has("429") || has("resource_exhausted") || has("rate limit") || has("quota") {
        ProviderError::RateLimited {
            message,
            retry_after,
        }
    } else if has(" 401") || has(" 403") || has("unauthorized") || has("permission denied") {
        ProviderError::Auth(message)
    } else if is_auth_config_failure(text) {
        // No-credentials / resolver failure surfaced via the streaming path — permanent (excluded),
        // not a transient outage.
        ProviderError::Auth(message)
    } else {
        // A dropped/5xx stream — treat as a transient provider problem worth failing over.
        ProviderError::Unavailable(message)
    }
}

/// Scan an error body for a cooldown: Gemini's `"retryDelay": "37s"` or a `retry in 37.04s`
/// phrase. Returns the first match.
pub(super) fn parse_retry_after_body(body: &str) -> Option<std::time::Duration> {
    let lower = body.to_lowercase();
    for marker in ["retrydelay", "retry in", "retry after", "please retry in"] {
        if let Some(idx) = lower.find(marker) {
            if let Some(d) = parse_secs(&lower[idx + marker.len()..]).and_then(duration_from_secs) {
                return Some(d);
            }
        }
    }
    None
}

/// Build a `Duration` from a parsed seconds value, REJECTING non-finite / negative / absurd values
/// instead of panicking. `Duration::from_secs_f64` panics on NaN, infinity, a negative, or a value
/// too large to represent — an adversarial 429 body (`"retryDelay":"99999999999999999999s"`) would
/// otherwise crash the error-classification / failover path. Caps at a day; no sane cooldown is
/// longer, and clamping keeps a bogusly-huge hint from parking a model out of rotation forever.
pub(super) fn duration_from_secs(secs: f64) -> Option<std::time::Duration> {
    if !secs.is_finite() || secs < 0.0 {
        return None;
    }
    std::time::Duration::try_from_secs_f64(secs.min(86_400.0)).ok()
}

/// Pull the first floating-point number out of `s` (skipping leading quotes/colons/spaces),
/// e.g. `": \"37.04s\""` → `37.04`. Stops at the first non-numeric char after digits.
pub(super) fn parse_secs(s: &str) -> Option<f64> {
    let mut num = String::new();
    let mut started = false;
    for c in s.chars() {
        // Accept a leading decimal point (`.5s`) too, and at most one dot.
        if c.is_ascii_digit() || (c == '.' && !num.contains('.')) {
            num.push(c);
            started = true;
        } else if started {
            break;
        } else if c == '"' || c == ':' || c == '=' || c.is_ascii_whitespace() {
            // Skip whitespace too (incl. `\n`/`\t`) — pretty-printed JSON puts a newline between the
            // key and value (`"retryDelay":\n  "37s"`), which used to abort the parse and drop the hint.
            continue;
        } else {
            // a non-numeric, non-separator char before any digit — give up.
            return None;
        }
    }
    num.parse::<f64>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Alibaba Model Studio's transient per-minute throughput throttle: HTTP 429, error code
    /// `Throttling.AllocationQuota`, and a message that both says "allocated quota exceeded" and
    /// carries the OpenAI-compatible `insufficient_quota` marker. Live-regressed: a pinned
    /// `qwencloud::deepseek-v4-flash-0731` turn died with "model unsupported" (Capability) even
    /// though the model recovered minutes later. It must classify as RATE-LIMITED (back-off +
    /// retry the same model) — never the permanent Capability exclusion.
    #[test]
    fn alibaba_throttling_allocation_quota_is_rate_limited_not_capability() {
        let body = r#"{"error":{"message":"Allocated quota exceeded, please increase your quota limit. For details, see: https://www.alibabacloud.com/help/en/model-studio/error-code#token-limit","type":"insufficient_quota","code":"Throttling.AllocationQuota"}}"#;
        // The typed HTTP path (non-streaming) goes through classify_status.
        match classify_status(429, "HTTP 429".into(), body, None) {
            ProviderError::RateLimited { .. } => {}
            other => panic!("expected RateLimited, got {other:?}"),
        }
        // The OpenAI-compatible structured body (streaming ChatResponse path) uses classify_error_body.
        let parsed = serde_json::from_str::<serde_json::Value>(body).unwrap();
        match classify_error_body(&parsed) {
            Some(ProviderError::RateLimited { .. }) => {}
            other => panic!("expected RateLimited, got {other:?}"),
        }
        // The free-text streaming path (no typed status) must agree too.
        match classify_text(body, "HTTP 429".into()) {
            ProviderError::RateLimited { .. } => {}
            other => panic!("expected RateLimited, got {other:?}"),
        }
    }

    /// OpenAI's genuine out-of-credits condition shares the `insufficient_quota` marker but is
    /// PERMANENT — the account is out of credits and won't recover by retrying. It must stay
    /// Capability (excluded), not be swept into the transient throttle path.
    #[test]
    fn openai_insufficient_quota_stays_capability() {
        let body = r#"{"error":{"message":"You exceeded your current quota, please check your plan and billing details.","type":"insufficient_quota","code":"insufficient_quota"}}"#;
        match classify_status(429, "HTTP 429".into(), body, None) {
            ProviderError::Capability(_) => {}
            other => panic!("expected Capability, got {other:?}"),
        }
        let parsed = serde_json::from_str::<serde_json::Value>(body).unwrap();
        match classify_error_body(&parsed) {
            Some(ProviderError::Capability(_)) => {}
            other => panic!("expected Capability, got {other:?}"),
        }
        match classify_text(body, "HTTP 429".into()) {
            ProviderError::Capability(_) => {}
            other => panic!("expected Capability, got {other:?}"),
        }
    }

    /// SambaNova's "a payment method is required" — a genuinely permanent per-key/account
    /// incapability — must remain Capability.
    #[test]
    fn sambanova_payment_method_required_stays_capability() {
        let body = r#"{"error":{"message":"A payment method is required to use this model.","type":"payment_required","code":"payment_required"}}"#;
        match classify_status(402, "HTTP 402".into(), body, None) {
            ProviderError::Capability(_) => {}
            other => panic!("expected Capability, got {other:?}"),
        }
        let parsed = serde_json::from_str::<serde_json::Value>(body).unwrap();
        match classify_error_body(&parsed) {
            Some(ProviderError::Capability(_)) => {}
            other => panic!("expected Capability, got {other:?}"),
        }
        match classify_text(body, "HTTP 402".into()) {
            ProviderError::Capability(_) => {}
            other => panic!("expected Capability, got {other:?}"),
        }
    }

    /// The throttle discriminator must be narrow: it recognizes the `throttling` code and the
    /// "allocated quota exceeded" phrase, but not OpenAI's ordinary "exceeded your current quota".
    #[test]
    fn throttling_discriminator_is_narrow() {
        assert!(is_throttling_failure("Throttling.AllocationQuota"));
        assert!(is_throttling_failure(
            "allocated quota exceeded, please increase your quota limit"
        ));
        assert!(!is_throttling_failure(
            "you exceeded your current quota, please check your plan and billing details"
        ));
        assert!(!is_throttling_failure(
            "A payment method is required to use this model."
        ));
    }
}
