//! Live health probes kept separate from the main doctor report to keep each module reviewable.

use crate::doctor::{check, short, Check, Status};

/// Report the state that decides whether the mesh can select each configured provider. Live
/// discovery is deliberately passed in separately: it is supporting evidence, while this line is
/// the provider's routing verdict.
pub(crate) fn provider_routing_checks(reachability: &[Check]) -> Vec<Check> {
    let Some(path) = crate::doctor::doctor_store_path() else {
        return Vec::new();
    };
    if !path.exists() {
        return Vec::new();
    }
    let Ok(store) = forge_store::Store::open(&path) else {
        return Vec::new();
    };

    let excluded = store.current_excluded_providers().unwrap_or_default();
    let quota = store.current_quota().unwrap_or_default();
    provider_routing_report(reachability, &excluded, &quota)
}

fn configured_routing_providers() -> std::collections::BTreeSet<String> {
    let config = forge_config::load().ok();
    let mut providers = std::collections::BTreeSet::new();
    providers.extend(
        forge_config::known_key_providers()
            .filter(|provider| forge_config::has_api_key(provider))
            .map(str::to_string),
    );
    providers.extend(
        forge_provider::CliKind::all()
            .into_iter()
            .filter(|kind| kind.available())
            .map(|kind| kind.prefix().to_string()),
    );
    if let Some(config) = config {
        providers.extend(config.mesh.subscriptions.into_keys());
    }
    providers
}

fn provider_routing_report(
    reachability: &[Check],
    excluded: &[(String, i64, String)],
    quota: &forge_types::SubscriptionQuota,
) -> Vec<Check> {
    let mut providers = configured_routing_providers();
    providers.extend(excluded.iter().map(|(provider, _, _)| provider.clone()));
    providers.extend(quota.providers().map(str::to_string));
    providers.extend(reachability.iter().filter_map(|check| {
        check
            .label
            .strip_suffix(" reachability")
            .map(str::to_string)
    }));
    providers
        .into_iter()
        .map(|provider| {
            let excluded = excluded
                .iter()
                .find(|(name, _, _)| name == &provider)
                .map(|(_, until, reason)| (*until, reason.as_str()));
            let reach = reachability
                .iter()
                .find(|check| check.label == format!("{provider} reachability"));
            let rejected = reach.is_some_and(|check| {
                check.status == Status::Fail
                    && check
                        .detail
                        .starts_with(crate::doctor::AUTH_REJECTED_PREFIX)
            });
            let unreachable = !rejected && reach.is_some_and(|check| check.status == Status::Fail);
            routing_check(
                &provider,
                excluded,
                quota.status_for(&provider),
                quota.observed_fraction_for(&provider),
                unreachable,
                rejected,
            )
        })
        .collect()
}

fn routing_check(
    provider: &str,
    excluded: Option<(i64, &str)>,
    quota: forge_types::QuotaStatus,
    fraction: Option<f64>,
    unreachable: bool,
    rejected: bool,
) -> Check {
    // A refused credential outranks every other verdict: nothing routes to this provider until the
    // key is replaced, and saying "usable" here is what made an invalid key look healthy.
    if rejected {
        return check(
            Status::Fail,
            &format!("{provider} routing"),
            "not usable — the provider rejected the stored credential",
            Some(&format!(
                "`forge auth {provider}` to replace the stored key"
            )),
        );
    }
    if let Some((until, reason)) = excluded {
        check(
            Status::Warn,
            &format!("{provider} routing"),
            format!(
                "excluded — out of routing for {} ({})",
                human_remaining(until),
                short(reason)
            ),
            Some("re-verify with `forge models --probe`, or repair the provider credential"),
        )
    } else if quota == forge_types::QuotaStatus::Exhausted {
        let fraction = fraction
            .map(|fraction| format!(" ({:.0}% observed)", fraction * 100.0))
            .unwrap_or_default();
        check(
            Status::Warn,
            &format!("{provider} routing"),
            format!("quota-exhausted{fraction} — mesh routes around it"),
            Some("wait for the subscription window to reset or use another provider"),
        )
    } else if unreachable {
        check(
            Status::Warn,
            &format!("{provider} routing"),
            "unreachable — mesh cannot route to it",
            Some("check the provider/network and run `forge doctor` again"),
        )
    } else {
        check(Status::Ok, &format!("{provider} routing"), "usable", None)
    }
}

/// Report what the mesh is currently refusing to route to, and why.
///
/// A provider-wide exclusion was previously invisible: it appeared in no doctor section, no mesh
/// overview, no statusline, and nowhere on the phone. The only way to find one was to query
/// `model_health` by hand — which is how a healthy claude-cli subscription stayed benched while
/// every complex task piled onto a codex window that then ran out.
///
/// The provider rows are keyed `__forge_provider__::<name>`, so they are read through
/// `current_excluded_providers` rather than by filtering model ids on a provider prefix — a filter
/// like `claude-cli%` matches none of them.
pub(crate) fn model_health_checks() -> Vec<Check> {
    let Some(path) = crate::doctor::doctor_store_path() else {
        return Vec::new();
    };
    if !path.exists() {
        return Vec::new();
    }
    let store = match forge_store::Store::open(&path) {
        Ok(store) => store,
        // The store section already reports an unopenable store; don't duplicate the failure.
        Err(_) => return Vec::new(),
    };
    let excluded = store.current_excluded_providers().unwrap_or_default();
    let benched = store.current_benched_report().unwrap_or_default();
    model_health_report(&excluded, benched.len())
}

/// Pure half of [`model_health_checks`], so the reporting is testable without a real store.
fn model_health_report(excluded: &[(String, i64, String)], benched_rows: usize) -> Vec<Check> {
    let mut out = Vec::new();
    for (provider, until, reason) in excluded {
        out.push(check(
            Status::Warn,
            "provider excluded",
            format!(
                "{provider} — every model is out of routing for {} ({})",
                human_remaining(*until),
                short(reason)
            ),
            Some("`forge models --probe` to re-verify it now, or `forge auth <provider>` if the credential really is dead"),
        ));
    }
    let model_rows = benched_rows.saturating_sub(excluded.len());
    if model_rows > 0 {
        out.push(check(
            Status::Info,
            "models benched",
            format!("{model_rows} model(s) currently out of routing"),
            Some("`forge models --probe` to recheck them"),
        ));
    }
    if out.is_empty() {
        out.push(check(
            Status::Ok,
            "model health",
            "no provider or model is benched",
            None,
        ));
    }
    out
}

/// A compact "12m" / "3h 20m" remaining, from an absolute epoch-second expiry.
fn human_remaining(until_epoch: i64) -> String {
    let now = chrono::Utc::now().timestamp();
    let secs = (until_epoch - now).max(0);
    if secs >= 3600 {
        format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
    } else if secs >= 60 {
        format!("{}m", secs / 60)
    } else {
        format!("{secs}s")
    }
}

pub(crate) async fn daemon_fleet_check(port: u16) -> Check {
    // Prefer the live discovery record so LAN daemons are probed over HTTPS with their current
    // token. A stale/missing record is expected after a crash, so fall back to the persisted token
    // and the local HTTP endpoint used by `forge serve --local`.
    let state = crate::serve::read_state()
        .ok()
        .flatten()
        .filter(|state| state.port == port && state.process_is_alive());
    let token = match state.as_ref().map(|state| state.token.clone()) {
        Some(token) => token,
        None => match crate::serve::read_daemon_token() {
            Ok(token) => token,
            Err(error) => {
                return check(
                    Status::Fail,
                    "daemon fleet",
                    format!(
                        "port {port} responds, but its token is unavailable: {}",
                        short(&error.to_string())
                    ),
                    Some(
                        "start `forge serve` once to create the daemon token, then run `forge doctor` again",
                    ),
                )
            }
        },
    };
    let tls = state.as_ref().is_some_and(|state| state.exposure == "lan");
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .danger_accept_invalid_certs(tls)
        .build()
    {
        Ok(client) => client,
        Err(error) => {
            return check(
                Status::Fail,
                "daemon fleet",
                format!(
                    "could not build the discovery client: {}",
                    short(&error.to_string())
                ),
                Some("check the Forge installation and run `forge doctor` again"),
            )
        }
    };
    let scheme = if tls { "https" } else { "http" };
    let url = format!("{scheme}://127.0.0.1:{port}/{token}/api/sessions");
    let response = match client.get(&url).send().await {
        Ok(response) => response,
        Err(error) => {
            return check(
                Status::Fail,
                "daemon fleet",
                format!(
                    "port {port} responds, but /api/sessions failed: {}",
                    short(&error.to_string())
                ),
                Some("`forge service restart`; if it persists, inspect the daemon journal"),
            )
        }
    };
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return check(
            Status::Fail,
            "daemon fleet",
            "daemon rejected its persisted token (404)",
            Some("`forge service restart` to refresh the daemon and its discovery state"),
        );
    }
    if !response.status().is_success() {
        return check(
            Status::Fail,
            "daemon fleet",
            format!("/api/sessions returned {}", response.status()),
            Some("`forge service restart`; if it persists, inspect the daemon journal"),
        );
    }
    let sessions = match response.json::<Vec<crate::attach::SessionInfo>>().await {
        Ok(sessions) => sessions,
        Err(error) => {
            return check(
                Status::Fail,
                "daemon fleet",
                format!(
                    "/api/sessions returned invalid JSON: {}",
                    short(&error.to_string())
                ),
                Some("upgrade Forge so the daemon and doctor use the same API version"),
            )
        }
    };
    if sessions.is_empty() {
        // An idle daemon is valid, but make the fact visible: the endpoint was exercised and no
        // live fleet is currently hosted, rather than silently treating an empty response as proof
        // that the daemon can resurrect and serve sessions.
        check(
            Status::Warn,
            "daemon fleet",
            "discovery endpoint responds, but no live sessions are hosted",
            Some("start a session with `forge run` or `forge serve` to verify the live fleet"),
        )
    } else {
        check(
            Status::Ok,
            "daemon fleet",
            format!("{} live session(s)", sessions.len()),
            None,
        )
    }
}

/// Report Anywhere independently from local daemon health. A configured account can still be
/// unreachable when the local daemon (which owns the connector supervisor) is down.
pub(crate) fn anywhere_checks() -> Vec<Check> {
    let config = match forge_config::load() {
        Ok(config) => config,
        Err(error) => {
            return vec![check(
                Status::Warn,
                "Anywhere connector",
                format!(
                    "could not load configuration: {}",
                    short(&error.to_string())
                ),
                Some("fix the configuration, then run `forge doctor` again"),
            )]
        }
    };
    if !config.anywhere.enabled {
        return vec![check(
            Status::Info,
            "Anywhere connector",
            "disabled",
            Some("`forge anywhere enable` to sync this host with Forge Anywhere"),
        )];
    }

    let state = match crate::anywhere::StateStore::platform().and_then(|store| store.load()) {
        Ok(state) => state,
        Err(error) => {
            return vec![check(
                Status::Fail,
                "Anywhere connector",
                format!(
                    "enabled, but local enrollment state is unreadable: {}",
                    short(&error.to_string())
                ),
                Some("run `forge anywhere doctor`; restore the state file or log in again"),
            )]
        }
    };
    if !state.is_logged_in() || state.host_id.is_none() {
        return vec![check(
            Status::Warn,
            "Anywhere connector",
            "enabled, but this host is not enrolled",
            Some("`forge anywhere setup` to enroll and activate this host"),
        )];
    }

    let daemon_running = crate::serve::read_state()
        .ok()
        .flatten()
        .is_some_and(|serve| serve.process_is_alive());
    if daemon_running {
        vec![check(
            Status::Ok,
            "Anywhere connector",
            "enabled and enrolled; local daemon is running",
            None,
        )]
    } else {
        vec![check(
            Status::Fail,
            "Anywhere connector",
            "enabled and enrolled, but the local daemon is offline",
            Some("`forge service start` (or `forge serve --local`) to start the connector supervisor"),
        )]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `forge doctor` is the command people actually run when Forge feels wrong. A subscription
    /// that has silently vanished from routing must appear there, with its reason and how long it
    /// stays gone — the whole reason this defect cost a day was that nothing said so anywhere.
    #[test]
    fn excluded_but_reachable_provider_is_not_reported_healthy() {
        let until = chrono::Utc::now().timestamp() + 20 * 60;
        let reachability = vec![check(
            Status::Ok,
            "opencode_go reachability",
            "33 models",
            None,
        )];
        let rows = provider_routing_report(
            &reachability,
            &[(
                "opencode_go".to_string(),
                until,
                "excluded: provider auth failed".to_string(),
            )],
            &forge_types::SubscriptionQuota::default(),
        );
        let row = rows
            .iter()
            .find(|row| row.label == "opencode_go routing")
            .expect("excluded reachable provider gets a routing verdict");
        assert_eq!(row.status, Status::Warn);
        assert!(row.detail.contains("excluded"), "{}", row.detail);
        assert!(!row.detail.contains("usable"), "{}", row.detail);
    }

    #[test]
    fn a_rejected_credential_makes_routing_not_usable() {
        let reachability = vec![check(
            Status::Fail,
            "groq reachability",
            crate::doctor::auth_rejected_detail("groq", 401),
            None,
        )];
        let rows = provider_routing_report(
            &reachability,
            &[],
            &forge_types::SubscriptionQuota::default(),
        );
        let row = rows
            .iter()
            .find(|row| row.label == "groq routing")
            .expect("a rejected provider still gets a routing verdict");
        assert_eq!(row.status, Status::Fail);
        assert!(row.detail.starts_with("not usable"), "{}", row.detail);
        assert_eq!(
            row.fix.as_deref(),
            Some("`forge auth groq` to replace the stored key")
        );
    }

    #[test]
    fn a_timed_out_provider_keeps_the_unreachable_wording() {
        let reachability = vec![check(
            Status::Fail,
            "groq reachability",
            "discovery timed out (> 8s)",
            None,
        )];
        let rows = provider_routing_report(
            &reachability,
            &[],
            &forge_types::SubscriptionQuota::default(),
        );
        let row = rows
            .iter()
            .find(|row| row.label == "groq routing")
            .expect("routing verdict");
        assert_eq!(row.status, Status::Warn);
        assert!(row.detail.contains("unreachable"), "{}", row.detail);
    }

    #[test]
    fn exhausted_subscription_reports_observed_fraction() {
        let quota = forge_types::SubscriptionQuota::new(std::collections::HashMap::from([(
            "claude-cli".to_string(),
            forge_types::QuotaStatus::Exhausted,
        )]))
        .with_fractions(std::collections::HashMap::from([(
            "claude-cli".to_string(),
            0.97,
        )]));
        let rows = provider_routing_report(&[], &[], &quota);
        let row = rows
            .iter()
            .find(|row| row.label == "claude-cli routing")
            .expect("quota-only provider gets a routing verdict");
        assert_eq!(row.status, Status::Warn);
        assert!(row.detail.contains("quota-exhausted"), "{}", row.detail);
        assert!(row.detail.contains("97% observed"), "{}", row.detail);
    }

    #[test]
    fn doctor_reports_an_excluded_provider_with_its_reason_and_expiry() {
        let until = chrono::Utc::now().timestamp() + 20 * 60;
        let checks = model_health_report(
            &[(
                "claude-cli".to_string(),
                until,
                "excluded: provider auth failed: auth failed".to_string(),
            )],
            1,
        );
        let row = checks
            .iter()
            .find(|c| c.label == "provider excluded")
            .expect("an excluded provider gets its own doctor line");
        assert_eq!(row.status, Status::Warn);
        assert!(row.detail.contains("claude-cli"), "{}", row.detail);
        assert!(row.detail.contains("auth failed"), "{}", row.detail);
        assert!(row.detail.contains("20m"), "{}", row.detail);
        assert!(row.fix.as_deref().unwrap().contains("forge models --probe"));
    }

    #[test]
    fn doctor_says_so_plainly_when_nothing_is_benched() {
        let checks = model_health_report(&[], 0);
        assert_eq!(checks.len(), 1);
        assert_eq!(checks[0].status, Status::Ok);
    }

    #[test]
    fn benched_model_rows_are_counted_separately_from_provider_rows() {
        let until = chrono::Utc::now().timestamp() + 600;
        let checks =
            model_health_report(&[("groq".to_string(), until, "auth failed".to_string())], 3);
        let models = checks
            .iter()
            .find(|c| c.label == "models benched")
            .expect("the remaining model rows are still reported");
        assert!(models.detail.starts_with("2 model(s)"), "{}", models.detail);
    }

    #[test]
    fn human_remaining_never_goes_negative() {
        let now = chrono::Utc::now().timestamp();
        assert_eq!(human_remaining(now - 100), "0s");
        assert_eq!(human_remaining(now + 125), "2m");
        assert_eq!(human_remaining(now + 7200 + 60), "2h 1m");
    }
}
