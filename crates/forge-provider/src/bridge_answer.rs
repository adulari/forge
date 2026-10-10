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

#[cfg(test)]
mod tests {
    use super::bridge_answer;

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
}
