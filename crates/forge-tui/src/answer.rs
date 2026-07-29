//! Typed answer resolution for interactive questions.

use forge_types::QChoice;

/// Resolve a typed answer line against the options: a number `1..=N` picks that option's label;
/// otherwise, if `allow_other`, the trimmed line is a free-text answer. `None` = invalid input
/// (not a valid number and free text not allowed) → the caller should re-prompt.
pub fn resolve_answer(line: &str, options: &[QChoice], allow_other: bool) -> Option<String> {
    let t = line.trim();
    if let Ok(n) = t.parse::<usize>() {
        if n >= 1 && n <= options.len() {
            return Some(options[n - 1].label.clone());
        }
    }
    if allow_other && !t.is_empty() {
        return Some(t.to_string());
    }
    None
}
