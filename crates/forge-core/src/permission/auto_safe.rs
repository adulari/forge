//! The "known safe" half of the `auto` verdict: commands whose behaviour is fixed by their name
//! and (for tool front-ends) subcommand. Anything not listed here is `Unknown` to the classifier —
//! an unfamiliar binary, a script, `make release`, `npm run deploy`, an interpreter running a
//! file — because a name we have never seen tells us nothing about what it does.
//!
//! This runs only on lines `auto::shell_risk` already cleared, so destructive flags (`rm -rf`,
//! `chmod -R`, `git reset --hard`, writes outside the workspace) never reach it.

use super::auto::{split_words, strip_wrappers};

/// Directories whose binaries are the system's own; a command named by any other path
/// (`./ls`, `/tmp/x/git`) is a script wearing a familiar name.
const SYSTEM_BIN_DIRS: &[&str] = &["/usr/bin", "/bin", "/usr/local/bin", "/opt/homebrew/bin"];

const FILE_AND_TEXT_TOOLS: &[&str] = &[
    "ls",
    "ll",
    "la",
    "cat",
    "head",
    "tail",
    "less",
    "more",
    "wc",
    "grep",
    "egrep",
    "fgrep",
    "rg",
    "ag",
    "fd",
    "fdfind",
    "echo",
    "printf",
    "pwd",
    "cd",
    "true",
    "false",
    "test",
    "[",
    "sort",
    "uniq",
    "cut",
    "tr",
    "diff",
    "cmp",
    "stat",
    "file",
    "which",
    "type",
    "whoami",
    "id",
    "date",
    "basename",
    "dirname",
    "realpath",
    "readlink",
    "tree",
    "du",
    "df",
    "uname",
    "hostname",
    "sleep",
    "jq",
    "yq",
    "column",
    "nl",
    "rev",
    "tac",
    "paste",
    "comm",
    "join",
    "fold",
    "expand",
    "od",
    "hexdump",
    "xxd",
    "md5sum",
    "sha1sum",
    "sha256sum",
    "sha512sum",
    "cksum",
    "seq",
    "mkdir",
    "touch",
    "cp",
    "mv",
    "rm",
    "rmdir",
    "ln",
    "tee",
    "chmod",
    "chown",
    "chgrp",
    "export",
    "unset",
    "ps",
    "pgrep",
    "free",
    "uptime",
    "nproc",
    "man",
    "mktemp",
    "sed",
    "rsync",
    "curl",
    "wget",
    "http",
    "https",
    "xh",
];

const DEV_TOOLS: &[&str] = &[
    "pytest",
    "ruff",
    "mypy",
    "black",
    "flake8",
    "isort",
    "pylint",
    "eslint",
    "prettier",
    "tsc",
    "jest",
    "vitest",
    "rustfmt",
    "gofmt",
    "shellcheck",
];

const SAFE_GIT: &[&str] = &[
    "status",
    "diff",
    "log",
    "show",
    "branch",
    "add",
    "commit",
    "checkout",
    "switch",
    "restore",
    "reset",
    "stash",
    "fetch",
    "pull",
    "push",
    "merge",
    "tag",
    "remote",
    "rev-parse",
    "rev-list",
    "blame",
    "ls-files",
    "ls-tree",
    "grep",
    "describe",
    "worktree",
    "shortlog",
    "cat-file",
    "show-ref",
    "mv",
    "rm",
    "init",
    "revert",
    "format-patch",
    "clean",
    "reflog",
    "version",
    "help",
];

const SAFE_CARGO: &[&str] = &[
    "build", "b", "check", "c", "test", "t", "clippy", "fmt", "doc", "d", "bench", "tree",
    "metadata", "fetch", "add", "remove", "rm", "update", "clean", "nextest", "llvm-cov", "run",
    "r", "search", "version",
];

const SAFE_GO: &[&str] = &[
    "build", "test", "vet", "fmt", "mod", "list", "get", "version", "run",
];

const SAFE_JS_PACKAGE: &[&str] = &[
    "install",
    "i",
    "ci",
    "test",
    "t",
    "ls",
    "list",
    "outdated",
    "audit",
    "view",
    "info",
    "why",
    "add",
    "remove",
    "update",
    "up",
    "build",
    "lint",
    "check",
    "typecheck",
    "type-check",
    "format",
    "fmt",
];

/// `npm run <script>` names that are conventionally build/test/lint/format steps.
const SAFE_SCRIPTS: &[&str] = &[
    "build",
    "test",
    "lint",
    "check",
    "typecheck",
    "type-check",
    "format",
    "fmt",
    "tsc",
    "prettier",
];

const SAFE_MAKE_TARGETS: &[&str] = &[
    "test", "tests", "check", "build", "lint", "fmt", "format", "clean", "all", "vet",
];

const SAFE_PYTHON_MODULES: &[&str] = &[
    "pytest",
    "unittest",
    "mypy",
    "ruff",
    "black",
    "flake8",
    "isort",
    "compileall",
    "json.tool",
];

const SAFE_DOCKER: &[&str] = &[
    "ps", "images", "logs", "inspect", "version", "info", "stats", "top",
];

const SAFE_KUBECTL: &[&str] = &[
    "get",
    "describe",
    "logs",
    "top",
    "version",
    "explain",
    "api-resources",
];

const SAFE_GH_GROUPS: &[&str] = &["pr", "issue", "run", "repo", "release", "workflow"];
const SAFE_GH_ACTIONS: &[&str] = &["view", "list", "status", "checks", "diff"];

/// Interpreters whose only safe use here is asking for their version; running a file or inline
/// code is exactly the unknown the classifier exists for.
fn is_interpreter(cmd: &str) -> bool {
    cmd.starts_with("python")
        || matches!(
            cmd,
            "node" | "nodejs" | "deno" | "bun" | "perl" | "ruby" | "php" | "osascript"
        )
}

pub(super) fn segment_is_known_safe(segment: &str) -> bool {
    let words = split_words(segment);
    let Some(words) = strip_wrappers(&words) else {
        return true;
    };
    let Some((cmd_word, rest)) = words.split_first() else {
        return true;
    };
    // `2>&1` and `&> file` split into segments that are only the tail of a redirect; the
    // redirect targets were already vetted by `shell_risk`.
    if cmd_word.starts_with('>') || cmd_word.chars().all(|c| c.is_ascii_digit()) {
        return true;
    }
    let Some(cmd) = system_command_name(cmd_word) else {
        return false;
    };
    let flags: Vec<&str> = rest
        .iter()
        .filter(|w| w.starts_with('-'))
        .map(String::as_str)
        .collect();
    let args: Vec<&str> = rest
        .iter()
        .filter(|w| !w.starts_with('-') && !w.starts_with('+'))
        .map(String::as_str)
        .collect();
    let asks_version = !flags.is_empty()
        && args.is_empty()
        && flags
            .iter()
            .all(|f| matches!(*f, "--version" | "-V" | "-v" | "--help" | "-h"));

    if is_interpreter(cmd) {
        return python_module_is_safe(cmd, rest) || asks_version;
    }
    match cmd {
        "find" => !rest.iter().any(|w| {
            matches!(
                w.as_str(),
                "-exec" | "-execdir" | "-ok" | "-okdir" | "-fprint" | "-fprintf" | "-fls"
            )
        }),
        "awk" | "gawk" => {
            let program = rest.join(" ").to_ascii_lowercase();
            !program.contains("system") && !program.contains("\"|") && !program.contains("getline")
        }
        "git" => git_subcommand(rest).is_some_and(|(sub, tail)| git_is_safe(sub, tail)),
        "rustup" => asks_version,
        "cargo" => subcommand_in(&args, SAFE_CARGO) || asks_version,
        "go" => subcommand_in(&args, SAFE_GO) || asks_version,
        "npm" | "pnpm" | "yarn" | "bun" => js_package_is_safe(&args) || asks_version,
        "make" | "gmake" => {
            !args.is_empty()
                && args
                    .iter()
                    .all(|t| SAFE_MAKE_TARGETS.contains(t) || is_var(t))
        }
        "docker" | "podman" => subcommand_in(&args, SAFE_DOCKER) || asks_version,
        "kubectl" => subcommand_in(&args, SAFE_KUBECTL) || asks_version,
        "gh" => {
            matches!(args.as_slice(), [group, action, ..]
                if SAFE_GH_GROUPS.contains(group) && SAFE_GH_ACTIONS.contains(action))
                || matches!(args.as_slice(), ["auth", "status"])
        }
        "pip" | "pip3" => {
            matches!(args.first(), Some(&("list" | "show" | "freeze"))) || asks_version
        }
        c => FILE_AND_TEXT_TOOLS.contains(&c) || DEV_TOOLS.contains(&c),
    }
}

/// The command's own name, or `None` when it was spelled as a path that is not a system binary
/// directory (`./deploy.sh`, `/tmp/x/ls`) and so cannot be vouched for by name.
fn system_command_name(cmd_word: &str) -> Option<&str> {
    match cmd_word.rsplit_once('/') {
        None => Some(cmd_word),
        Some((dir, name)) => SYSTEM_BIN_DIRS.contains(&dir).then_some(name),
    }
}

fn is_var(arg: &str) -> bool {
    arg.split_once('=')
        .is_some_and(|(k, _)| !k.is_empty() && k.chars().all(|c| c.is_alphanumeric() || c == '_'))
}

fn subcommand_in(args: &[&str], allowed: &[&str]) -> bool {
    args.first().is_some_and(|sub| allowed.contains(sub))
}

fn js_package_is_safe(args: &[&str]) -> bool {
    let Some((sub, tail)) = args.split_first() else {
        return false;
    };
    let script_ok = |name: &str| {
        SAFE_SCRIPTS.contains(&name)
            || ["test:", "lint:", "build:", "check:", "format:"]
                .iter()
                .any(|p| name.starts_with(p))
    };
    match *sub {
        "run" | "run-script" => tail.first().is_some_and(|name| script_ok(name)),
        s => SAFE_JS_PACKAGE.contains(&s),
    }
}

fn python_module_is_safe(cmd: &str, rest: &[String]) -> bool {
    if !cmd.starts_with("python") {
        return false;
    }
    let Some(pos) = rest.iter().position(|w| w == "-m") else {
        return false;
    };
    let module = rest.get(pos + 1).map(String::as_str).unwrap_or("");
    SAFE_PYTHON_MODULES.contains(&module)
}

/// The git subcommand and its arguments, skipping git's own global options.
fn git_subcommand(rest: &[String]) -> Option<(&str, &[String])> {
    let mut i = 0;
    while i < rest.len() {
        match rest[i].as_str() {
            "-C" | "-c" | "--git-dir" | "--work-tree" => i += 2,
            w if w.starts_with('-') => i += 1,
            _ => break,
        }
    }
    let sub = rest.get(i)?.as_str();
    Some((sub, &rest[i + 1..]))
}

fn git_is_safe(sub: &str, tail: &[String]) -> bool {
    if sub == "config" {
        return tail
            .iter()
            .any(|w| matches!(w.as_str(), "--get" | "--get-all" | "--list" | "-l"));
    }
    SAFE_GIT.contains(&sub)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_script_path_is_not_vouched_for_by_its_basename() {
        assert!(segment_is_known_safe("ls -la"));
        assert!(segment_is_known_safe("/usr/bin/ls"));
        assert!(!segment_is_known_safe("./ls"));
        assert!(!segment_is_known_safe("/tmp/x/git status"));
        assert!(!segment_is_known_safe("$CMD --flag"));
    }

    #[test]
    fn tool_front_ends_are_judged_by_subcommand() {
        for c in [
            "cargo test -p forge-core",
            "cargo +nightly fmt --all",
            "npm install",
            "npm run build",
            "npm run test:unit",
            "pnpm lint",
            "make test",
            "make -j8 build",
            "go test ./...",
            "git -C sub status",
            "gh pr view 12",
            "docker ps",
            "python3 -m pytest -q",
            "node --version",
            "cargo build 2>&1",
            "> out.txt",
        ] {
            assert!(segment_is_known_safe(c), "{c}");
        }
        for c in [
            "make release",
            "make",
            "npm run deploy",
            "npx create-thing",
            "cargo xtask ship",
            "cargo install foo",
            "docker run --rm img",
            "gh pr merge 12",
            "gh api /user",
            "git config alias.x '!sh'",
            "git rebase main",
            "python3 build.py",
            "node server.js",
            "pip install foo",
        ] {
            assert!(!segment_is_known_safe(c), "{c}");
        }
    }
}
