//! Per-run settings a `forge run` caller applies to a built session: Claude Code's
//! `--fallback-model` chain, and the tool list `stream-json` reports in `system/init`.

use super::*;

impl Session {
    /// Models to fail over to, in this order, when the routed model is unavailable — before the
    /// mesh's own chain (`--fallback-model`). Applied after the pin clamp: naming a fallback is the
    /// user widening their own pin, so a pinned run may fall to exactly these models and no others.
    pub fn set_fallback_models(&mut self, models: Vec<String>) {
        self.fallback_models = models;
    }

    pub(crate) fn apply_fallback_models(&self, decision: &mut forge_mesh::RoutingDecision) {
        if self.fallback_models.is_empty() {
            return;
        }
        decision.fallbacks = ordered_chain(
            &decision.model,
            &self.fallback_models,
            std::mem::take(&mut decision.fallbacks),
        );
    }

    /// Every tool the model can be offered this session, by name (before the lean surface defers
    /// any of them).
    pub fn tool_names(&self) -> Vec<String> {
        self.full_tool_specs()
            .into_iter()
            .map(|spec| spec.name)
            .collect()
    }
}

/// `preferred` in order (minus the routed model), then whatever the mesh chain adds.
fn ordered_chain(model: &str, preferred: &[String], mesh: Vec<String>) -> Vec<String> {
    let mut chain: Vec<String> = Vec::new();
    for m in preferred.iter().cloned().chain(mesh) {
        if m != model && !chain.contains(&m) {
            chain.push(m);
        }
    }
    chain
}

#[cfg(test)]
mod tests {
    use super::ordered_chain;

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn named_fallbacks_come_first_in_their_order_then_the_mesh_chain() {
        assert_eq!(
            ordered_chain("a", &ids(&["c", "b"]), ids(&["b", "d"])),
            ["c", "b", "d"]
        );
    }

    #[test]
    fn the_routed_model_is_never_its_own_fallback() {
        assert_eq!(ordered_chain("a", &ids(&["a", "b"]), Vec::new()), ["b"]);
    }
}
