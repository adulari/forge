//! Account data-key epochs and 24-word recovery material.

use bip39::{Language, Mnemonic};

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

/// Recovery phrase validation failures.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RecoveryError {
    #[error("recovery entropy must contain exactly 256 bits")]
    InvalidEntropy,
    #[error("recovery phrase is not a valid English BIP39 mnemonic")]
    InvalidMnemonic,
    #[error("recovery phrase must contain 24 words, got {0}")]
    WrongWordCount(usize),
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
}
