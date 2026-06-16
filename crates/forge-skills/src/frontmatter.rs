//! A small, lenient YAML-frontmatter reader — just enough for command/skill metadata, with no
//! external YAML dependency. Supports `key: value`, inline lists `[a, b]`, and block lists
//! (`- item` lines). Unknown keys are kept; a line with no `:` (and not a list item) is an
//! error so the caller can skip a genuinely-malformed file.

use std::collections::BTreeMap;

#[derive(Debug, Clone)]
enum FmValue {
    Scalar(String),
    List(Vec<String>),
}

#[derive(Debug, Clone, Default)]
pub struct Frontmatter {
    map: BTreeMap<String, FmValue>,
}

impl Frontmatter {
    /// A scalar value for `key` (None if absent, empty, or a list).
    pub fn scalar(&self, key: &str) -> Option<String> {
        match self.map.get(key) {
            Some(FmValue::Scalar(s)) if !s.is_empty() => Some(s.clone()),
            _ => None,
        }
    }

    /// A list value for `key`. A non-empty scalar is promoted to a one-element list; absent or
    /// empty keys yield an empty list.
    pub fn list(&self, key: &str) -> Vec<String> {
        match self.map.get(key) {
            Some(FmValue::List(v)) => v.clone(),
            Some(FmValue::Scalar(s)) if !s.is_empty() => vec![s.clone()],
            _ => Vec::new(),
        }
    }
}

/// Split a file into its `---`-fenced frontmatter (if any) and the body. A file without a valid
/// opening+closing fence yields `(None, whole_file)`.
pub fn split(raw: &str) -> (Option<&str>, &str) {
    let s = raw.strip_prefix('\u{feff}').unwrap_or(raw);
    let lead = s.len() - s.trim_start_matches(['\n', '\r', ' ', '\t']).len();
    let rest = &s[lead..];
    let first_line_len = rest.find('\n').map(|i| i + 1).unwrap_or(rest.len());
    if rest[..first_line_len].trim_end() != "---" {
        return (None, raw);
    }
    let fm_start = lead + first_line_len;
    let after = &s[fm_start..];
    let mut off = 0;
    for line in after.split_inclusive('\n') {
        if line.trim_end() == "---" {
            let fm = &s[fm_start..fm_start + off];
            let body_start = fm_start + off + line.len();
            let body = s.get(body_start..).unwrap_or("");
            return (Some(fm), body);
        }
        off += line.len();
    }
    (None, raw) // no closing fence → treat the whole file as body (lenient)
}

/// Parse a frontmatter block. Handles `key: value`, inline `[a, b]` and block (`- item`) lists,
/// `>`/`|` block scalars, and **indented continuation lines** (a folded multi-line value — common
/// in real Claude-Code skill `description:` fields). Only a non-indented line with no `:` is a
/// genuine error, so the caller skips the file and warns.
pub fn parse(text: &str) -> Result<Frontmatter, String> {
    let mut map: BTreeMap<String, FmValue> = BTreeMap::new();
    let mut last_key: Option<String> = None;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let indented = line.starts_with([' ', '\t']);

        // A `- item` line extends the current key's list (block-list syntax), but only when that
        // key is actually awaiting a list (an empty scalar or an existing list) — so a folded
        // description that happens to start with "- " isn't misread.
        if let Some(item) = trimmed.strip_prefix('-').map(str::trim) {
            if let Some(k) = last_key.clone() {
                let slot = map.entry(k).or_insert_with(|| FmValue::List(Vec::new()));
                match slot {
                    FmValue::List(v) => {
                        v.push(strip_quotes(item).to_string());
                        continue;
                    }
                    FmValue::Scalar(s) if s.is_empty() => {
                        *slot = FmValue::List(vec![strip_quotes(item).to_string()]);
                        continue;
                    }
                    _ => {} // fall through: treat as a continuation/new key below
                }
            }
        }

        if indented {
            // Continuation of the previous scalar (YAML folded/block multi-line value).
            if let Some(FmValue::Scalar(s)) = last_key.as_ref().and_then(|k| map.get_mut(k)) {
                if !s.is_empty() {
                    s.push(' ');
                }
                s.push_str(trimmed);
            }
            continue; // an indented line with no scalar context is ignored leniently
        }

        let (key, val) = match line.split_once(':') {
            Some((k, v)) => (k.trim().to_lowercase(), v.trim()),
            None => return Err(format!("malformed frontmatter line: {trimmed:?}")),
        };
        if key.is_empty() {
            return Err("empty frontmatter key".into());
        }
        if val.is_empty() || matches!(val, ">" | "|" | ">-" | "|-") {
            // Awaiting a block list or block scalar on the following (indented) lines.
            map.insert(key.clone(), FmValue::Scalar(String::new()));
        } else if val.starts_with('[') && val.ends_with(']') {
            let inner = &val[1..val.len() - 1];
            let items = inner
                .split(',')
                .map(|s| strip_quotes(s.trim()).to_string())
                .filter(|s| !s.is_empty())
                .collect();
            map.insert(key.clone(), FmValue::List(items));
        } else {
            map.insert(key.clone(), FmValue::Scalar(strip_quotes(val).to_string()));
        }
        last_key = Some(key);
    }
    Ok(Frontmatter { map })
}

fn strip_quotes(s: &str) -> &str {
    let s = s.trim();
    for q in ['"', '\''] {
        if s.len() >= 2 && s.starts_with(q) && s.ends_with(q) {
            return &s[1..s.len() - 1];
        }
    }
    s
}
