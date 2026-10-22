//! Prompt-template substitution.
//!
//! - `$ARGUMENTS` is the raw remainder of the line; `$ARGUMENTS[N]` is argument N, 0-based.
//! - `$NAME` is a declared named argument (`args:` or Claude Code's `arguments:`), by position.
//! - `$1..$9` are positional. Forge counts from 1 (`$1` is the first argument). A body that uses
//!   `$0` or `$ARGUMENTS[N]` was written for Claude Code's 0-based numbering, so in that body `$0`
//!   is the first argument and `$1` the second. A missing 0-based index stays literal, as in
//!   Claude Code; a missing 1-based one expands to nothing, as Forge always did.
//! - A single backslash before a placeholder (`\$1.00`) keeps it literal.
//! - `${CLAUDE_SESSION_ID}`, `${CLAUDE_PROJECT_DIR}`, `${CLAUDE_SKILL_DIR}`, `${CLAUDE_EFFORT}` and
//!   their `${FORGE_*}` spellings are replaced after the arguments, see [`TemplateVars`].
//!
//! Arguments split shell-style, so `"hello world" second` is two arguments. Values substitute as
//! typed: an argument that itself contains `$1` is not expanded again.

/// Values for the `${CLAUDE_*}` / `${FORGE_*}` variables. A variable without a value stays literal,
/// so a template rendered without a session still shows what it needed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TemplateVars {
    pub session_id: Option<String>,
    pub project_dir: Option<String>,
    pub skill_dir: Option<String>,
    pub effort: Option<String>,
}

impl TemplateVars {
    pub fn apply(&self, text: &str) -> String {
        let mut out = text.to_string();
        for (name, value) in [
            ("SESSION_ID", &self.session_id),
            ("PROJECT_DIR", &self.project_dir),
            ("SKILL_DIR", &self.skill_dir),
            ("EFFORT", &self.effort),
        ] {
            let Some(value) = value else { continue };
            for prefix in ["CLAUDE", "FORGE"] {
                let token = format!("${{{prefix}_{name}}}");
                if out.contains(&token) {
                    out = out.replace(&token, value);
                }
            }
        }
        out
    }

    /// The same variables with `skill_dir` set, for rendering one skill's files.
    pub fn for_skill_dir(&self, dir: &std::path::Path) -> TemplateVars {
        TemplateVars {
            skill_dir: (!dir.starts_with("<builtin>")).then(|| dir.display().to_string()),
            ..self.clone()
        }
    }
}

/// Split the text after a command name into arguments, shell-style: whitespace separates, and a
/// single- or double-quoted span is one argument with its quotes removed.
pub fn split_args(raw: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut in_arg = false;
    let mut quote: Option<char> = None;
    for ch in raw.chars() {
        match quote {
            Some(q) if ch == q => quote = None,
            Some(_) => current.push(ch),
            None if ch == '"' || ch == '\'' => {
                quote = Some(ch);
                in_arg = true;
            }
            None if ch.is_whitespace() => {
                if in_arg {
                    args.push(std::mem::take(&mut current));
                    in_arg = false;
                }
            }
            None => {
                current.push(ch);
                in_arg = true;
            }
        }
    }
    if in_arg {
        args.push(current);
    }
    args
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Placeholder {
    All,
    Index(usize),
    Positional(usize),
    Named(String),
}

/// The placeholder starting at the `$` at `at`, and the index just past it.
fn placeholder(
    chars: &[char],
    at: usize,
    named: &[(String, String)],
) -> Option<(Placeholder, usize)> {
    let start = at + 1;
    let mut end = start;
    while end < chars.len() && (chars[end].is_alphanumeric() || chars[end] == '_') {
        end += 1;
    }
    let token: String = chars[start..end].iter().collect();
    if token.is_empty() {
        return None;
    }
    if token == "ARGUMENTS" {
        if chars.get(end) == Some(&'[') {
            let digits_start = end + 1;
            let mut close = digits_start;
            while close < chars.len() && chars[close].is_ascii_digit() {
                close += 1;
            }
            if close > digits_start && chars.get(close) == Some(&']') {
                let n: String = chars[digits_start..close].iter().collect();
                if let Ok(n) = n.parse() {
                    return Some((Placeholder::Index(n), close + 1));
                }
            }
        }
        return Some((Placeholder::All, end));
    }
    if let Ok(n) = token.parse::<usize>() {
        return Some((Placeholder::Positional(n), end));
    }
    named
        .iter()
        .any(|(k, _)| *k == token)
        .then_some((Placeholder::Named(token), end))
}

/// Every unescaped placeholder in `body`, in order.
fn placeholders(body: &str, named: &[(String, String)]) -> Vec<Placeholder> {
    let chars: Vec<char> = body.chars().collect();
    let mut found = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '$' && !escaped(&chars, i) {
            if let Some((ph, end)) = placeholder(&chars, i, named) {
                found.push(ph);
                i = end;
                continue;
            }
        }
        i += 1;
    }
    found
}

/// Whether the `$` at `at` is escaped by exactly one backslash (`\$1`, not `\\$1`).
fn escaped(chars: &[char], at: usize) -> bool {
    at >= 1 && chars[at - 1] == '\\' && (at < 2 || chars[at - 2] != '\\')
}

/// Whether `body` numbers its arguments from 0 (Claude Code) rather than from 1 (Forge).
pub fn zero_based(body: &str) -> bool {
    placeholders(body, &[])
        .iter()
        .any(|p| matches!(p, Placeholder::Positional(0) | Placeholder::Index(_)))
}

/// Whether `body` contains any argument placeholder that would consume the user's typed text:
/// `$ARGUMENTS`, `$ARGUMENTS[N]`, a positional `$N`, or one of the declared `$NAME` args. Used so a
/// command whose body has no placeholder doesn't silently drop a typed task.
pub fn uses_args(body: &str, named: &[(String, String)]) -> bool {
    !placeholders(body, named).is_empty()
}

/// Expand `body` using positional args, named args (`(name, value)`), and `raw` (the verbatim
/// text after the command name, for `$ARGUMENTS`).
pub fn expand(body: &str, positional: &[&str], named: &[(String, String)], raw: &str) -> String {
    let zero = zero_based(body);
    let chars: Vec<char> = body.chars().collect();
    let mut out = String::with_capacity(body.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\\' && chars.get(i + 1) == Some(&'$') && !escaped(&chars, i + 1) {
            // `\\$1`: the doubled backslash stays and the placeholder still expands below.
            out.push(c);
            i += 1;
            continue;
        }
        if c == '\\' && chars.get(i + 1) == Some(&'$') {
            if let Some((_, end)) = placeholder(&chars, i + 1, named) {
                out.extend(&chars[i + 1..end]);
                i = end;
                continue;
            }
        }
        if c != '$' {
            out.push(c);
            i += 1;
            continue;
        }
        let Some((ph, end)) = placeholder(&chars, i, named) else {
            out.push('$');
            i += 1;
            continue;
        };
        let literal: String = chars[i..end].iter().collect();
        match ph {
            Placeholder::All => out.push_str(raw),
            Placeholder::Index(n) => match positional.get(n) {
                Some(v) => out.push_str(v),
                None => out.push_str(&literal),
            },
            Placeholder::Positional(n) if zero => match positional.get(n) {
                Some(v) => out.push_str(v),
                None => out.push_str(&literal),
            },
            Placeholder::Positional(0) => out.push_str(&literal),
            Placeholder::Positional(n) => {
                out.push_str(positional.get(n - 1).copied().unwrap_or(""))
            }
            Placeholder::Named(k) => {
                if let Some((_, v)) = named.iter().find(|(name, _)| *name == k) {
                    out.push_str(v);
                }
            }
        }
        i = end;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn forge_bodies_count_from_one() {
        assert_eq!(expand("$1 then $2", &["a", "b"], &[], "a b"), "a then b");
        assert_eq!(expand("missing: [$3]", &["a"], &[], "a"), "missing: []");
        assert!(!zero_based("$1 $2"));
    }

    #[test]
    fn claude_code_bodies_count_from_zero() {
        let body = "first=$0 second=$1 third=$ARGUMENTS[2] none=$4";
        assert!(zero_based(body));
        assert_eq!(
            expand(body, &["a", "b", "c"], &[], "a b c"),
            "first=a second=b third=c none=$4"
        );
        assert_eq!(
            expand("$ARGUMENTS[0] / $ARGUMENTS[9]", &["x"], &[], "x"),
            "x / $ARGUMENTS[9]"
        );
    }

    #[test]
    fn arguments_and_named_args_substitute() {
        let n = named(&[("issue", "42"), ("branch", "")]);
        assert_eq!(
            expand("fix #$issue on [$branch]: $ARGUMENTS", &["42"], &n, "42"),
            "fix #42 on []: 42"
        );
    }

    #[test]
    fn backslash_escapes_one_placeholder() {
        assert_eq!(expand(r"costs \$1.00", &["x"], &[], "x"), "costs $1.00");
        assert_eq!(expand(r"keep \\$1", &["x"], &[], "x"), r"keep \\x");
        assert_eq!(expand(r"other \$HOME", &[], &[], ""), r"other \$HOME");
        assert!(!zero_based(r"\$0 is literal"));
    }

    #[test]
    fn values_are_not_expanded_twice() {
        assert_eq!(
            expand("Summarize $0", &["$ARGUMENTS from yesterday"], &[], ""),
            "Summarize $ARGUMENTS from yesterday"
        );
    }

    #[test]
    fn uses_args_sees_every_placeholder_kind() {
        assert!(uses_args("x $ARGUMENTS", &[]));
        assert!(uses_args("x $0", &[]));
        assert!(uses_args("x $ARGUMENTS[1]", &[]));
        assert!(uses_args("x $2", &[]));
        assert!(uses_args("x $path", &named(&[("path", "")])));
        assert!(!uses_args("x $path costs \\$5", &[]));
    }

    #[test]
    fn args_split_shell_style() {
        assert_eq!(
            split_args(r#""hello world" second"#),
            ["hello world", "second"]
        );
        assert_eq!(split_args("  a   'b c'  d"), ["a", "b c", "d"]);
        assert_eq!(split_args(r#"empty "" x"#), ["empty", "", "x"]);
        assert!(split_args("   ").is_empty());
    }

    #[test]
    fn variables_replace_both_spellings_and_unknown_stay() {
        let vars = TemplateVars {
            session_id: Some("s-1".into()),
            project_dir: Some("/repo".into()),
            skill_dir: None,
            effort: Some("high".into()),
        };
        assert_eq!(
            vars.apply(
                "${CLAUDE_SESSION_ID} ${FORGE_PROJECT_DIR} ${CLAUDE_SKILL_DIR} ${CLAUDE_EFFORT}"
            ),
            "s-1 /repo ${CLAUDE_SKILL_DIR} high"
        );
        let skill = vars.for_skill_dir(std::path::Path::new("/skills/x"));
        assert_eq!(
            skill.apply("${CLAUDE_SKILL_DIR}/run.sh"),
            "/skills/x/run.sh"
        );
        let builtin = vars.for_skill_dir(std::path::Path::new("<builtin>/skills/rust"));
        assert_eq!(builtin.skill_dir, None);
    }
}
