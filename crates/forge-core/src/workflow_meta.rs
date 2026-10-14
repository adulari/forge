//! `meta` header handling for workflow scripts. Models (and saved library scripts) commonly open
//! with `export const meta = { name, description, phases: [...] }` — the Claude Code workflow
//! convention. The engine has no module system, so the `export` keyword is dropped (the
//! `const meta` binding stays, so the body may still read it), and the literal is parsed — best
//! effort, as pure data — for display. A meta block that isn't a pure literal still runs; it just
//! yields no display info.

#[derive(Debug, Clone, Default, PartialEq)]
pub struct WorkflowMeta {
    pub name: Option<String>,
    pub description: Option<String>,
    pub phases: Vec<String>,
}

impl WorkflowMeta {
    /// One transcript line announcing the workflow, or `None` when meta carried nothing.
    pub fn banner(&self) -> Option<String> {
        let mut out = String::new();
        if let Some(name) = &self.name {
            out.push_str(name);
        }
        if let Some(desc) = &self.description {
            if !out.is_empty() {
                out.push_str(" — ");
            }
            out.push_str(desc);
        }
        if !self.phases.is_empty() {
            if !out.is_empty() {
                out.push_str(" · ");
            }
            out.push_str("phases: ");
            out.push_str(&self.phases.join(" → "));
        }
        (!out.is_empty()).then_some(out)
    }
}

/// Appended to engine parse errors that mention module syntax, so the retry uses the supported
/// shape instead of guessing.
pub const SYNTAX_HINT: &str = "hint: a workflow script is a plain sequence of statements run \
inside an async function — no `import`/`export`/`require` and no top-level function wrapper. \
Supported header: an optional leading `export const meta = { name, description, phases: \
[{ title }] };` (a pure literal; `const meta = {...}` also works). Everything else is ordinary \
JS using agent/parallel/pipeline/phase/log/workflow; end with `return <result>`.";

pub fn is_module_syntax_error(err: &str) -> bool {
    let e = err.to_lowercase();
    e.contains("export") || e.contains("import")
}

/// Splits a leading `[export] const|let|var meta = {…};` off `script`: returns the executable
/// body (declaration kept, minus `export`) and the parsed meta if it was a pure literal. Any
/// other `export` form is left untouched for the engine to reject.
pub fn prepare(script: &str) -> (String, Option<WorkflowMeta>) {
    let start = skip_trivia(script, 0);
    let rest = &script[start..];
    let (exported, after_kw) = match strip_word(rest, "export") {
        Some(r) => (true, r),
        None => (false, rest),
    };
    let decl = ["const", "let", "var"]
        .iter()
        .find_map(|kw| strip_word(after_kw.trim_start(), kw));
    let Some(after_decl) = decl else {
        return (script.to_string(), None);
    };
    let Some(after_name) = strip_word(after_decl.trim_start(), "meta") else {
        return (script.to_string(), None);
    };
    let Some(after_eq) = after_name.trim_start().strip_prefix('=') else {
        return (script.to_string(), None);
    };
    let lit_start = script.len() - after_eq.trim_start().len();
    if !script[lit_start..].starts_with('{') {
        return (script.to_string(), None);
    }
    let Some(lit_end) = balanced_end(script, lit_start) else {
        return (script.to_string(), None);
    };
    let meta = parse_literal(&script[lit_start..lit_end]).map(|v| meta_from_json(&v));
    let body = if exported {
        format!("{}{}", &script[..start], after_kw.trim_start())
    } else {
        script.to_string()
    };
    (body, meta.flatten())
}

fn strip_word<'a>(s: &'a str, word: &str) -> Option<&'a str> {
    let rest = s.strip_prefix(word)?;
    match rest.chars().next() {
        Some(c) if c.is_alphanumeric() || c == '_' || c == '$' => None,
        _ => Some(rest),
    }
}

fn skip_trivia(s: &str, mut i: usize) -> usize {
    loop {
        let rest = &s[i..];
        let trimmed = rest.trim_start();
        i += rest.len() - trimmed.len();
        if trimmed.starts_with("//") {
            i += trimmed.find('\n').map_or(trimmed.len(), |n| n + 1);
        } else if trimmed.starts_with("/*") {
            i += trimmed.find("*/").map_or(trimmed.len(), |n| n + 2);
        } else {
            return i;
        }
    }
}

/// Byte offset just past the `}` matching the `{` at `open`, consuming a trailing `;`.
fn balanced_end(s: &str, open: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut chars = s[open..].char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        match c {
            '\'' | '"' | '`' => {
                let mut escaped = false;
                for (_, d) in chars.by_ref() {
                    if escaped {
                        escaped = false;
                    } else if d == '\\' {
                        escaped = true;
                    } else if d == c {
                        break;
                    }
                }
            }
            '/' if matches!(chars.peek(), Some((_, '/'))) => {
                for (_, d) in chars.by_ref() {
                    if d == '\n' {
                        break;
                    }
                }
            }
            '/' if matches!(chars.peek(), Some((_, '*'))) => {
                let mut prev = ' ';
                for (_, d) in chars.by_ref() {
                    if prev == '*' && d == '/' {
                        break;
                    }
                    prev = d;
                }
            }
            '{' => depth += 1,
            '}' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(open + i + 1);
                }
            }
            _ => {}
        }
    }
    None
}

fn meta_from_json(v: &serde_json::Value) -> Option<WorkflowMeta> {
    let obj = v.as_object()?;
    let text = |k: &str| {
        obj.get(k)
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    let phases = obj
        .get("phases")
        .and_then(|p| p.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|p| match p {
                    serde_json::Value::String(s) => Some(s.clone()),
                    serde_json::Value::Object(o) => o
                        .get("title")
                        .or_else(|| o.get("name"))
                        .and_then(|t| t.as_str())
                        .map(str::to_string),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();
    Some(WorkflowMeta {
        name: text("name"),
        description: text("description"),
        phases,
    })
}

/// Parses a JS object/array literal (unquoted keys, single/double/backtick-without-`${` strings,
/// trailing commas, comments) into JSON. `None` for anything not pure data.
fn parse_literal(src: &str) -> Option<serde_json::Value> {
    let mut p = Lit {
        s: src.as_bytes(),
        i: 0,
    };
    let v = p.value(0)?;
    p.ws();
    (p.i == p.s.len()).then_some(v)
}

struct Lit<'a> {
    s: &'a [u8],
    i: usize,
}

impl Lit<'_> {
    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }

    fn ws(&mut self) {
        loop {
            match self.peek() {
                Some(b' ' | b'\t' | b'\r' | b'\n') => self.i += 1,
                Some(b'/') if self.s.get(self.i + 1) == Some(&b'/') => {
                    while !matches!(self.peek(), None | Some(b'\n')) {
                        self.i += 1;
                    }
                }
                Some(b'/') if self.s.get(self.i + 1) == Some(&b'*') => {
                    self.i += 2;
                    while self.peek().is_some() && !self.s[self.i..].starts_with(b"*/") {
                        self.i += 1;
                    }
                    self.i = (self.i + 2).min(self.s.len());
                }
                _ => return,
            }
        }
    }

    fn value(&mut self, depth: usize) -> Option<serde_json::Value> {
        if depth > 16 {
            return None;
        }
        self.ws();
        match self.peek()? {
            b'{' => {
                self.i += 1;
                let mut map = serde_json::Map::new();
                loop {
                    self.ws();
                    if self.peek()? == b'}' {
                        self.i += 1;
                        return Some(map.into());
                    }
                    let key = match self.peek()? {
                        b'\'' | b'"' => self.string()?,
                        _ => self.ident()?,
                    };
                    self.ws();
                    if self.peek()? != b':' {
                        return None;
                    }
                    self.i += 1;
                    map.insert(key, self.value(depth + 1)?);
                    self.ws();
                    match self.peek()? {
                        b',' => self.i += 1,
                        b'}' => {}
                        _ => return None,
                    }
                }
            }
            b'[' => {
                self.i += 1;
                let mut items = Vec::new();
                loop {
                    self.ws();
                    if self.peek()? == b']' {
                        self.i += 1;
                        return Some(items.into());
                    }
                    items.push(self.value(depth + 1)?);
                    self.ws();
                    match self.peek()? {
                        b',' => self.i += 1,
                        b']' => {}
                        _ => return None,
                    }
                }
            }
            b'\'' | b'"' | b'`' => self.string().map(Into::into),
            b'-' | b'0'..=b'9' => {
                let start = self.i;
                self.i += 1;
                while matches!(
                    self.peek(),
                    Some(b'0'..=b'9' | b'.' | b'e' | b'E' | b'+' | b'-')
                ) {
                    self.i += 1;
                }
                let n: f64 = std::str::from_utf8(&self.s[start..self.i])
                    .ok()?
                    .parse()
                    .ok()?;
                serde_json::Number::from_f64(n).map(Into::into)
            }
            _ => match self.ident()?.as_str() {
                "true" => Some(true.into()),
                "false" => Some(false.into()),
                "null" => Some(serde_json::Value::Null),
                _ => None,
            },
        }
    }

    fn ident(&mut self) -> Option<String> {
        let start = self.i;
        while matches!(self.peek(), Some(c) if c.is_ascii_alphanumeric() || c == b'_' || c == b'$')
        {
            self.i += 1;
        }
        (self.i > start).then(|| String::from_utf8_lossy(&self.s[start..self.i]).into_owned())
    }

    fn string(&mut self) -> Option<String> {
        let quote = self.peek()?;
        self.i += 1;
        let mut out: Vec<u8> = Vec::new();
        loop {
            let c = self.peek()?;
            self.i += 1;
            if c == quote {
                return String::from_utf8(out).ok();
            }
            match c {
                b'$' if quote == b'`' && self.peek() == Some(b'{') => return None,
                b'\\' => {
                    let e = self.peek()?;
                    self.i += 1;
                    match e {
                        b'n' => out.push(b'\n'),
                        b't' => out.push(b'\t'),
                        b'r' => out.push(b'\r'),
                        b'\n' => {}
                        b'u' | b'x' => return None,
                        other => out.push(other),
                    }
                }
                other => out.push(other),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_const_meta_is_parsed_and_export_dropped() {
        let script = "export const meta = {\n  name: 'audit',\n  description: \"scan; {all}\",\n  phases: [{ title: 'Scan' }, { title: 'Fix' },],\n};\nphase('Scan');\nreturn 1;";
        let (body, meta) = prepare(script);
        assert!(!body.contains("export"), "{body}");
        assert!(body.contains("const meta = {"), "binding kept: {body}");
        assert!(body.contains("phase('Scan');\nreturn 1;"), "{body}");
        let meta = meta.unwrap();
        assert_eq!(meta.name.as_deref(), Some("audit"));
        assert_eq!(meta.description.as_deref(), Some("scan; {all}"));
        assert_eq!(meta.phases, vec!["Scan", "Fix"]);
        assert_eq!(
            meta.banner().unwrap(),
            "audit — scan; {all} · phases: Scan → Fix"
        );
    }

    #[test]
    fn plain_const_meta_and_string_phases_work() {
        let script = "// header\nconst meta = { name: 'x', phases: ['A', 'B'] }\nreturn meta.name;";
        let (body, meta) = prepare(script);
        assert_eq!(body, script, "plain const meta runs as-is");
        assert_eq!(meta.unwrap().phases, vec!["A", "B"]);
    }

    #[test]
    fn non_literal_meta_still_runs_without_display_info() {
        let script = "export const meta = { name: compute() };\nreturn 1;";
        let (body, meta) = prepare(script);
        assert!(!body.contains("export"));
        assert!(meta.is_none());
    }

    #[test]
    fn scripts_without_meta_and_other_exports_are_untouched() {
        for script in [
            "const x = 1;\nreturn x;",
            "export const helper = 3;\nreturn helper;",
            "export default { a: 1 };",
            "const metadata = {};\nreturn 1;",
        ] {
            let (body, meta) = prepare(script);
            assert_eq!(body, script);
            assert!(meta.is_none());
        }
    }

    #[test]
    fn module_syntax_errors_are_recognised() {
        assert!(is_module_syntax_error(
            "script parse/setup error: unsupported keyword: export"
        ));
        assert!(!is_module_syntax_error("x is not defined"));
    }
}
