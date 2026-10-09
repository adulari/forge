//! Provider-native output items replayed verbatim with an assistant message, and the sibling
//! builder for the reply's private thinking.

use serde::{Deserialize, Serialize};

use crate::Message;

/// Opaque items a provider returned alongside an assistant reply, plus the exact model id
/// (`namespace::name`) that produced them. See [`Message::provider_items`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderItems {
    pub source: String,
    pub items: Vec<serde_json::Value>,
}

impl Message {
    /// Attach provider-native items (see [`Message::provider_items`]). Empty means none.
    #[must_use]
    pub fn with_provider_items(
        mut self,
        source: impl Into<String>,
        items: Vec<serde_json::Value>,
    ) -> Self {
        self.provider_items = (!items.is_empty()).then(|| ProviderItems {
            source: source.into(),
            items,
        });
        self
    }

    /// Attach the reply's own private thinking (see [`Message::reasoning`]). Empty means the
    /// provider returned none, which is stored as `None` rather than an empty string so the
    /// replayed message carries no reasoning part at all.
    #[must_use]
    pub fn with_reasoning(mut self, reasoning: impl Into<String>) -> Self {
        let reasoning = reasoning.into();
        self.reasoning = (!reasoning.is_empty()).then_some(reasoning);
        self
    }
}
