//! Following a catalog that a background refresher keeps current.
//!
//! The session's catalog is a snapshot taken at startup. A long-lived session (a TUI left open
//! overnight, a daemon-hosted session) would otherwise never see a provider added to config, a key
//! stored with `forge auth`, or a model a provider started listing, until restart. The composition
//! root hands the session the same [`LiveCatalog`] slot the routers already follow; adopting from it
//! only ever replaces the catalog snapshot (picker rows, pin validation, rung lookup). The pinned
//! model, tier pin and any turn in flight are untouched: callers reach this through the session
//! lock, so it lands between turns.

use std::collections::BTreeMap;
use std::sync::Arc;

use forge_mesh::{LiveCatalog, ModelCatalog};

use crate::Session;

impl Session {
    /// Follow `feed`. Whatever it holds right now is the baseline, so only later publishes count
    /// as changes.
    pub fn set_catalog_feed(&mut self, feed: LiveCatalog) {
        self.catalog_feed_seen = feed.current();
        self.catalog_feed = Some(feed);
    }

    /// Adopt the feed's catalog if a refresh published a new one since the last look. Emits a
    /// one-line note when the set of models changed and stays silent otherwise (a refresh that
    /// only re-ranked is adopted quietly). Returns the note text, if one was emitted.
    pub fn sync_catalog_feed(&mut self) -> Option<String> {
        let current = self.catalog_feed.as_ref()?.current()?;
        if self
            .catalog_feed_seen
            .as_ref()
            .is_some_and(|seen| Arc::ptr_eq(seen, &current))
        {
            return None;
        }
        let note = delta_note(
            self.catalog.as_ref().map_or(&[], ModelCatalog::models),
            current.models(),
        );
        self.set_catalog(Some((*current).clone()));
        self.catalog_feed_seen = Some(current);
        if let Some(text) = &note {
            self.presenter
                .emit(forge_types::PresenterEvent::Progress(text.clone()));
        }
        note
    }
}

/// "catalog refreshed: +50 modelgateway, -2 groq" — per-provider counts of models that appeared or
/// vanished. `None` when the id sets are identical.
pub fn delta_note(before: &[String], after: &[String]) -> Option<String> {
    let before_set: std::collections::HashSet<&str> = before.iter().map(String::as_str).collect();
    let after_set: std::collections::HashSet<&str> = after.iter().map(String::as_str).collect();
    let mut per_provider: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
    for id in after_set.difference(&before_set) {
        per_provider.entry(provider_of(id)).or_default().0 += 1;
    }
    for id in before_set.difference(&after_set) {
        per_provider.entry(provider_of(id)).or_default().1 += 1;
    }
    if per_provider.is_empty() {
        return None;
    }
    let parts: Vec<String> = per_provider
        .iter()
        .flat_map(|(provider, (added, removed))| {
            let provider = if provider.is_empty() {
                "other"
            } else {
                provider
            };
            let mut out = Vec::new();
            if *added > 0 {
                out.push(format!("+{added} {provider}"));
            }
            if *removed > 0 {
                out.push(format!("-{removed} {provider}"));
            }
            out
        })
        .collect();
    Some(format!("catalog refreshed: {}", parts.join(", ")))
}

fn provider_of(id: &str) -> &str {
    forge_config::provider_of(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn identical_model_sets_are_silent() {
        let a = ids(&["groq::a", "groq::b"]);
        let b = ids(&["groq::b", "groq::a"]);
        assert_eq!(delta_note(&a, &b), None);
    }

    #[test]
    fn additions_and_removals_are_counted_per_provider() {
        let before = ids(&["groq::a", "groq::b", "ollama::x"]);
        let after = ids(&[
            "groq::a",
            "modelgateway::m1",
            "modelgateway::m2",
            "modelgateway::m3",
        ]);
        assert_eq!(
            delta_note(&before, &after).as_deref(),
            Some("catalog refreshed: -1 groq, +3 modelgateway, -1 ollama")
        );
    }
}
