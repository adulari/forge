//! Request interception rules.
//!
//! When any rule is active, Chrome pauses every outbound request (`Fetch.enable`) and the session
//! must resolve each one — continue it, continue it with changes, or fail it. A paused request that
//! is never answered hangs the page, so the decision has to be a fast, total function of the
//! request, never a round trip to the model. That is what this is: a declarative ruleset the pump
//! evaluates synchronously.
//!
//! This is deliberately not a general "the model decides per request" interceptor. That shape reads
//! well and works badly: every request would block on an agent turn, and a page makes dozens. Rules
//! set up front — block this noise, inject that header — cover the reverse-engineering needs
//! (silence analytics, force a header, kill a request to see the fallback) without that trap.

/// What to do with one paused request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Let it proceed unchanged.
    Continue,
    /// Let it proceed with these request headers merged over its own (case-insensitive name match;
    /// a rule value replaces the request's, new names are added).
    ContinueWithHeaders(Vec<(String, String)>),
    /// Fail it, as if blocked by a client extension.
    Block,
}

/// The active interception ruleset.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InterceptionRules {
    /// Case-insensitive URL substrings; a request whose URL contains any of these is failed.
    pub block: Vec<String>,
    /// Headers to set on every non-blocked request.
    pub set_request_headers: Vec<(String, String)>,
}

impl InterceptionRules {
    pub fn is_empty(&self) -> bool {
        self.block.is_empty() && self.set_request_headers.is_empty()
    }

    /// Decide what to do with a request, given its URL and its own headers.
    pub fn decide(&self, url: &str, request_headers: &[(String, String)]) -> Decision {
        let lower = url.to_lowercase();
        if self
            .block
            .iter()
            .any(|pattern| lower.contains(&pattern.to_lowercase()))
        {
            return Decision::Block;
        }
        if self.set_request_headers.is_empty() {
            return Decision::Continue;
        }
        Decision::ContinueWithHeaders(self.merge_headers(request_headers))
    }

    /// The request's own headers with the rule headers applied over them: a rule name replaces any
    /// existing header of the same name (case-insensitive), and a rule name not present is added.
    fn merge_headers(&self, request_headers: &[(String, String)]) -> Vec<(String, String)> {
        let mut merged: Vec<(String, String)> = request_headers
            .iter()
            .filter(|(name, _)| {
                !self
                    .set_request_headers
                    .iter()
                    .any(|(rule_name, _)| rule_name.eq_ignore_ascii_case(name))
            })
            .cloned()
            .collect();
        merged.extend(self.set_request_headers.iter().cloned());
        merged
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_blocked_url_is_failed_regardless_of_headers() {
        let rules = InterceptionRules {
            block: vec!["google-analytics.com".into(), "/telemetry".into()],
            ..InterceptionRules::default()
        };
        assert_eq!(
            rules.decide("https://www.GOOGLE-analytics.com/collect", &[]),
            Decision::Block,
            "matching is case-insensitive"
        );
        assert_eq!(
            rules.decide("https://app.test/telemetry/event", &[]),
            Decision::Block
        );
        assert_eq!(
            rules.decide("https://app.test/api/login", &[]),
            Decision::Continue
        );
    }

    #[test]
    fn a_header_rule_replaces_not_duplicates() {
        // Two Authorization headers is not "override" — servers pick one unpredictably. The rule
        // must replace the request's own value, matching the header name case-insensitively.
        let rules = InterceptionRules {
            set_request_headers: vec![("Authorization".into(), "Bearer NEW".into())],
            ..InterceptionRules::default()
        };
        let decision = rules.decide(
            "https://app.test/api",
            &[
                ("authorization".into(), "Bearer OLD".into()),
                ("Accept".into(), "application/json".into()),
            ],
        );
        match decision {
            Decision::ContinueWithHeaders(headers) => {
                let auth: Vec<_> = headers
                    .iter()
                    .filter(|(name, _)| name.eq_ignore_ascii_case("authorization"))
                    .collect();
                assert_eq!(auth.len(), 1, "exactly one Authorization: {headers:?}");
                assert_eq!(auth[0].1, "Bearer NEW");
                assert!(
                    headers
                        .iter()
                        .any(|(n, v)| n == "Accept" && v == "application/json"),
                    "untouched headers survive: {headers:?}"
                );
            }
            other => panic!("expected header override, got {other:?}"),
        }
    }

    #[test]
    fn a_new_header_is_added() {
        let rules = InterceptionRules {
            set_request_headers: vec![("X-Debug".into(), "1".into())],
            ..InterceptionRules::default()
        };
        match rules.decide("https://app.test/", &[("Accept".into(), "*/*".into())]) {
            Decision::ContinueWithHeaders(headers) => {
                assert!(headers.iter().any(|(n, v)| n == "X-Debug" && v == "1"));
                assert!(headers.iter().any(|(n, _)| n == "Accept"));
            }
            other => panic!("expected header override, got {other:?}"),
        }
    }

    #[test]
    fn no_rules_is_a_plain_continue_so_nothing_pays_for_interception() {
        let rules = InterceptionRules::default();
        assert!(rules.is_empty());
        assert_eq!(rules.decide("https://x.test/", &[]), Decision::Continue);
    }
}
