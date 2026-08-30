//! Error-classification policy for CLI bridge output.

/// Whether normalized CLI output reports a credential problem.
///
/// The classifier used to match the bare substring `"auth"`, which an auth verdict is far too
/// expensive to rest on: it fires on `oauth`, `author`, `authority`, and on any of those appearing
/// anywhere in the stderr tail appended as evidence. These phrases only mean that the credential
/// did not work.
pub(super) fn is_auth_failure(normalized: &str) -> bool {
    const PHRASES: &[&str] = &[
        "authentication",
        "authorization",
        "unauthenticated",
        "unauthorized",
        "auth failed",
        "auth error",
        "auth required",
        "invalid api key",
        "invalid_api_key",
        "api key not valid",
        "not logged in",
        "login required",
        "please log in",
        "please run /login",
        "permission denied",
        "credentials",
    ];

    PHRASES.iter().any(|phrase| normalized.contains(phrase))
}
