//! Anchored ("hashline") editing, `[harness] edit_mode = "hashline"`.
//!
//! `read_file` prints `[path#TAG]` then `N:text` rows. `edit_file` then takes the tag and line
//! numbers instead of the exact old text: `{path, tag, edits: [{start, end?, new} | {after, new}]}`.
//! The tag is a 16-bit hash of the whole file (trailing blanks ignored), so any change since the
//! read is caught before a stale line number can land on the wrong text.

use forge_types::{DiffKind, FileDiff};
use serde_json::{json, Value};

use super::{check_readable_size, confine, io_with_path, lang_from_path, shown};
use crate::ToolError;

pub(crate) const READ_DESCRIPTION: &str = "Read a UTF-8 text file. Output is `[path#TAG]` then \
    `N:text` rows (N = line number; `N:` and the tag are not part of the file). Slice with \
    `start_line`/`end_line` (1-indexed, inclusive). Very large files return an outline; read the \
    span you need. Pass `paths` (array, entries may be `file:120-180`) to read several files in \
    one call. Always read a file before editing it; edit_file needs the tag.";

pub(crate) const EDIT_DESCRIPTION: &str = "Edit a file by line number, using the tag from \
    read_file's `[path#TAG]` header. `edits` is a list; each is {start, end?, new} to replace \
    lines start..end (end defaults to start; `new` = \"\" deletes them) or {after, new} to insert \
    after line `after` (0 = top of file). Line numbers are those you read; every edit in one call \
    is applied together, so do not adjust numbers for earlier edits. `new` is plain file text, \
    without `N:` prefixes. If the file changed since the tag, nothing is applied and the error \
    shows the current lines with the new tag. Success returns the new tag and the edited lines, \
    so you can keep editing without re-reading.";

pub(crate) fn edit_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "path": { "type": "string", "description": "File to edit." },
            "tag": {
                "type": "string",
                "description": "The 4-character tag from read_file's `[path#TAG]` header."
            },
            "edits": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "start": { "type": "integer", "minimum": 1,
                                   "description": "First line to replace." },
                        "end": { "type": "integer", "minimum": 1,
                                 "description": "Last line to replace (default: start)." },
                        "after": { "type": "integer", "minimum": 0,
                                   "description": "Insert after this line instead (0 = top)." },
                        "new": { "type": "string",
                                 "description": "Replacement or inserted text; empty deletes." }
                    },
                    "required": ["new"]
                }
            }
        },
        "required": ["path", "tag", "edits"]
    })
}

const MAX_SHOWN_CHARS: usize = 1000;
const MAX_RESULT_ROWS: usize = 60;
const STALE_CONTEXT: usize = 3;

struct Line<'a> {
    text: &'a str,
    eol: &'a str,
}

fn split_lines(content: &str) -> Vec<Line<'_>> {
    content
        .split_inclusive('\n')
        .map(|raw| {
            if let Some(text) = raw.strip_suffix("\r\n") {
                Line { text, eol: "\r\n" }
            } else if let Some(text) = raw.strip_suffix('\n') {
                Line { text, eol: "\n" }
            } else {
                Line { text: raw, eol: "" }
            }
        })
        .collect()
}

/// The file's tag: FNV-1a over every line without its trailing blanks, folded to 16 bits.
pub fn file_tag(content: &str) -> String {
    let mut hash: u32 = 0x811c_9dc5;
    for line in split_lines(content) {
        for byte in line
            .text
            .trim_end_matches([' ', '\t', '\r'])
            .bytes()
            .chain(std::iter::once(b'\n'))
        {
            hash ^= u32::from(byte);
            hash = hash.wrapping_mul(0x0100_0193);
        }
    }
    format!("{:04X}", (hash ^ (hash >> 16)) & 0xFFFF)
}

fn clip(text: &str) -> String {
    match text.char_indices().nth(MAX_SHOWN_CHARS) {
        Some((at, _)) => format!("{}…(+{} chars)", &text[..at], text[at..].chars().count()),
        None => text.to_string(),
    }
}

/// Tagged rows for `from..to`. `whole` keeps every line whole (a read, where a line that is
/// cut cannot be edited); otherwise long lines are clipped (error context and edit summaries).
fn rows(lines: &[Line<'_>], from: usize, to: usize, whole: bool) -> String {
    let mut out = String::new();
    for (i, line) in lines.iter().enumerate().take(to).skip(from) {
        let text = if whole {
            line.text.to_string()
        } else {
            clip(line.text)
        };
        out.push_str(&format!("{}:{text}\n", i + 1));
    }
    out
}

/// `[path#TAG]` plus the tagged rows of the 1-indexed inclusive span (the whole file when `None`).
pub(super) fn render(label: &str, content: &str, span: Option<(usize, usize)>) -> String {
    let lines = split_lines(content);
    let (from, to) = match span {
        Some((start, end)) => (start.max(1) - 1, end.min(lines.len())),
        None => (0, lines.len()),
    };
    let mut out = format!("[{label}#{}]\n", file_tag(content));
    out.push_str(&rows(&lines, from.min(to), to, true));
    out.truncate(out.trim_end_matches('\n').len());
    out
}

enum Kind {
    Replace { start: usize, end: usize },
    Insert { after: usize },
}

struct Edit {
    kind: Kind,
    new: Vec<String>,
}

impl Edit {
    fn position(&self) -> usize {
        match self.kind {
            Kind::Replace { start, .. } => start - 1,
            Kind::Insert { after } => after,
        }
    }
}

fn number(item: &Value, key: &str) -> Result<Option<usize>, ToolError> {
    match item.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_u64()
            .map(|n| Some(n as usize))
            .ok_or_else(|| ToolError::BadArgs(format!("`{key}` must be a non-negative integer"))),
    }
}

fn new_lines(text: &str) -> Vec<String> {
    let mut pieces: Vec<&str> = text.split('\n').collect();
    if pieces.last() == Some(&"") {
        pieces.pop();
    }
    pieces
        .into_iter()
        .map(|l| l.strip_suffix('\r').unwrap_or(l).to_string())
        .collect()
}

fn parse(args: &Value, line_count: usize) -> Result<Vec<Edit>, ToolError> {
    let items = args
        .get("edits")
        .and_then(Value::as_array)
        .filter(|a| !a.is_empty())
        .ok_or_else(|| ToolError::BadArgs("`edits` must be a non-empty array".to_string()))?;
    let mut edits = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let n = index + 1;
        let new = item
            .get("new")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::BadArgs(format!("edit #{n}: missing string `new`")))?;
        let start = number(item, "start")?.filter(|s| *s != 0);
        // A model that fills every optional property sends `after: 0` beside `start`.
        let after = number(item, "after")?.filter(|after| start.is_none() || *after != 0);
        let kind = match (start, after) {
            (Some(start), None) => {
                let end = number(item, "end")?.filter(|e| *e != 0).unwrap_or(start);
                if end < start {
                    return Err(ToolError::Failed(format!(
                        "edit #{n}: end {end} is before start {start}"
                    )));
                }
                if end > line_count {
                    return Err(ToolError::Failed(format!(
                        "edit #{n}: line {end} is past the end (the file has {line_count} lines)"
                    )));
                }
                Kind::Replace { start, end }
            }
            (None, Some(after)) => {
                if after > line_count {
                    return Err(ToolError::Failed(format!(
                        "edit #{n}: `after` {after} is past the end (the file has {line_count} \
                         lines)"
                    )));
                }
                Kind::Insert { after }
            }
            _ => {
                return Err(ToolError::BadArgs(format!(
                    "edit #{n}: give either `start` (with optional `end`) or `after`"
                )))
            }
        };
        edits.push(Edit {
            kind,
            new: new_lines(new),
        });
    }
    let mut spans: Vec<(usize, usize, usize)> = Vec::new();
    for (i, edit) in edits.iter().enumerate() {
        if let Kind::Replace { start, end } = edit.kind {
            spans.push((start, end, i + 1));
        }
    }
    spans.sort_unstable();
    for pair in spans.windows(2) {
        if pair[1].0 <= pair[0].1 {
            return Err(ToolError::Failed(format!(
                "edits #{} and #{} overlap on line {}; merge them into one edit",
                pair[0].2, pair[1].2, pair[1].0
            )));
        }
    }
    for (i, edit) in edits.iter().enumerate() {
        if let Kind::Insert { after } = edit.kind {
            if let Some((start, end, other)) = spans
                .iter()
                .copied()
                .find(|(start, end, _)| *start <= after && after < *end)
            {
                return Err(ToolError::Failed(format!(
                    "edit #{} inserts after line {after}, inside the lines {start}-{end} that \
                     edit #{other} replaces; put the new text in that edit",
                    i + 1
                )));
            }
        }
    }
    Ok(edits)
}

fn normalize_tag(raw: &str) -> String {
    raw.trim()
        .trim_matches(|c| c == '[' || c == ']')
        .rsplit('#')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_uppercase()
}

/// The result of applying a call: the new file text and what to tell the model.
pub(super) struct Applied {
    pub(super) updated: String,
    pub(super) summary: String,
}

fn stale(label: &str, given: &str, content: &str, args: &Value) -> ToolError {
    let lines = split_lines(content);
    let anchor = args
        .get("edits")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .and_then(|e| {
            number(e, "start")
                .ok()
                .flatten()
                .or_else(|| number(e, "after").ok().flatten())
        })
        .unwrap_or(1)
        .clamp(1, lines.len().max(1));
    let from = anchor.saturating_sub(STALE_CONTEXT + 1);
    let to = (anchor + STALE_CONTEXT).min(lines.len());
    let tag = file_tag(content);
    let given = if given.is_empty() {
        "none".to_string()
    } else {
        format!("#{given}")
    };
    ToolError::Failed(format!(
        "edit rejected for {label}: it changed since you read it (your tag {given}, now #{tag}). \
         Nothing was applied. Current lines around line {anchor}:\n[{label}#{tag}]\n{}\
         Re-issue the edit with tag #{tag} and the line numbers from this view; read_file the span \
         if you need more.",
        rows(&lines, from, to, false)
    ))
}

pub(super) fn apply(label: &str, content: &str, args: &Value) -> Result<Applied, ToolError> {
    let given = normalize_tag(args.get("tag").and_then(Value::as_str).unwrap_or(""));
    if given != file_tag(content) {
        return Err(stale(label, &given, content, args));
    }
    let lines = split_lines(content);
    let edits = parse(args, lines.len())?;
    let crlf = lines.iter().filter(|l| l.eol == "\r\n").count();
    let lf = lines.iter().filter(|l| l.eol == "\n").count();
    let eol = if crlf > lf { "\r\n" } else { "\n" };

    let mut order: Vec<usize> = (0..edits.len()).collect();
    order.sort_by_key(|&i| {
        let is_replace = matches!(edits[i].kind, Kind::Replace { .. });
        std::cmp::Reverse((edits[i].position(), is_replace, i))
    });

    let mut out: Vec<(String, String)> = lines
        .iter()
        .map(|l| (l.text.to_string(), l.eol.to_string()))
        .collect();
    for &i in &order {
        let edit = &edits[i];
        let (lo, hi, last_eol) = match edit.kind {
            Kind::Replace { start, end } => (start - 1, end, out[end - 1].1.clone()),
            Kind::Insert { after } => {
                let mut tail = eol.to_string();
                if after == out.len() && after > 0 && out[after - 1].1.is_empty() {
                    out[after - 1].1 = eol.to_string();
                    tail = String::new();
                }
                (after, after, tail)
            }
        };
        let count = edit.new.len();
        let block = edit.new.iter().enumerate().map(|(k, text)| {
            let line_eol = if k + 1 == count { &last_eol } else { eol };
            (text.clone(), line_eol.to_string())
        });
        out.splice(lo..hi, block);
    }
    let updated: String = out.iter().map(|(t, e)| format!("{t}{e}")).collect();
    if updated == content {
        return Err(ToolError::Failed(format!(
            "no change: the edits leave {label} as it was"
        )));
    }

    let mut ascending: Vec<usize> = (0..edits.len()).collect();
    ascending.sort_by_key(|&i| {
        (
            edits[i].position(),
            matches!(edits[i].kind, Kind::Replace { .. }),
            i,
        )
    });
    let mut delta: isize = 0;
    let mut windows: Vec<(usize, usize)> = Vec::new();
    for &i in &ascending {
        let edit = &edits[i];
        let removed = match edit.kind {
            Kind::Replace { start, end } => end - start + 1,
            Kind::Insert { .. } => 0,
        };
        let begin = (edit.position() as isize + delta) as usize;
        windows.push((begin.saturating_sub(1), begin + edit.new.len() + 1));
        delta += edit.new.len() as isize - removed as isize;
    }
    windows.sort_unstable();
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for (lo, hi) in windows {
        match merged.last_mut() {
            Some(last) if lo <= last.1 => last.1 = last.1.max(hi),
            _ => merged.push((lo, hi)),
        }
    }
    let new_lines_view = split_lines(&updated);
    let mut body = String::new();
    let mut budget = MAX_RESULT_ROWS;
    for (lo, hi) in merged {
        let hi = hi.min(new_lines_view.len());
        let lo = lo.min(hi);
        if budget == 0 {
            break;
        }
        let take = (hi - lo).min(budget);
        body.push_str(&rows(&new_lines_view, lo, lo + take, false));
        budget -= take;
        if lo + take < hi {
            body.push_str("…\n");
        }
    }
    let n = edits.len();
    let summary = format!(
        "edited {label} ({n} edit{}) [{label}#{}]\n{}",
        if n == 1 { "" } else { "s" },
        file_tag(&updated),
        body.trim_end_matches('\n')
    );
    Ok(Applied { updated, summary })
}

async fn load(path: &str) -> Result<String, ToolError> {
    confine(path)?;
    check_readable_size(path).await?;
    tokio::fs::read_to_string(path)
        .await
        .map_err(|e| io_with_path(e, path))
}

pub(super) async fn run(path: &str, args: &Value) -> Result<String, ToolError> {
    let content = load(path).await?;
    let applied = apply(&shown(path), &content, args)?;
    crate::fs_write::write(path, &applied.updated).await?;
    Ok(applied.summary)
}

pub(super) async fn preview(path: &str, args: &Value) -> Option<FileDiff> {
    let content = load(path).await.ok()?;
    let applied = apply(&shown(path), &content, args).ok()?;
    Some(FileDiff {
        path: path.to_string(),
        kind: DiffKind::Modified,
        old: Some(content),
        new: Some(applied.updated),
        lang: lang_from_path(path),
        binary: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ToolRegistry;
    use forge_config::EditMode;

    fn edit(content: &str, edits: Value) -> Result<Applied, ToolError> {
        let args = json!({"path": "f.txt", "tag": file_tag(content), "edits": edits});
        apply("f.txt", content, &args)
    }

    fn updated(content: &str, edits: Value) -> String {
        edit(content, edits).unwrap().updated
    }

    #[test]
    fn the_tag_ignores_trailing_blanks_and_line_endings_but_not_content() {
        let a = file_tag("one\ntwo\n");
        assert_eq!(a, file_tag("one  \ntwo\t\n"));
        assert_eq!(a, file_tag("one\r\ntwo\r\n"));
        assert_ne!(a, file_tag("one\ntwo!\n"));
        assert_ne!(a, file_tag("one\n\ntwo\n"));
        assert_eq!(a.len(), 4);
    }

    #[test]
    fn render_shows_the_tag_and_numbered_rows() {
        let out = render("src/a.rs", "alpha\nbeta\ngamma\n", None);
        assert_eq!(
            out,
            format!(
                "[src/a.rs#{}]\n1:alpha\n2:beta\n3:gamma",
                file_tag("alpha\nbeta\ngamma\n")
            )
        );
        let ranged = render("src/a.rs", "alpha\nbeta\ngamma\n", Some((2, 3)));
        assert!(ranged.ends_with("\n2:beta\n3:gamma") && !ranged.contains("1:alpha"));
    }

    #[test]
    fn replace_one_line_a_range_and_delete() {
        let src = "a\nb\nc\nd\ne\n";
        assert_eq!(
            updated(src, json!([{"start": 2, "new": "B"}])),
            "a\nB\nc\nd\ne\n"
        );
        assert_eq!(
            updated(src, json!([{"start": 2, "end": 4, "new": "X\nY"}])),
            "a\nX\nY\ne\n"
        );
        assert_eq!(
            updated(src, json!([{"start": 2, "end": 3, "new": ""}])),
            "a\nd\ne\n"
        );
        assert_eq!(
            updated(src, json!([{"start": 5, "new": "E\n"}])),
            "a\nb\nc\nd\nE\n"
        );
    }

    #[test]
    fn insert_after_a_line_at_the_top_and_at_the_end() {
        let src = "a\nb\n";
        assert_eq!(
            updated(src, json!([{"after": 0, "new": "top"}])),
            "top\na\nb\n"
        );
        assert_eq!(
            updated(src, json!([{"after": 1, "new": "mid\nmid2"}])),
            "a\nmid\nmid2\nb\n"
        );
        assert_eq!(
            updated(src, json!([{"after": 2, "new": "end"}])),
            "a\nb\nend\n"
        );
        assert_eq!(
            updated("", json!([{"after": 0, "new": "first"}])),
            "first\n"
        );
    }

    #[test]
    fn several_edits_use_the_original_numbers_whatever_their_order() {
        let src = "1\n2\n3\n4\n5\n6\n";
        let edits = json!([
            {"start": 2, "new": "two\ntwo-b"},
            {"start": 5, "end": 6, "new": "tail"},
            {"after": 3, "new": "after-three"},
            {"after": 0, "new": "head"}
        ]);
        let expected = "head\n1\ntwo\ntwo-b\n3\nafter-three\n4\ntail\n";
        assert_eq!(updated(src, edits.clone()), expected);
        let mut reversed = edits.as_array().unwrap().clone();
        reversed.reverse();
        assert_eq!(updated(src, Value::Array(reversed)), expected);
    }

    #[test]
    fn an_insert_next_to_a_replace_goes_on_the_right_side_of_it() {
        let src = "a\nb\nc\n";
        assert_eq!(
            updated(
                src,
                json!([{"start": 2, "new": "B"}, {"after": 1, "new": "before-B"}])
            ),
            "a\nbefore-B\nB\nc\n"
        );
        assert_eq!(
            updated(
                src,
                json!([{"start": 2, "new": "B"}, {"after": 2, "new": "after-B"}])
            ),
            "a\nB\nafter-B\nc\n"
        );
        assert_eq!(
            updated(
                src,
                json!([{"after": 1, "new": "one"}, {"after": 1, "new": "two"}])
            ),
            "a\none\ntwo\nb\nc\n"
        );
    }

    #[test]
    fn overlapping_or_out_of_range_edits_name_the_problem() {
        let src = "a\nb\nc\n";
        let overlap = |e| edit(src, e).err().unwrap().to_string();
        assert!(
            overlap(json!([{"start": 1, "end": 2, "new": "x"}, {"start": 2, "new": "y"}]))
                .contains("overlap")
        );
        assert!(
            overlap(json!([{"start": 1, "end": 3, "new": "x"}, {"after": 2, "new": "y"}]))
                .contains("inside the lines 1-3")
        );
        assert!(overlap(json!([{"start": 4, "new": "x"}])).contains("past the end (the file has 3"));
        assert!(overlap(json!([{"after": 4, "new": "x"}])).contains("past the end"));
        assert!(overlap(json!([{"start": 3, "end": 2, "new": "x"}])).contains("before start"));
        assert!(overlap(json!([{"new": "x"}])).contains("either `start`"));
        assert!(overlap(json!([{"start": 1, "after": 1, "new": "x"}])).contains("either `start`"));
        assert!(overlap(json!([])).contains("non-empty"));
    }

    #[test]
    fn a_stale_tag_applies_nothing_and_shows_the_current_lines() {
        let before = "a\nb\nc\nd\ne\nf\ng\nh\n";
        let tag = file_tag(before);
        let after = "a\nb\nc\nINSERTED\nd\ne\nf\ng\nh\n";
        let args = json!({"path": "f.txt", "tag": tag, "edits": [{"start": 5, "new": "E"}]});
        let msg = apply("f.txt", after, &args).err().unwrap().to_string();
        assert!(msg.contains(&format!("your tag #{tag}")), "{msg}");
        assert!(msg.contains(&format!("now #{}", file_tag(after))), "{msg}");
        assert!(msg.contains("Nothing was applied"), "{msg}");
        assert!(
            msg.contains("4:INSERTED") && msg.contains("5:d") && msg.contains("2:b"),
            "{msg}"
        );
        assert!(!msg.contains("1:a"), "context is bounded: {msg}");
    }

    #[test]
    fn a_missing_or_decorated_tag_is_handled() {
        let src = "a\nb\n";
        let none = json!({"path": "f.txt", "edits": [{"start": 1, "new": "x"}]});
        assert!(apply("f.txt", src, &none)
            .err()
            .unwrap()
            .to_string()
            .contains("tag none"));
        let decorated = json!({"path": "f.txt", "tag": format!("[f.txt#{}]", file_tag(src).to_lowercase()),
            "edits": [{"start": 1, "new": "x"}]});
        assert_eq!(apply("f.txt", src, &decorated).unwrap().updated, "x\nb\n");
    }

    #[test]
    fn an_edit_that_changes_nothing_is_an_error() {
        let err = edit("a\nb\n", json!([{"start": 1, "new": "a"}]))
            .err()
            .unwrap();
        assert!(err.to_string().contains("no change"));
    }

    #[test]
    fn crlf_files_stay_crlf() {
        let src = "a\r\nb\r\nc\r\n";
        assert_eq!(
            updated(
                src,
                json!([{"start": 2, "new": "B1\nB2"}, {"after": 3, "new": "d"}])
            ),
            "a\r\nB1\r\nB2\r\nc\r\nd\r\n"
        );
        assert_eq!(
            updated(src, json!([{"start": 1, "new": "A\r\n"}])),
            "A\r\nb\r\nc\r\n"
        );
    }

    #[test]
    fn mixed_endings_keep_untouched_lines_as_they_were() {
        let src = "a\r\nb\nc\r\n";
        assert_eq!(
            updated(src, json!([{"start": 2, "new": "B"}])),
            "a\r\nB\nc\r\n"
        );
    }

    #[test]
    fn a_missing_final_newline_is_kept_and_an_append_adds_the_break() {
        assert_eq!(updated("a\nb", json!([{"start": 2, "new": "B"}])), "a\nB");
        assert_eq!(
            updated("a\nb", json!([{"start": 2, "new": "B\nC"}])),
            "a\nB\nC"
        );
        assert_eq!(
            updated("a\nb", json!([{"after": 2, "new": "c"}])),
            "a\nb\nc"
        );
        assert_eq!(
            updated("a\nb\n", json!([{"start": 1, "new": "A"}])),
            "A\nb\n"
        );
        assert_eq!(updated("one", json!([{"start": 1, "new": "uno\n"}])), "uno");
    }

    #[test]
    fn blank_lines_can_be_inserted_and_replaced() {
        assert_eq!(
            updated("a\nb\n", json!([{"after": 1, "new": "\n"}])),
            "a\n\nb\n"
        );
        assert_eq!(
            updated("a\n\nb\n", json!([{"start": 2, "new": "x"}])),
            "a\nx\nb\n"
        );
    }

    #[test]
    fn unicode_is_edited_and_tagged_by_line() {
        let src = "héllo wörld\n日本語のテキスト\n🦀 rust\n";
        let out = updated(src, json!([{"start": 2, "new": "日本語 edited 🎉"}]));
        assert_eq!(out, "héllo wörld\n日本語 edited 🎉\n🦀 rust\n");
        assert_ne!(file_tag(src), file_tag(&out));
        assert!(render("u.txt", src, None).contains("3:🦀 rust"));
    }

    #[test]
    fn a_very_long_line_is_read_whole_so_it_can_be_edited_whole() {
        let long = "x".repeat(5000);
        let src = format!("short\n{long}\nend\n");
        let shown_text = render("big.txt", &src, None);
        assert!(shown_text.contains(&format!("2:{long}")) && !shown_text.contains('…'));
        let out = updated(&src, json!([{"start": 1, "new": "SHORT"}]));
        assert!(
            out.contains(&long),
            "an untouched long line survives byte for byte"
        );
        let applied = edit(&src, json!([{"start": 3, "new": "END"}])).unwrap();
        assert!(
            applied.summary.contains("…(+4000 chars)"),
            "summaries stay short"
        );
    }

    #[test]
    fn placeholder_after_zero_beside_start_does_not_reject_a_replace() {
        let src = "a\nb\n";
        assert_eq!(
            updated(src, json!([{"start": 2, "after": 0, "new": "B"}])),
            "a\nB\n"
        );
        assert_eq!(
            updated(src, json!([{"after": 0, "new": "top"}])),
            "top\na\nb\n"
        );
        let both = edit(src, json!([{"start": 2, "after": 1, "new": "x"}]))
            .err()
            .unwrap();
        assert!(both.to_string().contains("either `start`"));
    }

    #[test]
    fn the_success_summary_carries_the_new_tag_and_the_edited_rows() {
        let src = (1..=20).map(|n| format!("line{n}\n")).collect::<String>();
        let applied = edit(&src, json!([{"start": 10, "new": "TEN\nTEN-B"}])).unwrap();
        let tag = file_tag(&applied.updated);
        assert!(applied
            .summary
            .starts_with(&format!("edited f.txt (1 edit) [f.txt#{tag}]\n")));
        assert!(
            applied.summary.contains("10:TEN\n11:TEN-B"),
            "{}",
            applied.summary
        );
        assert!(applied.summary.contains("9:line9") && applied.summary.contains("12:line11"));
        assert!(!applied.summary.contains("line3"));
        // The summary's numbers are the NEW file's, so a follow-up edit needs no re-read.
        let next = json!({"path": "f.txt", "tag": tag, "edits": [{"start": 11, "new": "again"}]});
        assert!(apply("f.txt", &applied.updated, &next)
            .unwrap()
            .updated
            .contains("TEN\nagain\n"));
    }

    #[test]
    fn the_summary_is_bounded() {
        let src = (1..=400).map(|n| format!("l{n}\n")).collect::<String>();
        let edits: Vec<Value> = (0..100)
            .map(|i| json!({"start": 2 + i * 3, "new": "a\nb\nc\nd"}))
            .collect();
        let applied = edit(&src, Value::Array(edits)).unwrap();
        assert!(applied.summary.lines().count() <= MAX_RESULT_ROWS + 3);
    }

    async fn hashline_registry(dir: &std::path::Path) -> ToolRegistry {
        let mut registry = ToolRegistry::with_core_tools_in(dir);
        registry.set_edit_mode(EditMode::Hashline);
        registry
    }

    #[tokio::test]
    async fn the_registry_swaps_read_edit_and_drops_multi_edit_only_in_hashline_mode() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("f.txt");
        std::fs::write(&path, "alpha\nbeta\n").unwrap();
        let p = path.to_str().unwrap();

        let plain = ToolRegistry::with_core_tools_in(dir.path());
        assert!(plain.get("multi_edit").is_some());
        assert_eq!(
            plain
                .get("read_file")
                .unwrap()
                .run(&json!({"path": p}))
                .await
                .unwrap(),
            "alpha\nbeta\n"
        );
        assert!(plain.get("edit_file").unwrap().schema()["properties"]
            .get("old")
            .is_some());

        let mut registry = hashline_registry(dir.path()).await;
        assert!(registry.get("multi_edit").is_none());
        let edit_tool = registry.get("edit_file").unwrap();
        assert!(edit_tool.schema()["properties"].get("edits").is_some());
        assert!(edit_tool.schema()["properties"].get("old").is_none());
        assert!(edit_tool.description().contains("line number"));
        let read = registry
            .get("read_file")
            .unwrap()
            .run(&json!({"path": p}))
            .await
            .unwrap();
        let tag = file_tag("alpha\nbeta\n");
        assert!(
            read.contains(&format!("#{tag}]\n1:alpha\n2:beta")),
            "{read}"
        );
        let batch = registry
            .get("read_file")
            .unwrap()
            .run(&json!({"paths": [format!("{p}:2-2")]}))
            .await
            .unwrap();
        assert!(
            batch.contains("2:beta") && !batch.contains("1:alpha"),
            "{batch}"
        );

        let args = json!({"path": p, "tag": tag, "edits": [{"start": 2, "new": "BETA"}]});
        let diff = registry
            .get("edit_file")
            .unwrap()
            .preview(&args)
            .await
            .unwrap();
        assert_eq!(diff.old.as_deref(), Some("alpha\nbeta\n"));
        assert_eq!(diff.new.as_deref(), Some("alpha\nBETA\n"));
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "alpha\nbeta\n",
            "preview writes nothing"
        );
        let out = registry.get("edit_file").unwrap().run(&args).await.unwrap();
        assert!(out.contains("edited"), "{out}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "alpha\nBETA\n");

        let stale = registry
            .get("edit_file")
            .unwrap()
            .run(&args)
            .await
            .unwrap_err()
            .to_string();
        assert!(stale.contains("changed since you read it"), "{stale}");

        let legacy = json!({"path": p, "old": "alpha", "new": "ALPHA"});
        registry
            .get("edit_file")
            .unwrap()
            .run(&legacy)
            .await
            .unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "ALPHA\nBETA\n");

        registry.set_edit_mode(EditMode::Replace);
        assert!(registry.get("multi_edit").is_some());
        assert_eq!(
            registry
                .get("read_file")
                .unwrap()
                .run(&json!({"path": p}))
                .await
                .unwrap(),
            "ALPHA\nBETA\n"
        );
    }

    #[tokio::test]
    async fn the_tool_refuses_paths_outside_the_workspace_like_edit_file_does() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let victim = outside.path().join("x.txt");
        std::fs::write(&victim, "a\n").unwrap();
        let registry = hashline_registry(dir.path()).await;
        let args = json!({"path": victim.to_str().unwrap(), "tag": file_tag("a\n"), "edits": [{"start": 1, "new": "b"}]});
        let err = registry
            .get("edit_file")
            .unwrap()
            .run(&args)
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("outside the workspace"), "{err}");
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "a\n");
    }
}
