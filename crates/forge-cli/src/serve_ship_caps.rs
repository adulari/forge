//! What `GET /api/identity` says about shipping over the relay.
//!
//! A host that predates the bridged PR routes omits the field, which is how an app tells "update
//! the host" from "turned off on this host".

#[derive(serde::Serialize)]
pub(crate) struct ShipCapability {
    /// The PR reads and the PR Watch read are bridged (always true on a host that reports this).
    pr: bool,
    /// `[anywhere] allow_ship_over_relay`: ship and PR Watch writes are accepted over the relay.
    over_relay: bool,
}

impl ShipCapability {
    pub(crate) fn current() -> Self {
        Self {
            pr: true,
            over_relay: forge_config::load().is_ok_and(|c| c.anywhere.allow_ship_over_relay),
        }
    }
}
