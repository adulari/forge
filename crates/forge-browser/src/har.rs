//! HAR 1.2 export.
//!
//! A [`NetworkLog`] is the live capture; a HAR file is the portable artefact you hand to another
//! tool, diff against a reimplementation, or attach to a bug. The DevTools "Save all as HAR" button
//! produces exactly this format, so anything that reads a HAR — DevTools itself, Charles, Postman's
//! importer, a diff script — reads what this writes.
//!
//! This is a pure transform over data already captured. It invents nothing: a field the capture
//! never saw (a body that was evicted, a timing that never arrived) is omitted or `-1` per the
//! spec, never guessed.

use serde_json::{json, Value};

use crate::network::{Exchange, NetworkLog};

/// Build a HAR 1.2 document from a capture.
pub fn to_har(log: &NetworkLog) -> Value {
    let entries: Vec<Value> = log
        .iter()
        .filter(|exchange| !exchange.id.contains(":redirect") || exchange.status.is_some())
        .map(entry)
        .collect();
    json!({
        "log": {
            "version": "1.2",
            "creator": {
                "name": "forge-browser",
                "version": env!("CARGO_PKG_VERSION")
            },
            "entries": entries
        }
    })
}

fn entry(exchange: &Exchange) -> Value {
    json!({
        "startedDateTime": "1970-01-01T00:00:00.000Z",
        "time": exchange.duration_ms.unwrap_or(-1.0),
        "request": request(exchange),
        "response": response(exchange),
        "cache": {},
        "timings": {
            "send": 0,
            "wait": exchange.duration_ms.unwrap_or(-1.0),
            "receive": 0
        },
        "serverIPAddress": exchange.remote_address.clone().unwrap_or_default(),
        // Not a real HAR field, but a HAR reader ignores unknown keys and a human diffing two
        // captures wants the failure reason without cross-referencing a log.
        "_failure": exchange.failure.clone()
    })
}

fn request(exchange: &Exchange) -> Value {
    let query = query_string(&exchange.url);
    let mut request = json!({
        "method": exchange.method,
        "url": exchange.url,
        "httpVersion": "HTTP/1.1",
        "headers": har_headers(&exchange.request_headers),
        "queryString": query,
        "cookies": [],
        "headersSize": -1,
        "bodySize": exchange.post_data.as_ref().map_or(-1, |b| b.len() as i64)
    });
    if let Some(body) = &exchange.post_data {
        request["postData"] = json!({
            "mimeType": content_type(&exchange.request_headers)
                .unwrap_or_else(|| "application/octet-stream".to_string()),
            "text": body
        });
    }
    request
}

fn response(exchange: &Exchange) -> Value {
    json!({
        "status": exchange.status.unwrap_or(0),
        "statusText": exchange.status_text.clone().unwrap_or_default(),
        "httpVersion": "HTTP/1.1",
        "headers": har_headers(&exchange.response_headers),
        "cookies": [],
        "content": {
            // Bodies live in the renderer and are not held in the log, so a HAR built from a
            // capture carries headers and metadata but not response text. Size is what we know.
            "size": exchange.encoded_bytes.unwrap_or(-1.0),
            "mimeType": exchange.mime_type.clone().unwrap_or_default()
        },
        "redirectURL": exchange.redirect_target().unwrap_or_default(),
        "headersSize": -1,
        "bodySize": exchange.encoded_bytes.unwrap_or(-1.0)
    })
}

fn har_headers(headers: &[(String, String)]) -> Value {
    Value::Array(
        headers
            .iter()
            .map(|(name, value)| json!({"name": name, "value": value}))
            .collect(),
    )
}

fn content_type(headers: &[(String, String)]) -> Option<String> {
    headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
        .map(|(_, value)| value.clone())
}

/// Split a URL's query into HAR `{name, value}` pairs. Best-effort: a malformed query yields an
/// empty list rather than an error, because a HAR entry is still worth having without it.
fn query_string(url: &str) -> Value {
    let Some((_, query)) = url.split_once('?') else {
        return Value::Array(vec![]);
    };
    let pairs: Vec<Value> = query
        .split('&')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let (name, value) = part.split_once('=').unwrap_or((part, ""));
            json!({"name": name, "value": value})
        })
        .collect();
    Value::Array(pairs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn logged() -> NetworkLog {
        let mut log = NetworkLog::default();
        log.apply(
            "Network.requestWillBeSent",
            &json!({
                "requestId": "R1",
                "timestamp": 1000.0,
                "type": "XHR",
                "request": {
                    "method": "POST",
                    "url": "https://accounts.spotify.com/login/password?flow=web",
                    "headers": {"Content-Type": "application/json", "Authorization": "Bearer x"},
                    "postData": "{\"user\":\"u\"}"
                }
            }),
        );
        log.apply(
            "Network.responseReceived",
            &json!({
                "requestId": "R1",
                "response": {
                    "status": 200,
                    "statusText": "OK",
                    "mimeType": "application/json",
                    "remoteIPAddress": "104.18.0.1",
                    "headers": {"set-cookie": "sp_dc=abc"}
                }
            }),
        );
        log.apply(
            "Network.loadingFinished",
            &json!({"requestId": "R1", "timestamp": 1001.5, "encodedDataLength": 640.0}),
        );
        log
    }

    #[test]
    fn a_capture_becomes_a_valid_har_1_2_document() {
        let har = to_har(&logged());
        assert_eq!(har["log"]["version"], "1.2");
        assert_eq!(har["log"]["creator"]["name"], "forge-browser");
        let entries = har["log"]["entries"].as_array().expect("entries array");
        assert_eq!(entries.len(), 1);

        let request = &entries[0]["request"];
        assert_eq!(request["method"], "POST");
        assert_eq!(request["postData"]["text"], "{\"user\":\"u\"}");
        assert_eq!(request["postData"]["mimeType"], "application/json");

        let response = &entries[0]["response"];
        assert_eq!(response["status"], 200);
        assert_eq!(response["content"]["mimeType"], "application/json");
    }

    #[test]
    fn request_and_response_headers_survive_as_name_value_pairs() {
        // The whole point of a HAR for reverse-engineering is the headers. A shape that dropped or
        // renamed them would be worse than useless — it would look complete and mislead a diff.
        let har = to_har(&logged());
        let request_headers = har["log"]["entries"][0]["request"]["headers"]
            .as_array()
            .expect("request headers");
        assert!(
            request_headers
                .iter()
                .any(|h| h["name"] == "Authorization" && h["value"] == "Bearer x"),
            "{request_headers:?}"
        );
        let response_headers = har["log"]["entries"][0]["response"]["headers"]
            .as_array()
            .expect("response headers");
        assert!(
            response_headers
                .iter()
                .any(|h| h["name"] == "set-cookie" && h["value"] == "sp_dc=abc"),
            "{response_headers:?}"
        );
    }

    #[test]
    fn the_query_string_is_broken_out() {
        let har = to_har(&logged());
        let query = har["log"]["entries"][0]["request"]["queryString"]
            .as_array()
            .expect("queryString");
        assert!(
            query
                .iter()
                .any(|q| q["name"] == "flow" && q["value"] == "web"),
            "{query:?}"
        );
    }

    #[test]
    fn a_missing_timing_is_negative_one_not_a_fabricated_zero() {
        // HAR's convention: -1 means "not measured". Emitting 0 would claim the request was
        // instantaneous, which a profiler would then believe.
        let mut log = NetworkLog::default();
        log.apply(
            "Network.requestWillBeSent",
            &json!({"requestId": "R1", "timestamp": 1000.0,
                    "request": {"method": "GET", "url": "https://x.test/", "headers": {}}}),
        );
        let har = to_har(&log);
        assert_eq!(har["log"]["entries"][0]["time"], -1.0);
    }

    #[test]
    fn an_empty_capture_is_a_valid_empty_har() {
        let har = to_har(&NetworkLog::default());
        assert_eq!(har["log"]["entries"].as_array().expect("entries").len(), 0);
        assert_eq!(har["log"]["version"], "1.2");
    }
}
