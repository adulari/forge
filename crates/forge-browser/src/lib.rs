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

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use tokio::sync::mpsc;

/// Re-exported so dependents can name the error type without taking an `anyhow` dependency
/// of their own.
pub use anyhow::Error;

pub mod cdp;
pub mod launch;
pub mod network;

pub use launch::{BrowserProcess, Display, LaunchConfig};
pub use network::{Exchange, Filter, NetworkLog};

/// Cap on a single response body handed back to the model. A 40 MB bundle would blow the context
/// window; the caller can narrow with a filter and re-ask.
pub const MAX_BODY_CHARS: usize = 200_000;

/// How long `navigate` waits for the load event before returning what it has. A page that never
/// finishes loading (long-polling, a hung third-party script) is still usable, so this is a
/// deadline rather than a failure.
const LOAD_TIMEOUT: Duration = Duration::from_secs(30);

/// A live browser: one Chrome, one page, and everything that page fetched.
pub struct BrowserSession {
    process: BrowserProcess,
    client: cdp::CdpClient,
    log: Arc<Mutex<NetworkLog>>,
    pump: tokio::task::JoinHandle<()>,
    loaded: Arc<Mutex<bool>>,
}

impl std::fmt::Debug for BrowserSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BrowserSession")
            .field("profile", &self.process.profile_dir)
            .field("port", &self.process.active.port)
            .finish_non_exhaustive()
    }
}

impl Drop for BrowserSession {
    fn drop(&mut self) {
        self.pump.abort();
    }
}

impl BrowserSession {
    /// Launch a browser and attach to its first page.
    pub async fn open(config: &LaunchConfig) -> Result<Self> {
        let process = BrowserProcess::launch(config).await?;
        Self::attach_to(process).await
    }

    /// Re-attach to a browser Forge already launched on this profile — the path that lets a user
    /// log in by hand once and have every later turn drive the session they left open.
    pub async fn reattach(profile_dir: impl Into<PathBuf>) -> Result<Self> {
        let process = BrowserProcess::attach(&profile_dir.into()).await?;
        Self::attach_to(process).await
    }

    async fn attach_to(process: BrowserProcess) -> Result<Self> {
        let ws_url = first_page_target(&process.active.http_base()).await?;
        let (events_tx, mut events_rx) = mpsc::unbounded_channel();
        let client = cdp::CdpClient::connect(&ws_url, events_tx).await?;

        let log = Arc::new(Mutex::new(NetworkLog::default()));
        let loaded = Arc::new(Mutex::new(false));
        let pump_log = Arc::clone(&log);
        let pump_loaded = Arc::clone(&loaded);
        let pump = tokio::spawn(async move {
            while let Some(event) = events_rx.recv().await {
                if event.method.starts_with("Network.") {
                    pump_log
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .apply(&event.method, &event.params);
                } else if event.method == "Page.loadEventFired" {
                    *pump_loaded
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner) = true;
                }
            }
        });

        let session = Self {
            process,
            client,
            log,
            pump,
            loaded,
        };
        // Capture must be armed BEFORE the first navigation, or the very requests the caller
        // opened the browser to see have already happened.
        session.client.call("Network.enable", json!({})).await?;
        session.client.call("Page.enable", json!({})).await?;
        session.client.call("Runtime.enable", json!({})).await.ok();
        Ok(session)
    }

    /// Leave Chrome running when this session drops.
    pub fn detach(&mut self) {
        self.process.detach();
    }

    pub fn profile_dir(&self) -> &std::path::Path {
        &self.process.profile_dir
    }

    /// Navigate and wait for the load event (or [`LOAD_TIMEOUT`]).
    pub async fn navigate(&self, url: &str) -> Result<String> {
        *self
            .loaded
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = false;
        let result = self
            .client
            .call("Page.navigate", json!({ "url": url }))
            .await?;
        if let Some(error) = result.get("errorText").and_then(Value::as_str) {
            bail!("navigation to {url} failed: {error}");
        }
        let deadline = std::time::Instant::now() + LOAD_TIMEOUT;
        while std::time::Instant::now() < deadline {
            if *self
                .loaded
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        self.current_url().await
    }

    pub async fn current_url(&self) -> Result<String> {
        let value = self.eval("location.href").await?;
        Ok(value.as_str().unwrap_or_default().to_string())
    }

    /// Evaluate JavaScript in the page and return its value.
    pub async fn eval(&self, expression: &str) -> Result<Value> {
        let result = self
            .client
            .call(
                "Runtime.evaluate",
                json!({
                    "expression": expression,
                    "returnByValue": true,
                    "awaitPromise": true,
                    // Without this, any expression touching a user-gesture-gated API fails in a
                    // way that reads as a script error rather than a permission one.
                    "userGesture": true
                }),
            )
            .await?;
        if let Some(details) = result.get("exceptionDetails") {
            let text = details
                .get("exception")
                .and_then(|e| e.get("description"))
                .and_then(Value::as_str)
                .or_else(|| details.get("text").and_then(Value::as_str))
                .unwrap_or("script threw");
            bail!("{text}");
        }
        Ok(result
            .get("result")
            .and_then(|r| r.get("value"))
            .cloned()
            .unwrap_or(Value::Null))
    }

    /// The page's rendered HTML — post-JavaScript, which is the point.
    pub async fn html(&self) -> Result<String> {
        let value = self.eval("document.documentElement.outerHTML").await?;
        Ok(value.as_str().unwrap_or_default().to_string())
    }

    /// Click the first element matching a CSS selector.
    pub async fn click(&self, selector: &str) -> Result<()> {
        let script = format!(
            "(() => {{ const el = document.querySelector({sel}); if (!el) return 'missing'; \
             el.scrollIntoView({{block:'center'}}); el.click(); return 'ok'; }})()",
            sel = json!(selector)
        );
        match self.eval(&script).await?.as_str() {
            Some("ok") => Ok(()),
            _ => bail!("no element matches selector {selector}"),
        }
    }

    /// Type into the first element matching a CSS selector.
    ///
    /// Sets the value through the native setter and then fires `input`/`change`. Assigning
    /// `el.value` alone is invisible to React and every other framework that tracks its own state,
    /// so the field looks filled and the form submits empty.
    pub async fn type_text(&self, selector: &str, text: &str) -> Result<()> {
        let script = format!(
            "(() => {{ const el = document.querySelector({sel}); if (!el) return 'missing'; \
             el.focus(); \
             const proto = el instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype; \
             const setter = Object.getOwnPropertyDescriptor(proto, 'value').set; \
             setter.call(el, {text}); \
             el.dispatchEvent(new Event('input', {{bubbles:true}})); \
             el.dispatchEvent(new Event('change', {{bubbles:true}})); \
             return 'ok'; }})()",
            sel = json!(selector),
            text = json!(text)
        );
        match self.eval(&script).await?.as_str() {
            Some("ok") => Ok(()),
            _ => bail!("no element matches selector {selector}"),
        }
    }

    /// A base64 PNG of the viewport.
    pub async fn screenshot(&self) -> Result<String> {
        let result = self
            .client
            .call("Page.captureScreenshot", json!({"format": "png"}))
            .await?;
        Ok(result
            .get("data")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string())
    }

    /// Every cookie the browser holds — the artefact `browser_cookies.json` was being produced by
    /// hand for.
    pub async fn cookies(&self) -> Result<Value> {
        let result = self.client.call("Network.getCookies", json!({})).await?;
        Ok(result
            .get("cookies")
            .cloned()
            .unwrap_or(Value::Array(vec![])))
    }

    /// Captured exchanges matching `filter`.
    pub fn network(&self, filter: &Filter) -> Vec<Exchange> {
        self.log
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .query(filter)
            .into_iter()
            .cloned()
            .collect()
    }

    pub fn network_len(&self) -> usize {
        self.log
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
    }

    pub fn clear_network(&self) {
        self.log
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
    }

    /// The response body for a captured exchange.
    ///
    /// Bodies live in the renderer, not in the log: Chrome can evict one at any time, and asking
    /// before the response finished simply has no answer yet. Both cases are reported as
    /// themselves rather than as an empty body, which would read as "the server returned nothing".
    pub async fn response_body(&self, request_id: &str) -> Result<String> {
        let known = self
            .log
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(request_id)
            .cloned();
        let Some(exchange) = known else {
            bail!("no captured request with id {request_id}");
        };
        if !exchange.finished {
            bail!("request {request_id} has not completed yet, so it has no body to read");
        }
        let result = self
            .client
            .call("Network.getResponseBody", json!({"requestId": request_id}))
            .await
            .with_context(|| {
                format!(
                    "the browser no longer holds a body for {request_id} ({}). Chrome evicts \
                     bodies as the page runs; capture and read it closer to the request.",
                    exchange.url
                )
            })?;
        let body = result
            .get("body")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let base64 = result
            .get("base64Encoded")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if base64 {
            return Ok(format!(
                "<{} bytes of binary {} — not decoded>",
                body.len(),
                exchange.mime_type.as_deref().unwrap_or("data")
            ));
        }
        Ok(truncate(body, MAX_BODY_CHARS))
    }
}

/// Truncate on a char boundary, saying so.
pub fn truncate(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let kept: String = text.chars().take(max_chars).collect();
    format!("{kept}\n… truncated at {max_chars} characters")
}

/// The WebSocket URL of a page target, creating one if the browser has none.
async fn first_page_target(http_base: &str) -> Result<String> {
    let client = reqwest::Client::new();
    let list: Value = client
        .get(format!("{http_base}/json/list"))
        .send()
        .await
        .context("list DevTools targets")?
        .json()
        .await
        .context("parse the DevTools target list")?;
    if let Some(url) = page_ws_url(&list) {
        return Ok(url);
    }
    let created: Value = client
        .put(format!("{http_base}/json/new?about:blank"))
        .send()
        .await
        .context("open a new browser tab")?
        .json()
        .await
        .context("parse the new-tab response")?;
    created
        .get("webSocketDebuggerUrl")
        .and_then(Value::as_str)
        .map(str::to_string)
        .context("the browser opened a tab with no DevTools endpoint")
}

/// Pick a page target's WS URL from `/json/list`.
///
/// Split out and tested because the list also contains service workers, extension background
/// pages, and `chrome://` internals; attaching to one of those yields a session where navigation
/// silently does nothing.
fn page_ws_url(list: &Value) -> Option<String> {
    list.as_array()?
        .iter()
        .find(|target| {
            target.get("type").and_then(Value::as_str) == Some("page")
                && target
                    .get("url")
                    .and_then(Value::as_str)
                    .is_some_and(|url| !url.starts_with("devtools://"))
        })
        .and_then(|target| target.get("webSocketDebuggerUrl").and_then(Value::as_str))
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_real_page_target_is_attached_to() {
        // Attaching to a service worker or an extension page produces a session where every
        // navigation succeeds and nothing happens.
        let list = json!([
            {"type": "service_worker", "url": "https://x.test/sw.js",
             "webSocketDebuggerUrl": "ws://127.0.0.1/sw"},
            {"type": "background_page", "url": "chrome-extension://abc/_generated",
             "webSocketDebuggerUrl": "ws://127.0.0.1/ext"},
            {"type": "page", "url": "https://accounts.spotify.com/login",
             "webSocketDebuggerUrl": "ws://127.0.0.1/page"}
        ]);
        assert_eq!(page_ws_url(&list).as_deref(), Some("ws://127.0.0.1/page"));
    }

    #[test]
    fn a_devtools_window_is_not_a_page_to_drive() {
        let list = json!([
            {"type": "page", "url": "devtools://devtools/bundled/inspector.html",
             "webSocketDebuggerUrl": "ws://127.0.0.1/inspector"}
        ]);
        assert_eq!(page_ws_url(&list), None);
    }

    #[test]
    fn no_targets_means_no_url_rather_than_a_panic() {
        assert_eq!(page_ws_url(&json!([])), None);
        assert_eq!(page_ws_url(&json!({})), None);
    }

    #[test]
    fn truncation_reports_itself_and_respects_char_boundaries() {
        assert_eq!(truncate("short", 100), "short");
        let text = "é".repeat(10);
        let cut = truncate(&text, 4);
        assert!(cut.starts_with("éééé"), "{cut}");
        assert!(cut.contains("truncated at 4 characters"), "{cut}");
    }
}
