//! Live subscription-window awareness for OpenCode Go (`opencode_go`).
//!
//! OpenCode Go bills a dollar-denominated allowance shared across every Go model in a workspace,
//! over three simultaneous windows (5-hour rolling, weekly, monthly). Chat completions return no
//! rate-limit headers at all — only a token `usage` block — so per-request observation, the way
//! Claude and Codex learn their windows, is impossible here. The only programmatic source is
//! `GET https://opencode.ai/zen/go/v1/usage`, which is UNDOCUMENTED upstream: the published docs
//! (<https://opencode.ai/docs/go/#usage-limits>, read 2026-09-01) only say to check the console.
//! Verified live against a real Go account on 2026-09-01:
//!
//! ```text
//! GET /zen/go/v1/usage   Authorization: Bearer <opencode_go key>
//! 200 {"usage":{"rolling" :{"status":"ok","percent":0,"resetsAt":"2026-09-02T00:30:48.917Z"},
//!               "weekly"  :{"status":"ok","percent":0,"resetsAt":"2026-09-07T00:00:00.917Z"},
//!               "monthly" :{"status":"ok","percent":0,"resetsAt":"2026-10-01T19:25:30.917Z"}}}
//! ```
//!
//! Re-probed 2026-09-02 for a per-model split (the dashboard shows one weekly quota per model,
//! with the pool percentage the sum of per-model percentages): `?breakdown=true` and
//! `?detail=models` return the same three-window body, `/usage/models`, `/usage/breakdown`,
//! `/quota` and `/limits` 404, and `/models` lists ids only. Per-model quota therefore comes from
//! the bundled table in `forge_mesh::subscription_cost`, not from this poll.
//!
//! Unauthenticated requests return 401; `/limits`, `/me` and `/zen/v1/usage` all 404. Because the
//! endpoint carries no compatibility promise, every failure mode here degrades to "unknown" — no
//! store write at all — rather than to a fabricated 0%. A window Forge cannot read must look
//! unobserved, never empty: recording 0% would tell the router a spent subscription is fresh.

use forge_store::Store;

/// The undocumented usage endpoint. See the module docs for the verified response shape.
const USAGE_URL: &str = "https://opencode.ai/zen/go/v1/usage";

/// How long a Go usage reading stays authoritative. The allowance is shared across a workspace and
/// can be spent outside Forge, so this matches the Codex freshness bound rather than the window
/// lengths — but it is also the minimum interval between polls, which keeps the refresher off the
/// per-request path.
pub(crate) const OPENCODE_GO_QUOTA_MAX_AGE_SECS: i64 = forge_types::CODEX_QUOTA_FRESHNESS_SECS;

/// Forge's provider namespace for OpenCode Go, and the key that store rows are written under.
pub(crate) const OPENCODE_GO_PROVIDER: &str = "opencode_go";

/// Response window name -> Forge's window vocabulary. `rolling` is the 5-hour window.
const WINDOW_KINDS: &[(&str, &str)] = &[
    ("rolling", "five_hour"),
    ("weekly", "weekly"),
    ("monthly", "monthly"),
];

/// Parse the usage body into quota hints. Pure and total: a malformed body, an absent window, a
/// non-`ok` per-window `status`, or an unparseable `resetsAt` each yield NO hint for that window,
/// so an unreadable response is indistinguishable from never having polled.
///
/// A window without a reset instant is deliberately dropped rather than recorded: pacing cannot
/// place a fraction on a timeline without one, and the endpoint has always supplied it.
fn parse_usage(body: &str) -> Vec<forge_types::QuotaHint> {
    let Ok(root) = serde_json::from_str::<serde_json::Value>(body) else {
        return Vec::new();
    };
    let Some(usage) = root.get("usage") else {
        return Vec::new();
    };
    WINDOW_KINDS
        .iter()
        .filter_map(|(remote, window)| {
            let entry = usage.get(remote)?;
            // `status` is the provider's own verdict on whether the reading is meaningful; only
            // A window that is over pace reports a non-"ok" status ("warning", "exceeded", …).
            // Those are exactly the observations pacing needs; dropping them froze the weekly
            // row at its last "ok" value while the pool kept draining (2026-09-02). Only a
            // window with no numeric percent at all is unobserved.
            let status = entry
                .get("status")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            // The field is named `percent` and is documented as a percentage of the window's
            // dollar cap; clamp rather than trust, since the value is undocumented upstream.
            let percent = entry.get("percent").and_then(serde_json::Value::as_f64)?;
            let fraction = (percent / 100.0).clamp(0.0, 1.0);
            let status = status_from_remote(status, fraction);
            let resets_at = entry
                .get("resetsAt")
                .and_then(serde_json::Value::as_str)
                .and_then(|iso| chrono::DateTime::parse_from_rfc3339(iso).ok())?
                .timestamp();
            Some(forge_types::QuotaHint {
                provider: OPENCODE_GO_PROVIDER.to_string(),
                window: (*window).to_string(),
                status,
                resets_at: Some(resets_at),
                fraction_used: Some(fraction),
            })
        })
        .collect()
}

/// Whether a fresh enough reading already exists, in which case polling is skipped.
/// The remote status word decides only the exhausted case; below that the observed fraction
/// decides, so a "warning" at 67% and an "ok" at 67% read the same.
fn status_from_remote(status: &str, fraction: f64) -> forge_types::QuotaStatus {
    let s = status.to_ascii_lowercase();
    if s.contains("exceed") || s.contains("exhaust") || s.contains("limit") || s == "error" {
        return forge_types::QuotaStatus::Exhausted;
    }
    forge_config::quota_status::status_from_fraction(fraction)
}

fn is_fresh(store: &Store) -> bool {
    store
        .subscription_age_secs(OPENCODE_GO_PROVIDER)
        .is_some_and(|age| age <= OPENCODE_GO_QUOTA_MAX_AGE_SECS)
}

/// Refresh the OpenCode Go subscription windows before a routing decision.
///
/// Best-effort in every direction: no key, an unreachable endpoint, a non-200 status, or a body
/// that does not parse all leave the store untouched, and the existing staleness filter then makes
/// the provider read as unobserved instead of as freshly empty.
pub(crate) async fn refresh_opencode_go_quota(store: &Store) {
    if is_fresh(store) {
        return;
    }
    // Read through the normal key-resolution path (env, then keyring/encrypted store); the key is
    // never logged or persisted here.
    let Ok(key) = forge_config::api_key(OPENCODE_GO_PROVIDER) else {
        return;
    };
    if key.is_empty() {
        return;
    }
    refresh_with_key_and_url(store, &key, USAGE_URL).await;
}

async fn refresh_with_key_and_url(store: &Store, key: &str, url: &str) {
    let Some(body) = fetch_usage(url, key).await else {
        return;
    };
    let now = chrono::Utc::now().timestamp();
    for hint in parse_usage(&body) {
        // A live poll genuinely was observed now, so `record_quota_at`'s stale guard only ever
        // rejects a write when something newer already landed.
        let _ = store.record_quota_at(&hint, now);
    }
}

async fn fetch_usage(url: &str, key: &str) -> Option<String> {
    let response = forge_provider::bundled_http_client()
        .get(url)
        .bearer_auth(key)
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        tracing::debug!("opencode_go usage endpoint returned {}", response.status());
        return None;
    }
    response.text().await.ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The exact body observed live on 2026-09-01, with non-zero percentages substituted so the
    /// mapping is actually asserted.
    const REAL_SHAPE: &str = r#"{"usage":{
        "rolling":{"status":"ok","percent":42,"resetsAt":"2026-09-02T00:30:48.917Z"},
        "weekly":{"status":"ok","percent":10.5,"resetsAt":"2026-09-07T00:00:00.917Z"},
        "monthly":{"status":"ok","percent":3,"resetsAt":"2026-10-01T19:25:30.917Z"}}}"#;

    fn hint<'a>(
        hints: &'a [forge_types::QuotaHint],
        window: &str,
    ) -> Option<&'a forge_types::QuotaHint> {
        hints.iter().find(|hint| hint.window == window)
    }

    #[test]
    fn all_three_windows_map_to_forge_window_kinds_with_their_reset_instants() {
        let hints = parse_usage(REAL_SHAPE);
        assert_eq!(hints.len(), 3);
        let five = hint(&hints, "five_hour").unwrap();
        assert_eq!(five.provider, OPENCODE_GO_PROVIDER);
        assert!((five.fraction_used.unwrap() - 0.42).abs() < 1e-9);
        assert_eq!(
            five.resets_at,
            Some(
                chrono::DateTime::parse_from_rfc3339("2026-09-02T00:30:48.917Z")
                    .unwrap()
                    .timestamp()
            )
        );
        assert!((hint(&hints, "weekly").unwrap().fraction_used.unwrap() - 0.105).abs() < 1e-9);
        assert!((hint(&hints, "monthly").unwrap().fraction_used.unwrap() - 0.03).abs() < 1e-9);
        assert!(
            hints.iter().all(|hint| hint.resets_at.is_some()),
            "a window with no reset instant is unpaceable"
        );
    }

    #[test]
    fn a_missing_window_leaves_only_the_windows_that_were_reported() {
        let hints = parse_usage(
            r#"{"usage":{"rolling":{"status":"ok","percent":5,"resetsAt":"2026-09-02T00:30:48Z"}}}"#,
        );
        assert_eq!(hints.len(), 1);
        assert_eq!(hints[0].window, "five_hour");
    }

    #[test]
    fn an_over_pace_window_is_still_an_observation() {
        // The weekly pool at 67% reports "warning"; the old parser dropped it and the weekly
        // row froze at its last "ok" value while grok-4.6 drained the pool.
        let hints = parse_usage(
            r#"{"usage":{"rolling":{"status":"ok","percent":5,"resetsAt":"2026-09-02T00:30:48Z"},
                        "weekly":{"status":"warning","percent":67.3,"resetsAt":"2026-09-07T00:00:00Z"},
                        "monthly":{"status":"exceeded","percent":100,"resetsAt":"2026-10-01T00:00:00Z"}}}"#,
        );
        assert_eq!(
            hints.len(),
            3,
            "every window with a percent is an observation"
        );
        let weekly = hints.iter().find(|h| h.window == "weekly").unwrap();
        assert!((weekly.fraction_used.unwrap() - 0.673).abs() < 1e-9);
        assert_eq!(
            weekly.status,
            forge_types::QuotaStatus::Ok,
            "67% is below the warning line"
        );
        let monthly = hints.iter().find(|h| h.window == "monthly").unwrap();
        assert_eq!(monthly.status, forge_types::QuotaStatus::Exhausted);
        // A window with an error status but no usable percent is still unobserved.
        let hints = parse_usage(
            r#"{"usage":{"rolling":{"status":"error","resetsAt":"2026-09-02T00:30:48Z"}}}"#,
        );
        assert!(hints.is_empty());
    }

    #[test]
    fn malformed_or_unexpected_bodies_yield_no_observation_at_all() {
        for body in [
            "",
            "not json",
            "{}",
            r#"{"usage":null}"#,
            r#"{"usage":{"rolling":"ok"}}"#,
            // percent present but not a number, and a window with no reset instant.
            r#"{"usage":{"rolling":{"status":"ok","percent":"42","resetsAt":"2026-09-02T00:30:48Z"},
                        "weekly":{"status":"ok","percent":9}}}"#,
            r#"{"usage":{"monthly":{"status":"ok","percent":9,"resetsAt":"not-a-date"}}}"#,
        ] {
            assert!(
                parse_usage(body).is_empty(),
                "must degrade to unknown, not 0%: {body}"
            );
        }
    }

    #[test]
    fn percentages_are_clamped_into_a_fraction() {
        let hints = parse_usage(
            r#"{"usage":{"rolling":{"status":"ok","percent":140,"resetsAt":"2026-09-02T00:30:48Z"},
                        "weekly":{"status":"ok","percent":-3,"resetsAt":"2026-09-07T00:00:00Z"}}}"#,
        );
        assert_eq!(hint(&hints, "five_hour").unwrap().fraction_used, Some(1.0));
        assert_eq!(hint(&hints, "weekly").unwrap().fraction_used, Some(0.0));
    }

    #[test]
    fn a_spent_window_is_recorded_as_exhausted_so_routing_treats_it_as_unusable() {
        // Hitting a Go limit leaves only the free models reachable and Zen-credit fallback is off
        // by default, so a spent window means the provider is unusable until reset — the same
        // semantics as a spent Claude/Codex window.
        let hints = parse_usage(
            r#"{"usage":{"rolling":{"status":"ok","percent":100,"resetsAt":"2026-09-02T00:30:48Z"}}}"#,
        );
        assert_eq!(hints[0].status, forge_types::QuotaStatus::Exhausted);
    }

    /// The body observed live on 2026-09-02 (weekly 67% = Grok 4.6's 41.5% + Kimi K3's 25.8% on
    /// the dashboard). It carries no per-model split, so the router's per-model quota multiplier
    /// has to come from the bundled table rather than from this poll.
    const CAPTURED_2026_09_02: &str = r#"{"usage":{"rolling":{"status":"ok","percent":0,"resetsAt":"2026-09-02T13:58:21.065Z"},"weekly":{"status":"ok","percent":67,"resetsAt":"2026-09-07T00:00:00.065Z"},"monthly":{"status":"ok","percent":33,"resetsAt":"2026-10-01T19:25:30.065Z"}}}"#;

    #[test]
    fn captured_body_parses_and_has_no_per_model_breakdown() {
        let hints = parse_usage(CAPTURED_2026_09_02);
        assert_eq!(hints.len(), 3);
        assert!((hint(&hints, "weekly").unwrap().fraction_used.unwrap() - 0.67).abs() < 1e-9);
        let root: serde_json::Value = serde_json::from_str(CAPTURED_2026_09_02).unwrap();
        let usage = root["usage"].as_object().unwrap();
        assert_eq!(
            usage.len(),
            3,
            "only the three windows, no per-model entries"
        );
        for window in usage.values() {
            assert!(window.get("models").is_none() && window.get("quota").is_none());
        }
        // The multiplier for the models that ate this week's pool comes from the bundled table.
        assert_eq!(
            forge_mesh::opencode_go_quota_multiplier("opencode_go::grok-4.6"),
            Some(4.0)
        );
    }

    /// Prints the response's key paths only (no values, no key material).
    #[tokio::test]
    #[ignore = "live probe"]
    async fn probe_live_shape() {
        let key = forge_config::api_key(OPENCODE_GO_PROVIDER).unwrap();
        let body = fetch_usage(USAGE_URL, &key).await.unwrap();
        let value: serde_json::Value = serde_json::from_str(&body).unwrap();
        fn shape(v: &serde_json::Value, path: &str, out: &mut Vec<String>) {
            match v {
                serde_json::Value::Object(map) => {
                    for (k, vv) in map {
                        shape(vv, &format!("{path}.{k}"), out);
                    }
                }
                serde_json::Value::Array(items) => match items.first() {
                    Some(first) => shape(first, &format!("{path}[]"), out),
                    None => out.push(format!("{path}[]")),
                },
                _ => out.push(path.to_string()),
            }
        }
        let mut out = Vec::new();
        shape(&value, "", &mut out);
        println!("SHAPE:\n{}", out.join("\n"));
    }

    fn serve_once(status: u16, body: &'static str) -> String {
        use std::io::{Read, Write};

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 2048];
            let bytes = stream.read(&mut request).unwrap();
            let request = String::from_utf8_lossy(&request[..bytes]);
            assert!(request.contains("Authorization: Bearer test-key"));
            let reason = match status {
                401 => "Unauthorized",
                500 => "Internal Server Error",
                _ => "OK",
            };
            write!(
                stream,
                "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        });
        format!("http://{address}/zen/go/v1/usage")
    }

    #[tokio::test]
    async fn non_success_responses_are_unknown_and_do_not_write_the_store() {
        for status in [401, 500] {
            let store = Store::open_in_memory().unwrap();
            let url = serve_once(
                status,
                r#"{"usage":{"rolling":{"status":"ok","percent":0,"resetsAt":"2026-09-02T00:30:48Z"}}}"#,
            );
            refresh_with_key_and_url(&store, "test-key", &url).await;
            assert_eq!(store.subscription_age_secs(OPENCODE_GO_PROVIDER), None);
            assert!(!store
                .bridge_fractions()
                .unwrap()
                .contains_key(OPENCODE_GO_PROVIDER));
        }
    }

    #[test]
    fn an_unreadable_response_never_writes_and_never_looks_fresh() {
        let store = Store::open_in_memory().unwrap();
        for hint in parse_usage("{\"error\":\"boom\"}") {
            store.record_quota_at(&hint, 100).unwrap();
        }
        assert!(!is_fresh(&store));
        assert_eq!(store.subscription_age_secs(OPENCODE_GO_PROVIDER), None);
        assert!(!store
            .bridge_fractions()
            .unwrap()
            .contains_key(OPENCODE_GO_PROVIDER));
    }

    #[test]
    fn a_fresh_reading_suppresses_the_next_poll() {
        let store = Store::open_in_memory().unwrap();
        let now = chrono::Utc::now().timestamp();
        for hint in parse_usage(REAL_SHAPE) {
            store
                .record_quota_at(
                    &forge_types::QuotaHint {
                        resets_at: Some(now + 3600),
                        ..hint
                    },
                    now,
                )
                .unwrap();
        }
        assert!(is_fresh(&store));
    }
}
