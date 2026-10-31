//! Keeps DeepSeek DSML tool-call markup out of the LIVE text stream.
//!
//! `tool_recovery` strips a leaked `<｜｜DSML｜｜ calls>…` block from the stored reply, but by
//! then the streamed deltas have already reached the TUI / attach / app. The guard sits between
//! the provider's chunks and `StreamEvent::Text`: text that could still turn into a DSML opener is
//! held back, released as soon as it provably is not one, and everything after a confirmed opener
//! is swallowed (the recovery pass turns that block into tool calls).

use crate::dsml::{is_dsml_bar, MARKER};

#[derive(Default)]
pub(crate) struct DsmlStreamGuard {
    held: String,
    swallowing: bool,
}

enum Opener {
    /// `<[/]｜+DSML｜` — certainly template markup.
    Confirmed,
    /// Every char so far fits an opener but the text ends before it is decided.
    Partial,
    NotDsml,
}

/// Classify text that starts with `<`.
fn classify(s: &str) -> Opener {
    match chat_template_opener(s) {
        Opener::NotDsml => {}
        decided => return decided,
    }
    let body = s[1..].strip_prefix('/').unwrap_or(&s[1..]);
    let bars = body.trim_start_matches(is_dsml_bar);
    let had_bar = bars.len() < body.len();
    if bars.is_empty() {
        return Opener::Partial;
    }
    if !had_bar {
        // `<DSML` without a leading bar is not the template's spelling.
        return Opener::NotDsml;
    }
    match bars.strip_prefix(MARKER) {
        Some(after) => match after.chars().next() {
            None => Opener::Partial,
            Some(c) if is_dsml_bar(c) => Opener::Confirmed,
            Some(_) => Opener::NotDsml,
        },
        None if MARKER.starts_with(bars) => Opener::Partial,
        None => Opener::NotDsml,
    }
}

/// `<|tool_call…` / `<｜tool▁call…`: the Kimi and DeepSeek V3 chat-template tool-call tokens
/// (`chat_template_calls`), spelled with ASCII or fullwidth bars and `_` or `▁`.
fn chat_template_opener(s: &str) -> Opener {
    const WANT: &str = "tool_call";
    let Some(rest) = s[1..].strip_prefix(['|', '｜']) else {
        return Opener::NotDsml;
    };
    let seen: String = rest
        .chars()
        .take(WANT.len())
        .map(|c| if c == '▁' { '_' } else { c })
        .collect();
    if seen == WANT {
        Opener::Confirmed
    } else if seen.len() < WANT.len() && WANT.starts_with(&seen) {
        Opener::Partial
    } else {
        Opener::NotDsml
    }
}

impl DsmlStreamGuard {
    /// Feed one streamed chunk; returns the text that is safe to show now.
    pub(crate) fn push(&mut self, chunk: &str) -> String {
        if self.swallowing {
            return String::new();
        }
        let mut buf = std::mem::take(&mut self.held);
        buf.push_str(chunk);
        let mut out = String::with_capacity(buf.len());
        let mut rest = buf.as_str();
        while let Some(lt) = rest.find('<') {
            out.push_str(&rest[..lt]);
            rest = &rest[lt..];
            match classify(rest) {
                Opener::Confirmed => {
                    self.swallowing = true;
                    return out;
                }
                Opener::Partial => {
                    self.held = rest.to_string();
                    return out;
                }
                Opener::NotDsml => {
                    out.push('<');
                    rest = &rest[1..];
                }
            }
        }
        out.push_str(rest);
        out
    }

    /// End of stream: whatever was held never became a DSML opener, so release it.
    pub(crate) fn finish(&mut self) -> String {
        std::mem::take(&mut self.held)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stream(chunks: &[&str]) -> String {
        let mut guard = DsmlStreamGuard::default();
        let mut shown: String = chunks.iter().map(|c| guard.push(c)).collect();
        shown.push_str(&guard.finish());
        shown
    }

    fn every_split(text: &str) -> Vec<Vec<String>> {
        let idx: Vec<usize> = text.char_indices().map(|(i, _)| i).collect();
        let mut out = vec![text.chars().map(String::from).collect()];
        for &i in &idx[1..] {
            out.push(vec![text[..i].to_string(), text[i..].to_string()]);
        }
        out
    }

    const BLOCK: &str = "<｜｜DSML｜｜tool_calls><｜｜DSML｜｜invoke name=\"read_file\"><｜｜DSML｜｜parameter name=\"path\" string=\"true\">a.rs</｜｜DSML｜｜parameter></｜｜DSML｜｜invoke></｜｜DSML｜｜tool_calls>";

    #[test]
    fn kimi_and_deepseek_v3_tokens_never_reach_the_live_view_at_any_split() {
        let kimi = "Ok.<|tool_calls_section_begin|><|tool_call_begin|>functions.shell:0<|tool_call_argument_begin|>{}<|tool_call_end|><|tool_calls_section_end|>";
        let v3 = "Ok.<｜tool▁calls▁begin｜><｜tool▁call▁begin｜>function<｜tool▁sep｜>shell\n```json\n{}\n```<｜tool▁call▁end｜><｜tool▁calls▁end｜>";
        for text in [kimi, v3] {
            for chunks in every_split(text) {
                let refs: Vec<&str> = chunks.iter().map(String::as_str).collect();
                assert_eq!(stream(&refs), "Ok.", "{chunks:?}");
            }
        }
        assert_eq!(
            stream(&["a <| b and <|tool|> ok"]),
            "a <| b and <|tool|> ok"
        );
    }

    #[test]
    fn dsml_block_never_reaches_the_live_view_at_any_split() {
        let text = format!("Looking.\n{BLOCK}");
        for chunks in every_split(&text) {
            let refs: Vec<&str> = chunks.iter().map(String::as_str).collect();
            assert_eq!(stream(&refs).trim_end(), "Looking.", "chunks: {chunks:?}");
        }
    }

    #[test]
    fn spaced_v41_block_never_reaches_the_live_view_at_any_split() {
        let block = "<｜｜DSML｜｜ calls>\n<｜｜DSML｜｜ invoke name=\"read_file\">\n\
            <｜｜DSML｜｜ parameter name=\"path\" string=\"true\">a.rs</｜｜DSML｜｜ parameter>\n\
            </｜｜DSML｜｜ invoke>\n</｜｜DSML｜｜ calls>";
        let text = format!("I'll look.\n\n{block}");
        for chunks in every_split(&text) {
            let refs: Vec<&str> = chunks.iter().map(String::as_str).collect();
            assert_eq!(stream(&refs).trim_end(), "I'll look.", "chunks: {chunks:?}");
        }
    }

    #[test]
    fn opener_split_across_three_chunks_is_held_then_swallowed() {
        assert_eq!(stream(&["ok <", "｜｜DS", "ML｜｜tool_calls>x"]), "ok ");
    }

    #[test]
    fn lookalike_text_is_released_in_order() {
        for text in [
            "a < b and c > d",
            "<div>hi</div>",
            "x <｜ not it",
            "<DSML only>",
            "tail <",
            "tail <｜｜DS",
            "1 </ 2",
        ] {
            for chunks in every_split(text) {
                let refs: Vec<&str> = chunks.iter().map(String::as_str).collect();
                assert_eq!(stream(&refs), text, "chunks: {chunks:?}");
            }
        }
    }

    #[test]
    fn ascii_bars_are_recognized_too() {
        assert_eq!(stream(&["hi <|DS", "ML|tool_calls>"]), "hi ");
    }
}
