//! Encrypted replay-share plaintext and metadata-only service API types.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Current replay-share payload and API version.
pub const SHARE_VERSION: u8 = 1;
/// Maximum size of one complete encoded `kind=7` envelope.
pub const MAX_SHARE_ENVELOPE_BYTES: u64 = 32 * 1024 * 1024;
/// Allowed share lifetimes, in seconds.
pub const SHARE_EXPIRY_SECONDS: [u64; 3] = [24 * 60 * 60, 7 * 24 * 60 * 60, 30 * 24 * 60 * 60];
/// Media type for exact encrypted Anywhere objects.
pub const ANYWHERE_OBJECT_MEDIA_TYPE: &str = "application/vnd.forge-anywhere";

/// Client-generated opaque identifier for an encrypted replay share.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ShareId([u8; 16]);

impl ShareId {
    pub const fn new(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    pub const fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }
}

impl fmt::Display for ShareId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&hex::encode(self.0))
    }
}

impl Serialize for ShareId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for ShareId {
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
                "share_id must be 32 lowercase hexadecimal characters",
            ));
        }
        let bytes = hex::decode(value).map_err(serde::de::Error::custom)?;
        Ok(Self(bytes.try_into().map_err(|_| {
            serde::de::Error::custom("share_id must contain exactly 16 bytes")
        })?))
    }
}

/// Decrypted plaintext of a `kind=7` envelope.
///
/// `T` is the public Forge replay JSON representation. Keeping the wrapper generic lets clients
/// validate that representation without double-encoding it as a string.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplayShare<T> {
    pub version: u8,
    pub session_id: String,
    pub created_at_ms: u64,
    pub expires_at_ms: u64,
    pub replay: T,
}

/// Authenticated reservation for an immutable encrypted share object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShareReserveRequest {
    pub version: u8,
    pub share_id: ShareId,
    pub ciphertext_bytes: u64,
    #[serde(with = "base64_hash")]
    pub ciphertext_sha256: [u8; 32],
    pub expires_in_seconds: u64,
}

/// Metadata returned after reserving storage for a share.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShareReservation {
    pub version: u8,
    pub share_id: ShareId,
    pub upload_url: String,
    pub expires_at_ms: u64,
}

/// Metadata returned after the service verifies and publishes an uploaded share.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShareCompletion {
    pub version: u8,
    pub share_id: ShareId,
    pub url_path: String,
    pub expires_at_ms: u64,
}

mod base64_hash {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine as _;
    use serde::{Deserialize as _, Deserializer, Serializer};

    pub fn serialize<S>(value: &[u8; 32], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&URL_SAFE_NO_PAD.encode(value))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<[u8; 32], D::Error>
    where
        D: Deserializer<'de>,
    {
        URL_SAFE_NO_PAD
            .decode(String::deserialize(deserializer)?)
            .map_err(serde::de::Error::custom)?
            .try_into()
            .map_err(|_| serde::de::Error::custom("ciphertext_sha256 must contain 32 bytes"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reservation_json_uses_canonical_hex_and_base64url() {
        let request = ShareReserveRequest {
            version: SHARE_VERSION,
            share_id: ShareId::new([0xab; 16]),
            ciphertext_bytes: 4096,
            ciphertext_sha256: [0xff; 32],
            expires_in_seconds: SHARE_EXPIRY_SECONDS[0],
        };
        let json = serde_json::to_value(&request).expect("serialize reservation");
        assert_eq!(json["share_id"], "abababababababababababababababab");
        assert_eq!(
            json["ciphertext_sha256"],
            "__________________________________________8"
        );
        assert_eq!(
            serde_json::from_value::<ShareReserveRequest>(json).expect("decode reservation"),
            request
        );
    }

    #[test]
    fn share_id_rejects_noncanonical_text() {
        let uppercase = serde_json::from_str::<ShareId>("\"ABABABABABABABABABABABABABABABAB\"");
        assert!(uppercase.is_err());
        assert!(serde_json::from_str::<ShareId>("\"abcd\"").is_err());
    }
}
