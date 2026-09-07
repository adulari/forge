//! Arg parsing for `/subagents [free|pinned]` — whether this session's children may route off its
//! model pin (docs/features/mesh-routing.md §9.1). Its own submodule so the `parse_command` arm in
//! `commands.rs` stays a one-liner: that file sits at its CI file-size ratchet ceiling
//! (`scripts/ci/architecture_size.py`), so new logic has to live in new files rather than grow it.

/// `Some(true)` = free the children, `Some(false)` = bind them to the pin, `None` = toggle.
///
/// An unrecognised argument toggles rather than guessing a direction: a typo must never silently
/// mean "free", which would send a pinned session's fan-out onto other models unasked.
pub(crate) fn parse_subagents_arg(arg: &str) -> Option<bool> {
    match arg.trim().to_lowercase().as_str() {
        "free" | "any" | "unpinned" | "off" | "0" => Some(true),
        "pinned" | "pin" | "inherit" | "on" | "1" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn free_and_pinned_are_explicit_and_everything_else_toggles() {
        assert_eq!(parse_subagents_arg("free"), Some(true));
        assert_eq!(parse_subagents_arg(" PINNED "), Some(false));
        assert_eq!(parse_subagents_arg(""), None);
        assert_eq!(parse_subagents_arg("freee"), None);
    }
}
