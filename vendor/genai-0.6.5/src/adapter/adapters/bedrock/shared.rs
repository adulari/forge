//! Shared helpers used by both the SigV4 and API-key Bedrock adapters.

use crate::adapter::{AdapterKind, ServiceType};
use crate::resolver::Endpoint;
use crate::{Error, ModelIden, Result};
use reqwest::RequestBuilder;

/// The hostname prefix for the Bedrock runtime endpoint. Regions are interpolated between this
/// prefix and `.amazonaws.com`.
pub(super) const BEDROCK_RUNTIME_HOST_PREFIX: &str = "bedrock-runtime";

/// Curated snapshot of model IDs. Dynamic listing requires the `bedrock` (control plane) API,
/// not `bedrock-runtime`, so we return a hard-coded list here.
pub(super) fn curated_model_names() -> Vec<String> {
	vec![
		"anthropic.claude-sonnet-4-5-20250929-v1:0".to_string(),
		"anthropic.claude-opus-4-1-20250805-v1:0".to_string(),
		"anthropic.claude-haiku-4-5-20251001-v1:0".to_string(),
		"amazon.nova-pro-v1:0".to_string(),
		"amazon.nova-lite-v1:0".to_string(),
		"amazon.nova-micro-v1:0".to_string(),
		"meta.llama3-1-70b-instruct-v1:0".to_string(),
		"mistral.mistral-large-2407-v1:0".to_string(),
	]
}

/// Build the Converse / ConverseStream URL for a given model + service type.
pub(super) fn build_service_url(
	model: &ModelIden,
	service_type: ServiceType,
	endpoint: Endpoint,
	adapter_kind: AdapterKind,
) -> Result<String> {
	let base_url = endpoint.base_url();
	let (_, model_name) = model.model_name.namespace_and_name();
	// Model IDs contain ':' (e.g., anthropic.claude-sonnet-4-5-20250929-v1:0) and must be
	// URL-encoded inside the path segment.
	let encoded = urlencode_path_segment(model_name);

	let url = match service_type {
		ServiceType::Chat => format!("{base_url}model/{encoded}/converse"),
		ServiceType::ChatStream => format!("{base_url}model/{encoded}/converse-stream"),
		ServiceType::Embed => {
			return Err(Error::AdapterNotSupported {
				adapter_kind,
				feature: "embeddings via Converse (use /invoke instead, not yet supported)".to_string(),
			});
		}
	};
	Ok(url)
}

fn urlencode_path_segment(s: &str) -> String {
	let mut out = String::with_capacity(s.len());
	for b in s.bytes() {
		match b {
			b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => out.push(b as char),
			_ => {
				out.push('%');
				out.push_str(&format!("{:02X}", b));
			}
		}
	}
	out
}

/// Build the error body genai surfaces for a Bedrock failure, keeping the exception type.
///
/// Bedrock carries the exception type OUTSIDE the JSON payload: in the `:exception-type` (or
/// `:error-code`) event-stream header on a streamed failure, and in the `x-amzn-ErrorType` response
/// header on an HTTP one (`ThrottlingException:http://internal.amazon.com/...`). The payload is just
/// `{"message": "..."}`, so dropping the header made a throttle, an outage and a validation error
/// indistinguishable. Folded in as `{"error":{"type":..,"message":..}}`, the shape callers already
/// read. Without a type the payload is returned unchanged.
pub(super) fn error_body(error_type: Option<&str>, header_message: Option<&str>, raw: &[u8]) -> serde_json::Value {
	let parsed: Option<serde_json::Value> = serde_json::from_slice(raw).ok();
	let Some(error_type) = error_type
		.map(|t| t.split(':').next().unwrap_or(t).trim())
		.filter(|t| !t.is_empty())
	else {
		return parsed.unwrap_or(serde_json::Value::Null);
	};
	let from_payload = parsed.as_ref().and_then(|v| {
		["message", "Message", "originalMessage"]
			.iter()
			.find_map(|k| v.get(*k).and_then(|m| m.as_str()))
	});
	let message = match (from_payload, parsed.as_ref()) {
		(Some(m), _) => m.to_string(),
		(None, _) if header_message.is_some() => header_message.unwrap_or_default().to_string(),
		(None, Some(v)) if !v.is_null() => v.to_string(),
		_ => String::from_utf8_lossy(raw).trim().to_string(),
	};
	serde_json::json!({ "error": { "type": error_type, "message": message } })
}

/// Turn a `reqwest::RequestBuilder` into a byte stream for the event-stream frame parser.
/// On HTTP error, yields a single error item so the parser surfaces it through the normal
/// error path.
pub(super) fn async_stream_bytes(
	reqwest_builder: RequestBuilder,
) -> impl futures::Stream<Item = std::result::Result<bytes::Bytes, crate::error::BoxError>> + Send {
	use futures::StreamExt;
	async_stream_once(reqwest_builder).flat_map(|result| match result {
		Ok(stream) => stream.boxed(),
		Err(err) => futures::stream::once(async move { Err(err) }).boxed(),
	})
}

fn async_stream_once(
	reqwest_builder: RequestBuilder,
) -> impl futures::Stream<
	Item = std::result::Result<
		futures::stream::BoxStream<'static, std::result::Result<bytes::Bytes, crate::error::BoxError>>,
		crate::error::BoxError,
	>,
> + Send {
	use futures::StreamExt;
	futures::stream::once(async move {
		let resp = reqwest_builder
			.send()
			.await
			.map_err(|e| Box::new(e) as crate::error::BoxError)?;
		let status = resp.status();
		if !status.is_success() {
			let error_type = resp
				.headers()
				.get("x-amzn-errortype")
				.and_then(|v| v.to_str().ok())
				.map(str::to_string);
			let raw = resp.text().await.unwrap_or_default();
			let body = match error_type.as_deref() {
				Some(ty) => error_body(Some(ty), None, raw.as_bytes()).to_string(),
				None => raw,
			};
			let err = crate::Error::HttpError {
				status,
				canonical_reason: status.canonical_reason().unwrap_or("Unknown").to_string(),
				body,
			};
			return Err(Box::new(err) as crate::error::BoxError);
		}
		let bytes: futures::stream::BoxStream<'static, std::result::Result<bytes::Bytes, crate::error::BoxError>> =
			resp.bytes_stream()
				.map(|r| r.map_err(|e| Box::new(e) as crate::error::BoxError))
				.boxed();
		Ok(bytes)
	})
}

#[cfg(test)]
mod tests {
	use super::error_body;

	#[test]
	fn exception_type_is_kept_beside_the_message() {
		let body = error_body(
			Some("throttlingException"),
			None,
			br#"{"message":"Too many tokens, please wait before trying again."}"#,
		);
		assert_eq!(body["error"]["type"], "throttlingException");
		assert_eq!(
			body["error"]["message"],
			"Too many tokens, please wait before trying again."
		);
	}

	#[test]
	fn http_error_type_header_drops_the_documentation_suffix() {
		let body = error_body(
			Some("ThrottlingException:http://internal.amazon.com/coral/com.amazon.bedrock/"),
			None,
			br#"{"Message":"slow down"}"#,
		);
		assert_eq!(body["error"]["type"], "ThrottlingException");
		assert_eq!(body["error"]["message"], "slow down");
	}

	#[test]
	fn stream_error_falls_back_to_original_message_then_header_message() {
		let body = error_body(
			Some("modelStreamErrorException"),
			None,
			br#"{"originalStatusCode":500,"originalMessage":"upstream blew up"}"#,
		);
		assert_eq!(body["error"]["message"], "upstream blew up");
		let body = error_body(Some("internalServerException"), Some("boom"), b"");
		assert_eq!(body["error"]["message"], "boom");
	}

	#[test]
	fn without_a_type_the_payload_is_returned_unchanged() {
		let body = error_body(None, None, br#"{"message":"x"}"#);
		assert_eq!(body, serde_json::json!({"message":"x"}));
		assert_eq!(error_body(None, None, b"not json"), serde_json::Value::Null);
	}
}
