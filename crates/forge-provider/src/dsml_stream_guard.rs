//! Keeps DeepSeek DSML tool-call markup out of the LIVE text stream.
//!
//! `tool_recovery` strips a leaked `<｜｜DSML｜｜tool_calls>…` block from the stored reply, but by
//! then the streamed deltas have already reached the TUI / attach / app. The guard sits between
//! the provider's chunks and `StreamEvent::Text`: text that could still turn into a DSML opener is
//! held back, released as soon as it provably is not one, and everything after a confirmed opener
//! is swallowed (the recovery pass turns that block into tool calls).

use crate::tool_recovery::is_dsml_bar;

const MARKER: &str = "DSML";

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
    fn dsml_block_never_reaches_the_live_view_at_any_split() {
        let text = format!("Looking.\n{BLOCK}");
        for chunks in every_split(&text) {
            let refs: Vec<&str> = chunks.iter().map(String::as_str).collect();
            assert_eq!(stream(&refs).trim_end(), "Looking.", "chunks: {chunks:?}");
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
