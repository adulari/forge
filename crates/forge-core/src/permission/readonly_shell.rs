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
    "branch",
    "remote",
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
pub(super) fn strip_harmless_redirects(cmd: &str) -> Option<String> {
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
    // Where the shell ends the redirect's word: a blank or an operator. NOT `char::is_whitespace`,
    // which also takes no-break and other Unicode spaces that `sh` keeps inside the word: with
    // `>&2` followed by U+00A0 and a name, the shell redirects into a file called `2<nbsp>name`
    // while a Unicode-aware reading sees `>&2`, a separator and a harmless word.
    let delimited = |end: usize| {
        chars
            .get(end)
            .is_none_or(|c| matches!(c, ' ' | '\t' | '\n' | ';' | '&' | '|' | ')'))
    };
    let mut i = at + 1;
    if chars.get(i) == Some(&'&') {
        i += 1;
        // `>&1` duplicates a descriptor, but `>&1file` is a redirect into a file called `1file`.
        return match chars.get(i) {
            Some(c) if c.is_ascii_digit() || *c == '-' => {
                let mut end = i + 1;
                while chars.get(end).is_some_and(char::is_ascii_digit) {
                    end += 1;
                }
                delimited(end).then_some(end)
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
    (target == "/dev/null" && delimited(end)).then_some(end)
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
    if let Some(verdict) = shared_args_read_only(cmd, &rest) {
        return verdict;
    }
    match cmd {
        "date" => !rest.iter().any(|w| *w == "-s" || w.starts_with("--set")),
        "git" => git_read_only(&rest, GIT_READ_ONLY),
        "cargo" => rest
            .first()
            .is_some_and(|sub| CARGO_READ_ONLY.contains(sub) || *sub == "--version"),
        c => ALWAYS_READ_ONLY.contains(&c),
    }
}

/// The argument rules for binaries whose read-only status depends on a flag, shared by the Plan
/// classifier and the Ask auto-allow so the two cannot drift: `Some(verdict)` for a binary with
/// such a rule, `None` for one without. A binary reaching here is already known to be one the
/// caller allows; this only vets its arguments.
pub(super) fn shared_args_read_only(cmd: &str, rest: &[&str]) -> Option<bool> {
    let long = |full: &str| rest.iter().any(|w| is_long_option(w, full));
    Some(match cmd {
        // `uniq IN OUT` writes OUT, and a wildcard can turn one operand into two (`uniq *.txt`);
        // `sort -o` / `tree -o` write a file. `sort` also execs the `--compress-program` it is
        // given and writes spill files under `-T`.
        "uniq" => uniq_operands(rest) <= 1 && !rest.iter().any(|w| w.contains(['*', '?', '['])),
        // bash's builtin `printf -v NAME` assigns a shell variable, `-v 'a[$(cmd)]'` evaluates the
        // subscript, and `-v PATH` or `-v HOME` changes what every later statement of the line
        // runs or reads. Only a format string, not an option, may come first.
        "printf" => rest.first().is_none_or(|first| !first.starts_with('-')),
        "sort" => {
            !(rest
                .iter()
                .any(|w| is_short_flag(w, 'o') || is_short_flag(w, 'T'))
                || long("--output")
                || long("--compress-program")
                || long("--temporary-directory"))
        }
        "tree" => !rest.iter().any(|w| is_short_flag(w, 'o')) && !long("--output"),
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
                    // bfs spells `-delete` this way too.
                    | "-rm"
            )
        }),
        // `--pre` runs a preprocessor command over every file searched, `--hostname-bin` runs a
        // program, and `-z` shells out to a decompressor for each file.
        "rg" => {
            !(long("--pre")
                || long("--hostname-bin")
                || long("--search-zip")
                || rest.iter().any(|w| is_short_flag(w, 'z')))
        }
        _ => return None,
    })
}

/// How many operands `uniq` was given. `-` (standard input) and everything after `--` are
/// operands too, so `uniq - out` is `INPUT OUTPUT` and counts as two. A value given as a separate
/// word (`-f 1`) counts as one as well, which only ever over-counts.
fn uniq_operands(rest: &[&str]) -> usize {
    let mut after_dashes = false;
    let mut count = 0;
    for word in rest {
        if after_dashes || *word == "-" || !word.starts_with('-') {
            count += 1;
        } else if *word == "--" {
            after_dashes = true;
        }
    }
    count
}

/// Whether `word` is the long option `full` or something a getopt_long-style parser accepts as an
/// abbreviation of it (`sort --out=x` is `sort --output=x`; `git grep --open=sh` is
/// `--open-files-in-pager=sh`). Too short to be an abbreviation (`--`) is not a match.
pub(super) fn is_long_option(word: &str, full: &str) -> bool {
    let name = word.split_once('=').map_or(word, |(name, _)| name);
    name.len() >= 3 && name.starts_with("--") && full.starts_with(name)
}

/// Whether `git <rest>` is one of the read subcommands in `subcommands` with no flag that writes a
/// file or runs a helper. Anything before the subcommand (`-c`, `-C`, `--git-dir`, `--exec-path`)
/// makes the first word a non-subcommand, so it fails closed.
pub(super) fn git_read_only(rest: &[&str], subcommands: &[&str]) -> bool {
    let Some((sub, args)) = rest.split_first() else {
        return false;
    };
    if args.iter().any(|a| git_arg_is_hostile(sub, a)) {
        return false;
    }
    match *sub {
        "branch" if subcommands.contains(&"branch") => args
            .iter()
            .all(|a| matches!(*a, "-a" | "-r" | "-v" | "-vv" | "--list" | "--show-current")),
        "remote" if subcommands.contains(&"remote") => args.iter().all(|a| *a == "-v"),
        "tag" if subcommands.contains(&"tag") => tag_only_lists(args),
        "reflog" if subcommands.contains(&"reflog") => {
            args.first().is_none_or(|first| *first == "show")
        }
        "stash" | "worktree" if subcommands.contains(sub) => args.first() == Some(&"list"),
        s => subcommands.contains(&s),
    }
}

/// `git tag` with no arguments lists; with arguments it creates or deletes unless it is told to
/// list, and then only the listing flags are accepted.
fn tag_only_lists(args: &[&str]) -> bool {
    args.is_empty()
        || (args.iter().any(|a| matches!(*a, "-l" | "--list"))
            && args
                .iter()
                .filter(|a| a.starts_with('-'))
                .all(|a| matches!(*a, "-l" | "--list" | "-n" | "-i" | "--ignore-case")))
}

/// A git argument that writes a file or runs a helper. `--output`, `--ext-diff` and `--textconv`
/// run helpers or write files; `-c` injects config; `-O<cmd>` is `git grep`'s pager command and may
/// sit inside a cluster of short flags (`-nOsh`); `--exec*` and `--config-env` redirect what runs;
/// `--show-signature`, the `%G` placeholders and `for-each-ref`'s `signature` atoms run `gpg` over a
/// signature the repository supplied;
/// `--submodule` and `--recurse-submodules` read through a submodule's own repository.
fn git_arg_is_hostile(sub: &str, arg: &str) -> bool {
    let name = arg.split_once('=').map_or(arg, |(name, _)| name);
    // Real options that only start like a hostile one: git takes an exact name over an
    // abbreviation, but only for the subcommands that have the option.
    if matches!(sub, "diff" | "log" | "show" | "grep")
        && matches!(
            name,
            "--text"
                | "--output-indicator-new"
                | "--output-indicator-old"
                | "--output-indicator-context"
        )
    {
        return false;
    }
    arg.starts_with("--output")
        || arg == "-c"
        // `%(signature)` atoms and `--sort=signature` verify a signature with `gpg`, as `%G` does.
        || (sub == "for-each-ref" && arg.contains("signature"))
        || is_short_flag(arg, 'O')
        || arg.starts_with("--exec")
        || arg.contains("%G")
        || [
            "--ext-diff",
            "--textconv",
            "--open-files-in-pager",
            "--config-env",
            "--show-signature",
            "--submodule",
            "--recurse-submodules",
            "--exec-path",
            "--output",
        ]
        .iter()
        .any(|full| is_long_option(arg, full))
}

pub(super) fn is_short_flag(word: &str, c: char) -> bool {
    word.starts_with('-') && !word.starts_with("--") && word[1..].contains(c)
}

/// A bare `VAR=value` prefix is stripped by `effective_commands`, which would hide
/// `LD_PRELOAD=… ls` or `GIT_EXTERNAL_DIFF=… git diff`, so the raw line is checked for any
/// assignment-shaped word (over-strict for a quoted `a=b` argument, which is fine).
pub(super) fn has_assignment_word(cmd: &str) -> bool {
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
            "ls >&2",
            "ls >&1 2>&1 | wc -l",
            "ls 2>&-",
            "git diff --text",
            "git diff --output-indicator-new=+",
            "printf %s hi",
            "printf '%s\\n' a b",
            "uniq",
            "uniq -c",
            "uniq calc.py",
            "cat f | uniq -",
        ] {
            assert!(ro(cmd), "should allow: {cmd}");
        }
    }

    /// `sh` ends the word of `>&2` at a blank or an operator. `>&2<nbsp>h/out` is one word, so the
    /// shell redirects both streams into a file called `2<nbsp>h/out`; an end test that takes any
    /// Unicode space for a separator strips `>&2` as a harmless descriptor copy and sees `h/out`
    /// as a lone argument.
    #[test]
    fn a_redirect_ends_at_a_blank_and_never_at_a_unicode_space() {
        let spaces = [
            '\u{a0}', '\u{85}', '\u{b}', '\u{c}', '\r', '\u{1680}', '\u{2000}', '\u{2003}',
            '\u{200a}', '\u{2028}', '\u{2029}', '\u{202f}', '\u{205f}', '\u{3000}',
        ];
        for space in spaces {
            for cmd in [
                format!("printf x >&2{space}h/out"),
                format!("printf x >&1{space}out"),
                format!("printf x >&-{space}out"),
                format!("printf x 2>&1{space}out"),
                format!("printf x >/dev/null{space}out"),
                format!("printf x 2>/dev/null{space}out"),
                format!("printf x >> /dev/null{space}out"),
                format!("cat f >&2{space}h/.bashrc"),
            ] {
                assert!(!ro(&cmd), "should deny: {cmd:?}");
            }
        }
        for cmd in [
            "printf x >&2",
            "printf x >&2 ; ls",
            "printf x >&2\tcalc.py",
            "ls 2>&1 | wc -l",
            "ls >/dev/null && cat f",
            "ls >&2|wc -l",
        ] {
            assert!(ro(cmd), "should allow: {cmd:?}");
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
            "ls >&1out",
            "ls >&2err",
            "ls >&-x",
            "ls 2>&1out",
            "ls >&1>out",
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
            "sort --out=x in",
            "sort --compress-program=sh in",
            "sort -T /tmp in",
            "git grep -O'sh -c x' foo",
            "git grep --open=sh foo",
            "git grep -nOsh foo",
            "git grep -inO'touch x;' foo",
            "git show --textc HEAD:f",
            "git log --show-signature",
            "git log --format=%G? -1",
            "git diff --submodule=diff",
            "printf -v PATH %s .",
            "printf -vx %s 1",
            "printf -v 'a[$(touch pwned)]' %s 1",
            "printf -v PATH %s . && ls",
            "git for-each-ref --format=%(signature)",
            "uniq - out",
            "uniq -c - out",
            "uniq -- -x out",
            "uniq *.txt",
            "uniq a?.txt",
            "uniq 'a[1].txt'",
            "echo x | uniq - out",
            "find . -name x -rm",
            "rg -z x",
            "rg --pr ./evil x",
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
