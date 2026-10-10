use serde_json::json;

use crate::ToolRegistry;

fn registry_with(files: &[(&str, &str)]) -> (tempfile::TempDir, ToolRegistry) {
    let dir = tempfile::tempdir().unwrap();
    for (path, text) in files {
        let path = dir.path().join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    let registry = ToolRegistry::with_core_tools_in(dir.path());
    (dir, registry)
}

#[tokio::test]
async fn batched_read_serves_line_ranges_and_whole_files_in_one_call() {
    let (_dir, registry) = registry_with(&[
        ("src/a.rs", "l1\nl2\nl3\nl4\nl5\n"),
        ("src/b.rs", "whole\n"),
    ]);
    let out = registry
        .get("read_file")
        .unwrap()
        .run(&json!({ "paths": ["src/a.rs:2-3", "src/b.rs", "src/a.rs:5"] }))
        .await
        .unwrap();
    assert_eq!(
        out,
        "===== src/a.rs:2-3 =====\nl2\nl3\n===== src/b.rs =====\nwhole\n===== src/a.rs:5 =====\nl5\n"
    );
}

#[tokio::test]
async fn a_file_whose_name_contains_a_colon_is_not_split_into_a_range() {
    let (_dir, registry) = registry_with(&[("odd:2-3", "named\n")]);
    let out = registry
        .get("read_file")
        .unwrap()
        .run(&json!({ "paths": ["odd:2-3"] }))
        .await
        .unwrap();
    assert_eq!(out, "===== odd:2-3 =====\nnamed\n");
}

#[tokio::test]
async fn write_and_edit_confirmations_name_the_file_relative_to_the_workspace() {
    let (_dir, registry) = registry_with(&[("src/a.rs", "x\n")]);
    let wrote = registry
        .get("write_file")
        .unwrap()
        .run(&json!({ "path": "src/new.rs", "content": "hi" }))
        .await
        .unwrap();
    assert_eq!(wrote, "wrote 2 bytes to src/new.rs");
    let edited = registry
        .get("edit_file")
        .unwrap()
        .run(&json!({ "path": "src/a.rs", "old": "x", "new": "y" }))
        .await
        .unwrap();
    assert!(
        edited.starts_with("edited src/a.rs (1 replacement)"),
        "{edited}"
    );
}
