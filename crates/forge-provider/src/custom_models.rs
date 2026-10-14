//! One entry of a custom OpenAI-compatible provider's `/models` listing, reduced to what discovery
//! uses. Beyond the id, gateways publish metadata worth keeping: a human label (Kimi Code's
//! `display_name`), the effort names a model accepts (InferHub's `reasoning_levels`) and what it
//! outputs (InferHub's `output_modality`).

/// One model a custom OpenAI-compatible provider listed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomModel {
    /// Full Forge id (`namespace::model`).
    pub id: String,
    /// The provider's human label, when it publishes one.
    pub display: Option<String>,
    /// Effort names the provider says this model accepts (`["low","high"]`), verbatim. Empty when
    /// the provider publishes none.
    pub reasoning_levels: Vec<String>,
}

impl CustomModel {
    /// Read one listing entry. `None` for an entry without an id, a clearly non-chat id
    /// (embedding / reranking), or a model whose published `output_modality` has no text in it
    /// (InferHub's image models): none of those can serve a chat turn.
    pub(crate) fn from_listing(namespace: &str, m: &serde_json::Value) -> Option<Self> {
        let id = format!("{namespace}::{}", m.get("id").and_then(|i| i.as_str())?);
        if forge_config::is_non_chat_model(&id) {
            return None;
        }
        let text_out = m
            .get("output_modality")
            .and_then(|o| o.as_str())
            .is_none_or(|o| o.split(',').any(|part| part.trim() == "text"));
        if !text_out {
            return None;
        }
        // Kimi Code's `display_name` ("K3" / "K2.8 Preview"), so opaque plan-slot ids can be shown
        // as the model they are.
        let display = m
            .get("display_name")
            .and_then(|d| d.as_str())
            .map(str::trim)
            .filter(|d| !d.is_empty())
            .map(str::to_string);
        let reasoning_levels = m
            .get("reasoning_levels")
            .and_then(|r| r.as_array())
            .map(|r| {
                r.iter()
                    .filter_map(|l| l.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        Some(Self {
            id,
            display,
            reasoning_levels,
        })
    }
}
