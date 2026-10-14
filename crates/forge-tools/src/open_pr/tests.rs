//! `open_pr` against a real git repository whose GitHub URL is rewritten (`insteadOf`) to a local
//! bare remote, with a fake `gh` that logs its calls.

use super::*;

struct Fixture {
    _dir: tempfile::TempDir,
    work: PathBuf,
    remote: PathBuf,
    gh_log: PathBuf,
    body_file: PathBuf,
    gh: PathBuf,
}

fn git(cwd: &Path, args: &[&str]) -> String {
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

/// `permission` is what `gh repo view` reports; `pr_list` is what `gh pr list` prints.
fn fixture(permission: &str, pr_list: &str) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let remote = root.join("remote.git");
    let work = root.join("work");
    std::fs::create_dir_all(&work).unwrap();
    git(
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
    git(&work, &["init", "-q", "-b", "main"]);
    for (key, value) in [
        ("user.name", "Test"),
        ("user.email", "test@example.com"),
        ("commit.gpgsign", "false"),
        ("remote.origin.url", "https://github.com/acme/shop.git"),
        ("remote.origin.fetch", "+refs/heads/*:refs/remotes/origin/*"),
    ] {
        git(&work, &["config", key, value]);
    }
    let rewrite = format!("url.{}.insteadOf", remote.display());
    git(
        &work,
        &["config", &rewrite, "https://github.com/acme/shop.git"],
    );
    std::fs::write(work.join("README.md"), "shop\n").unwrap();
    git(&work, &["add", "README.md"]);
    git(&work, &["commit", "-q", "-m", "init"]);
    git(&work, &["push", "-q", "-u", "origin", "main"]);
    git(&work, &["switch", "-q", "-c", "feat/cart"]);
    std::fs::write(work.join("cart.rs"), "fn cart() {}\n").unwrap();
    git(&work, &["add", "cart.rs"]);
    git(&work, &["commit", "-q", "-m", "feat: cart"]);

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
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755)).unwrap();
    Fixture {
        _dir: dir,
        work,
        remote,
        gh_log,
        body_file,
        gh,
    }
}

fn ctx(fx: &Fixture, session: Option<&str>, settings: OpenPrSettings) -> Ctx {
    Ctx {
        git: PathBuf::from("git"),
        gh: fx.gh.clone(),
        settings,
        session: session.map(str::to_string),
        ledger: session.and_then(|s| Ledger::new(&fx.work, s)),
    }
}

fn args(title: &str) -> Args {
    Args {
        title: title.to_string(),
        body: "Adds a cart.".to_string(),
        base: None,
        draft: false,
        allow_untracked: false,
        publish_images: true,
    }
}

fn remote_has(fx: &Fixture, reference: &str) -> bool {
    std::process::Command::new("git")
        .args(["rev-parse", "--verify", "-q", reference])
        .current_dir(&fx.remote)
        .output()
        .unwrap()
        .status
        .success()
}

fn gh_calls(fx: &Fixture) -> String {
    std::fs::read_to_string(&fx.gh_log).unwrap_or_default()
}

#[tokio::test]
async fn opens_a_pr_with_published_evidence_and_leaves_the_tree_alone() {
    let fx = fixture("WRITE", NO_PRS);
    let ledger = Ledger::new(&fx.work, "sess-ok").unwrap();
    ledger
        .record(
            EvidenceKind::Screenshot,
            "browser",
            "png",
            b"\x89PNG fake",
            evidence::Capture {
                caption: Some("cart page"),
                ..Default::default()
            },
        )
        .unwrap();
    ledger.record_test_output("cargo test", 0, "ok").unwrap();
    ledger
        .record_preview("https://gone.trycloudflare.com", 4242, 5173)
        .unwrap();
    let head_before = git(&fx.work, &["rev-parse", "HEAD"]);
    let settings = OpenPrSettings {
        provenance: Some(Arc::new(|session: &str, diff: Option<&str>| {
            format!(
                "### Provenance\n- Session: `{session}`\n- Diff: {}\n",
                diff.unwrap_or("?")
            )
        })),
        redact: Some(Arc::new(|text: &str| {
            text.replace("cart page", "[redacted]")
        })),
        ..OpenPrSettings::default()
    };

    let text = open_pr(
        &ctx(&fx, Some("sess-ok"), settings),
        &fx.work,
        &args("feat: cart"),
    )
    .await
    .unwrap();

    assert!(
        text.starts_with("opened PR https://github.com/acme/shop/pull/9"),
        "{text}"
    );
    assert!(
        text.contains("`feat/cart` → `main` on acme/shop, 1 commit(s)"),
        "{text}"
    );
    assert!(
        text.contains("1 passing check(s), 1 screenshot(s) published to forge-evidence@"),
        "{text}"
    );
    assert!(remote_has(&fx, "refs/heads/feat/cart"));
    assert!(remote_has(&fx, "refs/heads/forge-evidence"));
    let published = git(
        &fx.remote,
        &["ls-tree", "-r", "--name-only", "forge-evidence"],
    );
    assert_eq!(published, "sess-ok/0001-browser.png");
    let evidence_commit = git(&fx.remote, &["rev-parse", "forge-evidence"]);
    assert_eq!(
        git(&fx.remote, &["rev-list", "--count", "forge-evidence"]),
        "1"
    );

    let body = std::fs::read_to_string(&fx.body_file).unwrap();
    assert!(
        body.starts_with("Adds a cart.\n\n---\n### Evidence (captured, not reviewed)\n"),
        "{body}"
    );
    assert!(body.contains("`cargo test` — exit 0"), "{body}");
    assert!(
        body.contains(&format!(
            "https://github.com/acme/shop/blob/{evidence_commit}/sess-ok/0001-browser.png?raw=true"
        )),
        "{body}"
    );
    assert!(
        !body.contains("cart page"),
        "redaction must run over the body: {body}"
    );
    assert!(
        body.contains("- 1 earlier preview URL(s) from this session have since closed"),
        "{body}"
    );
    assert!(
        body.contains(
            "### Provenance\n- Session: `sess-ok`\n- Diff: 1 file changed, 1 insertion(+)"
        ),
        "{body}"
    );

    let calls = gh_calls(&fx);
    assert!(
        calls.contains("repo view acme/shop --json viewerPermission,defaultBranchRef,isArchived"),
        "{calls}"
    );
    assert!(calls.contains("pr create --repo acme/shop --base main --head feat/cart --title feat: cart --body-file -"), "{calls}");
    assert_eq!(git(&fx.work, &["rev-parse", "HEAD"]), head_before);
    assert_eq!(git(&fx.work, &["status", "--porcelain"]), "");
    assert_eq!(
        git(&fx.work, &["symbolic-ref", "--short", "HEAD"]),
        "feat/cart"
    );
}

#[tokio::test]
async fn a_second_session_adds_to_the_evidence_branch() {
    let fx = fixture("ADMIN", NO_PRS);
    let repo = github::parse_remote_url("https://github.com/acme/shop").unwrap();
    let shot = fx.work.join(".forge-shot.png");
    std::fs::write(&shot, b"one").unwrap();
    let files = vec![("0001-browser.png".to_string(), shot.clone())];
    let first = evidence_branch::publish(Path::new("git"), &fx.work, "origin", &repo, "s1", &files)
        .await
        .unwrap();
    std::fs::write(&shot, b"two").unwrap();
    let second =
        evidence_branch::publish(Path::new("git"), &fx.work, "origin", &repo, "s2", &files)
            .await
            .unwrap();
    let again = evidence_branch::publish(Path::new("git"), &fx.work, "origin", &repo, "s2", &files)
        .await
        .unwrap();
    std::fs::remove_file(&shot).unwrap();

    assert_ne!(first.commit, second.commit);
    assert_eq!(
        again.commit, second.commit,
        "an identical publish makes no new commit"
    );
    let tree = git(
        &fx.remote,
        &["ls-tree", "-r", "--name-only", "forge-evidence"],
    );
    assert_eq!(tree, "s1/0001-browser.png\ns2/0001-browser.png");
    assert_eq!(
        git(&fx.remote, &["rev-parse", "forge-evidence^"]),
        first.commit
    );
    assert_eq!(git(&fx.work, &["status", "--porcelain"]), "");
}

#[tokio::test]
async fn dirty_or_untracked_trees_are_refused_before_anything_is_pushed() {
    let fx = fixture("WRITE", NO_PRS);
    std::fs::write(fx.work.join("cart.rs"), "fn cart() { todo!() }\n").unwrap();
    let error = open_pr(
        &ctx(&fx, None, OpenPrSettings::default()),
        &fx.work,
        &args("t"),
    )
    .await
    .unwrap_err();
    assert!(error.contains("uncommitted changes in cart.rs"), "{error}");
    git(&fx.work, &["checkout", "--", "cart.rs"]);

    std::fs::write(fx.work.join("scratch.txt"), "x").unwrap();
    std::fs::create_dir_all(fx.work.join(".forge/jobs")).unwrap();
    std::fs::write(fx.work.join(".forge/jobs/1.json"), "{}").unwrap();
    let error = open_pr(
        &ctx(&fx, None, OpenPrSettings::default()),
        &fx.work,
        &args("t"),
    )
    .await
    .unwrap_err();
    assert!(error.contains("untracked files: scratch.txt."), "{error}");
    assert!(!remote_has(&fx, "refs/heads/feat/cart"));
    assert_eq!(
        gh_calls(&fx),
        "",
        "nothing reaches gh before the local checks pass"
    );

    let mut allow = args("t");
    allow.allow_untracked = true;
    let text = open_pr(&ctx(&fx, None, OpenPrSettings::default()), &fx.work, &allow)
        .await
        .unwrap();
    assert!(text.contains("pull/9"), "{text}");
    let body = std::fs::read_to_string(&fx.body_file).unwrap();
    assert_eq!(body, "Adds a cart.\n\n---\n🔨 Created with Forge\n");
}

#[tokio::test]
async fn no_write_permission_means_no_push() {
    let fx = fixture("READ", NO_PRS);
    let error = open_pr(
        &ctx(&fx, None, OpenPrSettings::default()),
        &fx.work,
        &args("t"),
    )
    .await
    .unwrap_err();
    assert!(
        error.contains("READ access to acme/shop, not write access"),
        "{error}"
    );
    assert!(!remote_has(&fx, "refs/heads/feat/cart"));
    assert!(!gh_calls(&fx).contains("pr create"));
}

#[tokio::test]
async fn the_base_branch_itself_is_refused() {
    let fx = fixture("WRITE", NO_PRS);
    git(&fx.work, &["switch", "-q", "main"]);
    let error = open_pr(
        &ctx(&fx, None, OpenPrSettings::default()),
        &fx.work,
        &args("t"),
    )
    .await
    .unwrap_err();
    assert!(
        error.contains("on `main`, the base branch itself"),
        "{error}"
    );
}

#[tokio::test]
async fn an_open_pr_for_the_branch_gets_the_push_not_a_duplicate() {
    let fx = fixture("WRITE", OPEN_PR);
    let text = open_pr(
        &ctx(&fx, None, OpenPrSettings::default()),
        &fx.work,
        &args("t"),
    )
    .await
    .unwrap();
    assert!(
        text.contains("pushed `feat/cart` to the open PR https://github.com/acme/shop/pull/3"),
        "{text}"
    );
    assert!(remote_has(&fx, "refs/heads/feat/cart"));
    assert!(!gh_calls(&fx).contains("pr create"));
}

fn screenshot(fx: &Fixture, session: &str) -> Ledger {
    let ledger = Ledger::new(&fx.work, session).unwrap();
    ledger
        .record(
            EvidenceKind::Screenshot,
            "browser",
            "png",
            b"\x89PNG fake",
            evidence::Capture::default(),
        )
        .unwrap();
    ledger
}

#[tokio::test]
async fn a_forks_pr_from_a_same_named_branch_is_not_taken_for_ours() {
    let fx = fixture(
        "WRITE",
        r#"[{"url":"https://github.com/acme/shop/pull/4","isCrossRepository":true}]"#,
    );
    let text = open_pr(
        &ctx(&fx, None, OpenPrSettings::default()),
        &fx.work,
        &args("t"),
    )
    .await
    .unwrap();
    assert!(
        text.starts_with("opened PR https://github.com/acme/shop/pull/9"),
        "{text}"
    );
    let calls = gh_calls(&fx);
    assert!(calls.contains("--json url,isCrossRepository"), "{calls}");
    assert!(calls.contains("pr create"), "{calls}");
}

#[tokio::test]
async fn a_repo_config_that_turns_publishing_off_keeps_screenshots_local() {
    let fx = fixture("WRITE", NO_PRS);
    screenshot(&fx, "sess-off");
    let work = fx.work.clone();
    let settings = OpenPrSettings {
        repo_publish_images: Some(Arc::new(move |dir: &Path| (dir == work).then_some(false))),
        ..OpenPrSettings::default()
    };
    let text = open_pr(&ctx(&fx, Some("sess-off"), settings), &fx.work, &args("t"))
        .await
        .unwrap();
    assert!(text.contains("screenshots were not published"), "{text}");
    assert!(remote_has(&fx, "refs/heads/feat/cart"));
    assert!(!remote_has(&fx, "refs/heads/forge-evidence"));
    let body = std::fs::read_to_string(&fx.body_file).unwrap();
    assert!(
        body.contains("0001-browser.png") && !body.contains("!["),
        "{body}"
    );
}

#[tokio::test]
async fn a_manifest_entry_naming_a_file_outside_the_ledger_is_never_pushed() {
    let fx = fixture("WRITE", NO_PRS);
    let ledger = screenshot(&fx, "sess-esc");
    std::fs::write(fx.work.join("../secret.png"), b"secret").unwrap();
    let line = r#"{"ts":"t","kind":"screenshot","file":"../../../../secret.png","sha256":"x","source":"browser"}"#;
    let manifest = ledger.dir().join(evidence::MANIFEST);
    let mut text = std::fs::read_to_string(&manifest).unwrap();
    text.push_str(line);
    text.push('\n');
    std::fs::write(&manifest, text).unwrap();

    let out = open_pr(
        &ctx(&fx, Some("sess-esc"), OpenPrSettings::default()),
        &fx.work,
        &args("t"),
    )
    .await
    .unwrap();
    assert!(
        out.contains("not published (not an intact capture"),
        "{out}"
    );
    let published = git(
        &fx.remote,
        &["ls-tree", "-r", "--name-only", "forge-evidence"],
    );
    assert_eq!(published, "sess-esc/0001-browser.png");
}

#[tokio::test]
async fn a_shorthand_remote_resolves_through_insteadof() {
    let fx = fixture("WRITE", NO_PRS);
    git(&fx.work, &["config", "remote.origin.url", "gh:acme/shop"]);
    git(
        &fx.work,
        &["config", "url.https://github.com/.insteadOf", "gh:"],
    );
    let repo = github_repo(Path::new("git"), &fx.work, "origin")
        .await
        .unwrap();
    assert_eq!(repo.slug(), "acme/shop");
}

#[test]
fn args_need_a_one_line_title() {
    assert!(Args::parse(&json!({"body": "x"})).is_err());
    assert!(Args::parse(&json!({"title": "a\nb", "body": "x"})).is_err());
    let parsed = Args::parse(&json!({"title": " t ", "body": "b", "draft": true})).unwrap();
    assert_eq!(parsed.title, "t");
    assert!(parsed.draft && parsed.publish_images && !parsed.allow_untracked);
}

#[test]
fn the_tool_is_external_so_the_broker_gates_it() {
    let tool = OpenPrTool::default();
    assert_eq!(tool.name(), OPEN_PR_TOOL);
    assert_eq!(tool.side_effect(), SideEffect::External);
    assert!(crate::ToolRegistry::with_core_tools()
        .get(OPEN_PR_TOOL)
        .is_some());
}
