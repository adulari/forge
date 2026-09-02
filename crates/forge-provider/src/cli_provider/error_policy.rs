//! Error-classification policy for CLI bridge output.

/// Whether normalized CLI output reports a credential problem.
///
/// The classifier used to match the bare substring `"auth"`, which an auth verdict is far too
/// expensive to rest on: it fires on `oauth`, `author`, `authority`, and on any of those appearing
/// anywhere in the stderr tail appended as evidence. These phrases only mean that the credential
/// did not work.
///
/// Two phrases were removed on the same grounds (2026-09-02). `permission denied` is what Claude
/// Code's own permission system prints when a TOOL call is refused, and what the OS prints for a
/// file — neither says anything about the login. `credentials` appears in keychain / credential-
/// file notices on perfectly healthy installs. Either one benched `claude-cli::opus[1m]` as
/// "auth failed" for 30 minutes while `claude --model opus -p` answered on the same login. An auth
/// verdict is permanent and account-wide in effect, so only text that names the LOGIN may earn it.
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
    ];

    PHRASES.iter().any(|phrase| normalized.contains(phrase))
}
