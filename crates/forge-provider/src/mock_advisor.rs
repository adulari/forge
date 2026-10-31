//! The offline mock's answer to the advisor review: silent unless the input carries
//! `ADVISOR_MOCK_BLOCKER`, so `--mock` runs and tests can exercise both outcomes.

use forge_types::{Message, Role};

/// Must match `forge_core::advisor::ADVISOR_MARKER`.
const MARKER: &str = "You are the advisor for an autonomous coding agent";

pub(crate) fn is_advisor_review(messages: &[Message]) -> bool {
    messages
        .iter()
        .any(|m| m.role == Role::System && m.content.starts_with(MARKER))
}

pub(crate) fn reply(messages: &[Message]) -> String {
    let flagged = messages
        .iter()
        .any(|m| m.role == Role::User && m.content.contains("ADVISOR_MOCK_BLOCKER"));
    if flagged {
        r#"{"verdict":"blocker","notes":[{"severity":"blocker","text":"The requested check was never run."}],"next_step":"Run the check and show its output."}"#
    } else {
        r#"{"verdict":"ok","notes":[],"next_step":""}"#
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn review(user: &str) -> Vec<Message> {
        vec![
            Message::system(format!("{MARKER}. rest")),
            Message::user(user),
        ]
    }

    #[test]
    fn recognises_a_review_and_stays_silent_by_default() {
        assert!(is_advisor_review(&review("x")));
        assert!(!is_advisor_review(&[Message::user("x")]));
        assert!(reply(&review("x")).contains(r#""verdict":"ok""#));
    }

    #[test]
    fn the_marker_in_the_input_raises_a_blocker() {
        assert!(reply(&review("ADVISOR_MOCK_BLOCKER")).contains(r#""verdict":"blocker""#));
    }
}
