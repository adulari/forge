//! Live health probes kept separate from the main doctor report to keep each module reviewable.

use crate::doctor::{check, short, Check, Status};

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
        None => match crate::serve::daemon_token(false) {
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
