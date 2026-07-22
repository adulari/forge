//! The Adapter layer allows adapting client requests/responses to various AI providers.
//! Currently, it employs a static dispatch pattern with the `Adapter` trait and `AdapterDispatcher` implementation.
//! Adapter implementations are organized by adapter type under the `adapters` submodule.
//!
//! Notes:
//! - All `Adapter` trait methods take the `AdapterKind` as an argument, and for now, the `Adapter` trait functions
//!   are all static (i.e., no `&self`). This reduces state management and ensures that all states are passed as arguments.
//! - Only `AdapterKind` from `AdapterConfig` is publicly exported.

// region:    --- Modules

mod adapter_kind;
mod adapter_types;
mod adapters;
mod dispatcher;
mod dispatcher_macros;

// -- Flatten (private, crate, public)
use adapters::*;

pub(crate) use adapter_types::*;
pub(crate) use dispatcher::*;

pub use adapter_kind::*;

/// Build the stable Gemini request configuration (`systemInstruction` + `tools`) in the exact
/// wire format used by the Gemini adapter. Callers can store this in Google's `cachedContents`
/// API and then reference it from generation requests.
pub fn gemini_cache_config(
	model_iden: &crate::ModelIden,
	chat_req: crate::chat::ChatRequest,
) -> crate::Result<serde_json::Value> {
	adapters::gemini::GeminiAdapter::build_cache_config(model_iden, chat_req)
}

// -- Crate modules
pub(crate) mod inter_stream;

// endregion: --- Modules
