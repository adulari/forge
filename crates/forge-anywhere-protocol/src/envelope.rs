//! Canonical `FANY` binary envelope encoding and authenticated encryption.

use chacha20poly1305::aead::{Aead, Payload};
use chacha20poly1305::{KeyInit, XChaCha20Poly1305, XNonce};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};

use crate::{AccountId, DeviceId, RecipientId};

/// Four-byte discriminator at the start of every Anywhere object.
pub const MAGIC: [u8; 4] = *b"FANY";
/// Current Anywhere protocol version.
pub const VERSION: u8 = 1;
/// Number of bytes in the fixed authenticated header.
pub const HEADER_LEN: usize = 105;
/// Number of bytes in an Ed25519 signature.
pub const SIGNATURE_LEN: usize = 64;
/// XChaCha20-Poly1305 authentication tag length.
pub const TAG_LEN: usize = 16;

/// The encrypted object's semantic type. Values are stable on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum EnvelopeKind {
    BridgeRequest = 1,
    BridgeResponse = 2,
    WebSocketFrame = 3,
    SyncRecord = 4,
    KeyWrap = 5,
    Capsule = 6,
    Share = 7,
    Blob = 8,
    Command = 9,
    Acknowledgement = 10,
}

impl TryFrom<u8> for EnvelopeKind {
    type Error = EnvelopeError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::BridgeRequest),
            2 => Ok(Self::BridgeResponse),
            3 => Ok(Self::WebSocketFrame),
            4 => Ok(Self::SyncRecord),
            5 => Ok(Self::KeyWrap),
            6 => Ok(Self::Capsule),
            7 => Ok(Self::Share),
            8 => Ok(Self::Blob),
            9 => Ok(Self::Command),
            10 => Ok(Self::Acknowledgement),
            other => Err(EnvelopeError::UnknownKind(other)),
        }
    }
}

/// How the relay interprets `recipient_id`. Values are stable on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum RecipientKind {
    Device = 1,
    Host = 2,
    Account = 3,
    Share = 4,
}

impl TryFrom<u8> for RecipientKind {
    type Error = EnvelopeError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::Device),
            2 => Ok(Self::Host),
            3 => Ok(Self::Account),
            4 => Ok(Self::Share),
            other => Err(EnvelopeError::UnknownRecipientKind(other)),
        }
    }
}

/// Cleartext routing metadata authenticated as AAD and covered by the sender signature.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvelopeMetadata {
    pub kind: EnvelopeKind,
    pub flags: u16,
    pub account_id: AccountId,
    pub sender_device_id: DeviceId,
    pub recipient_kind: RecipientKind,
    pub recipient_id: RecipientId,
    pub key_epoch: u32,
    pub sequence: u64,
    pub created_at_ms: u64,
    pub nonce: [u8; 24],
}

/// A decoded envelope. Ciphertext remains opaque until [`Envelope::open`] succeeds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Envelope {
    pub metadata: EnvelopeMetadata,
    ciphertext: Vec<u8>,
    signature: [u8; SIGNATURE_LEN],
}

/// Strict envelope parsing, authentication, and decryption failures.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum EnvelopeError {
    #[error("envelope is shorter than the v1 minimum")]
    TooShort,
    #[error("invalid envelope magic")]
    InvalidMagic,
    #[error("unsupported envelope version {0}")]
    UnsupportedVersion(u8),
    #[error("unknown envelope kind {0}")]
    UnknownKind(u8),
    #[error("unknown recipient kind {0}")]
    UnknownRecipientKind(u8),
    #[error("ciphertext length is invalid")]
    InvalidCiphertextLength,
    #[error("sender signature is invalid")]
    InvalidSignature,
    #[error("payload authentication failed")]
    AuthenticationFailed,
    #[error("plaintext is too large for the v1 envelope")]
    PayloadTooLarge,
}

impl Envelope {
    /// Encrypt and sign a payload using a caller-supplied nonce.
    ///
    /// Nonces must be unique for a given encryption key. Production callers should generate a
    /// fresh random nonce; accepting it here makes cross-language golden vectors deterministic.
    pub fn seal(
        metadata: EnvelopeMetadata,
        plaintext: &[u8],
        encryption_key: &[u8; 32],
        signing_key: &SigningKey,
    ) -> Result<Self, EnvelopeError> {
        let ciphertext_len = plaintext
            .len()
            .checked_add(TAG_LEN)
            .and_then(|len| u32::try_from(len).ok())
            .ok_or(EnvelopeError::PayloadTooLarge)?;
        let header = encode_header(&metadata, ciphertext_len);
        let cipher = XChaCha20Poly1305::new(encryption_key.into());
        let nonce = XNonce::from(metadata.nonce);
        let ciphertext = cipher
            .encrypt(
                &nonce,
                Payload {
                    msg: plaintext,
                    aad: &header,
                },
            )
            .map_err(|_| EnvelopeError::AuthenticationFailed)?;
        let signed = signed_bytes(&header, &ciphertext);
        let signature = signing_key.sign(&signed).to_bytes();
        Ok(Self {
            metadata,
            ciphertext,
            signature,
        })
    }

    /// Parse a canonical v1 envelope without decrypting it.
    pub fn decode(bytes: &[u8]) -> Result<Self, EnvelopeError> {
        if bytes.len() < HEADER_LEN + TAG_LEN + SIGNATURE_LEN {
            return Err(EnvelopeError::TooShort);
        }
        if bytes[..4] != MAGIC {
            return Err(EnvelopeError::InvalidMagic);
        }
        if bytes[4] != VERSION {
            return Err(EnvelopeError::UnsupportedVersion(bytes[4]));
        }

        let ciphertext_len = u32::from_be_bytes(
            bytes[101..105]
                .try_into()
                .map_err(|_| EnvelopeError::TooShort)?,
        ) as usize;
        if ciphertext_len < TAG_LEN
            || HEADER_LEN
                .checked_add(ciphertext_len)
                .and_then(|len| len.checked_add(SIGNATURE_LEN))
                != Some(bytes.len())
        {
            return Err(EnvelopeError::InvalidCiphertextLength);
        }

        let metadata = EnvelopeMetadata {
            kind: EnvelopeKind::try_from(bytes[5])?,
            flags: u16::from_be_bytes(
                bytes[6..8]
                    .try_into()
                    .map_err(|_| EnvelopeError::TooShort)?,
            ),
            account_id: bytes[8..24]
                .try_into()
                .map_err(|_| EnvelopeError::TooShort)?,
            sender_device_id: bytes[24..40]
                .try_into()
                .map_err(|_| EnvelopeError::TooShort)?,
            recipient_kind: RecipientKind::try_from(bytes[40])?,
            recipient_id: bytes[41..57]
                .try_into()
                .map_err(|_| EnvelopeError::TooShort)?,
            key_epoch: u32::from_be_bytes(
                bytes[57..61]
                    .try_into()
                    .map_err(|_| EnvelopeError::TooShort)?,
            ),
            sequence: u64::from_be_bytes(
                bytes[61..69]
                    .try_into()
                    .map_err(|_| EnvelopeError::TooShort)?,
            ),
            created_at_ms: u64::from_be_bytes(
                bytes[69..77]
                    .try_into()
                    .map_err(|_| EnvelopeError::TooShort)?,
            ),
            nonce: bytes[77..101]
                .try_into()
                .map_err(|_| EnvelopeError::TooShort)?,
        };
        let ciphertext_end = HEADER_LEN + ciphertext_len;
        let ciphertext = bytes[HEADER_LEN..ciphertext_end].to_vec();
        let signature = bytes[ciphertext_end..]
            .try_into()
            .map_err(|_| EnvelopeError::InvalidSignature)?;
        Ok(Self {
            metadata,
            ciphertext,
            signature,
        })
    }

    /// Verify the sender signature without requiring the payload encryption key.
    pub fn verify(&self, verifying_key: &VerifyingKey) -> Result<(), EnvelopeError> {
        let header = encode_header(
            &self.metadata,
            u32::try_from(self.ciphertext.len())
                .map_err(|_| EnvelopeError::InvalidCiphertextLength)?,
        );
        let signed = signed_bytes(&header, &self.ciphertext);
        let signature = Signature::from_bytes(&self.signature);
        verifying_key
            .verify_strict(&signed, &signature)
            .map_err(|_| EnvelopeError::InvalidSignature)
    }

    /// Verify the signature, authenticate the fixed header, and decrypt the payload.
    pub fn open(
        &self,
        encryption_key: &[u8; 32],
        verifying_key: &VerifyingKey,
    ) -> Result<Vec<u8>, EnvelopeError> {
        self.verify(verifying_key)?;
        let ciphertext_len = u32::try_from(self.ciphertext.len())
            .map_err(|_| EnvelopeError::InvalidCiphertextLength)?;
        let header = encode_header(&self.metadata, ciphertext_len);
        let cipher = XChaCha20Poly1305::new(encryption_key.into());
        let nonce = XNonce::from(self.metadata.nonce);
        cipher
            .decrypt(
                &nonce,
                Payload {
                    msg: &self.ciphertext,
                    aad: &header,
                },
            )
            .map_err(|_| EnvelopeError::AuthenticationFailed)
    }

    /// Encode the canonical wire representation.
    pub fn encode(&self) -> Result<Vec<u8>, EnvelopeError> {
        let ciphertext_len = u32::try_from(self.ciphertext.len())
            .map_err(|_| EnvelopeError::InvalidCiphertextLength)?;
        let header = encode_header(&self.metadata, ciphertext_len);
        let mut bytes = Vec::with_capacity(HEADER_LEN + self.ciphertext.len() + SIGNATURE_LEN);
        bytes.extend_from_slice(&header);
        bytes.extend_from_slice(&self.ciphertext);
        bytes.extend_from_slice(&self.signature);
        Ok(bytes)
    }

    /// Encrypted payload bytes including the Poly1305 tag.
    pub fn ciphertext(&self) -> &[u8] {
        &self.ciphertext
    }

    /// Sender signature bytes.
    pub fn signature(&self) -> &[u8; SIGNATURE_LEN] {
        &self.signature
    }
}

fn encode_header(metadata: &EnvelopeMetadata, ciphertext_len: u32) -> [u8; HEADER_LEN] {
    let mut header = [0_u8; HEADER_LEN];
    header[..4].copy_from_slice(&MAGIC);
    header[4] = VERSION;
    header[5] = metadata.kind as u8;
    header[6..8].copy_from_slice(&metadata.flags.to_be_bytes());
    header[8..24].copy_from_slice(&metadata.account_id);
    header[24..40].copy_from_slice(&metadata.sender_device_id);
    header[40] = metadata.recipient_kind as u8;
    header[41..57].copy_from_slice(&metadata.recipient_id);
    header[57..61].copy_from_slice(&metadata.key_epoch.to_be_bytes());
    header[61..69].copy_from_slice(&metadata.sequence.to_be_bytes());
    header[69..77].copy_from_slice(&metadata.created_at_ms.to_be_bytes());
    header[77..101].copy_from_slice(&metadata.nonce);
    header[101..105].copy_from_slice(&ciphertext_len.to_be_bytes());
    header
}

fn signed_bytes(header: &[u8; HEADER_LEN], ciphertext: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(HEADER_LEN + ciphertext.len());
    bytes.extend_from_slice(header);
    bytes.extend_from_slice(ciphertext);
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metadata() -> EnvelopeMetadata {
        EnvelopeMetadata {
            kind: EnvelopeKind::BridgeRequest,
            flags: 0x0102,
            account_id: [0x11; 16],
            sender_device_id: [0x22; 16],
            recipient_kind: RecipientKind::Host,
            recipient_id: [0x33; 16],
            key_epoch: 7,
            sequence: 42,
            created_at_ms: 1_750_000_000_123,
            nonce: [0x44; 24],
        }
    }

    #[test]
    fn round_trip_and_tamper_detection() {
        let signing_key = SigningKey::from_bytes(&[0x55; 32]);
        let encryption_key = [0x66; 32];
        let envelope = Envelope::seal(
            metadata(),
            b"typed bridge payload",
            &encryption_key,
            &signing_key,
        )
        .expect("seal fixture");
        let encoded = envelope.encode().expect("encode fixture");
        let decoded = Envelope::decode(&encoded).expect("decode fixture");
        assert_eq!(
            decoded.open(&encryption_key, &signing_key.verifying_key()),
            Ok(b"typed bridge payload".to_vec())
        );

        let mut tampered = encoded;
        tampered[HEADER_LEN] ^= 1;
        let decoded = Envelope::decode(&tampered).expect("tampered envelope remains parseable");
        assert_eq!(
            decoded.verify(&signing_key.verifying_key()),
            Err(EnvelopeError::InvalidSignature)
        );
    }

    #[test]
    fn wrong_key_and_corrupted_header_fail_closed() {
        let signing_key = SigningKey::from_bytes(&[0x55; 32]);
        let envelope =
            Envelope::seal(metadata(), b"secret", &[0x66; 32], &signing_key).expect("seal fixture");
        assert_eq!(
            envelope.open(&[0x67; 32], &signing_key.verifying_key()),
            Err(EnvelopeError::AuthenticationFailed)
        );

        let mut encoded = envelope.encode().expect("encode fixture");
        encoded[61] ^= 1;
        let decoded = Envelope::decode(&encoded).expect("corrupt header remains parseable");
        assert_eq!(
            decoded.verify(&signing_key.verifying_key()),
            Err(EnvelopeError::InvalidSignature)
        );
    }
}
