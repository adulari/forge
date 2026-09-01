use crate::adapter::adapters::openai_resp::OpenAIRespAdapter;
use crate::adapter::adapters::support::get_api_key;
use crate::adapter::anthropic::{AnthropicAdapter, AnthropicRequestParts};
use crate::adapter::openai::OpenAIAdapter;
use crate::adapter::{Adapter, AdapterKind, ServiceType, WebRequestData};
use crate::chat::{ChatOptionsSet, ChatRequest, ChatResponse, ChatStreamResponse};
use crate::embed::{EmbedOptionsSet, EmbedRequest, EmbedResponse};
use crate::resolver::{AuthData, Endpoint};
use crate::webc::WebResponse;
use crate::{Error, Headers, ModelIden, Result, ServiceTarget};
use reqwest::RequestBuilder;
use serde_json::json;
use value_ext::JsonValueExt;

pub struct OpenCodeGoAdapter;

impl OpenCodeGoAdapter {
	pub const API_KEY_DEFAULT_ENV_NAME: &str = "OPENCODE_GO_API_KEY";
}

// region:    --- OpenCodeGoModelKind

/// Internal enum to dispatch wire format based on model name prefix.
///
/// OpenCode Go serves each model on ONE of three wire formats and `GET /models` does not say
/// which (`{"id","object","created","owned_by":"opencode"}` only). Sending a model to the wrong
/// one is an instant, deterministic failure — measured 2026-09-02 against all 33 Go models:
/// `gpt-5.6-luna`, `grok-4.5`, `grok-4.6` and `muse-spark-1.2-contributor` answer 500/503/401 on
/// `chat/completions` and 500 on `messages`, and only work on `responses` (OpenAI Responses API).
/// Every other model works on `chat/completions` (MiniMax additionally on `messages`) and
/// 401/500s on `responses`. So:
/// - MiniMax models use the Anthropic protocol (`messages`);
/// - the OpenAI-family models (`gpt-*`, `grok-*`, `muse-*`) use the Responses protocol;
/// - everything else, including models the proxy adds later, uses Chat Completions.
///
/// A model that turns out to need the Responses path can be registered at runtime with
/// [`mark_responses_model`]; the caller learns it from the failure shape above.
enum OpenCodeGoModelKind {
	OpenAI,
	Anthropic,
	Responses,
}

/// Runtime overrides for models learned to need the Responses path (see
/// [`OpenCodeGoModelKind`]). Process-wide because the adapter is stateless by design.
fn responses_overrides() -> &'static std::sync::RwLock<std::collections::HashSet<String>> {
	static OVERRIDES: std::sync::OnceLock<std::sync::RwLock<std::collections::HashSet<String>>> =
		std::sync::OnceLock::new();
	OVERRIDES.get_or_init(|| std::sync::RwLock::new(std::collections::HashSet::new()))
}

/// Register `model_name` (bare, no `opencode_go::` namespace) as served on the Responses API.
/// Idempotent. Returns `true` when this call changed the routing for the model.
pub fn mark_responses_model(model_name: &str) -> bool {
	let lower = model_name.to_lowercase();
	if OpenCodeGoModelKind::seeded_responses(&lower) {
		return false;
	}
	responses_overrides().write().map(|mut set| set.insert(lower)).unwrap_or(false)
}

/// Undo [`mark_responses_model`] for a model whose Responses retry did not answer either —
/// a genuine outage on a chat-only model must not leave it pinned to a path that would then
/// fail on every later request. Seeded families cannot be unmarked. Returns `true` on change.
pub fn unmark_responses_model(model_name: &str) -> bool {
	let lower = model_name.to_lowercase();
	responses_overrides().write().map(|mut set| set.remove(&lower)).unwrap_or(false)
}

/// True when `model_name` is routed to the Responses API (seeded or learned).
pub fn is_responses_model(model_name: &str) -> bool {
	matches!(
		OpenCodeGoModelKind::from_model_name(model_name),
		OpenCodeGoModelKind::Responses
	)
}

impl OpenCodeGoModelKind {
	/// The measured Responses-only families. Prefix-matched so a `grok-4.7` or `gpt-5.7-*` the
	/// proxy adds later lands on the right path without a code change.
	fn seeded_responses(lower: &str) -> bool {
		lower.starts_with("gpt-") || lower.starts_with("grok-") || lower.starts_with("muse-")
	}

	fn from_model_name(name: &str) -> Self {
		let lower = name.to_lowercase();
		if lower.starts_with("minimax-") {
			Self::Anthropic
		} else if Self::seeded_responses(&lower)
			|| responses_overrides().read().map(|set| set.contains(&lower)).unwrap_or(false)
		{
			Self::Responses
		} else {
			Self::OpenAI
		}
	}
}

// endregion: --- OpenCodeGoModelKind

impl Adapter for OpenCodeGoAdapter {
	const DEFAULT_API_KEY_ENV_NAME: Option<&'static str> = Some(Self::API_KEY_DEFAULT_ENV_NAME);

	fn default_auth() -> AuthData {
		match Self::DEFAULT_API_KEY_ENV_NAME {
			Some(env_name) => AuthData::from_env(env_name),
			None => AuthData::None,
		}
	}

	fn default_endpoint() -> Endpoint {
		Endpoint::from_static("https://opencode.ai/zen/go/v1/")
	}

	async fn all_model_names(kind: AdapterKind, endpoint: Endpoint, auth: AuthData) -> Result<Vec<String>> {
		OpenAIAdapter::list_model_names_for_end_target(kind, endpoint, auth).await
	}

	fn get_service_url(model: &ModelIden, _service_type: ServiceType, endpoint: Endpoint) -> Result<String> {
		let base_url = endpoint.base_url();
		let (_, model_name) = model.model_name.namespace_and_name();
		let model_kind = OpenCodeGoModelKind::from_model_name(model_name);
		let suffix = match model_kind {
			OpenCodeGoModelKind::OpenAI => "chat/completions",
			OpenCodeGoModelKind::Anthropic => "messages",
			OpenCodeGoModelKind::Responses => "responses",
		};
		Ok(format!("{base_url}{suffix}"))
	}

	fn to_web_request_data(
		target: ServiceTarget,
		service_type: ServiceType,
		chat_req: ChatRequest,
		options_set: ChatOptionsSet<'_, '_>,
	) -> Result<WebRequestData> {
		let ServiceTarget { endpoint, auth, model } = target;
		let (_, model_name) = model.model_name.namespace_and_name();
		let model_kind = OpenCodeGoModelKind::from_model_name(model_name);

		match model_kind {
			OpenCodeGoModelKind::OpenAI => OpenAIAdapter::util_to_web_request_data(
				ServiceTarget { endpoint, auth, model },
				service_type,
				chat_req,
				options_set,
				None,
			),
			// The Responses adapter resolves the URL through the dispatcher, i.e. back through
			// `Self::get_service_url`, so the Go base URL + `responses` suffix is what it hits.
			OpenCodeGoModelKind::Responses => OpenAIRespAdapter::to_web_request_data(
				ServiceTarget { endpoint, auth, model },
				service_type,
				chat_req,
				options_set,
			),
			OpenCodeGoModelKind::Anthropic => {
				let model_name = model_name.to_string();

				let AnthropicRequestParts {
					system,
					messages,
					tools,
				} = AnthropicAdapter::into_anthropic_request_parts(chat_req)?;

				let stream = matches!(service_type, ServiceType::ChatStream);
				let mut payload = json!({
					"model": model_name,
					"messages": messages,
					"stream": stream,
				});

				if let Some(system) = system {
					payload.x_insert("system", system)?;
				}

				if let Some(tools) = tools {
					payload.x_insert("tools", tools)?;
				}

				if let Some(temperature) = options_set.temperature() {
					payload.x_insert("temperature", temperature)?;
				}

				if let Some(top_p) = options_set.top_p() {
					payload.x_insert("top_p", top_p)?;
				}

				if !options_set.stop_sequences().is_empty() {
					payload.x_insert("stop_sequences", options_set.stop_sequences())?;
				}

				if let Some(max_tokens) = options_set.max_tokens() {
					payload.x_insert("max_tokens", max_tokens)?;
				}

				// MiniMax /v1/messages requires `x-api-key` (Bearer returns 401).
				let api_key = get_api_key(auth, &model)?;
				let headers = Headers::from(("x-api-key".to_string(), api_key));

				let url = Self::get_service_url(&model, service_type, endpoint)?;

				Ok(WebRequestData { url, headers, payload })
			}
		}
	}

	fn to_chat_response(
		model_iden: ModelIden,
		web_response: WebResponse,
		options_set: ChatOptionsSet<'_, '_>,
	) -> Result<ChatResponse> {
		let (_, model_name) = model_iden.model_name.namespace_and_name();
		let model_kind = OpenCodeGoModelKind::from_model_name(model_name);

		match model_kind {
			OpenCodeGoModelKind::OpenAI => OpenAIAdapter::to_chat_response(model_iden, web_response, options_set),
			OpenCodeGoModelKind::Responses => {
				OpenAIRespAdapter::to_chat_response(model_iden, web_response, options_set)
			}
			OpenCodeGoModelKind::Anthropic => AnthropicAdapter::to_chat_response(model_iden, web_response, options_set),
		}
	}

	fn to_chat_stream(
		model_iden: ModelIden,
		reqwest_builder: RequestBuilder,
		options_set: ChatOptionsSet<'_, '_>,
	) -> Result<ChatStreamResponse> {
		let (_, model_name) = model_iden.model_name.namespace_and_name();
		let model_kind = OpenCodeGoModelKind::from_model_name(model_name);

		match model_kind {
			OpenCodeGoModelKind::OpenAI => OpenAIAdapter::to_chat_stream(model_iden, reqwest_builder, options_set),
			OpenCodeGoModelKind::Responses => {
				OpenAIRespAdapter::to_chat_stream(model_iden, reqwest_builder, options_set)
			}
			OpenCodeGoModelKind::Anthropic => {
				AnthropicAdapter::to_chat_stream(model_iden, reqwest_builder, options_set)
			}
		}
	}

	fn to_embed_request_data(
		_service_target: ServiceTarget,
		_embed_req: EmbedRequest,
		_options_set: EmbedOptionsSet<'_, '_>,
	) -> Result<WebRequestData> {
		Err(Error::AdapterNotSupported {
			adapter_kind: AdapterKind::OpenCodeGo,
			feature: "embeddings".to_string(),
		})
	}

	fn to_embed_response(
		_model_iden: ModelIden,
		_web_response: WebResponse,
		_options_set: EmbedOptionsSet<'_, '_>,
	) -> Result<EmbedResponse> {
		Err(Error::AdapterNotSupported {
			adapter_kind: AdapterKind::OpenCodeGo,
			feature: "embeddings".to_string(),
		})
	}
}

// region:    --- Tests

#[cfg(test)]
mod tests {
	use super::*;
	use crate::ServiceTarget;
	use crate::adapter::{Adapter, ServiceType};
	use crate::chat::{ChatOptions, ChatOptionsSet, ChatRequest};
	use crate::embed::{EmbedOptionsSet, EmbedRequest};
	use crate::resolver::AuthData;

	fn test_target(model_name: &str) -> ServiceTarget {
		ServiceTarget {
			endpoint: OpenCodeGoAdapter::default_endpoint(),
			auth: AuthData::from_single("test-key"),
			model: ModelIden::new(AdapterKind::OpenCodeGo, model_name),
		}
	}

	fn make_request(model_name: &str, service_type: ServiceType) -> WebRequestData {
		OpenCodeGoAdapter::to_web_request_data(
			test_target(model_name),
			service_type,
			ChatRequest::from_user("hello"),
			ChatOptionsSet::default(),
		)
		.expect("to_web_request_data should succeed")
	}

	#[test]
	fn test_url_openai_path() {
		let data = make_request("glm-5", ServiceType::Chat);
		assert!(
			data.url.ends_with("chat/completions"),
			"OpenAI path URL should end with chat/completions: {}",
			data.url
		);
	}

	/// The four models measured Responses-only (2026-09-02) must not be sent to chat/completions,
	/// where they answer an instant 500/503/401 — the failure a pinned turn spun on for 6m47s.
	#[test]
	fn test_url_responses_path_for_openai_family() {
		for name in ["gpt-5.6-luna", "grok-4.6", "grok-4.5", "muse-spark-1.2-contributor"] {
			let data = make_request(name, ServiceType::Chat);
			assert!(
				data.url.ends_with("responses"),
				"{name} must use the Responses path, got {}",
				data.url
			);
			// The Responses body shape: `input`, never `messages`.
			assert!(
				data.payload.get("input").is_some(),
				"{name}: Responses payload carries `input`"
			);
			assert!(
				data.payload.get("messages").is_none(),
				"{name}: not a chat/completions payload"
			);
			assert!(
				data.url.starts_with("https://opencode.ai/zen/go/v1/"),
				"{name}: Go base URL kept"
			);
		}
	}

	#[test]
	fn test_responses_auth_is_bearer() {
		let data = make_request("muse-spark-1.2-contributor", ServiceType::Chat);
		let auth = data
			.headers
			.iter()
			.find(|(k, _)| k.eq_ignore_ascii_case("Authorization"))
			.map(|(_, v)| v.as_str());
		assert_eq!(auth, Some("Bearer test-key"));
	}

	/// A model learned at runtime to need the Responses path is honored on the next request;
	/// seeded families report "no change". Uses a name no seed matches.
	#[test]
	fn test_learned_responses_override() {
		assert!(make_request("hy9-future", ServiceType::Chat).url.ends_with("chat/completions"));
		assert!(mark_responses_model("hy9-future"));
		assert!(!mark_responses_model("hy9-future"), "idempotent");
		assert!(!mark_responses_model("gpt-5.6-luna"), "seeded family needs no override");
		assert!(is_responses_model("HY9-Future"), "case-insensitive");
		assert!(make_request("hy9-future", ServiceType::Chat).url.ends_with("responses"));
	}

	#[test]
	fn test_url_minimax_path() {
		let data = make_request("minimax-m2.5", ServiceType::Chat);
		assert!(
			data.url.ends_with("messages"),
			"Minimax path URL should end with messages: {}",
			data.url
		);
	}

	#[test]
	fn test_auth_header_openai() {
		let data = make_request("glm-5", ServiceType::Chat);
		let auth = data
			.headers
			.iter()
			.find(|(k, _)| k.eq_ignore_ascii_case("Authorization"))
			.map(|(_, v)| v.as_str());
		assert_eq!(auth, Some("Bearer test-key"));
	}

	#[test]
	fn test_auth_header_minimax() {
		let data = make_request("minimax-m2.5", ServiceType::Chat);
		let auth = data
			.headers
			.iter()
			.find(|(k, _)| k.eq_ignore_ascii_case("Authorization"))
			.map(|(_, v)| v.as_str());
		assert_eq!(auth, None, "Minimax should not have Authorization header");
		let x_key = data
			.headers
			.iter()
			.find(|(k, _)| k.eq_ignore_ascii_case("x-api-key"))
			.map(|(_, v)| v.as_str());
		assert_eq!(x_key, Some("test-key"));
	}

	#[test]
	fn test_no_x_api_key_in_openai_path() {
		let data = make_request("glm-5", ServiceType::Chat);
		let x_key = data
			.headers
			.iter()
			.find(|(k, _)| k.eq_ignore_ascii_case("x-api-key"))
			.map(|(_, v)| v.as_str());
		assert_eq!(x_key, None, "OpenAI path should not have x-api-key header");
	}

	#[test]
	fn test_x_api_key_in_minimax_path() {
		let data = make_request("minimax-m2.5", ServiceType::Chat);
		let x_key = data
			.headers
			.iter()
			.find(|(k, _)| k.eq_ignore_ascii_case("x-api-key"))
			.map(|(_, v)| v.as_str());
		assert_eq!(x_key, Some("test-key"));
	}

	#[test]
	fn test_payload_model_name_openai() {
		let data = make_request("glm-5", ServiceType::Chat);
		let model = data.payload.get("model").and_then(|v| v.as_str());
		assert_eq!(model, Some("glm-5"));
	}

	#[test]
	fn test_payload_model_name_minimax() {
		let data = make_request("minimax-m2.5", ServiceType::Chat);
		let model = data.payload.get("model").and_then(|v| v.as_str());
		assert_eq!(model, Some("minimax-m2.5"));
	}

	#[test]
	fn test_payload_messages_array() {
		for (name, model_name) in [("OpenAI", "glm-5"), ("Minimax", "minimax-m2.5")] {
			let data = make_request(model_name, ServiceType::Chat);
			let messages = data.payload.get("messages").and_then(|v| v.as_array());
			assert!(messages.is_some(), "{name} payload should have messages array");
			let messages = messages.unwrap();
			assert!(!messages.is_empty(), "{name} messages array should not be empty");
			let last = messages.last().unwrap();
			let content = last.get("content").and_then(|v| v.as_str());
			assert_eq!(content, Some("hello"), "{name} last message content mismatch");
		}
	}

	#[test]
	fn test_payload_stream_false_for_chat() {
		for (name, model_name) in [("OpenAI", "glm-5"), ("Minimax", "minimax-m2.5")] {
			let data = make_request(model_name, ServiceType::Chat);
			let stream = data.payload.get("stream").and_then(|v| v.as_bool());
			assert_eq!(stream, Some(false), "{name} stream should be false for Chat");
		}
	}

	#[test]
	fn test_payload_stream_true_for_chat_stream() {
		for (name, model_name) in [("OpenAI", "glm-5"), ("Minimax", "minimax-m2.5")] {
			let data = make_request(model_name, ServiceType::ChatStream);
			let stream = data.payload.get("stream").and_then(|v| v.as_bool());
			assert_eq!(stream, Some(true), "{name} stream should be true for ChatStream");
		}
	}

	#[test]
	fn test_minimax_prefix_case_insensitive() {
		let data = make_request("MINIMAX-M2.5", ServiceType::Chat);
		assert!(
			data.url.ends_with("messages"),
			"MINIMAX-M2.5 should route to messages URL: {}",
			data.url
		);

		let data = make_request("Minimax-m2.5", ServiceType::Chat);
		assert!(
			data.url.ends_with("messages"),
			"Minimax-m2.5 should route to messages URL: {}",
			data.url
		);
	}

	#[test]
	fn test_minimax_payload_with_options() {
		let target = test_target("minimax-m2.5");
		let chat_options = ChatOptions::default().with_temperature(0.5).with_max_tokens(100);
		let options_set = ChatOptionsSet::default().with_chat_options(Some(&chat_options));
		let data = OpenCodeGoAdapter::to_web_request_data(
			target,
			ServiceType::Chat,
			ChatRequest::from_user("hello").with_system("system-prompt"),
			options_set,
		)
		.expect("to_web_request_data should succeed");

		assert_eq!(
			data.payload.get("temperature").and_then(|v| v.as_f64()),
			Some(0.5),
			"temperature should be present"
		);
		assert_eq!(
			data.payload.get("max_tokens").and_then(|v| v.as_u64()),
			Some(100),
			"max_tokens should be present"
		);
		assert_eq!(
			data.payload.get("system").and_then(|v| v.as_str()),
			Some("system-prompt"),
			"system should be present"
		);
	}

	#[test]
	fn test_embed_not_supported() {
		let result = OpenCodeGoAdapter::to_embed_request_data(
			test_target("glm-5"),
			EmbedRequest::new("test"),
			EmbedOptionsSet::default(),
		);
		assert!(result.is_err(), "embed should not be supported");
		match result.unwrap_err() {
			Error::AdapterNotSupported { adapter_kind, feature } => {
				assert_eq!(adapter_kind, AdapterKind::OpenCodeGo);
				assert_eq!(feature, "embeddings");
			}
			_ => panic!("Expected AdapterNotSupported error"),
		}
	}
}

// endregion: --- Tests
