//! Provably read-only shell commands, the only shell `plan` mode lets through.
//!
//! Plan mode denies every side effect, which used to include `ls`, `wc -l` and `git log`: a model
//! told to inspect a tree said "I'll run `wc -l`" a dozen times, was refused each time, and never
//! got its answer. Claude Code's plan mode runs read-only commands, so this is a strict allowlist
//! in the same spirit: a command passes only when EVERY effective segment (pipeline stage, `&&`
//! operand, `$(…)` body) is a known inspection command with no write-capable flag, and the line
//! carries no output redirect. Anything unrecognised, unparseable or clever stays denied. Like the
//! rest of the broker this is a floor against accidents, not a sandbox.

use super::effective_commands;

/// Commands that cannot modify state whatever their arguments (output goes to stdout only).
const ALWAYS_READ_ONLY: &[&str] = &[
    "ls",
    "wc",
    "cat",
    "head",
    "tail",
    "pwd",
    "echo",
    "printf",
    "stat",
    "file",
    "du",
    "df",
    "nl",
    "cut",
    "tr",
    "diff",
    "cmp",
    "md5sum",
    "sha1sum",
    "sha256sum",
    "b3sum",
    "basename",
    "dirname",
    "realpath",
    "readlink",
    "which",
    "whoami",
    "uname",
    "id",
    "hostname",
    "column",
    "rev",
    "tac",
    "fold",
    "expand",
    "grep",
    "egrep",
    "fgrep",
    "jq",
    "true",
    "false",
    "test",
    "[",
    "seq",
];

const GIT_READ_ONLY: &[&str] = &[
    "status",
    "log",
    "diff",
    "show",
    "rev-parse",
    "rev-list",
    "ls-files",
    "ls-tree",
    "blame",
    "describe",
    "shortlog",
    "cat-file",
    "grep",
    "merge-base",
    "name-rev",
    "diff-tree",
    "show-ref",
];

const CARGO_READ_ONLY: &[&str] = &["metadata", "tree", "locate-project", "pkgid", "version"];

/// Appended to the `shell` tool's description in plan mode so the model knows inspection commands
/// are available instead of narrating "I'll run `wc -l`" and reaching for another tool.
pub(crate) const PLAN_SHELL_NOTE: &str = " In plan mode only read-only commands run (ls, wc, cat, \
head, tail, rg, grep, find without -exec/-delete, git status/log/diff/show, cargo metadata, du, \
stat, file); redirects, writers and other commands are denied.";

/// What a denied call returns in plan mode: the standard marker first (failure classification keys
/// on it), then what is allowed, so the model retries with a legal command or answers.
pub(crate) fn plan_shell_denial() -> String {
    format!("permission denied by policy:{PLAN_SHELL_NOTE} Retry with such a command or answer from what you have.")
}

/// True only when running `cmd` cannot write anything.
pub(crate) fn is_read_only_shell(cmd: &str) -> bool {
    let Some(clean) = strip_harmless_redirects(cmd) else {
        return false;
    };
    if has_assignment_word(&clean) {
        return false;
    }
    let (segments, parsed_ok) = effective_commands(&clean);
    parsed_ok && !segments.is_empty() && segments.iter().all(|s| segment_is_read_only(s))
}

/// Remove `2>&1`-style fd duplication and `>/dev/null` discards (no file is written), and refuse
/// any other unquoted redirect or input/process substitution. A redirect inside `$(…)`/backticks
/// is invisible to the quote tracker, so a line using either must contain no `<`/`>` at all.
fn strip_harmless_redirects(cmd: &str) -> Option<String> {
    if (cmd.contains("$(") || cmd.contains('`')) && cmd.contains(['<', '>']) {
        return None;
    }
    let chars: Vec<char> = cmd.chars().collect();
    let mut out = String::with_capacity(cmd.len());
    let mut quote: Option<char> = None;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match quote {
            Some(q) => {
                if c == '\\' && q == '"' && i + 1 < chars.len() {
                    out.push(c);
                    out.push(chars[i + 1]);
                    i += 2;
                    continue;
                }
                if c == q {
                    quote = None;
                }
                out.push(c);
            }
            None => match c {
                '\\' if i + 1 < chars.len() => {
                    out.push(c);
                    out.push(chars[i + 1]);
                    i += 2;
                    continue;
                }
                '\'' | '"' => {
                    quote = Some(c);
                    out.push(c);
                }
                '<' => return None,
                '>' => {
                    i = harmless_redirect_end(&chars, i)?;
                    continue;
                }
                _ => out.push(c),
            },
        }
        i += 1;
    }
    Some(out)
}

/// Index just past a redirect starting at `chars[at] == '>'` when it writes nowhere, else `None`.
/// A leading fd digit or `&` was already copied to the output; they are inert without the `>`.
fn harmless_redirect_end(chars: &[char], at: usize) -> Option<usize> {
    let mut i = at + 1;
    if chars.get(i) == Some(&'&') {
        i += 1;
        return match chars.get(i) {
            Some(c) if c.is_ascii_digit() || *c == '-' => {
                let mut end = i + 1;
                while chars.get(end).is_some_and(char::is_ascii_digit) {
                    end += 1;
                }
                Some(end)
            }
            _ => None,
        };
    }
    if chars.get(i) == Some(&'>') {
        i += 1;
    }
    while chars.get(i) == Some(&' ') {
        i += 1;
    }
    let target: String = chars[i..].iter().take("/dev/null".len()).collect();
    let end = i + "/dev/null".len();
    let delimited = chars
        .get(end)
        .is_none_or(|c| c.is_whitespace() || ";&|)".contains(*c));
    (target == "/dev/null" && delimited).then_some(end)
}

fn segment_is_read_only(segment: &str) -> bool {
    let Ok(words) = shell_words::split(segment) else {
        return false;
    };
    let Some((cmd_word, rest)) = words.split_first() else {
        return false;
    };
    if cmd_word.contains('/') {
        return false;
    }
    let cmd = cmd_word.as_str();
    let rest: Vec<&str> = rest.iter().map(String::as_str).collect();
    let flag = |name: &str| {
        rest.iter()
            .any(|w| *w == name || w.starts_with(&format!("{name}=")))
    };
    match cmd {
        // `uniq IN OUT` writes OUT; `sort -o` / `tree -o` write a file; `date -s` sets the clock.
        "uniq" => rest.iter().filter(|w| !w.starts_with('-')).count() <= 1,
        "sort" => !rest
            .iter()
            .any(|w| *w == "-o" || w.starts_with("--output") || is_short_flag(w, 'o')),
        "tree" => !rest.iter().any(|w| is_short_flag(w, 'o')) && !flag("--output"),
        "date" => !rest.iter().any(|w| *w == "-s" || w.starts_with("--set")),
        "find" => !rest.iter().any(|w| {
            matches!(
                *w,
                "-delete"
                    | "-exec"
                    | "-execdir"
                    | "-ok"
                    | "-okdir"
                    | "-fprint"
                    | "-fprint0"
                    | "-fprintf"
                    | "-fls"
            )
        }),
        // `--pre` runs a preprocessor command over every file searched.
        "rg" => !flag("--pre") && !flag("--hostname-bin"),
        "git" => git_read_only(&rest),
        "cargo" => rest
            .first()
            .is_some_and(|sub| CARGO_READ_ONLY.contains(sub) || *sub == "--version"),
        c => ALWAYS_READ_ONLY.contains(&c),
    }
}

fn git_read_only(rest: &[&str]) -> bool {
    let Some((sub, args)) = rest.split_first() else {
        return false;
    };
    // `--output`, `--ext-diff` and `--textconv` run helpers or write files; `-c` injects config.
    let hostile = args.iter().any(|a| {
        a.starts_with("--output") || matches!(*a, "--ext-diff" | "--textconv") || *a == "-c"
    });
    if hostile {
        return false;
    }
    match *sub {
        "branch" => args
            .iter()
            .all(|a| matches!(*a, "-a" | "-r" | "-v" | "-vv" | "--list" | "--show-current")),
        "remote" => args.iter().all(|a| *a == "-v"),
        s => GIT_READ_ONLY.contains(&s),
    }
}

fn is_short_flag(word: &str, c: char) -> bool {
    word.starts_with('-') && !word.starts_with("--") && word[1..].contains(c)
}

/// A bare `VAR=value` prefix is stripped by `effective_commands`, which would hide
/// `LD_PRELOAD=… ls` or `GIT_EXTERNAL_DIFF=… git diff`, so the raw line is checked for any
/// assignment-shaped word (over-strict for a quoted `a=b` argument, which is fine).
fn has_assignment_word(cmd: &str) -> bool {
    let spaced: String = cmd
        .chars()
        .map(|c| if ";&|()\n\r".contains(c) { ' ' } else { c })
        .collect();
    shell_words::split(&spaced).map_or(true, |ws| ws.iter().any(|w| is_assignment(w)))
}

fn is_assignment(word: &str) -> bool {
    word.split_once('=').is_some_and(|(name, _)| {
        !name.is_empty()
            && !name.starts_with(|c: char| c.is_ascii_digit())
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}

#[cfg(test)]
mod tests {
    use super::is_read_only_shell as ro;

    #[test]
    fn allows_inspection_commands() {
        for cmd in [
            "ls -la crates/forge-mesh/src",
            "wc -l crates/forge-mesh/src/*.rs",
            "wc -l crates/forge-mesh/src/*.rs | sort -n | tail -3",
            "cat Cargo.toml | head -20",
            "git status",
            "git log --oneline -5",
            "git diff HEAD~1 -- crates",
            "git show HEAD:README.md",
            "git branch -a",
            "rg -n 'fn main' crates | head",
            "grep -rn TODO . 2>/dev/null",
            "find . -name '*.rs' -type f",
            "cargo metadata --no-deps --format-version 1",
            "du -sh target && ls",
            "echo \"a > b\"",
            "ls 2>&1 | wc -l",
            "echo $(pwd)",
            "git status > /dev/null",
        ] {
            assert!(ro(cmd), "should allow: {cmd}");
        }
    }

    #[test]
    fn denies_anything_that_can_write_or_is_unknown() {
        for cmd in [
            "ls > out.txt",
            "ls >> out.txt",
            "cat a > b",
            "echo hi | tee f",
            "cat < f",
            "cat <(ls)",
            "ls &> out",
            "ls 2> err.log",
            "find . -delete",
            "find . -name x -exec rm {} ;",
            "find . -fprint out",
            "rg --pre ./evil x",
            "sort -o out in",
            "sort in -oout",
            "uniq in out",
            "date -s tomorrow",
            "git commit -m x",
            "git push",
            "git checkout main",
            "git -c core.pager=x log",
            "git diff --output=f",
            "git branch -D x",
            "git branch newbranch",
            "git config user.name x",
            "cargo build",
            "cargo install foo",
            "rm -rf x",
            "ls; rm x",
            "ls && touch f",
            "ls | xargs rm",
            "echo $(rm x)",
            "echo `touch f`",
            "echo $(ls > f)",
            "FOO=1 ls",
            "LD_PRELOAD=x ls",
            "env rm x",
            "bash -c 'rm x'",
            "sed -i s/a/b/ f",
            "awk 'BEGIN{system(\"x\")}'",
            "./script.sh",
            "/bin/ls",
            "curl http://x",
            "python -c 'print(1)'",
            "ls\nrm x",
            "ls 'unterminated",
            "",
        ] {
            assert!(!ro(cmd), "should deny: {cmd:?}");
        }
    }
}
