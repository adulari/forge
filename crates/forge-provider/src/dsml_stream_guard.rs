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
    /// Text from a chat-template opener onward, held back (not discarded) until the stream ends.
    template: Option<String>,
    code: CodeState,
}

enum Opener {
    /// `<[/]｜+DSML｜` — certainly template markup.
    Confirmed,
    /// A complete Kimi / DeepSeek V3 block opener token; hidden only if the block parses.
    Template,
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

/// A complete chat-template block opener (`<|tool_call_begin|>`, `<|tool_calls_section_begin|>`,
/// `<|tool_calls_begin|>`), spelled with ASCII or fullwidth bars and `_` or `▁`. Only a whole
/// token counts: `<|tool_call` alone is not enough to start hiding text.
fn chat_template_opener(s: &str) -> Opener {
    const TOKENS: [&str; 3] = [
        "tool_call_begin",
        "tool_calls_section_begin",
        "tool_calls_begin",
    ];
    let Some(rest) = s[1..].strip_prefix(['|', '｜']) else {
        return Opener::NotDsml;
    };
    let mut name = String::new();
    let mut chars = rest.chars();
    while let Some(c) = chars.next() {
        if c == '|' || c == '｜' {
            return match chars.next() {
                _ if !TOKENS.contains(&name.as_str()) => Opener::NotDsml,
                Some('>') => Opener::Template,
                None => Opener::Partial,
                Some(_) => Opener::NotDsml,
            };
        }
        name.push(if c == '▁' { '_' } else { c });
        if !TOKENS.iter().any(|t| t.starts_with(&name)) {
            return Opener::NotDsml;
        }
    }
    Opener::Partial
}

/// Tracks whether the text shown so far ends inside a code fence or inline code span.
#[derive(Default)]
struct CodeState {
    fence: bool,
    inline_ticks: usize,
    run: usize,
}

impl CodeState {
    fn feed(&mut self, text: &str) {
        for c in text.chars() {
            if c == '`' {
                self.run += 1;
                if self.run == 3 {
                    self.fence = !self.fence;
                    self.run = 0;
                }
                continue;
            }
            if !self.fence {
                self.inline_ticks += self.run;
            }
            self.run = 0;
            if c == '\n' {
                self.inline_ticks = 0;
            }
        }
    }

    fn inside(&self) -> bool {
        self.fence || (self.inline_ticks + self.run) % 2 == 1
    }
}

impl DsmlStreamGuard {
    /// Feed one streamed chunk; returns the text that is safe to show now.
    pub(crate) fn push(&mut self, chunk: &str) -> String {
        if self.swallowing {
            return String::new();
        }
        if let Some(held) = self.template.as_mut() {
            held.push_str(chunk);
            return String::new();
        }
        let mut buf = std::mem::take(&mut self.held);
        buf.push_str(chunk);
        let mut out = String::with_capacity(buf.len());
        let mut rest = buf.as_str();
        while let Some(lt) = rest.find('<') {
            out.push_str(&rest[..lt]);
            self.code.feed(&rest[..lt]);
            rest = &rest[lt..];
            match classify(rest) {
                Opener::Template if self.code.inside() => {
                    out.push('<');
                    rest = &rest[1..];
                }
                Opener::Template => {
                    self.template = Some(rest.to_string());
                    return out;
                }
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
                    self.code.feed("<");
                    rest = &rest[1..];
                }
            }
        }
        out.push_str(rest);
        self.code.feed(rest);
        out
    }

    /// End of stream: whatever was held never became a DSML opener, so release it.
    pub(crate) fn finish(&mut self) -> String {
        if let Some(block) = self.template.take() {
            if crate::chat_template_calls::recover(&block).is_some() {
                return String::new();
            }
            return block;
        }
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

    // Regression (review finding 16): quoted tokens in prose or code must not eat the reply, and a
    // block that does not parse is released at the end of the stream.
    #[test]
    fn quoted_or_unparseable_tokens_never_cost_the_user_text() {
        let prose = "Kimi opens a call with <|tool_call_begin|> then a name, and that is all.";
        let fenced = "Example:\n```\n<|tool_call_begin|>functions.x:0<|tool_call_argument_begin|>{}<|tool_call_end|>\n```\nDone.";
        let inline = "Use `<|tool_calls_section_begin|>` to open it. Then more text.";
        for text in [prose, fenced, inline] {
            for chunks in every_split(text) {
                let refs: Vec<&str> = chunks.iter().map(String::as_str).collect();
                assert_eq!(stream(&refs), text, "{chunks:?}");
            }
        }
        // A bare `<|tool_call` prefix is not a token.
        assert_eq!(
            stream(&["a <|tool_call and then more"]),
            "a <|tool_call and then more"
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
