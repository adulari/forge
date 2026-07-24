// Machined About panel (docs/design/machined/Desktop.dc.html L983-987): ember mark, wordmark,
// `2.7.1 (4812) · protocol v7`, `daemon 2.7.1 · MacBook Pro`, and Check for updates /
// Acknowledgements buttons.
//
// WHY a dedicated shell window instead of an RN route or the OS About panel:
//   * The OS panel (`PredefinedMenuItem::about`) cannot render the Machined card the design
//     specifies, and cannot show live daemon version/host at all.
//   * An RN route would only exist once the bundle has booted and a server is paired, but
//     "About" is exactly what a user opens when the app is *not* working — and it would put
//     this surface in `src/app/`, which other builders are editing concurrently.
// So the page is a self-contained document served by a private URI scheme: no JS, no IPC, no
// capability grant, and it renders identically before the RN bundle has loaded. The two
// buttons are plain links whose navigation is intercepted below and turned into ordinary menu
// events, so they run the same TS handlers the menu bar's items do.
use std::borrow::Cow;
use std::sync::Mutex;

use tauri::{AppHandle, Manager, Runtime, WebviewUrl, WebviewWindowBuilder};

use crate::menu;

pub const WINDOW_LABEL: &str = "about";
pub const URI_SCHEME: &str = "forge-about";

/// Machined tokens (theme/tokens.ts dark scheme) — hard-coded here because this document is
/// outside the RN theme system by design.
const BG: &str = "#09090B";
const PANEL: &str = "#0E0E12";
const HAIRLINE: &str = "rgba(244,244,246,.11)";
const INK: &str = "#F4F4F6";
const INK2: &str = "#9A9AA6";
const INK4: &str = "#5F5F6B";
const EMBER: &str = "#FF8A3D";

// Geist ships with the app bundle but not with the OS, and this document cannot reach the RN
// asset pipeline — embedding the three faces it uses is the only way the panel matches the rest
// of the app rather than falling back to the system UI font.
const GEIST_REGULAR: &[u8] = include_bytes!("../../assets/Geist-Regular.ttf");
const GEIST_SEMIBOLD: &[u8] = include_bytes!("../../assets/Geist-SemiBold.ttf");
const GEIST_MONO: &[u8] = include_bytes!("../../assets/GeistMono-Regular.ttf");

/// Facts the shell cannot know: the webview owns the daemon connection and the protocol
/// handshake, so it pushes them in with `set_about_info` (see `desktopMenu.ts`).
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AboutInfo {
    pub app_version: Option<String>,
    pub build: Option<String>,
    pub protocol: Option<String>,
    pub daemon_version: Option<String>,
    pub host: Option<String>,
}

#[derive(Default)]
pub struct AboutState(pub Mutex<AboutInfo>);

/// Merges rather than replaces: the app-level facts (version, protocol) and the daemon-level
/// ones (version, host) are known by different parts of the webview and pushed independently,
/// so a later partial push must not blank an earlier one.
#[tauri::command]
pub fn set_about_info<R: Runtime>(app: AppHandle<R>, info: AboutInfo) {
    if let Ok(mut slot) = app.state::<AboutState>().0.lock() {
        if info.app_version.is_some() {
            slot.app_version = info.app_version;
        }
        if info.build.is_some() {
            slot.build = info.build;
        }
        if info.protocol.is_some() {
            slot.protocol = info.protocol;
        }
        if info.daemon_version.is_some() {
            slot.daemon_version = info.daemon_version;
        }
        if info.host.is_some() {
            slot.host = info.host;
        }
    }
}

#[tauri::command]
pub fn open_about_window<R: Runtime>(app: AppHandle<R>) -> Result<(), String> {
    open(&app).map_err(|e| e.to_string())
}

/// Windows and Android serve custom schemes over `http://<scheme>.localhost`; everywhere else
/// keeps the real scheme. Getting this wrong is a blank window, not a compile error, hence the
/// explicit branch.
fn about_url() -> tauri::Url {
    let raw = if cfg!(any(target_os = "windows", target_os = "android")) {
        "http://forge-about.localhost/"
    } else {
        "forge-about://localhost/"
    };
    raw.parse().expect("about url literal is valid")
}

pub fn open<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    if let Some(existing) = app.get_webview_window(WINDOW_LABEL) {
        existing.show()?;
        existing.set_focus()?;
        return Ok(());
    }

    let handle = app.clone();
    WebviewWindowBuilder::new(app, WINDOW_LABEL, WebviewUrl::CustomProtocol(about_url()))
        .title("About Forge")
        .inner_size(340.0, 320.0)
        .resizable(false)
        .maximizable(false)
        .minimizable(false)
        .center()
        .on_navigation(move |url| {
            // The page's own load. Everything else is a button press: convert it to the menu
            // event the TS side already handles and cancel the navigation.
            if url.scheme() == URI_SCHEME || url.host_str() == Some("forge-about.localhost") {
                return true;
            }
            let action = match url.host_str().unwrap_or_default() {
                "check-updates" => Some(menu::APP_CHECK_UPDATES),
                "acknowledgements" => Some(menu::HELP_ACKNOWLEDGEMENTS),
                _ => None,
            };
            if let Some(action) = action {
                // Both actions land somewhere in the main window (Settings' updater card, the
                // browser). Raise it, or the result of the click happens behind this panel.
                if let Some(main) = handle.get_webview_window("main") {
                    let _ = main.show();
                    let _ = main.set_focus();
                }
                menu::handle_event(&handle, action);
            }
            false
        })
        .build()?;
    Ok(())
}

/// URI-scheme handler: `/` renders the panel, `/*.ttf` serves the embedded faces.
pub fn serve<R: Runtime>(
    ctx: tauri::UriSchemeContext<'_, R>,
    request: tauri::http::Request<Vec<u8>>,
) -> tauri::http::Response<Cow<'static, [u8]>> {
    let font = |bytes: &'static [u8]| {
        tauri::http::Response::builder()
            .header("Content-Type", "font/ttf")
            .header("Cache-Control", "max-age=31536000")
            .body(Cow::Borrowed(bytes))
            .expect("static font response is well-formed")
    };

    match request.uri().path() {
        "/geist-regular.ttf" => font(GEIST_REGULAR),
        "/geist-semibold.ttf" => font(GEIST_SEMIBOLD),
        "/geist-mono.ttf" => font(GEIST_MONO),
        _ => {
            let info = ctx
                .app_handle()
                .try_state::<AboutState>()
                .and_then(|state| state.0.lock().ok().map(|guard| guard.clone()))
                .unwrap_or_default();
            let fallback = ctx.app_handle().package_info().version.to_string();
            tauri::http::Response::builder()
                .header("Content-Type", "text/html; charset=utf-8")
                .body(Cow::Owned(render(&info, &fallback).into_bytes()))
                .expect("about response is well-formed")
        }
    }
}

/// Minimal escaping — every interpolated value below is a version string or a hostname, but
/// hostnames come off the network and this document has no other sanitiser.
fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn render(info: &AboutInfo, fallback_version: &str) -> String {
    let version = info
        .app_version
        .as_deref()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or(fallback_version);

    let mut version_line = escape(version);
    if let Some(build) = info.build.as_deref().filter(|b| !b.trim().is_empty()) {
        version_line.push_str(&format!(" ({})", escape(build)));
    }
    if let Some(protocol) = info.protocol.as_deref().filter(|p| !p.trim().is_empty()) {
        version_line.push_str(&format!(" · protocol {}", escape(protocol)));
    }

    // Rendered only when something is actually known — an unpaired app has no daemon to report,
    // and an empty line beats asserting a connection state we cannot see from here.
    let daemon_line = match (info.daemon_version.as_deref(), info.host.as_deref()) {
        (Some(v), Some(h)) if !v.trim().is_empty() && !h.trim().is_empty() => {
            format!(
                r#"<span class="meta">daemon {} · {}</span>"#,
                escape(v),
                escape(h)
            )
        }
        (Some(v), _) if !v.trim().is_empty() => {
            format!(r#"<span class="meta">daemon {}</span>"#, escape(v))
        }
        (_, Some(h)) if !h.trim().is_empty() => {
            format!(r#"<span class="meta">{}</span>"#, escape(h))
        }
        _ => String::new(),
    };

    format!(
        r##"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8" />
<title>About Forge</title>
<style>
  @font-face {{ font-family: 'Geist'; font-weight: 400; src: url('/geist-regular.ttf') format('truetype'); }}
  @font-face {{ font-family: 'Geist'; font-weight: 600; src: url('/geist-semibold.ttf') format('truetype'); }}
  @font-face {{ font-family: 'Geist Mono'; font-weight: 400; src: url('/geist-mono.ttf') format('truetype'); }}
  html, body {{ margin: 0; height: 100%; background: {BG}; }}
  body {{
    display: flex; align-items: center; justify-content: center;
    font-family: 'Geist', -apple-system, system-ui, sans-serif; color: {INK};
    -webkit-user-select: none; user-select: none; cursor: default;
  }}
  .panel {{
    width: 300px; box-sizing: border-box; background: {PANEL};
    border: 1px solid {HAIRLINE}; border-radius: 6px; padding: 22px;
    display: flex; flex-direction: column; align-items: center; gap: 8px;
  }}
  .name {{ font-size: 15px; font-weight: 600; letter-spacing: -0.2px; }}
  .meta {{ font-family: 'Geist Mono', ui-monospace, monospace; font-size: 10px; color: {INK4}; }}
  .actions {{ display: flex; gap: 7px; padding-top: 4px; }}
  a.button {{
    font-size: 10px; color: {INK2}; text-decoration: none;
    border: 1px solid rgba(244,244,246,.1); border-radius: 3px; padding: 3px 10px;
  }}
  a.button:hover {{ color: {INK}; border-color: {HAIRLINE}; }}
</style>
</head>
<body>
  <div class="panel">
    <svg width="34" height="34" viewBox="0 0 24 24" fill="{EMBER}" aria-hidden="true"><path d="M12 2l2.4 7.6L22 12l-7.6 2.4L12 22l-2.4-7.6L2 12l7.6-2.4z"></path></svg>
    <span class="name">Forge</span>
    <span class="meta">{version_line}</span>
    {daemon_line}
    <div class="actions">
      <a class="button" href="forge://check-updates">Check for updates</a>
      <a class="button" href="forge://acknowledgements">Acknowledgements</a>
    </div>
  </div>
</body>
</html>"##
    )
}
