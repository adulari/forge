use std::path::Path;

use tempfile::TempDir;

use super::*;

const INERT: &str = "[core]\n\trepositoryformatversion = 0\n\tfilemode = true\n\tbare = false\n\
[remote \"origin\"]\n\turl = https://example.invalid/x.git\n\tfetch = +refs/heads/*:refs/remotes/origin/*\n\
[branch \"main\"]\n\tremote = origin\n\tmerge = refs/heads/main\n";

fn workspace() -> (TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    (dir, root)
}

fn make_git_dir(dir: &Path, config: &str) {
    std::fs::create_dir_all(dir.join("objects")).unwrap();
    std::fs::create_dir_all(dir.join("refs")).unwrap();
    std::fs::write(dir.join("HEAD"), "ref: refs/heads/main\n").unwrap();
    std::fs::write(dir.join("config"), config).unwrap();
}

fn repo_with(config: &str) -> (TempDir, PathBuf) {
    let (dir, root) = workspace();
    make_git_dir(&root.join(".git"), config);
    (dir, root)
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
[branch \"x\"]\n\tvscode-merge-base = origin/main\n[core] bare = false\n[core]\n\tbare\n"
    )));
}

#[test]
fn every_key_that_runs_a_program_or_reaches_out_is_refused() {
    for line in [
        "[core]\n\tfsmonitor = touch ran; echo\n",
        "[core]\n\tfsmonitor = \"true\"\n",
        "[core]\n\tfsmonitor = true # builtin\n",
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
        "[submodule \"m\"]\n\tupdate = !touch ran\n",
        "[core]\n\tbare = false \\\n\tmore\n",
        "[core\n\tbare = false\n",
        "[core \"x]\n\tbare = false\n",
        "bare = false\n",
        "[]\n\tbare = false\n",
        "[core]\n\t= false\n",
        "[unknown]\n\tkey = v\n",
    ] {
        assert!(!inert(&format!("{INERT}{line}")), "{line:?}");
    }
}

#[test]
fn the_builtin_fsmonitor_daemon_is_not_a_hook() {
    assert!(inert(&format!("{INERT}[core]\n\tfsmonitor = true\n")));
    assert!(inert(&format!("{INERT}[core]\n\tfsmonitor = false\n")));
    assert!(inert(&format!("{INERT}[core]\n\tfsmonitor\n")));
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

#[test]
fn the_worktree_config_is_read_too() {
    let (_dir, root) = repo_with(INERT);
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
fn the_work_tree_is_where_the_dot_git_is_and_a_bare_repository_has_none() {
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
    let linked = root.join("linked");
    std::fs::create_dir_all(&linked).unwrap();
    std::fs::write(
        linked.join(".git"),
        format!("gitdir: {}\n", root.join(".git").display()),
    )
    .unwrap();
    assert_eq!(
        inspect(&linked),
        Some(Repo {
            work_tree: Some(linked)
        })
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
    std::fs::write(nested.join("info"), "").unwrap_or_default();
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
fn a_gitdir_file_is_followed_to_the_repository_and_its_common_directory() {
    let (_dir, root) = workspace();
    let main = root.join("main");
    make_git_dir(&main.join(".git"), INERT);
    let admin = main.join(".git/worktrees/w");
    std::fs::create_dir_all(&admin).unwrap();
    std::fs::write(admin.join("HEAD"), "ref: refs/heads/w\n").unwrap();
    std::fs::write(admin.join("commondir"), "../..\n").unwrap();
    let linked = root.join("linked");
    std::fs::create_dir_all(&linked).unwrap();
    std::fs::write(
        linked.join(".git"),
        format!("gitdir: {}\n", admin.display()),
    )
    .unwrap();
    assert!(is_inert_at(&linked));
    std::fs::write(
        main.join(".git/config"),
        format!("{INERT}[core]\n\tfsmonitor = ./hook\n"),
    )
    .unwrap();
    assert!(
        !is_inert_at(&linked),
        "the shared config is the linked one's"
    );
    // A relative target is relative to the directory holding the file.
    std::fs::write(main.join(".git/config"), INERT).unwrap();
    std::fs::write(linked.join(".git"), "gitdir: ../main/.git/worktrees/w\n").unwrap();
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

#[test]
fn a_submodule_repository_is_judged_too_and_may_set_its_worktree() {
    let (_dir, root) = repo_with(INERT);
    let module = root.join(".git/modules/vendor/lib");
    make_git_dir(
        &module,
        "[core]\n\trepositoryformatversion = 0\n\tbare = false\n\tworktree = ../../../../vendor/lib\n",
    );
    assert!(is_inert_at(&root));
    std::fs::write(
        module.join("config"),
        "[core]\n\tbare = false\n\tfsmonitor = ./hook\n",
    )
    .unwrap();
    assert!(!is_inert_at(&root));
    std::fs::write(
        root.join(".git/config"),
        format!("{INERT}[core]\n\tworktree = ../elsewhere\n"),
    )
    .unwrap();
    std::fs::write(module.join("config"), "[core]\n\tbare = false\n").unwrap();
    assert!(
        !is_inert_at(&root),
        "only a submodule's own config names a worktree"
    );
}

#[test]
fn a_flood_of_modules_is_unprovable() {
    let (_dir, root) = repo_with(INERT);
    for n in 0..(MAX_MODULE_DIRS + 4) {
        std::fs::create_dir_all(root.join(format!(".git/modules/m{n}"))).unwrap();
    }
    assert!(!is_inert_at(&root));
}

#[test]
fn an_oversized_or_unreadable_config_is_refused() {
    let (_dir, root) = repo_with(INERT);
    let padding = "# x\n".repeat((MAX_CONFIG_BYTES as usize / 4) + 8);
    std::fs::write(root.join(".git/config"), format!("{INERT}{padding}")).unwrap();
    assert!(!is_inert_at(&root));
    std::fs::write(root.join(".git/config"), [0xff, 0xfe, 0x00]).unwrap();
    assert!(!is_inert_at(&root));
    std::fs::remove_file(root.join(".git/config")).unwrap();
    std::fs::create_dir(root.join(".git/config")).unwrap();
    assert!(!is_inert_at(&root));
}

#[test]
fn a_hooks_path_is_judged_by_the_directory_it_names() {
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
    for odd in ["", "\"x\"", "a # b", "a;b", "~someone/hooks"] {
        std::fs::write(
            root.join(".git/config"),
            format!("{INERT}[core]\n\thooksPath = {odd}\n"),
        )
        .unwrap();
        assert!(!is_inert_at(&root), "{odd:?}");
    }
    std::fs::write(
        root.join(".git/config"),
        format!("{INERT}[core]\n\thooksPath\n"),
    )
    .unwrap();
    assert!(!is_inert_at(&root));
}

#[test]
fn a_hooks_path_in_a_submodule_config_is_refused() {
    let (_dir, root) = repo_with(INERT);
    make_git_dir(
        &root.join(".git/modules/m"),
        "[core]\n\tbare = false\n\thooksPath = /tmp/none\n",
    );
    assert!(!is_inert_at(&root));
}

#[test]
fn the_keys_editors_and_forges_leave_in_a_clone_are_inert() {
    assert!(inert(&format!(
        "{INERT}[branch \"x\"]\n\tvscode-merge-base = origin/main\n\tgithub-pr-owner-number = \"o#r#1\"\n"
    )));
}
