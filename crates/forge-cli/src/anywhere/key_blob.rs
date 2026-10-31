//! The device keys and Account Data Key epochs as stored in the secret store, bound to the
//! device and key epoch they were saved for.

use super::*;

/// Layout of [`KeyBlob`] that records which device and key epoch the keys belong to.
pub(super) const KEY_BLOB_VERSION: u8 = 1;

/// The secret-store copy of the device keys does not belong to the state file it is read beside:
/// it was written for another device or another key epoch. Applying it would label one Account
/// Data Key with another epoch's number, so loading stops instead.
#[derive(Debug)]
pub(super) struct KeyBlobMismatch(pub(super) String);

impl std::fmt::Display for KeyBlobMismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for KeyBlobMismatch {}

#[derive(Serialize, Deserialize, Default)]
pub(super) struct KeyBlob {
    /// 0 for blobs written before the binding existed; they carry no device or epoch.
    #[serde(default)]
    pub(super) version: u8,
    #[serde(default)]
    pub(super) device_id: Option<String>,
    #[serde(default)]
    pub(super) key_epoch: Option<u32>,
    pub(super) signing_private_key: Option<String>,
    pub(super) exchange_private_key: Option<String>,
    pub(super) account_data_key: Option<String>,
    #[serde(default)]
    pub(super) data_key_epochs: BTreeMap<u32, String>,
}

impl KeyBlob {
    pub(super) fn from_state(state: &LocalState) -> Self {
        Self {
            version: KEY_BLOB_VERSION,
            device_id: state.device_id.clone(),
            key_epoch: state.key_epoch,
            signing_private_key: state.signing_private_key.clone(),
            exchange_private_key: state.exchange_private_key.clone(),
            account_data_key: state.account_data_key.clone(),
            data_key_epochs: state.data_key_epochs.clone(),
        }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.signing_private_key.is_none()
            && self.exchange_private_key.is_none()
            && self.account_data_key.is_none()
            && self.data_key_epochs.is_empty()
    }

    /// Whether these keys were saved for the device and key epoch `state` names, and agree with
    /// themselves (the current epoch's entry is the current data key).
    pub(super) fn belongs_to(&self, state: &LocalState) -> bool {
        let coherent = match (self.key_epoch, &self.account_data_key) {
            (Some(epoch), Some(key)) => self
                .data_key_epochs
                .get(&epoch)
                .is_none_or(|entry| entry == key),
            _ => true,
        };
        self.version >= KEY_BLOB_VERSION
            && self.device_id == state.device_id
            && self.key_epoch == state.key_epoch
            && coherent
    }

    /// A blob from before the binding, taken only if it holds the key the state's epoch names.
    /// Without that check a stale copy in one store would be filed under a newer epoch.
    pub(super) fn is_legacy_for(&self, state: &LocalState) -> bool {
        if self.version != 0 {
            return false;
        }
        match state.key_epoch {
            None => true,
            Some(epoch) => {
                self.account_data_key.is_some()
                    && self.data_key_epochs.get(&epoch) == self.account_data_key.as_ref()
            }
        }
    }

    /// A bound blob of the same device that a cut-off save left ahead of the state file: it still
    /// holds the key of the epoch the file names, so rolling the file forward loses nothing.
    pub(super) fn is_ahead_of(&self, state: &LocalState) -> bool {
        match (self.key_epoch, state.key_epoch) {
            (Some(ahead), Some(current)) => {
                self.version >= KEY_BLOB_VERSION
                    && self.device_id == state.device_id
                    && ahead > current
                    && self.data_key_epochs.contains_key(&current)
                    && self.data_key_epochs.get(&ahead) == self.account_data_key.as_ref()
            }
            _ => false,
        }
    }

    pub(super) fn apply(self, state: &mut LocalState) {
        state.signing_private_key = self.signing_private_key;
        state.exchange_private_key = self.exchange_private_key;
        state.account_data_key = self.account_data_key;
        state.data_key_epochs = self.data_key_epochs;
    }
}
