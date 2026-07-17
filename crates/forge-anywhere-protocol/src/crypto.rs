//! X25519 exchange and domain-separated HKDF-SHA256 derivation.

use hkdf::Hkdf;
use sha2::Sha256;
use x25519_dalek::{PublicKey, StaticSecret};
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::AccountId;

const DEVICE_WRAP_CONTEXT: &[u8] = b"forge-anywhere/v1/device-wrap";
const RECOVERY_WRAP_CONTEXT: &[u8] = b"forge-anywhere/v1/recovery-wrap";

/// A secret 256-bit symmetric key that is zeroed when dropped.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct SecretKey([u8; 32]);

impl SecretKey {
    /// Construct a key from cryptographically random bytes supplied by the platform caller.
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Borrow key bytes without copying them.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl std::fmt::Debug for SecretKey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SecretKey([REDACTED])")
    }
}

/// Return the X25519 public key corresponding to a private scalar.
pub fn exchange_public_key(private_key: &[u8; 32]) -> [u8; 32] {
    PublicKey::from(&StaticSecret::from(*private_key)).to_bytes()
}

/// Derive the key used to wrap one account-data-key epoch to another device.
pub fn derive_device_wrap_key(
    private_key: &[u8; 32],
    peer_public_key: &[u8; 32],
    account_id: &AccountId,
    key_epoch: u32,
) -> Result<SecretKey, CryptoError> {
    let private = StaticSecret::from(*private_key);
    let peer = PublicKey::from(*peer_public_key);
    let shared = private.diffie_hellman(&peer);
    if !shared.was_contributory() {
        return Err(CryptoError::NonContributoryExchange);
    }
    let mut context = Vec::with_capacity(DEVICE_WRAP_CONTEXT.len() + 4);
    context.extend_from_slice(DEVICE_WRAP_CONTEXT);
    context.extend_from_slice(&key_epoch.to_be_bytes());
    derive_key(shared.as_bytes(), account_id, &context)
}

/// Derive the key used to wrap one account-data-key epoch to the recovery secret.
pub fn derive_recovery_wrap_key(
    recovery_secret: &[u8; 32],
    account_id: &AccountId,
    key_epoch: u32,
) -> Result<SecretKey, CryptoError> {
    let mut context = Vec::with_capacity(RECOVERY_WRAP_CONTEXT.len() + 4);
    context.extend_from_slice(RECOVERY_WRAP_CONTEXT);
    context.extend_from_slice(&key_epoch.to_be_bytes());
    derive_key(recovery_secret, account_id, &context)
}

fn derive_key(
    input_key_material: &[u8],
    salt: &[u8],
    context: &[u8],
) -> Result<SecretKey, CryptoError> {
    let hkdf = Hkdf::<Sha256>::new(Some(salt), input_key_material);
    let mut output = [0_u8; 32];
    hkdf.expand(context, &mut output)
        .map_err(|_| CryptoError::InvalidDerivationLength)?;
    Ok(SecretKey::from_bytes(output))
}

/// Key exchange and derivation failures.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CryptoError {
    #[error("peer supplied a non-contributory X25519 public key")]
    NonContributoryExchange,
    #[error("invalid HKDF output length")]
    InvalidDerivationLength,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_devices_derive_the_same_wrap_key() {
        let alice = [0x11; 32];
        let bob = [0x22; 32];
        let account = [0x33; 16];
        let alice_key = derive_device_wrap_key(&alice, &exchange_public_key(&bob), &account, 9)
            .expect("alice derivation");
        let bob_key = derive_device_wrap_key(&bob, &exchange_public_key(&alice), &account, 9)
            .expect("bob derivation");
        assert_eq!(alice_key.as_bytes(), bob_key.as_bytes());
    }

    #[test]
    fn epoch_is_domain_separated() {
        let key = [0x11; 32];
        let peer = exchange_public_key(&[0x22; 32]);
        let first = derive_device_wrap_key(&key, &peer, &[0x33; 16], 1).expect("first epoch");
        let second = derive_device_wrap_key(&key, &peer, &[0x33; 16], 2).expect("second epoch");
        assert_ne!(first.as_bytes(), second.as_bytes());
    }
}
