//! See a phone's traffic, change it, and replay it.
//!
//! `forge-browser` covers what a *browser* fetches. This covers everything else on the network —
//! a mobile app, a CLI, an embedded device — by running [mitmproxy] as a system proxy the device
//! points at. That is the only vantage point from which a native app's HTTPS is legible at all:
//! there is no DevTools to open, and the app will not tell you what it sends.
//!
//! The shape deliberately mirrors `browser` / `browser_network`, so there is one mental model for
//! "watch traffic, then act on it" rather than two:
//!
//! | browser                  | here                    |
//! |--------------------------|-------------------------|
//! | `browser open`           | [`Proxy::start`]        |
//! | `browser_network list`   | [`Proxy::flows`]        |
//! | `browser_network body`   | [`Proxy::flow`]         |
//! | `browser_network har`    | [`Proxy::har`]          |
//! | `browser intercept`      | [`Proxy::set_rules`]    |
//! | `browser replay`         | [`Proxy::replay`]       |
//!
//! [mitmproxy]: https://mitmproxy.org

use std::path::{Path, PathBuf};

pub use anyhow::Error;
use anyhow::{Context, Result};

mod flows;
mod rules;
mod session;

pub use flows::{Filter, Flow};
pub use rules::{BodyRule, HeaderRule, InterceptRules, StubRule};
pub use session::{Proxy, ProxyStatus};

/// The addon mitmdump loads. Compiled in rather than shipped alongside the binary: a Forge that
/// found no addon file would start a proxy that captured nothing, and the failure would look like
/// "the phone isn't sending traffic" rather than "the install is incomplete".
pub const ADDON: &str = include_str!("addon.py");

/// Default listen port. 8080 is mitmproxy's own default and what every phone-proxy walkthrough
/// tells people to type, so it is the port a user is least surprised by.
pub const DEFAULT_PORT: u16 = 8080;

/// Cap on a single body handed back to a model, independent of what the addon stored. The addon's
/// cap keeps the capture file sane; this one keeps one `flow` call from eating a context window.
pub const MAX_BODY_CHARS: usize = 100_000;

/// Where mitmproxy writes the CA certificate a device must trust before HTTPS is readable.
///
/// Without installing this, an intercepted HTTPS request fails at the TLS handshake — which the
/// app reports as "no internet", and which is the single most common reason a first attempt at
/// phone interception shows an empty capture.
pub fn ca_cert_path() -> PathBuf {
    dirs_home().join(".mitmproxy").join("mitmproxy-ca-cert.pem")
}

fn dirs_home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Truncate on a char boundary, saying so.
pub fn truncate(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let kept: String = text.chars().take(max_chars).collect();
    format!("{kept}\n… truncated at {max_chars} characters")
}

/// The LAN address a device should point at, and the steps to get there.
///
/// Written out in full because every one of these is a step people miss, and each failure looks
/// like a different problem: no CA → "no internet"; wrong IP → nothing arrives; certificate
/// pinning → only *some* apps break, which reads as the proxy being flaky.
pub fn setup_instructions(host: &str, port: u16) -> String {
    let cert = ca_cert_path();
    format!(
        "Point the device at this proxy:\n\
         1. Wi-Fi settings → the network → Proxy: Manual → host {host}, port {port}.\n\
         2. Install the CA so HTTPS is readable: browse to http://mitm.it on the device and \
            follow its instructions (the cert is also at {}).\n\
         3. Android 7+: a user-installed CA is NOT trusted by apps unless the app opts in. \
            Use an emulator with a system CA, or a debuggable build with a networkSecurityConfig \
            that trusts user certs.\n\
         4. Certificate pinning defeats interception entirely for apps that use it — the app will \
            refuse to connect and the capture stays empty. That is the app working as designed, \
            not a broken proxy.",
        cert.display()
    )
}

/// Resolve this machine's LAN IP — the address the phone has to reach, which is never the
/// loopback the proxy also binds.
pub fn lan_ip() -> Option<String> {
    let sock = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    // No packet is sent; connect() on UDP only selects a route, which is what reveals the
    // outbound interface's address on a multi-homed machine.
    sock.connect(("8.8.8.8", 80)).ok()?;
    let ip = sock.local_addr().ok()?.ip();
    (!ip.is_unspecified() && !ip.is_loopback()).then(|| ip.to_string())
}

/// Locate `mitmdump`, with an error that says how to get it rather than just that it is absent.
pub fn find_mitmdump() -> Result<PathBuf> {
    which("mitmdump").context(
        "mitmdump not found on PATH. Install mitmproxy (`pipx install mitmproxy`, \
         `brew install mitmproxy`, or your distro's package) — the proxy tools need it to run.",
    )
}

fn which(binary: &str) -> Result<PathBuf> {
    let path = std::env::var_os("PATH").context("PATH is unset")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(binary);
        if is_executable(&candidate) {
            return Ok(candidate);
        }
    }
    anyhow::bail!("{binary} is not on PATH")
}

fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path)
            .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncation_reports_itself_and_respects_char_boundaries() {
        assert_eq!(truncate("short", 100), "short");
        let cut = truncate(&"é".repeat(10), 4);
        assert!(cut.starts_with("éééé"), "{cut}");
        assert!(cut.contains("truncated at 4 characters"));
    }

    /// Every line here is a step whose omission produces a DIFFERENT confusing symptom, so the
    /// instructions have to name all of them — especially the two that make a correctly running
    /// proxy look broken.
    #[test]
    fn the_setup_steps_cover_what_actually_goes_wrong() {
        let text = setup_instructions("192.168.1.20", 8080);
        assert!(text.contains("192.168.1.20") && text.contains("8080"));
        assert!(
            text.contains("mitm.it"),
            "the CA install is step one of failure"
        );
        assert!(
            text.contains("Android 7+"),
            "a user CA silently not being trusted by apps is the classic dead end"
        );
        assert!(
            text.contains("pinning"),
            "pinning must be named, or a working proxy reads as a broken one"
        );
    }

    #[test]
    fn a_missing_mitmdump_says_how_to_install_it() {
        let error = format!("{:#}", which("definitely-not-a-real-binary").unwrap_err());
        assert!(error.contains("not on PATH"), "{error}");
    }
}
