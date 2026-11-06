use std::os::unix::fs::PermissionsExt as _;
use std::path::PathBuf;

use super::super::test_support::*;
use super::*;

struct Fixture {
    _dir: tempfile::TempDir,
    work: PathBuf,
    remote: PathBuf,
    gh_log: PathBuf,
    body_file: PathBuf,
    gh: PathBuf,
    state: Arc<DaemonState>,
    session: String,
}

fn sh(cwd: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(cwd)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

const NO_PRS: &str = "[]";
const OPEN_PR: &str =
    r#"[{"url":"https://github.com/acme/shop/pull/3","isCrossRepository":false}]"#;

/// A repository whose GitHub URL is rewritten to a bare remote, a fake `gh`, and a session whose
/// worktree is on `feat/cart` (or detached / on `main`, per `mode`) with uncommitted edits.
fn fixture(mode: &str, permission: &str, pr_list: &str) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let remote = root.join("remote.git");
    let main = root.join("main");
    std::fs::create_dir_all(&main).unwrap();
    sh(
        &root,
        &[
            "init",
            "-q",
            "--bare",
            "-b",
            "main",
            remote.to_str().unwrap(),
        ],
    );
    sh(&main, &["init", "-q", "-b", "main"]);
    for (key, value) in [
        ("user.name", "Test"),
        ("user.email", "test@example.com"),
        ("commit.gpgsign", "false"),
        ("remote.origin.url", "https://github.com/acme/shop.git"),
        ("remote.origin.fetch", "+refs/heads/*:refs/remotes/origin/*"),
    ] {
        sh(&main, &["config", key, value]);
    }
    let rewrite = format!("url.{}.insteadOf", remote.display());
    sh(
        &main,
        &["config", &rewrite, "https://github.com/acme/shop.git"],
    );
    std::fs::write(main.join("README.md"), "shop\n").unwrap();
    sh(&main, &["add", "README.md"]);
    sh(&main, &["commit", "-q", "-m", "init"]);
    sh(&main, &["push", "-q", "-u", "origin", "main"]);

    let work = match mode {
        "main" => main.clone(),
        _ => {
            let wt = root.join("wt");
            sh(
                &main,
                &[
                    "worktree",
                    "add",
                    "-q",
                    "-b",
                    "feat/cart",
                    wt.to_str().unwrap(),
                ],
            );
            if mode == "detached" {
                sh(&wt, &["checkout", "-q", "--detach"]);
            }
            wt
        }
    };
    std::fs::write(work.join("cart.rs"), "fn cart() {}\n").unwrap();
    std::fs::write(work.join("README.md"), "shop\ncart\n").unwrap();
    std::fs::create_dir_all(work.join(".forge")).unwrap();
    std::fs::write(work.join(".forge/state.json"), "{}").unwrap();

    let gh_log = root.join("gh.log");
    let body_file = root.join("body.md");
    let gh = root.join("gh");
    let script = format!(
        r#"#!/bin/sh
echo "$*" >> '{log}'
case "$1 $2" in
  "repo view") echo '{{"viewerPermission":"{permission}","defaultBranchRef":{{"name":"main"}},"isArchived":false}}' ;;
  "pr list") echo '{pr_list}' ;;
  "pr create") cat > '{body}'; echo "https://github.com/acme/shop/pull/9" ;;
  *) echo "unexpected gh call: $*" >&2; exit 1 ;;
esac
"#,
        log = gh_log.display(),
        body = body_file.display(),
    );
    std::fs::write(&gh, script).unwrap();
    std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755)).unwrap();

    let store = Arc::new(forge_store::Store::open_in_memory().unwrap());
    let session = store
        .create_session(&main.display().to_string(), "default")
        .unwrap();
    if mode != "main" {
        store
            .set_session_worktree(&session, &work.display().to_string())
            .unwrap();
    }
    let state = state(store, &main);
    Fixture {
        _dir: dir,
        work,
        remote,
        gh_log,
        body_file,
        gh,
        state,
        session,
    }
}

async fn ship(fx: &Fixture, request: ShipRequest) -> ShipResponse {
    ship_session(&fx.state, &fx.session, &request, &fx.gh)
        .await
        .unwrap()
}

fn step_names(response: &ShipResponse) -> Vec<(&'static str, &'static str)> {
    response.steps.iter().map(|s| (s.step, s.status)).collect()
}

fn gh_calls(fx: &Fixture) -> String {
    std::fs::read_to_string(&fx.gh_log).unwrap_or_default()
}

#[tokio::test]
async fn a_finished_worktree_session_ships_to_a_watched_pr() {
    let fx = fixture("worktree", "WRITE", NO_PRS);
    let response = ship(&fx, ShipRequest::default()).await;

    assert!(response.ok, "{:?}", response.refused.map(|r| r.reason));
    assert_eq!(
        step_names(&response),
        [
            ("branch", "done"),
            ("commit", "done"),
            ("push", "done"),
            ("pr", "done"),
            ("watch", "done")
        ]
    );
    let commit = response.commit.unwrap();
    assert_eq!(sh(&fx.work, &["rev-parse", "HEAD"]), commit.sha);
    assert!(sh(&fx.work, &["status", "--porcelain"]).contains(".forge"));
    let files = sh(&fx.work, &["show", "--name-only", "--format=", "HEAD"]);
    assert!(
        files.contains("cart.rs") && files.contains("README.md"),
        "{files}"
    );
    assert!(!files.contains(".forge"), "{files}");
    assert_eq!(
        sh(&fx.remote, &["rev-parse", "refs/heads/feat/cart"]),
        commit.sha
    );

    let calls = gh_calls(&fx);
    assert!(calls.contains("pr create"), "{calls}");
    assert!(calls.contains("--head feat/cart"), "{calls}");
    let pr = response.pr.unwrap();
    assert_eq!((pr.number, pr.existing), (Some(9), false));
    assert!(calls.contains(&format!("--title {}", pr.title)), "{calls}");
    let body = std::fs::read_to_string(&fx.body_file).unwrap();
    assert!(body.contains("### Provenance"), "{body}");
    assert!(body.contains(&fx.session), "{body}");

    let watch = crate::serve_pr_watch::registry().get(&fx.session).unwrap();
    assert_eq!(watch.mode, Mode::Merge);
    assert_eq!(response.watch.unwrap().method, "squash");
}

#[tokio::test]
async fn given_message_and_title_override_the_generated_ones() {
    let fx = fixture("worktree", "WRITE", NO_PRS);
    let response = ship(
        &fx,
        ShipRequest {
            message: Some("feat(cart): add the cart".into()),
            title: Some("Add the cart".into()),
            method: Some("rebase".into()),
            ..ShipRequest::default()
        },
    )
    .await;
    assert!(response.ok);
    assert_eq!(
        sh(&fx.work, &["log", "-1", "--format=%s"]),
        "feat(cart): add the cart"
    );
    assert!(gh_calls(&fx).contains("--title Add the cart"));
    assert_eq!(response.watch.unwrap().method, "rebase");
}

#[tokio::test]
async fn an_existing_pr_is_updated_by_the_push() {
    let fx = fixture("worktree", "WRITE", OPEN_PR);
    let response = ship(&fx, ShipRequest::default()).await;
    assert!(response.ok);
    let pr = response.pr.unwrap();
    assert_eq!((pr.number, pr.existing), (Some(3), true));
    assert!(!gh_calls(&fx).contains("pr create"));
}

#[tokio::test]
async fn a_detached_head_is_refused_before_anything_is_committed() {
    let fx = fixture("detached", "WRITE", NO_PRS);
    let head = sh(&fx.work, &["rev-parse", "HEAD"]);
    let response = ship(&fx, ShipRequest::default()).await;

    assert!(!response.ok);
    let refused = response.refused.unwrap();
    assert_eq!(refused.step, "branch");
    assert!(refused.reason.contains("detached"), "{}", refused.reason);
    assert_eq!(sh(&fx.work, &["rev-parse", "HEAD"]), head);
    assert!(gh_calls(&fx).is_empty());
    assert!(crate::serve_pr_watch::registry().get(&fx.session).is_none());
}

#[tokio::test]
async fn the_base_branch_is_refused_and_its_edits_stay_uncommitted() {
    let fx = fixture("main", "WRITE", NO_PRS);
    let head = sh(&fx.work, &["rev-parse", "HEAD"]);
    let response = ship(&fx, ShipRequest::default()).await;

    let refused = response.refused.unwrap();
    assert_eq!(refused.step, "branch");
    assert!(
        refused.reason.contains("create a branch first"),
        "{}",
        refused.reason
    );
    assert_eq!(sh(&fx.work, &["rev-parse", "HEAD"]), head);
    assert!(sh(&fx.work, &["status", "--porcelain"]).contains("cart.rs"));
    assert!(gh_calls(&fx).is_empty());
}

#[tokio::test]
async fn a_failing_commit_hook_stops_before_any_push() {
    let fx = fixture("worktree", "WRITE", NO_PRS);
    let hooks = sh(&fx.work, &["rev-parse", "--git-common-dir"]);
    let hook = PathBuf::from(&hooks).join("hooks/pre-commit");
    let hook = if hook.is_absolute() {
        hook
    } else {
        fx.work.join(hook)
    };
    std::fs::create_dir_all(hook.parent().unwrap()).unwrap();
    std::fs::write(&hook, "#!/bin/sh\necho 'lint failed' >&2\nexit 1\n").unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();

    let response = ship(&fx, ShipRequest::default()).await;
    let refused = response.refused.unwrap();
    assert_eq!(refused.step, "commit");
    assert!(refused.reason.contains("lint failed"), "{}", refused.reason);
    assert!(!response.steps.iter().any(|s| s.step == "push"));
    assert!(gh_calls(&fx).is_empty());
    assert!(sh(&fx.remote, &["branch", "--list", "feat/cart"]).is_empty());
}

#[tokio::test]
async fn read_only_access_is_refused_at_the_pr_step_without_pushing() {
    let fx = fixture("worktree", "READ", NO_PRS);
    let response = ship(&fx, ShipRequest::default()).await;
    let refused = response.refused.unwrap();
    assert_eq!(refused.step, "pr");
    assert!(
        refused.reason.contains("not write access"),
        "{}",
        refused.reason
    );
    assert!(response.commit.is_some());
    assert!(sh(&fx.remote, &["branch", "--list", "feat/cart"]).is_empty());
    assert!(crate::serve_pr_watch::registry().get(&fx.session).is_none());
}

#[tokio::test]
async fn a_branch_with_nothing_to_ship_is_refused_by_the_ahead_check() {
    let fx = fixture("worktree", "WRITE", NO_PRS);
    sh(&fx.work, &["checkout", "-q", "--", "README.md"]);
    std::fs::remove_file(fx.work.join("cart.rs")).unwrap();
    let response = ship(&fx, ShipRequest::default()).await;
    assert_eq!(step_names(&response)[1], ("commit", "skipped"));
    let refused = response.refused.unwrap();
    assert_eq!(refused.step, "pr");
    assert!(
        refused.reason.contains("no commits ahead"),
        "{}",
        refused.reason
    );
}

#[tokio::test]
async fn a_bad_method_and_an_unknown_session_do_not_start() {
    let fx = fixture("worktree", "WRITE", NO_PRS);
    let bad = ship_session(
        &fx.state,
        &fx.session,
        &ShipRequest {
            method: Some("fast-forward".into()),
            ..ShipRequest::default()
        },
        &fx.gh,
    )
    .await;
    assert_eq!(bad.err().unwrap().0, axum::http::StatusCode::BAD_REQUEST);
    let gone = ship_session(&fx.state, "nope", &ShipRequest::default(), &fx.gh).await;
    assert_eq!(gone.err().unwrap().0, axum::http::StatusCode::NOT_FOUND);
}

#[test]
fn staging_skips_forge_state_and_untracked_residue_but_refuses_conflicts() {
    let dir = repo_with(&[("a.txt", "a\n")]);
    std::fs::write(dir.path().join("a.txt"), "b\n").unwrap();
    std::fs::create_dir_all(dir.path().join(".forge")).unwrap();
    std::fs::write(dir.path().join(".forge/x"), "x").unwrap();
    std::fs::create_dir_all(dir.path().join("__pycache__")).unwrap();
    std::fs::write(dir.path().join("__pycache__/m.pyc"), "x").unwrap();
    std::fs::write(dir.path().join("new.rs"), "n").unwrap();
    let mut paths = pending_paths(dir.path()).unwrap();
    paths.sort();
    assert_eq!(paths, ["a.txt", "new.rs"]);
}

#[test]
fn model_replies_split_into_a_title_and_a_body() {
    let (title, body) =
        split_title_body("\n## Title: \"feat(cart): add cart\"\n\n- adds a cart\n- tests it\n")
            .unwrap();
    assert_eq!(title, "feat(cart): add cart");
    assert_eq!(body, "- adds a cart\n- tests it");
    assert!(split_title_body("  \n").is_none());
    let (title, _) = commit_log_text(&["feat: a".into(), "fix: b".into()]);
    assert_eq!(title, "feat: a");
}
