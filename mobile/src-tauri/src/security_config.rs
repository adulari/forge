//! Both checks guard a desktop that opens to an empty, unclickable window while the same bundle
//! works in a browser.

use urlpattern::{UrlPattern, UrlPatternInit, UrlPatternMatchInput};

fn tauri_conf() -> serde_json::Value {
    serde_json::from_str(include_str!("../tauri.conf.json")).unwrap()
}

fn capability() -> serde_json::Value {
    serde_json::from_str(include_str!("../capabilities/default.json")).unwrap()
}

/// Mirrors `parse_url_pattern` in tauri-plugin-http 2.5 (src/scope.rs).
fn parse_scope_entry(raw: &str) -> UrlPattern {
    let mut init = UrlPatternInit::parse_constructor_string::<regex::Regex>(raw, None).unwrap();
    if init.search.as_deref().is_none_or(str::is_empty) {
        init.search = Some("*".into());
    }
    if init.hash.as_deref().is_none_or(str::is_empty) {
        init.hash = Some("*".into());
    }
    if init
        .pathname
        .as_deref()
        .is_none_or(|p| p.is_empty() || p == "/")
    {
        init.pathname = Some("*".into());
    }
    UrlPattern::parse(init, Default::default()).unwrap()
}

fn http_allow_list() -> Vec<UrlPattern> {
    capability()["permissions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["identifier"] == "http:default")
        .expect("http:default permission")["allow"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| parse_scope_entry(entry["url"].as_str().unwrap()))
        .collect()
}

/// Tauri hashes the bundle's inline scripts and styles into the CSP. A nonce or hash in
/// `style-src` makes WebViews ignore `'unsafe-inline'`, so the `<style>` tags Reanimated injects
/// at runtime were refused: every entering animation stayed at its hidden first frame, and the
/// invisible update sheet sat over the window swallowing clicks.
#[test]
fn style_src_keeps_unsafe_inline() {
    let security = &tauri_conf()["app"]["security"];
    let csp = security["csp"].as_str().unwrap();
    let style_src = csp
        .split(';')
        .map(str::trim)
        .find(|d| d.starts_with("style-src"))
        .unwrap();
    assert!(style_src.contains("'unsafe-inline'"), "{style_src}");
    let untouched = security["dangerousDisableAssetCspModification"]
        .as_array()
        .expect("style-src must be excluded from Tauri's CSP hashing");
    assert!(untouched.iter().any(|d| d == "style-src"));
}

/// URLPattern reads `http://*/*` as "default port only", so a `forge serve` daemon on 7420 was
/// refused with "url not allowed on the configured scope" and the fleet never loaded.
#[test]
fn http_scope_reaches_daemons_on_any_port() {
    let allow = http_allow_list();
    for url in [
        "http://127.0.0.1:7420/0123456789abcdef0123456789abcdef/api/sessions",
        "http://localhost:7420/0123456789abcdef0123456789abcdef/api/sessions?limit=5",
        "http://100.64.1.2:7421/abcdef0123456789abcdef0123456789/api/changelog",
        "http://my-box.local/abcdef0123456789abcdef0123456789/api/sessions",
        "https://forge.example.com:8443/abcdef0123456789abcdef0123456789/api/sessions",
        "https://forge.example.com/abcdef0123456789abcdef0123456789/api/sessions",
    ] {
        let parsed = url::Url::parse(url).unwrap();
        assert!(
            allow
                .iter()
                .any(|p| p.test(UrlPatternMatchInput::Url(parsed.clone())).unwrap()),
            "{url} is outside the desktop HTTP scope"
        );
    }
}

/// Windows and Linux run undecorated and draw their own title bar. `core:default` only grants
/// reading window state, so dragging and the minimize/maximize/close buttons were all refused.
#[test]
fn custom_title_bar_can_move_and_control_the_window() {
    let granted: Vec<String> = capability()["permissions"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|p| p.as_str().map(str::to_owned))
        .collect();
    for needed in [
        "core:window:allow-start-dragging",
        "core:window:allow-minimize",
        "core:window:allow-maximize",
        "core:window:allow-unmaximize",
        "core:window:allow-close",
    ] {
        assert!(granted.iter().any(|g| g == needed), "missing {needed}");
    }
}
