//! Finding, launching, and re-attaching to a real Chrome.
//!
//! Two facts drive every decision in this file, both verified against Chrome 152 rather than
//! assumed:
//!
//! 1. **The default profile cannot be driven.** Chrome refuses remote debugging against its
//!    default user-data-dir — *"DevTools remote debugging requires a non-default data directory.
//!    Specify this using --user-data-dir."* So "control the user's ordinary browser" cannot mean
//!    "attach to the profile they already use". It means a real, headful, persistent profile that
//!    Forge owns, which keeps cookies and logins across runs and is a genuine browser in every
//!    other respect — not a sterile throwaway.
//! 2. **The debugging port must not be guessed.** A fixed port collides with anything else on the
//!    machine and silently attaches to the wrong browser. Chrome writes the port it actually bound
//!    to `DevToolsActivePort` inside the profile dir, so we ask for port 0 and read back the truth.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde_json::Value;

/// How long to wait for Chrome to write `DevToolsActivePort` after launch.
const PORT_FILE_TIMEOUT: Duration = Duration::from_secs(20);
const PORT_FILE_POLL: Duration = Duration::from_millis(100);

/// Executables to try, in order. `chromium` last: where both exist the user means Chrome.
const CHROME_BINARIES: &[&str] = &[
    "google-chrome-stable",
    "google-chrome",
    "chrome",
    "chromium-browser",
    "chromium",
];

/// macOS keeps Chrome outside `PATH`.
const MACOS_CHROME_PATHS: &[&str] = &[
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    "/Applications/Chromium.app/Contents/MacOS/Chromium",
];

/// How a browser session should present itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Display {
    /// A real window on the user's desktop. The default, and the point of the feature: a headless
    /// browser is fingerprinted and blocked by exactly the sites worth reverse-engineering.
    #[default]
    Windowed,
    /// No window. For CI, a headless host, or bulk work where nobody is watching.
    Headless,
}

/// Identity overrides applied to a session so a specific device can be emulated.
///
/// This is NOT the profile's real fingerprint (that comes from the OS and the Chrome build). It is
/// a set of CDP overrides — the same ones DevTools' device toolbar sets — enough to present a
/// consistent, chosen device to a server rather than the host's defaults. It does not reach the
/// depth of a dedicated anti-detect build (gologin and friends patch the binary), and that limit
/// is stated where a caller will see it rather than discovered when a site sees through it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Fingerprint {
    /// `navigator.userAgent`. Set via CDP so `platform` and `accept_language` move with it, rather
    /// than a launch-flag UA that leaves `navigator.platform` contradicting it.
    pub user_agent: Option<String>,
    /// `Accept-Language` header and `navigator.language`.
    pub accept_language: Option<String>,
    /// `navigator.platform`, e.g. `Win32`, `MacIntel`, `Linux x86_64`.
    pub platform: Option<String>,
    /// IANA timezone, e.g. `Europe/Amsterdam`. A timezone that contradicts the proxy's exit IP is
    /// a classic tell, so this pairs with `proxy`.
    pub timezone: Option<String>,
    /// Viewport as `(width, height)` in CSS pixels.
    pub viewport: Option<(u32, u32)>,
    /// Device scale factor (Retina = 2.0). Ignored unless `viewport` is set.
    pub device_scale_factor: Option<f64>,
    /// Present as a mobile device (touch, mobile UA hints). Ignored unless `viewport` is set.
    pub mobile: bool,
}

impl Fingerprint {
    pub fn is_empty(&self) -> bool {
        *self == Fingerprint::default()
    }
}

/// Everything needed to start or re-attach to a browser.
#[derive(Debug, Clone, Default)]
pub struct LaunchConfig {
    /// Profile directory. Persistent, so a login survives until the user clears it.
    pub profile_dir: PathBuf,
    pub display: Display,
    /// Upstream proxy, e.g. `http://user:pass@host:port` or `socks5://host:port`. Applied at
    /// launch (`--proxy-server`) because Chrome cannot change its proxy after start. A per-account
    /// proxy is why this exists: the ss-ultimate flow assigns one proxy per identity.
    pub proxy: Option<String>,
    /// Device/identity overrides. Applied via CDP after attach.
    pub fingerprint: Fingerprint,
    /// Extra Chrome flags, appended last so they win.
    pub extra_args: Vec<String>,
    /// Explicit binary; otherwise discovered.
    pub binary: Option<PathBuf>,
}

impl LaunchConfig {
    pub fn new(profile_dir: impl Into<PathBuf>) -> Self {
        Self {
            profile_dir: profile_dir.into(),
            ..Self::default()
        }
    }

    pub fn headless(mut self, headless: bool) -> Self {
        self.display = if headless {
            Display::Headless
        } else {
            Display::Windowed
        };
        self
    }

    pub fn proxy(mut self, proxy: impl Into<String>) -> Self {
        self.proxy = Some(proxy.into());
        self
    }
}

/// Locate a Chrome-family binary.
pub fn find_chrome() -> Result<PathBuf> {
    for candidate in CHROME_BINARIES {
        if let Ok(path) = which(candidate) {
            return Ok(path);
        }
    }
    for candidate in MACOS_CHROME_PATHS {
        let path = Path::new(candidate);
        if path.exists() {
            return Ok(path.to_path_buf());
        }
    }
    bail!(
        "no Chrome-family browser found on PATH (looked for: {}). Install Google Chrome or \
         Chromium, or set the binary explicitly.",
        CHROME_BINARIES.join(", ")
    )
}

fn which(binary: &str) -> Result<PathBuf> {
    let path = std::env::var_os("PATH").context("PATH is not set")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(binary);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    bail!("{binary} is not on PATH")
}

/// The command-line Chrome is launched with.
///
/// `--remote-debugging-port=0` is deliberate: see the module docs. `--no-first-run` and
/// `--no-default-browser-check` keep a fresh profile from opening dialogs that would sit in front
/// of the page the caller asked for.
pub fn chrome_args(config: &LaunchConfig) -> Vec<String> {
    let mut args = vec![
        "--remote-debugging-port=0".to_string(),
        format!("--user-data-dir={}", config.profile_dir.display()),
        "--no-first-run".to_string(),
        "--no-default-browser-check".to_string(),
        // Chrome's own automation banner and the `navigator.webdriver` flag are the first things
        // a bot check looks at. This browser is driven on the user's behalf, in their own
        // session, so presenting it as a normal browser is the accurate description of what it
        // is — and the whole reason a windowed real profile beats a headless throwaway.
        "--disable-blink-features=AutomationControlled".to_string(),
        "--disable-features=Translate,MediaRouter".to_string(),
    ];
    if let Some(proxy) = &config.proxy {
        // Must be a launch flag: Chrome resolves its proxy once, at start. A later CDP override
        // does not exist for the browser-wide proxy.
        args.push(format!("--proxy-server={proxy}"));
    }
    if let Some((width, height)) = config.fingerprint.viewport {
        // A window that matches the emulated viewport avoids a headful browser reporting an
        // outer size that contradicts the metrics override.
        args.push(format!("--window-size={width},{height}"));
    }
    if config.display == Display::Headless {
        args.push("--headless=new".to_string());
    }
    args.extend(config.extra_args.iter().cloned());
    args.push("about:blank".to_string());
    args
}

/// The CDP commands that apply a fingerprint, in order. Returned as data — `(method, params)` —
/// so the exact overrides are unit-testable without a browser, and applied by the session after
/// attach. Empty when the fingerprint is empty, so a caller pays nothing for not using it.
pub fn fingerprint_commands(fingerprint: &Fingerprint) -> Vec<(&'static str, serde_json::Value)> {
    use serde_json::json;
    let mut commands = Vec::new();

    if fingerprint.user_agent.is_some()
        || fingerprint.accept_language.is_some()
        || fingerprint.platform.is_some()
    {
        let mut params = serde_json::Map::new();
        // A UA override with no userAgent is invalid; fall back to a neutral one so platform and
        // language can still be set on their own.
        params.insert(
            "userAgent".into(),
            json!(fingerprint.user_agent.clone().unwrap_or_default()),
        );
        if let Some(language) = &fingerprint.accept_language {
            params.insert("acceptLanguage".into(), json!(language));
        }
        if let Some(platform) = &fingerprint.platform {
            params.insert("platform".into(), json!(platform));
        }
        commands.push(("Network.setUserAgentOverride", Value::Object(params)));
    }

    if let Some(timezone) = &fingerprint.timezone {
        commands.push((
            "Emulation.setTimezoneOverride",
            json!({ "timezoneId": timezone }),
        ));
    }

    if let Some(language) = &fingerprint.accept_language {
        // Keeps `navigator.language` consistent with the Accept-Language header above; a mismatch
        // between the two is a cheap bot tell.
        commands.push(("Emulation.setLocaleOverride", json!({ "locale": language })));
    }

    if let Some((width, height)) = fingerprint.viewport {
        commands.push((
            "Emulation.setDeviceMetricsOverride",
            json!({
                "width": width,
                "height": height,
                "deviceScaleFactor": fingerprint.device_scale_factor.unwrap_or(1.0),
                "mobile": fingerprint.mobile
            }),
        ));
    }

    commands
}

/// What Chrome wrote to `DevToolsActivePort`: the bound port and the browser-level WS path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivePort {
    pub port: u16,
    pub browser_ws_path: String,
}

impl ActivePort {
    pub fn http_base(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }
}

/// Parse `DevToolsActivePort`: the port on line one, the browser WS path on line two.
///
/// Returns `None` for a partially-written file rather than a wrong answer — Chrome creates it and
/// fills it in two steps, so a reader that races the write sees a valid-looking port with no path.
pub fn parse_active_port(contents: &str) -> Option<ActivePort> {
    let mut lines = contents.lines();
    let port: u16 = lines.next()?.trim().parse().ok()?;
    if port == 0 {
        return None;
    }
    let path = lines.next()?.trim();
    if !path.starts_with("/devtools/") {
        return None;
    }
    Some(ActivePort {
        port,
        browser_ws_path: path.to_string(),
    })
}

/// Wait for Chrome to publish its port, polling the profile dir.
pub async fn wait_for_active_port(profile_dir: &Path) -> Result<ActivePort> {
    let file = profile_dir.join("DevToolsActivePort");
    let deadline = std::time::Instant::now() + PORT_FILE_TIMEOUT;
    loop {
        if let Ok(contents) = tokio::fs::read_to_string(&file).await {
            if let Some(active) = parse_active_port(&contents) {
                return Ok(active);
            }
        }
        if std::time::Instant::now() >= deadline {
            bail!(
                "Chrome did not report a debugging port within {}s ({} was missing or \
                 incomplete). If a Chrome is already running on this profile directory, close it \
                 first — a second launch attaches to the first and never writes the file.",
                PORT_FILE_TIMEOUT.as_secs(),
                file.display()
            );
        }
        tokio::time::sleep(PORT_FILE_POLL).await;
    }
}

/// A launched browser process, killed on drop unless [`Self::detach`] is called.
#[derive(Debug)]
pub struct BrowserProcess {
    child: Option<tokio::process::Child>,
    pub active: ActivePort,
    pub profile_dir: PathBuf,
}

impl BrowserProcess {
    /// Launch Chrome and wait until it is driveable.
    pub async fn launch(config: &LaunchConfig) -> Result<Self> {
        let binary = match &config.binary {
            Some(path) => path.clone(),
            None => find_chrome()?,
        };
        tokio::fs::create_dir_all(&config.profile_dir)
            .await
            .with_context(|| {
                format!(
                    "create browser profile directory {}",
                    config.profile_dir.display()
                )
            })?;
        // A stale port file from a previous run would be read as this run's port and connect to
        // nothing. Chrome removes it on clean exit; a killed browser leaves it behind.
        let _ = tokio::fs::remove_file(config.profile_dir.join("DevToolsActivePort")).await;

        let child = tokio::process::Command::new(&binary)
            .args(chrome_args(config))
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .with_context(|| format!("launch {}", binary.display()))?;

        let active = wait_for_active_port(&config.profile_dir).await?;
        Ok(Self {
            child: Some(child),
            active,
            profile_dir: config.profile_dir.clone(),
        })
    }

    /// Re-attach to a browser already running on this profile, without launching one.
    pub async fn attach(profile_dir: &Path) -> Result<Self> {
        let file = profile_dir.join("DevToolsActivePort");
        let contents = tokio::fs::read_to_string(&file).await.with_context(|| {
            format!(
                "no running Forge browser for this profile ({} is absent)",
                file.display()
            )
        })?;
        let active =
            parse_active_port(&contents).context("DevToolsActivePort is present but unreadable")?;
        Ok(Self {
            child: None,
            active,
            profile_dir: profile_dir.to_path_buf(),
        })
    }

    /// Leave the browser running after this handle drops, so the window the user is looking at
    /// (and logged into) survives the end of a turn.
    pub fn detach(&mut self) {
        if let Some(child) = self.child.take() {
            // Into the runtime's hands: no kill_on_drop, no wait.
            std::mem::forget(child);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_partially_written_port_file_is_not_a_port() {
        // Chrome creates the file and fills it in two steps. A reader that races the write must
        // not return a port with no WS path and then fail to connect for reasons it cannot
        // explain — it must simply keep waiting.
        assert_eq!(parse_active_port(""), None);
        assert_eq!(parse_active_port("33955"), None);
        assert_eq!(parse_active_port("33955\n"), None);
        assert_eq!(parse_active_port("0\n/devtools/browser/abc"), None);
        assert_eq!(parse_active_port("not-a-port\n/devtools/browser/abc"), None);
        assert_eq!(parse_active_port("33955\ngarbage"), None);
    }

    #[test]
    fn a_complete_port_file_parses() {
        let active = parse_active_port("33955\n/devtools/browser/e8176db2-bc48\n")
            .expect("a complete file parses");
        assert_eq!(active.port, 33955);
        assert_eq!(active.browser_ws_path, "/devtools/browser/e8176db2-bc48");
        assert_eq!(active.http_base(), "http://127.0.0.1:33955");
    }

    #[test]
    fn the_launch_line_never_pins_a_port_and_never_uses_the_default_profile() {
        // Both are load-bearing. A fixed port silently attaches to whatever else is listening;
        // the default profile is refused outright by Chrome 136+ with
        // "DevTools remote debugging requires a non-default data directory".
        let config = LaunchConfig::new("/tmp/forge-profile");
        let args = chrome_args(&config);
        assert!(
            args.iter().any(|a| a == "--remote-debugging-port=0"),
            "{args:?}"
        );
        assert!(
            args.iter()
                .any(|a| a == "--user-data-dir=/tmp/forge-profile"),
            "{args:?}"
        );
        assert!(
            !args.iter().any(|a| a == "--headless=new"),
            "a windowed browser is the default: {args:?}"
        );
    }

    #[test]
    fn a_proxy_is_a_launch_flag_because_chrome_cannot_change_it_later() {
        let config = LaunchConfig::new("/tmp/p").proxy("socks5://127.0.0.1:9050");
        let args = chrome_args(&config);
        assert!(
            args.iter()
                .any(|a| a == "--proxy-server=socks5://127.0.0.1:9050"),
            "{args:?}"
        );
    }

    #[test]
    fn an_empty_fingerprint_costs_nothing() {
        assert!(Fingerprint::default().is_empty());
        assert!(fingerprint_commands(&Fingerprint::default()).is_empty());
    }

    #[test]
    fn a_user_agent_override_carries_platform_and_language_together() {
        // Setting the UA via a launch flag leaves navigator.platform contradicting it — a tell.
        // The CDP override moves all three as one.
        let fingerprint = Fingerprint {
            user_agent: Some("Mozilla/5.0 (Windows NT 10.0; Win64; x64)".into()),
            accept_language: Some("nl-NL".into()),
            platform: Some("Win32".into()),
            ..Fingerprint::default()
        };
        let commands = fingerprint_commands(&fingerprint);
        let ua = commands
            .iter()
            .find(|(method, _)| *method == "Network.setUserAgentOverride")
            .expect("a UA override");
        assert_eq!(ua.1["platform"], "Win32");
        assert_eq!(ua.1["acceptLanguage"], "nl-NL");
        assert!(ua.1["userAgent"].as_str().unwrap().contains("Windows"));
        // Locale is set too, so navigator.language matches the header.
        assert!(
            commands
                .iter()
                .any(|(method, params)| *method == "Emulation.setLocaleOverride"
                    && params["locale"] == "nl-NL"),
            "{commands:?}"
        );
    }

    #[test]
    fn a_viewport_emulates_a_device_and_sizes_the_window() {
        let fingerprint = Fingerprint {
            viewport: Some((390, 844)),
            device_scale_factor: Some(3.0),
            mobile: true,
            ..Fingerprint::default()
        };
        let metrics = fingerprint_commands(&fingerprint)
            .into_iter()
            .find(|(method, _)| *method == "Emulation.setDeviceMetricsOverride")
            .expect("device metrics");
        assert_eq!(metrics.1["width"], 390);
        assert_eq!(metrics.1["deviceScaleFactor"], 3.0);
        assert_eq!(metrics.1["mobile"], true);

        let args = chrome_args(&LaunchConfig {
            fingerprint,
            ..LaunchConfig::new("/tmp/p")
        });
        assert!(
            args.iter().any(|a| a == "--window-size=390,844"),
            "{args:?}"
        );
    }

    #[test]
    fn headless_is_opt_in_and_extra_args_win() {
        let mut config = LaunchConfig::new("/tmp/forge-profile").headless(true);
        config
            .extra_args
            .push("--proxy-server=127.0.0.1:8080".into());
        let args = chrome_args(&config);
        assert!(args.iter().any(|a| a == "--headless=new"), "{args:?}");
        let proxy = args
            .iter()
            .position(|a| a == "--proxy-server=127.0.0.1:8080")
            .expect("extra arg present");
        let profile = args
            .iter()
            .position(|a| a.starts_with("--user-data-dir="))
            .expect("profile arg present");
        assert!(
            proxy > profile,
            "extra args must come last so they can override: {args:?}"
        );
    }

    #[tokio::test]
    async fn a_missing_port_file_explains_the_likely_cause() {
        let dir = tempfile::tempdir().expect("temp dir");
        let err = BrowserProcess::attach(dir.path())
            .await
            .expect_err("no browser is running here");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("no running Forge browser"),
            "unhelpful error: {msg}"
        );
    }
}
