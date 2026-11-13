use super::super::test_support::*;
use super::*;

fn numbered(change: &[(usize, &str)]) -> String {
    (1..=30)
        .map(|n| match change.iter().find(|(line, _)| *line == n) {
            Some((_, text)) => format!("{text}\n"),
            None => format!("line {n}\n"),
        })
        .collect()
}

/// `(header, patch)` of hunk `n` in a one-file `git diff`, rebuilt as a single-hunk patch.
fn hunk_of(diff: &str, n: usize) -> (String, String) {
    let mut preamble = String::new();
    let mut hunks: Vec<String> = Vec::new();
    for line in diff.lines() {
        if line.starts_with("@@") {
            hunks.push(String::new());
        }
        match hunks.last_mut() {
            Some(current) => {
                current.push_str(line);
                current.push('\n');
            }
            None if !line.starts_with("index ") => {
                preamble.push_str(line);
                preamble.push('\n');
            }
            None => {}
        }
    }
    let body = hunks[n].clone();
    let header = body.lines().next().unwrap().to_string();
    (header, format!("{preamble}{body}"))
}

fn two_hunk_repo() -> tempfile::TempDir {
    let dir = repo_with(&[("f.txt", &numbered(&[]))]);
    std::fs::write(
        dir.path().join("f.txt"),
        numbered(&[(2, "TWO"), (28, "TWENTY-EIGHT")]),
    )
    .unwrap();
    dir
}

#[test]
fn staging_one_hunk_leaves_the_other_in_the_working_tree_only() {
    let dir = two_hunk_repo();
    let diff = git_out(dir.path(), &["diff"]);
    let (header, patch) = hunk_of(&diff, 1);
    let patch = validate_hunk_patch("f.txt", &header, &patch).unwrap();
    apply_hunk(dir.path(), HunkAction::Stage, &patch).unwrap();

    let staged = git_out(dir.path(), &["diff", "--cached"]);
    assert!(staged.contains("+TWENTY-EIGHT"));
    assert!(!staged.contains("+TWO\n"), "the first hunk stays unstaged");
    let unstaged = git_out(dir.path(), &["diff"]);
    assert!(unstaged.contains("+TWO\n"));
    assert!(!unstaged.contains("TWENTY-EIGHT"));
    let on_disk = std::fs::read_to_string(dir.path().join("f.txt")).unwrap();
    assert!(on_disk.contains("TWO") && on_disk.contains("TWENTY-EIGHT"));
}

#[test]
fn unstaging_a_hunk_undoes_staging_it() {
    let dir = two_hunk_repo();
    let (header, patch) = hunk_of(&git_out(dir.path(), &["diff"]), 0);
    let patch = validate_hunk_patch("f.txt", &header, &patch).unwrap();
    apply_hunk(dir.path(), HunkAction::Stage, &patch).unwrap();
    assert!(git_out(dir.path(), &["diff", "--cached"]).contains("+TWO"));

    let (header, patch) = hunk_of(&git_out(dir.path(), &["diff", "--cached"]), 0);
    let patch = validate_hunk_patch("f.txt", &header, &patch).unwrap();
    apply_hunk(dir.path(), HunkAction::Unstage, &patch).unwrap();
    assert_eq!(git_out(dir.path(), &["diff", "--cached"]), "");
    assert!(
        std::fs::read_to_string(dir.path().join("f.txt"))
            .unwrap()
            .contains("TWO"),
        "unstage never touches the working tree"
    );
}

#[test]
fn reverting_one_hunk_restores_only_that_hunk() {
    let dir = two_hunk_repo();
    let (header, patch) = hunk_of(&git_out(dir.path(), &["diff"]), 0);
    let patch = validate_hunk_patch("f.txt", &header, &patch).unwrap();
    apply_hunk(dir.path(), HunkAction::Revert, &patch).unwrap();
    let on_disk = std::fs::read_to_string(dir.path().join("f.txt")).unwrap();
    assert!(on_disk.contains("line 2\n"), "hunk 1 is back to HEAD");
    assert!(on_disk.contains("TWENTY-EIGHT"), "hunk 2 is untouched");
}

#[test]
fn a_stale_patch_is_refused_and_changes_nothing() {
    let dir = two_hunk_repo();
    let (header, patch) = hunk_of(&git_out(dir.path(), &["diff"]), 0);
    let patch = validate_hunk_patch("f.txt", &header, &patch).unwrap();
    // The agent rewrote the same lines after the client fetched the diff.
    std::fs::write(
        dir.path().join("f.txt"),
        numbered(&[(2, "SOMETHING ELSE"), (28, "TWENTY-EIGHT")]),
    )
    .unwrap();
    let before = std::fs::read_to_string(dir.path().join("f.txt")).unwrap();
    let (stale, _) = apply_hunk(dir.path(), HunkAction::Revert, &patch).unwrap_err();
    assert!(stale, "a hunk that no longer applies is a 409, not a 400");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("f.txt")).unwrap(),
        before
    );
}

#[test]
fn an_untracked_file_can_be_staged_as_a_new_file_hunk() {
    let dir = repo_with(&[("a.txt", "a\n")]);
    std::fs::write(dir.path().join("new.txt"), "x\ny\n").unwrap();
    let patch = "--- /dev/null\n+++ b/new.txt\n@@ -0,0 +1,2 @@\n+x\n+y\n";
    let patch = validate_hunk_patch("new.txt", "@@ -0,0 +1,2 @@", patch).unwrap();
    apply_hunk(dir.path(), HunkAction::Stage, &patch).unwrap();
    assert!(git_out(dir.path(), &["diff", "--cached", "--name-only"]).contains("new.txt"));
}

#[test]
fn a_bare_hunk_gets_the_plain_file_header() {
    let patch = validate_hunk_patch("src/a.rs", "@@ -1 +1 @@", "@@ -1 +1 @@\n-a\n+b\n").unwrap();
    assert_eq!(
        patch,
        "--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1 +1 @@\n-a\n+b\n"
    );
}

#[test]
fn validation_refuses_anything_that_is_not_one_hunk_of_the_named_path() {
    let ok = "--- a/f.txt\n+++ b/f.txt\n@@ -1 +1 @@\n-a\n+b\n";
    assert!(validate_hunk_patch("f.txt", "@@ -1 +1 @@", ok).is_ok());

    let other_file = "--- a/other.txt\n+++ b/other.txt\n@@ -1 +1 @@\n-a\n+b\n";
    assert!(validate_hunk_patch("f.txt", "@@ -1 +1 @@", other_file).is_err());

    let escape = "--- a/../x\n+++ b/../x\n@@ -1 +1 @@\n-a\n+b\n";
    assert!(validate_hunk_patch("f.txt", "@@ -1 +1 @@", escape).is_err());

    let two_files = format!("{ok}--- a/g.txt\n+++ b/g.txt\n@@ -1 +1 @@\n-a\n+b\n");
    assert!(validate_hunk_patch("f.txt", "@@ -1 +1 @@", &two_files).is_err());

    let two_hunks = "--- a/f.txt\n+++ b/f.txt\n@@ -1 +1 @@\n-a\n+b\n@@ -9 +9 @@\n-c\n+d\n";
    assert!(validate_hunk_patch("f.txt", "@@ -1 +1 @@", two_hunks).is_err());

    assert!(
        validate_hunk_patch("f.txt", "@@ -2 +2 @@", ok).is_err(),
        "header must match"
    );

    let rename =
        "rename from f.txt\nrename to g.txt\n--- a/f.txt\n+++ b/f.txt\n@@ -1 +1 @@\n-a\n+b\n";
    assert!(validate_hunk_patch("f.txt", "@@ -1 +1 @@", rename).is_err());

    let symlink = "new file mode 120000\n--- /dev/null\n+++ b/f.txt\n@@ -0,0 +1 @@\n+/etc/passwd\n";
    assert!(validate_hunk_patch("f.txt", "@@ -0,0 +1 @@", symlink).is_err());

    let prose = "--- a/f.txt\n+++ b/f.txt\n@@ -1 +1 @@\n-a\n+b\nrm -rf /\n";
    assert!(validate_hunk_patch("f.txt", "@@ -1 +1 @@", prose).is_err());

    assert!(validate_hunk_patch("f.txt", "@@ -1 +1 @@", "--- a/f.txt\n+++ b/f.txt\n").is_err());
}

#[tokio::test]
async fn the_route_maps_outcomes_to_status_codes() {
    use axum::http::StatusCode;
    let dir = two_hunk_repo();
    let store = Arc::new(forge_store::Store::open_in_memory().unwrap());
    let sid = store
        .create_session(&dir.path().display().to_string(), "default")
        .unwrap();
    let app = router(store, dir.path());
    let (header, patch) = hunk_of(&git_out(dir.path(), &["diff"]), 0);
    let body = |path: &str, patch: &str, action: &str| {
        serde_json::json!({
            "session": sid, "path": path, "hunk_header": header,
            "patch": patch, "action": action,
        })
    };

    let (status, reply) = http_post(&app, "/api/git/hunk", body("f.txt", &patch, "stage")).await;
    assert_eq!(status, StatusCode::OK, "{reply}");
    assert!(git_out(dir.path(), &["diff", "--cached"]).contains("+TWO"));

    // The same hunk again: the index already has it, so it no longer applies.
    let (status, reply) = http_post(&app, "/api/git/hunk", body("f.txt", &patch, "stage")).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(reply["code"], "stale_patch");

    // A path that leaves the repository never reaches git.
    let (status, _) = http_post(
        &app,
        "/api/git/hunk",
        body("../escape.txt", &patch, "stage"),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = http_post(&app, "/api/git/hunk", body("/etc/passwd", &patch, "stage")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // A patch for a different file than the named path is refused, even inside the repo.
    let (status, _) = http_post(&app, "/api/git/hunk", body("other.txt", &patch, "stage")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let ghost = serde_json::json!({
        "session": "ghost", "path": "f.txt", "hunk_header": header,
        "patch": patch, "action": "stage",
    });
    let (status, _) = http_post(&app, "/api/git/hunk", ghost).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, _) = http_post(&app, "/api/git/hunk", body("f.txt", &patch, "obliterate")).await;
    assert!(status.is_client_error(), "unknown action is rejected");
}

#[test]
fn a_hunk_touching_an_unterminated_last_line_stages_and_reverts() {
    let dir = repo_with(&[("n.txt", "one\ntwo\nthree")]);
    std::fs::write(dir.path().join("n.txt"), "one\ntwo\nTHREE").unwrap();
    let raw = git_out(dir.path(), &["diff"]);
    let files = super::super::diff::parse_unified_diff(&raw);
    let hunk = &files[0].hunks[0];
    assert_eq!(
        hunk.lines.iter().filter(|l| l.starts_with('\\')).count(),
        2,
        "both sides lack the final newline, so the marker follows `-three` and `+THREE`"
    );
    assert_eq!(files[0].total_lines, 4, "markers are not counted");
    let patch = format!(
        "--- a/n.txt\n+++ b/n.txt\n{}\n{}\n",
        hunk.header,
        hunk.lines.join("\n")
    );
    let patch = validate_hunk_patch("n.txt", &hunk.header, &patch).unwrap();

    apply_hunk(dir.path(), HunkAction::Stage, &patch).unwrap();
    assert!(git_out(dir.path(), &["diff", "--cached"]).contains("+THREE"));
    apply_hunk(dir.path(), HunkAction::Unstage, &patch).unwrap();
    assert!(git_out(dir.path(), &["diff", "--cached"]).is_empty());

    apply_hunk(dir.path(), HunkAction::Revert, &patch).unwrap();
    let on_disk = std::fs::read_to_string(dir.path().join("n.txt")).unwrap();
    assert_eq!(on_disk, "one\ntwo\nthree");
}

#[test]
fn hunks_of_a_crlf_file_stage_and_revert() {
    let dir = repo_with(&[("w.txt", "one\r\ntwo\r\nthree\r\n")]);
    std::fs::write(dir.path().join("w.txt"), "one\r\nTWO\r\nthree\r\n").unwrap();
    let diff = git_out(dir.path(), &["diff"]);
    let header = diff
        .split('\n')
        .find(|l| l.starts_with("@@"))
        .unwrap()
        .trim_end()
        .to_string();
    let patch = validate_hunk_patch("w.txt", &header, &diff).unwrap();
    apply_hunk(dir.path(), HunkAction::Stage, &patch).unwrap();
    assert!(git_out(dir.path(), &["diff", "--cached"]).contains("+TWO\r"));

    apply_hunk(dir.path(), HunkAction::Unstage, &patch).unwrap();
    apply_hunk(dir.path(), HunkAction::Revert, &patch).unwrap();
    assert_eq!(
        std::fs::read(dir.path().join("w.txt")).unwrap(),
        b"one\r\ntwo\r\nthree\r\n"
    );
}
