//! Key derivation for account sync: opaque record ids, content hashes and the extra seal that
//! keeps API keys out of reach of anything that merely renders the sync history.

use std::collections::BTreeMap;

use anyhow::{anyhow, bail, Context, Result};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use chacha20poly1305::aead::{Aead, Payload};
use chacha20poly1305::{KeyInit, XChaCha20Poly1305, XNonce};
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

use super::super::{decode_base64_array, LocalState};

const ID_INFO: &[u8] = b"forge-account-sync-id-v1";
const VAULT_INFO: &[u8] = b"forge-vault-v1";

/// A value sealed under the per-epoch vault sub-key.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct Sealed {
    pub epoch: u32,
    pub data: String,
}

pub(crate) struct Vault {
    epochs: BTreeMap<u32, [u8; 32]>,
    current: u32,
    hash_key: [u8; 32],
}

fn derive(key: &[u8; 32], info: &[u8]) -> [u8; 32] {
    let mut out = [0_u8; 32];
    Hkdf::<Sha256>::new(None, key)
        .expand(info, &mut out)
        .expect("32 bytes is a valid HKDF-SHA256 output length");
    out
}

fn hmac(key: &[u8; 32], message: &[u8]) -> Vec<u8> {
    let mut mac =
        <Hmac<Sha256> as hmac::KeyInit>::new_from_slice(key).expect("HMAC takes any key length");
    mac.update(message);
    mac.finalize().into_bytes().to_vec()
}

const HASH_KEY_SLOT: &str = "anywhere-sync-hash-key";

/// The key behind local change detection. It is persisted once so a key rotation, or a device that
/// only ever held later epochs, never makes unchanged items look modified. The first value is the
/// historical id key, so devices that already tracked items keep matching their stored hashes.
fn local_hash_key(epochs: &BTreeMap<u32, [u8; 32]>) -> [u8; 32] {
    if let Some(key) = forge_config::load_secret(HASH_KEY_SLOT)
        .and_then(|hex| hex::decode(hex).ok())
        .and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
    {
        return key;
    }
    let seed = epochs
        .values()
        .next()
        .map_or([0; 32], |base| derive(base, ID_INFO));
    let _ = forge_config::store_secret(HASH_KEY_SLOT, &hex::encode(seed));
    seed
}

impl Vault {
    pub fn from_state(state: &LocalState) -> Result<Self> {
        let mut epochs = BTreeMap::new();
        for (epoch, key) in &state.data_key_epochs {
            epochs.insert(*epoch, decode_base64_array::<32>(key, "account data key")?);
        }
        let current = state.key_epoch.context("Anywhere key epoch is missing")?;
        if let Some(key) = state.account_data_key.as_deref() {
            epochs
                .entry(current)
                .or_insert(decode_base64_array::<32>(key, "account data key")?);
        }
        if !epochs.contains_key(&current) {
            bail!("the current Account Data Key epoch is unavailable");
        }
        let hash_key = local_hash_key(&epochs);
        Ok(Self {
            epochs,
            current,
            hash_key,
        })
    }

    /// Ids and hashes use the oldest epoch this device holds. Rotating the Account Data Key must
    /// not rename every record, so the derivation input stays put while sealing moves forward.
    fn id_key(&self) -> [u8; 32] {
        let base = self
            .epochs
            .values()
            .next()
            .expect("vault holds at least one epoch");
        derive(base, ID_INFO)
    }

    pub fn stable_id(&self, category: &str, item: &str) -> String {
        hex::encode(hmac(
            &self.id_key(),
            format!("{category}/{item}").as_bytes(),
        ))
    }

    /// Keyed hash of an item's canonical content, safe to keep in the local database.
    pub fn content_hash(&self, canonical: &str) -> String {
        let id_key = self.hash_key;
        let mut message = b"content:".to_vec();
        message.extend_from_slice(canonical.as_bytes());
        hex::encode(hmac(&id_key, &message))
    }

    fn vault_key(&self, epoch: u32) -> Result<[u8; 32]> {
        let key = self
            .epochs
            .get(&epoch)
            .ok_or_else(|| anyhow!("this device does not hold Account Data Key epoch {epoch}"))?;
        Ok(derive(key, VAULT_INFO))
    }

    pub fn seal(&self, stable_id: &str, plaintext: &[u8]) -> Result<Sealed> {
        let key = self.vault_key(self.current)?;
        let nonce = rand::random::<[u8; 24]>();
        let ciphertext = XChaCha20Poly1305::new((&key).into())
            .encrypt(
                &XNonce::from(nonce),
                Payload {
                    msg: plaintext,
                    aad: stable_id.as_bytes(),
                },
            )
            .map_err(|_| anyhow!("could not seal a synced secret"))?;
        let mut data = nonce.to_vec();
        data.extend_from_slice(&ciphertext);
        Ok(Sealed {
            epoch: self.current,
            data: URL_SAFE_NO_PAD.encode(data),
        })
    }

    pub fn open(&self, stable_id: &str, sealed: &Sealed) -> Result<Vec<u8>> {
        let key = self.vault_key(sealed.epoch)?;
        let raw = URL_SAFE_NO_PAD
            .decode(&sealed.data)
            .context("synced secret is not valid base64")?;
        if raw.len() < 24 {
            bail!("synced secret is truncated");
        }
        let (nonce, ciphertext) = raw.split_at(24);
        let nonce: [u8; 24] = nonce.try_into().expect("split at 24");
        XChaCha20Poly1305::new((&key).into())
            .decrypt(
                &XNonce::from(nonce),
                Payload {
                    msg: ciphertext,
                    aad: stable_id.as_bytes(),
                },
            )
            .map_err(|_| anyhow!("synced secret failed authentication"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vault(epochs: &[(u32, u8)], current: u32) -> Vault {
        Vault {
            epochs: epochs.iter().map(|(e, b)| (*e, [*b; 32])).collect(),
            current,
            hash_key: [1; 32],
        }
    }

    #[test]
    fn ids_agree_across_devices_and_survive_rotation() {
        let a = vault(&[(1, 7)], 1);
        let b = vault(&[(1, 7)], 1);
        assert_eq!(a.stable_id("keys", "openai"), b.stable_id("keys", "openai"));
        assert_ne!(
            a.stable_id("keys", "openai"),
            a.stable_id("skills", "openai")
        );
        let rotated = vault(&[(1, 7), (2, 9)], 2);
        assert_eq!(
            a.stable_id("keys", "openai"),
            rotated.stable_id("keys", "openai")
        );
        assert!(!a.stable_id("keys", "openai").contains("openai"));
    }

    #[test]
    fn a_sealed_value_opens_only_with_its_epoch_and_id() {
        let old = vault(&[(1, 7)], 1);
        let sealed = old.seal("id-1", b"sk-secret").unwrap();
        assert!(!sealed.data.contains("sk-secret"));
        assert_eq!(old.open("id-1", &sealed).unwrap(), b"sk-secret");
        assert!(old.open("id-2", &sealed).is_err(), "bound to its record id");
        let rotated = vault(&[(1, 7), (2, 9)], 2);
        assert_eq!(rotated.open("id-1", &sealed).unwrap(), b"sk-secret");
        let stranger = vault(&[(1, 8)], 1);
        assert!(stranger.open("id-1", &sealed).is_err());
        let resealed = rotated.seal("id-1", b"sk-secret").unwrap();
        assert_eq!(resealed.epoch, 2);
    }
}
