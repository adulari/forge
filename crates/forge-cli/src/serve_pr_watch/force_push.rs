//! Whether a permission prompt is a force push, which PR Watch refuses.

pub(super) fn is_force_push(prompt: &str) -> bool {
    let lower = prompt.to_lowercase();
    lower.contains("push")
        && (lower.contains("--force")
            || lower.contains("force-with-lease")
            || lower.contains("force-push")
            || lower.contains("force push")
            || lower
                .split_whitespace()
                .any(|w| w == "-f" || w.starts_with('+')))
}
