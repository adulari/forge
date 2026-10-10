//! Frame cost while a long reply streams into a session that already has a big transcript. The
//! `#[ignore]`d benchmark prints p50/p95 per phase; the regular tests pin the structural reasons
//! the cost stays flat (bounded re-parsing, one frame per batch of deltas).

use super::*;
use ratatui::backend::TestBackend;
use ratatui::Terminal;
use std::time::Instant;

const TOKEN_CHARS: usize = 4;

fn long_markdown(tokens: usize) -> String {
    let target = tokens * TOKEN_CHARS;
    let mut out = String::new();
    let mut n = 1;
    while out.len() < target {
        out.push_str(&format!("## Section {n}: tuning the streaming path\n\n"));
        out.push_str(&format!(
            "The renderer keeps **{n} cached block** per `Frame` and only re-lays out the _open_ \
             block as tokens arrive, so a long answer should cost the same per token at the end \
             as it did at the start. Measuring is the only way to know.\n\n"
        ));
        out.push_str("- parse the delta once, never the whole reply\n- highlight a finished code block once\n- coalesce deltas so a burst paints one frame\n\n");
        out.push_str("| stage | cost | cached |\n|---|---|---|\n");
        out.push_str(&format!(
            "| parse | {n} us | yes |\n| wrap | {} us | yes |\n\n",
            n * 3
        ));
        out.push_str("```rust\n");
        out.push_str(&format!(
            "fn stage_{n}(input: &[u8]) -> Result<usize, String> {{\n"
        ));
        out.push_str(
            "    let mut total = 0usize;\n    for (index, byte) in input.iter().enumerate() {\n",
        );
        out.push_str(
            "        if *byte == b'\\n' {\n            total += index;\n        }\n    }\n",
        );
        out.push_str("    if total == 0 {\n        return Err(String::from(\"empty\"));\n    }\n    Ok(total)\n}\n```\n\n");
        n += 1;
    }
    out
}

/// A fullscreen app that already holds `lines` finished transcript lines.
fn app_with_history(lines: usize) -> App {
    let mut app = App {
        fullscreen: true,
        transcript_follow: true,
        ..Default::default()
    };
    let body: Vec<TextLine<'static>> = (0..lines)
        .map(|i| {
            TextLine::from(format!(
                "  history line {i}: the quick brown fox jumps over the lazy dog and keeps going \
                 so that wrapping has real work to do on a hundred and sixty column terminal"
            ))
        })
        .collect();
    app.push_scrollback(body);
    app
}

fn stream_chunks(text: &str, chunk_chars: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut start = 0;
    while start < text.len() {
        let mut end = (start + chunk_chars).min(text.len());
        while !text.is_char_boundary(end) {
            end += 1;
        }
        out.push(text[start..end].to_string());
        start = end;
    }
    out
}

fn percentile(sorted: &[u128], p: usize) -> u128 {
    if sorted.is_empty() {
        return 0;
    }
    sorted[((sorted.len() * p).div_ceil(100)).clamp(1, sorted.len()) - 1]
}

fn draw(term: &mut Terminal<TestBackend>, app: &App) -> u128 {
    let t = Instant::now();
    term.draw(|f| render_live(f, app)).unwrap();
    t.elapsed().as_micros()
}

/// Run with: `cargo test --release -p forge-agent-tui stream_frame_cost -- --ignored --nocapture`
#[test]
#[ignore = "benchmark: prints frame-time percentiles"]
fn stream_frame_cost() {
    let tokens: usize = std::env::var("BENCH_TOKENS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(20_000);
    let chunk_tokens: usize = std::env::var("BENCH_CHUNK_TOKENS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1);
    let mut app = app_with_history(5_000);
    app.busy = true;
    let mut term = Terminal::new(TestBackend::new(200, 50)).unwrap();
    // The wrap of the finished transcript is paid once, before the reply starts.
    draw(&mut term, &app);

    let text = long_markdown(tokens);
    let chunks = stream_chunks(&text, chunk_tokens * TOKEN_CHARS);
    // One frame per ~16 ms of a 250 tokens/s stream = 4 tokens per frame.
    let per_frame = (4 / chunk_tokens).max(1);
    let mut frame_us: Vec<u128> = Vec::new();
    let mut echo_us: Vec<u128> = Vec::new();
    let mut apply_us: Vec<u128> = Vec::new();
    let checkpoints = [tokens / 4, tokens / 2, tokens * 3 / 4, tokens];
    let mut fed_tokens = 0usize;
    let mut window: Vec<u128> = Vec::new();
    for (i, chunk) in chunks.iter().enumerate() {
        let t = Instant::now();
        app.apply(PresenterEvent::AssistantDelta(chunk.clone()));
        apply_us.push(t.elapsed().as_micros());
        fed_tokens += chunk_tokens;
        if (i + 1) % per_frame == 0 {
            let us = draw(&mut term, &app);
            frame_us.push(us);
            window.push(us);
            // A keystroke lands on this frame: type one char, draw again.
            if (i + 1) % (per_frame * 8) == 0 {
                app.input.push('x');
                app.input_cursor = app.input.len();
                echo_us.push(draw(&mut term, &app));
                if app.input.len() > 40 {
                    app.input.clear();
                    app.input_cursor = 0;
                }
            }
        }
        if checkpoints
            .iter()
            .any(|c| fed_tokens >= *c && fed_tokens < *c + chunk_tokens)
        {
            window.sort_unstable();
            eprintln!(
                "  @{fed_tokens:>6} tokens: frame p50 {:>7} us  p95 {:>7} us  max {:>7} us",
                percentile(&window, 50),
                percentile(&window, 95),
                window.last().copied().unwrap_or(0),
            );
            window.clear();
        }
    }
    let done = Instant::now();
    app.apply(PresenterEvent::AssistantDone);
    let flushed = app.drain_flush();
    let done_us = done.elapsed().as_micros();
    let after = draw(&mut term, &app);
    frame_us.sort_unstable();
    echo_us.sort_unstable();
    apply_us.sort_unstable();
    eprintln!(
        "frames n={} p50 {} us p95 {} us p99 {} us max {} us",
        frame_us.len(),
        percentile(&frame_us, 50),
        percentile(&frame_us, 95),
        percentile(&frame_us, 99),
        frame_us.last().copied().unwrap_or(0)
    );
    eprintln!(
        "echo   n={} p50 {} us p95 {} us max {} us",
        echo_us.len(),
        percentile(&echo_us, 50),
        percentile(&echo_us, 95),
        echo_us.last().copied().unwrap_or(0)
    );
    eprintln!(
        "apply  n={} p50 {} us p95 {} us max {} us",
        apply_us.len(),
        percentile(&apply_us, 50),
        percentile(&apply_us, 95),
        apply_us.last().copied().unwrap_or(0)
    );
    eprintln!(
        "finish (markdown of full reply + fold {} lines) {} us, next frame {} us",
        flushed.len(),
        done_us,
        after
    );
}
