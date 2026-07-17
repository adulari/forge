//! Durable encrypted command metadata and acknowledgement plaintext.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::DeviceId;

/// Maximum encoded size of a durable command or acknowledgement envelope.
pub const MAX_COMMAND_ENVELOPE_BYTES: u64 = 256 * 1024;

/// Lifetime of a queued command and its acknowledgement.
pub const COMMAND_EXPIRY_MS: u64 = 24 * 60 * 60 * 1000;

/// Opaque service-assigned identifier for one durable command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CommandId([u8; 16]);

impl CommandId {
    pub const fn new(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    pub const fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }
}

impl fmt::Display for CommandId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&hex::encode(self.0))
    }
}

impl Serialize for CommandId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&hex::encode(self.0))
    }
}

impl<'de> Deserialize<'de> for CommandId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        if value.len() != 32
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(serde::de::Error::custom(
                "command_id must be 32 lowercase hexadecimal characters",
            ));
        }
        let bytes = hex::decode(value).map_err(serde::de::Error::custom)?;
        let bytes = bytes
            .try_into()
            .map_err(|_| serde::de::Error::custom("command_id must contain exactly 16 bytes"))?;
        Ok(Self(bytes))
    }
}

/// Relay-visible metadata for one queued command.
///
/// Command plaintext is not part of this type and is available only by fetching and decrypting
/// the corresponding binary envelope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QueuedCommandMetadata {
    pub command_id: CommandId,
    #[serde(with = "hex_device_id")]
    pub sender_device_id: DeviceId,
    pub created_at_ms: u64,
    pub expires_at_ms: u64,
    pub ciphertext_bytes: u64,
}

mod hex_device_id {
    use serde::{Deserialize as _, Deserializer, Serializer};

    pub fn serialize<S>(value: &[u8; 16], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&hex::encode(value))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<[u8; 16], D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        if value.len() != 32
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(serde::de::Error::custom(
                "sender_device_id must be 32 lowercase hexadecimal characters",
            ));
        }
        hex::decode(value)
            .map_err(serde::de::Error::custom)?
            .try_into()
            .map_err(|_| serde::de::Error::custom("sender_device_id must contain 16 bytes"))
    }
}

impl QueuedCommandMetadata {
    /// Validate the public queue invariants before trusting service metadata.
    pub fn validate(&self) -> Result<(), CommandMetadataError> {
        let expected_expiry = self
            .created_at_ms
            .checked_add(COMMAND_EXPIRY_MS)
            .ok_or(CommandMetadataError::InvalidExpiry)?;
        if self.expires_at_ms != expected_expiry {
            return Err(CommandMetadataError::InvalidExpiry);
        }
        if self.ciphertext_bytes == 0 || self.ciphertext_bytes > MAX_COMMAND_ENVELOPE_BYTES {
            return Err(CommandMetadataError::InvalidCiphertextSize(
                self.ciphertext_bytes,
            ));
        }
        Ok(())
    }
}

/// Metadata-only response returned when a host lists its queued commands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QueuedCommandList {
    pub version: u8,
    #[serde(default)]
    pub commands: Vec<QueuedCommandMetadata>,
}

impl QueuedCommandList {
    pub fn validate(&self) -> Result<(), CommandMetadataError> {
        self.commands
            .iter()
            .try_for_each(QueuedCommandMetadata::validate)
    }
}

/// Validation failure in relay-visible queued command metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CommandMetadataError {
    #[error("queued command expiry must be exactly 24 hours after creation")]
    InvalidExpiry,
    #[error("queued command ciphertext size {0} is outside the allowed range")]
    InvalidCiphertextSize(u64),
}

/// Decrypted plaintext of a `kind=10` acknowledgement envelope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandAcknowledgement {
    /// Binds this acknowledgement to the queued `kind=9` command.
    pub command_id: CommandId,
    pub result: CommandResult,
}

/// Whether the host accepted the command, without free-form or secret-bearing detail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum CommandResult {
    Success,
    Error {
        code: CommandErrorCode,
        retryable: bool,
    },
}

/// Stable, non-sensitive error categories allowed in command acknowledgements.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandErrorCode {
    InvalidCommand,
    PermissionDenied,
    HostUnavailable,
    ExecutionFailed,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metadata() -> QueuedCommandMetadata {
        QueuedCommandMetadata {
            command_id: CommandId::new([0xab; 16]),
            sender_device_id: [0xcd; 16],
            created_at_ms: 1_750_000_000_000,
            expires_at_ms: 1_750_086_400_000,
            ciphertext_bytes: 4096,
        }
    }

    #[test]
    fn queued_command_list_round_trips_without_plaintext() {
        let list = QueuedCommandList {
            version: 1,
            commands: vec![metadata()],
        };
        let json = serde_json::to_value(&list).expect("serialize command list");

        assert_eq!(
            json["commands"][0]["command_id"],
            "abababababababababababababababab"
        );
        assert!(json["commands"][0].get("plaintext").is_none());
        assert_eq!(
            serde_json::from_value::<QueuedCommandList>(json).expect("deserialize command list"),
            list
        );
        assert_eq!(list.validate(), Ok(()));
    }

    #[test]
    fn queued_command_metadata_rejects_invalid_expiry_and_size() {
        let mut invalid = metadata();
        invalid.expires_at_ms -= 1;
        assert_eq!(invalid.validate(), Err(CommandMetadataError::InvalidExpiry));

        invalid = metadata();
        invalid.ciphertext_bytes = MAX_COMMAND_ENVELOPE_BYTES + 1;
        assert_eq!(
            invalid.validate(),
            Err(CommandMetadataError::InvalidCiphertextSize(
                MAX_COMMAND_ENVELOPE_BYTES + 1
            ))
        );

        invalid.ciphertext_bytes = 0;
        assert_eq!(
            invalid.validate(),
            Err(CommandMetadataError::InvalidCiphertextSize(0))
        );
    }

    #[test]
    fn acknowledgement_variants_round_trip() {
        for acknowledgement in [
            CommandAcknowledgement {
                command_id: CommandId::new([1; 16]),
                result: CommandResult::Success,
            },
            CommandAcknowledgement {
                command_id: CommandId::new([2; 16]),
                result: CommandResult::Error {
                    code: CommandErrorCode::HostUnavailable,
                    retryable: true,
                },
            },
        ] {
            let json = serde_json::to_vec(&acknowledgement).expect("serialize acknowledgement");
            assert_eq!(
                serde_json::from_slice::<CommandAcknowledgement>(&json)
                    .expect("deserialize acknowledgement"),
                acknowledgement
            );
        }
    }

    #[test]
    fn acknowledgement_rejects_secret_bearing_or_malformed_metadata() {
        let with_detail = serde_json::json!({
            "command_id": "01010101010101010101010101010101",
            "result": {
                "status": "error",
                "code": "execution_failed",
                "retryable": false,
                "detail": "must never contain daemon output"
            }
        });
        assert!(serde_json::from_value::<CommandAcknowledgement>(with_detail).is_err());

        let error_without_code = serde_json::json!({
            "command_id": "01010101010101010101010101010101",
            "result": { "status": "error", "retryable": false }
        });
        assert!(serde_json::from_value::<CommandAcknowledgement>(error_without_code).is_err());

        let uppercase_id = serde_json::json!({
            "command_id": "ABABABABABABABABABABABABABABABAB",
            "result": { "status": "success" }
        });
        assert!(serde_json::from_value::<CommandAcknowledgement>(uppercase_id).is_err());
    }
}
