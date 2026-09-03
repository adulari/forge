//! Turning a CLI's sign-in output into something a user can act on.
//!
//! A logged-out bridge does not report "not logged in": it prints a ~500-character OAuth consent
//! URL, a wait notice, and a prompt for a pasted code — and Forge then echoed all of it twice, as
//! a warning and again inside the final error, burying the one line that mattered.

/// Whether CLI output is an interactive sign-in prompt rather than a diagnosis: a consent URL to
/// open, or a request to paste a code back. This is a CLI asking a human to log in, which cannot
/// be answered inside a non-interactive turn.
pub(super) fn is_interactive_auth_prompt(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    // Deliberately narrow. A false positive kills a running turn, so "the output mentions oauth"
    // is not enough: it must carry an authorization-code REQUEST — the query parameters of a
    // consent URL, or the CLI's own prompt for a pasted code.
    let consent_url = lower.contains("http")
        && (lower.contains("response_type=code")
            || lower.contains("/o/oauth2/")
            || (lower.contains("oauth") && lower.contains("redirect_uri=")));
    consent_url
        || lower.contains("paste the authorization code")
        || lower.contains("waiting for authentication")
}

/// One actionable line for a bridge that is asking a human to sign in, in place of the CLI's own
/// output. The full text stays available at debug level.
pub(super) fn not_logged_in_message(binary: &str) -> String {
    format!(
        "{binary}: not logged in — run `{binary} login` (run with -v for the CLI's full output)"
    )
}

/// Replace an interactive sign-in prompt with [`not_logged_in_message`]; pass anything else
/// through unchanged. Applied wherever a bridge's own output reaches a warning or an error, so a
/// consent URL never becomes the user-facing message.
pub(super) fn collapse_oauth_urls(binary: &str, text: &str) -> String {
    if is_interactive_auth_prompt(text) {
        tracing::debug!("{binary} sign-in output: {text}");
        not_logged_in_message(binary)
    } else {
        text.to_string()
    }
}

/// Read a child's stderr to the shared cap, notifying the moment it turns into an interactive
/// sign-in prompt. The prompt is unanswerable here — the child's stdin is a pipe or `/dev/null`,
/// never a terminal — so the turn's only options are to notice now or to wait out the CLI's own
/// login timeout (60s for `agy`) for an answer that can never arrive.
pub(super) async fn read_to_cap_watching<R: tokio::io::AsyncRead + Unpin>(
    mut r: R,
    interactive_auth: Option<std::sync::Arc<tokio::sync::Notify>>,
) -> String {
    use tokio::io::AsyncReadExt as _;
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    let mut notified = false;
    while let Ok(n) = r.read(&mut chunk).await {
        if n == 0 || buf.len() >= super::STDERR_CAP {
            break;
        }
        let take = n.min(super::STDERR_CAP - buf.len());
        buf.extend_from_slice(&chunk[..take]);
        if !notified {
            if let Some(notify) = &interactive_auth {
                if is_interactive_auth_prompt(&String::from_utf8_lossy(&buf)) {
                    notified = true;
                    notify.notify_one();
                }
            }
        }
    }
    String::from_utf8_lossy(&buf).into_owned()
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    /// The literal `agy` output from a keyless run: a Google consent URL, a wait notice, and a
    /// prompt for a pasted code. None of it tells the user what to do.
    pub(crate) const AGY_SIGN_IN_STDERR: &str = "Please open this URL to sign in: \
https://accounts.google.com/o/oauth2/v2/auth?client_id=1234-abc.apps.googleusercontent.com&\
prompt=consent&redirect_uri=https%3A%2F%2Fantigravity.google%2Foauth-callback&\
response_type=code&scope=https%3A%2F%2Fwww.googleapis.com%2Fauth%2Fcloud-platform+openid&\
state=_rskYCYnKvMgfE6vxMfy5w\n\nWaiting for authentication (timeout 60s)...\n\
Or, paste the authorization code here and press Enter:\nError: authentication timed out.";

    #[test]
    fn an_oauth_consent_url_collapses_to_one_actionable_line() {
        let collapsed = collapse_oauth_urls("agy", AGY_SIGN_IN_STDERR);
        assert_eq!(
            collapsed,
            "agy: not logged in — run `agy login` (run with -v for the CLI's full output)"
        );
        assert!(!collapsed.contains("http"), "no URL survives: {collapsed}");
        assert_eq!(collapsed.lines().count(), 1, "one line: {collapsed}");
    }

    /// Collapsing must not eat a real diagnosis — those are the messages that explain a failure.
    #[test]
    fn ordinary_bridge_stderr_passes_through_uncollapsed() {
        for ordinary in [
            "error: model `gpt-9` is not available on your plan",
            "thread 'main' panicked at src/main.rs:1:1",
            "warning: fetched https://example.com/docs during the turn",
            "429 Too Many Requests",
        ] {
            assert_eq!(
                collapse_oauth_urls("codex", ordinary),
                ordinary,
                "must not be mistaken for a sign-in prompt"
            );
            assert!(!is_interactive_auth_prompt(ordinary));
        }
        assert!(is_interactive_auth_prompt(AGY_SIGN_IN_STDERR));
    }
}
