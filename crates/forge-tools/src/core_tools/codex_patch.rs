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
        move_to: Option<String>,
        hunks: Vec<Hunk>,
    },
}

/// One change inside an `Update File` section: `old` becomes `new`.
#[derive(Debug)]
struct Hunk {
    old: String,
    new: String,
    /// The lines this hunk comes after, when the patch says so: its `@@ <line>` header, or a chunk
    /// of nothing but context lines in front of it. Codex applies chunks in file order, so with
    /// this the hunk lands on the first match after those lines instead of failing as ambiguous.
    after: Option<String>,
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
            let mut move_to = None;
            if let Some(target) = lines
                .get(i)
                .and_then(|l| l.trim_start().strip_prefix("*** Move to:"))
            {
                move_to = Some(target.trim().to_string());
                i += 1;
            }
            let (hunks, next) = parse_update_hunks(&lines, i);
            ops.push(CodexOp::Update {
                path,
                move_to,
                hunks,
            });
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
/// them plus the index just past the section.
///
/// A chunk that has no `-`/`+` line changes nothing, so it is not a hunk. Codex still reads it as
/// a position: the next chunk is applied after it. That is kept as the next hunk's `after`, as is
/// the text of a `@@ <line>` header; a context-only chunk that nothing follows is dropped.
fn parse_update_hunks(lines: &[&str], start: usize) -> (Vec<Hunk>, usize) {
    let mut hunks = Vec::new();
    let mut old: Vec<&str> = Vec::new();
    let mut new: Vec<&str> = Vec::new();
    let mut header: Option<String> = None;
    let mut scope: Option<String> = None;
    let mut i = start;
    let mut started = false;

    let flush = |old: &mut Vec<&str>,
                 new: &mut Vec<&str>,
                 header: &mut Option<String>,
                 scope: &mut Option<String>,
                 hunks: &mut Vec<Hunk>| {
        if !old.is_empty() || !new.is_empty() {
            if old == new {
                *scope = Some(old.join("\n"));
            } else {
                hunks.push(Hunk {
                    old: old.join("\n"),
                    new: new.join("\n"),
                    after: header.take().or_else(|| scope.take()),
                });
                *scope = None;
            }
        } else if let Some(text) = header.take() {
            *scope = Some(text);
        }
        *header = None;
        old.clear();
        new.clear();
    };

    while i < lines.len() && !lines[i].starts_with("*** ") {
        let line = lines[i];
        i += 1;
        if let Some(rest) = line.strip_prefix("@@") {
            if started {
                flush(&mut old, &mut new, &mut header, &mut scope, &mut hunks);
            }
            started = true;
            header = Some(rest.trim().to_string()).filter(|text| !text.is_empty());
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
    flush(&mut old, &mut new, &mut header, &mut scope, &mut hunks);
    (hunks, i)
}

/// The tree as the patch will leave it: staged writes (`Some`) and deletions (`None`) layered
/// over the real filesystem, so later operations see earlier ones and nothing hits disk until
/// every operation has applied.
#[derive(Default)]
struct Staged {
    order: Vec<std::path::PathBuf>,
    files: std::collections::HashMap<std::path::PathBuf, Option<String>>,
}

impl Staged {
    fn put(&mut self, full: &Path, content: Option<String>) {
        if !self.files.contains_key(full) {
            self.order.push(full.to_path_buf());
        }
        self.files.insert(full.to_path_buf(), content);
    }

    async fn read(&self, full: &Path) -> std::io::Result<Option<String>> {
        match self.files.get(full) {
            Some(staged) => Ok(staged.clone()),
            None => tokio::fs::read_to_string(full).await.map(Some),
        }
    }

    async fn exists(&self, full: &Path) -> bool {
        match self.files.get(full) {
            Some(staged) => staged.is_some(),
            None => tokio::fs::metadata(full).await.is_ok(),
        }
    }
}

/// Apply a Codex patch envelope under `cwd` (already confined/absolute). Each `Update File`
/// section's hunks are folded over the file's content via [`apply_edits`] — exact match first,
/// falling back to the whitespace-insensitive and block-anchor tiers — so content drift is
/// tolerated the same way a hand-written `multi_edit` call is. All-or-nothing: every operation
/// is staged in memory and written only once all of them applied.
pub(super) async fn apply(patch: &str, cwd: &Path) -> Result<String, ToolError> {
    let ops = parse(patch).map_err(ToolError::Failed)?;
    let mut summary = Vec::new();
    let mut staged = Staged::default();

    for op in ops {
        match op {
            CodexOp::Add { path, content } => {
                let full = resolve(cwd, &path)?;
                if staged.exists(&full).await {
                    return Err(ToolError::Failed(format!(
                        "apply_patch: {path} already exists (use Update File to modify it)"
                    )));
                }
                staged.put(&full, Some(content));
                summary.push(format!("added {path}"));
            }
            CodexOp::Delete { path } => {
                let full = resolve(cwd, &path)?;
                if !staged.exists(&full).await {
                    return Err(ToolError::Failed(format!(
                        "apply_patch: deleting {path}: no such file"
                    )));
                }
                staged.put(&full, None);
                summary.push(format!("deleted {path}"));
            }
            CodexOp::Update {
                path,
                move_to,
                hunks,
            } => {
                if hunks.is_empty() {
                    return Err(ToolError::Failed(format!(
                        "apply_patch: {path} has no `-` or `+` lines, only context, so nothing \
                         changes"
                    )));
                }
                if hunks.iter().any(|hunk| hunk.old.is_empty()) {
                    return Err(ToolError::Failed(format!(
                        "apply_patch: {path} has a hunk with no context/removed lines to anchor \
                         on — a pure insertion needs at least one unchanged surrounding line"
                    )));
                }
                let full = resolve(cwd, &path)?;
                let original = staged
                    .read(&full)
                    .await
                    .map_err(|e| ToolError::Failed(format!("apply_patch: reading {path}: {e}")))?
                    .ok_or_else(|| {
                        ToolError::Failed(format!(
                            "apply_patch: {path} is deleted earlier in the patch"
                        ))
                    })?;
                let steps: Vec<EditStep> = hunks
                    .into_iter()
                    .enumerate()
                    .map(|(k, hunk)| EditStep {
                        number: k + 1,
                        old: hunk.old,
                        new: hunk.new,
                        replace_all: false,
                        ordered: true,
                        after: hunk.after,
                    })
                    .collect();
                let hunk_count = steps.len();
                let updated = apply_edits(&original, &steps)
                    .map_err(|e| ToolError::Failed(format!("apply_patch: {path}: {e}")))?;
                match move_to {
                    Some(target) if target != path => {
                        let target_full = resolve(cwd, &target)?;
                        if staged.exists(&target_full).await {
                            return Err(ToolError::Failed(format!(
                                "apply_patch: cannot move {path} to {target}: target already exists"
                            )));
                        }
                        staged.put(&target_full, Some(updated));
                        staged.put(&full, None);
                        summary.push(format!(
                            "updated {path} ({hunk_count} hunk(s)) and moved to {target}"
                        ));
                    }
                    _ => {
                        staged.put(&full, Some(updated));
                        summary.push(format!("updated {path} ({hunk_count} hunk(s))"));
                    }
                }
            }
        }
    }

    for full in &staged.order {
        match &staged.files[full] {
            Some(content) => {
                if let Some(parent) = full.parent() {
                    tokio::fs::create_dir_all(parent).await?;
                }
                crate::fs_write::write(full, content).await?;
            }
            None => {
                tokio::fs::remove_file(full).await.map_err(|e| {
                    ToolError::Failed(format!("apply_patch: deleting {}: {e}", full.display()))
                })?;
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
            CodexOp::Update { path, hunks, .. } => {
                assert_eq!(path, "f.rs");
                assert_eq!(hunks.len(), 1);
                assert_eq!(hunks[0].old, "fn a() {\n    old();\n}");
                assert_eq!(hunks[0].new, "fn a() {\n    new();\n}");
                assert_eq!(hunks[0].after, None);
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

    #[tokio::test]
    async fn apply_is_atomic_across_files() {
        let dir = tmp("atomic");
        write(&dir, "a.txt", "1\n2\n");
        write(&dir, "b.txt", "3\n4\n");
        let patch = "*** Begin Patch\n*** Update File: a.txt\n@@\n 1\n-2\n+TWO\n*** Update File: b.txt\n@@\n-NOPE\n+x\n*** End Patch\n";
        assert!(apply(patch, &dir).await.is_err());
        assert_eq!(
            std::fs::read_to_string(dir.join("a.txt")).unwrap(),
            "1\n2\n"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn apply_performs_move_to_and_keeps_crlf() {
        let dir = tmp("move");
        write(&dir, "r.txt", "a\r\nb\r\nc\r\n");
        let patch = "*** Begin Patch\n*** Update File: r.txt\n*** Move to: sub/r2.txt\n@@\n a\n-b\n+B\n+B2\n c\n*** End Patch\n";
        apply(patch, &dir).await.unwrap();
        assert!(!dir.join("r.txt").exists());
        assert_eq!(
            std::fs::read_to_string(dir.join("sub/r2.txt")).unwrap(),
            "a\r\nB\r\nB2\r\nc\r\n"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
    const TWO_FUNCTIONS: &str = "fn first() {\n    body();\n}\n\nfn second() {\n    body();\n}\n";

    #[test]
    fn a_context_only_chunk_scopes_the_next_hunk_and_is_not_a_hunk() {
        let patch = "*** Begin Patch\n*** Update File: f.rs\n@@\n fn second() {\n@@\n-    body();\n+    other();\n*** End Patch\n";
        let ops = parse(patch).unwrap();
        match &ops[0] {
            CodexOp::Update { hunks, .. } => {
                assert_eq!(hunks.len(), 1, "the context-only chunk is no hunk");
                assert_eq!(hunks[0].after.as_deref(), Some("fn second() {"));
            }
            _ => panic!("expected an Update op"),
        }
    }

    #[tokio::test]
    async fn the_hunk_after_a_context_only_chunk_lands_on_the_first_match_after_it() {
        let dir = tmp("scope");
        write(&dir, "f.rs", TWO_FUNCTIONS);
        // `    body();` occurs twice, so on its own the hunk is ambiguous; after `fn second() {` it is not.
        let patch = "*** Begin Patch\n*** Update File: f.rs\n@@\n fn second() {\n@@\n-    body();\n+    other();\n*** End Patch\n";
        let out = apply(patch, &dir).await;
        assert!(out.is_ok(), "{out:?}");
        assert_eq!(
            std::fs::read_to_string(dir.join("f.rs")).unwrap(),
            "fn first() {\n    body();\n}\n\nfn second() {\n    other();\n}\n"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn an_at_header_line_scopes_its_hunk_the_same_way() {
        let dir = tmp("header");
        write(&dir, "f.rs", TWO_FUNCTIONS);
        let patch = "*** Begin Patch\n*** Update File: f.rs\n@@ fn second() {\n-    body();\n+    other();\n*** End Patch\n";
        let out = apply(patch, &dir).await;
        assert!(out.is_ok(), "{out:?}");
        assert_eq!(
            std::fs::read_to_string(dir.join("f.rs")).unwrap(),
            "fn first() {\n    body();\n}\n\nfn second() {\n    other();\n}\n"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn an_anchor_that_is_not_in_the_file_does_not_stop_a_hunk_that_would_apply() {
        let dir = tmp("noanchor");
        write(&dir, "f.rs", "fn only() {\n    body();\n}\n");
        let patch = "*** Begin Patch\n*** Update File: f.rs\n@@ fn gone() {\n-    body();\n+    other();\n*** End Patch\n";
        let out = apply(patch, &dir).await;
        assert!(out.is_ok(), "{out:?}");
        assert!(std::fs::read_to_string(dir.join("f.rs"))
            .unwrap()
            .contains("other();"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_trailing_context_only_chunk_is_ignored() {
        let dir = tmp("trailing");
        write(&dir, "f.rs", "fn a() {\n    old();\n}\n\nfn b() {}\n");
        let patch = "*** Begin Patch\n*** Update File: f.rs\n@@\n-    old();\n+    new();\n@@\n fn b() {}\n*** End Patch\n";
        let out = apply(patch, &dir).await;
        assert!(out.is_ok(), "{out:?}");
        assert_eq!(
            std::fs::read_to_string(dir.join("f.rs")).unwrap(),
            "fn a() {\n    new();\n}\n\nfn b() {}\n"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_patch_of_only_context_says_so_instead_of_claiming_an_identical_edit() {
        let dir = tmp("onlyctx");
        write(&dir, "f.rs", "fn a() {}\n");
        let patch = "*** Begin Patch\n*** Update File: f.rs\n@@\n fn a() {}\n*** End Patch\n";
        let err = apply(patch, &dir).await.unwrap_err().to_string();
        assert!(err.contains("only context"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }
    /// The patch `gpt-6-luna` sent for the first edit of the b01 benchmark task (stockctl, a long
    /// multi-feature change to `src/cli.rs`), replayed on the task's original file. Its fourth chunk
    /// names a closing `}` that occurs dozens of times; the patch was refused as ambiguous.
    #[tokio::test]
    async fn a_recorded_model_patch_with_a_bare_closing_brace_chunk_applies() {
        let dir = tmp("b01-replay");
        write(&dir, "cli.rs", CLI_RS);
        let patch = B01_PATCH.replace("src/cli.rs", "cli.rs");
        let out = apply(&patch, &dir).await;
        assert!(out.is_ok(), "{out:?}");
        let updated = std::fs::read_to_string(dir.join("cli.rs")).unwrap();
        let export_at = updated.find("fn cmd_export").expect("cmd_export added");
        let list_at = updated.find("fn cmd_list").unwrap();
        let summary_at = updated.find("fn cmd_summary").unwrap();
        assert!(
            list_at < export_at && export_at < summary_at,
            "cmd_export lands between cmd_list and cmd_summary, after cmd_list's closing brace"
        );
        assert!(
            updated.contains("}\n\nfn cmd_export"),
            "after the function's own `}}`, not one nested inside it:\n{updated}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    const CLI_RS: &str = r####"//! Argument handling for the `stockctl` binary.

use std::io::Write;

use crate::model::Item;
use crate::{csv, report};

const USAGE: &str = "usage: stockctl <list|summary> <file>";

/// Runs the CLI and returns the process exit code.
pub fn run(args: &[String], out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    match args.first().map(String::as_str) {
        Some("list") => cmd_list(&args[1..], out, err),
        Some("summary") => cmd_summary(&args[1..], out, err),
        Some("--help") | Some("-h") => {
            let _ = writeln!(out, "{USAGE}");
            0
        }
        Some(other) => {
            let _ = writeln!(err, "stockctl: unknown command `{other}`\n{USAGE}");
            2
        }
        None => {
            let _ = writeln!(err, "{USAGE}");
            2
        }
    }
}

fn load(path: &str) -> Result<Vec<Item>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
    csv::parse_items(&text).map_err(|e| format!("{path}: {e}"))
}

fn cmd_list(args: &[String], out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    let [path] = args else {
        let _ = writeln!(err, "stockctl list: expected exactly one file\n{USAGE}");
        return 2;
    };
    match load(path) {
        Ok(items) => {
            let _ = write!(out, "{}", report::list(&items));
            0
        }
        Err(e) => {
            let _ = writeln!(err, "stockctl: {e}");
            1
        }
    }
}

fn cmd_summary(args: &[String], out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    let [path] = args else {
        let _ = writeln!(err, "stockctl summary: expected exactly one file\n{USAGE}");
        return 2;
    };
    match load(path) {
        Ok(items) => {
            let _ = write!(out, "{}", report::summary(&items));
            0
        }
        Err(e) => {
            let _ = writeln!(err, "stockctl: {e}");
            1
        }
    }
}
"####;

    const B01_PATCH: &str = r####"*** Begin Patch
*** Update File: src/cli.rs
@@
-const USAGE: &str = "usage: stockctl <list|summary> <file>";
+const USAGE: &str = "usage: stockctl <list|summary> <file> | stockctl export <file> --format <csv|json>";
@@
         Some("summary") => cmd_summary(&args[1..], out, err),
+        Some("export") => cmd_export(&args[1..], out, err),
@@
 fn cmd_list(args: &[String], out: &mut dyn Write, err: &mut dyn Write) -> i32 {
-    let [path] = args else {
-        let _ = writeln!(err, "stockctl list: expected exactly one file\n{USAGE}");
+    let (path, min_qty) = match args {
+        [path] => (path.as_str(), None),
+        [path, flag, value] if flag == "--min-qty" => match value.parse::<u32>() {
+            Ok(n) => (path.as_str(), Some(n)),
+            Err(_) => { let _ = writeln!(err, "stockctl list: invalid minimum quantity `{value}`\n{USAGE}"); return 2; }
+        },
+        _ => {
+            let _ = writeln!(err, "stockctl list: expected <file> [--min-qty N]\n{USAGE}");
+            return 2;
+        }
+    };
-        return 2;
-    };
     match load(path) {
         Ok(items) => {
-            let _ = write!(out, "{}", report::list(&items));
+            let filtered: Vec<Item> = items.into_iter().filter(|i| min_qty.is_none_or(|n| i.qty >= n)).collect();
+            let _ = write!(out, "{}", report::list(&filtered));
@@
 }
+
+fn cmd_export(args: &[String], out: &mut dyn Write, err: &mut dyn Write) -> i32 {
+    let [path, flag, format] = args else {
+        let _ = writeln!(err, "stockctl export: expected <file> --format <csv|json>\n{USAGE}");
+        return 2;
+    };
+    if flag != "--format" { let _ = writeln!(err, "stockctl export: expected --format <csv|json>\n{USAGE}"); return 2; }
+    if !["csv", "json"].contains(&format.as_str()) { let _ = writeln!(err, "stockctl export: unsupported format `{format}`; supported formats: csv, json"); return 2; }
+    match load(path) {
+        Ok(items) => { let _ = write!(out, "{}", if format == "csv" { report::export_csv(&items) } else { report::export_json(&items) }); 0 }
+        Err(e) => { let _ = writeln!(err, "stockctl: {e}"); 1 }
+    }
+}
@@
-            let _ = write!(out, "{}", report::summary(&items));
+            let mut text = report::summary(&items);
+            text.push_str(&report::categories(&items));
+            let _ = write!(out, "{text}");
*** End Patch"####;
}
