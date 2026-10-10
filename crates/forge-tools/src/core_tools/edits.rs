//! Matching an edit's `old` text, and refusing an edit that was cut off in transit.
//!
//! A model's `old` string is frequently *nearly* right — the indentation drifted, a trailing space
//! vanished — and a byte-exact-only matcher turns that into a failed turn. It is also frequently
//! *truncated*: the tail of a long replacement never arrived, and applying it would silently
//! delete the rest of a block. This module owns both halves of that judgement: the
//! uniqueness-preserving fallbacks that decide where an edit lands, and the balance heuristics
//! that decide it should not land at all. The tools themselves only do I/O.

/// Whitespace-insensitive fallback for `edit_file`: when `old` doesn't match the file byte-for-byte
/// (almost always a leading-indent / trailing-space difference), match it line-by-line ignoring each
/// line's surrounding whitespace. Returns the edited content ONLY when exactly one contiguous block
/// of lines matches (uniqueness preserved, so a near-miss can't hit the wrong place); otherwise
/// `None`. `new` is inserted verbatim, keeping the matched block's trailing newline.
pub(super) fn flexible_replace(content: &str, old: &str, new: &str) -> Option<String> {
    let old_lines: Vec<&str> = old.lines().map(str::trim).collect();
    if old_lines.is_empty() {
        return None;
    }
    // Lines WITH their `\n` terminators, so byte offsets reconstruct exactly.
    let segs: Vec<&str> = content.split_inclusive('\n').collect();
    if old_lines.len() > segs.len() {
        return None;
    }
    let mut hits = Vec::new();
    for i in 0..=(segs.len() - old_lines.len()) {
        if (0..old_lines.len()).all(|j| segs[i + j].trim() == old_lines[j]) {
            hits.push(i);
        }
    }
    if hits.len() != 1 {
        return None; // not found, or ambiguous — caller errors
    }
    let i = hits[0];
    let start: usize = segs[..i].iter().map(|s| s.len()).sum();
    let end: usize = start
        + segs[i..i + old_lines.len()]
            .iter()
            .map(|s| s.len())
            .sum::<usize>();
    let mut replacement = new.to_string();
    if content[start..end].ends_with('\n') && !replacement.ends_with('\n') {
        replacement.push('\n');
    }
    let mut out = String::with_capacity(content.len() - (end - start) + replacement.len());
    out.push_str(&content[..start]);
    out.push_str(&replacement);
    out.push_str(&content[end..]);
    Some(out)
}

/// Count whitespace-insensitive matches of `old` in `content`, so a failed edit can report whether
/// `old` was ambiguous (several near-matches) rather than simply missing.
fn flex_match_count(content: &str, old: &str) -> usize {
    let old_lines: Vec<&str> = old.lines().map(str::trim).collect();
    if old_lines.is_empty() {
        return 0;
    }
    let segs: Vec<&str> = content.split_inclusive('\n').collect();
    if old_lines.len() > segs.len() {
        return 0;
    }
    (0..=(segs.len() - old_lines.len()))
        .filter(|&i| (0..old_lines.len()).all(|j| segs[i + j].trim() == old_lines[j]))
        .count()
}

/// Anchored-block fallback for `edit_file`: the safety net for when a model paraphrases the *middle*
/// of a block but reproduces its first and last lines closely. Matches the first and last non-empty
/// trimmed lines of `old` as anchors and replaces the unique span between them — even if the interior
/// lines differ. Two rails keep this from silently eating the wrong region: (1) uniqueness — exactly
/// one anchored span may match, else `None` (caller errors); (2) disproportion — the matched span may
/// not balloon past `old` (≥ max(old+3, old*2) lines or max(+500, *4) chars), which would mean the
/// anchors landed too far apart. Only attempted for `old` of ≥3 lines; 1–2 line edits are fully
/// covered by exact + whitespace match.
pub(super) fn block_anchor_replace(content: &str, old: &str, new: &str) -> Option<String> {
    let old_lines: Vec<&str> = old.lines().collect();
    if old_lines.len() < 3 {
        return None;
    }
    let trimmed: Vec<&str> = old_lines.iter().map(|l| l.trim()).collect();
    let first_idx = trimmed.iter().position(|l| !l.is_empty())?;
    let last_idx = trimmed.iter().rposition(|l| !l.is_empty())?;
    if last_idx <= first_idx {
        return None; // need two distinct anchor lines
    }
    let (first, last) = (trimmed[first_idx], trimmed[last_idx]);

    let segs: Vec<&str> = content.split_inclusive('\n').collect();
    let old_len = old_lines.len();
    // Disproportion cap (lines); also bounds the forward search for the closing anchor.
    let max_span = old_len.saturating_mul(2).max(old_len + 3);

    let mut spans: Vec<(usize, usize)> = Vec::new();
    for (s, seg) in segs.iter().enumerate() {
        if seg.trim() != first {
            continue;
        }
        let hi = (s + max_span).min(segs.len());
        for (e, eseg) in segs.iter().enumerate().take(hi).skip(s + 1) {
            if eseg.trim() == last {
                spans.push((s, e)); // nearest closing anchor for this start
                break;
            }
        }
    }
    if spans.len() != 1 {
        return None; // not found, or ambiguous
    }
    let (s, e) = spans[0];
    if e - s + 1 >= max_span {
        return None; // disproportionate (lines)
    }
    let start: usize = segs[..s].iter().map(|x| x.len()).sum();
    let end: usize = start + segs[s..=e].iter().map(|x| x.len()).sum::<usize>();
    let old_chars = old.len();
    if end - start >= (old_chars + 500).max(old_chars.saturating_mul(4)) {
        return None; // disproportionate (chars)
    }

    let mut replacement = new.to_string();
    if content[start..end].ends_with('\n') && !replacement.ends_with('\n') {
        replacement.push('\n');
    }
    let mut out = String::with_capacity(content.len() - (end - start) + replacement.len());
    out.push_str(&content[..start]);
    out.push_str(&replacement);
    out.push_str(&content[end..]);
    Some(out)
}

pub(super) fn looks_truncated(new: &str) -> bool {
    !new.is_empty()
        && new
            .chars()
            .scan(false, |escaped, ch| {
                let is_unescaped_quote = ch == '"' && !*escaped;
                *escaped = ch == '\\';
                Some(is_unescaped_quote)
            })
            .filter(|is_quote| *is_quote)
            .count()
            % 2
            == 1
}

/// Net *unclosed* count for `(`, `{`, `[` in `s` — `max(0, opens - closes)` for each — skipping
/// characters inside `"..."`/`'...'` literals and `//` / `/* */` comments (best-effort, not a
/// full lexer: raw strings, Python triple-quotes, and `#`-comments aren't specially handled).
/// A bare `'` is only treated as opening a char literal when it actually closes nearby (`'x'`,
/// `'\n'`, `'\u{1F600}'`); otherwise it's left alone so Rust lifetimes/labels (`'a`, `'static`)
/// don't swallow the rest of the file as "inside a string".
fn bracket_net_open(s: &str) -> (i64, i64, i64) {
    #[derive(PartialEq)]
    enum St {
        Code,
        Str(char),
        Line,
        Block,
    }
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len();
    let (mut paren, mut brace, mut square) = (0i64, 0i64, 0i64);
    let mut st = St::Code;
    let mut i = 0;
    while i < n {
        let ch = chars[i];
        match st {
            St::Line => {
                if ch == '\n' {
                    st = St::Code;
                }
                i += 1;
            }
            St::Block => {
                if ch == '*' && i + 1 < n && chars[i + 1] == '/' {
                    st = St::Code;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            St::Str(q) => {
                if ch == '\\' && i + 1 < n {
                    i += 2;
                } else {
                    if ch == q {
                        st = St::Code;
                    }
                    i += 1;
                }
            }
            St::Code => match ch {
                '"' => {
                    st = St::Str('"');
                    i += 1;
                }
                '\'' => {
                    if let Some(close) = char_literal_end(&chars, i) {
                        i = close + 1;
                    } else {
                        i += 1; // lifetime/label tick, not a string
                    }
                }
                '/' if i + 1 < n && chars[i + 1] == '/' => {
                    st = St::Line;
                    i += 2;
                }
                '/' if i + 1 < n && chars[i + 1] == '*' => {
                    st = St::Block;
                    i += 2;
                }
                '(' => {
                    paren += 1;
                    i += 1;
                }
                ')' => {
                    paren -= 1;
                    i += 1;
                }
                '{' => {
                    brace += 1;
                    i += 1;
                }
                '}' => {
                    brace -= 1;
                    i += 1;
                }
                '[' => {
                    square += 1;
                    i += 1;
                }
                ']' => {
                    square -= 1;
                    i += 1;
                }
                _ => {
                    i += 1;
                }
            },
        }
    }
    (paren.max(0), brace.max(0), square.max(0))
}

/// If `chars[start]` (a `'`) opens a char literal — `'x'`, or an escape like `'\n'` / `'\''` /
/// `'\u{1F600}'` — returns the index of its closing `'`. Returns `None` for a Rust
/// lifetime/label tick (`'a`, `'static`), which never closes, so callers can leave it alone.
fn char_literal_end(chars: &[char], start: usize) -> Option<usize> {
    let n = chars.len();
    let mut i = start + 1;
    if i >= n {
        return None;
    }
    if chars[i] == '\\' {
        i += 1;
        if i >= n {
            return None;
        }
        if chars[i] == 'u' && i + 1 < n && chars[i + 1] == '{' {
            i += 2;
            while i < n && chars[i] != '}' {
                i += 1;
            }
            if i >= n {
                return None;
            }
            i += 1; // consume '}'
        } else {
            i += 1; // consume the escaped char itself
        }
    } else {
        i += 1; // a single plain char
    }
    if i < n && chars[i] == '\'' {
        Some(i)
    } else {
        None
    }
}

/// True when `after` has MORE net-unclosed brackets than `before`, for `(`, `{`, or `[` — the
/// signature of an edit that cut a block off mid-way. Comparing to `before` (rather than judging
/// `after` in isolation) means pre-existing imbalance, or a snippet that legitimately opens a
/// bracket the surrounding untouched file closes, doesn't trip it — only a REGRESSION does.
fn looks_bracket_truncated(before: &str, after: &str) -> bool {
    let (bp, bb, bs) = bracket_net_open(before);
    let (ap, ab, asq) = bracket_net_open(after);
    ap > bp || ab > bb || asq > bs
}

/// Locate the line in `content` that best matches `old`'s first non-empty line, so a "not found"
/// error can point at *where the model probably meant* instead of just saying no. Tries an exact
/// trimmed-line match first, then a substring probe on a short prefix of that line. Returns
/// `line_number, surrounding snippet` (1-indexed, one line of context on each side).
fn closest_candidate(content: &str, old: &str) -> Option<String> {
    let anchor = old.lines().map(str::trim).find(|l| !l.is_empty())?;
    let lines: Vec<&str> = content.lines().collect();
    let hit = lines.iter().position(|l| l.trim() == anchor).or_else(|| {
        let probe: String = anchor.chars().take(24).collect();
        (probe.trim().len() >= 6)
            .then(|| lines.iter().position(|l| l.contains(probe.trim())))
            .flatten()
    })?;
    let start = hit.saturating_sub(1);
    let end = (hit + 2).min(lines.len());
    let snippet: String = lines[start..end]
        .iter()
        .enumerate()
        .map(|(i, l)| format!("{}: {l}", start + i + 1))
        .collect::<Vec<_>>()
        .join("\n");
    Some(format!(
        "closest candidate near line {}:\n{snippet}",
        hit + 1
    ))
}

fn not_found_message(content: &str, old: &str) -> String {
    let n_lines = old.lines().count();
    let mut msg = format!(
        "`old` ({n_lines} lines) not found (also tried whitespace-insensitive and \
         block-anchor matches). The file may have changed — re-read the exact lines \
         with read_file and copy them verbatim into `old` before editing"
    );
    if n_lines > 40 {
        msg.push_str(
            "; `old` is large, so split it into several smaller edits, or use \
             write_file if you are creating a brand-new file",
        );
    }
    if let Some(candidate) = closest_candidate(content, old) {
        msg.push_str("; ");
        msg.push_str(&candidate);
    }
    msg
}

/// Apply one `old → new` replacement to `content`: an exact single match, else a UNIQUE
/// whitespace-insensitive fallback ([`flexible_replace`]). Returns `(updated, note)` or a
/// human-readable error. Shared by [`EditFileTool`] and [`MultiEditTool`].
pub(super) fn apply_edit(
    content: &str,
    old: &str,
    new: &str,
) -> Result<(String, &'static str), String> {
    if looks_truncated(new) {
        return Err("the replacement text looks truncated (unbalanced quotes — it was probably cut off). Make a smaller, targeted edit, or use write_file for a brand-new file."
            .to_string()
        );
    }

    let (updated, note) = match content.matches(old).count() {
        1 => Ok((content.replacen(old, new, 1), "")),
        0 => flexible_replace(content, old, new)
            .map(|u| (u, " (matched ignoring whitespace)"))
            .or_else(|| {
                block_anchor_replace(content, old, new).map(|u| (u, " (matched on block anchors)"))
            })
            .ok_or_else(|| {
                // A whitespace-insensitive ambiguous match also lands here (flexible_replace
                // returns None on >1 hit); surface that distinctly so the model adds context
                // rather than re-reading in vain.
                let flex_hits = flex_match_count(content, old);
                if flex_hits > 1 {
                    return format!(
                        "`old` matches {flex_hits} places (ignoring whitespace) — ambiguous; \
                         add surrounding context so it matches exactly once"
                    );
                }
                not_found_message(content, old)
            }),
        n => Err(format!(
            "`old` is ambiguous: {n} occurrences — add surrounding context, or pass \
             `replace_all: true` if every occurrence should change"
        )),
    }?;

    if looks_bracket_truncated(content, &updated) {
        return Err(
            "the edit looks truncated (unbalanced brackets — it leaves a `{`, `(`, or `[` \
             unclosed that wasn't unclosed before, as if it was cut off mid-block). Make a \
             smaller, targeted edit, or use write_file for a brand-new file."
                .to_string(),
        );
    }

    Ok((updated, note))
}

/// Replace EVERY occurrence of `old` with `new` — the explicit, opt-in counterpart to
/// [`apply_edit`]'s uniqueness requirement, for edits the model intends to apply everywhere
/// (a rename, a repeated literal). Exact matches only: a replace-all that also had to guess at
/// whitespace drift could silently rewrite the wrong span, so this doesn't fall through to the
/// fuzzy tiers — it either finds `old` byte-for-byte or reports the same actionable error.
pub(super) fn apply_edit_all(
    content: &str,
    old: &str,
    new: &str,
) -> Result<(String, &'static str), String> {
    if looks_truncated(new) {
        return Err("the replacement text looks truncated (unbalanced quotes — it was probably cut off). Make a smaller, targeted edit, or use write_file for a brand-new file."
            .to_string()
        );
    }
    let count = content.matches(old).count();
    if count == 0 {
        return Err(not_found_message(content, old));
    }
    let updated = content.replace(old, new);
    if looks_bracket_truncated(content, &updated) {
        return Err(
            "the edit looks truncated (unbalanced brackets — it leaves a `{`, `(`, or `[` \
             unclosed that wasn't unclosed before, as if it was cut off mid-block). Make a \
             smaller, targeted edit, or use write_file for a brand-new file."
                .to_string(),
        );
    }
    Ok((updated, " (replaced all)"))
}

/// Dispatch to [`apply_edit`] or, when `replace_all`, [`apply_edit_all`].
pub(super) fn apply_edit_mode(
    content: &str,
    old: &str,
    new: &str,
    replace_all: bool,
) -> Result<(String, &'static str), String> {
    if replace_all {
        apply_edit_all(content, old, new)
    } else {
        apply_edit(content, old, new)
    }
}

/// One `multi_edit` step: `old` → `new`, plus whether every occurrence should change
/// (`replace_all`) rather than requiring a unique match.
pub(super) struct EditStep {
    pub old: String,
    pub new: String,
    pub replace_all: bool,
}

/// A few numbered lines around where `new` landed in `updated`, so the model can see the result of
/// an edit without a follow-up read. Empty when `new` is blank or can't be located.
pub(super) fn edit_snippet(updated: &str, new: &str) -> String {
    const MAX_LINES: usize = 10;
    const MAX_LINE_CHARS: usize = 160;
    if new.trim().is_empty() {
        return String::new();
    }
    let Some(pos) = updated.find(new) else {
        return String::new();
    };
    let lines: Vec<&str> = updated.lines().collect();
    let first = updated[..pos].matches('\n').count();
    let from = first.saturating_sub(1);
    let to =
        (first + new.lines().count().clamp(1, MAX_LINES - 2)).min(lines.len().saturating_sub(1));
    let body = (from..=to)
        .map(|n| {
            let text: String = lines[n].chars().take(MAX_LINE_CHARS).collect();
            format!("{}\t{text}", n + 1)
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!("\n{body}")
}

/// Fold the edits over `content` in order (each on the running result), all-or-nothing: the first
/// edit that can't apply aborts with `edit #k: <reason>` and the caller writes nothing.
pub(super) fn apply_edits(content: &str, edits: &[EditStep]) -> Result<String, String> {
    let mut cur = content.to_string();
    for (k, step) in edits.iter().enumerate() {
        let (next, _) = apply_edit_mode(&cur, &step.old, &step.new, step.replace_all)
            .map_err(|e| format!("edit #{}: {e}", k + 1))?;
        cur = next;
    }
    Ok(cur)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(old: &str, new: &str) -> EditStep {
        EditStep {
            old: old.to_string(),
            new: new.to_string(),
            replace_all: false,
        }
    }

    #[test]
    fn edit_snippet_numbers_the_lines_around_the_change() {
        let updated = "a\nb\nNEW1\nNEW2\ne\nf\n";
        assert_eq!(
            edit_snippet(updated, "NEW1\nNEW2"),
            "\n2\tb\n3\tNEW1\n4\tNEW2\n5\te"
        );
        assert_eq!(edit_snippet(updated, "  "), "");
        assert_eq!(edit_snippet(updated, "absent"), "");
    }

    #[test]
    fn apply_edits_is_atomic_and_ordered() {
        let content = "a = 1\nb = 2\nc = 3\n";
        // Both edits apply, in order.
        let out =
            apply_edits(content, &[step("a = 1", "a = 10"), step("c = 3", "c = 30")]).unwrap();
        assert_eq!(out, "a = 10\nb = 2\nc = 30\n");
        // A failing edit aborts the whole batch (atomic) and names which one.
        let err = apply_edits(content, &[step("a = 1", "a = 10"), step("nope", "x")]).unwrap_err();
        assert!(err.starts_with("edit #2:"), "names the failing edit: {err}");
    }

    #[test]
    fn apply_edit_all_replaces_every_occurrence() {
        let content = "x = 1;\nx = 1;\nx = 1;\n";
        let (out, note) = apply_edit_all(content, "x = 1;", "x = 2;").unwrap();
        assert_eq!(out, "x = 2;\nx = 2;\nx = 2;\n");
        assert_eq!(note, " (replaced all)");
        // Absent `old` is still reported, not silently a no-op.
        assert!(apply_edit_all(content, "nope", "x").is_err());
    }

    #[test]
    fn not_found_error_includes_closest_candidate_with_line_number() {
        let content = "fn a() {}\nfn target_fn() {\n    old_body();\n}\nfn z() {}\n";
        // The anchor line (`old`'s first line) matches content line 2 exactly, but the second
        // line doesn't — too short for the block-anchor tier, so this falls all the way through
        // to "not found", which should still point at where the model probably meant.
        let err =
            apply_edit(content, "fn target_fn() {\n    totally_different();", "x").unwrap_err();
        assert!(err.contains("closest candidate near line 2"), "{err}");
        assert!(err.contains("old_body();"), "{err}");
    }

    #[test]
    fn flexible_replace_matches_ignoring_whitespace_when_unique() {
        let content = "fn a() {\n        let x = 1;\n}\n";
        // `old` has different indentation than the file — exact match would miss.
        let out = flexible_replace(content, "let x = 1;", "let x = 2;").unwrap();
        assert!(out.contains("let x = 2;"), "replaced: {out:?}");
        assert!(out.starts_with("fn a() {\n"), "rest preserved: {out:?}");
        assert!(out.ends_with("}\n"), "trailing preserved: {out:?}");
        // Ambiguous → None (two whitespace-equal matches).
        let dup = "  v\n    v\n";
        assert!(
            flexible_replace(dup, "v", "w").is_none(),
            "ambiguous → None"
        );
        // Genuinely absent → None.
        assert!(flexible_replace(content, "let y = 9;", "z").is_none());
    }

    #[test]
    fn block_anchor_replace_matches_when_middle_paraphrased() {
        // The model reproduced the first/last lines but rewrote the interior. Exact and
        // whitespace matching both miss; anchors recover it.
        let content = "fn f() {\n    let a = 1;\n    let b = 2;\n    return a + b;\n}\n";
        let old = "fn f() {\n    let a = 0;\n    return a;\n}";
        let new = "fn f() {\n    return 42;\n}";
        let out = block_anchor_replace(content, old, new).expect("anchors match");
        assert_eq!(
            out, "fn f() {\n    return 42;\n}\n",
            "interior replaced: {out:?}"
        );
    }

    #[test]
    fn block_anchor_replace_rejects_ambiguous_and_disproportionate() {
        // Two spans share the same first/last anchors → ambiguous → None.
        let dup = "a\nx\nb\na\ny\nb\n";
        assert!(
            block_anchor_replace(dup, "a\nq\nb", "z").is_none(),
            "ambiguous anchors → None"
        );
        // Anchors land far apart (interior dwarfs `old`) → disproportionate → None.
        let huge = format!("open\n{}close\n", "filler\n".repeat(50));
        let old = "open\nx\nclose";
        assert!(
            block_anchor_replace(&huge, old, "y").is_none(),
            "disproportionate span → None"
        );
        // 1–2 line `old` is never block-anchored (exact/whitespace own that case).
        assert!(block_anchor_replace("a\nb\n", "a\nb", "c").is_none());
    }

    #[test]
    fn apply_edit_falls_through_exact_then_whitespace_then_anchor() {
        // Exact wins outright.
        let (out, note) = apply_edit("let x = 1;\n", "let x = 1;", "let x = 2;").unwrap();
        assert_eq!(out, "let x = 2;\n");
        assert_eq!(note, "");
        // Falls through to the block-anchor tier when the interior drifts.
        let content = "fn g() {\n    old_body();\n    more();\n}\n";
        let (out, note) = apply_edit(
            content,
            "fn g() {\n    body();\n}",
            "fn g() {\n    new();\n}",
        )
        .unwrap();
        assert_eq!(out, "fn g() {\n    new();\n}\n");
        assert_eq!(note, " (matched on block anchors)");
    }

    #[test]
    fn looks_truncated_detects_odd_unescaped_quotes() {
        assert!(looks_truncated("let s = \"hello;"));
        assert!(!looks_truncated("let s = \"hello\";"));
        assert!(!looks_truncated("fn x() {}"));
    }

    #[test]
    fn apply_edit_rejects_truncated_replacement() {
        let err =
            apply_edit("let s = \"old\";\n", "let s = \"old\";", "let s = \"new;").unwrap_err();
        assert!(err.contains("truncated"), "{err}");
    }

    #[test]
    fn apply_edit_rejects_truncated_bracket_replacement() {
        let err = apply_edit(
            "fn f() {\n    body();\n}\n",
            "    body();",
            "    if cond {\n        body();",
        )
        .unwrap_err();
        assert!(err.contains("truncated"), "{err}");
        assert!(err.contains("brackets"), "{err}");
    }

    #[test]
    fn apply_edit_allows_balanced_bracket_replacement() {
        let (out, _) = apply_edit(
            "fn f() {\n    old();\n}\n",
            "    old();",
            "    if cond {\n        new();\n    }",
        )
        .unwrap();
        assert!(out.contains("if cond {"), "{out}");
    }

    #[test]
    fn apply_edit_not_found_advises_rereading() {
        let err = apply_edit("fn main() {}\n", "fn nope() {}", "x").unwrap_err();
        assert!(err.contains("not found"), "{err}");
        assert!(err.contains("re-read"), "{err}");
    }

    #[test]
    fn apply_edit_ambiguous_advises_adding_context() {
        let err = apply_edit("let x = 1;\nlet x = 1;\n", "let x = 1;", "let x = 2;").unwrap_err();
        assert!(err.contains("ambiguous"), "{err}");
        assert!(err.contains("context"), "{err}");
    }

    #[test]
    fn apply_edit_large_not_found_suggests_split_or_write_file() {
        let old = (0..50)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let err = apply_edit("fn main() {}\n", &old, "x").unwrap_err();
        assert!(err.contains("split"), "{err}");
        assert!(err.contains("write_file"), "{err}");
    }

    #[test]
    fn bracket_net_open_ignores_strings_comments_and_lifetimes() {
        // Brackets inside a string or a comment don't count.
        assert_eq!(bracket_net_open(r#"let s = "}{)((["; "#), (0, 0, 0));
        assert_eq!(
            bracket_net_open("// a stray { in a comment\nfn f() {}"),
            (0, 0, 0)
        );
        assert_eq!(
            bracket_net_open("/* block { comment */ fn f() {}"),
            (0, 0, 0)
        );
        // Rust lifetimes/labels aren't mistaken for an unterminated char literal that would
        // swallow the rest of the file.
        assert_eq!(
            bracket_net_open("fn f<'a>(x: &'a str) -> &'a str { x }"),
            (0, 0, 0)
        );
        // A genuine char literal is still skipped correctly.
        assert_eq!(bracket_net_open("let c = '{'; fn f() {}"), (0, 0, 0));
        // Real unclosed brackets are still counted.
        assert_eq!(bracket_net_open("fn f() {\n    if x {\n"), (0, 2, 0));
    }

    #[test]
    fn looks_bracket_truncated_flags_a_new_regression_only() {
        // (a) truncated mid-block: edit introduces an unclosed `{` that wasn't there before.
        assert!(looks_bracket_truncated(
            "fn f() {\n    body();\n}\n",
            "fn f() {\n    if cond {\n        body();\n"
        ));
        // (b) a normal, fully balanced edit is not flagged.
        assert!(!looks_bracket_truncated(
            "fn f() {\n    old();\n}\n",
            "fn f() {\n    new();\n}\n"
        ));
        // (c) brackets inside strings/comments in the new text don't trip it.
        assert!(!looks_bracket_truncated(
            "fn f() {}\n",
            "fn f() {\n    let s = \"}{[(\";\n    // } also fine\n}\n"
        ));
        // (d) file was already imbalanced before the edit and stays equally imbalanced — not a
        // NEW regression, so it's not flagged even though the absolute count is nonzero.
        assert!(!looks_bracket_truncated(
            "fn f() {\n    if x {\n",
            "fn f() {\n    if y {\n"
        ));
    }
}
