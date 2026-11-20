//! Negotiated encoding of the JSON payloads sealed inside bridge and stream envelopes.
//!
//! Wire v1 writes every byte field as a JSON array of decimal numbers, which costs ~3.57x on
//! random bytes and ~3.5x on JSON text. Wire v2 writes the same fields as unpadded base64url
//! strings (1.33x). Nothing else changes: the `FANY` envelope, its header, the AEAD and the
//! signature are byte-for-byte the same, and the relay service never sees the plaintext.
//!
//! Negotiation is per message pair, so an old peer never receives a form it cannot parse:
//! - Every reader accepts both forms for every byte field (see [`bytes`] and [`fixed`]).
//! - A sender adds `"wire": N` to bridge requests and responses: the highest version it reads.
//!   Old peers ignore the unknown field (the bridge types never denied unknown fields).
//! - A host answers a request, and the stream it opens, in v2 only when the request advertised
//!   v2. A controller sends v2 only after a response from that host advertised v2.
//!
//! Durable commands and sync records are stored by the service and read later by peers whose
//! version nobody negotiated, so their writers stay on v1; their readers accept both.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use serde::de::{self, Deserializer, SeqAccess, Visitor};
use serde::ser::Serializer;
use serde::{Deserialize, Serialize};

use crate::bridge::{
    BridgeRequest, BridgeResponse, FrameDirection, RelayBlobReference, RouteId, WebSocketFrame,
    WebSocketFrameKind,
};

/// Byte fields are JSON arrays of numbers. Every released peer before wire negotiation.
pub const WIRE_V1: u8 = 1;
/// Byte fields are unpadded base64url strings.
pub const WIRE_V2: u8 = 2;
/// The highest version this build reads and advertises.
pub const WIRE_LATEST: u8 = WIRE_V2;

/// Largest body a v1 sender keeps inline: 64 KiB seals to ~229 KiB under the relay's 300 KiB cap.
pub const MAX_OUTBOUND_INLINE_V1: usize = 64 * 1024;
/// Largest body a v2 sender keeps inline: 192 KiB encodes to 256 KiB, still under the 300 KiB
/// relay cap with room for headers, and under every receiver's 256 KiB raw inline limit.
pub const MAX_OUTBOUND_INLINE_V2: usize = 192 * 1024;

/// The encoding chosen for one message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Wire {
    #[default]
    V1,
    V2,
}

impl Wire {
    /// The encoding to use towards a peer that advertised `peer` (0 or absent means v1).
    pub fn negotiate(peer: u8) -> Self {
        if peer >= WIRE_V2 {
            Self::V2
        } else {
            Self::V1
        }
    }

    /// The largest body to send inline before offloading it to a relay blob.
    pub fn max_outbound_inline(self) -> usize {
        match self {
            Self::V1 => MAX_OUTBOUND_INLINE_V1,
            Self::V2 => MAX_OUTBOUND_INLINE_V2,
        }
    }
}

/// The wire version a decoded bridge request or response advertised; 1 when it said nothing.
pub fn advertised(plaintext: &[u8]) -> u8 {
    #[derive(Deserialize)]
    struct Probe {
        #[serde(default)]
        wire: u8,
    }
    serde_json::from_slice::<Probe>(plaintext)
        .map(|probe| probe.wire.max(WIRE_V1))
        .unwrap_or(WIRE_V1)
}

struct Bytes<'a>(&'a [u8], Wire);

impl Serialize for Bytes<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self.1 {
            Wire::V1 => serializer.collect_seq(self.0),
            Wire::V2 => serializer.serialize_str(&URL_SAFE_NO_PAD.encode(self.0)),
        }
    }
}

#[derive(Serialize)]
struct RequestOut<'a> {
    request_id: Bytes<'a>,
    route: RouteId,
    method: &'a str,
    parameters: &'a [String],
    headers: &'a [(String, String)],
    body: Bytes<'a>,
    #[serde(skip_serializing_if = "Option::is_none")]
    body_blob: Option<RelayBlobReference>,
    wire: u8,
}

#[derive(Serialize)]
struct ResponseOut<'a> {
    request_id: Bytes<'a>,
    status: u16,
    headers: &'a [(String, String)],
    body: Bytes<'a>,
    #[serde(skip_serializing_if = "Option::is_none")]
    body_blob: Option<RelayBlobReference>,
    wire: u8,
}

#[derive(Serialize)]
struct FrameOut<'a> {
    stream_id: Bytes<'a>,
    direction: FrameDirection,
    kind: WebSocketFrameKind,
    text: bool,
    bytes: Bytes<'a>,
    #[serde(skip_serializing_if = "Option::is_none")]
    bytes_blob: Option<RelayBlobReference>,
}

/// Encode a bridge request in `wire`, advertising [`WIRE_LATEST`].
pub fn encode_request(request: &BridgeRequest, wire: Wire) -> serde_json::Result<Vec<u8>> {
    serde_json::to_vec(&RequestOut {
        request_id: Bytes(&request.request_id, wire),
        route: request.route,
        method: &request.method,
        parameters: &request.parameters,
        headers: &request.headers,
        body: Bytes(&request.body, wire),
        body_blob: request.body_blob,
        wire: WIRE_LATEST,
    })
}

/// Encode a bridge response in `wire`, advertising [`WIRE_LATEST`].
pub fn encode_response(response: &BridgeResponse, wire: Wire) -> serde_json::Result<Vec<u8>> {
    serde_json::to_vec(&ResponseOut {
        request_id: Bytes(&response.request_id, wire),
        status: response.status,
        headers: &response.headers,
        body: Bytes(&response.body, wire),
        body_blob: response.body_blob,
        wire: WIRE_LATEST,
    })
}

/// Encode a stream frame in the `wire` negotiated when its stream opened.
pub fn encode_frame(frame: &WebSocketFrame, wire: Wire) -> serde_json::Result<Vec<u8>> {
    serde_json::to_vec(&FrameOut {
        stream_id: Bytes(&frame.stream_id, wire),
        direction: frame.direction,
        kind: frame.kind,
        text: frame.text,
        bytes: Bytes(&frame.bytes, wire),
        bytes_blob: frame.bytes_blob,
    })
}

struct ByteVisitor;

impl<'de> Visitor<'de> for ByteVisitor {
    type Value = Vec<u8>;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("an array of bytes or an unpadded base64url string")
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
        let mut bytes = Vec::with_capacity(seq.size_hint().unwrap_or(0).min(1 << 20));
        while let Some(byte) = seq.next_element::<u8>()? {
            bytes.push(byte);
        }
        Ok(bytes)
    }

    fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
        URL_SAFE_NO_PAD
            .decode(value)
            .map_err(|_| E::custom("byte field is not unpadded base64url"))
    }
}

/// `deserialize_with` for `Vec<u8>` fields: accepts wire v1 arrays and wire v2 strings.
pub mod bytes {
    use super::*;

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
        deserializer.deserialize_any(ByteVisitor)
    }
}

/// `deserialize_with` for `[u8; N]` fields: accepts wire v1 arrays and wire v2 strings.
pub mod fixed {
    use super::*;

    pub fn deserialize<'de, D: Deserializer<'de>, const N: usize>(
        deserializer: D,
    ) -> Result<[u8; N], D::Error> {
        let bytes = deserializer.deserialize_any(ByteVisitor)?;
        let length = bytes.len();
        bytes
            .try_into()
            .map_err(|_| de::Error::invalid_length(length, &"a fixed-length byte field"))
    }
}

/// `deserialize_with` for `Option<[u8; N]>` fields.
pub mod optional_fixed {
    use super::*;

    pub fn deserialize<'de, D: Deserializer<'de>, const N: usize>(
        deserializer: D,
    ) -> Result<Option<[u8; N]>, D::Error> {
        #[derive(Deserialize)]
        struct Fixed<const M: usize>(#[serde(deserialize_with = "fixed::deserialize")] [u8; M]);
        Ok(Option::<Fixed<N>>::deserialize(deserializer)?.map(|value| value.0))
    }
}

#[cfg(test)]
mod tests;
