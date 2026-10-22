//! Start the pinned CLI bridge while the person types the first prompt.
//!
//! A first turn on a Claude bridge pays for the `claude` process, its `forge mcp-serve` child and
//! their handshake before the prompt can even be written (the better part of a second). A session
//! pinned to one `claude-cli::` model knows which process its first turn will want, so the warm-up
//! starts at session build and the cost lands while the composer is still empty.

use std::sync::Arc;

use forge_types::Role;

use crate::Session;

impl Session {
    /// Warm the bridge process the first turn will use. Only a NEW conversation pinned to exactly
    /// one `claude-cli::` model is warmed: an unpinned session does not know its model yet, and a
    /// conversation with history resumes claude's own session, whose delta is cheaper than the full
    /// transcript a fresh process takes. Fire and forget; a turn that wins the race spawns its own.
    pub fn prewarm_pinned_bridge(&self) {
        let [model] = self.pinned_model.as_deref().unwrap_or_default() else {
            return;
        };
        if !model.starts_with("claude-cli::")
            || self.exact_effort.is_some()
            || self.transcript.iter().any(|m| m.role == Role::User)
        {
            return;
        }
        let model = model.clone();
        let checkpoint = self.checkpoint_context();
        // The pin is what a pinned turn sends: its decision carries no rung of its own.
        let effort = self.pinned_effort;
        let provider = Arc::clone(&self.provider);
        tokio::spawn(async move {
            provider.prewarm(&model, &checkpoint, effort).await;
        });
    }
}
