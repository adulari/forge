//! Unified-diff parsing and the helpers behind `GET /api/git/diff` / `GET /api/git/status`.

use std::path::Path;

use super::*;

/// Path out of a `diff --git a/<x> b/<y>` header. Git quotes non-ASCII/space paths in this header
/// unless `core.quotePath=false` is set (the callers do), so a plain split is faithful.
pub(super) fn header_paths(line: &str) -> Option<(String, String)> {
    let rest = line.strip_prefix("diff --git ")?;
    let (a, b) = rest.split_once(" b/")?;
    Some((a.strip_prefix("a/").unwrap_or(a).to_string(), b.to_string()))
}

/// Parse a multi-file unified diff into [`GitDiffFile`] rows, capping hunk lines per file.
pub(super) fn parse_unified_diff(raw: &str) -> Vec<GitDiffFile> {
    parse_unified_diff_window(raw, 0, MAX_DIFF_LINES)
}

/// [`parse_unified_diff`] returning hunk lines `[offset, offset + limit)` of each file (counted
/// across all of the file's hunks). A cap that only ever kept the head hid every addition of a
/// full rewrite (all the `-` lines come first); the window lets a client reach the rest.
pub(super) fn parse_unified_diff_window(
    raw: &str,
    offset: usize,
    limit: usize,
) -> Vec<GitDiffFile> {
    let limit = limit.min(MAX_DIFF_LINES);
    let mut files: Vec<GitDiffFile> = Vec::new();
    let mut current: Option<GitDiffFile> = None;
    for line in raw.split('\n') {
        if line.starts_with("diff --git ") {
            if let Some(file) = current.take() {
                files.push(file);
            }
            let (_, path) = header_paths(line).unwrap_or_default();
            current = Some(GitDiffFile {
                path,
                kind: "modified".to_string(),
                orig_path: None,
                binary: false,
                adds: 0,
                dels: 0,
                hunks: Vec::new(),
                skipped_lines: 0,
                offset,
                total_lines: 0,
            });
            continue;
        }
        let Some(file) = current.as_mut() else {
            continue;
        };
        if line.starts_with("new file mode") {
            file.kind = "created".to_string();
            file.orig_path = None;
            continue;
        }
        if line.starts_with("deleted file mode") {
            file.kind = "deleted".to_string();
            file.orig_path = None;
            continue;
        }
        if let Some(rename) = line.strip_prefix("rename from ") {
            file.kind = "renamed".to_string();
            file.orig_path = Some(rename.to_string());
            continue;
        }
        if line.starts_with("Binary files ") || line.starts_with("GIT binary patch") {
            file.binary = true;
            continue;
        }
        if line.starts_with("@@") {
            file.hunks.push(GitDiffHunk {
                header: line.to_string(),
                lines: Vec::new(),
            });
            continue;
        }
        // Everything before the first `@@` (index/mode/---/+++ lines) is metadata, not content.
        if file.hunks.is_empty() {
            continue;
        }
        match line.as_bytes().first() {
            Some(b'+') => file.adds += 1,
            Some(b'-') => file.dels += 1,
            Some(b' ') => {}
            // `\ No newline at end of file` and the blank line git emits after a diff.
            _ => continue,
        }
        let index = file.total_lines;
        file.total_lines += 1;
        if index < offset || index >= offset.saturating_add(limit) {
            file.skipped_lines += 1;
            continue;
        }
        if let Some(hunk) = file.hunks.last_mut() {
            hunk.lines.push(line.to_string());
        }
    }
    if let Some(file) = current.take() {
        files.push(file);
    }
    if offset > 0 {
        // A hunk wholly before the window would be a bare header; drop it.
        for file in &mut files {
            file.hunks.retain(|hunk| !hunk.lines.is_empty());
        }
    }
    files
}

/// Render an untracked file as an all-additions diff. `git diff` cannot produce one without
/// `--no-index` (which is not portable across the daemon's target platforms), so the content is
/// read directly — bounded by [`MAX_UNTRACKED_BYTES`] and by [`MAX_DIFF_LINES`].
pub(super) fn untracked_diff(
    root: &Path,
    rel: &str,
    offset: usize,
    limit: usize,
) -> Result<GitDiffFile, String> {
    let limit = limit.min(MAX_DIFF_LINES);
    let full = root.join(rel);
    let meta = std::fs::metadata(&full).map_err(|e| e.to_string())?;
    if !meta.is_file() {
        return Err("not a regular file".to_string());
    }
    let mut file = GitDiffFile {
        path: rel.to_string(),
        kind: "created".to_string(),
        orig_path: None,
        binary: false,
        adds: 0,
        dels: 0,
        hunks: Vec::new(),
        skipped_lines: 0,
        offset,
        total_lines: 0,
    };
    if meta.len() > MAX_UNTRACKED_BYTES {
        file.binary = true;
        return Ok(file);
    }
    let Ok(text) = std::fs::read_to_string(&full) else {
        file.binary = true;
        return Ok(file);
    };
    let mut lines: Vec<String> = Vec::new();
    for (index, line) in text.lines().enumerate() {
        file.adds += 1;
        file.total_lines += 1;
        if index < offset || index >= offset.saturating_add(limit) {
            file.skipped_lines += 1;
            continue;
        }
        lines.push(format!("+{line}"));
    }
    if !lines.is_empty() || offset == 0 {
        file.hunks.push(GitDiffHunk {
            header: format!("@@ -0,0 +1,{} @@", file.adds),
            lines,
        });
    }
    Ok(file)
}

/// Whether `rel` is an untracked, non-ignored path. `git diff` is empty both for an untracked file
/// and for an unchanged tracked one; only the former may be rendered as an all-additions hunk.
pub(super) fn is_untracked(root: &Path, rel: &str) -> bool {
    git_raw(
        root,
        &[
            "ls-files",
            "--others",
            "--exclude-standard",
            "-z",
            "--",
            rel,
        ],
    )
    .is_ok_and(|out| !out.is_empty())
}

/// The `(added, removed, binary)` an untracked file's status row reports: its line count, read
/// the same way the diff route renders it, so the row and the diff agree. `None` once the byte
/// budget for the whole status call is spent (the row keeps its zero counts).
pub(super) fn untracked_counts(
    root: &Path,
    rel: &str,
    budget: &mut u64,
) -> Option<(usize, usize, bool)> {
    let full = root.join(rel);
    let meta = std::fs::metadata(&full).ok()?;
    if !meta.is_file() {
        return None;
    }
    if meta.len() > MAX_UNTRACKED_BYTES {
        return Some((0, 0, true));
    }
    if *budget < meta.len() {
        return None;
    }
    *budget -= meta.len();
    match std::fs::read_to_string(&full) {
        Ok(text) => Some((text.lines().count(), 0, false)),
        Err(_) => Some((0, 0, true)),
    }
}

/// The index entry a staged `new_path` was renamed from, if any.
pub(super) fn staged_rename_source(root: &Path, new_path: &str) -> Option<String> {
    let raw = git_raw(
        root,
        &["diff", "--cached", "--find-renames", "--name-status", "-z"],
    )
    .ok()?;
    let text = String::from_utf8_lossy(&raw).into_owned();
    let mut fields = text.split('\0').filter(|f| !f.is_empty());
    while let Some(status) = fields.next() {
        if status.starts_with('R') || status.starts_with('C') {
            let (from, to) = (fields.next()?, fields.next()?);
            if to == new_path {
                return Some(from.to_string());
            }
        } else {
            fields.next();
        }
    }
    None
}
