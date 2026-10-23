//! The decision's dealings with the machine, as opposed to the text of the line: the trust gate,
//! which program a name in `PATH` becomes, what the git probe starts (and when it does not run at
//! all), and how long it can take. The rules on the text are in `tests.rs`.

use std::os::unix::fs::{symlink, PermissionsExt};
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::json;
use tempfile::TempDir;

use super::tests::{fixture, logging_git, rules, sane_path, write_repo, Fixture, INERT_CONFIG};
use super::*;

fn trusted(_: &Path) -> bool {
    true
}

fn untrusted(_: &Path) -> bool {
    false
}

fn host<'a>(f: &Fixture, path: OsString, trusted: &'a dyn Fn(&Path) -> bool) -> Host<'a> {
    Host {
        home: Some(f.home.clone()),
        path: Some(path),
        trusted,
    }
}

fn decide_args(f: &Fixture, args: serde_json::Value, host: Host<'_>) -> bool {
    auto_allows_on(&args, &rules(), Some(&f.root), host)
}

fn decide(f: &Fixture, command: &str, host: Host<'_>) -> bool {
    decide_args(f, json!({ "command": command }), host)
}

/// `sane_path()` with `dir` in front of it.
fn path_with(dir: &Path) -> OsString {
    let mut dirs = vec![dir.to_path_buf()];
    dirs.extend(std::env::split_paths(&sane_path()));
    std::env::join_paths(dirs).unwrap()
}

/// A directory outside the fixture's workspace, for programs the system supplies.
fn outside() -> (TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    (dir, root)
}

fn plant(path: &Path, body: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn calls(log: &Path) -> usize {
    std::fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .filter(|line| line.starts_with("ARGS "))
        .count()
}

// ---- the trust gate ----

#[test]
fn git_runs_unasked_only_in_a_project_the_user_trusted() {
    let f = fixture();
    for command in [
        "git status",
        "git diff",
        "git log --oneline -1",
        "git show HEAD",
        "git branch",
        "git rev-parse --show-toplevel",
        "git ls-files",
        "git grep -n add",
        "ls && git status",
        "git status | wc -l",
        "git --no-pager log -1",
    ] {
        assert!(
            decide(&f, command, host(&f, sane_path(), &trusted)),
            "trusted: {command}"
        );
        assert!(
            !decide(&f, command, host(&f, sane_path(), &untrusted)),
            "untrusted: {command}"
        );
    }
}

#[test]
fn the_probe_never_starts_git_in_an_untrusted_project() {
    let f = fixture();
    let (_dir, bin) = outside();
    let (_git, log) = logging_git(&bin);
    let path = path_with(&bin);
    for command in ["git status", "git diff", "ls && git log -1", "git grep add"] {
        assert!(!decide(&f, command, host(&f, path.clone(), &untrusted)));
    }
    assert_eq!(calls(&log), 0, "git ran in an untrusted project");
    assert!(decide(&f, "git status", host(&f, path.clone(), &trusted)));
    assert_eq!(
        calls(&log),
        4,
        "control: the probe runs once a project is trusted"
    );
}

// ---- the trust hint ----

fn hint(f: &Fixture, command: &str, host: Host<'_>) -> Option<&'static str> {
    trust_hint_on(
        &json!({ "command": command }),
        &rules(),
        Some(&f.root),
        host,
    )
}

#[test]
fn the_hint_names_git_when_trust_is_all_a_read_only_call_lacks() {
    let f = fixture();
    for command in [
        "git status",
        "git diff -- calc.py",
        "git log --oneline -3",
        "ls && git status",
        "git status | wc -l",
    ] {
        assert_eq!(
            hint(&f, command, host(&f, sane_path(), &untrusted)),
            Some("git"),
            "untrusted: {command}"
        );
        assert_eq!(
            hint(&f, command, host(&f, sane_path(), &trusted)),
            None,
            "a trusted project has nothing to be told: {command}"
        );
    }
}

#[test]
fn the_hint_stays_quiet_when_trust_would_not_make_the_call_run() {
    let f = fixture();
    for command in [
        // not read-only at all
        "git push",
        "git commit -m x",
        "git status > out.txt",
        "git -c core.pager=less log",
        "touch ccbench-marker.txt",
        "python -c 'print(1)'",
        // read-only and already runs without trust
        "ls",
        "cat calc.py",
        "grep -n add calc.py",
        // a secret the path guard refuses
        "cat .env",
    ] {
        assert_eq!(
            hint(&f, command, host(&f, sane_path(), &untrusted)),
            None,
            "{command}"
        );
    }
}

#[test]
fn the_hint_calls_a_non_git_trust_gated_binary_commands() {
    let f = fixture();
    assert_eq!(
        hint(&f, "file calc.py", host(&f, sane_path(), &untrusted)),
        Some("commands")
    );
}

#[test]
fn the_hint_respects_the_opt_out_and_never_starts_git() {
    let f = fixture();
    let (_dir, bin) = outside();
    let (_git, log) = logging_git(&bin);
    let path = path_with(&bin);
    assert_eq!(
        hint(&f, "git status", host(&f, path.clone(), &untrusted)),
        Some("git")
    );
    assert_eq!(
        calls(&log),
        0,
        "the hint must not run git in an untrusted project"
    );
    let opted_out = [forge_types::read_only_shell_opt_out_rule()];
    assert_eq!(
        trust_hint_on(
            &json!({ "command": "git status" }),
            &opted_out,
            Some(&f.root),
            host(&f, path, &untrusted)
        ),
        None,
        "auto_allow_read_only_shell = false: trust would not skip the prompt"
    );
}

/// What a repository can change about a binary decides whether it needs the trust. Adding a
/// binary to the list fails the completeness check below until its author says which it is.
const INDEPENDENT_OF_THE_REPOSITORY: &[(&str, &str)] = &[
    ("ls", "ls"),
    ("cat", "cat calc.py"),
    ("head", "head -n 1 calc.py"),
    ("tail", "tail -n 1 calc.py"),
    ("wc", "wc -l calc.py"),
    ("pwd", "pwd"),
    ("echo", "echo hi"),
    ("stat", "stat calc.py"),
    ("du", "du -s src"),
    ("nl", "nl calc.py"),
    ("tac", "tac calc.py"),
    ("cut", "cut -c1-3 calc.py"),
    ("tr", "tr a b"),
    ("sort", "sort calc.py"),
    ("uniq", "uniq calc.py"),
    ("basename", "basename calc.py"),
    ("dirname", "dirname calc.py"),
    ("realpath", "realpath calc.py"),
    ("readlink", "readlink calc.py"),
    ("which", "which ls"),
    ("md5sum", "md5sum calc.py"),
    ("sha1sum", "sha1sum calc.py"),
    ("sha256sum", "sha256sum calc.py"),
    ("b3sum", "b3sum calc.py"),
    ("diff", "diff calc.py calc.py"),
    ("cmp", "cmp calc.py calc.py"),
    ("grep", "grep -rn add src"),
    ("egrep", "egrep -n add calc.py"),
    ("fgrep", "fgrep -n add calc.py"),
    ("rg", "rg -n add src"),
    ("find", "find src -name '*.rs'"),
    ("sed", "sed -n '1,2p' calc.py"),
    ("printf", "printf '%s' x"),
    ("column", "column -t calc.py"),
    ("rev", "rev calc.py"),
    ("fold", "fold calc.py"),
    ("expand", "expand calc.py"),
    ("comm", "comm calc.py calc.py"),
    ("paste", "paste calc.py calc.py"),
    ("nproc", "nproc"),
    ("whoami", "whoami"),
    ("uname", "uname -a"),
    ("id", "id -u"),
    ("df", "df -h ."),
    ("hostname", "hostname -s"),
];

#[test]
fn every_other_binary_is_judged_by_its_arguments_alone_in_an_untrusted_project() {
    let f = fixture();
    for name in ALLOWED {
        if NEEDS_TRUST.contains(name) {
            continue;
        }
        let (_, sample) = INDEPENDENT_OF_THE_REPOSITORY
            .iter()
            .find(|(binary, _)| binary == name)
            .unwrap_or_else(|| {
                panic!(
                    "`{name}` is on ALLOWED: decide whether a repository can influence what it \
                     does (then it belongs in NEEDS_TRUST) and add a sample here"
                )
            });
        assert!(
            decide(&f, sample, host(&f, sane_path(), &untrusted)),
            "untrusted: {sample}"
        );
    }
    for (name, _) in INDEPENDENT_OF_THE_REPOSITORY {
        assert!(ALLOWED.contains(name), "{name} is not on ALLOWED");
        assert!(!NEEDS_TRUST.contains(name), "{name} needs trust");
    }
    for name in NEEDS_TRUST {
        assert!(ALLOWED.contains(name), "{name} is not on ALLOWED");
    }
}

#[test]
fn the_binaries_a_repository_can_influence_ask_in_an_untrusted_project() {
    let f = fixture();
    for command in [
        "file calc.py",
        "git status",
        "ls; file calc.py",
        "cat calc.py | file -",
        "git log -1 && ls",
    ] {
        assert!(
            decide(&f, command, host(&f, sane_path(), &trusted)),
            "trusted: {command}"
        );
        assert!(
            !decide(&f, command, host(&f, sane_path(), &untrusted)),
            "untrusted: {command}"
        );
    }
}

#[test]
fn a_secret_guard_and_the_text_rules_still_apply_in_an_untrusted_project() {
    let f = fixture();
    for command in ["cat .env", "grep -rn add ..", "ls > out", "wc -c .env"] {
        assert!(
            !decide(&f, command, host(&f, sane_path(), &untrusted)),
            "{command}"
        );
    }
}

#[test]
fn trust_must_cover_the_workspace_and_the_directory_the_command_runs_in() {
    let f = fixture();
    let other = f.root.parent().unwrap().join("other");
    write_repo(&other, INERT_CONFIG);
    let args = json!({ "command": "git status", "cwd": other.to_string_lossy() });
    let workspace_only = |dir: &Path| dir.starts_with(&f.root);
    let other_only = |dir: &Path| dir.starts_with(&other);
    let both = |dir: &Path| dir.starts_with(&f.root) || dir.starts_with(&other);
    let ask = |trusted: &dyn Fn(&Path) -> bool| {
        decide_args(&f, args.clone(), host(&f, sane_path(), trusted))
    };
    assert!(!ask(&workspace_only), "a cwd the user never vouched for");
    assert!(!ask(&other_only), "a workspace the user never vouched for");
    assert!(ask(&both));
    // `file` has no git directory to be judged by: only the two checks above stand between a
    // trusted workspace and a `cwd` that leads somewhere nobody vouched for.
    let file = json!({ "command": "file calc.py", "cwd": other.to_string_lossy() });
    assert!(!decide_args(
        &f,
        file.clone(),
        host(&f, sane_path(), &workspace_only)
    ));
    assert!(decide_args(&f, file, host(&f, sane_path(), &both)));
    // Inside the workspace a plain `cwd` needs no more than the workspace.
    let inside = json!({ "command": "git status", "cwd": "src" });
    assert!(decide_args(
        &f,
        inside,
        host(&f, sane_path(), &workspace_only)
    ));
}

/// The decision asks the trust store every time it is made. Real store, real `is_trusted`.
static STORE: Mutex<()> = Mutex::new(());

struct RealStore(#[allow(dead_code)] TempDir);

impl RealStore {
    /// `None` when the environment trusts every project (`FORGE_TRUST_PROJECT`), which these tests
    /// cannot tell from a trusted one.
    fn open() -> Option<RealStore> {
        if std::env::var(forge_config::project_trust::TRUST_ENV).is_ok() {
            return None;
        }
        let dir = tempfile::tempdir().unwrap();
        stand_in_for_trust(None);
        forge_config::project_trust::override_store_path(Some(
            dir.path().join("trusted-projects.json"),
        ));
        Some(RealStore(dir))
    }
}

impl Drop for RealStore {
    fn drop(&mut self) {
        forge_config::project_trust::override_store_path(None);
    }
}

fn git_in(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}");
}

#[test]
fn a_project_whose_trust_is_revoked_asks_again_on_the_very_next_call() {
    let _store_lock = STORE.lock().unwrap_or_else(|e| e.into_inner());
    let f = fixture();
    let Some(_store) = RealStore::open() else {
        return;
    };
    let (_dir, bin) = outside();
    let (_git, log) = logging_git(&bin);
    let path = path_with(&bin);
    let decide_now = || {
        decide(
            &f,
            "git status",
            host(&f, path.clone(), &project_is_trusted),
        )
    };

    assert!(!decide_now(), "a project nobody trusted");
    assert_eq!(calls(&log), 0);

    forge_config::project_trust::trust(&f.root).unwrap();
    assert!(decide_now());
    assert!(
        decide_now(),
        "and again, nothing is remembered but the store"
    );
    assert_eq!(calls(&log), 8);

    std::fs::remove_file(&log).unwrap();
    assert!(forge_config::project_trust::revoke(&f.root).unwrap());
    assert!(!decide_now(), "revoked between two calls");
    assert_eq!(
        calls(&log),
        0,
        "the probe ran git in a project that is no longer trusted"
    );

    // Trust covers everything below, and a directory outside it gets none.
    forge_config::project_trust::trust(&f.root).unwrap();
    assert!(decide_args(
        &f,
        json!({ "command": "git status", "cwd": "src" }),
        host(&f, path.clone(), &project_is_trusted)
    ));
    assert!(!decide_args(
        &f,
        json!({ "command": "git status", "cwd": f.home.to_string_lossy() }),
        host(&f, path.clone(), &project_is_trusted)
    ));
}

#[test]
fn a_linked_worktree_has_the_trust_of_the_checkout_it_belongs_to() {
    let _store_lock = STORE.lock().unwrap_or_else(|e| e.into_inner());
    let f = fixture();
    let Some(_store) = RealStore::open() else {
        return;
    };
    let (_dir, base) = outside();
    let main = base.join("main");
    std::fs::create_dir_all(&main).unwrap();
    git_in(&main, &["init", "-q", "-b", "main", "."]);
    git_in(
        &main,
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
    let linked = base.join("linked");
    git_in(&main, &["worktree", "add", "-q", "--detach", "../linked"]);
    let at = |dir: &Path| {
        auto_allows_on(
            &json!({ "command": "git status" }),
            &rules(),
            Some(dir),
            host(&f, sane_path(), &project_is_trusted),
        )
    };
    assert!(!at(&linked));
    forge_config::project_trust::trust(&main).unwrap();
    assert!(at(&main));
    assert!(at(&linked), "the linked worktree inherits the trust");
    forge_config::project_trust::revoke(&main).unwrap();
    assert!(!at(&main));
    assert!(!at(&linked));
}

// ---- which program a name in PATH becomes ----

/// A planted `name` for every way `PATH` can lead the shell into the checkout, and the line that
/// starts it. `pwd` starts nothing but `sh` itself.
const PLANTED: &[(&str, &str)] = &[
    ("ls", "ls"),
    ("cat", "cat calc.py"),
    ("grep", "grep -n add calc.py"),
    // Scripts that run `grep` through `PATH`.
    ("grep", "egrep -n add calc.py"),
    ("grep", "fgrep -n add calc.py"),
    ("wc", "wc -l calc.py"),
    ("git", "git status"),
    ("sh", "pwd"),
    ("rtk", "cat calc.py"),
];

#[test]
fn a_program_the_checkout_supplies_is_never_taken_for_the_systems() {
    let f = fixture();
    let ws = f.root.to_string_lossy().into_owned();
    let sane = sane_path().to_string_lossy().into_owned();
    // The workspace again, reached through a link that lies outside it.
    let linked = f.root.parent().unwrap().join("lnk");
    symlink(&f.root, &linked).unwrap();
    let variants = [
        ("relative entry", format!("bin:{sane}")),
        ("empty entry", format!(":{sane}")),
        ("trailing empty entry", format!("{sane}:")),
        ("dot", format!(".:{sane}")),
        ("absolute inside the workspace", format!("{ws}/bin:{sane}")),
        (
            "absolute, through a link",
            format!("{}/bin:{sane}", linked.display()),
        ),
    ];
    for (name, command) in PLANTED {
        for (label, path) in &variants {
            // Control: nothing planted, the same `PATH`.
            assert!(
                decide(&f, command, host(&f, OsString::from(path), &trusted)),
                "control: {command} via {label}"
            );
        }
        // A planted file where the shell looks: in `bin/` and in the working directory itself.
        plant(&f.root.join("bin").join(name), "#!/bin/sh\nexit 1\n");
        plant(&f.root.join(name), "#!/bin/sh\nexit 1\n");
        for (label, path) in &variants {
            // The empty entry at the end of PATH is looked up last, behind the system's own.
            if *label == "trailing empty entry" {
                continue;
            }
            assert!(
                !decide(&f, command, host(&f, OsString::from(path), &trusted)),
                "planted `{name}` via {label} runs when `{command}` does"
            );
        }
        std::fs::remove_file(f.root.join("bin").join(name)).unwrap();
        std::fs::remove_file(f.root.join(name)).unwrap();
    }
}

#[test]
fn a_planted_program_the_line_does_not_start_does_not_matter() {
    let f = fixture();
    plant(&f.root.join("bin/ls"), "#!/bin/sh\nexit 1\n");
    let path = OsString::from(format!("bin:{}", sane_path().to_string_lossy()));
    assert!(decide(&f, "cat calc.py", host(&f, path.clone(), &trusted)));
    assert!(!decide(&f, "ls", host(&f, path.clone(), &trusted)));
    assert!(!decide(&f, "cat calc.py | ls", host(&f, path, &trusted)));
}

#[test]
fn a_planted_program_is_refused_in_an_untrusted_project_too() {
    let f = fixture();
    plant(&f.root.join("bin/cat"), "#!/bin/sh\nexit 1\n");
    let path = OsString::from(format!("bin:{}", sane_path().to_string_lossy()));
    assert!(!decide(&f, "cat calc.py", host(&f, path, &untrusted)));
}

#[test]
fn a_virtualenv_that_holds_no_system_program_costs_nothing() {
    let f = fixture();
    plant(&f.root.join(".venv/bin/python"), "#!/bin/sh\n");
    plant(&f.root.join(".venv/bin/pip"), "#!/bin/sh\n");
    let path = OsString::from(format!(
        "{}/.venv/bin:{}",
        f.root.display(),
        sane_path().to_string_lossy()
    ));
    for command in ["ls", "git status", "cat calc.py", "grep -rn add src"] {
        assert!(
            decide(&f, command, host(&f, path.clone(), &trusted)),
            "{command}"
        );
    }
}

#[test]
fn the_probe_never_runs_a_git_the_checkout_supplies() {
    let f = fixture();
    let ran = f.root.join("planted-git-ran");
    let planted = format!("#!/bin/sh\ntouch '{}'\nexit 1\n", ran.display());
    plant(&f.root.join("bin/git"), &planted);
    plant(&f.root.join("git"), &planted);
    let sane = sane_path().to_string_lossy().into_owned();
    for path in [
        format!("{}/bin:{sane}", f.root.display()),
        format!("bin:{sane}"),
        format!(":{sane}"),
        format!(".:{sane}"),
    ] {
        assert!(
            !decide(&f, "git status", host(&f, OsString::from(&path), &trusted)),
            "{path}"
        );
        assert!(!ran.exists(), "the planted git ran with PATH={path}");
    }
}

#[test]
fn no_git_on_the_path_asks_and_an_unset_path_leaves_the_other_programs_alone() {
    let f = fixture();
    let (_dir, empty) = outside();
    let no_git = std::env::join_paths([&empty]).unwrap();
    assert!(!decide(&f, "git status", host(&f, no_git, &trusted)));
    let unset = Host {
        home: Some(f.home.clone()),
        path: None,
        trusted: &trusted,
    };
    assert!(decide(&f, "cat calc.py", unset));
    let unset = Host {
        home: Some(f.home.clone()),
        path: None,
        trusted: &trusted,
    };
    assert!(!decide(&f, "git status", unset));
}

// ---- what the probe starts, and when it runs ----

#[test]
fn the_probe_starts_git_without_provider_keys_and_with_lazy_fetch_and_transports_off() {
    const KEY: &str = "FORGE_ASK_SHELL_TEST_PROVIDER_KEY";
    const KEPT: &str = "FORGE_ASK_SHELL_TEST_USER_VARIABLE";
    let f = fixture();
    let (_dir, bin) = outside();
    let (_git, log) = logging_git(&bin);
    forge_types::child_env::scrub(KEY);
    std::env::set_var(KEY, "sk-not-a-real-key");
    std::env::set_var(KEPT, "visible");
    let allowed = decide(&f, "git status", host(&f, path_with(&bin), &trusted));
    forge_types::child_env::unscrub(KEY);
    std::env::remove_var(KEY);
    std::env::remove_var(KEPT);
    assert!(allowed);
    let text = std::fs::read_to_string(&log).unwrap();
    assert_eq!(calls(&log), 4);
    assert!(!text.contains(KEY), "a provider key reached the probe");
    assert!(
        text.contains(&format!("{KEPT}=visible")),
        "control: the rest is inherited"
    );
    for set in ["GIT_NO_LAZY_FETCH=1", "GIT_ALLOW_PROTOCOL=none"] {
        assert!(text.contains(set), "{set}");
    }
    for call in text.lines().filter(|l| l.starts_with("ARGS ")) {
        assert!(call.contains("-c core.fsmonitor=false"), "{call}");
    }
}

#[test]
fn a_line_that_asks_anyway_never_reaches_the_probe() {
    let f = fixture();
    let (_dir, bin) = outside();
    let (_git, log) = logging_git(&bin);
    let path = path_with(&bin);
    for command in [
        "git status && cargo build",
        "cargo build && git status",
        "git status; touch x",
        "git status && cat .env",
        "git status > out",
        "git status && cat .e*",
        "git status | tee x",
        "git -C .. status",
        "git status && git commit -m x",
    ] {
        assert!(
            !decide(&f, command, host(&f, path.clone(), &trusted)),
            "{command}"
        );
    }
    assert!(!decide_args(
        &f,
        json!({ "commands": ["git status", "touch x"] }),
        host(&f, path.clone(), &trusted)
    ));
    assert!(!decide_args(
        &f,
        json!({ "command": "git status", "background": true }),
        host(&f, path.clone(), &trusted)
    ));
    assert_eq!(calls(&log), 0, "the probe ran for a line that asks anyway");
    assert!(decide(&f, "git status", host(&f, path.clone(), &trusted)));
    assert_eq!(calls(&log), 4, "control: it runs for a line that passes");
    // Once per call, however many git statements the call holds.
    std::fs::remove_file(&log).unwrap();
    assert!(decide(
        &f,
        "git status && git log -1 && git diff && git branch",
        host(&f, path, &trusted)
    ));
    assert_eq!(calls(&log), 4);
}

#[test]
fn a_config_that_blocks_cannot_stall_the_decision_past_the_probe_budget() {
    let f = fixture();
    let config = f.root.join(".git/config");
    std::fs::remove_file(&config).unwrap();
    let made = Command::new("mkfifo")
        .arg(&config)
        .status()
        .expect("mkfifo");
    assert!(made.success());

    let started = Instant::now();
    assert!(!decide(&f, "git status", host(&f, sane_path(), &trusted)));
    let took = started.elapsed();
    assert!(took < Duration::from_millis(2500), "{took:?}");

    // A line that asks anyway does not wait for git at all, nor does one that never needs it.
    let started = Instant::now();
    assert!(!decide(
        &f,
        "git status && cargo build",
        host(&f, sane_path(), &trusted)
    ));
    assert!(decide(&f, "cat calc.py", host(&f, sane_path(), &trusted)));
    assert!(
        started.elapsed() < Duration::from_millis(500),
        "{:?}",
        started.elapsed()
    );
}

/// A nested repository that moves its work tree is refused through the whole decision too, and a
/// line that does not use git is never judged by it.
#[test]
fn a_nested_repository_with_its_work_tree_elsewhere_asks() {
    let f = fixture();
    let (_dir, base) = outside();
    let top = base.join("top");
    let victim = base.join("victim");
    std::fs::create_dir_all(victim.join(".ssh")).unwrap();
    std::fs::write(victim.join(".ssh/id_rsa"), "secret\n").unwrap();
    std::fs::create_dir_all(top.join("sub")).unwrap();
    git_in(&top, &["init", "-q", "-b", "main", "."]);
    git_in(&top.join("sub"), &["init", "-q", "-b", "main", "."]);
    git_in(
        &top.join("sub"),
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
    git_in(&top, &["add", "sub"]);
    let trusted_top = |dir: &Path| dir.starts_with(&top);
    let at = |args: serde_json::Value| {
        auto_allows_on(
            &args,
            &rules(),
            Some(&top),
            host(&f, sane_path(), &trusted_top),
        )
    };
    let status = json!({ "command": "git status --short" });
    assert!(at(status.clone()), "control: a nested repository in place");
    git_in(
        &top.join("sub"),
        &["config", "core.worktree", victim.to_str().unwrap()],
    );
    assert!(!at(status));
    assert!(at(json!({ "command": "ls" })));
}

// ---- unicode spaces are not the shell's blanks ----

/// Spaces `char::is_whitespace` knows and `sh` does not split words on.
const UNICODE_SPACES: &[char] = &[
    '\u{a0}', '\u{85}', '\u{b}', '\u{c}', '\r', '\u{1680}', '\u{2000}', '\u{2001}', '\u{2002}',
    '\u{2003}', '\u{2004}', '\u{2005}', '\u{2006}', '\u{2007}', '\u{2008}', '\u{2009}', '\u{200a}',
    '\u{2028}', '\u{2029}', '\u{202f}', '\u{205f}', '\u{3000}',
];

fn ask_allows(f: &Fixture, command: &str) -> bool {
    decide(f, command, host(f, sane_path(), &trusted))
}

/// `sh` reads `>&2<nbsp>h/out` as one word, `2<nbsp>h/out`, and `bash` redirects both streams into
/// a file of that name. A reading that ends the redirect at any Unicode space strips `>&2` as
/// harmless and judges the rest as a lone argument.
#[test]
fn a_redirect_ends_at_a_blank_and_not_at_a_unicode_space() {
    let f = fixture();
    for space in UNICODE_SPACES {
        for command in [
            format!("printf x >&2{space}h/out"),
            format!("printf x >&1{space}out"),
            format!("printf x >&-{space}out"),
            format!("printf x 2>&1{space}out"),
            format!("printf x >/dev/null{space}out"),
            format!("printf x 2>/dev/null{space}out"),
            format!("printf x >> /dev/null{space}out"),
            format!("ls >&2{space}"),
        ] {
            assert!(!ask_allows(&f, &command), "{command:?}");
            assert!(
                !crate::permission::readonly_shell::is_read_only_shell(&command),
                "plan mode: {command:?}"
            );
        }
    }
    for command in [
        "printf x >&2",
        "printf x >&2 ; ls",
        "printf x >&2\tcalc.py",
        "ls 2>&1 | wc -l",
        "ls >/dev/null && cat calc.py",
        "ls 2>/dev/null",
    ] {
        assert!(ask_allows(&f, command), "control: {command:?}");
    }
}

/// A trailing Unicode space is part of the last word for `sh`, so `cat plain<nbsp>` reads the file
/// of that name, which a repository can make a symlink to a secret. `str::trim` dropped it and
/// judged `cat plain`.
#[test]
fn a_unicode_space_at_the_end_of_a_line_is_part_of_the_name_the_shell_reads() {
    let f = fixture();
    for space in UNICODE_SPACES {
        if matches!(space, '\r' | '\u{b}' | '\u{c}' | '\u{85}') {
            continue; // control characters, refused outright
        }
        let link = f.root.join(format!("plain{space}"));
        symlink(f.root.join(".env"), &link).unwrap();
        for command in [
            format!("cat plain{space}"),
            format!("cat calc.py; cat plain{space}"),
            format!("cat plain{space} | head -1"),
            format!("ls && cat plain{space}"),
        ] {
            assert!(!ask_allows(&f, &command), "{command:?}");
        }
        // A space in front of the command is part of its name: no such program.
        assert!(!ask_allows(&f, &format!("{space}ls")), "{space:?}ls");
        assert!(!ask_allows(&f, &format!("ls;{space}ls")), "ls;{space:?}ls");
        std::fs::remove_file(link).unwrap();
    }
    assert!(ask_allows(&f, "cat calc.py "), "control: an ASCII blank");
    assert!(ask_allows(&f, "  ls\t"), "control: ASCII blanks around it");
}
