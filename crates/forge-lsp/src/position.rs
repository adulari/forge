//! Mapping between what the model writes (a 1-based line and an identifier) and what a language
//! server wants (a 0-based line and a UTF-16 column), plus URI to path conversion.

use std::path::PathBuf;

/// How far from the requested line a symbol is searched for before giving up.
const LINE_SLACK: u32 = 3;

#[derive(Debug, PartialEq, Eq)]
pub struct Located {
    pub line0: u32,
    pub col16: u32,
    /// The 1-based line the symbol was actually found on, when it differs from the one asked for.
    pub moved_to: Option<u32>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum LocateError {
    LineOutOfRange { total: usize },
    NotFound { line: String },
}

impl std::fmt::Display for LocateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LineOutOfRange { total } => {
                write!(f, "line is past the end of the file ({total} lines)")
            }
            Self::NotFound { line } => write!(f, "(line is: \"{}\")", line.trim()),
        }
    }
}

/// UTF-16 code units in `line[..byte_idx]`.
pub fn utf16_col(line: &str, byte_idx: usize) -> u32 {
    line[..byte_idx].encode_utf16().count() as u32
}

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Byte offsets of whole-word occurrences of `ident` in `line`.
fn word_matches(line: &str, ident: &str) -> Vec<usize> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(found) = line[from..].find(ident) {
        let start = from + found;
        let end = start + ident.len();
        let before_ok = line[..start]
            .chars()
            .next_back()
            .is_none_or(|c| !is_ident_char(c));
        let after_ok = line[end..].chars().next().is_none_or(|c| !is_ident_char(c));
        if before_ok && after_ok {
            out.push(start);
        }
        from = start + line[start..].chars().next().map_or(1, char::len_utf8);
    }
    out
}

/// Find the `occurrence`-th (1-based) whole-word `ident` on `line1`, else on the nearest line
/// within [`LINE_SLACK`]. A dotted `ident` (`json.dumps`) lands on its last segment.
pub fn locate_symbol(
    text: &str,
    line1: u32,
    ident: &str,
    occurrence: usize,
) -> Result<Located, LocateError> {
    let lines: Vec<&str> = text.lines().collect();
    if line1 == 0 || line1 as usize > lines.len() {
        return Err(LocateError::LineOutOfRange { total: lines.len() });
    }
    let ident = ident.trim();
    let nth = occurrence.max(1) - 1;
    let mut order = vec![line1];
    for d in 1..=LINE_SLACK {
        order.push(line1.saturating_sub(d));
        order.push(line1 + d);
    }
    for candidate in order {
        let Some(line) = candidate.checked_sub(1).and_then(|i| lines.get(i as usize)) else {
            continue;
        };
        if ident.is_empty() {
            break;
        }
        if let Some(&start) = word_matches(line, ident).get(nth) {
            let last_segment = ident.rfind('.').map_or(0, |dot| dot + 1);
            return Ok(Located {
                line0: candidate - 1,
                col16: utf16_col(line, start + last_segment),
                moved_to: (candidate != line1).then_some(candidate),
            });
        }
    }
    Err(LocateError::NotFound {
        line: lines[line1 as usize - 1].to_string(),
    })
}

/// The filesystem path of a `file://` URI; `None` for any other scheme (a bundled typeshed can
/// come back as `ty-builtins://...`), which has no file to read.
pub fn uri_to_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    let mut bytes = Vec::with_capacity(rest.len());
    let raw = rest.as_bytes();
    let mut i = 0;
    while i < raw.len() {
        if raw[i] == b'%' && i + 2 < raw.len() {
            if let Some(byte) = std::str::from_utf8(&raw[i + 1..i + 3])
                .ok()
                .and_then(|hex| u8::from_str_radix(hex, 16).ok())
            {
                bytes.push(byte);
                i += 3;
                continue;
            }
        }
        bytes.push(raw[i]);
        i += 1;
    }
    let decoded = String::from_utf8(bytes).ok()?;
    let b = decoded.as_bytes();
    let drive = b.len() >= 3 && b[0] == b'/' && b[1].is_ascii_alphabetic() && b[2] == b':';
    Some(PathBuf::from(if drive {
        &decoded[1..]
    } else {
        &decoded[..]
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16_column_counts_surrogate_pairs() {
        let line = "x = '日本' + '😀'; target(1)";
        let byte = line.find("target").unwrap();
        // 日本 are one unit each, the emoji is two.
        assert_eq!(
            utf16_col(line, byte),
            "x = '日本' + '😀'; ".encode_utf16().count() as u32
        );
        let located = locate_symbol(line, 1, "target", 1).unwrap();
        assert_eq!(located.col16, utf16_col(line, byte));
    }

    #[test]
    fn matches_whole_words_only() {
        let text = "total = rows + row\n";
        let located = locate_symbol(text, 1, "row", 1).unwrap();
        assert_eq!(located.col16, 15);
    }

    #[test]
    fn occurrence_selects_the_nth_match() {
        let text = "f(a, a, a)\n";
        assert_eq!(locate_symbol(text, 1, "a", 1).unwrap().col16, 2);
        assert_eq!(locate_symbol(text, 1, "a", 3).unwrap().col16, 8);
        assert!(locate_symbol(text, 1, "a", 4).is_err());
    }

    #[test]
    fn searches_nearby_lines_and_reports_the_move() {
        let text = "a\nb\nc\ncompute()\ne\n";
        let located = locate_symbol(text, 2, "compute", 1).unwrap();
        assert_eq!((located.line0, located.moved_to), (3, Some(4)));
        let err = locate_symbol("a\n\n\n\n\n\n\ncompute()\n", 1, "compute", 1).unwrap_err();
        assert_eq!(err, LocateError::NotFound { line: "a".into() });
        assert!(err.to_string().contains("line is: \"a\""));
    }

    #[test]
    fn dotted_symbol_lands_on_its_last_segment() {
        let located = locate_symbol("json.dumps({})\n", 1, "json.dumps", 1).unwrap();
        assert_eq!(located.col16, 5);
    }

    #[test]
    fn out_of_range_line_is_reported() {
        assert_eq!(
            locate_symbol("a\n", 9, "a", 1),
            Err(LocateError::LineOutOfRange { total: 1 })
        );
    }

    #[test]
    fn uri_to_path_decodes_and_rejects_foreign_schemes() {
        assert_eq!(
            uri_to_path("file:///a/b%20c/d%C3%A9.py"),
            Some(PathBuf::from("/a/b c/dé.py"))
        );
        assert_eq!(
            uri_to_path("file:///C:/x/y.py"),
            Some(PathBuf::from("C:/x/y.py"))
        );
        assert_eq!(uri_to_path("ty-builtins:///builtins.pyi"), None);
    }
}
