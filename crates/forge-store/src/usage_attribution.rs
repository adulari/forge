//! Per-session override of the provider that usage is recorded under.
//!
//! `forge --mock` answers every turn from the offline mock provider, but the router still picks a
//! real seed model (`ollama::…`), so the usage rows, their messages and the per-provider spend and
//! quota views all named a provider that was never called. A session marked here has the provider
//! namespace of every model id written for it replaced, so those rows read `mock::…`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use super::Store;

#[derive(Debug, Clone, Default)]
pub(crate) struct UsageAttribution(Arc<Mutex<HashMap<String, String>>>);

/// `provider::tail` with the namespace swapped, keeping the model name so rows stay distinguishable.
pub(crate) fn rewrite_provider(model: &str, provider: &str) -> String {
    let tail = model.split_once("::").map_or(model, |(_, tail)| tail);
    format!("{provider}::{tail}")
}

impl Store {
    /// Record usage for `session_id` under `provider` regardless of the model id it was routed to.
    pub fn attribute_session_usage_to(&self, session_id: &str, provider: &str) {
        if let Ok(mut map) = self.usage_attribution.0.lock() {
            map.insert(session_id.to_string(), provider.to_string());
        }
    }

    /// The model id to persist for `session_id`: the override when one is set, else `model`.
    pub(crate) fn attributed_model(&self, session_id: &str, model: Option<&str>) -> Option<String> {
        let model = model.map(str::trim).filter(|m| !m.is_empty())?;
        let map = self.usage_attribution.0.lock().ok()?;
        map.get(session_id)
            .map(|provider| rewrite_provider(model, provider))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_types::{Role, Usage};

    #[test]
    fn namespace_is_swapped_and_the_model_name_kept() {
        assert_eq!(rewrite_provider("ollama::qwen3", "mock"), "mock::qwen3");
        assert_eq!(rewrite_provider("bare", "mock"), "mock::bare");
    }

    #[test]
    fn a_marked_session_records_every_row_under_its_provider() {
        let store = Store::open_in_memory().unwrap();
        let marked = store.create_session("/repo", "default").unwrap();
        let real = store.create_session("/repo", "default").unwrap();
        store.attribute_session_usage_to(&marked, "mock");
        let usage = Usage {
            input_tokens: 10,
            output_tokens: 5,
            ..Default::default()
        };
        for sid in [&marked, &real] {
            let msg = store
                .add_message(sid, 0, Role::Assistant, "hi", Some("ollama::qwen3"))
                .unwrap();
            store
                .record_usage(sid, &msg, &usage, Some("ollama::qwen3"))
                .unwrap();
            store
                .record_side_call_usage_for(sid, "recap", Some("groq::llama"), &usage)
                .unwrap();
        }

        let marked_rows = store.usage_by_provider_for_session(&marked).unwrap();
        let providers: Vec<_> = marked_rows.iter().map(|r| r.provider.as_str()).collect();
        assert_eq!(providers, vec!["mock"], "{marked_rows:?}");
        let by_model = store.session_usage_by_model(&marked).unwrap();
        assert!(by_model
            .iter()
            .all(|m| m.model.as_deref().is_some_and(|m| m.starts_with("mock::"))));

        let mut real_providers: Vec<_> = store
            .usage_by_provider_for_session(&real)
            .unwrap()
            .into_iter()
            .map(|r| r.provider)
            .collect();
        real_providers.sort();
        assert_eq!(real_providers, vec!["groq", "ollama"], "unmarked untouched");
    }
}
