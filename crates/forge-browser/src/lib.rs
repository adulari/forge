//! Drive a real Chrome and read everything it fetches.
//!
//! Forge already has `web_fetch`, which retrieves a URL with an HTTP client. That is the wrong
//! tool for anything behind a login, anything rendered by JavaScript, and anything whose *traffic*
//! is the thing under investigation. This crate covers that gap: a genuine browser the user can
//! watch and log into, plus the DevTools Network tab as queryable data.
//!
//! - [`launch`] — finding, starting, and re-attaching to Chrome, and why the profile and port are
//!   handled the way they are.
//! - [`cdp`] — the DevTools Protocol client.
//! - [`network`] — the capture that makes this worth more than page automation.
//! - [`intercept`] — request blocking and header rewriting.
//! - [`har`] — export the capture as a HAR file.
//! - [`session`] — the live browser: control, capture, interception, and replay.

use std::time::Duration;

/// Re-exported so dependents can name the error type without taking an `anyhow` dependency
/// of their own.
pub use anyhow::Error;

pub mod cdp;
pub mod har;
pub mod intercept;
pub mod launch;
pub mod network;
mod session;

pub use intercept::InterceptionRules;
pub use launch::{BrowserProcess, Display, Fingerprint, LaunchConfig};
pub use network::{Exchange, Filter, NetworkLog};
pub use session::{BrowserSession, ReplayRequest};

/// Cap on a single response body handed back to the model. A 40 MB bundle would blow the context
/// window; the caller can narrow with a filter and re-ask.
pub const MAX_BODY_CHARS: usize = 200_000;

/// How long `navigate` waits for the load event before returning what it has. A page that never
/// finishes loading (long-polling, a hung third-party script) is still usable, so this is a
/// deadline rather than a failure.
pub(crate) const LOAD_TIMEOUT: Duration = Duration::from_secs(30);

/// Truncate on a char boundary, saying so.
pub fn truncate(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let kept: String = text.chars().take(max_chars).collect();
    format!("{kept}\n… truncated at {max_chars} characters")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncation_reports_itself_and_respects_char_boundaries() {
        assert_eq!(truncate("short", 100), "short");
        let text = "é".repeat(10);
        let cut = truncate(&text, 4);
        assert!(cut.starts_with("éééé"), "{cut}");
        assert!(cut.contains("truncated at 4 characters"), "{cut}");
    }
}
