//! Attaching to a browser that is already running — the one the user is logged into.
//!
//! Forge does not launch or own this browser, so the rules are the reverse of [`crate::launch`]:
//! find it from a URL the user supplied, open Forge's OWN tab in it, and never touch anything
//! else. The only thing ever closed is the tab Forge created, over `/json/close/<id>`. There is
//! deliberately no `Browser.close` and no `Target.closeTarget` anywhere in this crate's attach
//! path: either would take the user's windows with it.
//!
//! Chrome 136+ refuses `--remote-debugging-port` on its default profile, so the browser has to be
//! started on a dedicated `--user-data-dir` (see [`attach_launch_args`] / `forge browser attach`).

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde_json::Value;

/// Port `forge browser attach` asks for. Fixed on purpose: the user's config points at it.
pub const DEFAULT_ATTACH_PORT: u16 = 9222;

/// Browser executables to try, in order, with the label shown to the user.
const BROWSER_BINARIES: &[(&str, &str)] = &[
    ("google-chrome-stable", "Google Chrome"),
    ("google-chrome", "Google Chrome"),
    ("chromium", "Chromium"),
    ("chromium-browser", "Chromium"),
    ("brave-browser", "Brave"),
    ("brave", "Brave"),
    ("microsoft-edge-stable", "Microsoft Edge"),
    ("microsoft-edge", "Microsoft Edge"),
];

const MACOS_BROWSERS: &[(&str, &str)] = &[
    (
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        "Google Chrome",
    ),
    (
        "/Applications/Chromium.app/Contents/MacOS/Chromium",
        "Chromium",
    ),
    (
        "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser",
        "Brave",
    ),
    (
        "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
        "Microsoft Edge",
    ),
];

/// A browser executable found on this machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserBinary {
    pub name: &'static str,
    pub path: PathBuf,
}

/// Every Chromium-family browser found on `PATH` (and the macOS app locations), best first.
pub fn find_browsers() -> Vec<BrowserBinary> {
    let path_var = std::env::var_os("PATH").unwrap_or_default();
    let dirs: Vec<PathBuf> = std::env::split_paths(&path_var).collect();
    find_browsers_in(&dirs, MACOS_BROWSERS)
}

fn find_browsers_in(
    dirs: &[PathBuf],
    absolute: &[(&'static str, &'static str)],
) -> Vec<BrowserBinary> {
    let mut found: Vec<BrowserBinary> = Vec::new();
    let mut push = |name: &'static str, path: PathBuf| {
        if !found.iter().any(|b| b.path == path) {
            found.push(BrowserBinary { name, path });
        }
    };
    for (binary, name) in BROWSER_BINARIES {
        if let Some(path) = dirs.iter().map(|d| d.join(binary)).find(|p| p.is_file()) {
            push(name, path);
        }
    }
    for (path, name) in absolute {
        let path = Path::new(path);
        if path.is_file() {
            push(name, path.to_path_buf());
        }
    }
    found
}

/// The dedicated profile `forge browser attach` logs into: persistent, and never the default one.
pub fn default_attach_profile(data_home: &Path) -> PathBuf {
    data_home.join("forge").join("browser-attach-profile")
}

/// Command line that starts a browser Forge can attach to. Binds loopback only: anything that can
/// reach the debugging port can drive every logged-in session in this profile.
pub fn attach_launch_args(profile_dir: &Path, port: u16) -> Vec<String> {
    vec![
        format!("--remote-debugging-port={port}"),
        "--remote-debugging-address=127.0.0.1".to_string(),
        format!("--user-data-dir={}", profile_dir.display()),
        "--no-first-run".to_string(),
        "--no-default-browser-check".to_string(),
    ]
}

/// Where a running browser's DevTools endpoint lives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CdpEndpoint {
    /// `http://host:port`, no trailing slash.
    pub http_base: String,
    /// Browser-level WebSocket from `/json/version`.
    pub browser_ws_url: String,
    /// `Browser` field, e.g. `Chrome/152.0.0.0`.
    pub browser: String,
}

/// Normalise what a user types (`9222`, `host:9222`, `http://…`, `ws://…/devtools/browser/…`) to
/// an HTTP base URL.
pub fn normalize_endpoint(input: &str) -> Result<String> {
    let input = input.trim();
    if input.is_empty() {
        bail!("empty browser attach endpoint");
    }
    if input.chars().all(|c| c.is_ascii_digit()) {
        return Ok(format!("http://127.0.0.1:{input}"));
    }
    let (scheme, rest) = match input.split_once("://") {
        Some(("ws", rest)) => ("http", rest),
        Some(("wss", rest)) => ("https", rest),
        Some((scheme @ ("http" | "https"), rest)) => (scheme, rest),
        Some((other, _)) => {
            bail!("unsupported browser attach scheme '{other}' (use http:// or ws://)")
        }
        None => ("http", input),
    };
    let authority = rest.split('/').next().unwrap_or_default();
    if authority.is_empty() {
        bail!("browser attach endpoint '{input}' has no host");
    }
    Ok(format!("{scheme}://{authority}"))
}

/// Ask the browser who it is: `GET /json/version`.
pub async fn discover(endpoint: &str) -> Result<CdpEndpoint> {
    let http_base = normalize_endpoint(endpoint)?;
    let version: Value = reqwest::Client::new()
        .get(format!("{http_base}/json/version"))
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
        .with_context(|| {
            format!(
                "nothing answered at {http_base}/json/version. Start a browser with \
                 `forge browser attach` (or --remote-debugging-port=9222 on a non-default \
                 --user-data-dir) and point [browser] attach / FORGE_BROWSER_CDP at it"
            )
        })?
        .error_for_status()
        .with_context(|| format!("{http_base}/json/version returned an error"))?
        .json()
        .await
        .with_context(|| format!("{http_base}/json/version is not DevTools JSON"))?;
    let browser_ws_url = version
        .get("webSocketDebuggerUrl")
        .and_then(Value::as_str)
        .context("/json/version has no webSocketDebuggerUrl — not a Chromium DevTools endpoint")?
        .to_string();
    Ok(CdpEndpoint {
        http_base,
        browser_ws_url,
        browser: version
            .get("Browser")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string(),
    })
}

/// A tab Forge opened in the user's browser — the only target Forge may close.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnTab {
    pub http_base: String,
    pub id: String,
    pub ws_url: String,
}

/// Open a fresh `about:blank` tab (`PUT /json/new`) rather than adopting an existing one.
pub async fn open_own_tab(http_base: &str) -> Result<OwnTab> {
    let created: Value = reqwest::Client::new()
        .put(format!("{http_base}/json/new?about:blank"))
        .send()
        .await
        .context("open a new tab in the attached browser")?
        .error_for_status()
        .context("the attached browser refused to open a tab")?
        .json()
        .await
        .context("parse the new-tab response")?;
    let field = |name: &str| {
        created
            .get(name)
            .and_then(Value::as_str)
            .map(str::to_string)
            .with_context(|| format!("new-tab response has no {name}"))
    };
    Ok(OwnTab {
        http_base: http_base.to_string(),
        id: field("id")?,
        ws_url: field("webSocketDebuggerUrl")?,
    })
}

impl OwnTab {
    /// Close this tab only (`/json/close/<id>`).
    pub async fn close(&self) {
        let _ = reqwest::Client::new()
            .get(format!("{}/json/close/{}", self.http_base, self.id))
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoints_normalize_to_an_http_base() {
        assert_eq!(normalize_endpoint("9222").unwrap(), "http://127.0.0.1:9222");
        assert_eq!(
            normalize_endpoint("localhost:9222/").unwrap(),
            "http://localhost:9222"
        );
        assert_eq!(
            normalize_endpoint("ws://127.0.0.1:9333/devtools/browser/abc").unwrap(),
            "http://127.0.0.1:9333"
        );
        assert_eq!(
            normalize_endpoint(" http://127.0.0.1:9222/json/version ").unwrap(),
            "http://127.0.0.1:9222"
        );
        assert!(normalize_endpoint("").is_err());
        assert!(normalize_endpoint("ftp://x:1").is_err());
    }

    #[test]
    fn launch_args_use_a_dedicated_profile_and_loopback_only() {
        let args = attach_launch_args(Path::new("/data/forge/browser-attach-profile"), 9222);
        assert!(args.contains(&"--remote-debugging-port=9222".to_string()));
        assert!(args.contains(&"--remote-debugging-address=127.0.0.1".to_string()));
        assert!(args.contains(&"--user-data-dir=/data/forge/browser-attach-profile".to_string()));
    }

    #[test]
    fn browsers_are_detected_in_preference_order_without_duplicates() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["brave", "chromium", "google-chrome-stable", "not-a-browser"] {
            std::fs::write(dir.path().join(name), "").unwrap();
        }
        let found = find_browsers_in(&[dir.path().to_path_buf()], &[]);
        let names: Vec<_> = found.iter().map(|b| b.name).collect();
        assert_eq!(names, ["Google Chrome", "Chromium", "Brave"]);
    }
}
