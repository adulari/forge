//! AWS event-stream frame parser for Bedrock ConverseStream.
//!
//! ConverseStream uses `application/vnd.amazon.eventstream` framing, not SSE. Each message has:
//!
//!   [0..4)   total_len (big-endian u32)
//!   [4..8)   headers_len (big-endian u32)
//!   [8..12)  prelude_crc (CRC32 of bytes 0..8)
//!   [12..12+headers_len)  headers
//!   [12+headers_len..total_len-4)  payload
//!   [total_len-4..total_len)  message_crc (CRC32 of bytes 0..total_len-4)
//!
//! Headers are a series of { name_len u8, name utf8, value_type u8, value_len u16, value bytes }.
//! We only care about `:event-type` (string, type=7) to dispatch, and `:message-type` (also
//! string) to detect errors.
//!
//! See: https://docs.aws.amazon.com/transcribe/latest/dg/event-stream.html

use crate::adapter::adapters::support::{StreamerCapturedData, StreamerOptions};
use crate::adapter::inter_stream::{InterStreamEnd, InterStreamEvent};
use crate::chat::{ChatOptionsSet, StopReason, ToolCall, Usage};
use crate::{Error, ModelIden, Result};
use bytes::{Buf, BytesMut};
use futures::Stream;
use serde_json::{Map, Value};
use std::collections::VecDeque;
use std::pin::Pin;
use std::task::{Context, Poll};
use value_ext::JsonValueExt;

/// A streaming parser that pulls raw bytes from a [`WebStream`] and yields decoded
/// `InterStreamEvent`s by parsing AWS event-stream frames and the Converse payloads inside them.
pub(super) struct BedrockStreamer {
	// We reuse WebStream with a custom mode that hands us raw bytes. To keep the hotfix small,
	// we instead hold a boxed byte stream directly.
	inner: Pin<Box<dyn Stream<Item = std::result::Result<bytes::Bytes, crate::error::BoxError>> + Send>>,
	buf: BytesMut,
	options: StreamerOptions,
	captured_data: StreamerCapturedData,
	done: bool,
	emitted_start: bool,
	in_progress_tool: Option<ToolCallAccumulator>,
	/// Events decoded from a frame but not yet yielded: one frame can produce several.
	pending: VecDeque<InterStreamEvent>,
}

struct ToolCallAccumulator {
	call_id: String,
	fn_name: String,
	input: String,
}

impl BedrockStreamer {
	pub(super) fn new(
		bytes_stream: Pin<Box<dyn Stream<Item = std::result::Result<bytes::Bytes, crate::error::BoxError>> + Send>>,
		model_iden: ModelIden,
		options_set: ChatOptionsSet<'_, '_>,
	) -> Self {
		Self {
			inner: bytes_stream,
			buf: BytesMut::with_capacity(8 * 1024),
			options: StreamerOptions::new(model_iden, options_set),
			captured_data: StreamerCapturedData::default(),
			done: false,
			emitted_start: false,
			in_progress_tool: None,
			pending: VecDeque::new(),
		}
	}

	/// Try to pull one complete event-stream frame from `self.buf` and return its
	/// decoded payload (event-type + JSON), or None if the buffer doesn't yet contain a full frame.
	fn try_parse_frame(&mut self) -> Result<Option<DecodedFrame>> {
		if self.buf.len() < 12 {
			return Ok(None);
		}

		let total_len = u32::from_be_bytes(self.buf[0..4].try_into().unwrap()) as usize;
		let headers_len = u32::from_be_bytes(self.buf[4..8].try_into().unwrap()) as usize;

		if !(16..=16 * 1024 * 1024).contains(&total_len) {
			return Err(self.frame_err(format!("invalid event-stream total_len: {total_len}")));
		}

		if self.buf.len() < total_len {
			return Ok(None);
		}

		// Validate prelude CRC
		let prelude_crc = u32::from_be_bytes(self.buf[8..12].try_into().unwrap());
		let prelude_actual = crc32(&self.buf[0..8]);
		if prelude_crc != prelude_actual {
			return Err(self.frame_err(format!("prelude CRC mismatch: {prelude_crc} != {prelude_actual}")));
		}

		// Validate message CRC
		let msg_crc = u32::from_be_bytes(self.buf[total_len - 4..total_len].try_into().unwrap());
		let msg_actual = crc32(&self.buf[0..total_len - 4]);
		if msg_crc != msg_actual {
			return Err(self.frame_err(format!("message CRC mismatch: {msg_crc} != {msg_actual}")));
		}

		let headers_start = 12;
		let headers_end = headers_start + headers_len;
		let payload_end = total_len - 4;

		let headers = parse_headers(&self.buf[headers_start..headers_end])
			.map_err(|e| self.frame_err(format!("header parse: {e}")))?;
		let payload = self.buf[headers_end..payload_end].to_vec();

		// Advance past the frame
		self.buf.advance(total_len);

		Ok(Some(DecodedFrame { headers, payload }))
	}

	fn frame_err(&self, msg: String) -> Error {
		Error::ChatResponse {
			model_iden: self.options.model_iden.clone(),
			body: serde_json::json!({ "error": msg }),
		}
	}

	/// Dispatch a decoded frame into zero or more InterStreamEvents.
	fn handle_frame(&mut self, frame: DecodedFrame) -> Result<Vec<InterStreamEvent>> {
		let event_type = frame.headers.get(":event-type").cloned().unwrap_or_default();
		let message_type = frame.headers.get(":message-type").cloned().unwrap_or_default();

		if message_type == "exception" || message_type == "error" {
			let body: Value = serde_json::from_slice(&frame.payload).unwrap_or(Value::Null);
			return Err(Error::ChatResponse {
				model_iden: self.options.model_iden.clone(),
				body,
			});
		}

		let mut payload: Value = if frame.payload.is_empty() {
			Value::Null
		} else {
			serde_json::from_slice(&frame.payload).map_err(|serde_error| Error::StreamParse {
				model_iden: self.options.model_iden.clone(),
				serde_error,
			})?
		};

		let mut events: Vec<InterStreamEvent> = Vec::new();

		if !self.emitted_start {
			self.emitted_start = true;
			events.push(InterStreamEvent::Start);
		}

		match event_type.as_str() {
			"messageStart" => {
				// No-op: genai already emitted Start above.
			}
			"contentBlockStart" => {
				// { start: { toolUse: { toolUseId, name } }, contentBlockIndex }
				if let Ok(mut tool_use) = payload.x_take::<Value>("/start/toolUse") {
					let call_id: String = tool_use.x_take("toolUseId").unwrap_or_default();
					let fn_name: String = tool_use.x_take("name").unwrap_or_default();

					let tc = ToolCall {
						call_id: call_id.clone(),
						fn_name: fn_name.clone(),
						fn_arguments: Value::String(String::new()),
						thought_signatures: None,
					};
					self.in_progress_tool = Some(ToolCallAccumulator {
						call_id,
						fn_name,
						input: String::new(),
					});
					events.push(InterStreamEvent::ToolCallChunk(tc));
				}
			}
			"contentBlockDelta" => {
				// { delta: { text? | toolUse { input }? | reasoningContent { text | signature } }, contentBlockIndex }
				if let Ok(text) = payload.x_take::<String>("/delta/text") {
					if self.options.capture_content {
						match self.captured_data.content {
							Some(ref mut c) => c.push_str(&text),
							None => self.captured_data.content = Some(text.clone()),
						}
					}
					events.push(InterStreamEvent::Chunk(text));
				} else if let Ok(partial) = payload.x_take::<String>("/delta/toolUse/input") {
					if let Some(acc) = self.in_progress_tool.as_mut() {
						acc.input.push_str(&partial);
						events.push(InterStreamEvent::ToolCallChunk(ToolCall {
							call_id: acc.call_id.clone(),
							fn_name: acc.fn_name.clone(),
							fn_arguments: Value::String(acc.input.clone()),
							thought_signatures: None,
						}));
					}
				} else if let Ok(reasoning) = payload.x_take::<String>("/delta/reasoningContent/text") {
					if self.options.capture_reasoning_content {
						match self.captured_data.reasoning_content {
							Some(ref mut r) => r.push_str(&reasoning),
							None => self.captured_data.reasoning_content = Some(reasoning.clone()),
						}
					}
					events.push(InterStreamEvent::ReasoningChunk(reasoning));
				} else if let Ok(signature) = payload.x_take::<String>("/delta/reasoningContent/signature") {
					events.push(InterStreamEvent::ThoughtSignatureChunk(signature));
				}
			}
			"contentBlockStop" => {
				if let Some(acc) = self.in_progress_tool.take()
					&& self.options.capture_tool_calls
				{
					let fn_arguments = if acc.input.is_empty() {
						Value::Object(Map::new())
					} else {
						serde_json::from_str(&acc.input).unwrap_or(Value::Object(Map::new()))
					};
					let tc = ToolCall {
						call_id: acc.call_id,
						fn_name: acc.fn_name,
						fn_arguments,
						thought_signatures: None,
					};
					match self.captured_data.tool_calls {
						Some(ref mut t) => t.push(tc),
						None => self.captured_data.tool_calls = Some(vec![tc]),
					}
				}
			}
			"messageStop" => {
				if let Ok(reason) = payload.x_take::<String>("stopReason") {
					self.captured_data.stop_reason = Some(reason);
				}
			}
			"metadata" => {
				if self.options.capture_usage
					&& let Ok(usage_value) = payload.x_take::<Value>("usage")
				{
					self.captured_data.usage = Some(parse_stream_usage(usage_value));
				}
			}
			other => {
				tracing::debug!("Bedrock stream: unhandled event type {other}");
			}
		}

		Ok(events)
	}

	fn finalize_end(&mut self) -> InterStreamEvent {
		let captured_usage = if self.options.capture_usage {
			self.captured_data.usage.take()
		} else {
			None
		};
		let end = InterStreamEnd {
			captured_usage,
			captured_stop_reason: self.captured_data.stop_reason.take().map(StopReason::from),
			captured_text_content: self.captured_data.content.take(),
			captured_reasoning_content: self.captured_data.reasoning_content.take(),
			captured_tool_calls: self.captured_data.tool_calls.take(),
			captured_thought_signatures: None,
			captured_response_id: None,
		};
		InterStreamEvent::End(end)
	}
}

impl Stream for BedrockStreamer {
	type Item = Result<InterStreamEvent>;

	fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
		loop {
			if let Some(event) = self.pending.pop_front() {
				return Poll::Ready(Some(Ok(event)));
			}
			if self.done {
				return Poll::Ready(None);
			}

			match self.try_parse_frame() {
				Ok(Some(frame)) => {
					// `metadata` (token usage) is the last event of a ConverseStream and arrives
					// AFTER `messageStop`. Ending at `messageStop` threw it away, so every Bedrock
					// reply reported zero tokens: no cost, and a context gauge stuck at empty.
					let is_final = frame.headers.get(":event-type").map(|s| s.as_str()) == Some("metadata");
					let events = self.handle_frame(frame)?;
					self.pending.extend(events);
					if is_final {
						self.done = true;
						let end = self.finalize_end();
						self.pending.push_back(end);
					}
					continue;
				}
				Ok(None) => {
					// Need more bytes.
				}
				Err(err) => return Poll::Ready(Some(Err(err))),
			}

			match Pin::new(&mut self.inner).poll_next(cx) {
				Poll::Ready(Some(Ok(bytes))) => {
					self.buf.extend_from_slice(&bytes);
					continue;
				}
				Poll::Ready(Some(Err(err))) => {
					return Poll::Ready(Some(Err(Error::WebStream {
						model_iden: self.options.model_iden.clone(),
						cause: err.to_string(),
						error: err,
					})));
				}
				Poll::Ready(None) => {
					// A stream that closes without `metadata` still ends cleanly with what it had.
					self.done = true;
					if self.emitted_start {
						let end = self.finalize_end();
						self.pending.push_back(end);
					}
					continue;
				}
				Poll::Pending => return Poll::Pending,
			}
		}
	}
}

struct DecodedFrame {
	headers: std::collections::HashMap<String, String>,
	payload: Vec<u8>,
}

/// CRC32 (IEEE 802.3, poly 0xEDB88320) used by AWS event-stream framing.
///
/// Hand-rolled to avoid pulling in `crc32fast` for the `bedrock-api` feature. Table is computed
/// once at first use. This runs on small byte ranges (prelude: 8 bytes; message: up to the
/// frame size), so the per-byte loop is fine.
fn crc32(bytes: &[u8]) -> u32 {
	use std::sync::OnceLock;
	static TABLE: OnceLock<[u32; 256]> = OnceLock::new();
	let table = TABLE.get_or_init(|| {
		let mut t = [0u32; 256];
		for (i, slot) in t.iter_mut().enumerate() {
			let mut c = i as u32;
			for _ in 0..8 {
				c = if c & 1 != 0 { 0xEDB88320 ^ (c >> 1) } else { c >> 1 };
			}
			*slot = c;
		}
		t
	});
	let mut c: u32 = 0xFFFF_FFFF;
	for &b in bytes {
		let idx = ((c ^ b as u32) & 0xFF) as usize;
		c = table[idx] ^ (c >> 8);
	}
	c ^ 0xFFFF_FFFF
}

fn parse_headers(mut raw: &[u8]) -> std::result::Result<std::collections::HashMap<String, String>, String> {
	let mut out = std::collections::HashMap::new();
	while !raw.is_empty() {
		if raw.is_empty() {
			break;
		}
		let name_len = raw[0] as usize;
		raw = &raw[1..];
		if raw.len() < name_len + 1 {
			return Err("header name truncated".into());
		}
		let name = std::str::from_utf8(&raw[..name_len])
			.map_err(|e| format!("header name utf8: {e}"))?
			.to_string();
		raw = &raw[name_len..];
		let value_type = raw[0];
		raw = &raw[1..];

		// type 7 = string (value_len u16 + bytes). Others we skip.
		if value_type == 7 {
			if raw.len() < 2 {
				return Err("header value length truncated".into());
			}
			let value_len = u16::from_be_bytes([raw[0], raw[1]]) as usize;
			raw = &raw[2..];
			if raw.len() < value_len {
				return Err("header value truncated".into());
			}
			let value = std::str::from_utf8(&raw[..value_len])
				.map_err(|e| format!("header value utf8: {e}"))?
				.to_string();
			raw = &raw[value_len..];
			out.insert(name, value);
		} else {
			// For simplicity, skip non-string headers by consuming the rest (not strictly correct
			// but Bedrock only emits string headers for the fields we care about).
			break;
		}
	}
	Ok(out)
}

fn parse_stream_usage(value: Value) -> Usage {
	super::converse::parse_usage(value)
}

#[cfg(test)]
mod tests {
	use super::*;

	/// Test only needs a valid ModelIden; the AdapterKind doesn't affect parser behavior.
	fn test_model_iden() -> crate::ModelIden {
		crate::ModelIden::new(crate::adapter::AdapterKind::BedrockApi, "anthropic.claude-test")
	}

	/// Build a minimal valid event-stream frame with the given headers and payload.
	fn build_frame(event_type: &str, payload: &[u8]) -> Vec<u8> {
		// Headers: :event-type (string type 7) = event_type
		// Format per header: name_len u8 | name | type u8 | value_len u16 | value
		let mut headers = Vec::new();
		let name = ":event-type";
		headers.push(name.len() as u8);
		headers.extend_from_slice(name.as_bytes());
		headers.push(7);
		headers.extend_from_slice(&(event_type.len() as u16).to_be_bytes());
		headers.extend_from_slice(event_type.as_bytes());

		let headers_len = headers.len() as u32;
		let total_len = 12 + headers_len + payload.len() as u32 + 4;

		let mut frame = Vec::new();
		frame.extend_from_slice(&total_len.to_be_bytes());
		frame.extend_from_slice(&headers_len.to_be_bytes());
		let prelude_crc = super::crc32(&frame[..8]);
		frame.extend_from_slice(&prelude_crc.to_be_bytes());
		frame.extend_from_slice(&headers);
		frame.extend_from_slice(payload);
		let msg_crc = super::crc32(&frame);
		frame.extend_from_slice(&msg_crc.to_be_bytes());
		frame
	}

	#[test]
	fn parses_single_delta_frame() {
		let payload = br#"{"delta":{"text":"hello"},"contentBlockIndex":0}"#;
		let frame = build_frame("contentBlockDelta", payload);

		let model_iden = test_model_iden();
		// Empty stream for inner — we just exercise the header + payload decoding by injecting bytes.
		let inner: Pin<Box<dyn Stream<Item = _> + Send>> = Box::pin(futures::stream::empty());
		let mut streamer = BedrockStreamer::new(inner, model_iden, Default::default());
		streamer.buf.extend_from_slice(&frame);

		let decoded = streamer.try_parse_frame().expect("parse ok").expect("frame present");
		assert_eq!(
			decoded.headers.get(":event-type").map(String::as_str),
			Some("contentBlockDelta")
		);
		assert_eq!(decoded.payload, payload);
	}

	#[tokio::test]
	async fn usage_after_message_stop_reaches_the_end_event() {
		// Real ConverseStream order: the usage-carrying `metadata` frame comes after
		// `messageStop`. The stream used to end at `messageStop`, so every reply reported zero.
		use futures::StreamExt;
		let mut bytes = Vec::new();
		bytes.extend(build_frame("messageStart", br#"{"role":"assistant"}"#));
		bytes.extend(build_frame(
			"contentBlockDelta",
			br#"{"delta":{"text":"pong"},"contentBlockIndex":0}"#,
		));
		bytes.extend(build_frame("contentBlockStop", br#"{"contentBlockIndex":0}"#));
		bytes.extend(build_frame("messageStop", br#"{"stopReason":"end_turn"}"#));
		bytes.extend(build_frame(
			"metadata",
			br#"{"usage":{"inputTokens":7,"cacheReadInputTokens":83,"cacheWriteInputTokens":0,"outputTokens":69,"totalTokens":159},"metrics":{"latencyMs":1858}}"#,
		));
		let inner: Pin<Box<dyn Stream<Item = _> + Send>> =
			Box::pin(futures::stream::iter(vec![Ok(bytes::Bytes::from(bytes))]));
		let options = crate::chat::ChatOptions::default().with_capture_usage(true);
		let options_set = ChatOptionsSet::default().with_chat_options(Some(&options));
		let events: Vec<_> = BedrockStreamer::new(inner, test_model_iden(), options_set)
			.collect::<Vec<_>>()
			.await
			.into_iter()
			.map(|event| event.expect("event"))
			.collect();

		assert!(
			events
				.iter()
				.any(|event| matches!(event, InterStreamEvent::Chunk(text) if text == "pong")),
			"the text delta is still yielded"
		);
		let Some(InterStreamEvent::End(end)) = events.last() else {
			panic!("the stream ends with End");
		};
		let usage = end.captured_usage.as_ref().expect("usage captured");
		assert_eq!(usage.prompt_tokens, Some(90));
		assert_eq!(usage.completion_tokens, Some(69));
		assert_eq!(end.captured_stop_reason, Some(StopReason::from("end_turn".to_string())));
	}

	#[test]
	fn partial_frame_returns_none() {
		let model_iden = test_model_iden();
		let inner: Pin<Box<dyn Stream<Item = _> + Send>> = Box::pin(futures::stream::empty());
		let mut streamer = BedrockStreamer::new(inner, model_iden, Default::default());
		streamer.buf.extend_from_slice(&[0u8; 10]); // <12, not enough for prelude
		assert!(streamer.try_parse_frame().expect("ok").is_none());
	}

	#[test]
	fn streaming_usage_preserves_prompt_cache_tokens() {
		let usage = parse_stream_usage(serde_json::json!({
			"inputTokens": 12_000,
			"outputTokens": 32,
			"totalTokens": 12_032,
			"cacheReadInputTokens": 10_240,
			"cacheWriteInputTokens": 1_024,
		}));

		// `inputTokens` excludes cached tokens; the prompt is all three together.
		assert_eq!(usage.prompt_tokens, Some(12_000 + 10_240 + 1_024));
		let details = usage.prompt_tokens_details.expect("Bedrock cache usage details");
		assert_eq!(details.cached_tokens, Some(10_240));
		assert_eq!(details.cache_creation_tokens, Some(1_024));
	}
}
