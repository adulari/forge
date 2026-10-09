//! Native support for the Codex CLI `apply_patch` envelope: `*** Begin Patch` /
//! `*** Update File:` / `*** Add File:` / `*** Delete File:` / `@@` hunks. Models trained on
//! Codex emit this verbatim — it is NOT a unified diff, so handing it to `git apply` fails
//! outright with "No valid patches in input" before git ever looks at file content. This module
//! parses the envelope and applies each `Update File` hunk through the same fuzzy `apply_edit`
//! chain `edit_file`/`multi_edit` use, so a Codex-shaped hunk gets the same tolerance for
//! drifted whitespace/indentation as a plain edit.

use std::path::Path;

use crate::ToolError;

use super::edits::{apply_edits, EditStep};

/// One parsed section of a Codex patch envelope.
enum CodexOp {
    Add {
        path: String,
        content: String,
    },
    Delete {
        path: String,
    },
    Update {
        path: String,
        hunks: Vec<(String, String)>,
    },
}

/// Whether `patch` is the Codex envelope rather than a unified diff — checked before anything
/// else tries to parse it as one.
pub(super) fn is_codex_patch(patch: &str) -> bool {
    patch.trim_start().starts_with("*** Begin Patch")
}

/// Parse the envelope into its file-level operations. Returns an error naming the problem when
/// no recognizable section is found at all (callers only reach this after `is_codex_patch`, so
/// that generally means a malformed envelope, not a non-Codex patch).
fn parse(patch: &str) -> Result<Vec<CodexOp>, String> {
    let lines: Vec<&str> = patch.lines().collect();
    let mut i = 0;
    while i < lines.len() && lines[i].trim() != "*** Begin Patch" {
        i += 1;
    }
    i += 1;

    let mut ops = Vec::new();
    while i < lines.len() {
        let line = lines[i];
        if line.trim() == "*** End Patch" {
            break;
        }
        if let Some(path) = line.strip_prefix("*** Update File: ") {
            let path = path.trim().to_string();
            i += 1;
            if lines
                .get(i)
                .is_some_and(|l| l.trim_start().starts_with("*** Move to:"))
            {
                i += 1; // renames: applied in place at the original path, target name ignored
            }
            let (hunks, next) = parse_update_hunks(&lines, i);
            ops.push(CodexOp::Update { path, hunks });
            i = next;
        } else if let Some(path) = line.strip_prefix("*** Add File: ") {
            let path = path.trim().to_string();
            i += 1;
            let mut body = Vec::new();
            while i < lines.len() && !lines[i].starts_with("*** ") {
                body.push(lines[i].strip_prefix('+').unwrap_or(lines[i]));
                i += 1;
            }
            let mut content = body.join("\n");
            if !content.is_empty() {
                content.push('\n');
            }
            ops.push(CodexOp::Add { path, content });
        } else if let Some(path) = line.strip_prefix("*** Delete File: ") {
            let path = path.trim().to_string();
            i += 1;
            while i < lines.len() && !lines[i].starts_with("*** ") {
                i += 1; // tolerate a stray body under a delete section
            }
            ops.push(CodexOp::Delete { path });
        } else {
            i += 1; // blank line or stray content between sections
        }
    }

    if ops.is_empty() {
        return Err(
            "no `*** Add File:` / `*** Update File:` / `*** Delete File:` section found in the \
             Codex patch envelope"
                .to_string(),
        );
    }
    Ok(ops)
}

/// Parse the `@@`-delimited hunks of one `Update File` section starting at `start`, returning
/// them as `(old, new)` text pairs plus the index just past the section.
fn parse_update_hunks(lines: &[&str], start: usize) -> (Vec<(String, String)>, usize) {
    let mut hunks = Vec::new();
    let mut old: Vec<&str> = Vec::new();
    let mut new: Vec<&str> = Vec::new();
    let mut i = start;
    let mut started = false;

    let flush = |old: &mut Vec<&str>, new: &mut Vec<&str>, hunks: &mut Vec<(String, String)>| {
        if !old.is_empty() || !new.is_empty() {
            hunks.push((old.join("\n"), new.join("\n")));
        }
        old.clear();
        new.clear();
    };

    while i < lines.len() && !lines[i].starts_with("*** ") {
        let line = lines[i];
        i += 1;
        if line.starts_with("@@") {
            if started {
                flush(&mut old, &mut new, &mut hunks);
            }
            started = true;
            continue;
        }
        started = true;
        if let Some(rest) = line.strip_prefix(' ') {
            old.push(rest);
            new.push(rest);
        } else if let Some(rest) = line.strip_prefix('-') {
            old.push(rest);
        } else if let Some(rest) = line.strip_prefix('+') {
            new.push(rest);
        } else {
            // Some encoders drop the leading space from a blank context line.
            old.push(line);
            new.push(line);
        }
    }
    flush(&mut old, &mut new, &mut hunks);
    (hunks, i)
}

/// Apply a Codex patch envelope under `cwd` (already confined/absolute). Each `Update File`
/// section's hunks are folded over the file's content via [`apply_edits`] — exact match first,
/// falling back to the whitespace-insensitive and block-anchor tiers — so content drift is
/// tolerated the same way a hand-written `multi_edit` call is.
pub(super) async fn apply(patch: &str, cwd: &Path) -> Result<String, ToolError> {
    let ops = parse(patch).map_err(ToolError::Failed)?;
    let mut summary = Vec::new();

    for op in ops {
        match op {
            CodexOp::Add { path, content } => {
                let full = resolve(cwd, &path)?;
                if tokio::fs::metadata(&full).await.is_ok() {
                    return Err(ToolError::Failed(format!(
                        "apply_patch: {path} already exists (use Update File to modify it)"
                    )));
                }
                if let Some(parent) = full.parent() {
                    tokio::fs::create_dir_all(parent).await?;
                }
                tokio::fs::write(&full, &content).await?;
                summary.push(format!("added {path}"));
            }
            CodexOp::Delete { path } => {
                let full = resolve(cwd, &path)?;
                tokio::fs::remove_file(&full)
                    .await
                    .map_err(|e| ToolError::Failed(format!("apply_patch: deleting {path}: {e}")))?;
                summary.push(format!("deleted {path}"));
            }
            CodexOp::Update { path, hunks } => {
                if hunks.iter().any(|(old, _)| old.is_empty()) {
                    return Err(ToolError::Failed(format!(
                        "apply_patch: {path} has a hunk with no context/removed lines to anchor \
                         on — a pure insertion needs at least one unchanged surrounding line"
                    )));
                }
                let full = resolve(cwd, &path)?;
                let original = tokio::fs::read_to_string(&full)
                    .await
                    .map_err(|e| ToolError::Failed(format!("apply_patch: reading {path}: {e}")))?;
                let steps: Vec<EditStep> = hunks
                    .into_iter()
                    .map(|(old, new)| EditStep {
                        old,
                        new,
                        replace_all: false,
                    })
                    .collect();
                let hunk_count = steps.len();
                let updated = apply_edits(&original, &steps)
                    .map_err(|e| ToolError::Failed(format!("apply_patch: {path}: {e}")))?;
                tokio::fs::write(&full, &updated).await?;
                summary.push(format!("updated {path} ({hunk_count} hunk(s))"));
            }
        }
    }

    Ok(format!("applied Codex patch: {}", summary.join("; ")))
}

fn resolve(cwd: &Path, path: &str) -> Result<std::path::PathBuf, ToolError> {
    let full = cwd.join(path);
    let full_str = full
        .to_str()
        .ok_or_else(|| ToolError::Failed(format!("apply_patch: non-UTF-8 path: {path}")))?;
    super::confine(full_str)?;
    Ok(full)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, name: &str, content: &str) {
        std::fs::write(dir.join(name), content).unwrap();
    }

    fn tmp(tag: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("codex-patch-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn is_codex_patch_detects_the_envelope_not_a_unified_diff() {
        assert!(is_codex_patch("*** Begin Patch\n*** End Patch\n"));
        assert!(is_codex_patch("  \n*** Begin Patch\n"));
        assert!(!is_codex_patch(
            "--- a/f.txt\n+++ b/f.txt\n@@ -1 +1 @@\n-a\n+b\n"
        ));
    }

    #[test]
    fn parse_update_file_extracts_old_new_hunk() {
        let patch = "*** Begin Patch\n*** Update File: f.rs\n@@\n fn a() {\n-    old();\n+    new();\n }\n*** End Patch\n";
        let ops = parse(patch).unwrap();
        assert_eq!(ops.len(), 1);
        match &ops[0] {
            CodexOp::Update { path, hunks } => {
                assert_eq!(path, "f.rs");
                assert_eq!(hunks.len(), 1);
                assert_eq!(hunks[0].0, "fn a() {\n    old();\n}");
                assert_eq!(hunks[0].1, "fn a() {\n    new();\n}");
            }
            _ => panic!("expected an Update op"),
        }
    }

    #[test]
    fn parse_handles_add_and_delete_sections() {
        let patch = "*** Begin Patch\n*** Add File: new.txt\n+line one\n+line two\n*** Delete File: gone.txt\n*** End Patch\n";
        let ops = parse(patch).unwrap();
        assert_eq!(ops.len(), 2);
        match &ops[0] {
            CodexOp::Add { path, content } => {
                assert_eq!(path, "new.txt");
                assert_eq!(content, "line one\nline two\n");
            }
            _ => panic!("expected an Add op"),
        }
        assert!(matches!(&ops[1], CodexOp::Delete { path } if path == "gone.txt"));
    }

    #[tokio::test]
    async fn apply_updates_a_file_via_the_fuzzy_edit_chain() {
        let dir = tmp("update");
        write(&dir, "f.rs", "fn a() {\n    old();\n}\n");
        let patch = "*** Begin Patch\n*** Update File: f.rs\n@@\n fn a() {\n-    old();\n+    new();\n }\n*** End Patch\n";
        let out = apply(patch, &dir).await;
        assert!(out.is_ok(), "{out:?}");
        assert_eq!(
            std::fs::read_to_string(dir.join("f.rs")).unwrap(),
            "fn a() {\n    new();\n}\n"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn apply_tolerates_indentation_drift_via_whitespace_fallback() {
        let dir = tmp("fuzz");
        // File has 4-space indent; the hunk's context/removed lines use 2 spaces — exact match
        // misses, the whitespace-insensitive fallback (shared with edit_file) should still land it.
        write(&dir, "f.rs", "fn a() {\n    old();\n}\n");
        let patch = "*** Begin Patch\n*** Update File: f.rs\n@@\n fn a() {\n-  old();\n+  new();\n }\n*** End Patch\n";
        let out = apply(patch, &dir).await;
        assert!(out.is_ok(), "{out:?}");
        assert!(std::fs::read_to_string(dir.join("f.rs"))
            .unwrap()
            .contains("new();"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn apply_adds_and_deletes_files() {
        let dir = tmp("addel");
        write(&dir, "gone.txt", "bye\n");
        let patch = "*** Begin Patch\n*** Add File: new.txt\n+hello\n*** Delete File: gone.txt\n*** End Patch\n";
        let out = apply(patch, &dir).await;
        assert!(out.is_ok(), "{out:?}");
        assert_eq!(
            std::fs::read_to_string(dir.join("new.txt")).unwrap(),
            "hello\n"
        );
        assert!(!dir.join("gone.txt").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn apply_reports_which_hunk_failed_without_writing() {
        let dir = tmp("fail");
        write(&dir, "f.rs", "fn a() {\n    stays();\n}\n");
        let patch = "*** Begin Patch\n*** Update File: f.rs\n@@\n-    not_there();\n+    new();\n*** End Patch\n";
        let err = apply(patch, &dir).await.unwrap_err().to_string();
        assert!(err.contains("f.rs"), "{err}");
        assert!(err.contains("not found"), "{err}");
        assert_eq!(
            std::fs::read_to_string(dir.join("f.rs")).unwrap(),
            "fn a() {\n    stays();\n}\n",
            "untouched on failure"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
