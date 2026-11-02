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

use forge_mesh::LiveCatalog;

use crate::Session;

impl Session {
    /// Follow `feed`. Whatever it holds right now is the baseline, so only later publishes count
    /// as changes.
    pub fn set_catalog_feed(&mut self, feed: LiveCatalog) {
        self.catalog_feed_seen = feed.current();
        self.catalog_announced = self
            .catalog
            .as_ref()
            .map(|c| c.models().to_vec())
            .or_else(|| self.catalog_feed_seen.as_ref().map(|c| c.models().to_vec()))
            .unwrap_or_default();
        self.catalog_unannounced = false;
        self.catalog_feed = Some(feed);
    }

    /// Adopt the feed's catalog if a refresh published a new one since the last look. Adoption is
    /// immediate and silent: this never talks to the user, because it runs on a background timer
    /// and an idle session has nobody to tell. Returns whether a new snapshot was adopted. The
    /// user hears about it from [`Session::announce_catalog_changes`] at the next turn.
    pub fn sync_catalog_feed(&mut self) -> bool {
        let Some(current) = self.catalog_feed.as_ref().and_then(LiveCatalog::current) else {
            return false;
        };
        if self
            .catalog_feed_seen
            .as_ref()
            .is_some_and(|seen| Arc::ptr_eq(seen, &current))
        {
            return false;
        }
        self.set_catalog(Some((*current).clone()));
        self.catalog_feed_seen = Some(current);
        self.catalog_unannounced = true;
        true
    }

    /// At the start of a turn, say once what changed in the model set since the user was last
    /// told. A net-zero change (a model that flapped out and back) is silent. Returns the note.
    pub fn announce_catalog_changes(&mut self) -> Option<String> {
        if !std::mem::take(&mut self.catalog_unannounced) {
            return None;
        }
        let now = self.catalog.as_ref().map(|c| c.models().to_vec())?;
        let note = delta_note(&self.catalog_announced, &now);
        self.catalog_announced = now;
        if let Some(text) = &note {
            self.presenter
                .emit(forge_types::PresenterEvent::Progress(text.clone()));
        }
        note
    }
}

/// "models updated: +50 modelgateway, -2 groq" — per-provider counts of models that appeared or
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
    Some(format!("models updated: {}", parts.join(", ")))
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
            Some("models updated: -1 groq, +3 modelgateway, -1 ollama")
        );
    }
}
