//! `forge doctor`'s provider-key checks: the live reachability probe per keyed provider, the
//! auth-rejection classifier and the persisted key verdicts.

use super::{check, Check, Status};

/// The one wording for "this provider read the stored key and refused it", shared by the key row,
/// the reachability row and (by prefix match) the routing verdict.
pub(crate) fn auth_rejected_detail(provider: &str, code: u16) -> String {
    format!("rejected by {provider} (HTTP {code}): the stored key is invalid")
}

pub(crate) const AUTH_REJECTED_PREFIX: &str = "rejected by ";

/// Is this listing failure the provider REFUSING the credential (vs. a network/endpoint blip)?
/// Returns the HTTP status to report. The typed [`ProviderError::Auth`] variant is authoritative;
/// the text scan is a backstop for adapters that stringify a status before we can classify it.
///
/// Deliberately narrow: only 401/403 and unambiguous "invalid/unauthorized key" prose count, so a
/// connection refusal, a 5xx or a missing listing endpoint keeps the old, softer wording.
pub(crate) fn auth_rejection_status(err: &forge_provider::ProviderError) -> Option<u16> {
    let text = err.to_string();
    let lower = text.to_lowercase();
    let code = if lower.contains("403") || lower.contains("forbidden") {
        403
    } else {
        401
    };
    if matches!(err, forge_provider::ProviderError::Auth(_)) {
        return Some(code);
    }
    let names_401 = lower.contains("401") || lower.contains("unauthorized");
    let names_403 = lower.contains("403") || lower.contains("forbidden");
    let names_bad_key = lower.contains("invalid api key")
        || lower.contains("invalid_api_key")
        || lower.contains("incorrect api key")
        || lower.contains("authentication_error")
        || lower.contains("invalid authentication");
    (names_401 || names_403 || names_bad_key).then_some(code)
}

/// Downgrade the `<provider> key` rows for providers whose credential the provider itself
/// rejected. The key rows are built from key PRESENCE, which cannot see a 401.
pub(crate) fn mark_rejected_keys(checks: &mut [Check], reachability: &[Check]) {
    for reach in reachability {
        let Some(provider) = reach.label.strip_suffix(" reachability") else {
            continue;
        };
        if reach.status != Status::Fail || !reach.detail.starts_with(AUTH_REJECTED_PREFIX) {
            continue;
        }
        for row in checks.iter_mut() {
            if row.label == format!("{provider} key") {
                row.status = Status::Fail;
                row.detail = reach.detail.clone();
                row.fix = Some(format!("`forge auth {provider}` to replace the stored key"));
            }
        }
    }
}

/// LIVE: for each KEYED provider, can we actually list its models within a timeout? A keyed
/// provider whose discovery times out silently drops out of routing and the mesh falls back to a
/// keyless default (the "groq for everything" churn) — a key-PRESENCE check can't see this.
pub(super) async fn provider_reachability_checks() -> Vec<Check> {
    const REACH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(8);
    // Probe every keyed provider CONCURRENTLY — each is an independent network call, so a sequential
    // loop made doctor pay the SUM of every provider's timeout (N keyed providers × 8s). join_all
    // collapses that to the slowest single probe; results stay in provider order.
    let store = crate::open_store().ok().map(std::sync::Arc::new);
    let probes = forge_config::known_key_providers()
        .filter(|p| forge_config::has_api_key(p))
        .map(|p| {
            let store = store.clone();
            async move {
                let res = tokio::time::timeout(REACH_TIMEOUT, forge_provider::list_models(p)).await;
                if let Some(store) = &store {
                    record_probe_verdict(store, p, &res);
                }
                match res {
            Ok(Ok(list)) if !list.is_empty() => check(
                Status::Ok,
                &format!("{p} reachability"),
                format!("{} models", list.len()),
                None,
            ),
            // Reachable but empty listing — chat may still work; not actionable, so Info.
            Ok(Ok(_)) => check(
                Status::Info,
                &format!("{p} reachability"),
                "responded, but listed no models",
                None,
            ),
            // An authentication rejection IS a reliable usability signal: the provider read the
            // stored key and refused it, so every chat call fails the same way. Anything else
            // (no listing endpoint, a keying difference, a 5xx, a refused connection) is not —
            // several adapters (Gemini, Groq, the cerebras custom endpoint) error here while chat
            // works fine, so those stay Info.
            Ok(Err(e)) => match auth_rejection_status(&e) {
                Some(code) => check(
                    Status::Fail,
                    &format!("{p} reachability"),
                    match e.key_failure() {
                        Some((kind, reason)) => {
                            format!("{} — {reason}", auth_rejected_detail(p, code).replace(
                                "the stored key is invalid",
                                kind.headline(),
                            ))
                        }
                        None => auth_rejected_detail(p, code),
                    },
                    Some(&format!("`forge auth {p}` to replace the stored key")),
                ),
                None => check(
                    Status::Info,
                    &format!("{p} reachability"),
                    "model listing unavailable — chat unaffected",
                    None,
                ),
            },
            // The churn cause: keyed but unreachable. Its models won't route.
            Err(_) => check(
                Status::Fail,
                &format!("{p} reachability"),
                format!("discovery timed out (> {}s)", REACH_TIMEOUT.as_secs()),
                Some(
                    "provider/network unreachable — its models won't route this session; the mesh \
                     falls back to another provider",
                ),
            ),
            }
            }
        });
    futures::future::join_all(probes).await
}

/// A live listing is the freshest evidence of whether a key works, so it updates the persisted
/// verdict the providers screens show: refused sets it, a listing clears it, a blip leaves it.
fn record_probe_verdict(
    store: &forge_store::Store,
    provider: &str,
    result: &Result<
        Result<Vec<String>, forge_provider::ProviderError>,
        tokio::time::error::Elapsed,
    >,
) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    match result {
        Ok(Ok(list)) if !list.is_empty() => {
            let _ = store.clear_provider_verdict(provider);
        }
        Ok(Err(e)) => {
            if let Some((kind, reason)) = e.key_failure() {
                let _ = store.set_provider_verdict(provider, kind.as_str(), &reason, now);
            }
        }
        _ => {}
    }
}

/// The persisted key verdicts, one red row each, so a key refused weeks ago is still named here
/// even when this run's probe was skipped or timed out.
pub(super) fn key_verdict_checks(reachability: &[Check]) -> Vec<Check> {
    let Ok(store) = crate::open_store() else {
        return Vec::new();
    };
    store
        .provider_verdicts()
        .unwrap_or_default()
        .into_iter()
        .filter(|v| forge_config::has_api_key(&v.provider))
        // This run's own probe already said so, in its own words.
        .filter(|v| {
            let label = format!("{} reachability", v.provider);
            !reachability
                .iter()
                .any(|c| c.label == label && c.status == Status::Fail)
        })
        .map(|v| {
            let headline = forge_provider::KeyFailure::from_word(&v.status)
                .map_or("Key rejected", forge_provider::KeyFailure::headline);
            check(
                Status::Fail,
                &format!("{} key verdict", v.provider),
                format!(
                    "{headline}: {} (since {})",
                    v.reason,
                    short_age(v.first_seen)
                ),
                Some(&format!("`forge auth {}` to replace the key", v.provider)),
            )
        })
        .collect()
}

fn short_age(epoch: i64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let secs = (now - epoch).max(0);
    match secs {
        0..=3599 => format!("{}m ago", secs / 60),
        3600..=86_399 => format!("{}h ago", secs / 3600),
        _ => format!("{}d ago", secs / 86_400),
    }
}
