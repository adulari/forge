//! The capture: one JSON line per flow, and the queries that make it useful.

use anyhow::Result;
use serde::{Deserialize, Serialize};

/// One request/response pair the proxy saw.
///
/// Header maps are kept whole rather than summarised. The interesting header in a reversing
/// session is almost never one you predicted — an undocumented `x-`, a signature, a device id —
/// so filtering them down at capture time would throw away the answer before the question.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Flow {
    pub id: String,
    #[serde(default)]
    pub at: f64,
    pub method: String,
    pub url: String,
    #[serde(default)]
    pub host: String,
    #[serde(default)]
    pub status: Option<u16>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub request_headers: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    pub request_body: String,
    #[serde(default)]
    pub request_body_bytes: usize,
    #[serde(default)]
    pub request_body_clipped: bool,
    #[serde(default)]
    pub response_headers: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    pub response_body: String,
    #[serde(default)]
    pub response_body_bytes: usize,
    #[serde(default)]
    pub response_body_clipped: bool,
    #[serde(default)]
    pub blocked: bool,
}

impl Flow {
    /// One line for a listing: enough to pick the flow you want out of hundreds without opening
    /// any of them.
    pub fn summary(&self) -> String {
        let status = match (&self.error, self.status) {
            (Some(error), _) => format!("ERR {error}"),
            (None, Some(code)) => code.to_string(),
            (None, None) => "—".to_string(),
        };
        let blocked = if self.blocked { " [blocked]" } else { "" };
        let body = if self.request_body_bytes > 0 {
            format!(" ↑{}B", self.request_body_bytes)
        } else {
            String::new()
        };
        format!(
            "{}  {status:>3}  {} {}{body}{blocked}",
            &self.id[..self.id.len().min(8)],
            self.method,
            self.url
        )
    }
}

/// Which flows to return. Every field is ANDed; an empty filter matches everything.
#[derive(Debug, Clone, Default)]
pub struct Filter {
    pub url_contains: Option<String>,
    pub method: Option<String>,
    pub status: Option<u16>,
    pub host: Option<String>,
    /// Only flows that carried a request body — the fast path to "what did the app POST", which
    /// is the question a reversing session usually starts from.
    pub with_body: bool,
    /// Only flows that failed or were blocked.
    pub failed: bool,
}

impl Filter {
    pub fn matches(&self, flow: &Flow) -> bool {
        if let Some(needle) = &self.url_contains {
            if !flow.url.to_lowercase().contains(&needle.to_lowercase()) {
                return false;
            }
        }
        if let Some(method) = &self.method {
            if !flow.method.eq_ignore_ascii_case(method) {
                return false;
            }
        }
        if let Some(status) = self.status {
            if flow.status != Some(status) {
                return false;
            }
        }
        if let Some(host) = &self.host {
            if !flow.host.to_lowercase().contains(&host.to_lowercase()) {
                return false;
            }
        }
        if self.with_body && flow.request_body_bytes == 0 {
            return false;
        }
        if self.failed && flow.error.is_none() && !flow.blocked {
            return false;
        }
        true
    }
}

/// Read the capture file, skipping any line that does not parse.
///
/// A partial last line is normal, not an error: the file is appended to by a live process, so a
/// read can land mid-write. Failing the whole query for that would make the tool flaky exactly
/// when traffic is busiest.
pub fn read_capture(path: &std::path::Path) -> Result<Vec<Flow>> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Ok(Vec::new()); // nothing captured yet is not a failure
    };
    Ok(text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| serde_json::from_str::<Flow>(line).ok())
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flow(method: &str, url: &str, status: Option<u16>, body: usize) -> Flow {
        Flow {
            id: "abcdef1234".into(),
            at: 0.0,
            method: method.into(),
            url: url.into(),
            host: url.split('/').nth(2).unwrap_or_default().into(),
            status,
            error: None,
            request_headers: Default::default(),
            request_body: String::new(),
            request_body_bytes: body,
            request_body_clipped: false,
            response_headers: Default::default(),
            response_body: String::new(),
            response_body_bytes: 0,
            response_body_clipped: false,
            blocked: false,
        }
    }

    #[test]
    fn filters_and_all_of_them_at_once() {
        let f = flow("POST", "https://api.example.com/v1/login", Some(200), 42);
        assert!(Filter::default().matches(&f));
        assert!(Filter {
            url_contains: Some("LOGIN".into()),
            method: Some("post".into()),
            status: Some(200),
            with_body: true,
            ..Default::default()
        }
        .matches(&f));
        assert!(!Filter {
            status: Some(404),
            ..Default::default()
        }
        .matches(&f));
        assert!(
            !Filter {
                with_body: true,
                ..Default::default()
            }
            .matches(&flow("GET", "https://api.example.com/v1/me", Some(200), 0)),
            "with_body must exclude a GET that carried nothing"
        );
    }

    /// A capture file is appended to by a LIVE process, so a read can land mid-write. Dropping the
    /// partial line and keeping the rest is the difference between a tool that works under load
    /// and one that fails exactly when traffic is busiest.
    #[test]
    fn a_half_written_last_line_does_not_lose_the_whole_capture() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("capture.jsonl");
        let good = serde_json::to_string(&flow("GET", "https://a.test/1", Some(200), 0)).unwrap();
        std::fs::write(&path, format!("{good}\n{good}\n{{\"id\":\"trunc")).unwrap();

        let flows = read_capture(&path).unwrap();
        assert_eq!(flows.len(), 2, "the two complete lines survive");
    }

    #[test]
    fn a_capture_that_does_not_exist_yet_is_empty_not_an_error() {
        let flows = read_capture(std::path::Path::new("/nonexistent/forge/capture.jsonl")).unwrap();
        assert!(flows.is_empty());
    }

    #[test]
    fn a_summary_shows_the_failure_rather_than_a_blank_status() {
        let mut f = flow("GET", "https://pinned.example/api", None, 0);
        f.error = Some("connection refused".into());
        assert!(
            f.summary().contains("ERR connection refused"),
            "{}",
            f.summary()
        );
    }
}
