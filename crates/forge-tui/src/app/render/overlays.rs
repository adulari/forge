use super::*;
use crate::theme::styles;

mod btw;
mod pad;
pub(in crate::app::render) use pad::padded_panel;
mod usage;
pub use btw::render_btw_overlay;
pub use usage::render_usage_overlay;
mod mesh;
pub use mesh::render_mesh_overlay;

/// Honest cost label for a per-model row. A flat-rate subscription bridge (Claude Code / Codex
/// CLI) genuinely costs $0 per call, so it reads "subscription". A priced model shows its dollar
/// cost. At $0 with no bridge, `forge_mesh::catalog::is_free` tells apart a model we KNOW charges
/// nothing (ollama, groq, a `:free` gateway variant, …) from one we simply have no price for
/// (unpriced OpenRouter/OpenCode Zen models, which may still burn real gateway credit) — the
/// former reads "free", the latter "untracked" rather than lying that it's costless.
pub(crate) fn cost_cell(model: &str, cost: f64) -> String {
    let subscription = forge_mesh::catalog::is_subscription(model);
    if subscription {
        "subscription".to_string()
    } else if cost > 0.0 {
        format!("${cost:.5}")
    } else if forge_mesh::catalog::is_free(model, cost, subscription) {
        "free".to_string()
    } else {
        "untracked".to_string()
    }
}

/// A compact `→ 93% at reset ⚠` suffix for a quota line when a pace projection exists
/// (mesh-routing.md) — `""` when there isn't enough history to project one yet.
pub(crate) fn mesh_pace_suffix(
    projected_fraction_at_reset: Option<f64>,
    exhaustion_warning: bool,
) -> String {
    match projected_fraction_at_reset {
        Some(p) => format!(
            " → {:.0}% at reset{}",
            p * 100.0,
            if exhaustion_warning { " ⚠" } else { "" }
        ),
        None => String::new(),
    }
}
