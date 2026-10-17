//! Diffs: the rows a tool card carries, the plain export, and the remote projection.
//!
//! One `similar` computation feeds all three, so the card, the `/output` text and the phone show
//! the same hunks. The card rows are `<line number> <sigil> <text>` with a full-row tint (where
//! backgrounds are painted) and a brighter tint on the changed words of a rewritten line.

use crate::copy_mark;
use crate::theme::{glyphs::g, styles, t};
use forge_types::{DiffKind, FileDiff};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use similar::{ChangeTag, TextDiff};
use syntect::easy::HighlightLines;

use super::{highlighted_spans, highlighter};

/// Cap on rendered changed lines before truncating (keeps a huge diff from flooding scrollback).
pub(super) const MAX_DIFF_LINES: usize = 500;

/// A rewritten line is only word-diffed when the two versions are at least this similar;
/// below it the whole line is simply replaced and highlighting words would be noise.
const WORD_DIFF_MIN_RATIO: f32 = 0.35;

/// The rows of one file's diff, ready to sit inside a tool card.
#[derive(Debug, Clone)]
pub struct DiffRows {
    /// Hunk separators and change/context rows, at most the requested number of body rows.
    pub rows: Vec<Line<'static>>,
    /// Body rows that did not fit (`… N more lines`).
    pub hidden: usize,
}

struct Item {
    tag: ChangeTag,
    /// 1-based line number: the old file's for a deletion, the new file's otherwise.
    no: usize,
    text: String,
}

struct Hunk {
    header: String,
    items: Vec<Item>,
}

fn hunks(td: &TextDiff<'_, '_, str>) -> Vec<Hunk> {
    let mut out = Vec::new();
    for group in td.grouped_ops(3) {
        let (Some(first), Some(last)) = (group.first(), group.last()) else {
            continue;
        };
        let (os, oe) = (first.old_range().start, last.old_range().end);
        let (ns, ne) = (first.new_range().start, last.new_range().end);
        let mut items = Vec::new();
        for op in &group {
            for change in td.iter_changes(op) {
                let no = match change.tag() {
                    ChangeTag::Delete => change.old_index().unwrap_or(0) + 1,
                    _ => change.new_index().unwrap_or(0) + 1,
                };
                items.push(Item {
                    tag: change.tag(),
                    no,
                    text: change.value().trim_end_matches('\n').replace('\t', "    "),
                });
            }
        }
        out.push(Hunk {
            header: format!("@@ -{},{} +{},{} @@", os + 1, oe - os, ns + 1, ne - ns),
            items,
        });
    }
    out
}

/// `(old, new)` word segments of a rewritten line, each flagged when it is the changed part.
type Segments = (Vec<(String, bool)>, Vec<(String, bool)>);

fn word_segments(old: &str, new: &str) -> Option<Segments> {
    let td = TextDiff::from_words(old, new);
    if td.ratio() < WORD_DIFF_MIN_RATIO {
        return None;
    }
    let (mut a, mut b) = (Vec::new(), Vec::new());
    for change in td.iter_all_changes() {
        let text = change.value().to_string();
        match change.tag() {
            ChangeTag::Equal => {
                a.push((text.clone(), false));
                b.push((text, false));
            }
            ChangeTag::Delete => a.push((text, true)),
            ChangeTag::Insert => b.push((text, true)),
        }
    }
    Some((a, b))
}

/// Word emphasis for each item: a run of deletions followed by a run of insertions is paired
/// line by line.
fn emphasis(items: &[Item]) -> Vec<Option<Vec<(String, bool)>>> {
    let mut out: Vec<Option<Vec<(String, bool)>>> = vec![None; items.len()];
    let mut i = 0;
    while i < items.len() {
        if items[i].tag != ChangeTag::Delete {
            i += 1;
            continue;
        }
        let del_start = i;
        while i < items.len() && items[i].tag == ChangeTag::Delete {
            i += 1;
        }
        let ins_start = i;
        while i < items.len() && items[i].tag == ChangeTag::Insert {
            i += 1;
        }
        for k in 0..(ins_start - del_start).min(i - ins_start) {
            if let Some((a, b)) =
                word_segments(&items[del_start + k].text, &items[ins_start + k].text)
            {
                out[del_start + k] = Some(a);
                out[ins_start + k] = Some(b);
            }
        }
    }
    out
}

fn tint(add: bool) -> Style {
    let p = t().palette;
    if add { p.diff_add_bg } else { p.diff_del_bg }.style()
}

fn word_style(add: bool) -> Style {
    let p = t().palette;
    if add {
        p.diff_add_word
    } else {
        p.diff_del_word
    }
    .style()
}

/// Counts over the whole change, not just the rows that fit.
pub fn diff_stat(diff: &FileDiff) -> (usize, usize) {
    if diff.binary {
        return (0, 0);
    }
    let td = TextDiff::from_lines(
        diff.old.as_deref().unwrap_or(""),
        diff.new.as_deref().unwrap_or(""),
    );
    td.iter_all_changes()
        .fold((0, 0), |(a, d), c| match c.tag() {
            ChangeTag::Insert => (a + 1, d),
            ChangeTag::Delete => (a, d + 1),
            ChangeTag::Equal => (a, d),
        })
}

/// The diff as card body rows (no card gutter, no rail): at most `max_rows` change/context rows.
pub fn diff_rows(diff: &FileDiff, max_rows: usize) -> DiffRows {
    if diff.binary {
        let o = diff.old.as_ref().map(|s| s.len()).unwrap_or(0);
        let n = diff.new.as_ref().map(|s| s.len()).unwrap_or(0);
        return DiffRows {
            rows: vec![Line::from(Span::styled(
                format!("binary file ({o} → {n} bytes)"),
                styles::muted(),
            ))],
            hidden: 0,
        };
    }
    let td = TextDiff::from_lines(
        diff.old.as_deref().unwrap_or(""),
        diff.new.as_deref().unwrap_or(""),
    );
    let hunks = hunks(&td);
    let (ss, theme) = highlighter();
    let lang = diff.lang.as_deref().unwrap_or("");
    let syntax = (!lang.is_empty())
        .then(|| ss.find_syntax_by_token(lang))
        .flatten()
        .unwrap_or_else(|| ss.find_syntax_plain_text());
    let mut hl = HighlightLines::new(syntax, theme);
    let num_w = hunks
        .iter()
        .flat_map(|h| h.items.iter().map(|i| i.no))
        .max()
        .unwrap_or(1)
        .to_string()
        .len();
    let glyphs = g();
    let sep = diff.kind == DiffKind::Modified || hunks.len() > 1;
    let mut rows: Vec<Line<'static>> = Vec::new();
    let (mut shown, mut hidden) = (0usize, 0usize);
    for hunk in &hunks {
        if sep && shown < max_rows {
            let bar = if glyphs.unicode { "┄┄" } else { "--" };
            rows.push(Line::from(Span::styled(
                format!("{bar} {} {bar}", hunk.header),
                copy_mark::frame_row(styles::faint()),
            )));
        }
        let emph = emphasis(&hunk.items);
        for (item, words) in hunk.items.iter().zip(emph) {
            if shown >= max_rows {
                hidden += 1;
                continue;
            }
            shown += 1;
            let (sigil, sigil_style, row_tint) = match item.tag {
                ChangeTag::Delete => (glyphs.diff_minus, styles::err(), Some(false)),
                ChangeTag::Insert => ("+", styles::ok(), Some(true)),
                ChangeTag::Equal => (" ", styles::faint(), None),
            };
            let mut spans = vec![
                Span::styled(
                    format!("{:>num_w$} ", item.no),
                    copy_mark::decor(styles::faint()),
                ),
                Span::styled(format!("{sigil} "), copy_mark::decor(sigil_style)),
            ];
            match (words, row_tint) {
                (Some(segments), Some(add)) => {
                    for (text, changed) in segments {
                        let style = if changed {
                            styles::fg().patch(word_style(add))
                        } else {
                            styles::fg()
                        };
                        spans.push(Span::styled(text, style));
                    }
                    // Keep the highlighter's state in step with the rows it did not render.
                    let _ = highlighted_spans(&mut hl, ss, &item.text);
                }
                _ => spans.extend(highlighted_spans(&mut hl, ss, &item.text)),
            }
            let mut line = Line::from(spans);
            if let Some(add) = row_tint {
                line = line.style(tint(add));
            }
            rows.push(line);
        }
    }
    DiffRows { rows, hidden }
}

/// The whole diff as scrollback rows (no header; the card or caller names the file). Used where
/// a diff stands alone, and by the expanded card, which shows up to [`MAX_DIFF_LINES`] rows.
pub fn diff_to_lines(diff: &FileDiff) -> Vec<Line<'static>> {
    let mut r = diff_rows(diff, MAX_DIFF_LINES);
    if r.hidden > 0 {
        r.rows.push(Line::from(Span::styled(
            format!(
                "{} {} more lines (full change in the tool result)",
                g().ellipsis,
                r.hidden
            ),
            styles::muted(),
        )));
    }
    if r.rows.is_empty() {
        r.rows.push(Line::from(Span::styled(
            "(no textual changes)",
            styles::muted(),
        )));
    }
    r.rows
}

/// `new file` / `modified` / `deleted`.
fn diff_kind_label(diff: &FileDiff) -> &'static str {
    match diff.kind {
        DiffKind::Created => "new file",
        DiffKind::Modified => "modified",
        DiffKind::Deleted => "deleted",
    }
}

fn diff_header(diff: &FileDiff) -> String {
    format!("{} ({})", diff.path, diff_kind_label(diff))
}

/// Cap on hunk lines carried per file in the remote diff projection ([`diff_file_snapshot`]).
/// A phone card previews the change; anything beyond shows as "+N more lines" and the full
/// content stays host-side (the TUI renders up to [`MAX_DIFF_LINES`]).
pub const REMOTE_DIFF_MAX_HUNK_LINES: usize = 40;

/// Project one [`FileDiff`] into the plain-data shape the remote snapshot carries — the SAME
/// `similar`-computed hunks `diff_to_lines` renders locally (grouped ops, 3 context lines),
/// minus the styling: hunk headers keep the `@@ -a,b +c,d @@` old/new line spans, body lines
/// keep their `+`/`-`/` ` gutter as the first character. `adds`/`dels` count the WHOLE change
/// (not just the carried lines); lines beyond `max_lines` are dropped into `skipped_lines` so
/// the page can say "+N more".
pub fn diff_file_snapshot(diff: &FileDiff, max_lines: usize) -> crate::DiffFileSnapshot {
    let kind = match diff.kind {
        DiffKind::Created => "created",
        DiffKind::Modified => "modified",
        DiffKind::Deleted => "deleted",
    };
    let mut snap = crate::DiffFileSnapshot {
        path: diff.path.clone(),
        kind: kind.to_string(),
        binary: diff.binary,
        adds: 0,
        dels: 0,
        hunks: Vec::new(),
        skipped_lines: 0,
    };
    if diff.binary {
        return snap;
    }
    let td = TextDiff::from_lines(
        diff.old.as_deref().unwrap_or(""),
        diff.new.as_deref().unwrap_or(""),
    );
    let mut emitted = 0usize;
    for group in td.grouped_ops(3) {
        let (Some(first), Some(last)) = (group.first(), group.last()) else {
            continue;
        };
        let os = first.old_range().start;
        let oe = last.old_range().end;
        let ns = first.new_range().start;
        let ne = last.new_range().end;
        let mut lines: Vec<String> = Vec::new();
        for op in &group {
            for change in td.iter_changes(op) {
                let sym = match change.tag() {
                    ChangeTag::Delete => {
                        snap.dels += 1;
                        '-'
                    }
                    ChangeTag::Insert => {
                        snap.adds += 1;
                        '+'
                    }
                    ChangeTag::Equal => ' ',
                };
                if emitted < max_lines {
                    let text = change.value().trim_end_matches('\n');
                    lines.push(format!("{sym}{text}"));
                    emitted += 1;
                } else {
                    snap.skipped_lines += 1;
                }
            }
        }
        if !lines.is_empty() {
            snap.hunks.push(crate::DiffHunkSnapshot {
                header: format!("@@ -{},{} +{},{} @@", os + 1, oe - os, ns + 1, ne - ns),
                lines,
            });
        }
    }
    snap
}

/// Plain unified-diff text (no ANSI) for the headless/piped path.
pub fn diff_to_plain(diff: &FileDiff) -> String {
    let mut s = format!("{}\n", diff_header(diff));
    if diff.binary {
        s.push_str("  binary file (not shown)\n");
        return s;
    }
    let td = TextDiff::from_lines(
        diff.old.as_deref().unwrap_or(""),
        diff.new.as_deref().unwrap_or(""),
    );
    for change in td.iter_all_changes() {
        let sym = match change.tag() {
            ChangeTag::Delete => "-",
            ChangeTag::Insert => "+",
            ChangeTag::Equal => " ",
        };
        s.push_str(&format!("{sym} {}", change.value()));
    }
    if !s.ends_with('\n') {
        s.push('\n');
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::color;
    use ratatui::style::Color;

    fn text_of(lines: &[Line]) -> String {
        lines
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn modified(old: &str, new: &str) -> FileDiff {
        FileDiff {
            path: "src/a.rs".into(),
            kind: DiffKind::Modified,
            old: Some(old.into()),
            new: Some(new.into()),
            lang: Some("rust".into()),
            binary: false,
        }
    }

    #[test]
    fn diff_context_lines_are_readable_not_all_dim() {
        // Context rows keep their syntax-highlighted body; only the gutter is faint.
        let lines = diff_to_lines(&modified(
            "fn context_anchor() {}\nlet x = 1;\n",
            "fn context_anchor() {}\nlet x = 2;\n",
        ));
        let ctx = lines
            .iter()
            .find(|l| l.spans.iter().any(|s| s.content.contains("context_anchor")))
            .expect("context line present");
        let body_readable = ctx
            .spans
            .iter()
            .skip(2)
            .any(|s| !s.content.trim().is_empty() && s.style.fg != Some(color::fg_muted()));
        assert!(body_readable, "context body should be readable: {ctx:?}");
    }

    #[test]
    fn diff_modified_has_line_numbers_sigils_and_a_hunk_separator() {
        let t = text_of(&diff_to_lines(&modified("let x = 1;\n", "let x = 2;\n")));
        assert!(t.contains("@@ -1,1 +1,1 @@"), "hunk separator: {t:?}");
        assert!(t.contains("┄┄"), "separator is a faint rule: {t:?}");
        assert!(
            t.contains("1 − ") && t.contains("1 + "),
            "numbered sigils: {t:?}"
        );
        assert!(
            t.contains("x = 1") && t.contains("x = 2"),
            "both versions: {t:?}"
        );
    }

    #[test]
    fn diff_created_file_is_all_additions_without_a_separator() {
        let diff = FileDiff {
            path: "new.txt".into(),
            kind: DiffKind::Created,
            old: None,
            new: Some("hello\nworld\n".into()),
            lang: None,
            binary: false,
        };
        let t = text_of(&diff_to_lines(&diff));
        assert!(t.contains("1 + hello") && t.contains("2 + world"), "{t:?}");
        assert!(
            !t.contains("@@"),
            "a new file has a single implied hunk: {t:?}"
        );
        assert_eq!(diff_stat(&diff), (2, 0));
    }

    #[test]
    fn diff_binary_shows_summary_only() {
        let diff = FileDiff {
            path: "img.png".into(),
            kind: DiffKind::Modified,
            old: Some("ab".into()),
            new: Some("abcd".into()),
            lang: None,
            binary: true,
        };
        let t = text_of(&diff_to_lines(&diff));
        assert!(t.contains("binary file"), "{t:?}");
        assert!(!t.contains("@@"), "no textual diff for binary");
    }

    #[test]
    fn rows_are_capped_and_the_rest_counted() {
        let old = (1..=30).map(|i| format!("line {i}\n")).collect::<String>();
        let new = (1..=30)
            .map(|i| format!("line {i} changed\n"))
            .collect::<String>();
        let r = diff_rows(&modified(&old, &new), 8);
        let body = r
            .rows
            .iter()
            .filter(|l| l.style != Style::default())
            .count();
        assert!(r.hidden > 0, "the tail did not fit");
        assert!(body <= 8, "at most 8 body rows: {body}");
        assert_eq!(diff_stat(&modified(&old, &new)), (30, 30));
    }

    #[test]
    fn a_rewritten_line_emphasises_only_the_changed_words() {
        crate::theme::scoped(
            crate::theme::Theme::new(
                crate::theme::ColorTier::TrueColor,
                crate::theme::Mode::Dark,
                crate::theme::GlyphTier::Unicode,
                crate::theme::Motion::Full,
            ),
            || {
                let r = diff_rows(
                    &modified("let r = send(req)?;\n", "let r = retry(send(req))?;\n"),
                    8,
                );
                let word_bg = |add: bool| {
                    let want = if add {
                        t().palette.diff_add_word.bg
                    } else {
                        t().palette.diff_del_word.bg
                    };
                    r.rows
                        .iter()
                        .flat_map(|l| l.spans.iter())
                        .filter(|s| s.style.bg == Some(want) && want != Color::Reset)
                        .count()
                };
                assert!(word_bg(true) > 0, "inserted words are emphasised: {r:?}");
                assert!(word_bg(false) > 0, "removed words are emphasised: {r:?}");
            },
        );
    }

    #[test]
    fn diff_to_plain_is_ansi_free_unified_text() {
        let diff = FileDiff {
            path: "a.txt".into(),
            kind: DiffKind::Modified,
            old: Some("one\ntwo\n".into()),
            new: Some("one\nTWO\n".into()),
            lang: None,
            binary: false,
        };
        let s = diff_to_plain(&diff);
        assert!(
            s.contains("- two") && s.contains("+ TWO"),
            "plain diff: {s:?}"
        );
        assert!(!s.contains('\u{1b}'), "no ANSI escapes");
    }
}
