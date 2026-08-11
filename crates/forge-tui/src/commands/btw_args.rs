//! Arg parsing for `/btw` (alias `/side`) — an inline side question that never enters the
//! session transcript (docs/features/side-questions.md). Split out of `commands.rs` into its own
//! submodule so the `parse_command` match arm there stays a one-liner: `commands.rs` sits at its
//! CI file-size ratchet ceiling (`scripts/ci/architecture_size.py`), so new logic has to live in
//! new files rather than grow that one.

/// Everything after `/btw`/`/side`, trimmed. May be empty — `Session::ask_btw` shows the usage
/// hint for a blank question rather than making a model call.
pub(crate) fn parse_btw_arg(arg: &str) -> String {
    arg.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trims_surrounding_whitespace() {
        assert_eq!(parse_btw_arg("  what is forge?  "), "what is forge?");
    }

    #[test]
    fn empty_arg_stays_empty() {
        assert_eq!(parse_btw_arg(""), "");
        assert_eq!(parse_btw_arg("   "), "");
    }
}
