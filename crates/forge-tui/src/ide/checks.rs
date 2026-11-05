//! Project checks as plain data, ported from the desktop `checksModel.ts`: pass/fail from a
//! project action's run state, `file:line` links in its output, the changed files a failing check
//! implicates, and the "ask the agent to fix it" prompt.

/// One of `GET /api/terminals/actions`' rows.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize)]
pub struct WireAction {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub terminal_id: Option<String>,
    #[serde(default)]
    pub running: bool,
    #[serde(default)]
    pub exit_code: Option<i32>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize)]
pub struct WireActions {
    #[serde(default)]
    pub actions: Vec<WireAction>,
    #[serde(default)]
    pub untrusted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckState {
    Idle,
    Running,
    Pass,
    Fail,
}

pub fn check_state(action: &WireAction) -> CheckState {
    if action.running {
        CheckState::Running
    } else {
        match action.exit_code {
            None => CheckState::Idle,
            Some(0) => CheckState::Pass,
            Some(_) => CheckState::Fail,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRef {
    pub path: String,
    pub line: usize,
    pub column: Option<usize>,
}

/// Drop escape sequences (CSI, OSC, charset selects) and turn lone carriage returns into newlines,
/// so a progress bar's redraws read as lines.
pub fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\u{1b}' => match chars.next() {
                Some('[') => {
                    for c in chars.by_ref() {
                        if ('@'..='~').contains(&c) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    while let Some(c) = chars.next() {
                        if c == '\u{7}' {
                            break;
                        }
                        if c == '\u{1b}' {
                            chars.next();
                            break;
                        }
                    }
                }
                Some('(' | ')') => {
                    chars.next();
                }
                _ => {}
            },
            '\r' => {
                if chars.peek() != Some(&'\n') {
                    out.push('\n');
                }
            }
            c => out.push(c),
        }
    }
    out
}

fn path_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '@' | '.' | '+' | '~' | '-' | '/')
}

fn looks_like_path(path: &str) -> bool {
    let Some((stem, ext)) = path.rsplit_once('.') else {
        return false;
    };
    let name_ok = !stem.is_empty() && !stem.ends_with('/');
    let ext_ok = (1..=6).contains(&ext.len())
        && ext.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        && ext.chars().all(|c| c.is_ascii_alphanumeric());
    // "v1.2" and "1.5" are versions, not files.
    let version = path.chars().all(|c| c.is_ascii_digit() || c == '.');
    name_ok && ext_ok && !version && !path.contains("//")
}

fn number(text: &str) -> Option<(usize, &str)> {
    let end = text
        .char_indices()
        .find(|(_, c)| !c.is_ascii_digit())
        .map_or(text.len(), |(i, _)| i);
    if end == 0 {
        return None;
    }
    Some((text[..end].parse().ok()?, &text[end..]))
}

/// Every `path:line[:col]` and `path(line[,col])` in one line, with the byte range each spans.
pub fn refs_in(line: &str) -> Vec<(FileRef, std::ops::Range<usize>)> {
    let mut out = Vec::new();
    let bytes_len = line.len();
    let mut start = 0;
    while start < bytes_len {
        let Some((offset, _)) = line[start..].char_indices().find(|(_, c)| path_char(*c)) else {
            break;
        };
        let begin = start + offset;
        let end = line[begin..]
            .char_indices()
            .find(|(_, c)| !path_char(*c))
            .map_or(bytes_len, |(i, _)| begin + i);
        let word = &line[begin..end];
        let rest = &line[end..];
        let clean = word.strip_prefix("./").unwrap_or(word);
        let before_ok = begin == 0 || !line[..begin].ends_with(|c: char| c.is_alphanumeric());
        if before_ok && looks_like_path(clean) {
            if let Some(after) = rest.strip_prefix(':') {
                if let Some((n, tail)) = number(after) {
                    let mut stop = bytes_len - tail.len();
                    let mut column = None;
                    if let Some(more) = tail.strip_prefix(':') {
                        if let Some((c, tail2)) = number(more) {
                            column = Some(c);
                            stop = bytes_len - tail2.len();
                        }
                    }
                    out.push((
                        FileRef {
                            path: clean.to_string(),
                            line: n,
                            column,
                        },
                        begin..stop,
                    ));
                    start = stop;
                    continue;
                }
            } else if let Some(after) = rest.strip_prefix('(') {
                if let Some((n, tail)) = number(after) {
                    let mut column = None;
                    let mut tail = tail;
                    if let Some(more) = tail.strip_prefix(',') {
                        if let Some((c, tail2)) = number(more) {
                            column = Some(c);
                            tail = tail2;
                        }
                    }
                    if let Some(close) = tail.strip_prefix(')') {
                        let stop = bytes_len - close.len();
                        out.push((
                            FileRef {
                                path: clean.to_string(),
                                line: n,
                                column,
                            },
                            begin..stop,
                        ));
                        start = stop;
                        continue;
                    }
                }
            }
        }
        start = end.max(begin + 1);
    }
    out
}

/// The distinct refs of a whole output, first seen first.
pub fn parse_file_refs(text: &str, max: usize) -> Vec<FileRef> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for line in strip_ansi(text).lines() {
        for (found, _) in refs_in(line) {
            if seen.insert((found.path.clone(), found.line)) {
                out.push(found);
                if out.len() >= max {
                    return out;
                }
            }
        }
    }
    out
}

/// A ref names a changed file when one path is the other's suffix (actions can run in a subdir).
pub fn same_path(changed: &str, found: &str) -> bool {
    changed == found
        || changed.ends_with(&format!("/{found}"))
        || found.ends_with(&format!("/{changed}"))
}

pub fn failing_changed_files(changed: &[String], refs: &[FileRef]) -> Vec<String> {
    changed
        .iter()
        .filter(|path| refs.iter().any(|r| same_path(path, &r.path)))
        .cloned()
        .collect()
}

/// The path the editor should open for a ref, preferring a changed file's own spelling.
pub fn resolve_ref_path(found: &FileRef, changed: &[String]) -> String {
    changed
        .iter()
        .find(|path| same_path(path, &found.path))
        .cloned()
        .unwrap_or_else(|| found.path.clone())
}

const FIX_TAIL_LINES: usize = 60;

pub fn tail_lines(text: &str, count: usize) -> String {
    let clean = strip_ansi(text);
    let lines: Vec<&str> = clean.lines().collect();
    lines[lines.len().saturating_sub(count)..]
        .join("\n")
        .trim()
        .to_string()
}

pub fn fix_prompt(name: &str, command: &str, exit_code: Option<i32>, output: &str) -> String {
    let code = exit_code.map_or(String::new(), |c| format!(" (exit {c})"));
    format!(
        "The \"{name}\" check failed{code}. Command: {command}\n\nLast output:\n\n{}\n\nPlease fix what is failing and run it again.",
        tail_lines(output, FIX_TAIL_LINES)
    )
}

/// Project actions that are checks (test, lint, build). When none look like one, all are shown.
pub fn check_actions(actions: &[WireAction]) -> Vec<WireAction> {
    const WORDS: &[&str] = &[
        "test",
        "tests",
        "lint",
        "build",
        "check",
        "clippy",
        "fmt",
        "format",
        "tsc",
        "typecheck",
        "types",
        "vitest",
        "jest",
        "pytest",
        "ci",
    ];
    let is_check = |a: &WireAction| {
        let hay = format!("{} {} {}", a.id, a.name, a.command).to_lowercase();
        hay.split(|c: char| !c.is_alphanumeric())
            .any(|w| WORDS.contains(&w))
    };
    let checks: Vec<WireAction> = actions.iter().filter(|a| is_check(a)).cloned().collect();
    if checks.is_empty() {
        actions.to_vec()
    } else {
        checks
    }
}

pub const OUTPUT_LIMIT: usize = 200_000;

pub fn append_output(buffer: &mut String, chunk: &str) {
    buffer.push_str(chunk);
    if buffer.len() > OUTPUT_LIMIT {
        let mut cut = buffer.len() - OUTPUT_LIMIT;
        while !buffer.is_char_boundary(cut) {
            cut += 1;
        }
        buffer.drain(..cut);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn action(running: bool, exit: Option<i32>) -> WireAction {
        WireAction {
            running,
            exit_code: exit,
            ..WireAction::default()
        }
    }

    #[test]
    fn states_follow_the_run() {
        assert_eq!(check_state(&action(true, None)), CheckState::Running);
        assert_eq!(check_state(&action(false, None)), CheckState::Idle);
        assert_eq!(check_state(&action(false, Some(0))), CheckState::Pass);
        assert_eq!(check_state(&action(false, Some(101))), CheckState::Fail);
    }

    #[test]
    fn refs_cover_rustc_tsc_pytest_and_go() {
        let text = "error[E0308]: mismatched types\n  --> src/lib.rs:12:5\nsrc/app.ts(40,7): error TS2322\ntests/test_a.py:9: AssertionError\n./cmd/main.go:3:1: undefined: x";
        let refs = parse_file_refs(text, 50);
        assert_eq!(
            refs,
            vec![
                FileRef {
                    path: "src/lib.rs".into(),
                    line: 12,
                    column: Some(5)
                },
                FileRef {
                    path: "src/app.ts".into(),
                    line: 40,
                    column: Some(7)
                },
                FileRef {
                    path: "tests/test_a.py".into(),
                    line: 9,
                    column: None
                },
                FileRef {
                    path: "cmd/main.go".into(),
                    line: 3,
                    column: Some(1)
                },
            ]
        );
    }

    #[test]
    fn versions_and_hosts_are_not_files() {
        assert!(parse_file_refs("v1.2:3 localhost:8080 1.5:2 compiled in 2.3s", 10).is_empty());
    }

    #[test]
    fn duplicates_collapse_and_colour_codes_are_ignored() {
        let text = "\u{1b}[31msrc/a.rs:1\u{1b}[0m and src/a.rs:1 and src/a.rs:2";
        assert_eq!(parse_file_refs(text, 10).len(), 2);
    }

    #[test]
    fn the_ref_range_covers_path_and_position_only() {
        let line = "  --> src/lib.rs:12:5 note";
        let (found, range) = refs_in(line).remove(0);
        assert_eq!(&line[range], "src/lib.rs:12:5");
        assert_eq!(found.line, 12);
    }

    #[test]
    fn failing_checks_implicate_changed_files_by_suffix() {
        let changed = vec!["crates/a/src/lib.rs".to_string(), "README.md".to_string()];
        let refs = parse_file_refs("src/lib.rs:3: boom", 5);
        assert_eq!(
            failing_changed_files(&changed, &refs),
            vec!["crates/a/src/lib.rs".to_string()]
        );
        assert_eq!(
            resolve_ref_path(&refs[0], &changed),
            "crates/a/src/lib.rs".to_string()
        );
    }

    #[test]
    fn the_fix_prompt_carries_the_tail() {
        let output: String = (0..100).map(|i| format!("line {i}\n")).collect();
        let prompt = fix_prompt("Tests", "cargo test", Some(101), &output);
        assert!(prompt.starts_with("The \"Tests\" check failed (exit 101). Command: cargo test"));
        assert!(prompt.contains("line 99") && !prompt.contains("line 10\n"));
    }

    #[test]
    fn checks_prefer_test_lint_build_actions() {
        let mk = |id: &str| WireAction {
            id: id.into(),
            name: id.into(),
            command: "x".into(),
            ..WireAction::default()
        };
        let all = vec![mk("dev"), mk("test"), mk("lint")];
        assert_eq!(check_actions(&all).len(), 2);
        let none = vec![mk("dev"), mk("serve")];
        assert_eq!(check_actions(&none).len(), 2);
    }

    #[test]
    fn output_is_capped_on_a_char_boundary() {
        let mut buf = String::new();
        append_output(&mut buf, &"é".repeat(OUTPUT_LIMIT));
        assert!(buf.len() <= OUTPUT_LIMIT);
        assert!(buf.chars().all(|c| c == 'é'));
    }
}
