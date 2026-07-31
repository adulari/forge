//! Isolated browser-preview child webviews.
//!
//! Preview pages are arbitrary, untrusted HTTP(S) documents. They live in a child webview whose
//! label is outside the main window capability and never receive a Forge IPC bridge. Element
//! picking uses an initialization script that attempts a `forge-preview-annotation://` navigation;
//! the native navigation handler intercepts that URL, emits the bounded payload to the trusted
//! main webview, and cancels the navigation.

use serde::{Deserialize, Serialize};
use tauri::{
    webview::{PageLoadEvent, WebviewBuilder},
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, WebviewUrl,
};

const MAIN_WINDOW: &str = "main";
const STATE_EVENT: &str = "forge://preview-state";
const ANNOTATION_EVENT: &str = "forge://preview-annotation";
const ANNOTATION_SCHEME: &str = "forge-preview-annotation";
const MAX_ANNOTATION_PAYLOAD_BYTES: usize = 32 * 1024;

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewBounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewState {
    pub label: String,
    pub url: String,
    pub loaded: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PreviewElementAnnotation {
    pub url: String,
    pub title: String,
    pub selector: String,
    pub tag_name: String,
    pub element_id: Option<String>,
    pub role: Option<String>,
    pub accessible_name: Option<String>,
    pub text: String,
    pub attributes: Vec<PreviewElementAttribute>,
    pub rect: PreviewElementRect,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PreviewElementAttribute {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PreviewElementRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PreviewAnnotationEvent {
    label: String,
    annotation: PreviewElementAnnotation,
}

fn validate_label(label: &str) -> Result<(), String> {
    let valid = label.starts_with("preview-")
        && label.len() <= 96
        && label
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'));
    if valid {
        Ok(())
    } else {
        Err("invalid preview label".to_string())
    }
}

fn parse_http_url(raw: &str) -> Result<tauri::Url, String> {
    let url = raw
        .parse::<tauri::Url>()
        .map_err(|_| "preview URL is invalid".to_string())?;
    if matches!(url.scheme(), "http" | "https") {
        Ok(url)
    } else {
        Err("preview URL must use http or https".to_string())
    }
}

fn validate_bounds(bounds: PreviewBounds) -> Result<PreviewBounds, String> {
    let values = [bounds.x, bounds.y, bounds.width, bounds.height];
    if values.iter().any(|value| !value.is_finite())
        || bounds.x < 0.0
        || bounds.y < 0.0
        || !(1.0..=16_384.0).contains(&bounds.width)
        || !(1.0..=16_384.0).contains(&bounds.height)
    {
        return Err("invalid preview bounds".to_string());
    }
    Ok(bounds)
}

fn annotation_from_navigation(url: &tauri::Url) -> Option<PreviewElementAnnotation> {
    if url.scheme() != ANNOTATION_SCHEME {
        return None;
    }
    let payload = url
        .query_pairs()
        .find_map(|(key, value)| (key == "payload").then_some(value.into_owned()))?;
    if payload.len() > MAX_ANNOTATION_PAYLOAD_BYTES {
        return None;
    }
    serde_json::from_str(&payload).ok()
}

fn picker_script(label: &str) -> String {
    let encoded_label = serde_json::to_string(label).expect("preview label serializes");
    format!(
        r##"
(() => {{
  if (window.__forgePreview) return;
  const label = {encoded_label};
  const state = {{ active: false, hovered: null, overlay: null, badge: null }};
  const bounded = (value, length) => String(value ?? "").replace(/\s+/g, " ").trim().slice(0, length);
  const cssEscape = (value) => {{
    if (window.CSS?.escape) return window.CSS.escape(value);
    return String(value).replace(/[^a-zA-Z0-9_-]/g, (char) => `\\${{char}}`);
  }};
  const selectorFor = (element) => {{
    if (element.id) return `#${{cssEscape(element.id)}}`;
    const parts = [];
    let node = element;
    while (node && node.nodeType === Node.ELEMENT_NODE && parts.length < 7) {{
      let part = node.tagName.toLowerCase();
      const parent = node.parentElement;
      if (parent) {{
        const peers = Array.from(parent.children).filter((candidate) => candidate.tagName === node.tagName);
        if (peers.length > 1) part += `:nth-of-type(${{peers.indexOf(node) + 1}})`;
      }}
      parts.unshift(part);
      const candidate = parts.join(" > ");
      try {{
        if (document.querySelectorAll(candidate).length === 1) return candidate;
      }} catch {{}}
      node = parent;
    }}
    return parts.join(" > ");
  }};
  const ensureOverlay = () => {{
    if (state.overlay) return;
    const overlay = document.createElement("div");
    overlay.dataset.forgePreviewPicker = "outline";
    Object.assign(overlay.style, {{
      position: "fixed", pointerEvents: "none", zIndex: "2147483646",
      border: "2px solid #7c5cff", background: "rgba(124,92,255,.10)",
      boxSizing: "border-box", display: "none"
    }});
    const badge = document.createElement("div");
    Object.assign(badge.style, {{
      position: "fixed", pointerEvents: "none", zIndex: "2147483647",
      padding: "3px 6px", borderRadius: "3px", background: "#17131f", color: "#f8f7fb",
      font: "11px/1.3 ui-monospace, SFMono-Regular, Menlo, monospace",
      maxWidth: "min(420px, 90vw)", overflow: "hidden", textOverflow: "ellipsis",
      whiteSpace: "nowrap", display: "none"
    }});
    document.documentElement.append(overlay, badge);
    state.overlay = overlay;
    state.badge = badge;
  }};
  const draw = (element) => {{
    ensureOverlay();
    const rect = element.getBoundingClientRect();
    Object.assign(state.overlay.style, {{
      display: "block", left: `${{rect.left}}px`, top: `${{rect.top}}px`,
      width: `${{rect.width}}px`, height: `${{rect.height}}px`
    }});
    state.badge.textContent = `${{element.tagName.toLowerCase()}}  ${{selectorFor(element)}}`;
    const badgeTop = rect.top >= 24 ? rect.top - 22 : Math.min(innerHeight - 22, rect.bottom + 4);
    Object.assign(state.badge.style, {{
      display: "block", left: `${{Math.max(4, Math.min(rect.left, innerWidth - 240))}}px`,
      top: `${{Math.max(2, badgeTop)}}px`
    }});
  }};
  const cleanup = () => {{
    state.active = false;
    state.hovered = null;
    document.removeEventListener("mousemove", onMove, true);
    document.removeEventListener("click", onClick, true);
    document.removeEventListener("keydown", onKey, true);
    document.documentElement.style.cursor = "";
    state.overlay?.remove();
    state.badge?.remove();
    state.overlay = null;
    state.badge = null;
  }};
  const onMove = (event) => {{
    const element = event.target instanceof Element ? event.target : null;
    if (!element || element === state.overlay || element === state.badge) return;
    state.hovered = element;
    draw(element);
  }};
  const onKey = (event) => {{
    if (event.key === "Escape") {{
      event.preventDefault();
      event.stopPropagation();
      cleanup();
    }}
  }};
  const onClick = (event) => {{
    if (!state.active) return;
    event.preventDefault();
    event.stopImmediatePropagation();
    const element = event.target instanceof Element ? event.target : state.hovered;
    if (!element) return cleanup();
    const rect = element.getBoundingClientRect();
    const interesting = new Set(["aria-label", "data-testid", "href", "name", "placeholder", "role", "type"]);
    const attributes = Array.from(element.attributes)
      .filter((attribute) => interesting.has(attribute.name) || attribute.name.startsWith("data-"))
      .slice(0, 12)
      .map((attribute) => ({{ name: bounded(attribute.name, 80), value: bounded(attribute.value, 240) }}));
    const annotation = {{
      url: bounded(location.href, 2048),
      title: bounded(document.title, 300),
      selector: bounded(selectorFor(element), 1200),
      tagName: bounded(element.tagName.toLowerCase(), 80),
      elementId: element.id ? bounded(element.id, 240) : null,
      role: element.getAttribute("role") ? bounded(element.getAttribute("role"), 120) : null,
      accessibleName: element.getAttribute("aria-label")
        ? bounded(element.getAttribute("aria-label"), 300)
        : null,
      text: bounded(element.textContent, 800),
      attributes,
      rect: {{
        x: Math.round(rect.x * 10) / 10,
        y: Math.round(rect.y * 10) / 10,
        width: Math.round(rect.width * 10) / 10,
        height: Math.round(rect.height * 10) / 10
      }}
    }};
    cleanup();
    location.href = "forge-preview-annotation://pick?payload=" +
      encodeURIComponent(JSON.stringify(annotation)) + "&source=" + encodeURIComponent(label);
  }};
  window.__forgePreview = {{
    startPicker() {{
      cleanup();
      state.active = true;
      document.documentElement.style.cursor = "crosshair";
      document.addEventListener("mousemove", onMove, true);
      document.addEventListener("click", onClick, true);
      document.addEventListener("keydown", onKey, true);
    }},
    cancelPicker: cleanup
  }};
}})();
"##
    )
}

fn webview(app: &AppHandle, label: &str) -> Result<tauri::Webview, String> {
    validate_label(label)?;
    app.get_webview(label)
        .ok_or_else(|| "preview is not open".to_string())
}

#[tauri::command(async)]
pub async fn preview_open(
    app: AppHandle,
    label: String,
    url: String,
    bounds: PreviewBounds,
) -> Result<PreviewState, String> {
    validate_label(&label)?;
    let url = parse_http_url(&url)?;
    let bounds = validate_bounds(bounds)?;

    if let Some(existing) = app.get_webview(&label) {
        existing
            .set_bounds(tauri::Rect {
                position: LogicalPosition::new(bounds.x, bounds.y).into(),
                size: LogicalSize::new(bounds.width, bounds.height).into(),
            })
            .map_err(|error| error.to_string())?;
        let current = existing.url().map_err(|error| error.to_string())?;
        if current != url {
            existing
                .navigate(url.clone())
                .map_err(|error| error.to_string())?;
        }
        existing.show().map_err(|error| error.to_string())?;
        return Ok(PreviewState {
            label,
            url: url.to_string(),
            loaded: false,
        });
    }

    let window = app
        .get_window(MAIN_WINDOW)
        .ok_or_else(|| "main window is unavailable".to_string())?;
    let state_app = app.clone();
    let state_label = label.clone();
    let navigation_app = app.clone();
    let navigation_label = label.clone();
    let builder = WebviewBuilder::new(&label, WebviewUrl::External(url.clone()))
        .initialization_script(picker_script(&label))
        .on_page_load(move |_webview, payload| {
            let _ = state_app.emit_to(
                MAIN_WINDOW,
                STATE_EVENT,
                PreviewState {
                    label: state_label.clone(),
                    url: payload.url().to_string(),
                    loaded: payload.event() == PageLoadEvent::Finished,
                },
            );
        })
        .on_navigation(move |candidate| {
            if candidate.scheme() == ANNOTATION_SCHEME {
                if let Some(annotation) = annotation_from_navigation(candidate) {
                    let _ = navigation_app.emit_to(
                        MAIN_WINDOW,
                        ANNOTATION_EVENT,
                        PreviewAnnotationEvent {
                            label: navigation_label.clone(),
                            annotation,
                        },
                    );
                }
                return false;
            }
            matches!(candidate.scheme(), "http" | "https")
        })
        .zoom_hotkeys_enabled(false);

    window
        .add_child(
            builder,
            LogicalPosition::new(bounds.x, bounds.y),
            LogicalSize::new(bounds.width, bounds.height),
        )
        .map_err(|error| error.to_string())?;

    Ok(PreviewState {
        label,
        url: url.to_string(),
        loaded: false,
    })
}

#[tauri::command]
pub fn preview_set_bounds(
    app: AppHandle,
    label: String,
    bounds: PreviewBounds,
) -> Result<(), String> {
    let bounds = validate_bounds(bounds)?;
    webview(&app, &label)?
        .set_bounds(tauri::Rect {
            position: LogicalPosition::new(bounds.x, bounds.y).into(),
            size: LogicalSize::new(bounds.width, bounds.height).into(),
        })
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn preview_navigate(app: AppHandle, label: String, url: String) -> Result<(), String> {
    let url = parse_http_url(&url)?;
    webview(&app, &label)?
        .navigate(url)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn preview_history(app: AppHandle, label: String, direction: i8) -> Result<(), String> {
    let script = match direction {
        -1 => "history.back()",
        1 => "history.forward()",
        _ => return Err("history direction must be -1 or 1".to_string()),
    };
    webview(&app, &label)?
        .eval(script)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn preview_reload(app: AppHandle, label: String) -> Result<(), String> {
    webview(&app, &label)?
        .reload()
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn preview_set_zoom(app: AppHandle, label: String, zoom: f64) -> Result<(), String> {
    if !zoom.is_finite() || !(0.25..=3.0).contains(&zoom) {
        return Err("preview zoom must be between 25% and 300%".to_string());
    }
    webview(&app, &label)?
        .set_zoom(zoom)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn preview_set_picker(app: AppHandle, label: String, active: bool) -> Result<(), String> {
    let script = if active {
        "window.__forgePreview?.startPicker()"
    } else {
        "window.__forgePreview?.cancelPicker()"
    };
    webview(&app, &label)?
        .eval(script)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn preview_hide(app: AppHandle, label: String) -> Result<(), String> {
    webview(&app, &label)?
        .hide()
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn preview_close(app: AppHandle, label: String) -> Result<(), String> {
    webview(&app, &label)?
        .close()
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn annotation() -> PreviewElementAnnotation {
        PreviewElementAnnotation {
            url: "http://localhost:5173/".to_string(),
            title: "Forge".to_string(),
            selector: "main > button:nth-of-type(2)".to_string(),
            tag_name: "button".to_string(),
            element_id: None,
            role: Some("button".to_string()),
            accessible_name: Some("Save".to_string()),
            text: "Save".to_string(),
            attributes: vec![PreviewElementAttribute {
                name: "data-testid".to_string(),
                value: "save".to_string(),
            }],
            rect: PreviewElementRect {
                x: 10.0,
                y: 20.0,
                width: 90.0,
                height: 32.0,
            },
        }
    }

    #[test]
    fn only_preview_labels_are_accepted() {
        assert!(validate_label("preview-session_01").is_ok());
        assert!(validate_label("main").is_err());
        assert!(validate_label("preview-../main").is_err());
    }

    #[test]
    fn only_http_preview_urls_are_accepted() {
        assert!(parse_http_url("http://localhost:5173").is_ok());
        assert!(parse_http_url("https://example.com").is_ok());
        assert!(parse_http_url("file:///etc/passwd").is_err());
        assert!(parse_http_url("javascript:alert(1)").is_err());
    }

    #[test]
    fn annotation_navigation_decodes_bounded_json() {
        let expected = annotation();
        let encoded = serde_json::to_string(&expected).unwrap();
        let url = tauri::Url::parse_with_params(
            "forge-preview-annotation://pick",
            &[("payload", encoded)],
        )
        .unwrap();
        assert_eq!(annotation_from_navigation(&url), Some(expected));
    }

    #[test]
    fn non_annotation_navigation_is_ignored() {
        let url = "https://example.com/".parse().unwrap();
        assert_eq!(annotation_from_navigation(&url), None);
    }

    #[test]
    fn desktop_capability_does_not_authorize_every_child_webview() {
        let capability: serde_json::Value =
            serde_json::from_str(include_str!("../capabilities/default.json")).unwrap();
        assert_eq!(capability["webviews"], serde_json::json!(["main"]));
        assert!(capability.get("windows").is_none());
    }
}
