//! The DevTools Network tab, as data.
//!
//! This is the half of the feature that page automation cannot replace. Hooking `window.fetch` and
//! `XMLHttpRequest` from injected JS — the usual approach, and the one the ss-ultimate project was
//! using — sees only what page script chose to route through those two APIs. It misses document
//! and subresource loads, redirects and the `Location` chain, CORS preflights, `sendBeacon`,
//! EventSource, WebSocket upgrades, service-worker traffic, and anything issued before the hook
//! installed. For reverse-engineering a login flow, the requests it misses are usually the ones
//! that matter.
//!
//! CDP's `Network` domain is the same source DevTools itself renders, so what lands here is what
//! the Network tab would show.

use std::collections::VecDeque;

use serde::Serialize;
use serde_json::Value;

/// Exchanges retained. Roughly a heavy single-page app's first few minutes; old entries are
/// dropped oldest-first so a long session cannot grow without bound.
pub const DEFAULT_CAPACITY: usize = 2_000;

/// One request/response pair, in the shape the Network tab shows.
#[derive(Debug, Clone, Serialize, PartialEq, Default)]
pub struct Exchange {
    /// CDP request id — the handle for fetching a body later.
    pub id: String,
    pub method: String,
    pub url: String,
    /// `Document`, `XHR`, `Fetch`, `Script`, `Preflight`, ... as CDP reports it.
    pub resource_type: Option<String>,
    pub status: Option<u16>,
    pub status_text: Option<String>,
    pub mime_type: Option<String>,
    pub request_headers: Vec<(String, String)>,
    pub response_headers: Vec<(String, String)>,
    /// Request body, when the page sent one. The field that matters for a login POST.
    pub post_data: Option<String>,
    pub remote_address: Option<String>,
    /// Wall-clock milliseconds from request to completion, when both were observed.
    pub duration_ms: Option<f64>,
    pub encoded_bytes: Option<f64>,
    /// Set when the request failed or was blocked; `None` on success.
    pub failure: Option<String>,
    /// True once the response completed — a body can only be fetched after this.
    pub finished: bool,
    /// CDP monotonic timestamp of the request, in seconds. Public so callers can order or
    /// construct exchanges; `duration_ms` is the derived figure worth reading.
    pub started_at: f64,
}

impl Exchange {
    /// The `Location` a redirect pointed at, if this exchange was one.
    pub fn redirect_target(&self) -> Option<&str> {
        if !matches!(self.status, Some(300..=399)) {
            return None;
        }
        self.response_headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("location"))
            .map(|(_, value)| value.as_str())
    }
}

/// What to return from a query. Every field is optional; an empty filter means "everything".
#[derive(Debug, Clone, Default)]
pub struct Filter {
    /// Case-insensitive substring of the URL.
    pub url_contains: Option<String>,
    pub method: Option<String>,
    pub resource_type: Option<String>,
    /// Inclusive status range, e.g. `(400, 599)` for failures.
    pub status_range: Option<(u16, u16)>,
    /// Only exchanges that carried a request body — the fast path to "what did the form POST".
    pub with_post_data: bool,
    pub limit: Option<usize>,
}

impl Filter {
    fn matches(&self, exchange: &Exchange) -> bool {
        if let Some(needle) = &self.url_contains {
            if !exchange.url.to_lowercase().contains(&needle.to_lowercase()) {
                return false;
            }
        }
        if let Some(method) = &self.method {
            if !exchange.method.eq_ignore_ascii_case(method) {
                return false;
            }
        }
        if let Some(kind) = &self.resource_type {
            match &exchange.resource_type {
                Some(actual) if actual.eq_ignore_ascii_case(kind) => {}
                _ => return false,
            }
        }
        if let Some((low, high)) = self.status_range {
            match exchange.status {
                Some(status) if status >= low && status <= high => {}
                _ => return false,
            }
        }
        if self.with_post_data && exchange.post_data.is_none() {
            return false;
        }
        true
    }
}

/// A bounded, ordered capture of everything the browser fetched.
#[derive(Debug)]
pub struct NetworkLog {
    exchanges: VecDeque<Exchange>,
    capacity: usize,
}

impl Default for NetworkLog {
    fn default() -> Self {
        Self::with_capacity(DEFAULT_CAPACITY)
    }
}

impl NetworkLog {
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            exchanges: VecDeque::new(),
            capacity: capacity.max(1),
        }
    }

    pub fn len(&self) -> usize {
        self.exchanges.len()
    }

    pub fn is_empty(&self) -> bool {
        self.exchanges.is_empty()
    }

    pub fn clear(&mut self) {
        self.exchanges.clear();
    }

    /// Fold one CDP `Network.*` event into the log. Unknown methods are ignored.
    pub fn apply(&mut self, method: &str, params: &Value) {
        match method {
            "Network.requestWillBeSent" => self.on_request(params),
            "Network.responseReceived" => self.on_response(params),
            "Network.loadingFinished" => self.on_finished(params),
            "Network.loadingFailed" => self.on_failed(params),
            _ => {}
        }
    }

    fn on_request(&mut self, params: &Value) {
        let Some(id) = params.get("requestId").and_then(Value::as_str) else {
            return;
        };
        let request = params.get("request").unwrap_or(&Value::Null);

        // A redirect arrives as a NEW requestWillBeSent carrying the PREVIOUS response in
        // `redirectResponse`, reusing the same requestId. Without this the redirect hop is lost
        // and the chain looks like one request that mysteriously landed somewhere else — exactly
        // the detail an auth flow turns on.
        if let Some(redirect) = params.get("redirectResponse") {
            if !redirect.is_null() {
                if let Some(existing) = self.exchanges.iter_mut().find(|e| e.id == id) {
                    apply_response_fields(existing, redirect);
                    existing.finished = true;
                    let hop = existing.clone();
                    // Keep the completed hop under a distinguishable id so both survive.
                    if let Some(slot) = self.exchanges.iter_mut().find(|e| e.id == id) {
                        slot.id = format!("{id}:redirect{}", hop.status.unwrap_or(0));
                    }
                }
            }
        }

        let exchange = Exchange {
            id: id.to_string(),
            method: request
                .get("method")
                .and_then(Value::as_str)
                .unwrap_or("GET")
                .to_string(),
            url: request
                .get("url")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            resource_type: params
                .get("type")
                .and_then(Value::as_str)
                .map(str::to_string),
            status: None,
            status_text: None,
            mime_type: None,
            request_headers: headers(request.get("headers")),
            response_headers: Vec::new(),
            post_data: request
                .get("postData")
                .and_then(Value::as_str)
                .map(str::to_string),
            remote_address: None,
            duration_ms: None,
            encoded_bytes: None,
            failure: None,
            finished: false,
            started_at: params
                .get("timestamp")
                .and_then(Value::as_f64)
                .unwrap_or_default(),
        };
        self.push(exchange);
    }

    fn on_response(&mut self, params: &Value) {
        let Some(id) = params.get("requestId").and_then(Value::as_str) else {
            return;
        };
        let response = params.get("response").unwrap_or(&Value::Null);
        if let Some(exchange) = self.exchanges.iter_mut().find(|e| e.id == id) {
            apply_response_fields(exchange, response);
        }
    }

    fn on_finished(&mut self, params: &Value) {
        let Some(id) = params.get("requestId").and_then(Value::as_str) else {
            return;
        };
        let timestamp = params.get("timestamp").and_then(Value::as_f64);
        let bytes = params.get("encodedDataLength").and_then(Value::as_f64);
        if let Some(exchange) = self.exchanges.iter_mut().find(|e| e.id == id) {
            exchange.finished = true;
            exchange.encoded_bytes = bytes;
            if let Some(end) = timestamp {
                exchange.duration_ms = Some((end - exchange.started_at) * 1000.0);
            }
        }
    }

    fn on_failed(&mut self, params: &Value) {
        let Some(id) = params.get("requestId").and_then(Value::as_str) else {
            return;
        };
        let canceled = params
            .get("canceled")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let text = params
            .get("errorText")
            .and_then(Value::as_str)
            .unwrap_or("request failed");
        if let Some(exchange) = self.exchanges.iter_mut().find(|e| e.id == id) {
            exchange.finished = true;
            exchange.failure = Some(if canceled {
                format!("{text} (canceled)")
            } else {
                text.to_string()
            });
        }
    }

    fn push(&mut self, exchange: Exchange) {
        if self.exchanges.len() >= self.capacity {
            self.exchanges.pop_front();
        }
        self.exchanges.push_back(exchange);
    }

    /// Matching exchanges, newest last. `limit` keeps the NEWEST matches, because the question is
    /// almost always "what just happened".
    pub fn query(&self, filter: &Filter) -> Vec<&Exchange> {
        let mut hits: Vec<&Exchange> = self
            .exchanges
            .iter()
            .filter(|exchange| filter.matches(exchange))
            .collect();
        if let Some(limit) = filter.limit {
            if hits.len() > limit {
                hits.drain(..hits.len() - limit);
            }
        }
        hits
    }

    pub fn get(&self, id: &str) -> Option<&Exchange> {
        self.exchanges.iter().find(|exchange| exchange.id == id)
    }
}

fn apply_response_fields(exchange: &mut Exchange, response: &Value) {
    exchange.status = response
        .get("status")
        .and_then(Value::as_u64)
        .map(|status| status as u16);
    exchange.status_text = response
        .get("statusText")
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .map(str::to_string);
    exchange.mime_type = response
        .get("mimeType")
        .and_then(Value::as_str)
        .map(str::to_string);
    exchange.remote_address = response
        .get("remoteIPAddress")
        .and_then(Value::as_str)
        .map(str::to_string);
    exchange.response_headers = headers(response.get("headers"));
}

/// CDP delivers headers as a flat object. Values are stringified rather than dropped: a numeric
/// `Content-Length` is a number in JSON and would otherwise vanish.
fn headers(value: Option<&Value>) -> Vec<(String, String)> {
    let Some(Value::Object(map)) = value else {
        return Vec::new();
    };
    map.iter()
        .map(|(name, value)| {
            let text = match value {
                Value::String(text) => text.clone(),
                other => other.to_string(),
            };
            (name.clone(), text)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sent(id: &str, method: &str, url: &str) -> Value {
        json!({
            "requestId": id,
            "timestamp": 1000.0,
            "type": "XHR",
            "request": {
                "method": method,
                "url": url,
                "headers": {"Accept": "*/*", "Content-Length": 42},
                "postData": "username=a&password=b"
            }
        })
    }

    fn received(id: &str, status: u16) -> Value {
        json!({
            "requestId": id,
            "response": {
                "status": status,
                "statusText": "OK",
                "mimeType": "application/json",
                "remoteIPAddress": "104.18.0.1",
                "headers": {"content-type": "application/json", "set-cookie": "sp_dc=abc"}
            }
        })
    }

    #[test]
    fn a_request_and_its_response_become_one_exchange() {
        let mut log = NetworkLog::default();
        log.apply(
            "Network.requestWillBeSent",
            &sent("R1", "POST", "https://accounts.spotify.com/login/password"),
        );
        log.apply("Network.responseReceived", &received("R1", 200));
        log.apply(
            "Network.loadingFinished",
            &json!({"requestId": "R1", "timestamp": 1002.5, "encodedDataLength": 812.0}),
        );

        let all = log.query(&Filter::default());
        assert_eq!(all.len(), 1);
        let exchange = all[0];
        assert_eq!(exchange.method, "POST");
        assert_eq!(exchange.status, Some(200));
        assert_eq!(exchange.post_data.as_deref(), Some("username=a&password=b"));
        assert!(exchange.finished);
        assert_eq!(exchange.duration_ms, Some(2500.0));
        assert!(
            exchange
                .response_headers
                .iter()
                .any(|(k, v)| k == "set-cookie" && v == "sp_dc=abc"),
            "{:?}",
            exchange.response_headers
        );
    }

    #[test]
    fn a_numeric_header_survives_instead_of_vanishing() {
        // `Content-Length` arrives as a JSON number. Matching only on `Value::String` silently
        // drops it, and a missing Content-Length changes how a captured request replays.
        let mut log = NetworkLog::default();
        log.apply(
            "Network.requestWillBeSent",
            &sent("R1", "POST", "https://example.test/x"),
        );
        let exchange = log.get("R1").expect("captured");
        assert!(
            exchange
                .request_headers
                .iter()
                .any(|(k, v)| k == "Content-Length" && v == "42"),
            "{:?}",
            exchange.request_headers
        );
    }

    #[test]
    fn a_redirect_hop_is_kept_rather_than_overwritten() {
        // Chrome reuses ONE requestId across a redirect chain, delivering the previous response
        // inside the next requestWillBeSent. Overwriting in place loses the 302 and its Location —
        // which in an auth flow is the entire thing being reverse-engineered.
        let mut log = NetworkLog::default();
        log.apply(
            "Network.requestWillBeSent",
            &sent("R1", "GET", "https://accounts.example.test/authorize"),
        );
        log.apply(
            "Network.requestWillBeSent",
            &json!({
                "requestId": "R1",
                "timestamp": 1001.0,
                "type": "Document",
                "request": {"method": "GET", "url": "https://app.example.test/callback?code=XYZ",
                           "headers": {}},
                "redirectResponse": {
                    "status": 302,
                    "statusText": "Found",
                    "headers": {"location": "https://app.example.test/callback?code=XYZ"}
                }
            }),
        );

        let all = log.query(&Filter::default());
        assert_eq!(all.len(), 2, "both hops must survive: {all:?}");
        let hop = all
            .iter()
            .find(|e| e.status == Some(302))
            .expect("the redirect hop is retained");
        assert_eq!(
            hop.redirect_target(),
            Some("https://app.example.test/callback?code=XYZ")
        );
        assert!(all.iter().any(|e| e.url.contains("code=XYZ")));
    }

    #[test]
    fn a_failed_request_says_why_and_distinguishes_a_cancel() {
        let mut log = NetworkLog::default();
        log.apply(
            "Network.requestWillBeSent",
            &sent("R1", "GET", "https://blocked.test/x"),
        );
        log.apply(
            "Network.loadingFailed",
            &json!({"requestId": "R1", "errorText": "net::ERR_BLOCKED_BY_CLIENT", "canceled": true}),
        );
        let exchange = log.get("R1").expect("captured");
        let failure = exchange.failure.as_deref().expect("a failure is recorded");
        assert!(failure.contains("ERR_BLOCKED_BY_CLIENT"), "{failure}");
        assert!(failure.contains("canceled"), "{failure}");
    }

    #[test]
    fn filters_narrow_to_the_request_being_hunted() {
        let mut log = NetworkLog::default();
        log.apply(
            "Network.requestWillBeSent",
            &sent("R1", "GET", "https://cdn.test/app.js"),
        );
        log.apply("Network.responseReceived", &received("R1", 200));
        log.apply(
            "Network.requestWillBeSent",
            &sent("R2", "POST", "https://accounts.test/login/password"),
        );
        log.apply("Network.responseReceived", &received("R2", 401));

        let by_url = log.query(&Filter {
            url_contains: Some("LOGIN".into()),
            ..Filter::default()
        });
        assert_eq!(by_url.len(), 1, "URL match is case-insensitive");
        assert_eq!(by_url[0].id, "R2");

        let failures = log.query(&Filter {
            status_range: Some((400, 599)),
            ..Filter::default()
        });
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].status, Some(401));

        let posts = log.query(&Filter {
            method: Some("post".into()),
            ..Filter::default()
        });
        assert_eq!(posts.len(), 1, "method match is case-insensitive");
    }

    #[test]
    fn a_limit_keeps_the_newest_matches_not_the_oldest() {
        // "What just happened" is the question being asked; truncating from the front would answer
        // "what happened first", which on a busy page is a page-load asset nobody asked about.
        let mut log = NetworkLog::default();
        for n in 0..5 {
            log.apply(
                "Network.requestWillBeSent",
                &sent(&format!("R{n}"), "GET", &format!("https://test/{n}")),
            );
        }
        let recent = log.query(&Filter {
            limit: Some(2),
            ..Filter::default()
        });
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].id, "R3");
        assert_eq!(recent[1].id, "R4");
    }

    #[test]
    fn the_log_is_bounded_and_drops_oldest_first() {
        let mut log = NetworkLog::with_capacity(3);
        for n in 0..5 {
            log.apply(
                "Network.requestWillBeSent",
                &sent(&format!("R{n}"), "GET", &format!("https://test/{n}")),
            );
        }
        assert_eq!(log.len(), 3);
        assert!(log.get("R0").is_none(), "oldest is evicted");
        assert!(log.get("R4").is_some(), "newest is retained");
    }

    #[test]
    fn an_event_for_an_unknown_request_is_ignored_rather_than_inventing_a_row() {
        let mut log = NetworkLog::default();
        log.apply("Network.responseReceived", &received("ghost", 200));
        log.apply("Network.loadingFinished", &json!({"requestId": "ghost"}));
        assert!(log.is_empty());
    }
}
