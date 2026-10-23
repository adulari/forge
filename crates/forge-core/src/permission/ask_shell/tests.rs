use std::os::unix::fs::symlink;

use forge_types::{PermissionDecision, PermissionMode, PermissionRule, SideEffect};
use serde_json::json;
use tempfile::TempDir;

use super::*;
use crate::permission::{decide_in, denial_note};

/// A workspace and a separate home directory under one parent, so `..` from the workspace reaches
/// an ancestor of home.
struct Fixture {
    _parent: TempDir,
    root: PathBuf,
    home: PathBuf,
}

fn fixture() -> Fixture {
    let parent = tempfile::tempdir().unwrap();
    let base = std::fs::canonicalize(parent.path()).unwrap();
    let root = base.join("ws");
    let home = base.join("home");
    for dir in [
        root.join("src"),
        root.join("config"),
        home.join(".ssh"),
        home.join("docs"),
    ] {
        std::fs::create_dir_all(dir).unwrap();
    }
    let files = [
        (root.join("calc.py"), "def add(a, b):\n    return a - b\n"),
        (root.join("src/lib.rs"), "pub fn add() {}\n"),
        (root.join("src/main.rs"), "fn main() {}\n"),
        (root.join(".env"), "API_KEY=hunter2\n"),
        (root.join(".env.example"), "API_KEY=\n"),
        (root.join("config/.env.production"), "API_KEY=prod\n"),
        (root.join("server.pem"), "-----BEGIN-----\n"),
        (home.join(".ssh/id_rsa"), "-----BEGIN OPENSSH-----\n"),
        (home.join("docs/notes.txt"), "plain notes\n"),
    ];
    for (path, body) in files {
        std::fs::write(path, body).unwrap();
    }
    write_repo(&root, INERT_CONFIG);
    symlink(root.join(".env"), root.join("notes.txt")).unwrap();
    symlink(home.join("docs"), root.join("escape")).unwrap();
    std::fs::create_dir_all(root.join("deep/a/b")).unwrap();
    std::fs::write(root.join("deep/a/b/.env"), "TOKEN=1\n").unwrap();
    std::fs::write(root.join("deep/a/readme.txt"), "hi\n").unwrap();
    std::fs::create_dir_all(root.join("linked")).unwrap();
    symlink(root.join(".env"), root.join("linked/config.txt")).unwrap();
    Fixture {
        _parent: parent,
        root,
        home,
    }
}

/// What `git clone` leaves in `.git/config`, which runs nothing.
const INERT_CONFIG: &str =
    "[core]\n\trepositoryformatversion = 0\n\tfilemode = true\n\tbare = false\n\
[remote \"origin\"]\n\turl = https://example.invalid/calc.git\n\
\tfetch = +refs/heads/*:refs/remotes/origin/*\n[branch \"main\"]\n\tremote = origin\n\
\tmerge = refs/heads/main\n";

/// A directory git takes for a repository: `<dir>/.git` with the files discovery looks for.
fn write_repo(dir: &Path, config: &str) {
    let git = dir.join(".git");
    std::fs::create_dir_all(git.join("objects")).unwrap();
    std::fs::create_dir_all(git.join("refs")).unwrap();
    std::fs::write(git.join("HEAD"), "ref: refs/heads/main\n").unwrap();
    std::fs::write(git.join("config"), config).unwrap();
}

fn rules() -> Vec<PermissionRule> {
    forge_config::Config::default().permission_rules()
}

/// Whether the Ask auto-allow would run `args`, judged from the fixture workspace.
fn allows_args(f: &Fixture, args: serde_json::Value) -> bool {
    auto_allows_with(&args, &rules(), Some(&f.root), Some(f.home.clone()))
}

fn allows(f: &Fixture, command: &str) -> bool {
    allows_args(f, json!({ "command": command }))
}

/// Commands an agent runs to look at a tree, which must not prompt in Ask.
const ALLOWED: &[&str] = &[
    "git status",
    "git status --short --branch",
    "git status --porcelain=v1",
    "git diff",
    "git diff --stat HEAD~1",
    "git diff --cached",
    "git diff main...HEAD -- calc.py",
    "git log --oneline -5",
    "git log -p -n 3 -- calc.py",
    "git log --format='%h (%an) %s' -3",
    "git show HEAD",
    "git show HEAD:calc.py",
    "git branch",
    "git branch -a",
    "git branch --show-current",
    "git rev-parse --show-toplevel",
    "git ls-files",
    "git blame calc.py",
    "git grep -n add",
    "git --no-pager log -1",
    "ls",
    "ls -la src",
    "ls | wc -l",
    "ls ~",
    "cat calc.py",
    "cat src/lib.rs src/main.rs",
    "cat src/lib.rs | head -20",
    "cat .env.example",
    "grep -rn add src",
    "grep -rn \"fn main\" src | head",
    "grep -n \"a (b)\" calc.py",
    "grep -n 'a & b' calc.py",
    "grep -rn add src 2>/dev/null",
    "grep -rn \"name=\" src",
    "rg 'a=b' src",
    "ls FOO=1",
    "rg -n add src",
    "rg 'foo|bar' src",
    "wc -l calc.py",
    "wc -l src/*.rs",
    "head -n 5 calc.py",
    "tail -n 5 calc.py",
    "find . -name '*.py' -type f",
    "find src -type f | sort",
    "sort calc.py | uniq -c",
    "ls && git status",
    "ls; pwd",
    "git diff 2>&1 | head",
    "ls >&2",
    "ls 2>&1 | wc -l",
    "ls 2>&-",
    "git status > /dev/null",
    "echo hello",
    "stat calc.py",
    "du -sh src",
    "diff calc.py calc.py",
    "sed -n '1,5p' calc.py",
    "sed -n 5p calc.py",
    "sed -n '5,$p' calc.py",
    "git log | sed -n '1,10p'",
    "basename src/lib.rs",
    "git status --short ; git diff --stat",
    "git diff --text",
    "git log --text -1",
    "git tag",
    "git tag -l 'v*'",
    "git tag --list",
    "git for-each-ref --format='%(refname)'",
    "git name-rev HEAD",
    "git reflog",
    "git reflog show -3",
    "git stash list",
    "git worktree list",
    "git worktree list --porcelain",
    "rg --files src | wc -l",
    "printf '%s\\n' b a | sort",
    "whoami",
    "uname -a",
    "id",
    "id -un",
    "df -h",
    "hostname",
    "hostname --fqdn",
    "nproc",
    "file calc.py",
    "file -b src/lib.rs",
    "column -t calc.py",
    "rev calc.py",
    "fold -w 20 calc.py",
    "expand calc.py",
    "comm calc.py calc.py",
    "paste calc.py calc.py",
    "uniq calc.py",
    "uniq -c calc.py",
    "sort calc.py | uniq - ",
    "git diff calc.py src/lib.rs",
    "sha256sum calc.py",
    "md5sum -b calc.py",
    "cat calc.py | uniq -",
    "diff src src",
];

/// Every way a command can write, run other code, reach out, or be mistaken for something else,
/// each with the class it belongs to.
const REFUSED: &[(&str, &str)] = &[
    ("redirect", "ls > out.txt"),
    ("redirect", "ls >> out.txt"),
    ("redirect", "ls &> out.txt"),
    ("redirect", "ls &>/dev/null"),
    ("redirect", "ls >| out.txt"),
    ("redirect", "git diff > out"),
    ("redirect", "cat calc.py > copy.py"),
    ("redirect", "ls 2> err.log"),
    ("redirect", "ls 1>out"),
    ("redirect", "ls >&out"),
    ("redirect", "ls >&1out"),
    ("redirect", "ls >&2err"),
    ("redirect", "ls >&-x"),
    ("redirect", "ls 2>&1out"),
    ("redirect", "echo hi >&1file"),
    ("redirect", "ls >&1>out"),
    ("redirect", "ls > /dev/null/../x"),
    ("redirect", "cat < calc.py"),
    ("redirect", "cat <<EOF"),
    ("redirect", "cat <<< hi"),
    ("substitution", "cat <(rm x)"),
    ("substitution", "ls >(cat)"),
    ("substitution", "echo $(rm x)"),
    ("substitution", "echo `rm x`"),
    ("substitution", "ls $(pwd)"),
    ("substitution", "ls \"$(pwd)\""),
    ("substitution", "ls $HOME"),
    ("substitution", "ls ${HOME}"),
    ("substitution", "ls \"$HOME\""),
    ("substitution", "echo $((1+1))"),
    ("compound", "ls; rm -rf x"),
    ("compound", "ls && touch x"),
    ("compound", "ls || rm x"),
    ("compound", "git status; git push"),
    ("compound", "ls | sh"),
    ("compound", "ls | bash"),
    ("compound", "git log | sh"),
    ("compound", "grep -r x . | xargs rm"),
    ("compound", "ls | tee f"),
    ("compound", "cat calc.py | python"),
    ("backgrounding", "ls &"),
    ("backgrounding", "ls & touch x"),
    ("backgrounding", "ls |& cat"),
    ("subshell", "(ls)"),
    ("subshell", "(ls; rm x)"),
    ("subshell", "{ ls; }"),
    ("subshell", "ls; (rm x)"),
    ("subshell", "! ls"),
    ("env prefix", "FOO=1 ls"),
    ("env prefix", "GIT_EXTERNAL_DIFF=x git diff"),
    ("env prefix", "LD_PRELOAD=x.so ls"),
    ("wrapper", "env git status"),
    ("wrapper", "env FOO=1 git status"),
    ("wrapper", "command git status"),
    ("wrapper", "builtin pwd"),
    ("wrapper", "nohup ls"),
    ("wrapper", "time ls"),
    ("wrapper", "exec ls"),
    ("wrapper", "nice ls"),
    ("wrapper", "xargs ls"),
    ("absolute binary", "/usr/bin/git status"),
    ("absolute binary", "/bin/ls"),
    ("absolute binary", "./git status"),
    ("absolute binary", "../bin/ls"),
    ("absolute binary", "~/bin/ls"),
    ("quoting", "'g'it status"),
    ("quoting", "g\\it status"),
    ("quoting", "\"git\" status"),
    ("quoting", "gi\"\"t status"),
    ("quoting", "'r'm x"),
    ("quoting", "'g'it push"),
    ("unicode", "\u{ff47}it status"),
    ("unicode", "g\u{0456}t status"),
    ("unicode", "git\u{a0}status"),
    ("unicode", "ls\u{200b}"),
    ("unicode", "git\u{2028}status"),
    ("control character", "ls\u{7}"),
    ("control character", "ls\u{1b}[0m"),
    ("newline", "ls\nrm x"),
    ("newline", "git status\ngit push"),
    ("newline", "ls\ncat calc.py"),
    ("newline", "ls\r\nls"),
    ("git global option", "git -C dir diff"),
    ("git global option", "git --git-dir=x log"),
    ("git global option", "git -c core.pager='sh -c x' log"),
    ("git global option", "git -c core.fsmonitor=x status"),
    ("git global option", "git --exec-path=/tmp log"),
    ("git helper or output", "git log --output=x"),
    ("git helper or output", "git diff --output x"),
    ("git helper or output", "git diff --ext-diff"),
    ("git helper or output", "git show --textconv HEAD:calc.py"),
    ("git helper or output", "git grep -O'sh -c x' foo"),
    (
        "git helper or output",
        "git grep --open-files-in-pager=sh foo",
    ),
    ("git helper or output", "git log --exec=x"),
    ("git helper or output", "git diff --config-env=a=b"),
    ("git write", "git commit -m x"),
    ("git write", "git push"),
    ("git write", "git checkout main"),
    ("git write", "git branch -D x"),
    ("git write", "git branch -m a b"),
    ("git write", "git branch newbranch"),
    ("git write", "git config user.name x"),
    ("git write", "git stash"),
    ("git write", "git reset --hard"),
    ("git write", "git fetch"),
    ("git write", "git clean -fd"),
    ("git write", "git"),
    ("git leaks credentials", "git remote -v"),
    ("git leaks credentials", "git remote"),
    ("find", "find . -name x -delete"),
    ("find", "find . -exec rm {} \\;"),
    ("find", "find . -execdir rm {} +"),
    ("find", "find . -ok rm {} \\;"),
    ("find", "find . -okdir rm {} \\;"),
    ("find", "find . -fprint out"),
    ("find", "find . -fprint0 out"),
    ("find", "find . -fprintf out x"),
    ("find", "find . -fls out"),
    ("sed or awk", "sed -i s/a/b/ calc.py"),
    ("sed or awk", "sed -n 1w out calc.py"),
    ("sed or awk", "sed -n e calc.py"),
    ("sed or awk", "sed s/a/b/ calc.py"),
    ("sed or awk", "sed -n 1,5p -i calc.py"),
    ("sed or awk", "sed -n -e 1p calc.py"),
    ("sed or awk", "sed -n '/x/p' calc.py"),
    ("sed or awk", "sed -n 5p -s calc.py"),
    ("sed or awk", "awk '{print}' calc.py"),
    ("sed or awk", "awk 'BEGIN{system(\"x\")}'"),
    ("flag that writes or runs", "sort -o out calc.py"),
    ("flag that writes or runs", "sort calc.py -oout"),
    ("flag that writes or runs", "sort --output=out calc.py"),
    (
        "flag that writes or runs",
        "sort --compress-program=sh calc.py",
    ),
    ("flag that writes or runs", "sort -T /tmp calc.py"),
    ("flag that writes or runs", "uniq calc.py out"),
    ("flag that writes or runs", "tree -o out"),
    ("flag that writes or runs", "rg --pre ./evil x"),
    ("flag that writes or runs", "rg --pre=./evil x"),
    ("flag that writes or runs", "rg -z x"),
    ("flag that writes or runs", "rg --search-zip x"),
    ("flag that writes or runs", "tail -f calc.py"),
    ("flag that writes or runs", "tail -F calc.py"),
    ("flag that writes or runs", "tail --follow calc.py"),
    ("flag that writes or runs", "diff -l calc.py calc.py"),
    ("not on the list", "less calc.py"),
    ("not on the list", "more calc.py"),
    ("not on the list", "man ls"),
    ("not on the list", "tee f"),
    ("not on the list", "curl http://example.com"),
    ("not on the list", "wget http://example.com"),
    ("not on the list", "npm test"),
    ("not on the list", "cargo build"),
    ("not on the list", "make"),
    ("not on the list", "python -c 'print(1)'"),
    ("not on the list", "bash -c ls"),
    ("not on the list", "sh -c ls"),
    ("not on the list", "eval ls"),
    ("not on the list", "cd src && ls"),
    ("not on the list", "source x"),
    ("not on the list", ". x"),
    ("not on the list", "jq . calc.py"),
    ("not on the list", "jq -n env"),
    ("not on the list", "date"),
    ("not on the list", "env"),
    ("not on the list", "printenv"),
    ("not on the list", "ps aux"),
    ("not on the list", "touch x"),
    ("not on the list", "rm x"),
    ("not on the list", "cp calc.py x"),
    ("not on the list", "mv calc.py x"),
    ("not on the list", "mkdir x"),
    ("not on the list", "chmod +x calc.py"),
    ("not on the list", "ln -s a b"),
    ("not on the list", "dd if=a of=b"),
    ("not on the list", "ssh host"),
    ("not on the list", "nc host 80"),
    ("malformed", "ls 'unterminated"),
    ("malformed", "ls \"unterminated"),
    ("malformed", "ls \\"),
    ("malformed", ""),
    ("malformed", "   "),
    ("secret", "cat .env"),
    ("secret", "cat ./.env"),
    ("secret", "cat .env.production"),
    ("secret", "cat config/.env.production"),
    ("secret", "cat ../ws/.env"),
    ("secret", "tail .env"),
    ("secret", "wc -c .env"),
    ("secret", "head -c 5 .env"),
    ("secret", "cat server.pem"),
    ("secret", "sha256sum server.pem"),
    ("secret", "git show HEAD:.env"),
    ("secret", "git log -p -- .env"),
    ("secret", "grep -n KEY .env"),
    ("secret", "grep --file=.env calc.py"),
    ("secret", "grep -f.env calc.py"),
    ("secret", "cat notes.txt"),
    ("secret", "cat .e*"),
    ("secret", "cat .env*"),
    ("secret", "cat *.pem"),
    ("secret", "cat /proc/self/environ"),
    ("secret", "cat /proc/*/environ"),
    ("secret", "cat ~/.ssh/id_rsa"),
    ("secret", "cat ~/.ssh/id_*"),
    ("secret", "ls ~/.ssh"),
    ("secret", "cat ~root/.bashrc"),
    ("secret", "grep -r x .."),
    ("secret", "grep -rn x ~"),
    ("secret", "rg token ~"),
    ("secret", "rg token /"),
    ("secret", "diff -r . ~"),
    ("secret", "cat .ENV"),
    ("secret", "cat ID_RSA"),
    ("secret", "git log -p -- ':(top).env'"),
    ("secret", "git log -p -- ':(icase).ENV'"),
    ("secret", "git show HEAD:.ENV"),
    ("secret in a searched tree", "grep -rn add ."),
    ("secret in a searched tree", "grep -rn add"),
    ("secret in a searched tree", "grep -R add ."),
    (
        "secret in a searched tree",
        "grep -r add --include='*.py' .",
    ),
    ("secret in a searched tree", "grep --recursive add ."),
    ("secret in a searched tree", "grep -d recurse add ."),
    ("secret in a searched tree", "grep -drecurse add ."),
    (
        "secret in a searched tree",
        "grep --directories recurse add .",
    ),
    (
        "secret in a searched tree",
        "grep --directories=recurse add .",
    ),
    ("secret in a searched tree", "grep --dir=recurse add ."),
    ("secret in a searched tree", "grep --recur add ."),
    ("secret in a searched tree", "git grep --no-i KEY"),
    ("secret in a searched tree", "git grep --untr KEY"),
    ("wildcard fan-out", "cat */*/*/*/x"),
    ("secret in a searched tree", "grep -rn add config"),
    ("secret in a searched tree", "grep -rn add deep"),
    ("secret in a searched tree", "grep -rn add src config/.."),
    ("secret in a searched tree", "grep -rn add linked"),
    ("secret in a searched tree", "grep -rn add s*"),
    ("secret in a searched tree", "diff -r . src"),
    ("secret in a searched tree", "rg --hidden add"),
    ("secret in a searched tree", "rg -uu add"),
    ("secret in a searched tree", "rg -u add ."),
    ("secret in a searched tree", "rg --no-ignore add"),
    ("secret in a searched tree", "rg -g .env KEY"),
    ("secret in a searched tree", "rg --glob '*' KEY"),
    ("secret in a searched tree", "rg -. KEY"),
    ("secret in a searched tree", "git grep --untracked KEY"),
    ("secret in a searched tree", "git grep --no-index KEY"),
    ("not on the list", "tree -L 2"),
    // The reviewers' bypasses, each as the command that proved it.
    ("uniq writes its second operand", "uniq - out"),
    ("uniq writes its second operand", "uniq *.txt"),
    ("uniq writes its second operand", "uniq src/*.rs"),
    ("uniq writes its second operand", "uniq calc.p?"),
    ("uniq writes its second operand", "uniq calc.p[y]"),
    ("uniq writes its second operand", "uniq -c - out"),
    ("uniq writes its second operand", "cat calc.py | uniq - out"),
    ("uniq writes its second operand", "echo hi | uniq - out"),
    ("uniq writes its second operand", "uniq -- -x out"),
    ("uniq writes its second operand", "uniq - ~/.bashrc"),
    ("uniq writes its second operand", "uniq - .git/config"),
    ("uniq writes its second operand", "uniq - package.json"),
    ("uniq writes its second operand", "uniq -f 1 calc.py out"),
    ("clustered git pager", "git grep -nO'touch /tmp/pwned;' add"),
    ("clustered git pager", "git grep -nOsh add"),
    ("clustered git pager", "git grep -inOsh add"),
    ("clustered git pager", "git grep -n -Osh add"),
    ("clustered git pager", "git grep -O sh add"),
    (
        "git reads outside the repository",
        "git diff --no-index /var/empty .",
    ),
    (
        "git reads outside the repository",
        "git diff --no-index calc.py src/lib.rs",
    ),
    (
        "git reads outside the repository",
        "git diff --no-ind calc.py src/lib.rs",
    ),
    (
        "git reads outside the repository",
        "git --no-pager diff --no-index calc.py src/lib.rs",
    ),
    (
        "git reads outside the repository",
        "git grep --no-index add src",
    ),
    ("git signature check", "git log --show-signature"),
    ("git signature check", "git log --show-sig -1"),
    ("git signature check", "git show --show-signature HEAD"),
    ("git signature check", "git log --format=%G? -1"),
    ("git signature check", "git log --pretty=format:%GS"),
    ("git signature check", "git log '--pretty=%h %GK'"),
    ("git reads through a submodule", "git diff --submodule=diff"),
    ("git reads through a submodule", "git log -p --submodule"),
    (
        "git reads through a submodule",
        "git grep --recurse-submodules add",
    ),
    ("git write", "git tag v1"),
    ("git write", "git tag -d v1"),
    ("git write", "git tag -l -f v1"),
    ("git write", "git tag -a v1 -m x"),
    ("git write", "git stash pop"),
    ("git write", "git stash drop"),
    ("git write", "git stash push"),
    ("git write", "git reflog expire --all"),
    ("git write", "git reflog delete HEAD@{1}"),
    ("git write", "git worktree add ../x"),
    ("git write", "git worktree remove x"),
    ("directories are searched without -r", "diff . src"),
    ("directories are searched without -r", "diff src .."),
    ("directories are searched without -r", "diff config deep"),
    (
        "directories are searched without -r",
        "diff --from-file=src calc.py",
    ),
    (
        "directories are searched without -r",
        "diff --to-file src calc.py",
    ),
    (
        "a plain rg reads every file it is not told to skip",
        "rg -N .",
    ),
    (
        "a plain rg reads every file it is not told to skip",
        "rg BEGIN",
    ),
    (
        "a plain rg reads every file it is not told to skip",
        "rg -n add",
    ),
    (
        "a plain rg reads every file it is not told to skip",
        "rg --files",
    ),
    (
        "a plain rg reads every file it is not told to skip",
        "rg -L hunter2",
    ),
    (
        "a plain rg reads every file it is not told to skip",
        "rg --follow hunter2",
    ),
    (
        "a plain rg reads every file it is not told to skip",
        "rg --foll hunter2 .",
    ),
    (
        "grep abbreviates the action it recurses with",
        "grep -d rec -h KEY .",
    ),
    (
        "grep abbreviates the action it recurses with",
        "grep -drec -h KEY .",
    ),
    (
        "grep abbreviates the action it recurses with",
        "grep -nd rec KEY .",
    ),
    (
        "grep abbreviates the action it recurses with",
        "grep --directories=rec -h KEY .",
    ),
    (
        "grep abbreviates the action it recurses with",
        "grep --dir=rec -h KEY .",
    ),
    (
        "grep abbreviates the action it recurses with",
        "grep --directories rec KEY .",
    ),
    (
        "grep abbreviates the action it recurses with",
        "grep -d re KEY .",
    ),
    (
        "grep abbreviates the action it recurses with",
        "egrep -d rec KEY .",
    ),
    ("names read from a file", "wc --files0-from=calc.py"),
    ("names read from a file", "wc --files0-from=-"),
    ("names read from a file", "sort --files0-from=-"),
    ("names read from a file", "sort --files0-from=calc.py"),
    ("names read from a file", "du --files0-from=calc.py"),
    (
        "names read from a file",
        "printf '.env\\0' | sort --files0=-",
    ),
    ("find", "find . -name x -rm"),
    (
        "git diff outside the work tree is a no-index diff",
        "git diff calc.py ../home/docs/notes.txt",
    ),
    (
        "git diff outside the work tree is a no-index diff",
        "git diff /var/empty .",
    ),
    (
        "git diff outside the work tree is a no-index diff",
        "git diff ~/docs/notes.txt calc.py",
    ),
    (
        "git diff outside the work tree is a no-index diff",
        "git diff -- ../home/docs/notes.txt calc.py",
    ),
    (
        "git diff outside the work tree is a no-index diff",
        "git diff escape/notes.txt calc.py",
    ),
    (
        "git diff outside the work tree is a no-index diff",
        "git diff /etc/hostname calc.py",
    ),
    (
        "git diff outside the work tree is a no-index diff",
        "git --no-pager diff /var/empty .",
    ),
    (
        "a checksum list is a guessing oracle",
        "sha256sum -c calc.py",
    ),
    (
        "a checksum list is a guessing oracle",
        "md5sum --check calc.py",
    ),
    (
        "a checksum list is a guessing oracle",
        "sha1sum -bc calc.py",
    ),
    (
        "a checksum list is a guessing oracle",
        "b3sum --che calc.py",
    ),
    ("secret", "cat /proc/self/cmdline"),
    ("host name set", "hostname newname"),
    ("host name set", "hostname -F calc.py"),
    ("file compiles a magic file", "file -C"),
    ("file compiles a magic file", "file -m magic calc.py"),
    ("file compiles a magic file", "file -f calc.py"),
    ("file compiles a magic file", "file --compile"),
    ("a symlink out of the searched tree", "grep -rn add escape"),
    ("a symlink out of the searched tree", "grep -R add escape"),
];

#[test]
fn auto_allows_what_only_looks() {
    let f = fixture();
    for command in ALLOWED {
        assert!(allows(&f, command), "should auto-allow: {command:?}");
    }
}

#[test]
fn refuses_everything_that_can_write_run_or_leak() {
    let f = fixture();
    for (class, command) in REFUSED {
        assert!(!allows(&f, command), "{class}: should refuse {command:?}");
    }
}

#[test]
fn the_tables_are_big_enough_to_mean_something() {
    assert!(ALLOWED.len() + REFUSED.len() >= 60);
    let classes: std::collections::BTreeSet<&str> = REFUSED.iter().map(|(c, _)| *c).collect();
    for needed in [
        "redirect",
        "substitution",
        "compound",
        "backgrounding",
        "subshell",
        "env prefix",
        "wrapper",
        "absolute binary",
        "quoting",
        "unicode",
        "newline",
        "git global option",
        "find",
        "sed or awk",
        "not on the list",
        "secret",
    ] {
        assert!(classes.contains(needed), "no refused case for {needed}");
    }
}

#[test]
fn a_commands_batch_is_judged_entry_by_entry() {
    let f = fixture();
    assert!(allows_args(
        &f,
        json!({ "commands": ["git status --short", "git diff --stat", "ls"] })
    ));
    assert!(!allows_args(
        &f,
        json!({ "commands": ["git status", "touch x"] })
    ));
    assert!(!allows_args(&f, json!({ "commands": ["ls", "cat .env"] })));
    assert!(!allows_args(&f, json!({ "commands": [] })));
    assert!(!allows_args(&f, json!({ "commands": ["ls\nrm x"] })));
    assert!(!allows_args(&f, json!({ "commands": [1] })));
    assert!(!allows_args(&f, json!({ "commands": "ls" })));
    // The tool refuses both keys together, so the broker must not pick one of them.
    assert!(!allows_args(
        &f,
        json!({ "command": "ls", "commands": ["rm x"] })
    ));
    assert!(!allows_args(&f, json!({})));
    assert!(!allows_args(&f, json!({ "command": 7 })));
}

#[test]
fn background_poll_and_pty_runs_keep_their_prompt() {
    let f = fixture();
    for key in ["background", "poll_until_exit_zero", "pty"] {
        assert!(
            !allows_args(&f, json!({ "command": "git status", key: true })),
            "{key}"
        );
        assert!(
            allows_args(&f, json!({ "command": "git status", key: false })),
            "{key}: false"
        );
    }
}

#[test]
fn relative_paths_resolve_against_the_commands_working_directory() {
    let f = fixture();
    // In `config/`, `.env.production` is the secret; in the workspace root it does not exist and
    // names nothing the secret rules cover.
    let in_config = json!({ "command": "cat .env.production", "cwd": "config" });
    assert!(!allows_args(&f, in_config));
    let in_src = json!({ "command": "cat main.rs", "cwd": "src" });
    assert!(allows_args(&f, in_src));
    // `../notes.txt` is a symlink to `.env` from `src/`, and names nothing from the root.
    assert!(!allows_args(
        &f,
        json!({ "command": "cat ../notes.txt", "cwd": "src" })
    ));
    assert!(allows_args(&f, json!({ "command": "cat ../notes.txt" })));
    assert!(!allows_args(
        &f,
        json!({ "command": "cat .env", "cwd": f.root.to_string_lossy() })
    ));
    assert!(!allows_args(&f, json!({ "command": "ls", "cwd": 5 })));
}

#[test]
fn a_wildcard_is_judged_by_the_files_it_expands_to() {
    let f = fixture();
    assert!(allows(&f, "cat src/*.rs"));
    assert!(allows(&f, "cat calc.*"));
    assert!(!allows(&f, "cat *.pem"));
    assert!(!allows(&f, "cat config/.env.*"));
    // `sh` reads `**` as `*`; refusing it keeps a recursive expansion off the broker's hot path.
    assert!(!allows(&f, "cat **/*.rs"));
    // `sh` does not match a hidden file with a leading wildcard, so these read nothing secret:
    // `config/` holds only `.env.production`.
    assert!(allows(&f, "cat *.py"));
    assert!(allows(&f, "cat config/*"));
    assert!(!allows(&f, "cat config/.*"));
    assert!(!allows(&f, "cat config/.env.p*"));
    assert!(!allows(&f, "cat config/.e[n]v.production"));
}

#[test]
fn a_wildcard_that_expands_to_an_option_is_refused() {
    let f = fixture();
    let bait = f.root.join("bait");
    std::fs::create_dir(&bait).unwrap();
    std::fs::write(bait.join("a.txt"), "x").unwrap();
    assert!(allows_args(
        &f,
        json!({ "command": "sort *.txt", "cwd": "bait" })
    ));
    // A file named like an option would be read by `sort` as one: `sort *` would write `evil`.
    std::fs::write(bait.join("--output=evil"), "x").unwrap();
    assert!(!allows_args(
        &f,
        json!({ "command": "sort *", "cwd": "bait" })
    ));
    assert!(!allows_args(
        &f,
        json!({ "command": "git diff *", "cwd": "bait" })
    ));
    // Written with a directory in front, the same file is an operand, not an option.
    assert!(allows_args(&f, json!({ "command": "cat bait/*" })));
}

#[test]
fn getopt_abbreviations_of_hostile_options_are_hostile() {
    let f = fixture();
    for command in [
        "sort --out=x calc.py",
        "sort --o=x calc.py",
        "sort --compress=sh calc.py",
        "sort --temp=/tmp calc.py",
        "git grep --open=sh foo",
        "git grep --open-files=sh foo",
        "git diff --ext",
        "git show --textc HEAD:calc.py",
        "git log --outp=x",
        "tail --foll calc.py",
        "diff --pagin calc.py calc.py",
        "rg --pr ./evil x",
    ] {
        assert!(!allows(&f, command), "{command}");
    }
    for command in [
        "git log --oneline",
        "git diff --exit-code",
        "git ls-files --exclude-standard",
        "sort --reverse calc.py",
    ] {
        assert!(allows(&f, command), "{command}");
    }
}

#[test]
fn a_wildcard_that_names_too_many_files_cannot_be_proven() {
    let f = fixture();
    let many = f.root.join("many");
    std::fs::create_dir(&many).unwrap();
    for n in 0..(guard::MAX_GLOB_MATCHES + 8) {
        std::fs::write(many.join(format!("f{n}.txt")), "x").unwrap();
    }
    assert!(!allows(&f, "cat many/*.txt"));
}

#[test]
fn a_dotenv_template_is_readable_and_a_symlink_to_a_secret_is_not() {
    let f = fixture();
    assert!(allows(&f, "cat .env.example"));
    assert!(allows(&f, "git show HEAD:.env.example"));
    assert!(!allows(&f, "cat notes.txt"));
    symlink(f.root.join(".env"), f.root.join(".env.sample")).unwrap();
    assert!(!allows(&f, "cat .env.sample"));
}

#[test]
fn configured_read_denies_extend_the_secret_guard() {
    let f = fixture();
    let mut configured = rules();
    configured.push(PermissionRule {
        tool: "read_file".into(),
        patterns: vec!["**/src/**".into()],
        decision: PermissionDecision::Deny,
        source: forge_types::RuleSource::Configured,
        reason: None,
    });
    let args = json!({ "command": "cat src/lib.rs" });
    assert!(allows_args(&f, args.clone()));
    assert!(!auto_allows_with(
        &args,
        &configured,
        Some(&f.root),
        Some(f.home.clone())
    ));
}

#[test]
fn the_secret_guard_does_not_depend_on_the_caller_passing_rules() {
    let f = fixture();
    let args = json!({ "command": "cat .env" });
    assert!(!auto_allows_with(
        &args,
        &[],
        Some(&f.root),
        Some(f.home.clone())
    ));
}

#[test]
fn the_opt_out_rule_turns_the_auto_allow_off() {
    let f = fixture();
    let mut rules = rules();
    let args = json!({ "command": "git status" });
    assert!(auto_allows_with(&args, &rules, Some(&f.root), None));
    rules.push(forge_types::read_only_shell_opt_out_rule());
    assert!(!auto_allows_with(&args, &rules, Some(&f.root), None));
}

#[test]
fn a_recursive_search_with_no_home_to_compare_against_is_refused() {
    let f = fixture();
    let args = json!({ "command": "grep -rn add src" });
    assert!(auto_allows_with(
        &args,
        &rules(),
        Some(&f.root),
        Some(f.home.clone())
    ));
    assert!(!auto_allows_with(&args, &rules(), Some(&f.root), None));
    // A search that is not recursive needs no home to judge it.
    let plain = json!({ "command": "grep -n add calc.py" });
    assert!(auto_allows_with(&plain, &rules(), Some(&f.root), None));
}

#[test]
fn a_tilde_with_no_home_is_refused() {
    let f = fixture();
    let args = json!({ "command": "ls ~" });
    assert!(!auto_allows_with(&args, &rules(), Some(&f.root), None));
}

#[test]
fn a_long_line_is_refused_rather_than_scanned() {
    let f = fixture();
    let long = format!("echo {}", "a".repeat(MAX_LINE_CHARS));
    assert!(!allows(&f, &long));
}

/// `allows_args` for `command` run from `cwd`, a directory relative to the workspace.
fn allows_in(f: &Fixture, command: &str, cwd: &str) -> bool {
    allows_args(f, json!({ "command": command, "cwd": cwd }))
}

#[test]
fn a_plain_rg_skips_hidden_files_but_not_a_visible_secret() {
    let f = fixture();
    assert!(allows_in(&f, "rg -n add", "src"));
    assert!(allows_in(&f, "rg --files", "src"));
    // `deep/a/b/.env` is hidden, so rg never opens it; `grep -r` and `rg --hidden` do.
    assert!(allows_in(&f, "rg -n add", "deep/a"));
    assert!(!allows_in(&f, "grep -rn add .", "deep/a"));
    assert!(!allows_in(&f, "rg --hidden add", "deep/a"));
    // A key file is not hidden, and rg reads it unless it is told to skip it.
    std::fs::write(f.root.join("deep/a/b/deploy.key"), "x").unwrap();
    assert!(!allows_in(&f, "rg -n add", "deep/a"));
    assert!(!allows_in(&f, "rg -n add", "."));
}

#[test]
fn a_recursive_search_stays_inside_the_workspace() {
    let f = fixture();
    let docs = f.home.join("docs");
    let docs = docs.to_string_lossy();
    // `docs` holds nothing a secret glob names, and lies below the home directory, not above it.
    for command in [
        "grep -rn plain .",
        "grep -rn plain",
        "rg plain .",
        "diff -r . .",
    ] {
        assert!(!allows_in(&f, command, &docs), "{command} in {docs}");
    }
    for command in [
        format!("grep -rn plain {docs}"),
        "grep -rn plain ../home/docs".to_string(),
        "grep -rn plain ../home".to_string(),
        "rg plain ../home/docs".to_string(),
        // A symlink in the workspace that leads out of it, named plainly or walked through.
        "grep -rn plain escape".to_string(),
        "grep -rn plain escape/..".to_string(),
        "rg plain escape".to_string(),
    ] {
        assert!(!allows(&f, &command), "{command}");
    }
    // Naming a file outside the workspace is a read of that file, which the secret globs judge.
    assert!(allows_in(&f, "grep -n plain notes.txt", &docs));
    assert!(allows(&f, "cat ../home/docs/notes.txt"));
}

#[test]
fn a_symlink_out_of_a_searched_tree_is_unprovable_and_a_loop_ends() {
    let f = fixture();
    let tree = f.root.join("tree");
    std::fs::create_dir(&tree).unwrap();
    std::fs::write(tree.join("a.txt"), "x").unwrap();
    assert!(allows(&f, "grep -rn x tree"));
    symlink(&tree, tree.join("loop")).unwrap();
    assert!(allows(&f, "grep -rn x tree"));
    symlink(f.home.join("docs"), tree.join("out")).unwrap();
    assert!(!allows(&f, "grep -rn x tree"));
    assert!(!allows(&f, "rg -L x tree"));
    std::fs::remove_file(tree.join("out")).unwrap();
    // Inside the workspace, but it leads to a tree that holds a secret `tree` does not.
    symlink(f.root.join("deep"), tree.join("in")).unwrap();
    assert!(!allows(&f, "grep -R x tree"));
    // rg skips the hidden `.env` down there, but follows the link to a visible key file.
    assert!(allows(&f, "rg -L x tree"));
    std::fs::write(f.root.join("deep/leak.key"), "x").unwrap();
    assert!(!allows(&f, "rg -L x tree"));
}

#[test]
fn two_clean_directories_may_be_compared_and_a_dirty_one_may_not() {
    let f = fixture();
    for dir in ["one", "two"] {
        std::fs::create_dir(f.root.join(dir)).unwrap();
        std::fs::write(f.root.join(dir).join("a.txt"), dir).unwrap();
    }
    assert!(allows(&f, "diff one two"));
    assert!(allows(&f, "diff -r one two"));
    std::fs::write(f.root.join("two/.env"), "K=v\n").unwrap();
    assert!(!allows(&f, "diff one two"));
    assert!(!allows(&f, "diff two one"));
}

#[test]
fn git_runs_only_in_a_repository_whose_config_runs_nothing() {
    let f = fixture();
    let config = f.root.join(".git/config");
    let ask = |command: &str| !allows(&f, command);
    assert!(allows(&f, "git status"));
    for hostile in [
        "[core]\n\tfsmonitor = touch fsmonitor_ran; echo\n",
        "[core]\n\tpager = sh -c x\n",
        "[diff \"x\"]\n\ttextconv = touch ran; cat\n",
        "[filter \"x\"]\n\tclean = touch ran\n",
        "[extensions]\n\tpartialClone = origin\n",
        "[remote \"origin\"]\n\tpromisor = true\n",
        "[include]\n\tpath = /tmp/evil\n",
        "[includeIf \"gitdir:/\"]\n\tpath = /tmp/evil\n",
    ] {
        std::fs::write(&config, format!("{INERT_CONFIG}{hostile}")).unwrap();
        for command in [
            "git status",
            "git diff",
            "git log -p",
            "git show HEAD",
            "git blame calc.py",
        ] {
            assert!(ask(command), "{command} under {hostile:?}");
        }
        // Nothing but git is judged by the repository's config.
        assert!(allows(&f, "ls"), "{hostile:?}");
    }
    std::fs::write(
        &config,
        format!("{INERT_CONFIG}[core]\n\tfsmonitor = true\n"),
    )
    .unwrap();
    assert!(allows(&f, "git status"));
    std::fs::write(&config, INERT_CONFIG).unwrap();
    std::fs::create_dir_all(f.root.join(".git/hooks")).unwrap();
    std::fs::write(f.root.join(".git/hooks/post-index-change"), "#!/bin/sh\n").unwrap();
    assert!(ask("git status"));
    std::fs::remove_file(f.root.join(".git/hooks/post-index-change")).unwrap();
    assert!(allows(&f, "git status"));
}

#[test]
fn git_outside_a_repository_asks_because_diff_and_grep_turn_into_no_index_there() {
    let f = fixture();
    if f.home.ancestors().any(|dir| dir.join(".git").exists()) {
        return;
    }
    let home = f.home.to_string_lossy();
    for command in [
        "git status",
        "git log",
        "git grep plain",
        "git diff notes.txt docs",
        "git diff .ssh docs",
    ] {
        assert!(!allows_in(&f, command, &home), "{command}");
    }
}

#[test]
fn a_committed_bare_repository_is_not_trusted_as_the_repository() {
    let f = fixture();
    let nested = f.root.join("tests/fixtures/repo.git");
    std::fs::create_dir_all(nested.join("objects")).unwrap();
    std::fs::create_dir_all(nested.join("refs")).unwrap();
    std::fs::write(nested.join("HEAD"), "ref: refs/heads/main\n").unwrap();
    std::fs::write(
        nested.join("config"),
        "[diff \"x\"]\n\ttextconv = touch ran; cat\n[core]\n\tbare = true\n",
    )
    .unwrap();
    for command in [
        "git log -p",
        "git diff HEAD~1 HEAD",
        "git status",
        "git log --oneline",
        "git blame f.txt",
        "git show HEAD",
    ] {
        assert!(allows(&f, command), "{command} from the workspace");
        assert!(
            !allows_in(&f, command, "tests/fixtures/repo.git"),
            "{command} inside the committed repository"
        );
    }
}

// ---- the broker: what wins over the auto-allow, and what it leaves alone ----

fn decide_ask(f: &Fixture, command: &str, rules: &[PermissionRule]) -> PermissionDecision {
    decide_mode_in(f, PermissionMode::Default, command, rules)
}

fn decide_mode_in(
    f: &Fixture,
    mode: PermissionMode,
    command: &str,
    rules: &[PermissionRule],
) -> PermissionDecision {
    decide_in(
        mode,
        SideEffect::Shell,
        "shell",
        &json!({ "command": command }),
        rules,
        Some(&f.root),
    )
}

fn rule(decision: PermissionDecision, patterns: &[&str]) -> PermissionRule {
    PermissionRule {
        tool: "shell".into(),
        patterns: patterns.iter().map(|p| (*p).to_string()).collect(),
        decision,
        source: forge_types::RuleSource::Configured,
        reason: None,
    }
}

#[test]
fn ask_runs_read_only_shell_and_prompts_for_the_rest() {
    use PermissionDecision::{Allow, Ask};
    let f = fixture();
    let rules = rules();
    for command in [
        "git status",
        "git diff",
        "ls",
        "cat calc.py",
        "grep -rn add src",
    ] {
        assert_eq!(decide_ask(&f, command, &rules), Allow, "{command}");
    }
    for command in [
        "touch x",
        "git commit -m x",
        "cargo test",
        "ls > out",
        "ls; touch x",
    ] {
        assert_eq!(decide_ask(&f, command, &rules), Ask, "{command}");
    }
}

#[test]
fn deny_rules_and_the_safety_floor_beat_the_auto_allow() {
    use PermissionDecision::Deny;
    let f = fixture();
    let mut with_deny = rules();
    with_deny.push(rule(Deny, &["git diff*"]));
    assert_eq!(decide_ask(&f, "git diff", &with_deny), Deny);
    assert_eq!(
        decide_ask(&f, "git status", &with_deny),
        PermissionDecision::Allow
    );
    // The built-in secret verbs deny before the auto-allow is asked.
    let rules = rules();
    for command in [
        "cat .env",
        "cat ~/.ssh/id_rsa",
        "ls; cat server.pem",
        "grep KEY .env",
    ] {
        assert_eq!(decide_ask(&f, command, &rules), Deny, "{command}");
    }
    // The floor refuses whatever else the line holds.
    assert_eq!(decide_ask(&f, "ls; sudo ls", &rules), Deny);
    assert!(denial_note("shell", &json!({ "command": "ls; sudo ls" })).is_some());
    assert_eq!(decide_ask(&f, "rm -rf ~", &rules), Deny);
}

#[test]
fn a_secret_the_deny_verbs_miss_still_asks() {
    use PermissionDecision::Ask;
    let f = fixture();
    let rules = rules();
    for command in [
        "wc -c .env",
        "git show HEAD:.env",
        "cat notes.txt",
        "cat /proc/self/environ",
    ] {
        assert_eq!(decide_ask(&f, command, &rules), Ask, "{command}");
    }
}

#[test]
fn explicit_ask_rules_beat_the_auto_allow() {
    use PermissionDecision::{Allow, Ask};
    let f = fixture();
    let mut asking = rules();
    asking.push(rule(Ask, &["git *"]));
    assert_eq!(decide_ask(&f, "git status", &asking), Ask);
    assert_eq!(decide_ask(&f, "ls && git status", &asking), Ask);
    assert_eq!(decide_ask(&f, "ls", &asking), Allow);
    let mut everything = rules();
    everything.push(rule(Ask, &[]));
    assert_eq!(decide_ask(&f, "ls", &everything), Ask);
    let mut wildcard_tool = rules();
    wildcard_tool.push(PermissionRule {
        tool: "*".into(),
        ..rule(Ask, &["ls*"])
    });
    assert_eq!(decide_ask(&f, "ls", &wildcard_tool), Ask);
}

#[test]
fn an_allow_rule_for_one_segment_does_not_widen_the_auto_allow() {
    use PermissionDecision::Ask;
    let f = fixture();
    let mut allowing = rules();
    allowing.push(rule(PermissionDecision::Allow, &["git status"]));
    assert_eq!(decide_ask(&f, "git status && touch x", &allowing), Ask);
}

#[test]
fn read_only_temper_is_untouched() {
    use PermissionDecision::{Allow, Deny};
    let f = fixture();
    let rules = rules();
    for (command, expected) in [
        ("git status", Allow),
        ("ls | wc -l", Allow),
        // Plan's own classifier still takes these, which Ask's stricter one refuses.
        ("echo $(pwd)", Allow),
        ("touch x", Deny),
        ("git commit -m x", Deny),
        ("ls; touch x", Deny),
    ] {
        assert_eq!(
            decide_mode_in(&f, PermissionMode::Plan, command, &rules),
            expected,
            "{command}"
        );
    }
    assert_eq!(
        decide_ask(&f, "echo $(pwd)", &rules),
        PermissionDecision::Ask
    );
}

#[test]
fn other_tempers_are_unchanged() {
    use PermissionDecision::{Allow, Ask};
    let f = fixture();
    let rules = rules();
    // (mode, command, what the temper decided before the Ask auto-allow existed)
    let table = [
        (PermissionMode::AcceptEdits, "git status", Allow),
        (PermissionMode::AcceptEdits, "touch x", Allow),
        (
            PermissionMode::AcceptEdits,
            "cp calc.py /opt/forge-ask-shell-probe",
            Ask,
        ),
        (PermissionMode::Bypass, "git status", Allow),
        (PermissionMode::Bypass, "touch x", Allow),
        (PermissionMode::Auto, "git status", Allow),
        (PermissionMode::Auto, "ls", Allow),
        (
            PermissionMode::Auto,
            "cp calc.py /opt/forge-ask-shell-probe",
            Ask,
        ),
    ];
    for (mode, command, expected) in table {
        assert_eq!(
            decide_mode_in(&f, mode, command, &rules),
            expected,
            "{mode:?} {command}"
        );
    }
}

#[test]
fn only_the_shell_tool_is_auto_allowed() {
    use PermissionDecision::Ask;
    let f = fixture();
    let args = json!({ "command": "git status" });
    // A tool that is not the shell, even one claiming a shell side effect, still asks.
    assert_eq!(
        decide_in(
            PermissionMode::Default,
            SideEffect::Shell,
            "write_file",
            &args,
            &rules(),
            Some(&f.root)
        ),
        Ask
    );
    assert_eq!(
        decide_in(
            PermissionMode::Default,
            SideEffect::Write,
            "shell",
            &args,
            &rules(),
            Some(&f.root)
        ),
        Ask
    );
    assert_eq!(
        decide_in(
            PermissionMode::Default,
            SideEffect::External,
            "shell",
            &args,
            &rules(),
            Some(&f.root)
        ),
        Ask
    );
}

#[test]
fn the_setting_turns_the_auto_allow_off() {
    use PermissionDecision::{Allow, Ask};
    let f = fixture();
    let mut config = forge_config::Config::default();
    assert_eq!(
        decide_ask(&f, "git status", &config.permission_rules()),
        Allow
    );
    config.permissions.auto_allow_read_only_shell = false;
    assert_eq!(
        decide_ask(&f, "git status", &config.permission_rules()),
        Ask
    );
    // The off switch changes nothing for the tempers that never used the auto-allow.
    assert_eq!(
        decide_mode_in(
            &f,
            PermissionMode::AcceptEdits,
            "git status",
            &config.permission_rules()
        ),
        Allow
    );
}
