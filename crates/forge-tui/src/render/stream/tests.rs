use super::*;
use crate::render::markdown_to_lines;

fn corpus() -> Vec<String> {
    let mut docs: Vec<String> = [
        "# Title\n\nFirst paragraph with **bold** and `code`.\n\nSecond paragraph.\n\n## Sub\n\ntext\n",
        "- one\n- two\n- three\n\nafter the list\n",
        "- loose one\n\n- loose two\n\n- loose three\n\nafter\n",
        "3. third\n4. fourth\n\n5. fifth\n\ntail\n",
        "- outer\n  - inner a\n  - inner b\n- outer two\n\n  continued paragraph in item\n\nend\n",
        "> quote line\n> more quote\n\n> second quote\n\nplain\n",
        "> lazy\ncontinuation\n\nplain\n",
        "```rust\nfn main() {\n\n    println!(\"hi\");\n}\n```\n\nafter code\n",
        "intro\n```\ncode directly after a paragraph\n\nwith a blank inside\n```\ntrailing text\n",
        "~~~python\nprint(1)\n\nprint(2)\n~~~\n\nok\n",
        "- item with code\n  ```sh\n  ls\n\n  pwd\n  ```\n- next item\n\nend\n",
        "| a | b |\n|---|---|\n| 1 | 2 |\n| 3 | 4 |\n\nafter the table\n",
        "text\n\n---\n\nmore text\n",
        "Setext\n======\n\nbody\n\nSub\n---\n\nbody\n",
        "para\n\n    indented code\n\n    more indented\n\nback\n",
        "<div>\nhtml block\n</div>\n\npara\n",
        "para ending without newline",
        "one\r\n\r\ntwo\r\n\r\n```\r\ncode\r\n```\r\n",
        "1. a\n2. b\n\n```\ncode after list\n\nstill code\n```\n\nend\n",
        "A\n\n\n\nB\n\n\nC\n",
        "~~struck~~ and *emphasis* and a [link](http://x.y)\n\nnext\n",
    ]
    .iter()
    .map(|s| (*s).to_string())
    .collect();
    let mut long = String::new();
    for n in 0..40 {
        long.push_str(&format!(
            "## S{n}\n\nPara {n} with `x` and **y**.\n\n- a{n}\n- b{n}\n\n```rust\nlet v = {n};\n\nlet w = v;\n```\n\n| k | v |\n|---|---|\n| {n} | {n} |\n\n"
        ));
    }
    docs.push(long);
    docs
}

fn chunks(doc: &str, step: usize) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    while start < doc.len() {
        let mut end = (start + step).min(doc.len());
        while !doc.is_char_boundary(end) {
            end += 1;
        }
        out.push(&doc[start..end]);
        start = end;
    }
    out
}

fn incremental_view(md: &mut StreamMarkdown, text: &str) -> Vec<Line<'static>> {
    md.advance(text);
    let mut lines = md.committed_lines().to_vec();
    lines.extend(md.tail_lines(text));
    while matches!(lines.last(), Some(l) if l.spans.is_empty()) {
        lines.pop();
    }
    lines
}

#[test]
fn every_prefix_renders_like_the_whole_document_renderer() {
    for (i, doc) in corpus().iter().enumerate() {
        // Byte-at-a-time for the short documents; the long one only at coarse steps (every step
        // costs a full-document render).
        let steps: &[usize] = if doc.len() < 400 { &[1, 7, 61] } else { &[173] };
        for &step in steps {
            let mut md = StreamMarkdown::default();
            let mut text = String::new();
            for piece in chunks(doc, step) {
                text.push_str(piece);
                // A partial last line is an open tail either way; compare at every step.
                assert_eq!(
                    incremental_view(&mut md, &text),
                    markdown_to_lines(&text),
                    "doc #{i} step {step} prefix {:?}",
                    text
                );
            }
            if doc.len() >= 400 {
                assert!(
                    md.committed_bytes() > doc.len() / 2,
                    "blocks were committed along the way"
                );
            }
        }
    }
}

#[test]
fn finish_equals_a_single_full_parse() {
    for (i, doc) in corpus().iter().enumerate() {
        let mut md = StreamMarkdown::default();
        let mut text = String::new();
        for piece in chunks(doc, 13) {
            text.push_str(piece);
            md.advance(&text);
        }
        assert_eq!(md.finish(&text), markdown_to_lines(doc), "doc #{i}");
    }
}

#[test]
fn a_fence_with_blank_lines_is_never_split() {
    let text = "```\nfirst\n\nsecond\n\nthird\n";
    assert_eq!(safe_boundary(text, 0), 0, "still inside the fence");
    let closed = format!("{text}```\n");
    assert_eq!(
        safe_boundary(&closed, 0),
        closed.len(),
        "boundary after the closing fence"
    );
}

#[test]
fn a_list_stays_open_across_blank_lines_until_something_else_starts() {
    assert_eq!(safe_boundary("- a\n\n- b\n\n", 0), 0);
    assert_eq!(safe_boundary("- a\n\n  more\n\n", 0), 0);
    let ended = "- a\n\nplain\n";
    assert_eq!(safe_boundary(ended, 0), "- a\n\n".len());
}

#[test]
fn only_complete_lines_count() {
    assert_eq!(safe_boundary("para\n\nnext blo", 0), "para\n\n".len());
    assert_eq!(
        safe_boundary("para\n", 0),
        0,
        "no blank line yet: the paragraph may grow"
    );
}

#[test]
fn boundaries_resume_from_the_committed_offset() {
    let text = "a\n\nb\n\nc\n";
    let first = safe_boundary(text, 0);
    assert_eq!(first, "a\n\nb\n\n".len());
    assert_eq!(safe_boundary(text, first), first, "nothing new to commit");
}

#[test]
fn streaming_a_long_reply_parses_it_about_once() {
    let mut doc = String::new();
    for n in 0..800 {
        doc.push_str(&format!(
            "## S{n}\n\nPara {n} with `x`.\n\n```rust\nlet v = {n};\n```\n\n"
        ));
    }
    PARSED_BYTES.with(|n| n.set(0));
    let mut cache = StreamCache::default();
    let mut text = String::new();
    for piece in chunks(&doc, 16) {
        text.push_str(piece);
        cache.refresh(&text, text.len() as u64, 120);
    }
    let parsed = PARSED_BYTES.with(std::cell::Cell::get);
    assert!(
        parsed < doc.len() * 3,
        "parsed {parsed} bytes for a {} byte reply: the whole reply is being re-parsed",
        doc.len()
    );
}

#[test]
fn cache_rows_match_wrapping_the_whole_render() {
    for doc in corpus() {
        let mut cache = StreamCache::default();
        let mut text = String::new();
        for piece in chunks(&doc, 29) {
            text.push_str(piece);
            cache.refresh(&text, text.len() as u64, 40);
        }
        // The last refresh may be rate-limited; force the final state like a quiet moment would.
        cache.refresh(&text, text.len() as u64 + 1, 40);
        let want = wrap_lines(&markdown_to_lines(&text), 39);
        assert_eq!(cache.window(0, usize::MAX), want);
        assert_eq!(cache.row_count(), want.len());
    }
}

#[test]
fn a_width_change_rewraps_finished_blocks() {
    let doc = "alpha beta gamma delta epsilon zeta eta theta iota kappa\n\nsecond block\n\n";
    let mut cache = StreamCache::default();
    cache.refresh(doc, 1, 80);
    let wide = cache.row_count();
    cache.refresh(doc, 1, 20);
    assert!(
        cache.row_count() > wide,
        "narrower width wraps into more rows"
    );
    assert_eq!(
        cache.window(0, usize::MAX),
        wrap_lines(&markdown_to_lines(doc), 19)
    );
}

#[test]
fn a_new_reply_starts_from_a_clean_cache() {
    let mut cache = StreamCache::default();
    cache.refresh("old reply\n\nmore\n\n", 1, 80);
    cache.refresh("new", 2, 80);
    assert_eq!(
        cache.window(0, usize::MAX),
        wrap_lines(&markdown_to_lines("new"), 79)
    );
}

#[test]
fn a_burst_is_caught_up_in_slices_and_ends_identical() {
    let mut doc = String::new();
    for n in 0..1500 {
        doc.push_str(&format!(
            "## S{n}\n\nPara {n} with `x`.\n\n```rust\nlet v = {n};\n```\n\n"
        ));
    }
    assert!(doc.len() > 5 * COMMIT_SLICE_BYTES);
    let mut cache = StreamCache::default();
    cache.refresh(&doc, 1, 80);
    assert!(
        cache.catching_up,
        "one refresh does not take the whole backlog"
    );
    assert!(
        cache.md.committed_bytes() < COMMIT_SLICE_BYTES * 2,
        "a refresh takes about one slice, took {}",
        cache.md.committed_bytes()
    );
    // The far end of the burst is already on screen, as plain text, while the slices catch up.
    assert!(cache.row_count() > 0);
    let mut refreshes = 1;
    while cache.catching_up && refreshes < 1000 {
        std::thread::sleep(CATCH_UP_GAP);
        cache.refresh(&doc, 1, 80);
        refreshes += 1;
    }
    assert!(!cache.catching_up, "converges");
    assert!(refreshes > 4, "took {refreshes} slices");
    cache.refresh(&doc, 2, 80);
    assert_eq!(
        cache.window(0, usize::MAX),
        wrap_lines(&markdown_to_lines(&doc), 79)
    );
}
