//! Auditable, ordered context injected ahead of a Forge model call.
//!
//! The transcript remains the provider-facing source of truth. A [`ContextPack`] records why
//! system context entered it so local, TUI, and Anywhere surfaces can explain the effective
//! prompt without reconstructing injection policy themselves.

/// The system that contributed context to a model turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ContextSource {
    ProjectInstructions,
    CommandGuidance,
    TurnContract,
    Memory,
    Orchestration,
    Workflow,
    Attribution,
    Lattice,
}

/// One persisted system-context contribution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextEntry {
    source: ContextSource,
    reason: String,
    token_estimate: usize,
}

impl ContextEntry {
    fn new(source: ContextSource, reason: impl Into<String>, content: &str) -> Self {
        Self {
            source,
            reason: reason.into(),
            token_estimate: content.chars().count().saturating_add(3) / 4,
        }
    }

    /// The context system that produced this entry.
    pub fn source(&self) -> ContextSource {
        self.source
    }

    /// Human-readable reason the entry was included.
    pub fn reason(&self) -> &str {
        &self.reason
    }

    /// Conservative prompt-token estimate, used for audit rather than billing.
    pub fn token_estimate(&self) -> usize {
        self.token_estimate
    }
}

/// Ordered context injected during the most recent Forge turn.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContextPack {
    entries: Vec<ContextEntry>,
}

impl ContextPack {
    /// Record a persisted context injection in provider-visible order.
    pub fn push(&mut self, source: ContextSource, reason: impl Into<String>, content: &str) {
        self.entries
            .push(ContextEntry::new(source, reason, content));
    }

    /// Ordered, immutable audit entries.
    pub fn entries(&self) -> &[ContextEntry] {
        &self.entries
    }

    /// Total estimated prompt tokens contributed by this pack.
    pub fn total_token_estimate(&self) -> usize {
        self.entries.iter().map(ContextEntry::token_estimate).sum()
    }

    /// Whether this turn added no contextual system messages.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_provider_order_and_accounts_for_every_entry() {
        let mut pack = ContextPack::default();
        pack.push(
            ContextSource::ProjectInstructions,
            "repository rules",
            "abcd",
        );
        pack.push(ContextSource::Lattice, "relevant code", "abcdef");
        assert_eq!(
            pack.entries()
                .iter()
                .map(ContextEntry::source)
                .collect::<Vec<_>>(),
            vec![ContextSource::ProjectInstructions, ContextSource::Lattice]
        );
        assert_eq!(pack.total_token_estimate(), 3);
    }

    #[test]
    fn empty_pack_has_no_estimated_cost() {
        let pack = ContextPack::default();
        assert!(pack.is_empty());
        assert_eq!(pack.total_token_estimate(), 0);
    }
}
