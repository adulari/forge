//! Account data-key epochs and versioned recovery material.

use bip39::{Language, Mnemonic};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::crypto::SecretKey;
use crate::DeviceId;

/// The current account data key and its monotonically increasing epoch number.
#[derive(Debug, Clone)]
pub struct KeyEpoch {
    pub epoch: u32,
    pub data_key: SecretKey,
}

/// An encrypted data-key epoch addressed to one authorized target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WrappedEpochKey {
    pub epoch: u32,
    pub recipient_device_id: Option<DeviceId>,
    pub envelope: Vec<u8>,
}

/// A 256-bit recovery secret represented to the user as exactly 24 BIP39 English words.
#[derive(Debug, Clone)]
pub struct RecoverySecret(SecretKey);

impl RecoverySecret {
    /// Encode 32 bytes of platform-generated entropy as a 24-word mnemonic.
    pub fn from_entropy(entropy: [u8; 32]) -> Result<Self, RecoveryError> {
        Mnemonic::from_entropy_in(Language::English, &entropy)
            .map_err(|_| RecoveryError::InvalidEntropy)?;
        Ok(Self(SecretKey::from_bytes(entropy)))
    }

    /// Parse and checksum-validate a 24-word English mnemonic.
    pub fn from_words(words: &str) -> Result<Self, RecoveryError> {
        let mnemonic = Mnemonic::parse_in_normalized(Language::English, words)
            .map_err(|_| RecoveryError::InvalidMnemonic)?;
        if mnemonic.word_count() != 24 {
            return Err(RecoveryError::WrongWordCount(mnemonic.word_count()));
        }
        let entropy = mnemonic.to_entropy();
        let bytes: [u8; 32] = entropy
            .try_into()
            .map_err(|_| RecoveryError::InvalidEntropy)?;
        Ok(Self(SecretKey::from_bytes(bytes)))
    }

    /// Render the checksum-protected 24-word phrase for one-time display.
    pub fn words(&self) -> Result<String, RecoveryError> {
        Mnemonic::from_entropy_in(Language::English, self.0.as_bytes())
            .map(|mnemonic| mnemonic.to_string())
            .map_err(|_| RecoveryError::InvalidEntropy)
    }

    /// Borrow the underlying 256-bit secret for domain-separated key derivation.
    pub fn as_bytes(&self) -> &[u8; 32] {
        self.0.as_bytes()
    }
}

/// A v2 128-bit recovery bearer secret represented as 12 checksum-protected English words.
#[derive(Debug, Clone)]
pub struct RecoverySecretV2([u8; 16]);

impl RecoverySecretV2 {
    /// Validate platform-generated entropy and retain it in the v2 representation.
    pub fn from_entropy(entropy: [u8; 16]) -> Result<Self, RecoveryError> {
        Mnemonic::from_entropy_in(Language::English, &entropy)
            .map_err(|_| RecoveryError::InvalidV2Entropy)?;
        Ok(Self(entropy))
    }

    /// Parse and checksum-validate a 12-word English mnemonic.
    pub fn from_words(words: &str) -> Result<Self, RecoveryError> {
        let mnemonic = Mnemonic::parse_in_normalized(Language::English, words)
            .map_err(|_| RecoveryError::InvalidMnemonic)?;
        if mnemonic.word_count() != 12 {
            return Err(RecoveryError::WrongWordCount(mnemonic.word_count()));
        }
        let entropy: [u8; 16] = mnemonic
            .to_entropy()
            .try_into()
            .map_err(|_| RecoveryError::InvalidV2Entropy)?;
        Ok(Self(entropy))
    }

    /// Render the checksum-protected 12-word phrase.
    pub fn words(&self) -> Result<String, RecoveryError> {
        Mnemonic::from_entropy_in(Language::English, &self.0)
            .map(|mnemonic| mnemonic.to_string())
            .map_err(|_| RecoveryError::InvalidV2Entropy)
    }

    /// Borrow the v2 bearer entropy for its account-bound HKDF.
    pub fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }
}

/// Machine-readable v2 Recovery Kit payload used by `.forge-recovery` files and QR codes.
///
/// The checksum detects corruption; it is not an authenticator. Possession of `words` grants
/// recovery, so callers must handle the serialized value as a secret.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RecoveryKitV2 {
    version: u8,
    service: String,
    account_id: String,
    words: String,
    checksum: String,
}

impl RecoveryKitV2 {
    /// Build an account- and service-bound kit from a v2 recovery secret.
    pub fn new(
        secret: &RecoverySecretV2,
        service: &str,
        account_id: &crate::AccountId,
    ) -> Result<Self, RecoveryError> {
        let service = normalize_service(service)?;
        let words = secret.words()?;
        Ok(Self {
            version: 2,
            checksum: kit_checksum(&service, account_id, secret.as_bytes()),
            service,
            account_id: hex::encode(account_id),
            words,
        })
    }

    /// Serialize the portable kit. The same UTF-8 payload can be encoded in a QR code.
    pub fn to_json(&self) -> Result<String, RecoveryError> {
        serde_json::to_string_pretty(self).map_err(|_| RecoveryError::MalformedKit)
    }

    /// Parse a kit, verify its corruption checksum, and enforce its account/service bindings.
    pub fn from_json(
        json: &str,
        expected_service: &str,
        expected_account_id: &crate::AccountId,
    ) -> Result<(Self, RecoverySecretV2), RecoveryError> {
        let kit: Self = serde_json::from_str(json).map_err(|_| RecoveryError::MalformedKit)?;
        if kit.version != 2 {
            return Err(RecoveryError::UnsupportedKitVersion(kit.version));
        }
        let expected_service = normalize_service(expected_service)?;
        if kit.service != expected_service {
            return Err(RecoveryError::WrongService);
        }
        if kit.account_id != hex::encode(expected_account_id) {
            return Err(RecoveryError::WrongAccount);
        }
        let secret = RecoverySecretV2::from_words(&kit.words)?;
        let checksum = kit_checksum(&kit.service, expected_account_id, secret.as_bytes());
        if kit.checksum != checksum {
            return Err(RecoveryError::CorruptKit);
        }
        Ok((kit, secret))
    }

    /// The human-readable phrase carried by this bearer-secret document.
    pub fn words(&self) -> &str {
        &self.words
    }
}

fn normalize_service(service: &str) -> Result<String, RecoveryError> {
    let service = service.trim().trim_end_matches('/');
    if service.is_empty() {
        return Err(RecoveryError::MalformedKit);
    }
    Ok(service.to_owned())
}

fn kit_checksum(service: &str, account_id: &crate::AccountId, entropy: &[u8; 16]) -> String {
    let mut hash = Sha256::new();
    hash.update(b"forge-anywhere/v2/recovery-kit-checksum\0");
    hash.update((service.len() as u64).to_be_bytes());
    hash.update(service.as_bytes());
    hash.update(account_id);
    hash.update(entropy);
    hex::encode(hash.finalize())
}

/// Recovery phrase validation failures.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RecoveryError {
    #[error("recovery entropy must contain exactly 256 bits")]
    InvalidEntropy,
    #[error("v2 recovery entropy must contain exactly 128 bits")]
    InvalidV2Entropy,
    #[error("recovery phrase is not a valid English BIP39 mnemonic")]
    InvalidMnemonic,
    #[error("recovery phrase has an unsupported word count: {0}")]
    WrongWordCount(usize),
    #[error("recovery kit is malformed")]
    MalformedKit,
    #[error("recovery kit version {0} is unsupported")]
    UnsupportedKitVersion(u8),
    #[error("recovery kit belongs to another Forge service")]
    WrongService,
    #[error("recovery kit belongs to another account")]
    WrongAccount,
    #[error("recovery kit is corrupt")]
    CorruptKit,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovery_words_round_trip() {
        let secret = RecoverySecret::from_entropy([0x42; 32]).expect("valid entropy");
        let words = secret.words().expect("encode words");
        assert_eq!(words.split_whitespace().count(), 24);
        let parsed = RecoverySecret::from_words(&words).expect("parse words");
        assert_eq!(parsed.as_bytes(), secret.as_bytes());
    }

    #[test]
    fn recovery_v2_words_and_file_round_trip() {
        let secret = RecoverySecretV2::from_entropy([0x42; 16]).expect("valid entropy");
        let words = secret.words().expect("encode words");
        assert_eq!(words.split_whitespace().count(), 12);
        let parsed = RecoverySecretV2::from_words(&words).expect("parse words");
        assert_eq!(parsed.as_bytes(), secret.as_bytes());

        let account = [0x33; 16];
        let kit =
            RecoveryKitV2::new(&secret, "https://app.forge.test/", &account).expect("create kit");
        let json = kit.to_json().expect("serialize kit");
        let (_, decoded) =
            RecoveryKitV2::from_json(&json, "https://app.forge.test", &account).expect("parse kit");
        assert_eq!(decoded.as_bytes(), secret.as_bytes());
    }

    #[test]
    fn recovery_v2_rejects_wrong_account_and_corruption() {
        let secret = RecoverySecretV2::from_entropy([0x42; 16]).expect("valid entropy");
        let account = [0x33; 16];
        let kit =
            RecoveryKitV2::new(&secret, "https://app.forge.test", &account).expect("create kit");
        let json = kit.to_json().expect("serialize kit");
        assert_eq!(
            RecoveryKitV2::from_json(&json, "https://app.forge.test", &[0x34; 16])
                .expect_err("wrong account"),
            RecoveryError::WrongAccount
        );

        let mut corrupted_value: serde_json::Value =
            serde_json::from_str(&json).expect("valid kit json");
        corrupted_value["checksum"] = serde_json::Value::String("00".repeat(32));
        let corrupted = serde_json::to_string(&corrupted_value).expect("corrupt kit json");
        assert!(RecoveryKitV2::from_json(&corrupted, "https://app.forge.test", &account).is_err());
    }
}
