//! `genai` library - A client library for any AI provider.
//! See [examples/c00-readme.rs](./examples/c00-readme.rs)

// region:    --- Modules

mod support;

mod client;
mod common;
mod error;

// -- Flatten
pub use client::*;
pub use common::*;
pub use error::{BoxError, Error, Result};

// -- Public Modules
pub mod adapter;
pub mod chat;
pub mod embed;
pub mod resolver;
pub mod webc;

/// OpenCode Go per-model wire-format routing, for callers that learn a model's endpoint from a
/// failure at runtime (see `adapter::adapters::opencode_go`).
pub mod opencode_go {
	pub use crate::adapter::opencode_go::{is_responses_model, mark_responses_model, unmark_responses_model};
}

// endregion: --- Modules

// region:    --- TLS Backend Guard

// TLS backends are mutually exclusive (forwarded to reqwest; see Cargo.toml / README).
// Enabling `native-tls` without `default-features = false` leaves `rustls-tls` on from
// the default set; turn that silent mis-selection into a clear compile-time error.
// The "neither feature" case is intentionally allowed — it is the supported
// bring-your-own-client path (`with_reqwest`).
#[cfg(all(feature = "rustls-tls", feature = "native-tls"))]
compile_error!(
	"genai: `rustls-tls` and `native-tls` are mutually exclusive. \
	 To use native-tls, set `default-features = false` and enable `native-tls`."
);

// endregion: --- TLS Backend Guard
