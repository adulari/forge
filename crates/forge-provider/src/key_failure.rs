//! Telling "this key is dead" apart from "the network blinked", so a screen can say
//! "Key expired — renew" instead of the owner finding models quietly missing.

use crate::ProviderError;

/// Why a provider's credential cannot be used. Persisted as its [`as_str`](Self::as_str) word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyFailure {
    Expired,
    Rejected,
    OutOfQuota,
}

impl KeyFailure {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Expired => "expired",
            Self::Rejected => "rejected",
            Self::OutOfQuota => "quota",
        }
    }

    pub fn from_word(word: &str) -> Option<Self> {
        match word {
            "expired" => Some(Self::Expired),
            "rejected" => Some(Self::Rejected),
            "quota" => Some(Self::OutOfQuota),
            _ => None,
        }
    }

    /// The red row text on every surface.
    pub fn headline(self) -> &'static str {
        match self {
            Self::Expired => "Key expired — renew",
            Self::Rejected => "Key rejected",
            Self::OutOfQuota => "Out of quota — top up",
        }
    }
}

const QUOTA_MARKERS: [&str; 8] = [
    "payment required",
    "insufficient_quota",
    "insufficient quota",
    "exceeded your current quota",
    "out of credits",
    "requires more credits",
    "billing",
    "quota exceeded",
];

const REJECTION_MARKERS: [&str; 7] = [
    "provider auth failed",
    "401",
    "unauthorized",
    "invalid api key",
    "invalid_api_key",
    "incorrect api key",
    "authentication_error",
];

/// Classify a failure message (a discovery detail, a stringified error). `None` for anything that
/// is not the provider refusing the account: timeouts, 5xx, a plain 429 and a missing listing
/// endpoint all stay out, because a verdict is only cleared by a success and must not outlive a
/// blip.
pub fn classify_key_failure(text: &str) -> Option<(KeyFailure, String)> {
    let lower = text.to_ascii_lowercase();
    let kind = if QUOTA_MARKERS.iter().any(|m| lower.contains(m)) {
        KeyFailure::OutOfQuota
    } else if REJECTION_MARKERS.iter().any(|m| lower.contains(m)) {
        if lower.contains("expired") {
            KeyFailure::Expired
        } else {
            KeyFailure::Rejected
        }
    } else {
        return None;
    };
    Some((kind, short_reason(text)))
}

/// The provider's own sentence, not Forge's wrapping of it: `{"error":"This API key has expired"}`
/// inside `provider auth failed: HttpError status: 401, body: {...}` reads as the inner message.
fn short_reason(text: &str) -> String {
    let inner = ["\"message\"", "\"error\"", "\"detail\""]
        .iter()
        .find_map(|key| {
            let after = &text[text.find(key)? + key.len()..];
            let value = after.trim_start().strip_prefix(':')?.trim_start();
            let value = value.strip_prefix('"')?;
            let end = value.find('"').unwrap_or(value.len());
            Some(value[..end].trim().to_string())
        })
        .filter(|s| !s.is_empty());
    let base = inner.unwrap_or_else(|| {
        text.trim()
            .trim_start_matches("using configured seed models: ")
            .trim_start_matches("provider auth failed: ")
            .to_string()
    });
    let mut out: String = base.chars().take(140).collect();
    if base.chars().count() > 140 {
        out.push('…');
    }
    out
}

/// A 429 is a routine per-minute/per-day limit unless it names a wall no retry can clear. Gemini
/// and Groq word their ordinary limits with "quota" and a billing link, so those words alone prove
/// nothing here.
fn is_hard_quota_wall(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    let zero_limit = lower
        .match_indices("limit: 0")
        .any(|(i, m)| !lower[i + m.len()..].starts_with(|c: char| c.is_ascii_digit() || c == '.'));
    zero_limit
        || [
            "insufficient_quota",
            "insufficient quota",
            "payment required",
            "out of credits",
            "requires more credits",
        ]
        .iter()
        .any(|m| lower.contains(m))
}

impl ProviderError {
    /// The key verdict this error implies, if any. Typed [`Auth`](Self::Auth) is always a bad
    /// credential; quota and billing walls arrive as `Capability`/`RateLimited` and are read from
    /// their text. A plain rate limit is not a verdict.
    pub fn key_failure(&self) -> Option<(KeyFailure, String)> {
        match self {
            Self::Auth(msg) => Some(
                classify_key_failure(&self.to_string())
                    .unwrap_or_else(|| (KeyFailure::Rejected, short_reason(msg))),
            ),
            Self::Capability(msg) => {
                let (kind, reason) = classify_key_failure(msg)?;
                (kind == KeyFailure::OutOfQuota).then_some((kind, reason))
            }
            Self::RateLimited { message: msg, .. } => {
                if !is_hard_quota_wall(msg) {
                    return None;
                }
                let (kind, reason) = classify_key_failure(msg)?;
                (kind == KeyFailure::OutOfQuota).then_some((kind, reason))
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_expired_key_message_is_an_expired_verdict_with_the_providers_sentence() {
        let text = r#"provider auth failed: HttpError status: 401 Unauthorized, body: {"error":"This API key has expired"}"#;
        let (kind, reason) = classify_key_failure(text).unwrap();
        assert_eq!(kind, KeyFailure::Expired);
        assert_eq!(reason, "This API key has expired");
    }

    #[test]
    fn pretty_printed_json_bodies_read_the_same() {
        let text = r#"demoexp `/models` returned HTTP 401: {"error": "This API key has expired"}"#;
        assert_eq!(
            classify_key_failure(text).unwrap().1,
            "This API key has expired"
        );
    }

    #[test]
    fn revoked_and_invalid_keys_are_rejected_not_expired() {
        let (kind, _) = classify_key_failure("provider auth failed: invalid api key").unwrap();
        assert_eq!(kind, KeyFailure::Rejected);
        assert_eq!(KeyFailure::Rejected.headline(), "Key rejected");
    }

    #[test]
    fn billing_walls_are_out_of_quota_and_blips_are_nothing() {
        let (kind, _) = classify_key_failure("model unsupported: Payment required").unwrap();
        assert_eq!(kind, KeyFailure::OutOfQuota);
        for blip in [
            "provider unavailable: 503",
            "after 8s",
            "rate limited: slow down",
            "error sending request: connection refused",
        ] {
            assert!(classify_key_failure(blip).is_none(), "{blip}");
        }
    }

    #[test]
    fn typed_errors_map_without_reading_a_status_code() {
        assert_eq!(
            ProviderError::Auth("nope".into()).key_failure().unwrap().0,
            KeyFailure::Rejected
        );
        assert!(ProviderError::Unavailable("401 in a path".into())
            .key_failure()
            .is_none());
        assert!(ProviderError::RateLimited {
            message: "too fast".into(),
            retry_after: None
        }
        .key_failure()
        .is_none());
        assert_eq!(
            ProviderError::Capability("insufficient_quota".into())
                .key_failure()
                .unwrap()
                .0,
            KeyFailure::OutOfQuota
        );
    }

    #[test]
    fn routine_429s_are_never_a_key_verdict() {
        let gemini = r#"{"error":{"code":429,"message":"You exceeded your current quota, please check your plan and billing details. Quota exceeded for metric: generativelanguage.googleapis.com/generate_content_free_tier_requests, limit: 15, model: gemini-2.0-flash. Please retry in 37.047405996s.","status":"RESOURCE_EXHAUSTED"}}"#;
        let groq = r#"{"error":{"message":"Rate limit reached for model `llama-3.3-70b-versatile` in organization `org_x` service tier `on_demand` on requests per day (RPD): Limit 1000, Used 1000, Requested 1. Please try again in 8m24s. Need more tokens? Upgrade to Dev Tier today at https://console.groq.com/settings/billing","type":"requests","code":"rate_limit_exceeded"}}"#;
        for (body, retry_after) in [(gemini, Some(37)), (groq, None)] {
            let e = ProviderError::RateLimited {
                message: body.into(),
                retry_after: retry_after.map(std::time::Duration::from_secs),
            };
            assert!(e.key_failure().is_none(), "{body}");
        }
        let hard = ProviderError::RateLimited {
            message: "You exceeded your current quota. limit: 0, model: x".into(),
            retry_after: None,
        };
        assert_eq!(hard.key_failure().unwrap().0, KeyFailure::OutOfQuota);
        let hard = ProviderError::RateLimited {
            message: r#"{"error":{"code":"insufficient_quota"}}"#.into(),
            retry_after: None,
        };
        assert_eq!(hard.key_failure().unwrap().0, KeyFailure::OutOfQuota);
    }

    #[test]
    fn words_round_trip() {
        for k in [
            KeyFailure::Expired,
            KeyFailure::Rejected,
            KeyFailure::OutOfQuota,
        ] {
            assert_eq!(KeyFailure::from_word(k.as_str()), Some(k));
        }
    }
}
