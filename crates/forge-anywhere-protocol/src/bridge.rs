//! Typed requests for the managed relay-to-daemon bridge.

use serde::{Deserialize, Serialize};

/// Explicit daemon routes that a host connector may invoke.
///
/// There is intentionally no arbitrary URL/path variant: adding a daemon capability requires a
/// reviewed protocol change on both sides of the bridge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RouteId {
    Health,
    ListSessions,
    CreateSession,
    SessionSnapshot,
    SessionHistory,
    SessionInput,
    ArchiveSession,
    PastSessions,
    SessionTree,
    ForkSession,
    MergeSession,
    DiscardSession,
    ListProjects,
    BrowseProjects,
    Upload,
    VoiceTranscribe,
    ListSkills,
    ListModels,
    ReadConfig,
    UpdateConfig,
    ListHooks,
    ListPlans,
    ReadMcp,
    UpdateMcp,
    Usage,
    Answer,
    PushKey,
    PushSubscribe,
    PushUnsubscribe,
    WebSocket,
}

/// Authenticated reference to a temporary encrypted relay object.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelayBlobReference {
    #[serde(with = "hex_id")]
    pub blob_id: [u8; 16],
    pub ciphertext_bytes: u64,
    #[serde(with = "base64_hash")]
    pub ciphertext_sha256: [u8; 32],
}

mod hex_id {
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
        let bytes = hex::decode(&value).map_err(serde::de::Error::custom)?;
        bytes
            .try_into()
            .map_err(|_| serde::de::Error::custom("blob_id must contain 16 bytes"))
    }
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
        let value = String::deserialize(deserializer)?;
        let bytes = URL_SAFE_NO_PAD
            .decode(value)
            .map_err(serde::de::Error::custom)?;
        bytes
            .try_into()
            .map_err(|_| serde::de::Error::custom("ciphertext_sha256 must contain 32 bytes"))
    }
}

/// An encrypted bridge request sent from a controller to one host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeRequest {
    pub request_id: [u8; 16],
    pub route: RouteId,
    /// Redundant with `route` by design: the host validates both before dispatch.
    pub method: String,
    /// Route-relative path parameters, never a URL.
    #[serde(default)]
    pub parameters: Vec<String>,
    /// End-to-end request headers from a narrow allowlist (for example `content-type`).
    #[serde(default)]
    pub headers: Vec<(String, String)>,
    /// Opaque local-daemon request bytes.
    #[serde(default)]
    pub body: Vec<u8>,
    /// Temporary encrypted object containing `body` when it exceeds the inline limit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body_blob: Option<RelayBlobReference>,
}

/// An encrypted bridge response returned by the host connector.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeResponse {
    pub request_id: [u8; 16],
    pub status: u16,
    #[serde(default)]
    pub headers: Vec<(String, String)>,
    #[serde(default)]
    pub body: Vec<u8>,
    /// Temporary encrypted object containing `body` when it exceeds the inline limit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body_blob: Option<RelayBlobReference>,
}

/// One encrypted remote-v8 frame, forwarded without interpreting session semantics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebSocketFrame {
    pub stream_id: [u8; 16],
    pub direction: FrameDirection,
    pub kind: WebSocketFrameKind,
    #[serde(default)]
    pub bytes: Vec<u8>,
    /// Temporary encrypted object containing `bytes` when it exceeds the inline limit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes_blob: Option<RelayBlobReference>,
}

/// Stream lifecycle marker; data bytes remain opaque to the connector and relay.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WebSocketFrameKind {
    Data,
    Close,
}

/// Which side produced a proxied WebSocket frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FrameDirection {
    ControllerToHost,
    HostToController,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_inline_bridge_request_defaults_blob_reference() {
        let request: BridgeRequest = serde_json::from_value(serde_json::json!({
            "request_id": vec![1; 16],
            "route": "health",
            "method": "GET"
        }))
        .expect("decode legacy request");

        assert!(request.body.is_empty());
        assert_eq!(request.body_blob, None);
        let encoded = serde_json::to_value(&request).expect("encode request");
        assert!(encoded.get("body_blob").is_none());

        let response: BridgeResponse = serde_json::from_value(serde_json::json!({
            "request_id": vec![1; 16],
            "status": 204
        }))
        .expect("decode legacy response");
        assert!(response.body.is_empty());
        assert_eq!(response.body_blob, None);
        assert!(serde_json::to_value(response)
            .expect("encode response")
            .get("body_blob")
            .is_none());
    }

    #[test]
    fn blob_references_round_trip_and_none_is_omitted() {
        let reference = RelayBlobReference {
            blob_id: [2; 16],
            ciphertext_bytes: 4096,
            ciphertext_sha256: [3; 32],
        };
        let response = BridgeResponse {
            request_id: [1; 16],
            status: 200,
            headers: Vec::new(),
            body: Vec::new(),
            body_blob: Some(reference),
        };
        let encoded = serde_json::to_vec(&response).expect("encode response");
        let json: serde_json::Value = serde_json::from_slice(&encoded).expect("decode JSON");
        assert_eq!(json["body_blob"]["blob_id"], hex::encode([2; 16]));
        assert_eq!(
            json["body_blob"]["ciphertext_sha256"],
            "AwMDAwMDAwMDAwMDAwMDAwMDAwMDAwMDAwMDAwMDAwM"
        );
        assert_eq!(
            serde_json::from_slice::<BridgeResponse>(&encoded).expect("decode response"),
            response
        );

        let frame: WebSocketFrame = serde_json::from_value(serde_json::json!({
            "stream_id": vec![4; 16],
            "direction": "host_to_controller",
            "kind": "close"
        }))
        .expect("decode legacy frame");
        assert!(frame.bytes.is_empty());
        assert_eq!(frame.bytes_blob, None);
        assert!(serde_json::to_value(frame)
            .expect("encode frame")
            .get("bytes_blob")
            .is_none());
    }
}
