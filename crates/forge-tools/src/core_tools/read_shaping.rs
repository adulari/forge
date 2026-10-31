//! `[harness] read_shaping`: how `read_file` presents a file when the flag is on. Pure functions
//! over the file text so they are tested without the flag; `ReadFileTool` decides when to call
//! them. Borrowed from oh-my-pi's read tool: ranges inside the path string, a declarations-only
//! outline for a bare read of a long code file, a 300-line default page that says how to
//! continue, and a per-line byte cap.

use std::fmt::Write as _;

pub(super) const DEFAULT_PAGE_LINES: usize = 300;
pub(super) const OUTLINE_MIN_LINES: usize = 100;
pub(super) const LINE_CAP_BYTES: usize = 768;
const OUTLINE_MAX_CHARS: usize = 16_000;
const GAPS_LISTED: usize = 8;

/// What part of the file a read asks for, from `path:SPEC` or the `start_line`/`end_line` params.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Selector {
    Bare,
    /// Inclusive 1-indexed spans; `end` of `usize::MAX` means to the end of the file.
    Ranges(Vec<(usize, usize)>),
    Tail(usize),
    Raw,
}

/// Split `path:50-100`, `path:-60`, `path:raw`, `path:5-16,960-973`, `path:40-` and `path:12` into
/// the file and its selector. A name that exists as written, or whose suffix is not a selector,
/// is returned whole.
pub(super) fn split_path_spec(entry: &str) -> (&str, Selector) {
    if std::path::Path::new(entry).exists() {
        return (entry, Selector::Bare);
    }
    let Some((path, spec)) = entry.rsplit_once(':') else {
        return (entry, Selector::Bare);
    };
    match parse_spec(spec) {
        Some(sel) if !path.is_empty() => (path, sel),
        _ => (entry, Selector::Bare),
    }
}

fn parse_spec(spec: &str) -> Option<Selector> {
    if spec == "raw" {
        return Some(Selector::Raw);
    }
    if let Some(n) = spec.strip_prefix('-') {
        return n
            .parse::<usize>()
            .ok()
            .filter(|n| *n >= 1)
            .map(Selector::Tail);
    }
    let mut ranges = Vec::new();
    for part in spec.split(',') {
        let (a, b) = part.split_once('-').unwrap_or((part, part));
        let start = a.parse::<usize>().ok().filter(|s| *s >= 1)?;
        let end = if b.is_empty() {
            usize::MAX
        } else {
            b.parse::<usize>().ok().filter(|e| *e >= start)?
        };
        ranges.push((start, end));
    }
    (!ranges.is_empty()).then_some(Selector::Ranges(ranges))
}

/// Cap every line at [`LINE_CAP_BYTES`], on a char boundary, with a marker saying how much went.
/// Returns the text and whether any line was cut.
pub(super) fn cap_lines(text: &str) -> (String, bool) {
    let mut out = String::with_capacity(text.len());
    let mut cut = false;
    for (i, line) in text.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }
        if line.len() <= LINE_CAP_BYTES {
            out.push_str(line);
            continue;
        }
        let mut end = LINE_CAP_BYTES;
        while !line.is_char_boundary(end) {
            end -= 1;
        }
        out.push_str(&line[..end]);
        let _ = write!(out, " [… {} more bytes]", line.len() - end);
        cut = true;
    }
    (out, cut)
}

fn capped_note(cut: bool, path: &str) -> String {
    if cut {
        format!("\n[some lines were cut at {LINE_CAP_BYTES} bytes; `{path}:raw` returns the file verbatim]")
    } else {
        String::new()
    }
}

/// The text `selector` picks out of `content`, shaped. `page_from` is the 1-indexed first line for
/// an open-ended read (`start_line` with no end), which also gets the default page.
pub(super) fn shape(path: &str, content: &str, selector: &Selector) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let total = lines.len();
    match selector {
        Selector::Raw => content.to_string(),
        Selector::Tail(n) => {
            let start = total.saturating_sub(*n);
            let (body, cut) = cap_lines(&lines[start..].join("\n"));
            format!(
                "{body}\n[lines {}-{total} of {total}]{}",
                start + 1,
                capped_note(cut, path)
            )
        }
        Selector::Ranges(ranges) => {
            if let Some(&(s, _)) = ranges.first().filter(|(s, _)| *s > total.max(1)) {
                return format!("{path} has {total} lines; line {s} is past the end");
            }
            let mut out = String::new();
            let mut any_cut = false;
            let mut footer = String::new();
            for (n, &(s, e)) in ranges.iter().enumerate() {
                let open = e == usize::MAX;
                let end = if open {
                    (s + DEFAULT_PAGE_LINES - 1).min(total)
                } else {
                    e.min(total)
                };
                let start = s.min(total + 1);
                let slice = lines[(start - 1).min(total)..end.max(start - 1).min(total)].join("\n");
                let (body, cut) = cap_lines(&slice);
                any_cut |= cut;
                if n > 0 {
                    out.push_str("\n…\n");
                }
                out.push_str(&body);
                if open && end < total {
                    let _ = write!(
                        footer,
                        "\n[lines {s}-{end} of {total}; continue with `{path}:{}-{}`]",
                        end + 1,
                        end + DEFAULT_PAGE_LINES
                    );
                }
            }
            out.push_str(&footer);
            out.push_str(&capped_note(any_cut, path));
            out
        }
        Selector::Bare => page(path, content, &lines),
    }
}

/// A bare read: first [`DEFAULT_PAGE_LINES`] lines, with a footer to continue.
fn page(path: &str, content: &str, lines: &[&str]) -> String {
    let total = lines.len();
    if total <= DEFAULT_PAGE_LINES {
        let (body, cut) = cap_lines(content.strip_suffix('\n').unwrap_or(content));
        return format!("{body}{}", capped_note(cut, path));
    }
    let (body, cut) = cap_lines(&lines[..DEFAULT_PAGE_LINES].join("\n"));
    format!(
        "{body}\n[lines 1-{DEFAULT_PAGE_LINES} of {total}; continue with `{path}:{}-{}`, or `{path}:-60` for the tail]{}",
        DEFAULT_PAGE_LINES + 1,
        DEFAULT_PAGE_LINES * 2,
        capped_note(cut, path)
    )
}

/// Declarations-only view of a code file of [`OUTLINE_MIN_LINES`]+ lines: each definition's line
/// span and signature, then the non-blank stretches outside any definition, and how to read what
/// was left out. `entries` come from the tree-sitter outline (`L<start>-<end>  <kind> <name>`,
/// indented by depth); `None` when there is nothing to summarise.
pub(super) fn outline_summary(path: &str, lines: &[&str], entries: &[String]) -> Option<String> {
    if lines.len() < OUTLINE_MIN_LINES || entries.is_empty() {
        return None;
    }
    let mut covered = vec![false; lines.len() + 2];
    for e in entries {
        if let Some((s, t)) = entry_span(e) {
            for slot in covered.iter_mut().take(t.min(lines.len()) + 1).skip(s) {
                *slot = true;
            }
        }
    }
    let mut gaps: Vec<(usize, usize)> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let n = i + 1;
        if covered[n] || line.trim().is_empty() {
            continue;
        }
        match gaps.last_mut() {
            Some((_, e)) if *e + 1 >= n => *e = n,
            _ => gaps.push((n, n)),
        }
    }
    let mut out = format!(
        "[{path}: {} lines. Declarations only; bodies elided.]\n",
        lines.len()
    );
    let mut chars = 0;
    let mut shown = 0;
    for e in entries {
        chars += e.len() + 1;
        if chars > OUTLINE_MAX_CHARS {
            break;
        }
        out.push_str(e);
        out.push('\n');
        shown += 1;
    }
    if shown < entries.len() {
        let _ = writeln!(out, "… {} more entries", entries.len() - shown);
    }
    if !gaps.is_empty() {
        let list: Vec<String> = gaps
            .iter()
            .take(GAPS_LISTED)
            .map(|(s, e)| {
                if s == e {
                    s.to_string()
                } else {
                    format!("{s}-{e}")
                }
            })
            .collect();
        let more = gaps.len().saturating_sub(GAPS_LISTED);
        let _ = writeln!(
            out,
            "Outside definitions (imports, constants, top-level code): lines {}{}",
            list.join(", "),
            if more > 0 {
                format!(", +{more} more spans")
            } else {
                String::new()
            }
        );
    }
    let example = entries
        .iter()
        .find_map(|e| entry_span(e))
        .map(|(s, t)| format!("{s}-{t}"))
        .unwrap_or_else(|| "1-80".to_string());
    let _ = write!(
        out,
        "[elided: every body. Read what you need with `{path}:{example}` (comma-separate several \
         spans), `{path}:-60` for the tail, or `{path}:raw` for the whole file verbatim.]"
    );
    Some(out)
}

/// The whole shaped result of one single-file read: the outline for a long code file read bare,
/// the existing large-file map for a file too big to return whole, else the selected lines.
pub(super) fn shaped_read(path: &str, content: &str, selector: &Selector) -> String {
    if *selector == Selector::Bare {
        let lines: Vec<&str> = content.lines().collect();
        if lines.len() >= OUTLINE_MIN_LINES {
            let entries = super::code_outline(path, content);
            if let Some(summary) = outline_summary(path, &lines, &entries) {
                return summary;
            }
        }
        if content.len() > super::READ_WHOLE_MAX_BYTES && lines.len() <= DEFAULT_PAGE_LINES * 4 {
            return super::outline_read(path, content);
        }
    }
    super::cap_read(shape(path, content, selector))
}

fn entry_span(entry: &str) -> Option<(usize, usize)> {
    let rest = entry.trim_start().strip_prefix('L')?;
    let span = rest.split_whitespace().next()?;
    let (s, e) = span.split_once('-')?;
    Some((s.parse().ok()?, e.parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn numbered(n: usize) -> String {
        (1..=n).map(|i| format!("line {i}\n")).collect()
    }

    #[test]
    fn path_specs_split_into_selectors() {
        assert_eq!(
            split_path_spec("nope/a.rs:50-100"),
            ("nope/a.rs", Selector::Ranges(vec![(50, 100)]))
        );
        assert_eq!(
            split_path_spec("nope/a.rs:-60"),
            ("nope/a.rs", Selector::Tail(60))
        );
        assert_eq!(
            split_path_spec("nope/a.rs:raw"),
            ("nope/a.rs", Selector::Raw)
        );
        assert_eq!(
            split_path_spec("nope/a.rs:5-16,960-973"),
            ("nope/a.rs", Selector::Ranges(vec![(5, 16), (960, 973)]))
        );
        assert_eq!(
            split_path_spec("nope/a.rs:40-"),
            ("nope/a.rs", Selector::Ranges(vec![(40, usize::MAX)]))
        );
        assert_eq!(
            split_path_spec("nope/a.rs:12"),
            ("nope/a.rs", Selector::Ranges(vec![(12, 12)]))
        );
        assert_eq!(
            split_path_spec("nope/a.rs:abc"),
            ("nope/a.rs:abc", Selector::Bare)
        );
        assert_eq!(split_path_spec("nope/a.rs"), ("nope/a.rs", Selector::Bare));
    }

    #[test]
    fn a_range_returns_those_lines_only() {
        let out = shape("f", &numbered(500), &Selector::Ranges(vec![(10, 12)]));
        assert_eq!(out, "line 10\nline 11\nline 12");
    }

    #[test]
    fn tail_returns_the_last_lines_with_their_numbers() {
        let out = shape("f", &numbered(100), &Selector::Tail(3));
        assert!(out.starts_with("line 98\nline 99\nline 100"), "{out}");
        assert!(out.contains("[lines 98-100 of 100]"));
    }

    #[test]
    fn a_bare_read_pages_at_300_lines_and_says_how_to_continue() {
        let out = shape("f.txt", &numbered(450), &Selector::Bare);
        assert!(
            out.contains("line 300\n[lines 1-300 of 450; continue with `f.txt:301-600`"),
            "{out}"
        );
        assert!(!out.contains("line 301"));
        let small = shape("f.txt", &numbered(20), &Selector::Bare);
        assert_eq!(small, numbered(20).trim_end());
    }

    #[test]
    fn an_open_range_gets_a_page_and_a_footer() {
        let out = shape(
            "f",
            &numbered(900),
            &Selector::Ranges(vec![(100, usize::MAX)]),
        );
        assert!(
            out.contains("line 399\n[lines 100-399 of 900; continue with `f:400-699`"),
            "{out}"
        );
    }

    #[test]
    fn long_lines_are_cut_with_a_marker_and_raw_is_verbatim() {
        let long = format!("{}\nshort\n", "x".repeat(2000));
        let out = shape("m.js", &long, &Selector::Bare);
        assert!(out.contains("[… 1232 more bytes]"), "{out}");
        assert!(out.contains("m.js:raw"));
        assert_eq!(shape("m.js", &long, &Selector::Raw), long);
    }

    #[test]
    fn outline_lists_declarations_gaps_and_how_to_read_them() {
        let lines: Vec<String> = (1..=150).map(|i| format!("l{i}")).collect();
        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        let entries = vec![
            "L10-60  fn alpha() -> u32".to_string(),
            "  L20-30  fn inner()".to_string(),
            "L70-140  struct Beta".to_string(),
        ];
        let out = outline_summary("src/a.rs", &refs, &entries).unwrap();
        assert!(out.contains("L10-60  fn alpha"), "{out}");
        assert!(out.contains("lines 1-9, 61-69, 141-150"), "{out}");
        assert!(out.contains("`src/a.rs:10-60`"), "{out}");
        assert!(out.contains("src/a.rs:raw"));
        assert!(
            outline_summary("x", &refs[..50], &entries).is_none(),
            "short files are read whole"
        );
        assert!(outline_summary("x", &refs, &[]).is_none());
    }

    #[test]
    fn a_real_rust_file_reads_as_declarations_with_elided_ranges() {
        let mut src = String::from("use std::fmt;\nconst LIMIT: usize = 3;\n\n");
        for i in 0..25 {
            src.push_str(&format!(
                "pub fn handler_{i}(x: u32) -> u32 {{\n    let y = x + {i};\n    let z = y * 2;\n    z - 1\n}}\n\n"
            ));
        }
        let out = shaped_read("src/h.rs", &src, &Selector::Bare);
        assert!(out.contains("fn handler_3(x: u32) -> u32"), "{out}");
        assert!(!out.contains("let z = y * 2"), "bodies are elided: {out}");
        assert!(out.contains("src/h.rs:raw"), "{out}");
        let raw = shaped_read("src/h.rs", &src, &Selector::Raw);
        assert!(raw.contains("let z = y * 2"));
    }
}
