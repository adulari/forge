//! The two honesty markers a snapshot can carry: why the last turn failed, in a shape a client
//! can title and fold, and how much of the session's record a client is known to be missing.

/// Longest first-line gist kept as the headline; the full text rides in `details`.
const SUMMARY_CHARS: usize = 140;
/// Longest `details` a snapshot carries. Provider errors can echo whole request bodies.
const DETAILS_CHARS: usize = 4_000;

/// A failed turn: a coarse `category` to title it by, a one-line `summary`, and the host's full
/// message as `details`.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ThreadError {
    /// `rate_limit`, `auth`, `context`, `network`, `provider` or `other`.
    pub category: &'static str,
    pub summary: String,
    pub details: String,
}

impl ThreadError {
    pub fn from_message(message: &str) -> Self {
        let text = message.trim().trim_start_matches('⚠').trim();
        let lower = text.to_lowercase();
        let has = |needles: &[&str]| needles.iter().any(|n| lower.contains(n));
        let category = if has(&[
            "rate limit",
            "rate-limit",
            "429",
            "too many requests",
            "quota",
        ]) {
            "rate_limit"
        } else if has(&[
            "401",
            "403",
            "unauthorized",
            "unauthorised",
            "api key",
            "authentication",
            "not logged in",
            "forbidden",
        ]) {
            "auth"
        } else if has(&[
            "context length",
            "context window",
            "maximum context",
            "too many tokens",
            "prompt is too long",
        ]) {
            "context"
        } else if has(&[
            "timed out",
            "timeout",
            "connection",
            "dns",
            "network",
            "unreachable",
            "error sending request",
        ]) {
            "network"
        } else if has(&[
            "overloaded",
            "500",
            "502",
            "503",
            "529",
            "service unavailable",
            "internal server error",
        ]) {
            "provider"
        } else {
            "other"
        };
        let first_line = text.lines().next().unwrap_or_default().trim();
        Self {
            category,
            summary: clip(first_line, SUMMARY_CHARS),
            details: clip(text, DETAILS_CHARS),
        }
    }
}

fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// Updates a client is known to have missed. `count` is how many; `reason` says why:
/// `replay_window_exceeded` (it was away longer than the host keeps frames for, so it was sent
/// the current state instead of the frames in between).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct HistoryGap {
    pub count: u64,
    pub reason: &'static str,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failure_is_titled_by_what_the_host_said() {
        let cases = [
            ("⚠ provider returned 429 Too Many Requests", "rate_limit"),
            ("⚠ 401 Unauthorized: invalid x-api-key", "auth"),
            ("prompt is too long: 210000 tokens", "context"),
            ("error sending request: connection reset", "network"),
            ("upstream 529 overloaded", "provider"),
            ("the model returned nothing", "other"),
        ];
        for (message, category) in cases {
            assert_eq!(
                ThreadError::from_message(message).category,
                category,
                "{message}"
            );
        }
    }

    #[test]
    fn the_headline_is_one_clipped_line_and_the_details_keep_the_rest() {
        let message = format!("⚠ boom\nstack line 1\n{}", "x".repeat(9_000));
        let error = ThreadError::from_message(&message);
        assert_eq!(error.summary, "boom");
        assert!(error.details.starts_with("boom\nstack line 1"));
        assert_eq!(error.details.chars().count(), 4_000);
        assert!(error.details.ends_with('…'));
        let long = ThreadError::from_message(&"y".repeat(500));
        assert_eq!(long.summary.chars().count(), 140);
    }
}
