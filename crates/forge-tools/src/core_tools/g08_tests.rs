use crate::ToolRegistry;
use serde_json::{json, Value};

fn workspace() -> (tempfile::TempDir, ToolRegistry) {
    let dir = tempfile::tempdir().unwrap();
    let registry = ToolRegistry::with_core_tools_in(dir.path());
    (dir, registry)
}

async fn run(registry: &ToolRegistry, tool: &str, args: Value) -> Result<String, String> {
    registry
        .get(tool)
        .unwrap()
        .run(&args)
        .await
        .map_err(|e| e.to_string())
}

#[tokio::test]
async fn unified_diff_without_diff_git_lines_patches_every_file() {
    let (dir, registry) = workspace();
    std::fs::write(dir.path().join("g1.txt"), "a\nb\n").unwrap();
    std::fs::write(dir.path().join("g2.txt"), "c\nd\n").unwrap();
    let patch = "--- a/g1.txt\n+++ b/g1.txt\n@@ -1,2 +1,2 @@\n a\n-b\n+B\n--- a/g2.txt\n+++ b/g2.txt\n@@ -1,2 +1,2 @@\n c\n-d\n+D\n";
    run(&registry, "apply_patch", json!({ "patch": patch }))
        .await
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(dir.path().join("g1.txt")).unwrap(),
        "a\nB\n"
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("g2.txt")).unwrap(),
        "c\nD\n"
    );
}

#[tokio::test]
async fn missing_file_error_names_the_path_and_suggests_siblings() {
    let (dir, registry) = workspace();
    std::fs::create_dir(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/main.rs"), "fn main(){}\n").unwrap();
    let err = run(&registry, "read_file", json!({ "path": "src/mian.rs" }))
        .await
        .unwrap_err();
    assert!(
        err.contains("src/mian.rs") && err.contains("main.rs"),
        "{err}"
    );
}

#[tokio::test]
async fn read_file_reports_bad_ranges_and_conflicting_args() {
    let (dir, registry) = workspace();
    std::fs::write(dir.path().join("b.txt"), "1\n2\n3\n").unwrap();
    let past = run(
        &registry,
        "read_file",
        json!({ "path": "b.txt", "start_line": 99 }),
    )
    .await
    .unwrap();
    assert!(past.contains("3 lines"), "{past}");
    let inverted = run(
        &registry,
        "read_file",
        json!({ "path": "b.txt", "start_line": 3, "end_line": 1 }),
    )
    .await
    .unwrap();
    assert!(inverted.contains("before start_line"), "{inverted}");
    assert!(run(
        &registry,
        "read_file",
        json!({ "path": "b.txt", "paths": ["b.txt"] })
    )
    .await
    .is_err());
}

#[tokio::test]
async fn fifo_is_refused_by_read_file_and_skipped_by_search() {
    let (dir, registry) = workspace();
    std::fs::write(dir.path().join("f.txt"), "needle\n").unwrap();
    let made = std::process::Command::new("mkfifo")
        .arg(dir.path().join("pipe"))
        .status();
    if !made.is_ok_and(|s| s.success()) {
        return;
    }
    let err = run(&registry, "read_file", json!({ "path": "pipe" }))
        .await
        .unwrap_err();
    assert!(err.contains("not a regular file"), "{err}");
    let found = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        run(&registry, "search", json!({ "query": "needle" })),
    )
    .await
    .expect("search must not block on a FIFO")
    .unwrap();
    assert!(found.contains("f.txt"), "{found}");
}

#[tokio::test]
async fn glob_accepts_an_absolute_pattern_inside_the_workspace() {
    let (dir, registry) = workspace();
    std::fs::write(dir.path().join("host1.txt"), "x").unwrap();
    std::fs::write(dir.path().join("other.txt"), "x").unwrap();
    let pattern = format!("{}/host*", dir.path().display());
    let out = run(&registry, "glob", json!({ "pattern": pattern }))
        .await
        .unwrap();
    assert!(out.contains("host1.txt") && !out.contains("other"), "{out}");
}

#[tokio::test]
async fn edit_file_counts_replace_all_and_rejects_empty_old() {
    let (dir, registry) = workspace();
    std::fs::write(dir.path().join("e.txt"), "x\nx\nx\n").unwrap();
    let out = run(
        &registry,
        "edit_file",
        json!({ "path": "e.txt", "old": "x", "new": "y", "replace_all": true }),
    )
    .await
    .unwrap();
    assert!(out.contains("3 replacements"), "{out}");
    let err = run(
        &registry,
        "edit_file",
        json!({ "path": "e.txt", "old": "", "new": "z", "replace_all": true }),
    )
    .await
    .unwrap_err();
    assert!(err.contains("empty"), "{err}");
}

#[tokio::test]
async fn multi_edit_numbers_overall_and_merges_path_aliases() {
    let (dir, registry) = workspace();
    std::fs::write(dir.path().join("m.txt"), "k\n").unwrap();
    std::fs::write(dir.path().join("b2.txt"), "x\n").unwrap();
    run(
        &registry,
        "multi_edit",
        json!({ "edits": [
            { "path": "m.txt", "old": "k", "new": "k1" },
            { "path": "./m.txt", "old": "k1", "new": "k2" }
        ]}),
    )
    .await
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(dir.path().join("m.txt")).unwrap(),
        "k2\n"
    );
    let err = run(
        &registry,
        "multi_edit",
        json!({ "edits": [
            { "path": "m.txt", "old": "k2", "new": "z" },
            { "path": "b2.txt", "old": "MISSING", "new": "y" }
        ]}),
    )
    .await
    .unwrap_err();
    assert!(err.contains("edit #2"), "{err}");
}

#[tokio::test]
async fn shell_rejects_contradictory_args_and_keeps_the_output_tail() {
    let (_dir, registry) = workspace();
    assert!(run(
        &registry,
        "shell",
        json!({ "command": "echo a", "commands": ["echo b"] })
    )
    .await
    .is_err());
    assert!(run(
        &registry,
        "shell",
        json!({ "command": "sleep 1", "background": true, "poll_until_exit_zero": true })
    )
    .await
    .is_err());
    let out = run(&registry, "shell", json!({ "command": "seq 1 400000" }))
        .await
        .unwrap();
    assert!(out.contains("400000"), "tail survives the capture cap");
}

#[tokio::test]
async fn shell_timeout_returns_promptly_after_sigterm() {
    let (_dir, registry) = workspace();
    let started = std::time::Instant::now();
    let out = run(
        &registry,
        "shell",
        json!({ "command": "sleep 20", "timeout_secs": 1 }),
    )
    .await
    .unwrap();
    assert!(out.contains("timed out"), "{out}");
    assert!(
        started.elapsed() < std::time::Duration::from_millis(2500),
        "{:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn shell_children_do_not_inherit_scrubbed_provider_keys() {
    let (_dir, registry) = workspace();
    std::env::set_var("FORGE_TEST_SCRUBBED_KEY", "secret-value");
    forge_types::child_env::scrub("FORGE_TEST_SCRUBBED_KEY");
    let out = run(
        &registry,
        "shell",
        json!({ "command": "echo \"[${FORGE_TEST_SCRUBBED_KEY}]\"" }),
    )
    .await
    .unwrap();
    assert!(out.contains("[]") && !out.contains("secret-value"), "{out}");
}
