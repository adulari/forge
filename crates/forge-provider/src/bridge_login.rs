//! Whether the CLI bridges have a login to route through.

use crate::cli_provider::{self, CliKind};

/// Whether ANY installed CLI bridge has POSITIVE evidence of a login. Distinct from
/// [`CliKind::routable`], which deliberately treats `Unknown` as routable so an unproven bridge
/// still gets a turn: for "does this machine have any way to reach a model at all", an unproven
/// login is not a credential, and answering yes hides the setup guidance from the person who most
/// needs it.
pub fn any_bridge_logged_in() -> bool {
    CliKind::all().into_iter().any(|kind| {
        kind.available()
            && cli_provider::credentials::credentials(kind)
                == cli_provider::credentials::CliCredentials::Present
    })
}

/// Whether `provider` is a CLI bridge that is known to have no credentials right now, so routing
/// can drop it instead of spawning it to be told. Non-bridge providers are never "known absent"
/// here — their key check lives in `forge_config` — so this is a pure filter, not a policy about
/// which providers exist.
pub fn bridge_credentials_known_absent(provider: &str) -> bool {
    CliKind::all().into_iter().any(|kind| {
        kind.prefix() == provider
            && cli_provider::credentials::credentials(kind)
                == cli_provider::credentials::CliCredentials::Absent
    })
}
