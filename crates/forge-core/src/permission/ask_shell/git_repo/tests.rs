use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use tempfile::TempDir;

use super::super::tests::logging_git;
use super::*;

const INERT: &str = "[core]\n\trepositoryformatversion = 0\n\tfilemode = true\n\tbare = false\n\
[remote \"origin\"]\n\turl = https://example.invalid/x.git\n\tfetch = +refs/heads/*:refs/remotes/origin/*\n\
[branch \"main\"]\n\tremote = origin\n\tmerge = refs/heads/main\n";

/// A config section that starts a program the moment git refreshes the index.
const HOSTILE: &str = "[core]\n\tfsmonitor = touch ran; echo\n";

const SOME_OID: &str = "0123456789abcdef0123456789abcdef01234567";

/// [`super::inspect`] with the `git` on the test's `PATH`, in a project the user trusted.
fn inspect(cwd: &Path) -> Option<Repo> {
    inspect_trusting(cwd, &|_| true)
}

fn inspect_trusting(cwd: &Path, trusted: &dyn Fn(&Path) -> bool) -> Option<Repo> {
    super::inspect(cwd, Path::new("git"), trusted)
}

fn workspace() -> (TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    (dir, root)
}

/// Real git, run the way the probes run it: neither the developer's config nor their repository
/// variables.
fn git_command(dir: &Path, args: &[&str]) -> Command {
    let mut command = Command::new("git");
    command
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    for var in LOCAL_REPO_ENV {
        command.env_remove(var);
    }
    command
}

fn git(dir: &Path, args: &[&str]) {
    let status = git_command(dir, args).status().expect("git is installed");
    assert!(status.success(), "git {args:?} in {}", dir.display());
}

/// A directory with an empty commit, which is what `git worktree add` and the like need.
fn init_with_commit(dir: &Path) {
    std::fs::create_dir_all(dir).unwrap();
    git(dir, &["init", "-q", "-b", "main", "."]);
    git(
        dir,
        &[
            "-c",
            "user.name=n",
            "-c",
            "user.email=a@b",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "x",
        ],
    );
}

/// A repository made by git whose `.git/config` is then replaced with `config`.
fn repo_with(config: &str) -> (TempDir, PathBuf) {
    let (dir, root) = workspace();
    git(&root, &["init", "-q", "-b", "main", "."]);
    std::fs::write(root.join(".git/config"), config).unwrap();
    (dir, root)
}

/// What git needs to take a directory for a repository (or a bare one), with a config of its own.
fn make_git_dir(dir: &Path, config: &str) {
    std::fs::create_dir_all(dir.join("objects")).unwrap();
    std::fs::create_dir_all(dir.join("refs")).unwrap();
    std::fs::write(dir.join("HEAD"), "ref: refs/heads/main\n").unwrap();
    std::fs::write(dir.join("config"), config).unwrap();
}

/// Put gitlinks (submodule entries) into the index of the repository at `dir`, checked out or not.
fn add_gitlinks(dir: &Path, names: &[String]) {
    let mut child = git_command(dir, &["update-index", "--add", "--index-info"])
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    for name in names {
        writeln!(stdin, "160000 {SOME_OID} 0\t{name}").unwrap();
    }
    drop(stdin);
    assert!(child.wait().unwrap().success());
}

fn is_inert_at(cwd: &Path) -> bool {
    inspect(cwd).is_some()
}

fn inert(config: &str) -> bool {
    let (_dir, root) = repo_with(config);
    is_inert_at(&root)
}

#[test]
fn a_clone_style_config_runs_nothing() {
    assert!(inert(INERT));
    assert!(inert(""));
    assert!(inert(&format!(
        "# comment\n; another\n{INERT}[user]\n\tname = A B\n\temail = a@example.invalid\n\
[branch \"x\"]\n\tvscode-merge-base = origin/main\n[core] bare = false\n"
    )));
}

#[test]
fn a_repository_with_a_git_made_config_is_inert() {
    let (_dir, root) = workspace();
    git(&root, &["init", "-q", "-b", "main", "."]);
    assert!(is_inert_at(&root));
}

#[test]
fn every_key_that_runs_a_program_or_reaches_out_is_refused() {
    for line in [
        "[core]\n\tfsmonitor = touch ran; echo\n",
        "[core]\n\tfsmonitor = true\n",
        "[core]\n\tpager = sh -c x\n",
        "[core]\n\teditor = vi\n",
        "[core]\n\tsshCommand = ssh -o ProxyCommand=x\n",
        "[core]\n\taskPass = ./ask\n",
        "[core]\n\tgitProxy = ./proxy\n",
        "[core]\n\talternateRefsCommand = ./refs\n",
        "[core]\n\tworktree = ../elsewhere\n",
        "[core]\n\tattributesFile = /tmp/attributes\n",
        "[diff \"x\"]\n\ttextconv = touch ran; cat\n",
        "[diff]\n\texternal = ./drv\n",
        "[filter \"lfs\"]\n\tclean = git-lfs clean\n",
        "[merge \"x\"]\n\tdriver = ./drv\n",
        "[pager]\n\tlog = ./pager\n",
        "[gpg]\n\tprogram = ./gpg\n",
        "[gpg \"ssh\"]\n\tprogram = ./gpg\n",
        "[credential]\n\thelper = ./helper\n",
        "[alias]\n\tst = !touch ran\n",
        "[url \"ext::sh -c x\"]\n\tinsteadOf = https://example.invalid/\n",
        "[remote \"origin\"]\n\tpromisor = true\n",
        "[remote \"origin\"]\n\tvcs = ext\n",
        "[remote \"origin\"]\n\tpartialclonefilter = blob:none\n",
        "[extensions]\n\tpartialClone = origin\n",
        "[uploadpack]\n\tpackObjectsHook = ./hook\n",
        "[include]\n\tpath = /tmp/other\n",
        "[includeIf \"gitdir:/\"]\n\tpath = /tmp/other\n",
        "[includeIf \"hasconfig:remote.*.url:*\"]\n\tpath = /tmp/other\n",
        "[submodule \"m\"]\n\tupdate = !touch ran\n",
        "[hook \"h\"]\n\tcommand = touch ran\n",
        "[unknown]\n\tkey = v\n",
    ] {
        assert!(!inert(&format!("{INERT}{line}")), "{line:?}");
    }
}

#[test]
fn a_config_git_cannot_parse_is_refused() {
    for line in [
        "[core\n\tbare = false\n",
        "[core \"x]\n\tbare = false\n",
        "bare = false\n",
        "[]\n\tbare = false\n",
        "[core]\n\t= false\n",
        "[core]\n\tbare = false \\\n\tmore\n",
    ] {
        assert!(!inert(&format!("{INERT}{line}")), "{line:?}");
    }
}

#[test]
fn only_an_explicit_false_turns_the_fsmonitor_off() {
    assert!(inert(&format!("{INERT}[core]\n\tfsmonitor = false\n")));
    assert!(inert(&format!("{INERT}[core]\n\tfsmonitor = \"false\"\n")));
    // `true` is git's built-in daemon: it outlives the call, with watches and a socket in `.git`.
    assert!(!inert(&format!("{INERT}[core]\n\tfsmonitor = true\n")));
    assert!(!inert(&format!("{INERT}[core]\n\tfsmonitor = \"true\"\n")));
    assert!(!inert(&format!(
        "{INERT}[core]\n\tfsmonitor = true # builtin\n"
    )));
    assert!(!inert(&format!("{INERT}[core]\n\tfsmonitor\n")));
    assert!(!inert(&format!("{INERT}[core]\n\tfsmonitor = 1\n")));
}

#[test]
fn a_hook_git_runs_while_only_looking_is_refused() {
    for hook in ["post-index-change", "reference-transaction"] {
        let (_dir, root) = repo_with(INERT);
        let hooks = root.join(".git/hooks");
        std::fs::create_dir_all(&hooks).unwrap();
        std::fs::write(hooks.join("pre-commit"), "#!/bin/sh\n").unwrap();
        assert!(
            is_inert_at(&root),
            "a hook for a write is not run by a read"
        );
        std::fs::write(hooks.join(hook), "#!/bin/sh\n").unwrap();
        assert!(!is_inert_at(&root), "{hook}");
    }
}

/// `rev-parse --git-path` resolves a symlink, so a hook that is a link is judged by what it leads
/// to: a link to a program is a hook, and one to nothing is a hook git cannot run.
#[test]
fn a_hook_that_is_a_symlink_is_judged_by_what_it_leads_to() {
    let (_dir, root) = repo_with(INERT);
    let hooks = root.join(".git/hooks");
    std::fs::create_dir_all(&hooks).unwrap();
    let link = hooks.join("post-index-change");
    std::os::unix::fs::symlink("/nonexistent/hook", &link).unwrap();
    assert!(
        is_inert_at(&root),
        "git cannot run a hook that leads nowhere"
    );
    std::fs::remove_file(&link).unwrap();
    let program = root.join("program");
    std::fs::write(&program, "#!/bin/sh\n").unwrap();
    std::os::unix::fs::symlink(&program, &link).unwrap();
    assert!(!is_inert_at(&root));
}

#[test]
fn the_worktree_config_is_read_when_git_reads_it() {
    let (_dir, root) = repo_with(&format!("{INERT}[extensions]\n\tworktreeConfig = true\n"));
    assert!(is_inert_at(&root));
    std::fs::write(
        root.join(".git/config.worktree"),
        "[core]\n\tfsmonitor = ./hook\n",
    )
    .unwrap();
    assert!(!is_inert_at(&root));
}

#[test]
fn outside_a_repository_git_is_refused_because_it_reads_plain_files_there() {
    let (_dir, root) = workspace();
    // A repository further up would be judged instead, so only the plain case is pinned.
    if !root.ancestors().any(|dir| dir.join(".git").exists()) {
        assert_eq!(inspect(&root), None);
    }
    assert_eq!(inspect(&root.join("missing")), None);
}

#[test]
fn the_work_tree_is_the_top_of_the_checkout_and_a_bare_repository_has_none() {
    let (_dir, root) = repo_with(INERT);
    let sub = root.join("a/b");
    std::fs::create_dir_all(&sub).unwrap();
    assert_eq!(
        inspect(&sub),
        Some(Repo {
            work_tree: Some(root.clone())
        })
    );
    let bare = root.join("bare.git");
    make_git_dir(&bare, "[core]\n\tbare = true\n");
    assert_eq!(inspect(&bare), Some(Repo { work_tree: None }));
    assert_eq!(
        inspect(&root.join(".git")),
        Some(Repo { work_tree: None }),
        "inside the git directory there is no work tree"
    );
}

#[test]
fn a_directory_that_only_looks_like_a_repository_is_judged_like_one() {
    let (_dir, root) = repo_with(INERT);
    let nested = root.join("tests/fixtures/repo.git");
    make_git_dir(
        &nested,
        "[diff \"x\"]\n\ttextconv = touch ran; cat\n[core]\n\tbare = true\n",
    );
    assert!(is_inert_at(&root), "from the workspace root");
    assert!(
        !is_inert_at(&nested),
        "from inside the committed repository"
    );
    std::fs::create_dir_all(nested.join("sub")).unwrap();
    assert!(!is_inert_at(&nested.join("sub")));
    // Not bare-shaped without `refs/`, so git does not take it for a repository.
    std::fs::remove_dir_all(nested.join("refs")).unwrap();
    assert!(is_inert_at(&nested));
}

/// Git takes the first ancestor whose `HEAD` is valid, and skips one whose is not; a directory
/// with any `HEAD` file hid the bare-shaped repository git really adopts, which a plain clone can
/// bring along.
#[test]
fn a_decoy_head_does_not_hide_the_bare_repository_git_adopts() {
    let (_dir, root) = repo_with(INERT);
    let hostile = root.join("tests/fixture");
    make_git_dir(&hostile, &format!("[core]\n\tbare = true\n{HOSTILE}"));
    let decoy = hostile.join("inner");
    make_git_dir(&decoy, "");
    std::fs::write(decoy.join("HEAD"), "garbage\n").unwrap();
    assert!(
        !is_inert_at(&hostile),
        "control: the hostile directory itself"
    );
    assert!(
        !is_inert_at(&decoy),
        "git adopts the hostile directory from here"
    );
}

#[test]
fn a_dot_git_directory_that_is_not_a_repository_is_skipped_as_git_skips_it() {
    let (_dir, root) = repo_with("[core]\n\tfsmonitor = ./hook\n");
    let inner = root.join("inner");
    std::fs::create_dir_all(inner.join(".git")).unwrap();
    // An empty `.git` is no repository, so git carries on to the one above.
    assert!(!is_inert_at(&inner));
    make_git_dir(&inner.join(".git"), INERT);
    assert!(is_inert_at(&inner));
}

#[test]
fn a_linked_worktree_is_judged_by_the_shared_config_too() {
    let (_dir, root) = workspace();
    let main = root.join("main");
    init_with_commit(&main);
    let linked = root.join("linked");
    git(&main, &["worktree", "add", "-q", "--detach", "../linked"]);
    assert!(is_inert_at(&linked));
    assert_eq!(
        inspect(&linked),
        Some(Repo {
            work_tree: Some(linked.clone())
        })
    );
    let config = std::fs::read_to_string(main.join(".git/config")).unwrap();
    std::fs::write(
        main.join(".git/config"),
        format!("{config}[core]\n\tfsmonitor = ./hook\n"),
    )
    .unwrap();
    assert!(
        !is_inert_at(&linked),
        "the shared config is the linked one's"
    );
    // The same linked worktree reached through a relative `gitdir:` pointer.
    std::fs::write(main.join(".git/config"), config).unwrap();
    let admin = std::fs::read_dir(main.join(".git/worktrees"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .file_name();
    std::fs::write(
        linked.join(".git"),
        format!(
            "gitdir: ../main/.git/worktrees/{}\n",
            admin.to_string_lossy()
        ),
    )
    .unwrap();
    assert!(is_inert_at(&linked));
}

#[test]
fn a_broken_gitdir_file_is_refused() {
    for body in [
        "",
        "gitdir:",
        "gitdir: /nonexistent/elsewhere",
        "not a pointer",
    ] {
        let (_dir, root) = workspace();
        std::fs::write(root.join(".git"), body).unwrap();
        assert!(!is_inert_at(&root), "{body:?}");
    }
}

/// git reads a `.git` file as `gitdir: ` plus the path to the end of the line, byte for byte;
/// trimming it here dropped extra leading spaces, so the two named different directories.
#[test]
fn a_gitdir_file_path_is_not_trimmed_the_way_git_does_not_trim_it() {
    let (_dir, root) = workspace();
    make_git_dir(&root.join("gd"), INERT);
    make_git_dir(&root.join(" gd"), &format!("{INERT}{HOSTILE}"));
    std::fs::write(root.join(".git"), "gitdir:  gd\n").unwrap();
    assert!(!is_inert_at(&root), "git opens the directory named \" gd\"");
    std::fs::write(root.join(".git"), "gitdir: gd\n").unwrap();
    assert!(is_inert_at(&root), "control: the plain name");
}

#[test]
fn a_gitdir_path_ending_in_a_unicode_blank_is_not_trimmed_either() {
    let (_dir, root) = workspace();
    make_git_dir(&root.join("gd"), INERT);
    make_git_dir(&root.join("gd\u{a0}"), &format!("{INERT}{HOSTILE}"));
    std::fs::write(root.join(".git"), "gitdir: gd\u{a0}\n").unwrap();
    assert!(!is_inert_at(&root));
}

/// The same for `commondir`, with a no-break space that `str::trim` strips and git keeps.
#[test]
fn a_commondir_path_is_not_trimmed_the_way_git_does_not_trim_it() {
    let (_dir, root) = workspace();
    make_git_dir(&root.join("c"), INERT);
    make_git_dir(&root.join("c\u{a0}"), &format!("{INERT}{HOSTILE}"));
    let admin = root.join("admin");
    std::fs::create_dir_all(&admin).unwrap();
    std::fs::write(admin.join("HEAD"), "ref: refs/heads/main\n").unwrap();
    std::fs::write(admin.join("commondir"), "../c\u{a0}\n").unwrap();
    std::fs::create_dir_all(root.join("w")).unwrap();
    std::fs::write(root.join("w/.git"), "gitdir: ../admin\n").unwrap();
    assert!(
        !is_inert_at(&root.join("w")),
        "git reads the config of the c-nbsp directory"
    );
    std::fs::write(admin.join("commondir"), "../c\n").unwrap();
    assert!(is_inert_at(&root.join("w")), "control: the plain name");
}

#[test]
fn a_hooks_path_is_judged_by_where_git_looks_for_hooks() {
    let (_dir, root) = repo_with(&format!("{INERT}[core]\n\thooksPath = .husky/_\n"));
    assert!(is_inert_at(&root), "no such directory holds no hook");
    let hooks = root.join(".husky/_");
    std::fs::create_dir_all(&hooks).unwrap();
    std::fs::write(hooks.join("pre-commit"), "#!/bin/sh\n").unwrap();
    assert!(is_inert_at(&root), "hooks that only a write runs");
    std::fs::write(hooks.join("post-index-change"), "#!/bin/sh\n").unwrap();
    assert!(!is_inert_at(&root));
    std::fs::remove_file(hooks.join("post-index-change")).unwrap();
    std::fs::write(hooks.join("reference-transaction"), "#!/bin/sh\n").unwrap();
    assert!(!is_inert_at(&root));
    // A relative path is the work tree's, wherever the command runs from.
    let deep = root.join("a/b");
    std::fs::create_dir_all(&deep).unwrap();
    assert!(!is_inert_at(&deep));
    // An absolute path is taken as it is, wherever the repository is.
    let elsewhere = root.join("elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();
    std::fs::write(elsewhere.join("post-index-change"), "").unwrap();
    std::fs::write(
        root.join(".git/config"),
        format!("{INERT}[core]\n\thooksPath = {}\n", elsewhere.display()),
    )
    .unwrap();
    assert!(!is_inert_at(&root));
    std::fs::remove_file(elsewhere.join("post-index-change")).unwrap();
    assert!(is_inert_at(&root));
}

/// A config value keeps every character that is not ASCII blank, so a hooks directory named with a
/// no-break space is not the directory a trimmed value names.
#[test]
fn a_hooks_path_with_a_unicode_blank_names_another_directory() {
    for blank in ['\u{a0}', '\u{85}', '\u{2028}', '\u{3000}', '\u{b}', '\u{c}'] {
        let (_dir, root) = repo_with(&format!("{INERT}[core]\n\thooksPath = {blank}hk\n"));
        let hooks = root.join(format!("{blank}hk"));
        std::fs::create_dir_all(&hooks).unwrap();
        std::fs::write(hooks.join("post-index-change"), "#!/bin/sh\n").unwrap();
        assert!(!is_inert_at(&root), "{blank:?}");
    }
}

#[test]
fn a_hooks_path_that_git_reads_differently_from_its_text_is_still_judged_as_git_reads_it() {
    for odd in ["\"x\"", "a # b", "a;b", "~someone/hooks", ""] {
        let (_dir, root) = repo_with(&format!("{INERT}[core]\n\thooksPath = {odd}\n"));
        let wanted = match odd {
            "\"x\"" => "x",
            "a # b" | "a;b" => "a",
            other => other,
        };
        if wanted.is_empty() || wanted.starts_with('~') {
            continue;
        }
        let hooks = root.join(wanted);
        std::fs::create_dir_all(&hooks).unwrap();
        std::fs::write(hooks.join("post-index-change"), "").unwrap();
        assert!(!is_inert_at(&root), "{odd:?}");
    }
    let (_dir, root) = repo_with(&format!("{INERT}[core]\n\thooksPath\n"));
    assert!(
        !is_inert_at(&root),
        "a hooks path with no value is a git error"
    );
}

/// `git status` and `git diff` run `git status` inside every gitlink of the index that has a
/// `.git`, with that repository's own config, wherever the nested `.git` is.
#[test]
fn a_nested_repository_behind_a_gitlink_is_judged_too() {
    let (_dir, root) = workspace();
    init_with_commit(&root);
    init_with_commit(&root.join("sub"));
    git(&root, &["add", "sub"]);
    assert!(is_inert_at(&root), "control: an inert nested repository");
    assert!(is_inert_at(&root.join("sub")));
    let mut config = std::fs::read_to_string(root.join("sub/.git/config")).unwrap();
    config.push_str(HOSTILE);
    std::fs::write(root.join("sub/.git/config"), &config).unwrap();
    assert!(!is_inert_at(&root), "git status runs sub's fsmonitor");
    assert!(!is_inert_at(&root.join("sub")), "and from inside it");
    // From a directory under the root, which `git status` still covers entirely.
    std::fs::create_dir_all(root.join("a")).unwrap();
    assert!(!is_inert_at(&root.join("a")));
}

#[test]
fn a_nested_repository_that_is_not_in_the_index_is_not_run() {
    let (_dir, root) = workspace();
    init_with_commit(&root);
    init_with_commit(&root.join("sub"));
    let mut config = std::fs::read_to_string(root.join("sub/.git/config")).unwrap();
    config.push_str(HOSTILE);
    std::fs::write(root.join("sub/.git/config"), config).unwrap();
    assert!(
        is_inert_at(&root),
        "git status lists `sub/` and goes no further"
    );
}

#[test]
fn a_gitlink_with_nothing_checked_out_has_nothing_to_run() {
    let (_dir, root) = workspace();
    init_with_commit(&root);
    add_gitlinks(&root, &["sub".to_string()]);
    assert!(is_inert_at(&root));
    std::fs::create_dir_all(root.join("sub")).unwrap();
    assert!(is_inert_at(&root), "an empty directory is not a repository");
    std::fs::create_dir_all(root.join("sub/.git")).unwrap();
    assert!(
        is_inert_at(&root),
        "an empty .git is no repository: git finds the one above, or stops"
    );
    std::fs::remove_dir(root.join("sub/.git")).unwrap();
    std::fs::write(root.join("sub/.git"), "gitdir: ../nowhere\n").unwrap();
    assert!(
        !is_inert_at(&root),
        "a gitfile git cannot follow is not provable"
    );
}

/// `core.worktree` of the nested repository's own moves its work tree, but `git status` of the
/// parent still runs there, with that repository's config: the repository is judged by its git
/// directory, and one that moved its work tree away from the gitlink is refused outright (it is an
/// oracle for any file the work tree names, see the next test).
#[test]
fn a_nested_repository_that_moves_its_own_work_tree_is_refused_whatever_its_config_holds() {
    let (_dir, root) = workspace();
    init_with_commit(&root);
    init_with_commit(&root.join("sub"));
    git(&root, &["add", "sub"]);
    let config = std::fs::read_to_string(root.join("sub/.git/config")).unwrap();
    assert!(is_inert_at(&root), "control: nothing moved");
    for moved in ["../..", "../../..", "/", "../../elsewhere", "../inner"] {
        std::fs::write(
            root.join("sub/.git/config"),
            format!("{config}[core]\n\tworktree = {moved}\n"),
        )
        .unwrap();
        assert!(!is_inert_at(&root), "worktree = {moved}");
    }
    std::fs::write(
        root.join("sub/.git/config"),
        format!("{config}[core]\n\tworktree = ../..\n{HOSTILE}"),
    )
    .unwrap();
    assert!(!is_inert_at(&root));
}

/// A nested repository whose `core.worktree` names a directory that is not the gitlink's own
/// makes the parent's `git status` compare the index of the nested repository with that directory
/// and print `A  sub` or `Am sub`: one bit about any file the user can read, none of which any
/// argument of the call names. Only a work tree that is the gitlink's own directory is accepted,
/// however it is spelled.
#[test]
fn a_nested_work_tree_elsewhere_is_an_oracle_and_is_refused() {
    let (_dir, root) = workspace();
    let victim = root.join("victim");
    std::fs::create_dir_all(victim.join(".ssh")).unwrap();
    std::fs::write(victim.join(".ssh/id_rsa"), "secret\n").unwrap();
    let top = root.join("top");
    init_with_commit(&top);
    init_with_commit(&top.join("sub"));
    git(&top, &["add", "sub"]);
    let config = std::fs::read_to_string(top.join("sub/.git/config")).unwrap();
    let point = |work_tree: &str| {
        std::fs::write(
            top.join("sub/.git/config"),
            format!("{config}[core]\n\tworktree = {work_tree}\n"),
        )
        .unwrap();
    };
    point(victim.to_str().unwrap());
    assert!(!is_inert_at(&top), "an absolute work tree elsewhere");
    assert!(
        !is_inert_at(&top.join("sub")),
        "from inside the nested repository too"
    );
    // The gitlink's own directory, spelled three ways, is the work tree git would pick anyway.
    for same in ["..", "../.", &format!("{}/sub", top.display())] {
        point(same);
        assert!(is_inert_at(&top), "worktree = {same}");
    }
    // A work tree with a directory between it and the gitlink is not the gitlink.
    std::fs::create_dir_all(top.join("sub/inner")).unwrap();
    point(top.join("sub/inner").to_str().unwrap());
    assert!(!is_inert_at(&top));
}

/// A module directory that is a symlink was skipped by the scan (`DirEntry::file_type` does not
/// follow it), while git follows it from the submodule's `.git` file.
#[test]
fn a_symlinked_module_directory_is_followed_as_git_follows_it() {
    let (_dir, root) = workspace();
    init_with_commit(&root);
    let real = root.join("hostgit");
    make_git_dir(
        &real,
        &format!(
            "[core]\n\tbare = false\n\tworktree = ../sub\n{}",
            HOSTILE.trim_start_matches("[core]\n")
        ),
    );
    std::fs::create_dir_all(root.join(".git/modules")).unwrap();
    std::os::unix::fs::symlink(&real, root.join(".git/modules/m")).unwrap();
    std::fs::create_dir_all(root.join("sub")).unwrap();
    std::fs::write(root.join("sub/.git"), "gitdir: ../.git/modules/m\n").unwrap();
    add_gitlinks(&root, &["sub".to_string()]);
    assert!(!is_inert_at(&root), "git follows the link");
    std::fs::write(
        real.join("config"),
        "[core]\n\tbare = false\n\tworktree = ../sub\n",
    )
    .unwrap();
    assert!(
        is_inert_at(&root),
        "a submodule's own config may name its worktree"
    );
}

#[test]
fn only_a_submodules_own_config_may_name_a_worktree() {
    let (_dir, root) = repo_with(&format!("{INERT}[core]\n\tworktree = ../elsewhere\n"));
    assert!(!is_inert_at(&root));
    let (_dir, root) = workspace();
    init_with_commit(&root);
    let module = root.join(".git/modules/vendor");
    make_git_dir(
        &module,
        "[core]\n\trepositoryformatversion = 0\n\tbare = false\n\tworktree = ../../../vendor\n",
    );
    std::fs::create_dir_all(root.join("vendor")).unwrap();
    std::fs::write(root.join("vendor/.git"), "gitdir: ../.git/modules/vendor\n").unwrap();
    add_gitlinks(&root, &["vendor".to_string()]);
    assert!(is_inert_at(&root));
    assert!(
        !is_inert_at(&root.join("vendor")),
        "from inside the submodule it is the repository asked about"
    );
}

#[test]
fn a_hostile_hook_in_a_nested_repository_is_found() {
    let (_dir, root) = workspace();
    init_with_commit(&root);
    init_with_commit(&root.join("sub"));
    git(&root, &["add", "sub"]);
    assert!(is_inert_at(&root));
    std::fs::write(root.join("sub/.git/hooks/post-index-change"), "#!/bin/sh\n").unwrap();
    assert!(!is_inert_at(&root));
}

#[test]
fn a_flood_of_checked_out_gitlinks_is_unprovable() {
    let (_dir, root) = workspace();
    init_with_commit(&root);
    let names: Vec<String> = (0..MAX_REPOS + 6).map(|n| format!("m{n}")).collect();
    add_gitlinks(&root, &names);
    assert!(is_inert_at(&root), "none of them is checked out");
    for name in &names {
        std::fs::create_dir_all(root.join(name).join(".git")).unwrap();
    }
    assert!(!is_inert_at(&root));
}

#[test]
fn repositories_nested_too_deep_are_unprovable() {
    let (_dir, root) = workspace();
    let mut at = root.clone();
    let mut levels = Vec::new();
    for level in 0..(MAX_DEPTH + 2) {
        init_with_commit(&at);
        levels.push(at.clone());
        let sub = at.join(format!("s{level}"));
        std::fs::create_dir_all(&sub).unwrap();
        add_gitlinks(&at, &[format!("s{level}")]);
        at = sub;
    }
    init_with_commit(&at);
    assert!(!is_inert_at(&root));
    assert!(is_inert_at(levels.last().unwrap()));
}

#[test]
fn an_oversized_or_unreadable_config_is_refused() {
    let (_dir, root) = repo_with(INERT);
    let mut big = String::from(INERT);
    for n in 0..60_000 {
        big.push_str(&format!("[remote \"r{n}\"]\n\turl = x\n"));
    }
    std::fs::write(root.join(".git/config"), big).unwrap();
    assert!(!is_inert_at(&root));
    std::fs::write(
        root.join(".git/config"),
        [
            b'[', b'u', b's', b'e', b'r', b']', b'\n', b'n', b'a', b'm', b'e', b'=', 0xff, b'\n',
        ],
    )
    .unwrap();
    assert!(!is_inert_at(&root));
    std::fs::remove_file(root.join(".git/config")).unwrap();
    std::fs::create_dir(root.join(".git/config")).unwrap();
    assert!(!is_inert_at(&root));
}

#[test]
fn the_keys_editors_and_forges_leave_in_a_clone_are_inert() {
    assert!(inert(&format!(
        "{INERT}[branch \"x\"]\n\tvscode-merge-base = origin/main\n\tgithub-pr-owner-number = \"o#r#1\"\n"
    )));
}

#[test]
fn the_listing_parser_only_trusts_scopes_it_knows() {
    let entry = |scope: &str, key: &str| format!("{scope}\0{key}\0");
    assert!(config_is_inert("", false));
    assert!(config_is_inert(&entry("local", "core.bare\nfalse"), false));
    assert!(config_is_inert(
        &entry("global", "core.pager\ndelta"),
        false
    ));
    assert!(config_is_inert(
        &entry("system", "credential.helper\nx"),
        false
    ));
    assert!(config_is_inert(&entry("command", "core.pager\nx"), false));
    assert!(!config_is_inert(&entry("local", "core.pager\nx"), false));
    assert!(!config_is_inert(&entry("worktree", "core.pager\nx"), false));
    assert!(!config_is_inert(&entry("", "core.bare\nfalse"), false));
    assert!(!config_is_inert(
        &entry("unknown", "core.bare\nfalse"),
        false
    ));
    // What a git without `--show-scope` prints: an entry where the scope would be.
    assert!(!config_is_inert(
        "core.bare\nfalse\0core.filemode\ntrue\0",
        false
    ));
    assert!(
        !config_is_inert("local\0core.bare\nfalse", false),
        "unterminated"
    );
    assert!(!config_is_inert("local\0", false), "an entry is missing");
    assert!(
        !config_is_inert(&entry("local", "bare\nfalse"), false),
        "no section"
    );
    assert!(config_is_inert(
        &entry("local", "remote.Origin.x.url\nu"),
        false
    ));
    assert!(!config_is_inert(
        &entry("local", "core.worktree\n../x"),
        false
    ));
    assert!(config_is_inert(
        &entry("local", "core.worktree\n../x"),
        true
    ));
}

/// Trust covers the directories below the one the user trusted. The git directory git finds must
/// lie inside trusted ground too, so a trusted subdirectory of an untrusted checkout, or a gitfile
/// that leads out of the trusted tree, is refused before the config is listed or the index read.
#[test]
fn a_git_directory_outside_trusted_ground_is_refused_first() {
    let (_dir, root) = repo_with(INERT);
    let sub = root.join("sub");
    std::fs::create_dir_all(&sub).unwrap();
    assert!(inspect_trusting(&sub, &|dir| dir.starts_with(&root)).is_some());
    assert!(
        inspect_trusting(&sub, &|dir| dir.starts_with(&sub)).is_none(),
        "the checkout above the trusted directory is not trusted"
    );
    let asked = std::cell::RefCell::new(Vec::new());
    let verdict = inspect_trusting(&root, &|dir| {
        asked.borrow_mut().push(dir.to_path_buf());
        false
    });
    assert_eq!(verdict, None);
    assert_eq!(
        asked.borrow().as_slice(),
        [root.join(".git")],
        "asked once, about the git directory"
    );

    // A gitfile that leads out of the trusted tree.
    let (_other, outside) = workspace();
    make_git_dir(&outside.join("gd"), INERT);
    std::fs::write(
        root.join("sub/.git"),
        format!("gitdir: {}\n", outside.join("gd").display()),
    )
    .unwrap();
    assert!(inspect_trusting(&sub, &|dir| dir.starts_with(&root)).is_none());
    assert!(inspect_trusting(&sub, &|_| true).is_some());
}

/// What a probe is started with, recorded by a stand-in for git that then runs the real one.
#[test]
fn every_probe_starts_with_the_hardening_on_and_no_provider_key() {
    const KEY: &str = "FORGE_GIT_PROBE_TEST_PROVIDER_KEY";
    const KEPT: &str = "FORGE_GIT_PROBE_TEST_USER_VARIABLE";
    let (_dir, root) = repo_with(INERT);
    let (_bin, bin) = workspace();
    let (git, log) = logging_git(&bin);
    forge_types::child_env::scrub(KEY);
    std::env::set_var(KEY, "sk-not-a-real-key");
    std::env::set_var(KEPT, "visible");
    let verdict = super::inspect(&root, &git, &|_| true);
    forge_types::child_env::unscrub(KEY);
    std::env::remove_var(KEY);
    std::env::remove_var(KEPT);
    assert!(verdict.is_some());
    let log = std::fs::read_to_string(log).unwrap();
    let calls: Vec<&str> = log.lines().filter(|l| l.starts_with("ARGS ")).collect();
    assert_eq!(
        calls.len(),
        4,
        "rev-parse, config, rev-parse, ls-files: {calls:?}"
    );
    for call in &calls {
        assert!(
            call.contains("--no-pager -c core.fsmonitor=false "),
            "{call}"
        );
    }
    assert!(!log.contains(KEY), "a provider key reached the probe");
    assert!(
        log.contains(&format!("{KEPT}=visible")),
        "control: the rest is inherited"
    );
    for set in [
        "GIT_NO_LAZY_FETCH=1",
        "GIT_ALLOW_PROTOCOL=none",
        "GIT_OPTIONAL_LOCKS=0",
        "GIT_TERMINAL_PROMPT=0",
    ] {
        assert!(log.contains(set), "{set}");
    }
}

/// A FIFO standing in for `.git/config` (an unpacked archive can carry one) blocks git on open; the
/// probe gives up at its budget and kills it.
#[test]
fn a_git_that_blocks_is_given_up_on_at_the_budget() {
    let (_dir, root) = repo_with(INERT);
    std::fs::remove_file(root.join(".git/config")).unwrap();
    let made = Command::new("mkfifo")
        .arg(root.join(".git/config"))
        .status()
        .expect("mkfifo");
    assert!(made.success());
    let started = std::time::Instant::now();
    assert_eq!(inspect(&root), None);
    let took = started.elapsed();
    assert!(
        BUDGET <= Duration::from_secs(2),
        "the decision runs on the turn's thread"
    );
    assert!(took >= BUDGET.mul_f32(0.5), "{took:?}");
    assert!(took < Duration::from_millis(2500), "{took:?}");
}
