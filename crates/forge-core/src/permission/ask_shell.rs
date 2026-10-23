//! Shell lines the Ask temper runs without a prompt, because they provably cannot write, run other
//! code or reach the network (ADR-0008, "Ask auto-allows read-only shell").
//!
//! Ask used to prompt for every shell call, `git status` and `cat calc.py` included. Claude Code
//! does not, and in the scripted TUI benchmark that was one prompt more per task. This is the
//! narrow exception: a call qualifies only when EVERY part of it is understood. Anything this file
//! cannot prove read-only returns `false`, which leaves the decision where it was (an Ask), so a
//! gap here costs a prompt and never a permission.
//!
//! What is reused: [`split_operators`] (the splitter the newline/compound audit hardened),
//! [`strip_harmless_redirects`] (the Plan-mode classifier's redirect handling), the per-binary flag
//! rules in [`shared_args_read_only`] and [`git_read_only`], and the broker's own secret globs.
//! What is stricter than Plan mode, on purpose: a smaller allow-list, no wrapper binaries (`env`,
//! `command`, `nohup`, `time`) and no path-named binaries, no `$` expansion, no subshells or
//! backgrounding, one physical line per command, a path guard over the arguments, searches that
//! stay inside the workspace, and git only in a repository whose own config runs nothing.
//!
//! This runs only after the safety floor, every deny rule, the Plan contract and every ask rule
//! have been consulted, and `PreToolUse` hooks run before the broker is asked at all, so none of
//! them can be overridden from here. Like the rest of the broker it is a floor against accidents
//! and lazy injection, not a sandbox.

use std::path::{Path, PathBuf};

use forge_types::PermissionRule;
use serde_json::Value;

use super::readonly_shell::{
    git_read_only, is_long_option, is_short_flag, shared_args_read_only, strip_harmless_redirects,
};
use super::shell_commands::split_operators;
use guard::Guard;

mod git_repo;
mod guard;

/// Binaries whose every argument form is read-only, or whose flags [`args_read_only`] vets. Short
/// on purpose: a binary earns a place by being what an agent reaches for to look, not to act.
const ALLOWED: &[&str] = &[
    "ls",
    "cat",
    "head",
    "tail",
    "wc",
    "pwd",
    "echo",
    "stat",
    "du",
    "nl",
    "tac",
    "cut",
    "tr",
    "sort",
    "uniq",
    "basename",
    "dirname",
    "realpath",
    "readlink",
    "which",
    "md5sum",
    "sha1sum",
    "sha256sum",
    "b3sum",
    "diff",
    "cmp",
    "grep",
    "egrep",
    "fgrep",
    "rg",
    "find",
    "sed",
    "git",
    "printf",
    "column",
    "rev",
    "fold",
    "expand",
    "comm",
    "paste",
    "nproc",
    "whoami",
    "uname",
    "id",
    "df",
    "file",
    "hostname",
];

const GIT_SUBCOMMANDS: &[&str] = &[
    "status",
    "diff",
    "log",
    "show",
    "branch",
    "rev-parse",
    "ls-files",
    "blame",
    "grep",
    "ls-tree",
    "rev-list",
    "shortlog",
    "describe",
    "merge-base",
    "show-ref",
    "for-each-ref",
    "name-rev",
    "tag",
    "reflog",
    "stash",
    "worktree",
];

/// Flags git accepts BEFORE the subcommand that change nothing about what runs.
const GIT_LEADING_FLAGS: &[&str] = &["--no-pager", "-P", "--no-optional-locks"];

const MAX_LINE_CHARS: usize = 4096;
const MAX_WORDS: usize = 256;

/// Whether this `shell` call may run in the Ask temper without a prompt. `workspace` is the
/// session root; `rules` the session's rules (for the opt-out setting and configured read denies).
pub(super) fn auto_allows(
    args: &Value,
    rules: &[PermissionRule],
    workspace: Option<&Path>,
) -> bool {
    auto_allows_with(
        args,
        rules,
        workspace,
        std::env::var_os("HOME").map(PathBuf::from),
    )
}

/// [`auto_allows`] with the home directory `~` means supplied, so a test can name one.
fn auto_allows_with(
    args: &Value,
    rules: &[PermissionRule],
    workspace: Option<&Path>,
    home: Option<PathBuf>,
) -> bool {
    if !forge_types::auto_allow_read_only_shell_enabled(rules) {
        return false;
    }
    let Some(base) = workspace
        .map(Path::to_path_buf)
        .or_else(|| std::env::current_dir().ok())
    else {
        return false;
    };
    let Some(cwd) = command_cwd(args, &base) else {
        return false;
    };
    // Background, poll and pty runs outlive or re-run the call; they keep their prompt.
    let set = |key: &str| args.get(key).and_then(Value::as_bool).unwrap_or(false);
    if ["background", "poll_until_exit_zero", "pty"]
        .into_iter()
        .any(set)
    {
        return false;
    }
    let Some(lines) = command_lines(args) else {
        return false;
    };
    let guard = Guard::new(rules, cwd, base, home);
    lines.iter().all(|line| line_is_read_only(line, &guard))
}

/// The command lines of a call. A `commands` batch runs each entry as a command of its own, so
/// each is judged alone; a single `command` that holds a newline is one the check refuses.
fn command_lines(args: &Value) -> Option<Vec<&str>> {
    let lines: Vec<&str> = match (args.get("command"), args.get("commands")) {
        (Some(single), None) => vec![single.as_str()?],
        (None, Some(list)) => list
            .as_array()?
            .iter()
            .map(Value::as_str)
            .collect::<Option<_>>()?,
        _ => return None,
    };
    (!lines.is_empty()).then_some(lines)
}

/// The directory the command runs in, as the shell tool resolves it.
fn command_cwd(args: &Value, base: &Path) -> Option<PathBuf> {
    match args.get("cwd") {
        None => Some(base.to_path_buf()),
        Some(Value::String(dir)) if !dir.is_empty() => {
            let dir = Path::new(dir);
            Some(if dir.is_absolute() {
                dir.to_path_buf()
            } else {
                base.join(dir)
            })
        }
        Some(_) => None,
    }
}

fn line_is_read_only(line: &str, guard: &Guard) -> bool {
    let line = line.trim();
    if line.is_empty() || line.chars().count() > MAX_LINE_CHARS {
        return false;
    }
    let Some(clean) = strip_harmless_redirects(line) else {
        return false;
    };
    if !plain_syntax(&clean) {
        return false;
    }
    let mut words_seen = 0;
    let mut segments = 0;
    for raw in split_operators(&clean) {
        let segment = raw.trim();
        if segment.is_empty() {
            continue;
        }
        let Ok(words) = shell_words::split(segment) else {
            return false;
        };
        words_seen += words.len();
        if words.is_empty()
            || words_seen > MAX_WORDS
            || !segment_is_read_only(segment, &words, guard)
        {
            return false;
        }
        segments += 1;
    }
    segments > 0
}

/// Refuse every construct the segment checks cannot see through, wherever it sits in the line.
///
/// Outside quotes: subshells and groups, command and process substitution, `$` and backtick
/// expansion, braces, backgrounding (a lone `&`; `&&` is fine) and any redirect that survived
/// [`strip_harmless_redirects`]. Inside double quotes: `$` and backtick, which still expand.
/// Inside single quotes everything is literal. Control characters (newline included) are refused
/// everywhere, and an unterminated quote is refused outright.
fn plain_syntax(line: &str) -> bool {
    let chars: Vec<char> = line.chars().collect();
    let mut quote: Option<char> = None;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_control() && c != '\t' {
            return false;
        }
        match quote {
            Some('\'') => {
                if c == '\'' {
                    quote = None;
                }
            }
            Some(_) => match c {
                '\\' => {
                    i += 1;
                    if chars.get(i).is_some_and(|n| n.is_control() && *n != '\t') {
                        return false;
                    }
                }
                '$' | '`' => return false,
                '"' => quote = None,
                _ => {}
            },
            None => match c {
                '\'' | '"' => quote = Some(c),
                '\\' => {
                    i += 1;
                    match chars.get(i) {
                        Some(n) if !(n.is_control() && *n != '\t') => {}
                        _ => return false,
                    }
                }
                '$' | '`' | '(' | ')' | '{' | '}' | '<' | '>' => return false,
                '&' => {
                    if chars.get(i + 1) == Some(&'&') {
                        i += 1;
                    } else {
                        return false;
                    }
                }
                _ => {}
            },
        }
        i += 1;
    }
    quote.is_none()
}

fn segment_is_read_only(segment: &str, words: &[String], guard: &Guard) -> bool {
    let Some((cmd, rest)) = words.split_first() else {
        return false;
    };
    // No wrapper binaries and no path-named binaries: `env`, `command`, `/usr/bin/git` are simply
    // not on the list, so they fail here without being special-cased.
    if !ALLOWED.contains(&cmd.as_str()) {
        return false;
    }
    // The command word must be written plainly. `'g'it` and `g\it` reach the same binary, but a
    // quoted command word has no honest use and is what a disguised command looks like.
    let plain = segment
        .strip_prefix(cmd.as_str())
        .is_some_and(|after| after.is_empty() || after.starts_with(char::is_whitespace));
    if !plain {
        return false;
    }
    let rest: Vec<&str> = rest.iter().map(String::as_str).collect();
    args_read_only(cmd, &rest)
        && operands_are_safe(cmd, &rest, guard)
        && (cmd != "git" || git_runs_nothing_of_its_own(&rest, guard))
}

/// Git only in a repository whose own files run nothing, and `git diff` only on operands inside
/// its work tree: given two paths of which one lies outside it, or any two outside a repository,
/// `git diff` is `git diff --no-index` and prints whatever files or trees it is handed.
fn git_runs_nothing_of_its_own(rest: &[&str], guard: &Guard) -> bool {
    let Some(repo) = guard.git_repo() else {
        return false;
    };
    if git_subcommand(rest) != Some("diff") {
        return true;
    }
    let after_subcommand = rest
        .iter()
        .copied()
        .skip_while(|word| GIT_LEADING_FLAGS.contains(word))
        .skip(1)
        .collect::<Vec<_>>();
    let operands = operand_words(&after_subcommand);
    repo.work_tree
        .as_deref()
        .is_some_and(|tree| guard.all_inside(tree, &operands))
}

fn args_read_only(cmd: &str, rest: &[&str]) -> bool {
    // `--files0-from` makes these read the names of the files to open from a file or from standard
    // input, which no path check ever sees.
    if matches!(cmd, "wc" | "du" | "sort")
        && rest.iter().any(|w| is_long_option(w, "--files0-from"))
    {
        return false;
    }
    if let Some(verdict) = shared_args_read_only(cmd, rest) {
        return verdict;
    }
    match cmd {
        "git" => {
            let mut rest = rest;
            while let Some((first, tail)) = rest.split_first() {
                if !GIT_LEADING_FLAGS.contains(first) {
                    break;
                }
                rest = tail;
            }
            // `--no-index` makes `git diff` and `git grep` read two arbitrary paths, or a whole
            // tree, with no repository around them.
            git_read_only(rest, GIT_SUBCOMMANDS)
                && !rest.iter().any(|w| is_long_option(w, "--no-index"))
        }
        "sed" => sed_prints_a_line_range(rest),
        // `tail -f` never returns, so the call would sit out its timeout with nothing to answer.
        "tail" => !rest.iter().any(|w| {
            is_long_option(w, "--follow") || is_short_flag(w, 'f') || is_short_flag(w, 'F')
        }),
        // `diff -l` pipes its output through `pr`; `--from-file` and `--to-file` compare one
        // operand against many, directories included, before the operands are looked at.
        "diff" => !rest.iter().any(|w| {
            is_long_option(w, "--paginate")
                || is_long_option(w, "--from-file")
                || is_long_option(w, "--to-file")
                || is_short_flag(w, 'l')
        }),
        // `--check` hashes the files a list names and says which differ: a way to test a guess
        // against a secret without the path ever being on the command line.
        "md5sum" | "sha1sum" | "sha256sum" | "b3sum" => !rest
            .iter()
            .any(|w| is_short_flag(w, 'c') || is_long_option(w, "--check")),
        // Setting the host name is a positional argument; only the lookups are accepted.
        "hostname" => rest.iter().all(|w| {
            matches!(
                *w,
                "-s" | "-f"
                    | "-d"
                    | "-i"
                    | "-I"
                    | "-A"
                    | "--short"
                    | "--fqdn"
                    | "--long"
                    | "--domain"
                    | "--ip-address"
                    | "--all-ip-addresses"
                    | "--all-fqdns"
            )
        }),
        // `file -C` compiles a magic file, writing it; `-m` and `-f` read files nothing vetted.
        "file" => !rest.iter().any(|w| {
            ['C', 'm', 'f'].iter().any(|c| is_short_flag(w, *c))
                || is_long_option(w, "--compile")
                || is_long_option(w, "--magic-file")
                || is_long_option(w, "--files-from")
        }),
        _ => true,
    }
}

/// `sed -n 'N,Mp' [file...]` and nothing else. sed's script language writes (`w`), runs (`e`) and
/// reads (`r`) files, and a script cannot be vetted by inspection here, so the only script allowed
/// is a bare line-range print and the only flag is `-n`.
fn sed_prints_a_line_range(rest: &[&str]) -> bool {
    let [flag, script, files @ ..] = rest else {
        return false;
    };
    matches!(*flag, "-n" | "--quiet" | "--silent")
        && is_line_range_print(script)
        && files.iter().all(|f| !f.starts_with('-'))
}

fn is_line_range_print(script: &str) -> bool {
    let number = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    let Some(range) = script.strip_suffix('p') else {
        return false;
    };
    match range.split_once(',') {
        None => number(range) || range == "$",
        Some((from, to)) => number(from) && (number(to) || to == "$"),
    }
}

/// The words a command takes as files, not as options: everything that does not start with `-`
/// (a lone `-` included), and every word after `--` whatever it starts with. A file named `-esc`
/// is an operand after `--`, and a symlink so named can lead a recursive search out of the
/// workspace.
fn operand_words<'a>(words: &[&'a str]) -> Vec<&'a str> {
    let mut after_dashes = false;
    let mut operands = Vec::new();
    for &word in words {
        if after_dashes {
            operands.push(word);
        } else if word == "--" {
            after_dashes = true;
        } else if word == "-" || !word.starts_with('-') {
            operands.push(word);
        }
    }
    operands
}

fn operands_are_safe(cmd: &str, rest: &[&str], guard: &Guard) -> bool {
    if !rest.iter().all(|word| guard.word_is_safe(word)) {
        return false;
    }
    let operands = operand_words(rest);
    match recursive_search(cmd, rest, &operands, guard) {
        None => true,
        Some(search) => guard.search_is_safe(&operands, search == Search::SkipsHidden),
    }
}

#[derive(PartialEq, Eq)]
enum Search {
    /// Walks trees but leaves hidden and ignored files alone (plain `rg`).
    SkipsHidden,
    /// Reads every file it meets (`grep -r`, `diff -r`, `rg --hidden`).
    Everything,
}

/// Whether the command walks directory trees reading file contents, and how far it looks.
fn recursive_search(cmd: &str, rest: &[&str], operands: &[&str], guard: &Guard) -> Option<Search> {
    let long = |full: &str| rest.iter().any(|w| is_long_option(w, full));
    let short = |c: char| rest.iter().any(|w| is_short_flag(w, c));
    let recursive = short('r') || short('R') || long("--recursive");
    match cmd {
        "rg" => Some(if rg_reads_hidden_files(rest) {
            Search::Everything
        } else {
            Search::SkipsHidden
        }),
        "grep" | "egrep" | "fgrep" => {
            (recursive || long("--dereference-recursive") || directories_recurse(rest))
                .then_some(Search::Everything)
        }
        // Two directories are compared file by file without `-r` too, and a changed file is printed.
        "diff" => (recursive || operands.iter().any(|w| guard.names_directory(w)))
            .then_some(Search::Everything),
        // `git grep` reads tracked files; `--untracked` makes it read the working tree instead.
        "git" => (git_subcommand(rest) == Some("grep") && long("--untracked"))
            .then_some(Search::Everything),
        _ => None,
    }
}

/// The git subcommand: the first word that is not one of the flags allowed before it.
fn git_subcommand<'a>(rest: &[&'a str]) -> Option<&'a str> {
    rest.iter()
        .copied()
        .find(|word| !GIT_LEADING_FLAGS.contains(word))
}

/// `grep -d recurse`, `--directories=recurse`, `--directories recurse`, `-drecurse`, and the
/// abbreviations GNU grep takes of the action (`-d rec`). Only `read` and `skip` are known not to
/// recurse, so any other action counts.
fn directories_recurse(rest: &[&str]) -> bool {
    let recurses = |action: &str| !matches!(action, "read" | "skip");
    rest.iter().enumerate().any(|(at, word)| {
        let next = rest.get(at + 1).copied();
        if is_long_option(word, "--directories") {
            word.split_once('=')
                .map_or(next, |(_, value)| Some(value))
                .is_some_and(recurses)
        } else if is_short_flag(word, 'd') {
            let attached = word[1..].split_once('d').map_or("", |(_, tail)| tail);
            if attached.is_empty() {
                next.is_some_and(recurses)
            } else {
                recurses(attached)
            }
        } else {
            false
        }
    })
}

/// `rg` skips hidden files and anything ignored unless told otherwise. `-g/--glob` is "told
/// otherwise": it overrides every ignore rule, so `rg -g .env KEY` reads the dotenv file.
fn rg_reads_hidden_files(rest: &[&str]) -> bool {
    rest.iter().any(|w| {
        matches!(
            *w,
            "--hidden" | "--unrestricted" | "--glob" | "--iglob" | "-."
        ) || w.starts_with("--no-ignore")
            || w.starts_with("--glob=")
            || w.starts_with("--iglob=")
            || ['u', 'g', '.'].iter().any(|c| is_short_flag(w, *c))
    })
}

#[cfg(test)]
mod tests;
