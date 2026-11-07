//! A small, strict YAML-subset reader for GitHub workflow files, with no external dependency.
//!
//! Supported: block maps and sequences (a sequence may sit at its key's indent), `- key: v` items,
//! plain / single- / double-quoted scalars (continued over following lines), `|` and `>` block
//! scalars with `-`/`+` chomping, single- or multi-line flow collections `[a, b]` / `{k: v}`, and
//! comments. `${{ ... }}` expressions are ordinary text and never split a key or a flow item.
//! Every scalar is kept as a string (`true`, `10` and `null` are not interpreted).
//!
//! Anything else — anchors, aliases, tags, merge keys, explicit keys, multiple documents, tab
//! indentation, duplicate keys, explicit block-scalar indentation — is an error, so the caller can
//! mark the workflow "unparsed" instead of guessing.

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Yaml {
    Null,
    Str(String),
    Seq(Vec<Yaml>),
    Map(Vec<(String, Yaml)>),
}

impl Yaml {
    pub fn get(&self, key: &str) -> Option<&Yaml> {
        match self {
            Yaml::Map(entries) => entries.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Yaml::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_seq(&self) -> Option<&[Yaml]> {
        match self {
            Yaml::Seq(items) => Some(items),
            _ => None,
        }
    }

    pub fn as_map(&self) -> Option<&[(String, Yaml)]> {
        match self {
            Yaml::Map(entries) => Some(entries),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct YamlError {
    pub line: usize,
    pub msg: String,
}

impl fmt::Display for YamlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.msg)
    }
}

type Res<T> = Result<T, YamlError>;

/// Real workflows nest a handful of levels; the cap keeps hostile input from overflowing the stack.
const MAX_DEPTH: usize = 64;

struct Line {
    no: usize,
    indent: usize,
    text: String,
}

struct Parser {
    lines: Vec<Line>,
    pos: usize,
    depth: usize,
}

pub fn parse(src: &str) -> Res<Yaml> {
    let mut lines = Vec::new();
    let mut seen_content = false;
    for (i, raw) in src.lines().enumerate() {
        let no = i + 1;
        let raw = raw.trim_end_matches('\r');
        let trimmed = raw.trim_start_matches(' ');
        let indent = raw.len() - trimmed.len();
        let blank = trimmed.trim().is_empty() || trimmed.starts_with('#');
        if !blank {
            if trimmed.starts_with('\t') {
                return fail(no, "tab indentation");
            }
            if trimmed == "---" || trimmed.starts_with("--- ") || trimmed == "..." {
                if seen_content {
                    return fail(no, "multiple documents");
                }
                continue;
            }
            seen_content = true;
        }
        lines.push(Line {
            no,
            indent,
            text: trimmed.trim_end().to_string(),
        });
    }
    let mut p = Parser {
        lines,
        pos: 0,
        depth: 0,
    };
    p.skip_insignificant();
    if p.pos >= p.lines.len() {
        return Ok(Yaml::Null);
    }
    let indent = p.lines[p.pos].indent;
    let value = p.parse_block(indent)?;
    p.skip_insignificant();
    if p.pos < p.lines.len() {
        return fail(p.lines[p.pos].no, "unexpected content after the document");
    }
    Ok(value)
}

fn fail<T>(line: usize, msg: &str) -> Res<T> {
    Err(YamlError {
        line,
        msg: msg.to_string(),
    })
}

fn is_dash(text: &str) -> bool {
    text == "-" || text.starts_with("- ")
}

impl Parser {
    fn significant(&self, i: usize) -> bool {
        let t = &self.lines[i].text;
        !t.is_empty() && !t.starts_with('#')
    }

    fn skip_insignificant(&mut self) {
        while self.pos < self.lines.len() && !self.significant(self.pos) {
            self.pos += 1;
        }
    }

    fn parse_block(&mut self, indent: usize) -> Res<Yaml> {
        self.depth += 1;
        let value = self.parse_block_inner(indent);
        self.depth -= 1;
        value
    }

    fn parse_block_inner(&mut self, indent: usize) -> Res<Yaml> {
        let (no, text) = {
            let l = &self.lines[self.pos];
            (l.no, l.text.clone())
        };
        if self.depth > MAX_DEPTH {
            return fail(no, "nesting too deep");
        }
        if is_dash(&text) {
            self.parse_seq(indent)
        } else if split_key(&text, no)?.is_some() {
            self.parse_map(indent)
        } else {
            self.pos += 1;
            self.parse_inline(&text, indent.saturating_sub(1), no)
        }
    }

    fn parse_seq(&mut self, indent: usize) -> Res<Yaml> {
        let mut items = Vec::new();
        loop {
            self.skip_insignificant();
            if self.pos >= self.lines.len() {
                break;
            }
            let (line_indent, no, text) = {
                let l = &self.lines[self.pos];
                (l.indent, l.no, l.text.clone())
            };
            if line_indent < indent || (line_indent == indent && !is_dash(&text)) {
                break;
            }
            if line_indent > indent {
                return fail(no, "unexpected indentation in a sequence");
            }
            let rest = text[1..].trim_start();
            let offset = text.len() - rest.len();
            if rest.is_empty() || rest.starts_with('#') {
                self.pos += 1;
                self.skip_insignificant();
                let nested = self
                    .lines
                    .get(self.pos)
                    .filter(|l| l.indent > indent)
                    .map(|l| l.indent);
                items.push(match nested {
                    Some(n) => self.parse_block(n)?,
                    None => Yaml::Null,
                });
            } else if is_dash(rest) || split_key(rest, no)?.is_some() {
                let l = &mut self.lines[self.pos];
                l.indent = indent + offset;
                l.text = rest.to_string();
                items.push(self.parse_block(indent + offset)?);
            } else {
                self.pos += 1;
                items.push(self.parse_inline(rest, indent, no)?);
            }
        }
        Ok(Yaml::Seq(items))
    }

    fn parse_map(&mut self, indent: usize) -> Res<Yaml> {
        let mut entries: Vec<(String, Yaml)> = Vec::new();
        loop {
            self.skip_insignificant();
            if self.pos >= self.lines.len() {
                break;
            }
            let (line_indent, no, text) = {
                let l = &self.lines[self.pos];
                (l.indent, l.no, l.text.clone())
            };
            if line_indent < indent {
                break;
            }
            if line_indent > indent {
                return fail(no, "unexpected indentation in a mapping");
            }
            let Some((key, rest)) = split_key(&text, no)? else {
                return fail(no, "expected `key: value`");
            };
            if key == "<<" {
                return fail(no, "merge keys are not supported");
            }
            if entries.iter().any(|(k, _)| *k == key) {
                return fail(no, &format!("duplicate key `{key}`"));
            }
            self.pos += 1;
            let value = if rest.is_empty() || rest.starts_with('#') {
                self.skip_insignificant();
                match self.lines.get(self.pos) {
                    Some(l) if l.indent > indent => {
                        let n = l.indent;
                        self.parse_block(n)?
                    }
                    Some(l) if l.indent == indent && is_dash(&l.text) => self.parse_seq(indent)?,
                    _ => Yaml::Null,
                }
            } else {
                self.parse_inline(&rest, indent, no)?
            };
            entries.push((key, value));
        }
        Ok(Yaml::Map(entries))
    }

    /// A scalar or flow collection that starts on the current (already consumed) line.
    fn parse_inline(&mut self, text: &str, parent_indent: usize, no: usize) -> Res<Yaml> {
        let first = text.chars().next().unwrap_or(' ');
        match first {
            '|' | '>' => self.block_scalar(text, parent_indent, no),
            '"' | '\'' => {
                let mut joined = text.to_string();
                loop {
                    if let Some((value, rest)) = read_quoted(&joined, no)? {
                        let rest = rest.trim();
                        if !rest.is_empty() && !rest.starts_with('#') {
                            return fail(no, "text after a quoted string");
                        }
                        return Ok(Yaml::Str(value));
                    }
                    joined = self.append_line(joined, no, "unterminated quoted string")?;
                }
            }
            '[' | '{' => {
                let mut joined = strip_comment(text);
                while !flow_balanced(&joined) {
                    let next =
                        self.append_line(String::new(), no, "unterminated flow collection")?;
                    joined.push(' ');
                    joined.push_str(&strip_comment(&next));
                }
                let mut f = Flow {
                    chars: joined.chars().collect(),
                    i: 0,
                    no,
                    depth: self.depth,
                };
                let value = f.value()?;
                f.ws();
                if f.i < f.chars.len() {
                    return fail(no, "text after a flow collection");
                }
                Ok(value)
            }
            '&' | '*' | '!' | '@' | '`' | '%' => {
                fail(no, "anchors, aliases and tags are not supported")
            }
            _ => {
                let mut value = strip_comment(text);
                while let Some(l) = self.lines.get(self.pos) {
                    if l.text.is_empty() || l.text.starts_with('#') || l.indent <= parent_indent {
                        break;
                    }
                    if split_key(&l.text, l.no)?.is_some() {
                        return fail(l.no, "mapping entry where a scalar continues");
                    }
                    value.push(' ');
                    value.push_str(&strip_comment(&l.text));
                    self.pos += 1;
                }
                if value.is_empty() {
                    Ok(Yaml::Null)
                } else {
                    Ok(Yaml::Str(value))
                }
            }
        }
    }

    /// Consume the next raw line, appending it (space-joined) to `acc`.
    fn append_line(&mut self, mut acc: String, no: usize, why: &str) -> Res<String> {
        let Some(l) = self.lines.get(self.pos) else {
            return fail(no, why);
        };
        if !acc.is_empty() {
            acc.push(' ');
        }
        acc.push_str(l.text.trim());
        self.pos += 1;
        Ok(acc)
    }

    fn block_scalar(&mut self, header: &str, parent_indent: usize, no: usize) -> Res<Yaml> {
        let folded = header.starts_with('>');
        let mut chomp = 'c';
        let mut tail = &header[1..];
        while let Some(c) = tail.chars().next() {
            match c {
                '-' | '+' => chomp = c,
                c if c.is_ascii_digit() => return fail(no, "explicit block indentation"),
                _ => break,
            }
            tail = &tail[1..];
        }
        let tail = tail.trim();
        if !tail.is_empty() && !tail.starts_with('#') {
            return fail(no, "bad block scalar header");
        }
        let mut body: Vec<(usize, &str)> = Vec::new();
        while let Some(l) = self.lines.get(self.pos) {
            let blank = l.text.is_empty();
            if !blank && l.indent <= parent_indent {
                break;
            }
            body.push((l.indent, l.text.as_str()));
            self.pos += 1;
        }
        let Some(block_indent) = body
            .iter()
            .filter(|(_, t)| !t.is_empty())
            .map(|(i, _)| *i)
            .min()
        else {
            return Ok(Yaml::Str(String::new()));
        };
        let mut rows: Vec<String> = body
            .iter()
            .map(|(i, t)| {
                if t.is_empty() {
                    String::new()
                } else {
                    format!("{}{t}", " ".repeat(i - block_indent))
                }
            })
            .collect();
        let mut trailing = 0;
        while rows.last().is_some_and(String::is_empty) {
            rows.pop();
            trailing += 1;
        }
        let mut out = if folded { fold(&rows) } else { rows.join("\n") };
        match chomp {
            '-' => {}
            '+' => out.push_str(&"\n".repeat(1 + trailing)),
            _ => out.push('\n'),
        }
        Ok(Yaml::Str(out))
    }
}

fn fold(rows: &[String]) -> String {
    let mut out = String::new();
    let mut prev_text = false;
    for row in rows {
        if row.is_empty() {
            out.push('\n');
            prev_text = false;
        } else if row.starts_with(' ') {
            out.push_str(&format!("\n{row}"));
            prev_text = false;
        } else {
            if prev_text {
                out.push(' ');
            }
            out.push_str(row);
            prev_text = true;
        }
    }
    out
}

/// Split `key: rest` (key plain or quoted). `None` when the line is not a mapping entry.
fn split_key(text: &str, no: usize) -> Res<Option<(String, String)>> {
    let first = text.chars().next().unwrap_or(' ');
    if first == '?' && text[1..].starts_with(' ') {
        return fail(no, "explicit keys are not supported");
    }
    if first == '"' || first == '\'' {
        let Some((key, rest)) = read_quoted(text, no)? else {
            return Ok(None);
        };
        let rest = rest.trim_start();
        return Ok(match rest.strip_prefix(':') {
            Some(after) if after.is_empty() || after.starts_with(' ') => {
                Some((key, after.trim().to_string()))
            }
            _ => None,
        });
    }
    if matches!(first, '[' | '{' | '|' | '>') {
        return Ok(None);
    }
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut i = 0;
    while i < chars.len() {
        let (at, c) = chars[i];
        if c == '$' && text[at..].starts_with("${{") {
            match text[at..].find("}}") {
                Some(end) => {
                    while i < chars.len() && chars[i].0 < at + end + 2 {
                        i += 1;
                    }
                    continue;
                }
                None => return Ok(None),
            }
        }
        if c == '#' && at > 0 && text[..at].ends_with(' ') {
            return Ok(None);
        }
        if c == ':' {
            let after = &text[at + 1..];
            if after.is_empty() || after.starts_with(' ') {
                let key = text[..at].trim().to_string();
                if key.is_empty() {
                    return Ok(None);
                }
                return Ok(Some((key, after.trim().to_string())));
            }
        }
        i += 1;
    }
    Ok(None)
}

/// Drop a trailing ` # comment` from unquoted text (and trim). A leading `#` is a comment too.
fn strip_comment(text: &str) -> String {
    let t = text.trim();
    if t.starts_with('#') {
        return String::new();
    }
    let bytes = t.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // Byte-level so `i` may sit inside a multibyte char without slicing `t` there.
        if bytes[i..].starts_with(b"${{") {
            match bytes[i..].windows(2).position(|w| w == b"}}") {
                Some(end) => {
                    i += end + 2;
                    continue;
                }
                None => break,
            }
        }
        if bytes[i] == b'#' && i > 0 && bytes[i - 1] == b' ' {
            return t[..i].trim_end().to_string();
        }
        i += 1;
    }
    t.to_string()
}

/// Read a quoted scalar from the start of `text`. `Ok(None)` when the closing quote is missing.
fn read_quoted(text: &str, no: usize) -> Res<Option<(String, &str)>> {
    let mut chars = text.char_indices();
    let (_, quote) = chars.next().expect("caller checked the first char");
    let mut out = String::new();
    while let Some((i, c)) = chars.next() {
        if quote == '\'' {
            if c == '\'' {
                if text[i + 1..].starts_with('\'') {
                    out.push('\'');
                    chars.next();
                    continue;
                }
                return Ok(Some((out, &text[i + 1..])));
            }
            out.push(c);
        } else {
            match c {
                '"' => return Ok(Some((out, &text[i + 1..]))),
                '\\' => match chars.next().map(|(_, e)| e) {
                    Some('n') => out.push('\n'),
                    Some('t') => out.push('\t'),
                    Some('r') => out.push('\r'),
                    Some('0') => out.push('\0'),
                    Some(e @ ('"' | '\\' | '/' | ' ')) => out.push(e),
                    _ => return fail(no, "unsupported escape in a double-quoted string"),
                },
                c => out.push(c),
            }
        }
    }
    Ok(None)
}

fn flow_balanced(text: &str) -> bool {
    let mut depth = 0i32;
    let mut quote: Option<char> = None;
    let mut it = text.chars().peekable();
    while let Some(c) = it.next() {
        match (quote, c) {
            (Some('"'), '\\') => {
                it.next();
            }
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, '[' | '{') => depth += 1,
            (None, ']' | '}') => depth -= 1,
            _ => {}
        }
    }
    depth <= 0 && quote.is_none()
}

struct Flow {
    chars: Vec<char>,
    i: usize,
    no: usize,
    depth: usize,
}

impl Flow {
    fn ws(&mut self) {
        while self.chars.get(self.i).is_some_and(|c| c.is_whitespace()) {
            self.i += 1;
        }
    }

    fn rest(&self) -> String {
        self.chars[self.i..].iter().collect()
    }

    fn value(&mut self) -> Res<Yaml> {
        self.depth += 1;
        let value = self.value_inner();
        self.depth -= 1;
        value
    }

    fn value_inner(&mut self) -> Res<Yaml> {
        if self.depth > MAX_DEPTH {
            return fail(self.no, "nesting too deep");
        }
        self.ws();
        match self.chars.get(self.i) {
            Some('[') => {
                self.i += 1;
                let mut items = Vec::new();
                loop {
                    self.ws();
                    match self.chars.get(self.i) {
                        Some(']') => {
                            self.i += 1;
                            return Ok(Yaml::Seq(items));
                        }
                        Some(_) => {
                            items.push(self.value()?);
                            self.ws();
                            if self.chars.get(self.i) == Some(&',') {
                                self.i += 1;
                            }
                        }
                        None => return fail(self.no, "unterminated flow sequence"),
                    }
                }
            }
            Some('{') => {
                self.i += 1;
                let mut entries: Vec<(String, Yaml)> = Vec::new();
                loop {
                    self.ws();
                    match self.chars.get(self.i) {
                        Some('}') => {
                            self.i += 1;
                            return Ok(Yaml::Map(entries));
                        }
                        Some(_) => {
                            let key = match self.scalar(true)? {
                                Yaml::Str(s) => s,
                                _ => return fail(self.no, "empty key in a flow mapping"),
                            };
                            self.ws();
                            if self.chars.get(self.i) != Some(&':') {
                                return fail(self.no, "expected `:` in a flow mapping");
                            }
                            self.i += 1;
                            if entries.iter().any(|(k, _)| *k == key) {
                                return fail(self.no, &format!("duplicate key `{key}`"));
                            }
                            let v = self.value()?;
                            entries.push((key, v));
                            self.ws();
                            if self.chars.get(self.i) == Some(&',') {
                                self.i += 1;
                            }
                        }
                        None => return fail(self.no, "unterminated flow mapping"),
                    }
                }
            }
            Some(_) => self.scalar(false),
            None => fail(self.no, "missing flow value"),
        }
    }

    fn scalar(&mut self, is_key: bool) -> Res<Yaml> {
        self.ws();
        let c = self.chars.get(self.i).copied();
        if matches!(c, Some('"' | '\'')) {
            let rest = self.rest();
            let Some((value, after)) = read_quoted(&rest, self.no)? else {
                return fail(self.no, "unterminated quoted string");
            };
            self.i += rest.chars().count() - after.chars().count();
            return Ok(Yaml::Str(value));
        }
        if matches!(c, Some('&' | '*' | '!')) {
            return fail(self.no, "anchors, aliases and tags are not supported");
        }
        let start = self.i;
        while let Some(&ch) = self.chars.get(self.i) {
            if ch == '$' && self.chars[self.i..].starts_with(&['$', '{', '{']) {
                let rest = self.rest();
                match rest.find("}}") {
                    Some(end) => {
                        self.i += rest[..end + 2].chars().count();
                        continue;
                    }
                    None => return fail(self.no, "unterminated expression"),
                }
            }
            let stop = matches!(ch, ',' | ']' | '}')
                || (is_key && ch == ':' && self.chars.get(self.i + 1).is_none_or(|n| *n == ' '));
            if stop {
                break;
            }
            self.i += 1;
        }
        let text: String = self.chars[start..self.i].iter().collect();
        let text = text.trim();
        Ok(if text.is_empty() {
            Yaml::Null
        } else {
            Yaml::Str(text.to_string())
        })
    }
}

#[cfg(test)]
mod tests;
