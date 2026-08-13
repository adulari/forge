//! The core coding tools shipped in v0.1.

use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

use async_trait::async_trait;
use forge_types::{DiffKind, FileDiff, SideEffect};
use serde_json::{json, Value};

use crate::{str_arg, Tool, ToolError};

// ---------------------------------------------------------------------------------------------
// Workspace confinement (defense-in-depth behind the permission gate).
//
// The in-process file tools call `tokio::fs` directly on a model-supplied path. Only the *shell*
// tool is Landlock-sandboxed, so in `accept-edits`/`bypass` mode an absolute or `../` path
// (`~/.ssh/id_rsa`, `/etc/passwd`) would otherwise let the model read or overwrite anything outside
// the project. Before any fs op the file tools resolve the target and require it to live under an
// allowed root:
//   - the WORKSPACE ROOT — the process current dir; Forge runs in, and resolves relative paths
//     against, the project/worktree directory (the bridge `mcp-serve` child and subagent worktrees
//     all set cwd to their workspace, so this is correct there too), or
//   - the system TEMP dir — so scratch-file workflows keep working.
// Anything else is refused with a clear error, by default, in every mode. This is a floor, not the
// only guard: the permission broker's secret denylist still runs ahead of it.
// ---------------------------------------------------------------------------------------------

/// The roots an in-process file op is allowed to touch (workspace cwd + system temp), canonicalized.
fn workspace_roots() -> Vec<PathBuf> {
    let scoped = crate::SESSION_WORKSPACE.try_with(Clone::clone).ok();
    let mut roots = Vec::new();
    if let Some(cwd) = scoped {
        roots.push(cwd.canonicalize().unwrap_or(cwd));
    } else if let Ok(cwd) = std::env::current_dir() {
        roots.push(cwd.canonicalize().unwrap_or(cwd));
    }
    // Scratch access is a standalone compatibility behavior. A scoped session may
    // only access its immutable workspace; otherwise sibling temp workspaces leak.
    if crate::SESSION_WORKSPACE.try_with(Clone::clone).is_err() {
        let tmp = std::env::temp_dir();
        roots.push(tmp.canonicalize().unwrap_or(tmp));
    }
    roots
}

/// Collapse `.` and `..` components lexically (no filesystem access), so a non-existent tail like
/// `a/../../etc/passwd` can't escape before symlink resolution runs.
fn lexical_clean(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Resolve `path` to an absolute, symlink- and `..`-collapsed form WITHOUT requiring the whole path
/// to exist (a file being created won't). Lexically collapses `.`/`..`, then canonicalizes the
/// deepest existing ancestor — resolving symlinks so a symlinked parent can't smuggle the target
/// out of the workspace — and re-appends the remaining components.
fn resolve_target(path: &Path) -> PathBuf {
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        crate::SESSION_WORKSPACE
            .try_with(Clone::clone)
            .or_else(|_| std::env::current_dir())
            .map(|c| c.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    };
    let cleaned = lexical_clean(&abs);
    let mut prefix: &Path = &cleaned;
    let mut tail: Vec<OsString> = Vec::new();
    loop {
        if let Ok(real) = prefix.canonicalize() {
            let mut out = real;
            for c in tail.iter().rev() {
                out.push(c);
            }
            return out;
        }
        match prefix.parent() {
            Some(parent) => {
                if let Some(name) = prefix.file_name() {
                    tail.push(name.to_os_string());
                }
                prefix = parent;
            }
            None => return cleaned,
        }
    }
}

pub(crate) fn normalize_target(path: &Path) -> PathBuf {
    resolve_target(path)
}

/// Refuse a path that resolves outside the allowed workspace roots. Returns the resolved path on
/// success so callers don't resolve twice. See the module-level confinement note.
pub(crate) fn confine(path_str: &str) -> Result<PathBuf, ToolError> {
    let target = resolve_target(Path::new(path_str));
    if workspace_roots().iter().any(|r| target.starts_with(r)) {
        Ok(target)
    } else {
        Err(ToolError::Failed(format!(
            "path '{path_str}' resolves outside the workspace and is refused \
             (workspace-confinement safety net). Operate on paths inside the project directory."
        )))
    }
}

/// Hard ceiling on any local file this crate will load whole into memory via `read_to_string`,
/// checked via a metadata pre-check BEFORE the read. Unlike `cap_read`/`cap_bytes` (which only
/// truncate what's RETURNED to the model, after the full file is already buffered), this stops
/// the allocation itself: `read_to_string` on a multi-GB file would try to allocate the whole
/// thing as a UTF-8 String before any cap applies, which can OOM the process. Mirrors the fix
/// web.rs applies to HTTP response bodies (see `MAX_BODY_BYTES` there) — comfortably above any
/// real source file yet far below a process-killing allocation.
const MAX_READABLE_FILE_BYTES: u64 = 64 * 1024 * 1024;

/// Refuse to load `path` whole into memory if it's over [`MAX_READABLE_FILE_BYTES`]. Call this
/// before every `tokio::fs::read_to_string`/`std::fs::read_to_string` in this module.
pub(super) async fn check_readable_size(path: &str) -> Result<(), ToolError> {
    let meta = tokio::fs::metadata(path).await?;
    if meta.len() > MAX_READABLE_FILE_BYTES {
        return Err(ToolError::Failed(format!(
            "{path} is {} MiB, over the {} MiB single-read limit — too large to load into memory \
             whole",
            meta.len() / (1024 * 1024),
            MAX_READABLE_FILE_BYTES / (1024 * 1024)
        )));
    }
    Ok(())
}

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
        "Read the contents of a UTF-8 text file, returned verbatim (no line numbers — so the text \
         can be matched exactly by edit_file). Optionally slice to a line range with \
         `start_line`/`end_line` (both 1-indexed, inclusive). Very large files are truncated; pass \
         a line range to read a specific section. To read SEVERAL files at once, pass `paths` (an \
         array) instead of `path` — they come back in one response under `===== <path> =====` \
         headers, which is far cheaper than one call per file when exploring. Always read a file \
         before editing it."
    }
    fn side_effect(&self) -> SideEffect {
        SideEffect::ReadOnly
    }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Single file to read." },
                "paths": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Several files to read in ONE call (batch). Each is returned \
                                    whole under a `===== <path> =====` header; per-file and total \
                                    size caps apply. Prefer this over many read_file calls when \
                                    gathering context. `start_line`/`end_line` are ignored here."
                },
                "start_line": {
                    "type": "integer",
                    "description": "First line to read (1-indexed, inclusive). Default: 1. \
                                    Single-file (`path`) only."
                },
                "end_line": {
                    "type": "integer",
                    "description": "Last line to read (1-indexed, inclusive). Default: end of file. \
                                    Single-file (`path`) only."
                }
            }
        })
    }
    async fn run(&self, args: &Value) -> Result<String, ToolError> {
        if let Some(paths) = args
            .get("paths")
            .and_then(Value::as_array)
            .filter(|paths| !paths.is_empty())
        {
            return Ok(read_many(paths).await);
        }
        let path = str_arg(args, "path")?;
        confine(path)?;
        check_readable_size(path).await?;
        let start_line = args
            .get("start_line")
            .and_then(Value::as_u64)
            .map(|n| n as usize);
        let end_line = args
            .get("end_line")
            .and_then(Value::as_u64)
            .map(|n| n as usize);

        let content = tokio::fs::read_to_string(path).await?;
        let out = if start_line.is_none() && end_line.is_none() {
            content
        } else {
            let lines: Vec<&str> = content.lines().collect();
            let start = start_line.unwrap_or(1).saturating_sub(1); // 0-indexed
            let end = end_line.map(|e| e.min(lines.len())).unwrap_or(lines.len());
            // Clamp start to `end` so an inverted range (start_line > end_line) yields an empty
            // slice instead of panicking `lines[7..3]` and crashing the whole turn on tool input.
            let start = start.min(end);
            lines[start..end].join("\n")
        };
        Ok(cap_read(out))
    }
}

/// Per-file byte cap for a batched `read_file` (`paths`). Smaller than the single-file cap so one
/// large file in a batch can't crowd out the others; the model can re-read it alone with a range.
const BATCH_READ_PER_FILE_BYTES: usize = 64 * 1024;
/// Total byte cap across a whole batch, so a many-file request can't flood context. Once exceeded,
/// remaining files are listed but not read (the model can request them in a follow-up batch).
const BATCH_READ_TOTAL_BYTES: usize = 256 * 1024;

/// Read several files in one call. A missing/unreadable file becomes an inline `[error: …]` block
/// rather than failing the whole batch — partial context still helps. Each file is capped, and the
/// batch stops reading once the total budget is spent (remaining paths are noted, not silently
/// dropped).
async fn read_many(paths: &[Value]) -> String {
    let mut out = String::new();
    let mut spent = 0usize;
    for (i, p) in paths.iter().enumerate() {
        let Some(path) = p.as_str() else { continue };
        out.push_str(&format!("===== {path} =====\n"));
        if let Err(e) = confine(path) {
            out.push_str(&format!("[error: {e}]\n"));
            continue;
        }
        if let Err(e) = check_readable_size(path).await {
            out.push_str(&format!("[error: {e}]\n"));
            continue;
        }
        if spent >= BATCH_READ_TOTAL_BYTES {
            let left = paths.len() - i;
            out.push_str(&format!(
                "[batch total cap reached — {left} file(s) not read; request them in a follow-up]\n"
            ));
            break;
        }
        match tokio::fs::read_to_string(path).await {
            Ok(content) => {
                let body = cap_bytes(content, BATCH_READ_PER_FILE_BYTES);
                spent += body.len();
                out.push_str(&body);
                if !out.ends_with('\n') {
                    out.push('\n');
                }
            }
            Err(e) => out.push_str(&format!("[error: {e}]\n")),
        }
    }
    out
}

/// Truncate at a byte budget on a char boundary, appending a range hint when cut.
fn cap_bytes(s: String, max: usize) -> String {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!(
        "{}\n[… truncated at {} KiB — read this file alone with a line range for more …]",
        &s[..end],
        max / 1024
    )
}

/// Hard cap on a single `read_file` result so one read can't flood the model's context. A whole
/// file over this is truncated (head kept — imports/signatures live there) with a marker telling
/// the model to request a specific line range instead.
const READ_MAX_BYTES: usize = 256 * 1024;

fn cap_read(s: String) -> String {
    if s.len() <= READ_MAX_BYTES {
        return s;
    }
    let mut end = READ_MAX_BYTES;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!(
        "{}\n[… file truncated at {} KiB — pass start_line/end_line to read a specific section …]",
        &s[..end],
        READ_MAX_BYTES / 1024
    )
}

/// Write (create/overwrite) a text file. Mutates the workspace.
pub struct WriteFileTool;

#[async_trait]
impl Tool for WriteFileTool {
    fn name(&self) -> &str {
        "write_file"
    }
    fn description(&self) -> &str {
        "Write content to a file at the given path, creating it or OVERWRITING it whole. For an \
         existing file, read it first and prefer edit_file for targeted changes — write_file \
         replaces the entire file, so any content you omit is lost. Best for new files or full \
         rewrites. For a very large generated file, write one coherent first chunk and continue \
         with append_file rather than risking an oversized single call."
    }
    fn side_effect(&self) -> SideEffect {
        SideEffect::Write
    }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Destination file. Emit this field before content."
                },
                "content": { "type": "string" }
            },
            "required": ["path", "content"]
        })
    }
    async fn run(&self, args: &Value) -> Result<String, ToolError> {
        let path = str_arg(args, "path")?;
        let content = str_arg(args, "content")?;
        confine(path)?;
        tokio::fs::write(path, content).await?;
        Ok(format!("wrote {} bytes to {path}", content.len()))
    }

    async fn preview(&self, args: &Value) -> Option<FileDiff> {
        let path = str_arg(args, "path").ok()?;
        let content = str_arg(args, "content").ok()?;
        confine(path).ok()?;
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

/// Append text to a file, creating it when absent. This gives models a structured, permissioned
/// alternative to giant one-shot writes or shell heredocs when generating large artifacts.
pub struct AppendFileTool;

#[async_trait]
impl Tool for AppendFileTool {
    fn name(&self) -> &str {
        "append_file"
    }

    fn description(&self) -> &str {
        "Append content verbatim to the end of a file, creating it if absent. Use write_file for \
         the first chunk, then append_file for subsequent coherent chunks of a large generated \
         file. This never replaces or deduplicates existing text, so do not retry an append unless \
         you verified the previous call failed."
    }

    fn side_effect(&self) -> SideEffect {
        SideEffect::Write
    }

    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "File to append to. Emit this field before content."
                },
                "content": { "type": "string", "description": "Text to append verbatim." }
            },
            "required": ["path", "content"]
        })
    }

    async fn run(&self, args: &Value) -> Result<String, ToolError> {
        use tokio::io::AsyncWriteExt as _;

        let path = str_arg(args, "path")?;
        let content = str_arg(args, "content")?;
        confine(path)?;
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .await?;
        file.write_all(content.as_bytes()).await?;
        file.flush().await?;
        Ok(format!("appended {} bytes to {path}", content.len()))
    }

    async fn preview(&self, args: &Value) -> Option<FileDiff> {
        let path = str_arg(args, "path").ok()?;
        let addition = str_arg(args, "content").ok()?;
        confine(path).ok()?;
        let old = tokio::fs::read_to_string(path).await.ok();
        let mut new = old.clone().unwrap_or_default();
        new.push_str(addition);
        Some(FileDiff {
            path: path.to_string(),
            kind: if old.is_some() {
                DiffKind::Modified
            } else {
                DiffKind::Created
            },
            old,
            new: Some(new),
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
        "Replace text in a file: swaps the single, EXACT occurrence of `old` with `new`. `old` must \
         match the file byte-for-byte including indentation and whitespace, and must be UNIQUE — \
         include enough surrounding lines of context that it matches exactly once. It is an error \
         if `old` is absent or appears more than once (then add more context and retry). Read the \
         file first so your `old` matches. To insert, set `old` to a unique nearby anchor and put \
         that anchor plus the new lines in `new`. For new files or whole-file rewrites use \
         write_file instead."
    }
    fn side_effect(&self) -> SideEffect {
        SideEffect::Write
    }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "File to edit." },
                "old": {
                    "type": "string",
                    "description": "Exact text to replace — must occur exactly once; include \
                     surrounding context to disambiguate."
                },
                "new": { "type": "string", "description": "Replacement text." }
            },
            "required": ["path", "old", "new"]
        })
    }
    async fn run(&self, args: &Value) -> Result<String, ToolError> {
        let path = str_arg(args, "path")?;
        let old = str_arg(args, "old")?;
        let new = str_arg(args, "new")?;

        confine(path)?;
        check_readable_size(path).await?;
        let content = tokio::fs::read_to_string(path).await?;
        let (updated, note) = apply_edit(&content, old, new)
            .map_err(|e| ToolError::Failed(format!("{e} (in {path})")))?;
        tokio::fs::write(path, &updated).await?;
        Ok(format!("edited {path} (1 replacement){note}"))
    }

    async fn preview(&self, args: &Value) -> Option<FileDiff> {
        let path = str_arg(args, "path").ok()?;
        let old = str_arg(args, "old").ok()?;
        let new = str_arg(args, "new").ok()?;
        confine(path).ok()?;
        check_readable_size(path).await.ok()?;
        let content = tokio::fs::read_to_string(path).await.ok()?;
        // Mirror run() (skip the diff and let run() surface the error when it can't apply).
        let (updated, _) = apply_edit(&content, old, new).ok()?;
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

/// Apply several `old → new` edits to ONE file in a single call. Mutates the workspace.
pub struct MultiEditTool;

#[async_trait]
impl Tool for MultiEditTool {
    fn name(&self) -> &str {
        "multi_edit"
    }
    fn description(&self) -> &str {
        "Apply several edits to ONE file in a single call, in order. Each edit is {old, new} with \
         exactly edit_file's rules (each `old` exact + unique, with a whitespace-insensitive \
         fallback). ATOMIC: applied in sequence to the in-memory file, and if ANY edit can't be \
         applied the file is left untouched and the failing edit is reported — so partial edits \
         never land. Prefer this over many edit_file calls when changing one file in several places."
    }
    fn side_effect(&self) -> SideEffect {
        SideEffect::Write
    }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "File to edit." },
                "edits": {
                    "type": "array",
                    "description": "Edits applied in order; each is {old, new} (same rules as edit_file).",
                    "items": {
                        "type": "object",
                        "properties": {
                            "old": { "type": "string" },
                            "new": { "type": "string" }
                        },
                        "required": ["old", "new"]
                    }
                }
            },
            "required": ["path", "edits"]
        })
    }
    async fn run(&self, args: &Value) -> Result<String, ToolError> {
        let path = str_arg(args, "path")?;
        let edits = multi_edit_pairs(args)?;
        confine(path)?;
        check_readable_size(path).await?;
        let original = tokio::fs::read_to_string(path).await?;
        let updated = apply_edits(&original, &edits)
            .map_err(|e| ToolError::Failed(format!("{e} (in {path}; no edits applied)")))?;
        tokio::fs::write(path, &updated).await?;
        Ok(format!("edited {path} ({} edits applied)", edits.len()))
    }

    async fn preview(&self, args: &Value) -> Option<FileDiff> {
        let path = str_arg(args, "path").ok()?;
        let edits = multi_edit_pairs(args).ok()?;
        confine(path).ok()?;
        check_readable_size(path).await.ok()?;
        let original = tokio::fs::read_to_string(path).await.ok()?;
        let updated = apply_edits(&original, &edits).ok()?;
        Some(FileDiff {
            path: path.to_string(),
            kind: DiffKind::Modified,
            old: Some(original),
            new: Some(updated),
            lang: lang_from_path(path),
            binary: false,
        })
    }
}

mod edits;
use edits::{apply_edit, apply_edits, multi_edit_pairs};

mod notebook;
pub use notebook::NotebookEditTool;

/// Apply a unified diff to the workspace via `git apply`. Mutates the workspace.
pub struct ApplyPatchTool;

#[async_trait]
impl Tool for ApplyPatchTool {
    fn name(&self) -> &str {
        "apply_patch"
    }
    fn description(&self) -> &str {
        "Apply a unified diff (git / `diff -u` format) to the workspace — best for multi-file or \
         large changes where a patch is cleaner than edit_file. The `--- a/path` / `+++ b/path` \
         headers name the files (a patch can also create or delete files). Applied with `git apply` \
         (line-number drift tolerated); if it doesn't apply cleanly the error is returned verbatim \
         so you can regenerate the patch. For small single-file edits prefer edit_file / multi_edit."
    }
    fn side_effect(&self) -> SideEffect {
        SideEffect::Write
    }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "patch": { "type": "string", "description": "A unified diff to apply." },
                "cwd": { "type": "string", "description": "Directory to apply in (default: current)." }
            },
            "required": ["patch"]
        })
    }
    async fn run(&self, args: &Value) -> Result<String, ToolError> {
        use tokio::io::AsyncWriteExt;
        let patch = str_arg(args, "patch")?;
        let cwd = args.get("cwd").and_then(Value::as_str).unwrap_or(".");
        // Confine the application directory to the workspace. `git apply` (without `--unsafe-paths`)
        // already rejects patches whose `a/`/`b/` headers escape the tree, so this gates the one
        // model-supplied path on this tool — the dir the patch is applied in.
        confine(cwd)?;
        let mut child = tokio::process::Command::new("git")
            // Apply byte-faithfully: a global core.autocrlf=true (the default on GitHub's
            // Windows runners) would otherwise rewrite the patched file's line endings.
            .args([
                "-c",
                "core.autocrlf=false",
                "-c",
                "core.eol=lf",
                "apply",
                "--recount",
                "--whitespace=nowarn",
                "-",
            ])
            .current_dir(cwd)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| ToolError::Failed(format!("spawning git apply: {e}")))?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(patch.as_bytes())
                .await
                .map_err(|e| ToolError::Failed(format!("writing patch to git apply: {e}")))?;
            if !patch.ends_with('\n') {
                stdin.write_all(b"\n").await.map_err(|e| {
                    ToolError::Failed(format!("writing patch terminator to git apply: {e}"))
                })?; // git apply wants a trailing newline
            }
            stdin
                .shutdown()
                .await
                .map_err(|e| ToolError::Failed(format!("closing patch input: {e}")))?;
        }
        let out = child
            .wait_with_output()
            .await
            .map_err(|e| ToolError::Failed(format!("running git apply: {e}")))?;
        if out.status.success() {
            let files = patch
                .lines()
                .filter(|l| l.starts_with("+++ "))
                .count()
                .max(1);
            Ok(format!("applied patch ({files} file(s) changed)"))
        } else {
            Err(ToolError::Failed(format!(
                "git apply failed (regenerate the patch against the current file): {}",
                String::from_utf8_lossy(&out.stderr).trim()
            )))
        }
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
        confine(path)?;
        tokio::fs::remove_file(path).await?;
        Ok(format!("deleted {path}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discovery_tools::{GlobTool, ListDirTool, SearchTool};
    use std::path::Path;
    use std::path::PathBuf;

    #[tokio::test]
    async fn apply_patch_applies_a_unified_diff() {
        let dir = temp_dir("applypatch");
        std::fs::write(dir.join("f.txt"), "a\nb\nc\n").unwrap();
        let patch = "--- a/f.txt\n+++ b/f.txt\n@@ -1,3 +1,3 @@\n a\n-b\n+B\n c\n";
        let out = ApplyPatchTool
            .run(&json!({ "patch": patch, "cwd": dir.to_str().unwrap() }))
            .await;
        assert!(out.is_ok(), "apply failed: {out:?}");
        assert_eq!(
            std::fs::read_to_string(dir.join("f.txt")).unwrap(),
            "a\nB\nc\n"
        );
        // A patch that doesn't match the file is reported as an error (not silently dropped).
        let bad = "--- a/f.txt\n+++ b/f.txt\n@@ -1 +1 @@\n-zzz\n+q\n";
        assert!(ApplyPatchTool
            .run(&json!({ "patch": bad, "cwd": dir.to_str().unwrap() }))
            .await
            .is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cap_read_truncates_oversized_with_a_marker() {
        let small = "fn main() {}".to_string();
        assert_eq!(cap_read(small.clone()), small, "small content is untouched");
        let big = "x".repeat(READ_MAX_BYTES + 5_000);
        let capped = cap_read(big);
        assert!(
            capped.len() <= READ_MAX_BYTES + 200,
            "capped near the limit"
        );
        assert!(
            capped.contains("truncated"),
            "explains the cut + how to read more"
        );
    }

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
        // A missing file INSIDE an allowed root (temp) passes confinement and fails as an IO error.
        let dir = temp_dir("delete-missing");
        let missing = dir.join("xyz.txt");
        let err = DeleteFileTool
            .run(&json!({ "path": missing.to_str().unwrap() }))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::Io(_)));
        let _ = std::fs::remove_dir_all(&dir);
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
        std::fs::create_dir(dir.join("node_modules")).unwrap();
        std::fs::write(dir.join("node_modules/n.txt"), "find ME").unwrap();

        let out = SearchTool
            .run(&json!({ "query": "find ME", "path": dir.to_str().unwrap() }))
            .await
            .unwrap();

        assert!(out.contains("a.txt:2: find ME here"), "got:\n{out}");
        assert!(!out.contains("target"), "must skip target/:\n{out}");
        assert!(!out.contains("g.txt"), "must skip .git/:\n{out}");
        assert!(!out.contains("n.txt"), "must skip node_modules/:\n{out}");
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
    async fn search_context_shows_surrounding_lines() {
        let dir = temp_dir("search-context");
        std::fs::write(dir.join("a.txt"), "l1\nl2\nNEEDLE\nl4\nl5").unwrap();

        let out = SearchTool
            .run(&json!({
                "query": "NEEDLE",
                "path": dir.to_str().unwrap(),
                "context": 1
            }))
            .await
            .unwrap();

        // match line uses `:`, context lines use `-`, only ±1 line shown
        assert!(out.contains("a.txt:3: NEEDLE"), "got:\n{out}");
        assert!(out.contains("a.txt:2- l2"), "got:\n{out}");
        assert!(out.contains("a.txt:4- l4"), "got:\n{out}");
        assert!(!out.contains("l1"), "context must not exceed N:\n{out}");
        assert!(!out.contains("l5"), "context must not exceed N:\n{out}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn search_context_merges_adjacent_hits_into_one_hunk() {
        let dir = temp_dir("search-context-merge");
        std::fs::write(dir.join("a.txt"), "HIT\nmid\nHIT\nx\ny\nz\nHIT").unwrap();

        let out = SearchTool
            .run(&json!({
                "query": "HIT",
                "path": dir.to_str().unwrap(),
                "context": 1
            }))
            .await
            .unwrap();

        // lines 1 and 3 (windows [1-2] and [2-4]) merge -> one hunk, no `--` between them;
        // line 7 is separated by gap -> its own hunk after a `--`.
        let sep_count = out.matches("\n--\n").count();
        assert_eq!(
            sep_count, 1,
            "expected exactly one hunk separator, got:\n{out}"
        );
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
    async fn search_accepts_a_file_path_and_searches_that_file() {
        // Baseline defect (harness-robustness wave 2): models routinely pass a FILE path and got
        // "X is not a directory" — a wasted round-trip. A file path must search that single file,
        // same output format, path column = the file as given.
        let dir = temp_dir("search-file-path");
        std::fs::write(dir.join("a.txt"), "hello\nfind ME here\nbye").unwrap();
        std::fs::write(dir.join("b.txt"), "find ME too").unwrap();
        let file = dir.join("a.txt");
        let file = file.to_str().unwrap();

        let out = SearchTool
            .run(&json!({ "query": "find ME", "path": file }))
            .await
            .unwrap();

        assert_eq!(out, format!("{file}:2: find ME here"), "got:\n{out}");
        assert!(!out.contains("b.txt"), "only the named file is searched");

        let out = SearchTool
            .run(&json!({ "query": "find ME", "path": file, "file_pattern": "*.rs" }))
            .await
            .unwrap();
        assert_eq!(
            out, "No matches found.",
            "the explicit file must satisfy file_pattern"
        );
        let out = SearchTool
            .run(&json!({ "query": "find ME", "path": file, "file_pattern": "*.txt" }))
            .await
            .unwrap();
        assert_eq!(out, format!("{file}:2: find ME here"));

        // Same regex + context semantics as the directory walk.
        let out = SearchTool
            .run(&json!({ "query": r"find \w+", "path": file, "regex": true, "context": 1 }))
            .await
            .unwrap();
        assert!(
            out.contains(&format!("{file}:2: find ME here")),
            "got:\n{out}"
        );
        assert!(out.contains(&format!("{file}:1- hello")), "got:\n{out}");
        assert!(out.contains(&format!("{file}:3- bye")), "got:\n{out}");

        // No hits in the file reads exactly like an empty directory search.
        let out = SearchTool
            .run(&json!({ "query": "absent", "path": file }))
            .await
            .unwrap();
        assert_eq!(out, "no matches for 'absent'");
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
        assert!(out.contains("forge-agent-tools"));
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
    async fn read_file_inverted_line_range_does_not_panic() {
        // start_line > end_line used to panic `lines[7..3]` and crash the turn on tool input.
        let dir = temp_dir("read-inverted");
        let path = dir.join("f.txt");
        std::fs::write(&path, "l1\nl2\nl3\nl4\nl5\nl6\nl7\nl8\nl9").unwrap();
        let out = ReadFileTool
            .run(&json!({ "path": path.to_str().unwrap(), "start_line": 8, "end_line": 3 }))
            .await
            .expect("inverted range must not panic");
        assert_eq!(out, "", "inverted range yields an empty slice, not a panic");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn read_file_requires_path() {
        let err = ReadFileTool.run(&json!({})).await.unwrap_err();
        assert!(matches!(err, ToolError::BadArgs(_)));
    }

    #[tokio::test]
    async fn read_file_ignores_an_empty_optional_batch_array() {
        // Tool-call schemas commonly materialize optional arrays as `[]`. A valid single `path`
        // must still win, otherwise a model receives an empty successful result and can falsely
        // claim that a non-empty file has no content.
        let dir = temp_dir("read-empty-batch");
        let path = dir.join("f.txt");
        std::fs::write(&path, "actual contents").unwrap();

        let out = ReadFileTool
            .run(&json!({ "path": path.to_str().unwrap(), "paths": [] }))
            .await
            .unwrap();

        assert_eq!(out, "actual contents");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn read_file_batches_multiple_paths_in_one_call() {
        let dir = temp_dir("read-batch");
        let a = dir.join("a.txt");
        let b = dir.join("b.txt");
        std::fs::write(&a, "alpha-body").unwrap();
        std::fs::write(&b, "beta-body").unwrap();

        let out = ReadFileTool
            .run(&json!({ "paths": [a.to_str().unwrap(), b.to_str().unwrap()] }))
            .await
            .unwrap();

        assert!(out.contains(&format!("===== {} =====", a.display())));
        assert!(out.contains("alpha-body"));
        assert!(out.contains(&format!("===== {} =====", b.display())));
        assert!(out.contains("beta-body"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn read_file_batch_reports_missing_file_inline_not_fatal() {
        let dir = temp_dir("read-batch-miss");
        let a = dir.join("present.txt");
        std::fs::write(&a, "here").unwrap();
        let missing = dir.join("nope.txt");

        let out = ReadFileTool
            .run(&json!({ "paths": [a.to_str().unwrap(), missing.to_str().unwrap()] }))
            .await
            .unwrap();

        assert!(out.contains("here"));
        assert!(out.contains("[error:"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- workspace confinement ----

    /// A `..` traversal carrying enough segments to climb past the filesystem root from wherever
    /// this checkout lives, so it lands on an absolute `/<target>` regardless of depth.
    ///
    /// A hardcoded count was location-dependent: from a checkout inside the system temp dir (CI
    /// scratch directories and `mktemp -d` clones do this), six `..` collapsed to `<tmp>/etc/passwd`
    /// — still inside a root `confine` allows deliberately. The assertion then failed while
    /// `confine` was behaving correctly, reporting what looks like a sandbox escape. Deriving the
    /// count from the actual depth removes the cliff rather than moving it.
    fn escaping_traversal(target: &str) -> String {
        let depth = std::env::current_dir()
            .map(|cwd| cwd.components().count())
            .unwrap_or(32);
        "../".repeat(depth + 1) + target
    }

    #[test]
    fn escaping_traversal_leaves_every_allowed_root() {
        let escaped = resolve_target(Path::new(&escaping_traversal("etc/passwd")));
        assert_eq!(
            escaped,
            Path::new("/etc/passwd"),
            "the traversal must reach the filesystem root, or the escape tests prove nothing"
        );
        assert!(
            !workspace_roots().iter().any(|r| escaped.starts_with(r)),
            "the escape target must sit outside every allowed root, including the temp dir"
        );
    }

    #[test]
    fn confine_allows_in_workspace_and_temp_but_rejects_escapes() {
        // A relative path inside the workspace (the crate dir during tests) is allowed.
        assert!(confine("Cargo.toml").is_ok());
        assert!(confine("src/core_tools.rs").is_ok());
        // The system temp dir is an allowed root (scratch workflows).
        let tmp = std::env::temp_dir().join("forge-confine-probe.txt");
        assert!(confine(tmp.to_str().unwrap()).is_ok());
        // Absolute escapes outside the workspace are refused.
        assert!(confine("/etc/passwd").is_err());
        if let Some(home) = std::env::var_os("HOME") {
            let ssh = Path::new(&home).join(".ssh/id_rsa");
            assert!(
                confine(ssh.to_str().unwrap()).is_err(),
                "must refuse an absolute path into $HOME/.ssh"
            );
        }
        // `..` traversal out of the workspace is refused (lexically collapsed before the check).
        assert!(confine(&escaping_traversal("etc/passwd")).is_err());
    }

    #[tokio::test]
    async fn write_file_refuses_path_outside_workspace() {
        let err = WriteFileTool
            .run(&json!({ "path": "/etc/forge_should_not_write", "content": "x" }))
            .await
            .unwrap_err();
        match err {
            ToolError::Failed(m) => assert!(m.contains("workspace"), "msg: {m}"),
            other => panic!("expected a confinement Failed error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn edit_file_refuses_dotdot_escape() {
        // Even with a real file existing, a `..`-escaping target must be refused before any fs op.
        let err = EditFileTool
            .run(&json!({
                "path": escaping_traversal("etc/hosts"),
                "old": "x",
                "new": "y"
            }))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::Failed(m) if m.contains("workspace")));
    }

    #[tokio::test]
    async fn list_dir_refuses_path_outside_workspace() {
        let err = ListDirTool
            .run(&json!({ "path": "/etc" }))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::Failed(m) if m.contains("workspace")));
    }

    #[tokio::test]
    async fn search_refuses_path_outside_workspace() {
        let err = SearchTool
            .run(&json!({ "query": "root", "path": "/etc" }))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::Failed(m) if m.contains("workspace")));
    }

    #[tokio::test]
    async fn search_errors_on_missing_root_instead_of_no_matches() {
        let dir = temp_dir("search-missing-root");
        let missing = dir.join("does-not-exist");
        let err = SearchTool
            .run(&json!({ "query": "x", "path": missing.to_str().unwrap() }))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::Failed(m) if m.contains("does not exist")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn glob_refuses_path_outside_workspace() {
        let err = GlobTool
            .run(&json!({ "pattern": "**/*", "path": "/etc" }))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::Failed(m) if m.contains("workspace")));
    }

    #[tokio::test]
    async fn glob_errors_on_missing_root_instead_of_no_matches() {
        let dir = temp_dir("glob-missing-root");
        let missing = dir.join("does-not-exist");
        let err = GlobTool
            .run(&json!({ "pattern": "**/*", "path": missing.to_str().unwrap() }))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::Failed(m) if m.contains("does not exist")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn read_file_refuses_oversized_file() {
        let dir = temp_dir("read-oversized");
        let path = dir.join("big.bin");
        // Sparse file: allocate a size over the cap without actually writing that many bytes.
        let f = std::fs::File::create(&path).unwrap();
        f.set_len(MAX_READABLE_FILE_BYTES + 1).unwrap();
        drop(f);

        let err = ReadFileTool
            .run(&json!({ "path": path.to_str().unwrap() }))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::Failed(m) if m.contains("too large")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn write_then_edit_within_workspace_still_works() {
        // Confinement must not break ordinary in-repo writes/edits (here via the temp root).
        let dir = temp_dir("confine-ok");
        let path = dir.join("f.txt");
        WriteFileTool
            .run(&json!({ "path": path.to_str().unwrap(), "content": "alpha" }))
            .await
            .unwrap();
        EditFileTool
            .run(&json!({ "path": path.to_str().unwrap(), "old": "alpha", "new": "beta" }))
            .await
            .unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "beta");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
