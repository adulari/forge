//! genai ChatRequest ↔ Bedrock Converse JSON mapping.
//!
//! Converse normalizes message shape across all Bedrock publishers, so the main wire-format
//! work lives here. Publisher-specific bits (reasoning budget, etc.) go under
//! `additionalModelRequestFields` via [`BedrockPublisher`].

use crate::chat::{
	Binary, BinarySource, ChatOptionsSet, ChatRequest, ChatResponse, ChatRole, ContentPart, MessageContent,
	ReasoningEffort, StopReason, Tool, ToolCall, ToolName, Usage,
};
use crate::webc::WebResponse;
use crate::{Error, ModelIden, Result};
use serde_json::{Map, Value, json};
use tracing::warn;
use value_ext::JsonValueExt;

/// Which Bedrock publisher a model ID targets. Used only to fill
/// `additionalModelRequestFields` — message shape is identical across publishers.
#[derive(Debug, Clone, Copy)]
pub(super) enum BedrockPublisher {
	Anthropic,
	AmazonNova,
	Other,
}

impl BedrockPublisher {
	/// Model IDs are of the form `<publisher>.<model>...` or
	/// `<region>.<publisher>.<model>...` (cross-region inference profiles).
	pub(super) fn from_model_id(model_id: &str) -> Self {
		// Strip an optional leading "us."/"eu."/"apac." inference-profile prefix so we can
		// match the real publisher segment.
		let tail = model_id.split_once('.').map(|(_, rest)| rest).unwrap_or(model_id);
		let publisher_segment = tail.split_once('.').map(|(p, _)| p).unwrap_or(tail);

		// For non-profile IDs, the leading segment IS the publisher.
		let publisher = if publisher_segment.is_empty() {
			model_id.split_once('.').map(|(p, _)| p).unwrap_or(model_id)
		} else {
			publisher_segment
		};

		match publisher {
			"anthropic" => Self::Anthropic,
			"amazon" => Self::AmazonNova, // Nova models; Titan would also hit this
			_ => Self::Other,
		}
	}
}

/// Build the JSON body for a Converse / ConverseStream call.
pub(super) fn build_converse_payload(
	model_iden: &ModelIden,
	chat_req: ChatRequest,
	options_set: ChatOptionsSet<'_, '_>,
) -> Result<Value> {
	let (_, model_name) = model_iden.model_name.namespace_and_name();
	let publisher = BedrockPublisher::from_model_id(model_name);

	let ConverseRequestParts {
		system,
		messages,
		tools,
	} = into_converse_request_parts(chat_req)?;

	let mut payload = json!({ "messages": messages });

	if let Some(system) = system {
		payload.x_insert("system", system)?;
	}

	if let Some(tools) = tools {
		payload.x_insert("toolConfig", json!({ "tools": tools }))?;
	}

	// inferenceConfig
	let mut inference: Map<String, Value> = Map::new();
	let max_tokens = resolve_max_tokens(model_name, &options_set);
	inference.insert("maxTokens".to_string(), json!(max_tokens));
	if let Some(temperature) = options_set.temperature() {
		inference.insert("temperature".to_string(), json!(temperature));
	}
	if let Some(top_p) = options_set.top_p() {
		inference.insert("topP".to_string(), json!(top_p));
	}
	if !options_set.stop_sequences().is_empty() {
		inference.insert("stopSequences".to_string(), json!(options_set.stop_sequences()));
	}
	payload.x_insert("inferenceConfig", Value::Object(inference))?;

	// additionalModelRequestFields — publisher-specific (reasoning, etc.)
	if let Some(effort) = options_set.reasoning_effort()
		&& let Some(additional) = publisher_additional_fields(publisher, effort)
	{
		payload.x_insert("additionalModelRequestFields", additional)?;
	}

	Ok(payload)
}

/// Parse a Converse JSON response into a genai `ChatResponse`.
pub(super) fn parse_converse_response(model_iden: ModelIden, web_response: WebResponse) -> Result<ChatResponse> {
	let WebResponse { mut body, .. } = web_response;

	// -- Stop reason
	let stop_reason = body
		.x_take::<Option<String>>("stopReason")
		.ok()
		.flatten()
		.map(|s| StopReason::from(normalize_stop_reason(s.as_str()).to_string()));

	// -- Usage
	let usage_value = body.x_take::<Value>("usage").ok();
	let usage = usage_value.map(parse_usage).unwrap_or_default();

	// -- Content — output.message.content is an array of blocks
	let content_items: Vec<Value> = body.x_take("/output/message/content").unwrap_or_default();

	let mut content: MessageContent = MessageContent::default();
	let mut reasoning_content: Vec<String> = Vec::new();

	for mut item in content_items {
		// Each item has exactly one field indicating block type.
		if let Ok(text) = item.x_take::<String>("text") {
			content.push(ContentPart::from_text(text));
		} else if let Ok(mut tool_use) = item.x_take::<Value>("toolUse") {
			let call_id = tool_use.x_take::<String>("toolUseId")?;
			let fn_name = tool_use.x_take::<String>("name")?;
			let fn_arguments = tool_use.x_take::<Value>("input").unwrap_or_default();
			content.push(ContentPart::ToolCall(ToolCall {
				call_id,
				fn_name,
				fn_arguments,
				thought_signatures: None,
			}));
		} else if let Ok(mut reasoning) = item.x_take::<Value>("reasoningContent") {
			// Converse reasoning block: { reasoningText: { text, signature? } }
			if let Ok(text) = reasoning.x_take::<String>("/reasoningText/text") {
				reasoning_content.push(text);
			}
		} else {
			// Unknown block type — preserve as custom for forward-compat.
			content.push(ContentPart::from_custom(item, Some(model_iden.clone())));
		}
	}

	let reasoning_content = if reasoning_content.is_empty() {
		None
	} else {
		Some(reasoning_content.join("\n"))
	};

	let provider_model_iden = model_iden.clone();

	Ok(ChatResponse {
		content,
		reasoning_content,
		model_iden,
		provider_model_iden,
		stop_reason,
		usage,
		captured_raw_body: None,
		response_id: None,
	})
}

/// Map Converse `stopReason` values onto the set expected by `StopReason::from`.
/// Converse values include: end_turn, tool_use, max_tokens, stop_sequence, guardrail_intervened, content_filtered.
pub(super) fn normalize_stop_reason(converse_reason: &str) -> &str {
	// genai's StopReason::from already handles common names ("end_turn", "tool_use", "max_tokens",
	// "stop_sequence"). Pass through as-is.
	converse_reason
}

/// Converse usage → genai `Usage`, whose `prompt_tokens` is the WHOLE prompt. Bedrock's
/// `inputTokens` counts only the uncached part: a 6,101-token prompt served from cache reports
/// `inputTokens: 7` beside `cacheReadInputTokens: 6094` (its `totalTokens` is the sum of all
/// three, measured on Kimi K3, 2026-09-23). Taken as the prompt size, that reads a nearly full
/// context as empty, so the three are added here.
pub(super) fn parse_usage(mut usage_value: Value) -> Usage {
	let uncached_input: i32 = usage_value.x_take("inputTokens").ok().unwrap_or(0);
	let output_tokens: i32 = usage_value.x_take("outputTokens").ok().unwrap_or(0);
	let cache_read: Option<i32> = usage_value.x_take("cacheReadInputTokens").ok();
	let cache_write: Option<i32> = usage_value.x_take("cacheWriteInputTokens").ok();
	let input_tokens = uncached_input + cache_read.unwrap_or(0) + cache_write.unwrap_or(0);
	let total_tokens: i32 = usage_value.x_take("totalTokens").ok().unwrap_or(input_tokens + output_tokens);

	let prompt_tokens_details = if cache_read.is_some() || cache_write.is_some() {
		Some(crate::chat::PromptTokensDetails {
			cache_creation_tokens: cache_write,
			cache_creation_details: None,
			cached_tokens: cache_read,
			audio_tokens: None,
		})
	} else {
		None
	};

	Usage {
		prompt_tokens: Some(input_tokens),
		prompt_tokens_details,
		completion_tokens: Some(output_tokens),
		completion_tokens_details: None,
		total_tokens: Some(total_tokens),
	}
}

fn resolve_max_tokens(model_name: &str, options_set: &ChatOptionsSet) -> u32 {
	options_set.max_tokens().unwrap_or_else(|| {
		// Conservative defaults by publisher; most Bedrock publishers require maxTokens in inferenceConfig.
		match BedrockPublisher::from_model_id(model_name) {
			BedrockPublisher::Anthropic => {
				// Mirror the Anthropic adapter's heuristics for parity.
				if model_name.contains("claude-sonnet")
					|| model_name.contains("claude-haiku")
					|| model_name.contains("claude-opus-4-5")
				{
					crate::adapter::adapters::anthropic::MAX_TOKENS_64K.max(64000)
				} else if model_name.contains("claude-opus-4") {
					32000
				} else if model_name.contains("claude-3-5") {
					8192
				} else {
					4096
				}
			}
			BedrockPublisher::AmazonNova => 5000,
			BedrockPublisher::Other => 4096,
		}
	})
}

fn publisher_additional_fields(publisher: BedrockPublisher, effort: &ReasoningEffort) -> Option<Value> {
	match publisher {
		BedrockPublisher::Anthropic => {
			let budget = match effort {
				ReasoningEffort::None => return None,
				ReasoningEffort::Budget(n) => *n,
				ReasoningEffort::Minimal | ReasoningEffort::Low => 1024,
				ReasoningEffort::Medium => 8000,
				ReasoningEffort::High | ReasoningEffort::XHigh | ReasoningEffort::Max => 24000,
			};
			Some(json!({
				"thinking": {
					"type": "enabled",
					"budget_tokens": budget,
				}
			}))
		}
		BedrockPublisher::AmazonNova => {
			// Nova surfaces reasoning via inferenceConfig.reasoningConfig today; when a user explicitly sets
			// ReasoningEffort, opt in.
			match effort {
				ReasoningEffort::None => None,
				_ => Some(json!({
					"inferenceConfig": { "reasoningConfig": { "type": "enabled" } }
				})),
			}
		}
		BedrockPublisher::Other => None,
	}
}

struct ConverseRequestParts {
	system: Option<Value>,
	messages: Vec<Value>,
	tools: Option<Vec<Value>>,
}

/// Translate a genai `ChatRequest` into Converse's `{system, messages, toolConfig}` shape.
fn into_converse_request_parts(chat_req: ChatRequest) -> Result<ConverseRequestParts> {
	let mut messages: Vec<Value> = Vec::new();
	let mut systems: Vec<(String, bool)> = Vec::new();

	if let Some(system) = chat_req.system {
		systems.push((system, false));
	}

	for msg in chat_req.messages {
		let cache_point = msg
			.options
			.as_ref()
			.and_then(|options| options.cache_control.as_ref())
			.is_some();
		match msg.role {
			ChatRole::System => {
				if let Some(text) = msg.content.joined_texts() {
					systems.push((text, cache_point));
				}
			}
			ChatRole::User => {
				let blocks = with_cache_point(user_content_to_converse_blocks(msg.content), cache_point);
				if !blocks.is_empty() {
					messages.push(json!({ "role": "user", "content": blocks }));
				}
			}
			ChatRole::Assistant => {
				let blocks = with_cache_point(assistant_content_to_converse_blocks(msg.content), cache_point);
				if !blocks.is_empty() {
					messages.push(json!({ "role": "assistant", "content": blocks }));
				}
			}
			ChatRole::Tool => {
				// Tool responses become a user message whose content is tool_result blocks.
				let blocks = with_cache_point(tool_content_to_converse_blocks(msg.content), cache_point);
				if !blocks.is_empty() {
					messages.push(json!({ "role": "user", "content": blocks }));
				}
			}
		}
	}

	let system = if systems.is_empty() {
		None
	} else {
		// Converse accepts cachePoint blocks alongside system text blocks. The point caches every
		// stable block before it. Callers retry without the optional points when an older model or
		// endpoint explicitly reports that it does not support prompt caching.
		let parts: Vec<Value> = systems
			.into_iter()
			.flat_map(|(text, cache_point)| {
				let mut blocks = vec![json!({ "text": text })];
				if cache_point {
					blocks.push(json!({ "cachePoint": { "type": "default" } }));
				}
				blocks
			})
			.collect();
		Some(Value::Array(parts))
	};

	let tools: Option<Vec<Value>> = chat_req
		.tools
		.map(|tools| tools.into_iter().map(tool_to_converse_tool).collect::<Result<Vec<Value>>>())
		.transpose()?;

	// Converse accepts toolUse/toolResult blocks only beside a toolConfig ("The toolConfig field must
	// be defined when using toolUse and toolResult content blocks"). A tool-less call — a summary,
	// a recap, compaction — over a transcript with tool history carries the history as text.
	let tools = tools.filter(|tools| !tools.is_empty());
	let messages = if tools.is_some() {
		messages
	} else {
		tool_blocks_as_text(messages)
	};

	Ok(ConverseRequestParts {
		system,
		messages: pair_tool_turns(messages),
		tools,
	})
}

fn tool_blocks_as_text(messages: Vec<Value>) -> Vec<Value> {
	messages
		.into_iter()
		.map(|mut message| {
			if let Some(blocks) = message.get_mut("content").and_then(Value::as_array_mut) {
				for block in blocks.iter_mut() {
					if let Some(tool_use) = block.get("toolUse") {
						let name = tool_use.get("name").and_then(Value::as_str).unwrap_or("tool");
						let input = tool_use.get("input").map(Value::to_string).unwrap_or_default();
						*block = json!({ "text": format!("[called {name} with {input}]") });
					} else if let Some(result) = block.get("toolResult") {
						let text = result.pointer("/content/0/text").and_then(Value::as_str).unwrap_or_default();
						*block = json!({ "text": format!("[tool result]\n{text}") });
					}
				}
			}
			message
		})
		.collect()
}

/// Reshape a transcript into the strict turn grammar Converse enforces, which OpenAI-style APIs do
/// not: roles alternate, and every `toolUse` is answered in the very next user message by a
/// `toolResult` with its id — and nothing else claims to be a result.
///
/// Parallel tool calls arrived as one user message PER result, so the first message answered one
/// id and Bedrock rejected the turn ("Expected toolResult blocks at messages.14.content for the
/// following Ids: …"). A call left unanswered by an interrupted or failed tool killed every later
/// request of the session the same way, even one that was fine on the model that made it.
fn pair_tool_turns(messages: Vec<Value>) -> Vec<Value> {
	let mut merged: Vec<(String, Vec<Value>)> = Vec::new();
	for message in messages {
		let role = message.get("role").and_then(Value::as_str).unwrap_or("user").to_string();
		let blocks = message.get("content").and_then(Value::as_array).cloned().unwrap_or_default();
		match merged.last_mut() {
			Some((last_role, last_blocks)) if *last_role == role => last_blocks.extend(blocks),
			_ => merged.push((role, blocks)),
		}
	}

	let mut out: Vec<(String, Vec<Value>)> = Vec::with_capacity(merged.len() + 1);
	let mut open_calls: Vec<String> = Vec::new();
	for (role, blocks) in merged {
		if role == "assistant" {
			if !open_calls.is_empty() {
				out.push(("user".to_string(), answer_missing(Vec::new(), &mut open_calls)));
			}
			open_calls = blocks.iter().filter_map(tool_use_id).collect();
			out.push((role, blocks));
		} else {
			out.push((role, answer_missing(blocks, &mut open_calls)));
		}
	}
	if !open_calls.is_empty() {
		out.push(("user".to_string(), answer_missing(Vec::new(), &mut open_calls)));
	}
	if out.first().is_some_and(|(role, _)| role == "assistant") {
		out.insert(
			0,
			("user".to_string(), vec![json!({ "text": "(conversation continues)" })]),
		);
	}
	out.into_iter()
		.map(|(role, content)| json!({ "role": role, "content": content }))
		.collect()
}

fn tool_use_id(block: &Value) -> Option<String> {
	block.pointer("/toolUse/toolUseId").and_then(Value::as_str).map(str::to_string)
}

/// A user turn that answers exactly `open_calls`: results first (the order Bedrock's Anthropic
/// models require), a stand-in error result for any call that never got one, and any result whose
/// call is not open kept as plain text rather than as a result for nothing.
fn answer_missing(blocks: Vec<Value>, open_calls: &mut Vec<String>) -> Vec<Value> {
	let mut results = Vec::new();
	let mut rest = Vec::new();
	for block in blocks {
		match block.pointer("/toolResult/toolUseId").and_then(Value::as_str) {
			Some(id) if open_calls.iter().any(|open| open == id) => {
				let id = id.to_string();
				open_calls.retain(|open| *open != id);
				results.push(block);
			}
			Some(id) => {
				let text = block
					.pointer("/toolResult/content/0/text")
					.and_then(Value::as_str)
					.unwrap_or_default();
				rest.push(json!({ "text": format!("[result of tool call {id}]\n{text}") }));
			}
			None => rest.push(block),
		}
	}
	for id in open_calls.drain(..) {
		results.push(json!({
			"toolResult": {
				"toolUseId": id,
				"content": [{ "text": "This tool call did not complete, so it has no result." }],
				"status": "error",
			}
		}));
	}
	results.extend(rest);
	results
}

/// Append an AWS Bedrock Converse cache point after a message's content. Forge marks only stable
/// transcript boundaries, and Bedrock treats the point as a request hint rather than user content.
fn with_cache_point(mut blocks: Vec<Value>, enabled: bool) -> Vec<Value> {
	if enabled && !blocks.is_empty() {
		blocks.push(json!({ "cachePoint": { "type": "default" } }));
	}
	blocks
}

fn user_content_to_converse_blocks(content: MessageContent) -> Vec<Value> {
	let mut blocks = Vec::new();
	for part in content {
		match part {
			ContentPart::Text(text) => blocks.push(json!({ "text": text })),
			ContentPart::Binary(binary) => {
				if let Some(block) = binary_to_converse_block(binary) {
					blocks.push(block);
				}
			}
			ContentPart::ToolResponse(tool_response) => {
				blocks.push(json!({
					"toolResult": {
						"toolUseId": tool_response.call_id,
						"content": [{ "text": tool_response.content }],
					}
				}));
			}
			// Not valid in user role for Converse — skip.
			ContentPart::ToolCall(_) => {}
			ContentPart::ThoughtSignature(_) => {}
			ContentPart::ReasoningContent(_) => {}
			ContentPart::Custom(_) => {}
		}
	}
	blocks
}

fn assistant_content_to_converse_blocks(content: MessageContent) -> Vec<Value> {
	let mut blocks = Vec::new();
	for part in content {
		match part {
			ContentPart::Text(text) => blocks.push(json!({ "text": text })),
			ContentPart::ToolCall(tool_call) => {
				let input = if tool_call.fn_arguments.is_null() {
					Value::Object(Map::new())
				} else {
					tool_call.fn_arguments
				};
				blocks.push(json!({
					"toolUse": {
						"toolUseId": tool_call.call_id,
						"name": tool_call.fn_name,
						"input": input,
					}
				}));
			}
			// Unsupported in assistant role for Converse.
			ContentPart::Binary(_) => {}
			ContentPart::ToolResponse(_) => {}
			ContentPart::ThoughtSignature(_) => {}
			ContentPart::ReasoningContent(_) => {}
			ContentPart::Custom(_) => {}
		}
	}
	blocks
}

fn tool_content_to_converse_blocks(content: MessageContent) -> Vec<Value> {
	let mut blocks = Vec::new();
	for part in content {
		if let ContentPart::ToolResponse(tool_response) = part {
			blocks.push(json!({
				"toolResult": {
					"toolUseId": tool_response.call_id,
					"content": [{ "text": tool_response.content }],
				}
			}));
		}
	}
	blocks
}

fn binary_to_converse_block(binary: Binary) -> Option<Value> {
	let is_image = binary.is_image();
	let Binary {
		content_type, source, ..
	} = binary;

	// Converse format: image blocks use { image: { format, source: { bytes } } }
	// and document blocks use { document: { format, name, source: { bytes } } }.
	// URL-based sources aren't supported here yet.
	let data = match source {
		BinarySource::Base64(data) => data,
		BinarySource::Url(_) => {
			warn!("Bedrock Converse: URL-based binary sources are not yet supported, skipping");
			return None;
		}
	};

	let format = converse_format_from_content_type(&content_type, is_image)?;

	if is_image {
		Some(json!({
			"image": {
				"format": format,
				"source": { "bytes": data },
			}
		}))
	} else {
		Some(json!({
			"document": {
				"format": format,
				"name": "document",
				"source": { "bytes": data },
			}
		}))
	}
}

fn converse_format_from_content_type(content_type: &str, is_image: bool) -> Option<&'static str> {
	if is_image {
		match content_type {
			"image/jpeg" | "image/jpg" => Some("jpeg"),
			"image/png" => Some("png"),
			"image/gif" => Some("gif"),
			"image/webp" => Some("webp"),
			_ => {
				warn!("Bedrock Converse: unsupported image content-type: {content_type}");
				None
			}
		}
	} else {
		match content_type {
			"application/pdf" => Some("pdf"),
			"text/csv" => Some("csv"),
			"application/msword" => Some("doc"),
			"application/vnd.openxmlformats-officedocument.wordprocessingml.document" => Some("docx"),
			"application/vnd.ms-excel" => Some("xls"),
			"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" => Some("xlsx"),
			"text/html" => Some("html"),
			"text/plain" => Some("txt"),
			"text/markdown" => Some("md"),
			_ => {
				warn!("Bedrock Converse: unsupported document content-type: {content_type}");
				None
			}
		}
	}
}

fn tool_to_converse_tool(tool: Tool) -> Result<Value> {
	let Tool {
		name,
		description,
		schema,
		..
	} = tool;

	let name = match name {
		ToolName::Custom(name) => name,
		ToolName::WebSearch => {
			return Err(Error::AdapterNotSupported {
				adapter_kind: crate::adapter::AdapterKind::BedrockApi,
				feature: "web_search builtin tool".to_string(),
			});
		}
	};

	let mut tool_spec = json!({
		"name": name,
		"inputSchema": { "json": schema },
	});
	if let Some(description) = description {
		tool_spec.x_insert("description", description)?;
	}

	Ok(json!({ "toolSpec": tool_spec }))
}

#[cfg(test)]
mod tool_pairing_tests {
	use super::*;

	fn use_block(id: &str) -> Value {
		json!({ "toolUse": { "toolUseId": id, "name": "shell", "input": {} } })
	}
	fn result_block(id: &str) -> Value {
		json!({ "toolResult": { "toolUseId": id, "content": [{ "text": "ok" }] } })
	}
	fn msg(role: &str, content: Vec<Value>) -> Value {
		json!({ "role": role, "content": content })
	}
	fn result_ids(message: &Value) -> Vec<String> {
		message["content"]
			.as_array()
			.unwrap()
			.iter()
			.filter_map(|b| b.pointer("/toolResult/toolUseId").and_then(Value::as_str).map(str::to_string))
			.collect()
	}

	#[test]
	fn a_tool_less_request_carries_tool_history_as_text() {
		let messages = vec![
			crate::chat::ChatMessage::user("go"),
			crate::chat::ChatMessage::from(vec![crate::chat::ToolCall {
				call_id: "c1".into(),
				fn_name: "shell".into(),
				fn_arguments: json!({ "command": "ls" }),
				thought_signatures: None,
			}]),
			crate::chat::ChatMessage::from(crate::chat::ToolResponse::new("c1", "a.txt")),
			crate::chat::ChatMessage::user("summarise"),
		];
		let parts = into_converse_request_parts(ChatRequest::new(messages)).unwrap();
		assert!(parts.tools.is_none());
		let body = serde_json::to_string(&parts.messages).unwrap();
		assert!(!body.contains("toolUse") && !body.contains("toolResult"), "{body}");
		assert!(body.contains("[called shell with"), "{body}");
		assert!(body.contains("a.txt"), "{body}");
	}

	#[test]
	fn parallel_results_share_one_user_turn() {
		let out = pair_tool_turns(vec![
			msg("user", vec![json!({ "text": "go" })]),
			msg("assistant", vec![use_block("a"), use_block("b")]),
			msg("user", vec![result_block("a")]),
			msg("user", vec![result_block("b")]),
		]);
		assert_eq!(out.len(), 3);
		assert_eq!(result_ids(&out[2]), vec!["a", "b"]);
	}

	#[test]
	fn an_unanswered_call_gets_an_error_result_before_the_next_user_text() {
		let out = pair_tool_turns(vec![
			msg("user", vec![json!({ "text": "go" })]),
			msg("assistant", vec![use_block("lost")]),
			msg("user", vec![json!({ "text": "next request" })]),
		]);
		assert_eq!(out.len(), 3);
		assert_eq!(result_ids(&out[2]), vec!["lost"]);
		assert_eq!(out[2]["content"][0]["toolResult"]["status"], "error");
		assert_eq!(out[2]["content"][1]["text"], "next request");
	}

	#[test]
	fn a_call_followed_by_another_assistant_turn_is_answered_in_between() {
		let out = pair_tool_turns(vec![
			msg("user", vec![json!({ "text": "go" })]),
			msg("assistant", vec![use_block("lost")]),
			msg("assistant", vec![json!({ "text": "continuing" })]),
		]);
		// Same-role messages merge, so the stray text joins the call's turn; the call is then
		// answered at the end rather than left open.
		assert_eq!(out.last().unwrap()["role"], "user");
		assert_eq!(result_ids(out.last().unwrap()), vec!["lost"]);
	}

	#[test]
	fn a_result_without_its_call_becomes_text_and_roles_start_with_user() {
		let out = pair_tool_turns(vec![
			msg("assistant", vec![json!({ "text": "earlier summary" })]),
			msg("user", vec![result_block("orphan")]),
		]);
		assert_eq!(out[0]["role"], "user");
		assert_eq!(out[1]["role"], "assistant");
		assert!(result_ids(&out[2]).is_empty());
		assert!(out[2]["content"][0]["text"].as_str().unwrap().contains("orphan"));
	}
}

#[cfg(test)]
mod cache_tests {
	use super::*;
	use crate::chat::{CacheControl, ChatMessage};

	#[test]
	fn message_cache_controls_become_converse_cache_points() {
		let messages = vec![
			ChatMessage::system("stable system").with_options(CacheControl::Ephemeral),
			ChatMessage::user("stable user prefix").with_options(CacheControl::Ephemeral),
		];
		let parts = into_converse_request_parts(ChatRequest::new(messages)).unwrap();
		let system = parts.system.unwrap();
		assert_eq!(system[0]["text"], "stable system");
		assert_eq!(system[1]["cachePoint"]["type"], "default");
		assert_eq!(parts.messages[0]["content"][0]["text"], "stable user prefix");
		assert_eq!(parts.messages[0]["content"][1]["cachePoint"]["type"], "default");
	}

	#[test]
	fn unmarked_messages_do_not_gain_cache_points() {
		let parts = into_converse_request_parts(ChatRequest::new(vec![ChatMessage::user("hi")])).unwrap();
		assert_eq!(parts.messages[0]["content"].as_array().unwrap().len(), 1);
	}
}
