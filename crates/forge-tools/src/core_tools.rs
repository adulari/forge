//! The core coding tools shipped in v0.1.

use async_trait::async_trait;
use forge_types::{DiffKind, FileDiff, SideEffect};
use globset::{Glob, GlobMatcher};
use serde_json::{json, Value};

use crate::{str_arg, Tool, ToolError};

/// Map a file extension to a syntax-highlighting language token (best-effort; unknown
/// extensions pass through and fall back to plain highlighting downstream).
fn lang_from_path(path: &str) -> Option<String> {
    let ext = std::path::Path::new(path).extension()?.to_str()?;
    let tok = match ext {
        "rs" => "rust",
        "py" => "python",
        "ts" | "tsx" => "typescript",
        "js" | "mjs" | "cjs" | "jsx" => "javascript",
        "go" => "go",
        "toml" => "toml",
        "json" => "json",
        "md" | "markdown" => "markdown",
        "sh" | "bash" => "bash",
        "yaml" | "yml" => "yaml",
        "html" | "htm" => "html",
        "css" => "css",
        "c" | "h" => "c",
        "cpp" | "cc" | "cxx" | "hpp" => "cpp",
        other => other,
    };
    Some(tok.to_string())
}

/// Read a UTF-8 text file. Supports optional line-range slicing.
pub struct ReadFileTool;

#[async_trait]
impl Tool for ReadFileTool {
    fn name(&self) -> &str {
        "read_file"
    }
    fn description(&self) -> &str {
        "Read the contents of a UTF-8 text file. Optionally slice to a line range with \
         `start_line`/`end_line` (both 1-indexed, inclusive) to avoid reading large files whole."
    }
    fn side_effect(&self) -> SideEffect {
        SideEffect::ReadOnly
    }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" },
                "start_line": {
                    "type": "integer",
                    "description": "First line to read (1-indexed, inclusive). Default: 1."
                },
                "end_line": {
                    "type": "integer",
                    "description": "Last line to read (1-indexed, inclusive). Default: end of file."
                }
            },
            "required": ["path"]
        })
    }
    async fn run(&self, args: &Value) -> Result<String, ToolError> {
        let path = str_arg(args, "path")?;
        let start_line = args.get("start_line").and_then(Value::as_u64).map(|n| n as usize);
        let end_line = args.get("end_line").and_then(Value::as_u64).map(|n| n as usize);

        let content = tokio::fs::read_to_string(path).await?;
        if start_line.is_none() && end_line.is_none() {
            return Ok(content);
        }
        let lines: Vec<&str> = content.lines().collect();
        let start = start_line.unwrap_or(1).saturating_sub(1); // 0-indexed
        let end = end_line.map(|e| e.min(lines.len())).unwrap_or(lines.len());
        Ok(lines[start.min(lines.len())..end].join("\n"))
    }
}

/// Write (create/overwrite) a text file. Mutates the workspace.
pub struct WriteFileTool;

#[async_trait]
impl Tool for WriteFileTool {
    fn name(&self) -> &str {
        "write_file"
    }
    fn description(&self) -> &str {
        "Write content to a file at the given path, creating or overwriting it."
    }
    fn side_effect(&self) -> SideEffect {
        SideEffect::Write
    }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" },
                "content": { "type": "string" }
            },
            "required": ["path", "content"]
        })
    }
    async fn run(&self, args: &Value) -> Result<String, ToolError> {
        let path = str_arg(args, "path")?;
        let content = str_arg(args, "content")?;
        tokio::fs::write(path, content).await?;
        Ok(format!("wrote {} bytes to {path}", content.len()))
    }

    async fn preview(&self, args: &Value) -> Option<FileDiff> {
        let path = str_arg(args, "path").ok()?;
        let content = str_arg(args, "content").ok()?;
        let old = tokio::fs::read_to_string(path).await.ok();
        let kind = if old.is_some() {
            DiffKind::Modified
        } else {
            DiffKind::Created
        };
        Some(FileDiff {
            path: path.to_string(),
            kind,
            old,
            new: Some(content.to_string()),
            lang: lang_from_path(path),
            binary: false,
        })
    }
}

/// Replace exactly one occurrence of `old` with `new` in a file. Mutates the workspace.
pub struct EditFileTool;

#[async_trait]
impl Tool for EditFileTool {
    fn name(&self) -> &str {
        "edit_file"
    }
    fn description(&self) -> &str {
        "Replace exactly one occurrence of `old` with `new` in the file at `path`."
    }
    fn side_effect(&self) -> SideEffect {
        SideEffect::Write
    }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" },
                "old": { "type": "string" },
                "new": { "type": "string" }
            },
            "required": ["path", "old", "new"]
        })
    }
    async fn run(&self, args: &Value) -> Result<String, ToolError> {
        let path = str_arg(args, "path")?;
        let old = str_arg(args, "old")?;
        let new = str_arg(args, "new")?;

        let content = tokio::fs::read_to_string(path).await?;
        let occurrences = content.matches(old).count();
        match occurrences {
            0 => return Err(ToolError::Failed(format!("`old` not found in {path}"))),
            1 => {}
            n => {
                return Err(ToolError::Failed(format!(
                    "`old` is ambiguous: {n} occurrences in {path}"
                )))
            }
        }

        let updated = content.replacen(old, new, 1);
        tokio::fs::write(path, &updated).await?;
        Ok(format!("edited {path} (1 replacement)"))
    }

    async fn preview(&self, args: &Value) -> Option<FileDiff> {
        let path = str_arg(args, "path").ok()?;
        let old = str_arg(args, "old").ok()?;
        let new = str_arg(args, "new").ok()?;
        let content = tokio::fs::read_to_string(path).await.ok()?;
        // Mirror run()'s exactly-one-occurrence rule; otherwise skip the diff and let run()
        // surface the real error.
        if content.matches(old).count() != 1 {
            return None;
        }
        let updated = content.replacen(old, new, 1);
        Some(FileDiff {
            path: path.to_string(),
            kind: DiffKind::Modified,
            old: Some(content),
            new: Some(updated),
            lang: lang_from_path(path),
            binary: false,
        })
    }
}

/// Delete a file. Mutates the workspace.
pub struct DeleteFileTool;

#[async_trait]
impl Tool for DeleteFileTool {
    fn name(&self) -> &str {
        "delete_file"
    }
    fn description(&self) -> &str {
        "Delete a file at the given path."
    }
    fn side_effect(&self) -> SideEffect {
        SideEffect::Write
    }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": { "path": { "type": "string" } },
            "required": ["path"]
        })
    }
    async fn run(&self, args: &Value) -> Result<String, ToolError> {
        let path = str_arg(args, "path")?;
        tokio::fs::remove_file(path).await?;
        Ok(format!("deleted {path}"))
    }
}

/// List the entries of a directory, sorted, directories marked with a trailing `/`.
pub struct ListDirTool;

#[async_trait]
impl Tool for ListDirTool {
    fn name(&self) -> &str {
        "list_dir"
    }
    fn description(&self) -> &str {
        "List the entries of a directory (directories marked with a trailing /)."
    }
    fn side_effect(&self) -> SideEffect {
        SideEffect::ReadOnly
    }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": { "path": { "type": "string" } }
        })
    }
    async fn run(&self, args: &Value) -> Result<String, ToolError> {
        let path = args.get("path").and_then(Value::as_str).unwrap_or(".");
        let meta = std::fs::metadata(path)?;
        if !meta.is_dir() {
            return Err(ToolError::Failed(format!("{path} is not a directory")));
        }
        let mut entries: Vec<String> = Vec::new();
        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if entry.file_type()?.is_dir() {
                entries.push(format!("{name}/"));
            } else {
                entries.push(name);
            }
        }
        entries.sort();
        Ok(entries.join("\n"))
    }
}

/// Recursively search text files for a pattern, returning `path:lineno: line` matches.
/// Supports substring (default) or full regex matching, and an optional file-path glob filter.
pub struct SearchTool;

const SEARCH_MATCH_CAP: usize = 50;

#[async_trait]
impl Tool for SearchTool {
    fn name(&self) -> &str {
        "search"
    }
    fn description(&self) -> &str {
        "Recursively search text files under `path` for lines matching `query`. \
         Set `regex: true` for regex matching (default: substring). \
         Use `file_pattern` (glob) to restrict which files are searched, e.g. \"**/*.rs\"."
    }
    fn side_effect(&self) -> SideEffect {
        SideEffect::ReadOnly
    }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string" },
                "path": { "type": "string" },
                "regex": {
                    "type": "boolean",
                    "description": "Treat `query` as a regex. Default: false (substring match)."
                },
                "file_pattern": {
                    "type": "string",
                    "description": "Glob to filter which files are searched, e.g. \"**/*.rs\"."
                }
            },
            "required": ["query"]
        })
    }
    async fn run(&self, args: &Value) -> Result<String, ToolError> {
        let query = str_arg(args, "query")?;
        let root = args.get("path").and_then(Value::as_str).unwrap_or(".");
        let use_regex = args.get("regex").and_then(Value::as_bool).unwrap_or(false);
        let file_pattern = args.get("file_pattern").and_then(Value::as_str);

        let re: Option<regex::Regex> = if use_regex {
            Some(
                regex::Regex::new(query)
                    .map_err(|e| ToolError::Failed(format!("invalid regex: {e}")))?,
            )
        } else {
            None
        };

        let file_glob: Option<GlobMatcher> = if let Some(pat) = file_pattern {
            Some(
                Glob::new(pat)
                    .map_err(|e| ToolError::Failed(format!("invalid file_pattern: {e}")))?
                    .compile_matcher(),
            )
        } else {
            None
        };

        let mut matches: Vec<String> = Vec::new();
        let mut stack = vec![std::path::PathBuf::from(root)];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with('.') || name == "target" {
                    continue;
                }
                let path = entry.path();
                let Ok(ft) = entry.file_type() else { continue };
                if ft.is_dir() {
                    stack.push(path);
                } else {
                    let rel = path.strip_prefix(root).unwrap_or(&path);
                    if let Some(ref fg) = file_glob {
                        if !fg.is_match(rel) {
                            continue;
                        }
                    }
                    if let Ok(content) = std::fs::read_to_string(&path) {
                        let rel_display = rel.display();
                        for (i, line) in content.lines().enumerate() {
                            let hit = if let Some(ref re) = re {
                                re.is_match(line)
                            } else {
                                line.contains(query)
                            };
                            if hit {
                                matches.push(format!(
                                    "{rel_display}:{}: {}",
                                    i + 1,
                                    line.trim_end()
                                ));
                                if matches.len() >= SEARCH_MATCH_CAP {
                                    matches.push(format!(
                                        "… (capped at {SEARCH_MATCH_CAP} matches)"
                                    ));
                                    return Ok(matches.join("\n"));
                                }
                            }
                        }
                    }
                }
            }
        }
        if matches.is_empty() {
            Ok(format!("no matches for '{query}'"))
        } else {
            Ok(matches.join("\n"))
        }
    }
}

/// List files matching a glob pattern, recursively. Skips hidden directories and `target/`.
pub struct GlobTool;

#[async_trait]
impl Tool for GlobTool {
    fn name(&self) -> &str {
        "glob"
    }
    fn description(&self) -> &str {
        "List files matching a glob pattern (e.g. \"**/*.rs\", \"src/**/*.toml\"). \
         Returns sorted relative paths. Skips hidden dirs and `target/`."
    }
    fn side_effect(&self) -> SideEffect {
        SideEffect::ReadOnly
    }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Glob pattern, e.g. \"**/*.rs\" or \"src/**/*.toml\"."
                },
                "path": {
                    "type": "string",
                    "description": "Root directory to search from (default: \".\")."
                }
            },
            "required": ["pattern"]
        })
    }
    async fn run(&self, args: &Value) -> Result<String, ToolError> {
        let pattern = str_arg(args, "pattern")?;
        let root = args.get("path").and_then(Value::as_str).unwrap_or(".");

        let matcher = Glob::new(pattern)
            .map_err(|e| ToolError::Failed(format!("invalid glob: {e}")))?
            .compile_matcher();

        let mut matches: Vec<String> = Vec::new();
        let mut stack = vec![std::path::PathBuf::from(root)];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with('.') || name == "target" {
                    continue;
                }
                let path = entry.path();
                let Ok(ft) = entry.file_type() else { continue };
                if ft.is_dir() {
                    stack.push(path);
                } else {
                    let rel = path.strip_prefix(root).unwrap_or(&path);
                    if matcher.is_match(rel) {
                        matches.push(rel.display().to_string());
                    }
                }
            }
        }

        if matches.is_empty() {
            Ok(format!("no files match '{pattern}'"))
        } else {
            matches.sort();
            Ok(matches.join("\n"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("forge-tools-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[tokio::test]
    async fn edit_file_replaces_a_unique_occurrence() {
        let dir = temp_dir("edit-unique");
        let path = dir.join("f.txt");
        std::fs::write(&path, "alpha BETA gamma").unwrap();

        EditFileTool
            .run(&json!({ "path": path.to_str().unwrap(), "old": "BETA", "new": "delta" }))
            .await
            .unwrap();

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "alpha delta gamma");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn edit_file_errors_when_old_is_missing() {
        let dir = temp_dir("edit-missing");
        let path = dir.join("f.txt");
        std::fs::write(&path, "nothing here").unwrap();
        let err = EditFileTool
            .run(&json!({ "path": path.to_str().unwrap(), "old": "ZZZ", "new": "x" }))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::Failed(_)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn edit_file_errors_when_old_is_ambiguous() {
        let dir = temp_dir("edit-ambiguous");
        let path = dir.join("f.txt");
        std::fs::write(&path, "dup dup").unwrap();
        let err = EditFileTool
            .run(&json!({ "path": path.to_str().unwrap(), "old": "dup", "new": "x" }))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::Failed(_)));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "dup dup");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn delete_file_removes_file() {
        let dir = temp_dir("delete");
        let path = dir.join("f.txt");
        std::fs::write(&path, "bye").unwrap();
        DeleteFileTool
            .run(&json!({ "path": path.to_str().unwrap() }))
            .await
            .unwrap();
        assert!(!path.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn delete_file_errors_on_missing() {
        let err = DeleteFileTool
            .run(&json!({ "path": "/no/such/file/xyz.txt" }))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::Io(_)));
    }

    #[tokio::test]
    async fn list_dir_lists_sorted_with_dir_markers() {
        let dir = temp_dir("listdir");
        std::fs::write(dir.join("file.txt"), "x").unwrap();
        std::fs::create_dir(dir.join("sub")).unwrap();

        let out = ListDirTool
            .run(&json!({ "path": dir.to_str().unwrap() }))
            .await
            .unwrap();
        assert_eq!(out, "file.txt\nsub/");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn list_dir_errors_on_non_directory() {
        let err = ListDirTool
            .run(&json!({ "path": "Cargo.toml" }))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::Failed(_)));
    }

    #[tokio::test]
    async fn search_finds_matches_and_skips_target_and_git() {
        let dir = temp_dir("search");
        std::fs::write(dir.join("a.txt"), "hello\nfind ME here\nbye").unwrap();
        std::fs::create_dir(dir.join("target")).unwrap();
        std::fs::write(dir.join("target/t.txt"), "find ME").unwrap();
        std::fs::create_dir(dir.join(".git")).unwrap();
        std::fs::write(dir.join(".git/g.txt"), "find ME").unwrap();

        let out = SearchTool
            .run(&json!({ "query": "find ME", "path": dir.to_str().unwrap() }))
            .await
            .unwrap();

        assert!(out.contains("a.txt:2: find ME here"), "got:\n{out}");
        assert!(!out.contains("target"), "must skip target/:\n{out}");
        assert!(!out.contains("g.txt"), "must skip .git/:\n{out}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn search_regex_matches_pattern() {
        let dir = temp_dir("search-regex");
        std::fs::write(dir.join("a.txt"), "fn hello() {}\nfn world() {}").unwrap();

        let out = SearchTool
            .run(&json!({
                "query": r"fn \w+\(\)",
                "path": dir.to_str().unwrap(),
                "regex": true
            }))
            .await
            .unwrap();

        assert!(out.contains("a.txt:1:"), "got:\n{out}");
        assert!(out.contains("a.txt:2:"), "got:\n{out}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn search_file_pattern_filters_extension() {
        let dir = temp_dir("search-filepattern");
        std::fs::write(dir.join("a.rs"), "needle").unwrap();
        std::fs::write(dir.join("b.txt"), "needle").unwrap();

        let out = SearchTool
            .run(&json!({
                "query": "needle",
                "path": dir.to_str().unwrap(),
                "file_pattern": "**/*.rs"
            }))
            .await
            .unwrap();

        assert!(out.contains("a.rs"), "got:\n{out}");
        assert!(!out.contains("b.txt"), "must skip non-rs:\n{out}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn glob_finds_files_by_pattern() {
        let dir = temp_dir("glob");
        std::fs::create_dir(dir.join("src")).unwrap();
        std::fs::write(dir.join("src/main.rs"), "").unwrap();
        std::fs::write(dir.join("src/lib.rs"), "").unwrap();
        std::fs::write(dir.join("README.md"), "").unwrap();

        let out = GlobTool
            .run(&json!({ "pattern": "**/*.rs", "path": dir.to_str().unwrap() }))
            .await
            .unwrap();

        assert!(out.contains("main.rs"), "got:\n{out}");
        assert!(out.contains("lib.rs"), "got:\n{out}");
        assert!(!out.contains("README.md"), "no md:\n{out}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn read_file_reads_workspace_manifest() {
        let out = ReadFileTool
            .run(&json!({ "path": "Cargo.toml" }))
            .await
            .unwrap();
        assert!(out.contains("forge-tools"));
    }

    #[tokio::test]
    async fn read_file_line_range() {
        let dir = temp_dir("read-range");
        let path = dir.join("f.txt");
        std::fs::write(&path, "line1\nline2\nline3\nline4\nline5").unwrap();

        let out = ReadFileTool
            .run(&json!({ "path": path.to_str().unwrap(), "start_line": 2, "end_line": 4 }))
            .await
            .unwrap();

        assert_eq!(out, "line2\nline3\nline4");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn read_file_requires_path() {
        let err = ReadFileTool.run(&json!({})).await.unwrap_err();
        assert!(matches!(err, ToolError::BadArgs(_)));
    }
}
