//! The `[host]` ceilings: what a daemon refuses once it is already running as much as it was told
//! to. A refusal carries the stable code `host_cap`, so every client can show it as a notice
//! instead of a generic failure.

use forge_config::host::HostConfig;

use crate::serve::SessionRegistry;

/// The stable error code clients key on.
pub(crate) const CODE: &str = "host_cap";

/// WebSocket close code a refused terminal attach is answered with (4000-4999 is application
/// space); the close reason is the human sentence.
pub(crate) const TERMINAL_CLOSE_CODE: u16 = 4429;

fn config() -> HostConfig {
    forge_config::load().unwrap_or_default().host
}

fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

pub(crate) fn sessions_refusal(live: usize, limit: u32) -> Option<String> {
    let max = HostConfig::limit(limit)?;
    (live >= max).then(|| {
        format!(
            "this host already has {} live, the most [host] max_active_sessions allows; archive \
             one and try again",
            plural(live, "session", "sessions")
        )
    })
}

pub(crate) fn terminals_refusal(open: usize, limit: u32) -> Option<String> {
    let max = HostConfig::limit(limit)?;
    (open >= max).then(|| {
        format!(
            "this host already has {} open, the most [host] max_terminals allows; close one and \
             try again",
            plural(open, "terminal", "terminals")
        )
    })
}

/// Why a new session must not start right now, if it must not. A resume of a session that is
/// already live adds nothing, so it is never refused.
pub(crate) async fn session_refusal(
    registry: &SessionRegistry,
    resuming: Option<&str>,
) -> Option<String> {
    let limit = config().max_active_sessions;
    if limit == 0 {
        return None;
    }
    if let Some(id) = resuming {
        if registry.get(id).await.is_some() {
            return None;
        }
    }
    sessions_refusal(registry.all().await.len(), limit)
}

/// Refuse a new session with the stable `host_cap` code when the host is at its ceiling.
pub(crate) async fn admit_session(
    state: &crate::serve::DaemonState,
    resuming: Option<&str>,
) -> Result<(), crate::serve::StartSessionError> {
    match session_refusal(&state.registry, resuming).await {
        Some(refusal) => Err(crate::serve::StartSessionError::coded(
            axum::http::StatusCode::TOO_MANY_REQUESTS,
            CODE,
            refusal,
        )),
        None => Ok(()),
    }
}

/// The configured terminal ceiling for the whole host, if any.
pub(crate) fn terminal_limit() -> u32 {
    config().max_terminals
}

/// Install the helper ceiling for this daemon process.
pub(crate) fn install_helper_limit() {
    if let Some(max) = HostConfig::limit(config().max_helpers) {
        forge_core::host_limits::set_helper_limit(max);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_limit_of_zero_never_refuses() {
        assert_eq!(sessions_refusal(500, 0), None);
        assert_eq!(terminals_refusal(500, 0), None);
    }

    #[test]
    fn the_nth_session_is_the_last_one_allowed() {
        assert_eq!(sessions_refusal(2, 3), None);
        let refused = sessions_refusal(3, 3).expect("at the ceiling");
        assert!(refused.contains("max_active_sessions"));
        assert!(terminals_refusal(1, 2).is_none());
        assert!(terminals_refusal(2, 2).unwrap().contains("max_terminals"));
    }
}
