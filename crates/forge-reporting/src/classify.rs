//! What counts as a bug worth reporting, and what does not.
//!
//! A report is a claim that Forge itself misbehaved. Rate limits, bad keys, a dead network, a
//! Ctrl-C and a broken config are the user's world, not Forge's bugs, and reporting them would
//! bury the maintainers in noise and tell them nothing. The rules are explicit and tested:
//!
//! * panics, render panics, task deaths and every JS/Tauri error kind are bugs, except a closed
//!   stdout/stderr (`forge run | head`);
//! * everything else (`Internal`, `ProviderProtocol`, `Store`, `ToolDispatch`, `Compaction`) is a
//!   bug only when its text does not name an expected condition.

use regex::Regex;
use std::sync::OnceLock;

use crate::report::ErrorKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Expected {
    RateLimit,
    Quota,
    Auth,
    Network,
    ProviderOutage,
    UserInterrupt,
    UserConfig,
    Environment,
    BrokenPipe,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    Bug,
    Expected(Expected),
}

impl Class {
    pub fn is_bug(self) -> bool {
        self == Class::Bug
    }
}

struct Rules {
    status: Regex,
    list: Vec<(Expected, Vec<&'static str>)>,
    protocol: Vec<&'static str>,
}

fn rules() -> &'static Rules {
    static RULES: OnceLock<Rules> = OnceLock::new();
    RULES.get_or_init(|| Rules {
        status: Regex::new(
            r"(?i)\b(?:http|status|code)\D{0,3}(401|402|403|429|500|502|503|504|529)\b",
        )
        .expect("static regex"),
        list: vec![
            (
                Expected::BrokenPipe,
                vec![
                    "broken pipe",
                    "failed printing to std",
                    "failed writing to std",
                ],
            ),
            (
                Expected::UserInterrupt,
                vec![
                    "interrupted",
                    "cancelled",
                    "canceled",
                    "aborted by user",
                    "ctrl-c",
                    "sigint",
                    "user declined",
                ],
            ),
            (
                Expected::RateLimit,
                vec![
                    "rate limit",
                    "rate-limit",
                    "rate limited",
                    "too many requests",
                    "resource_exhausted",
                ],
            ),
            (
                Expected::Quota,
                vec![
                    "quota",
                    "insufficient credit",
                    "out of credits",
                    "credit balance",
                    "billing",
                    "payment required",
                    "requires more credits",
                ],
            ),
            (
                Expected::Auth,
                vec![
                    "unauthorized",
                    "forbidden",
                    "authentication",
                    "invalid api key",
                    "incorrect api key",
                    "api key",
                    "not logged in",
                    "login required",
                    "token expired",
                    "invalid_grant",
                ],
            ),
            (
                Expected::Network,
                vec![
                    "connection refused",
                    "connection reset",
                    "connection closed",
                    "timed out",
                    "timeout",
                    "dns error",
                    "name resolution",
                    "failed to lookup",
                    "network is unreachable",
                    "no route to host",
                    "offline",
                    "error sending request",
                    "tls handshake",
                    "certificate verify",
                ],
            ),
            (
                Expected::ProviderOutage,
                vec![
                    "overloaded",
                    "service unavailable",
                    "bad gateway",
                    "gateway timeout",
                    "provider unavailable",
                    "provider is down",
                ],
            ),
            (
                Expected::UserConfig,
                vec![
                    "invalid config",
                    "config error",
                    "cannot parse config",
                    "unknown model",
                    "model not found",
                    "model_not_found",
                    "not a git repository",
                    "no such file or directory",
                    "is not trusted",
                ],
            ),
            (
                Expected::Environment,
                vec![
                    "no space left",
                    "disk is full",
                    "disk or disk is full",
                    "database or disk is full",
                    "read-only file system",
                    "readonly database",
                    "permission denied",
                    "too many open files",
                    "out of memory",
                    "unable to open database",
                    "database is locked",
                ],
            ),
        ],
        protocol: vec![
            "malformed",
            "unexpected",
            "failed to parse",
            "failed to deserialize",
            "failed to decode",
            "invalid json",
            "invalid response",
            "invalid utf",
            "missing field",
            "unknown variant",
            "eof while parsing",
            "stream ended",
            "protocol",
            "invalid type",
            "expected value",
        ],
    })
}

/// Classify by kind and (scrubbed or raw) message text.
pub fn classify(kind: ErrorKind, message: &str) -> Class {
    let lower = message.to_ascii_lowercase();
    let r = rules();
    let always_bug = matches!(
        kind,
        ErrorKind::Panic
            | ErrorKind::RenderPanic
            | ErrorKind::TaskDeath
            | ErrorKind::JsError
            | ErrorKind::JsUnhandledRejection
            | ErrorKind::ReactBoundary
            | ErrorKind::TauriCommand
            | ErrorKind::TauriPanic
            | ErrorKind::Manual
    );
    if always_bug {
        // Only a closed pipe is ever expected from a panic.
        if matches!(kind, ErrorKind::Panic | ErrorKind::TauriPanic) {
            if let Some((expected, _)) = r.list.iter().find(|(e, needles)| {
                *e == Expected::BrokenPipe && needles.iter().any(|n| lower.contains(n))
            }) {
                return Class::Expected(*expected);
            }
        }
        return Class::Bug;
    }
    for (expected, needles) in &r.list {
        if needles.iter().any(|n| lower.contains(n)) {
            return Class::Expected(*expected);
        }
    }
    if let Some(caps) = r.status.captures(&lower) {
        return Class::Expected(match &caps[1] {
            "401" | "403" => Expected::Auth,
            "402" => Expected::Quota,
            "429" => Expected::RateLimit,
            _ => Expected::ProviderOutage,
        });
    }
    Class::Bug
}

/// A non-retryable provider "request" error is the user's (context too long, content policy, a bad
/// parameter) unless its text shows Forge could not read what the provider sent back.
pub fn provider_request_is_bug(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    rules().protocol.iter().any(|n| lower.contains(n))
        && !classify(ErrorKind::ProviderProtocol, message).is_bug_expected()
}

impl Class {
    fn is_bug_expected(self) -> bool {
        matches!(self, Class::Expected(_))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expected_conditions_are_not_bugs() {
        for msg in [
            "rate limited: 429 Too Many Requests",
            "provider auth failed: invalid API key",
            "HTTP 401 from upstream",
            "error sending request for url: connection refused",
            "request timed out after 30s",
            "turn interrupted by user",
            "no such file or directory: ~/missing.toml",
            "provider unavailable: 503 service unavailable",
            "database or disk is full",
            "insufficient credits: payment required",
        ] {
            assert!(
                !classify(ErrorKind::Internal, msg).is_bug(),
                "should be expected: {msg}"
            );
        }
    }

    #[test]
    fn real_defects_are_bugs() {
        for msg in [
            "internal invariant violated: tool call without id",
            "stream produced an unexpected event ordering",
            "index out of bounds in transcript rewrite",
            "no such variant while rebuilding plan",
        ] {
            assert!(
                classify(ErrorKind::Internal, msg).is_bug(),
                "should be a bug: {msg}"
            );
        }
    }

    #[test]
    fn panics_are_bugs_except_a_closed_pipe() {
        assert!(classify(
            ErrorKind::Panic,
            "called `Option::unwrap()` on a `None` value"
        )
        .is_bug());
        assert!(classify(ErrorKind::Panic, "index out of bounds: the len is 0").is_bug());
        // A panic that merely mentions a rate limit is still a panic.
        assert!(classify(ErrorKind::Panic, "rate limit table was empty").is_bug());
        assert_eq!(
            classify(
                ErrorKind::Panic,
                "failed printing to stdout: Broken pipe (os error 32)"
            ),
            Class::Expected(Expected::BrokenPipe)
        );
    }

    #[test]
    fn js_and_tauri_kinds_are_always_bugs() {
        for kind in [
            ErrorKind::JsError,
            ErrorKind::ReactBoundary,
            ErrorKind::JsUnhandledRejection,
            ErrorKind::TauriCommand,
        ] {
            assert!(classify(kind, "timeout while rendering").is_bug());
        }
    }

    #[test]
    fn status_numbers_need_http_context() {
        assert!(classify(ErrorKind::Internal, "line 403 of the plan file is wrong").is_bug());
        assert!(!classify(ErrorKind::Internal, "upstream replied status 429").is_bug());
    }

    #[test]
    fn provider_request_errors_are_bugs_only_when_the_reply_was_unreadable() {
        assert!(provider_request_is_bug(
            "malformed tool call arguments in response"
        ));
        assert!(provider_request_is_bug(
            "failed to deserialize stream chunk: missing field `id`"
        ));
        assert!(!provider_request_is_bug(
            "context length exceeded: 210000 tokens"
        ));
        assert!(!provider_request_is_bug(
            "invalid request: temperature out of range"
        ));
    }
}
