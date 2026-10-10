//! The mock provider's long-answer scenario, for measuring TUI responsiveness while a big reply
//! streams. `mock:long` paces the chunks like a fast model; `mock:burst` delivers the same text in
//! large unpaced chunks (a proxy flushing a buffered SSE stream). Both end the turn with the full
//! text as the final content, exactly like a real answer.

use std::time::Duration;

use crate::{EventSink, StreamEvent};

/// Characters per token for English prose and code; the mock has no tokenizer.
const CHARS_PER_TOKEN: usize = 4;
const DEFAULT_TOKENS: usize = 20_000;
const BURST_CHUNK_TOKENS: usize = 64;
const DEFAULT_TOKEN_DELAY_US: u64 = 12_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LongMode {
    Paced,
    Burst,
}

pub(crate) fn mode_for(lowercased_prompt: &str) -> Option<LongMode> {
    if lowercased_prompt.contains("mock:burst") {
        Some(LongMode::Burst)
    } else if lowercased_prompt.contains("mock:long") {
        Some(LongMode::Paced)
    } else {
        None
    }
}

fn env_usize(name: &str) -> Option<usize> {
    std::env::var(name).ok()?.parse().ok().filter(|n| *n > 0)
}

/// Deterministic markdown of roughly `tokens` tokens: headings, prose with inline markup, bullet
/// and numbered lists, a table and fenced code blocks, repeated section after section.
pub(crate) fn long_markdown(tokens: usize) -> String {
    let target = tokens * CHARS_PER_TOKEN;
    let mut out = String::with_capacity(target + 2048);
    let mut section = 1;
    while out.len() < target {
        out.push_str(&format!(
            "## Section {section}: tuning the streaming path\n\n"
        ));
        out.push_str(&format!(
            "The renderer keeps **{section} cached block** per `Frame` and only re-lays out the \
             _open_ block as tokens arrive, so a long answer should cost the same per token at \
             the end as it did at the start. Measuring is the only way to know whether that \
             actually holds on a real terminal.\n\n"
        ));
        out.push_str("- parse the delta once, never the whole reply\n");
        out.push_str("- highlight a finished code block once and reuse it\n");
        out.push_str("- coalesce deltas so a burst paints one frame\n\n");
        out.push_str(
            "1. read the transcript tail\n2. wrap only what changed\n3. flush one diff\n\n",
        );
        out.push_str("| stage | cost | cached |\n|---|---|---|\n");
        out.push_str(&format!(
            "| parse | {section} us | yes |\n| wrap | {} us | yes |\n\n",
            section * 3
        ));
        out.push_str("```rust\n");
        out.push_str(&format!(
            "fn stage_{section}(input: &[u8]) -> Result<usize, String> {{\n"
        ));
        out.push_str("    let mut total = 0usize;\n");
        out.push_str("    for (index, byte) in input.iter().enumerate() {\n");
        out.push_str("        if *byte == b'\\n' {\n");
        out.push_str("            total += index;\n");
        out.push_str("        }\n");
        out.push_str("    }\n");
        out.push_str("    if total == 0 {\n");
        out.push_str("        return Err(format!(\"empty input at stage {}\", 1));\n");
        out.push_str("    }\n");
        out.push_str("    Ok(total)\n");
        out.push_str("}\n```\n\n");
        section += 1;
    }
    out
}

/// Stream `text` to the sink in chunks and return it. Honors `FORGE_MOCK_LONG_TOKENS` (size of the
/// reply) and `FORGE_MOCK_TOKEN_DELAY_US` (pause per token in paced mode).
pub(crate) async fn stream_long(mode: LongMode, on_event: &mut EventSink<'_>) -> String {
    let tokens = env_usize("FORGE_MOCK_LONG_TOKENS").unwrap_or(DEFAULT_TOKENS);
    let text = long_markdown(tokens);
    let (chunk_chars, delay) = match mode {
        LongMode::Burst => (BURST_CHUNK_TOKENS * CHARS_PER_TOKEN, Duration::ZERO),
        LongMode::Paced => (
            CHARS_PER_TOKEN,
            Duration::from_micros(
                env_usize("FORGE_MOCK_TOKEN_DELAY_US")
                    .map_or(DEFAULT_TOKEN_DELAY_US, |us| us as u64),
            ),
        ),
    };
    let mut start = 0;
    while start < text.len() {
        let mut end = (start + chunk_chars).min(text.len());
        while !text.is_char_boundary(end) {
            end += 1;
        }
        on_event(StreamEvent::Text(text[start..end].to_string()));
        start = end;
        if delay.is_zero() {
            tokio::task::yield_now().await;
        } else {
            tokio::time::sleep(delay).await;
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_selects_the_mode() {
        assert_eq!(mode_for("run mock:long please"), Some(LongMode::Paced));
        assert_eq!(mode_for("mock:burst"), Some(LongMode::Burst));
        assert_eq!(mode_for("hello"), None);
    }

    #[test]
    fn markdown_is_sized_in_tokens_and_has_every_block_kind() {
        let md = long_markdown(2_000);
        assert!(md.len() >= 2_000 * CHARS_PER_TOKEN);
        assert!(md.len() < 2_000 * CHARS_PER_TOKEN + 2_048);
        for needle in ["## Section", "```rust", "| stage |", "1. read", "- parse"] {
            assert!(md.contains(needle), "missing {needle}");
        }
        assert_eq!(md.matches("```").count() % 2, 0, "fences are balanced");
    }

    #[tokio::test]
    async fn burst_streams_the_whole_text_in_large_chunks() {
        let mut chunks = Vec::new();
        let text = stream_long(LongMode::Burst, &mut |ev| {
            if let StreamEvent::Text(t) = ev {
                chunks.push(t)
            }
        })
        .await;
        assert_eq!(chunks.concat(), text);
        assert!(chunks.len() < text.len() / (BURST_CHUNK_TOKENS * CHARS_PER_TOKEN / 2));
    }
}
