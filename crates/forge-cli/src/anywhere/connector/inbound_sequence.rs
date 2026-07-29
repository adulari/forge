//! Host-target admission and replay-safe acceptance for inbound relay envelopes.

use super::{Identity, LocalState, StateStore};
use anyhow::{bail, Result};
use forge_anywhere_protocol::{Envelope, RecipientKind};

/// Validate envelope routing without requiring the host's current key epoch.
/// Durable queued commands may legitimately target an older retained account-data-key epoch.
pub(super) fn validate_inbound_routing(envelope: &Envelope, identity: &Identity) -> Result<()> {
    let metadata = &envelope.metadata;
    if metadata.account_id != identity.account_id {
        bail!("relay envelope account does not match this host");
    }
    if metadata.recipient_kind != RecipientKind::Host || metadata.recipient_id != identity.host_id {
        bail!("relay envelope is not addressed to this host");
    }
    Ok(())
}

/// Validate that an envelope is addressed to this host and its active key epoch.
pub(super) fn validate_inbound_metadata(envelope: &Envelope, identity: &Identity) -> Result<()> {
    validate_inbound_routing(envelope, identity)?;
    if envelope.metadata.key_epoch != identity.key_epoch {
        bail!("relay envelope uses an unavailable account key epoch");
    }
    Ok(())
}

pub(super) fn accept_inbound_envelopes(
    store: &StateStore,
    sender_device_id: [u8; 16],
    key_epoch: u32,
    blob_sequence: Option<u64>,
    control_sequence: u64,
) -> Result<()> {
    store.update(|state| {
        let first_sequence = blob_sequence.unwrap_or(control_sequence);
        if blob_sequence.is_some_and(|sequence| sequence >= control_sequence) {
            bail!("relay blob sequence must precede its referencing envelope");
        }
        accept_inbound_sequence(state, sender_device_id, key_epoch, first_sequence)?;
        let namespace = format!("{}:{key_epoch}", hex::encode(sender_device_id));
        state.accepted_sequences.insert(namespace, control_sequence);
        Ok(())
    })?;
    Ok(())
}

fn accept_inbound_sequence(
    state: &mut LocalState,
    sender_device_id: [u8; 16],
    key_epoch: u32,
    sequence: u64,
) -> Result<()> {
    let namespace = format!("{}:{key_epoch}", hex::encode(sender_device_id));
    if state
        .accepted_sequences
        .get(&namespace)
        .is_some_and(|last| sequence <= *last)
    {
        bail!("replayed or out-of-order relay sequence");
    }
    state.accepted_sequences.insert(namespace, sequence);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn blob_and_control_sequences_are_accepted_atomically() {
        let dir = tempdir().unwrap();
        let store = StateStore {
            path: dir.path().join("state.json"),
        };
        accept_inbound_envelopes(&store, [0x77; 16], 3, Some(10), 11).unwrap();
        // The rejected pair must not commit its blob sequence. A fresh control sequence can still
        // accept that blob after the failed atomic update.
        assert!(accept_inbound_envelopes(&store, [0x77; 16], 3, Some(14), 14).is_err());
        accept_inbound_envelopes(&store, [0x77; 16], 3, Some(14), 15).unwrap();
        let reloaded = StateStore {
            path: dir.path().join("state.json"),
        };
        assert!(accept_inbound_envelopes(&reloaded, [0x77; 16], 3, Some(14), 16).is_err());
    }
}
