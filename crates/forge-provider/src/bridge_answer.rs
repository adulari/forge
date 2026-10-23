/// The answer a bridge turn hands back. `streamed` is every text block the CLI emitted across its
/// internal tool loop, so mid-turn narration ("Now I'll fix X") sits glued, with no separator, in
/// front of the closing message. The CLI's `result` event carries only that closing message; when it
/// is a clean tail of the stream it is the answer, and the narration has already been shown live.
pub(crate) fn bridge_answer(streamed: String, result: Option<String>) -> String {
    if streamed.is_empty() {
        return result.unwrap_or_default();
    }
    match result {
        Some(result)
            if !result.trim().is_empty() && streamed.trim_end().ends_with(result.trim()) =>
        {
            result
        }
        _ => streamed,
    }
}

/// What separates two of codex's messages in the streamed text. `item.completed` carries each
/// message whole, so unlike claude's deltas there is a boundary to keep.
const MESSAGE_BREAK: &str = "\n\n";

/// Start the next codex message on its own paragraph. Without it, the narration in front of a tool
/// call ("I'll run it.") and the closing message ("done.") read as one sentence.
pub(crate) fn break_messages(streamed: &mut String) {
    if !streamed.is_empty() && !streamed.ends_with('\n') {
        streamed.push_str(MESSAGE_BREAK);
    }
}

/// The closing message of a codex turn. Codex's `exec --json` has no `result` event, so what claude
/// hands back as `result` is rebuilt here: the messages after the last tool call, the ones that
/// answer rather than announce. Without it the whole stream is the answer and its last sentence
/// is whatever the narration before the tool said, which the continue-nudge then takes for an
/// abandoned intention.
#[derive(Default)]
pub(crate) struct ClosingMessage {
    text: String,
}

impl ClosingMessage {
    pub(crate) fn message(&mut self, message: &str) {
        break_messages(&mut self.text);
        self.text.push_str(message);
    }

    /// Everything said so far came before a tool call: narration, not the answer.
    pub(crate) fn tool_started(&mut self) {
        self.text.clear();
    }

    pub(crate) fn into_text(self) -> Option<String> {
        (!self.text.trim().is_empty()).then_some(self.text)
    }
}

#[cfg(test)]
mod tests {
    use super::{break_messages, bridge_answer, ClosingMessage};

    #[test]
    fn narration_before_a_tool_call_is_not_part_of_the_answer() {
        let streamed = "Now I'll fix the bug.Root cause: the buffer was never cleared.".to_string();
        assert_eq!(
            bridge_answer(
                streamed,
                Some("Root cause: the buffer was never cleared.".into())
            ),
            "Root cause: the buffer was never cleared."
        );
    }

    #[test]
    fn a_single_segment_turn_is_unchanged() {
        assert_eq!(
            bridge_answer("All done.".into(), Some("All done.".into())),
            "All done."
        );
    }

    #[test]
    fn a_result_that_is_not_a_tail_of_the_stream_does_not_replace_it() {
        assert_eq!(
            bridge_answer("streamed text".into(), Some("something else".into())),
            "streamed text"
        );
    }

    #[test]
    fn an_empty_result_keeps_the_stream_and_an_empty_stream_falls_back_to_the_result() {
        assert_eq!(
            bridge_answer("streamed".into(), Some("  ".into())),
            "streamed"
        );
        assert_eq!(bridge_answer(String::new(), Some("final".into())), "final");
        assert_eq!(bridge_answer(String::new(), None), "");
    }

    #[test]
    fn codex_narration_before_a_tool_is_not_the_answer() {
        let mut streamed = String::new();
        let mut closing = ClosingMessage::default();
        let message = "I'll run the requested shell command exactly as written.";
        break_messages(&mut streamed);
        closing.message(message);
        streamed.push_str(message);
        closing.tool_started();
        break_messages(&mut streamed);
        closing.message("done.");
        streamed.push_str("done.");
        assert_eq!(
            streamed,
            "I'll run the requested shell command exactly as written.\n\ndone."
        );
        assert_eq!(bridge_answer(streamed, closing.into_text()), "done.");
    }

    #[test]
    fn codex_messages_with_no_tool_between_them_stay_one_answer_in_order() {
        let mut streamed = String::new();
        let mut closing = ClosingMessage::default();
        for message in ["First.", "Second."] {
            break_messages(&mut streamed);
            closing.message(message);
            streamed.push_str(message);
        }
        assert_eq!(
            bridge_answer(streamed, closing.into_text()),
            "First.\n\nSecond."
        );
    }

    #[test]
    fn a_codex_turn_that_ends_on_a_tool_has_no_closing_message() {
        let mut closing = ClosingMessage::default();
        closing.message("Now running the build.");
        closing.tool_started();
        assert_eq!(closing.into_text(), None);
        assert_eq!(
            bridge_answer("Now running the build.".into(), None),
            "Now running the build."
        );
    }
}
