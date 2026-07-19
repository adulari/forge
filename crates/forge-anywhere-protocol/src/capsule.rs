//! Typed control-plane contract for encrypted workspace handoff capsules.
//!
//! Capsule contents and acknowledgements are `FANY` envelopes. The service only stores the
//! routing fields and hashes below; it never receives workspace or session plaintext.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Current capsule control-plane contract version.
pub const CAPSULE_VERSION: u8 = 1;
/// Hard V1 limit for the encrypted capsule object (100 MiB archive plus envelope overhead).
pub const MAX_CAPSULE_ENVELOPE_BYTES: u64 = 100 * 1024 * 1024 + 4 * 1024;
/// Completed capsules expire if the destination has not consumed them within one day.
pub const CAPSULE_EXPIRY_SECONDS: u64 = 24 * 60 * 60;
/// Signed envelope-header flag indicating that an acknowledgement transfers the session lease.
/// Failure acknowledgements use zero. The service can enforce the lease transition without keys.
pub const CAPSULE_FLAG_ACCEPTED: u16 = 0x0001;

/// Client-generated opaque capsule identifier used as the idempotency identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CapsuleId([u8; 16]);

impl CapsuleId {
    pub const fn new(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    pub const fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }
}

impl fmt::Display for CapsuleId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&hex::encode(self.0))
    }
}

impl std::str::FromStr for CapsuleId {
    type Err = &'static str;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.len() != 32
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err("capsule id must be 32 lowercase hexadecimal characters");
        }
        let bytes = hex::decode(value).map_err(|_| "capsule id contains invalid hexadecimal")?;
        Ok(Self(
            bytes
                .try_into()
                .map_err(|_| "capsule id must contain exactly 16 bytes")?,
        ))
    }
}

impl Serialize for CapsuleId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for CapsuleId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        String::deserialize(deserializer)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

/// Request to reserve storage and an authoritative pending session lease.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapsuleReserveRequest {
    pub version: u8,
    pub capsule_id: CapsuleId,
    pub source_session_id: String,
    pub source_host_id: String,
    pub destination_host_id: String,
    pub ciphertext_bytes: u64,
    pub ciphertext_sha256: String,
}

/// Immutable upload reservation returned by `POST /v1/capsules`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapsuleReservation {
    pub version: u8,
    pub capsule_id: CapsuleId,
    pub upload_url: Option<String>,
    #[serde(default)]
    pub required_headers: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    pub already_complete: bool,
    pub expires_at_ms: u64,
}

/// Integrity proof sent after the exact encrypted object has been uploaded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapsuleCompletion {
    pub version: u8,
    pub ciphertext_bytes: u64,
    pub ciphertext_sha256: String,
}

/// Metadata-only pending item returned to a destination host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingCapsule {
    pub version: u8,
    pub capsule_id: CapsuleId,
    pub source_host_id: String,
    pub source_device_id: String,
    pub key_epoch: u32,
    pub sequence: u64,
    pub ciphertext_bytes: u64,
    pub ciphertext_sha256: String,
    pub expires_at_ms: u64,
}

/// A bounded list of completed capsules waiting for this host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingCapsuleList {
    pub version: u8,
    #[serde(default)]
    pub capsules: Vec<PendingCapsule>,
}

/// One-time download response after the destination claims a capsule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapsuleClaim {
    pub version: u8,
    pub capsule_id: CapsuleId,
    pub download_url: String,
    #[serde(default)]
    pub required_headers: std::collections::BTreeMap<String, String>,
    pub ciphertext_bytes: u64,
    pub ciphertext_sha256: String,
}

/// Categorical destination outcome carried inside an encrypted acknowledgement envelope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapsuleOutcome {
    Accepted,
    BaseUnavailable,
    RepositoryMismatch,
    PatchConflict,
    UnsafeArchive,
    SessionCollision,
    ImportFailed,
}

/// Plaintext acknowledgement. This value must be encrypted before it leaves the destination.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapsuleAcknowledgement {
    pub version: u8,
    pub capsule_id: CapsuleId,
    pub outcome: CapsuleOutcome,
    pub destination_session_id: Option<String>,
    /// Actionable detail is encrypted end-to-end and must not be copied into service metadata.
    pub detail: Option<String>,
}

/// Exact encrypted acknowledgement submitted by the destination host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapsuleAcknowledgeRequest {
    pub version: u8,
    pub acknowledgement_envelope: String,
}

/// Source-visible capsule state. A lease moves only when `state` is `acknowledged` and the
/// decrypted acknowledgement outcome is `accepted`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapsuleStatus {
    pub version: u8,
    pub capsule_id: CapsuleId,
    pub state: String,
    pub acknowledgement_envelope: Option<String>,
    pub acknowledgement_signing_public_key: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acknowledgement_detail_is_only_in_the_encrypted_contract() {
        let acknowledgement = CapsuleAcknowledgement {
            version: CAPSULE_VERSION,
            capsule_id: CapsuleId::new([0; 16]),
            outcome: CapsuleOutcome::PatchConflict,
            destination_session_id: None,
            detail: Some("src/private.rs conflicts".into()),
        };
        let plaintext = serde_json::to_vec(&acknowledgement).expect("serialize acknowledgement");
        assert!(String::from_utf8_lossy(&plaintext).contains("src/private.rs"));

        let request = CapsuleAcknowledgeRequest {
            version: CAPSULE_VERSION,
            acknowledgement_envelope: "opaque-fany".into(),
        };
        let control = serde_json::to_string(&request).expect("serialize control request");
        assert!(!control.contains("private.rs"));
    }

    #[test]
    fn reservation_round_trip_preserves_exact_integrity_fields() {
        let request = CapsuleReserveRequest {
            version: CAPSULE_VERSION,
            capsule_id: CapsuleId::new([0xab; 16]),
            source_session_id: "session-a".into(),
            source_host_id: "host-a".into(),
            destination_host_id: "host-b".into(),
            ciphertext_bytes: 42,
            ciphertext_sha256: "cd".repeat(32),
        };
        let encoded = serde_json::to_vec(&request).expect("serialize reservation");
        let decoded: CapsuleReserveRequest =
            serde_json::from_slice(&encoded).expect("deserialize reservation");
        assert_eq!(decoded, request);
    }

    #[test]
    fn capsule_id_rejects_noncanonical_text() {
        assert!(serde_json::from_str::<CapsuleId>("\"ABABABABABABABABABABABABABABABAB\"").is_err());
        assert!(serde_json::from_str::<CapsuleId>("\"abcd\"").is_err());
        assert_eq!(CapsuleId::new([0xab; 16]).to_string(), "ab".repeat(16));
    }
}
