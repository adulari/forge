//! Which core errors are Forge bugs worth reporting (`forge-reporting`), and which are the user's
//! world (keys, quota, network, config, a stale host path).
//!
//! The mapping is by type, so it does not depend on message wording; only the gray areas (a
//! provider "request" error, a SQLite error, a lattice error) fall back to the text rules in
//! `forge_reporting::classify`.

use forge_provider::ProviderError;
use forge_reporting::{ErrorKind, Incident};
use forge_store::StoreError;

use crate::CoreError;

/// The incident to report for a turn-ending error, or `None` when it is an expected condition.
pub(crate) fn incident_for_turn(error: &CoreError) -> Option<Incident> {
    const SITE: &str = "forge_core::turn";
    match error {
        CoreError::Internal(message) => Some(Incident::bug(ErrorKind::Internal, SITE, message)),
        CoreError::Json(e) => Some(Incident::bug(ErrorKind::Internal, "forge_core::json", e)),
        CoreError::Store(e) => store_incident(e),
        CoreError::Lattice(e) => Some(Incident::gray(
            ErrorKind::Internal,
            "forge_core::lattice",
            e,
        )),
        CoreError::Provider(ProviderError::Request(message)) => {
            forge_reporting::provider_request_is_bug(message).then(|| {
                Incident::bug(
                    ErrorKind::ProviderProtocol,
                    "forge_provider::request",
                    message,
                )
            })
        }
        CoreError::NoHealthyModel {
            reason, last_error, ..
        } if *reason == "request error" => forge_reporting::provider_request_is_bug(last_error)
            .then(|| {
                Incident::bug(
                    ErrorKind::ProviderProtocol,
                    "forge_provider::chain",
                    last_error,
                )
            }),
        CoreError::Provider(_)
        | CoreError::NoHealthyModel { .. }
        | CoreError::ModelUnavailable { .. }
        | CoreError::SessionNotFound(_)
        | CoreError::Workspace(_)
        | CoreError::Precondition(_)
        | CoreError::StaleHostPath(_)
        | CoreError::TurnBlocked(_) => None,
    }
}

fn store_incident(error: &StoreError) -> Option<Incident> {
    match error {
        // The binary is older than the database: the fix is upgrading, not a bug report.
        StoreError::SchemaTooNew { .. } => None,
        other => Some(Incident::gray(ErrorKind::Store, "forge_store", other)),
    }
}

/// Report a turn-ending error if it is a bug. Cheap when reporting is off.
pub(crate) fn report_turn_error(error: &CoreError) {
    if let Some(incident) = incident_for_turn(error) {
        forge_reporting::report(incident);
    }
}

/// Report a failed compaction. Provider-side failures are expected; anything else is a bug.
pub(crate) fn report_compaction_error(error: &CoreError) {
    if let Some(mut incident) = incident_for_turn(error) {
        incident.kind = ErrorKind::Compaction;
        incident.site = "forge_core::compaction".to_string();
        forge_reporting::report(incident);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_world_errors_are_not_reported() {
        let expected = [
            CoreError::Provider(ProviderError::Auth("bad key".into())),
            CoreError::Provider(ProviderError::RateLimited {
                message: "slow down".into(),
                retry_after: None,
            }),
            CoreError::Provider(ProviderError::Unavailable("down".into())),
            CoreError::Provider(ProviderError::Request("context length exceeded".into())),
            CoreError::Precondition("commit your changes first".into()),
            CoreError::StaleHostPath("/old/host".into()),
            CoreError::SessionNotFound("abc".into()),
            CoreError::TurnBlocked("review gate".into()),
            CoreError::Store(StoreError::SchemaTooNew {
                found: 30,
                supported: 25,
            }),
            CoreError::NoHealthyModel {
                verdict: "all providers failed".into(),
                model: "m".into(),
                reason: "rate-limited",
                last_error: "429".into(),
            },
        ];
        for error in &expected {
            assert!(incident_for_turn(error).is_none(), "{error}");
        }
    }

    #[test]
    fn real_defects_are_reported_with_a_stable_site() {
        let internal = incident_for_turn(&CoreError::Internal("tool call without id".into()));
        assert_eq!(internal.unwrap().kind, ErrorKind::Internal);
        let protocol = incident_for_turn(&CoreError::Provider(ProviderError::Request(
            "malformed tool call arguments in response".into(),
        )))
        .unwrap();
        assert_eq!(protocol.kind, ErrorKind::ProviderProtocol);
        let store = incident_for_turn(&CoreError::Store(StoreError::InvalidValue(
            "negative seq".into(),
        )))
        .unwrap();
        assert_eq!(store.kind, ErrorKind::Store);
        assert!(store.check_text);
    }
}
