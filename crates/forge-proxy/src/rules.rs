//! Interception rules: block, rewrite, and stub traffic in flight.
//!
//! Serialized to the JSON file the addon re-reads on every request, so a change takes effect on
//! the next request with no restart — and therefore without losing the capture so far, which
//! matters when the interesting flow is one you only get once per login.

use anyhow::Result;
use serde::{Deserialize, Serialize};

/// A rewrite scoped to the URLs containing `url_contains`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HeaderRule {
    pub url_contains: String,
    pub headers: std::collections::BTreeMap<String, String>,
}

/// A body replacement scoped to the URLs containing `url_contains`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BodyRule {
    pub url_contains: String,
    pub body: String,
}

/// A canned response served instead of the real one.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StubRule {
    pub url_contains: String,
    #[serde(default = "default_status")]
    pub status: u16,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub headers: std::collections::BTreeMap<String, String>,
}

fn default_status() -> u16 {
    200
}

/// Everything the addon knows how to do. Empty = observe only, which is the default: a proxy that
/// started out modifying traffic would make the first capture a lie about what the app normally
/// does.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InterceptRules {
    /// URL substrings whose requests are refused outright (418 + `x-forge-blocked`). Answers
    /// "what does the app do when this endpoint is unreachable" without touching the network.
    #[serde(default)]
    pub block: Vec<String>,
    #[serde(default)]
    pub set_request_headers: Vec<HeaderRule>,
    #[serde(default)]
    pub replace_request_body: Vec<BodyRule>,
    #[serde(default)]
    pub set_response_headers: Vec<HeaderRule>,
    /// Serve a canned response. The REAL response is still captured first, so the capture shows
    /// both what the server said and what the app was given.
    #[serde(default)]
    pub stub_response: Vec<StubRule>,
}

impl InterceptRules {
    pub fn is_empty(&self) -> bool {
        self.block.is_empty()
            && self.set_request_headers.is_empty()
            && self.replace_request_body.is_empty()
            && self.set_response_headers.is_empty()
            && self.stub_response.is_empty()
    }

    /// Write atomically: the addon may read this file at any moment, and a half-written rules
    /// file would parse as "no rules" and silently drop interception for those requests.
    pub fn write(&self, path: &std::path::Path) -> Result<()> {
        let json = serde_json::to_string_pretty(self)?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    /// A one-line description for status output.
    pub fn describe(&self) -> String {
        if self.is_empty() {
            return "observe only (no rules)".to_string();
        }
        let mut parts = Vec::new();
        if !self.block.is_empty() {
            parts.push(format!("block {}", self.block.len()));
        }
        if !self.set_request_headers.is_empty() {
            parts.push(format!("req-headers {}", self.set_request_headers.len()));
        }
        if !self.replace_request_body.is_empty() {
            parts.push(format!("req-body {}", self.replace_request_body.len()));
        }
        if !self.set_response_headers.is_empty() {
            parts.push(format!("resp-headers {}", self.set_response_headers.len()));
        }
        if !self.stub_response.is_empty() {
            parts.push(format!("stub {}", self.stub_response.len()));
        }
        parts.join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_proxy_observes_and_does_not_modify() {
        let rules = InterceptRules::default();
        assert!(rules.is_empty());
        assert_eq!(rules.describe(), "observe only (no rules)");
    }

    /// The addon may read this file at any instant. A partial write parses as "no rules", which
    /// would silently stop intercepting — the worst failure mode here, because the traffic still
    /// flows and nothing looks wrong.
    #[test]
    fn rules_are_written_atomically_and_read_back_whole() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rules.json");
        let rules = InterceptRules {
            block: vec!["/telemetry".into()],
            stub_response: vec![StubRule {
                url_contains: "/v1/me".into(),
                status: 402,
                body: "{\"plan\":\"free\"}".into(),
                headers: Default::default(),
            }],
            ..Default::default()
        };
        rules.write(&path).unwrap();

        assert!(
            !dir.path().join("rules.json.tmp").exists(),
            "the temp file must be renamed away, not left beside the real one"
        );
        let back: InterceptRules =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(back.block, vec!["/telemetry".to_string()]);
        assert_eq!(back.stub_response[0].status, 402);
        assert!(back.describe().contains("block 1"));
        assert!(back.describe().contains("stub 1"));
    }

    /// The addon reads this exact JSON, so the field names are a contract with `addon.py`.
    /// Renaming one here without renaming it there disables that rule silently.
    #[test]
    fn the_wire_shape_matches_what_the_addon_reads() {
        let json = serde_json::to_string(&InterceptRules {
            block: vec!["x".into()],
            set_request_headers: vec![HeaderRule::default()],
            replace_request_body: vec![BodyRule::default()],
            set_response_headers: vec![HeaderRule::default()],
            stub_response: vec![StubRule {
                url_contains: String::new(),
                status: 200,
                body: String::new(),
                headers: Default::default(),
            }],
        })
        .unwrap();
        for key in [
            "block",
            "set_request_headers",
            "replace_request_body",
            "set_response_headers",
            "stub_response",
            "url_contains",
        ] {
            assert!(json.contains(key), "addon.py reads {key}: {json}");
        }
    }
}
