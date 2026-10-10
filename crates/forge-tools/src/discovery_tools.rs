//! Directory listing, search, and glob discovery tools.

use super::core_tools::confine;
use super::*;
use globset::{Glob, GlobMatcher};
use serde_json::json;

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
        confine(path)?;
        let path = path.to_string();
        tokio::task::spawn_blocking(move || -> Result<String, ToolError> {
            let meta = std::fs::metadata(&path)?;
            if !meta.is_dir() {
                return Err(ToolError::Failed(format!("{path} is not a directory")));
            }
            let mut entries: Vec<String> = Vec::new();
            for entry in std::fs::read_dir(&path)? {
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
        })
        .await
        .map_err(|e| ToolError::Failed(format!("list_dir task failed: {e}")))?
    }
}

/// The session workspace, when this call runs inside one. Result paths are shown relative to it so
/// the model can pass them straight to the next tool call, whatever sub-directory it searched.
fn workspace_base() -> Option<std::path::PathBuf> {
    crate::SESSION_WORKSPACE.try_with(Clone::clone).ok()
}

/// A file's label in a result: relative to the workspace when known, else to the search root.
fn result_label(path: &std::path::Path, root: &str, base: Option<&std::path::Path>) -> String {
    if let Some(rel) = base.and_then(|b| path.strip_prefix(b).ok()) {
        return rel.display().to_string();
    }
    let rel = path.strip_prefix(root).unwrap_or(path);
    if rel.as_os_str().is_empty() {
        return path.display().to_string();
    }
    rel.display().to_string()
}

/// Entries of `dir` in a stable order (by name): files first, then sub-directories, so a result
/// does not depend on the file system's enumeration order.
fn sorted_entries(dir: &std::path::Path) -> Vec<std::fs::DirEntry> {
    let Ok(read) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut entries: Vec<_> = read.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    entries
}

/// How a `search` reports its hits (Claude Code's `output_mode`).
#[derive(Clone, Copy, PartialEq, Eq)]
enum SearchMode {
    Content,
    Files,
    Count,
}

/// Search text files for a pattern, returning `path:lineno: line` matches. `path` may be a
/// directory (recursive walk) or a single file (models routinely pass a file path — the old
/// "is not a directory" error just burned a round-trip). Supports substring (default) or full
/// regex matching, and an optional file-path glob filter.
pub struct SearchTool;

const SEARCH_MATCH_CAP: usize = 200;

/// Longest a single matched line may be in a `search` result. A match line exists to tell the
/// model WHERE something is so it can open the file; past a few hundred bytes it stops being a
/// signal and becomes a data dump. Minified bundles, single-line JSON, and log files routinely
/// carry lines in the tens of kilobytes, and 200 of those is what turned one real `search` call
/// into a 651 KB (~163k token) tool result.
pub(crate) const SEARCH_LINE_MAX_BYTES: usize = 400;

/// Directory names skipped by `search` and `glob` (in addition to all dot-dirs): heavy vendor /
/// build / dependency trees that bury real results and aren't part of the source the agent edits.
const SEARCH_SKIP_DIRS: &[&str] = &[
    "node_modules",
    "target",
    "dist",
    "build",
    "vendor",
    "__pycache__",
    "venv",
    ".venv",
];

#[async_trait]
impl Tool for SearchTool {
    fn name(&self) -> &str {
        "search"
    }
    fn description(&self) -> &str {
        "Search text files for lines matching `query`. `path` may be a directory (searched \
         recursively) or a single file (searched by itself). \
         Set `regex: true` for regex matching (default: substring). \
         Use `file_pattern` (glob) to restrict which files are searched, e.g. \"**/*.rs\". \
         Set `context` to N to print N lines around each match (like grep -C) — often enough to \
         understand a hit WITHOUT a follow-up read_file, saving a round-trip. Context lines are \
         shown as `path:lineno-` and match lines as `path:lineno:`, with `--` between hunks."
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
                },
                "context": {
                    "type": "integer",
                    "description": "Lines of surrounding context to show around each match (grep -C). \
                                    Default 0 (match line only). Clamped to 10. Use this to read a \
                                    hit in place instead of a separate read_file call."
                },
                "case_insensitive": {
                    "type": "boolean",
                    "description": "Ignore case. Default: false."
                },
                "output_mode": {
                    "type": "string",
                    "enum": ["content", "files_with_matches", "count"],
                    "description": "content (default): matching lines. files_with_matches: only the \
                                    paths that match — use for \"where is X used\" over a big tree. \
                                    count: `path:N` per file."
                },
                "head_limit": {
                    "type": "integer",
                    "description": "Return at most this many result lines/paths."
                }
            },
            "required": ["query"]
        })
    }
    async fn run(&self, args: &Value) -> Result<String, ToolError> {
        // Claude Code's names (`pattern`, `glob`, `-i`, `-C`) are accepted as aliases: models
        // trained on that surface use them unprompted, and a refused call costs a round-trip.
        let query = args
            .get("query")
            .or_else(|| args.get("pattern"))
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::BadArgs("expected string 'query'".to_string()))?;
        let root = args.get("path").and_then(Value::as_str).unwrap_or(".");
        confine(root)?;
        let root_meta = std::fs::metadata(root).map_err(|e| {
            ToolError::Failed(format!(
                "path '{root}' does not exist or can't be read: {e}"
            ))
        })?;
        let root_is_file = root_meta.is_file();
        if !root_is_file && !root_meta.is_dir() {
            return Err(ToolError::Failed(format!(
                "{root} is neither a file nor a directory"
            )));
        }
        let use_regex = args.get("regex").and_then(Value::as_bool).unwrap_or(false);
        let file_pattern = args
            .get("file_pattern")
            .or_else(|| args.get("glob"))
            .and_then(Value::as_str);
        let context = ["context", "-C", "-A", "-B"]
            .iter()
            .filter_map(|key| args.get(*key).and_then(Value::as_u64))
            .max()
            .map(|n| n.min(10) as usize)
            .unwrap_or(0);
        let ignore_case = ["case_insensitive", "-i", "ignore_case"]
            .iter()
            .any(|key| args.get(*key).and_then(Value::as_bool).unwrap_or(false));
        let mode = match args.get("output_mode").and_then(Value::as_str) {
            Some("files_with_matches") => SearchMode::Files,
            Some("count") => SearchMode::Count,
            _ => SearchMode::Content,
        };
        let head_limit = args
            .get("head_limit")
            .and_then(Value::as_u64)
            .filter(|n| *n > 0)
            .map(|n| n as usize);

        let re: Option<regex::Regex> = if use_regex || ignore_case {
            let source = if use_regex {
                query.to_string()
            } else {
                regex::escape(query)
            };
            let source = if ignore_case {
                format!("(?i){source}")
            } else {
                source
            };
            Some(
                regex::Regex::new(&source)
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

        // Offload the recursive walk + per-file reads to a blocking thread so a large-repo search
        // doesn't stall the async executor (and any concurrent subagents/streams) while it runs.
        let root = root.to_string();
        let query = query.to_string();
        let base = workspace_base();
        tokio::task::spawn_blocking(move || -> Result<String, ToolError> {
            let base = base.as_deref();
            let mut matches: Vec<String> = Vec::new();
            let hit_count = |content: &str| match &re {
                Some(re) => content.lines().filter(|line| re.is_match(line)).count(),
                None => content.lines().filter(|line| line.contains(&query)).count(),
            };
            let report =
                |label: &str, content: &str, per_file_cap: usize, out: &mut Vec<String>| match mode
                {
                    SearchMode::Content => append_search_matches(
                        label,
                        content,
                        re.as_ref(),
                        &query,
                        context,
                        per_file_cap,
                        out,
                    ),
                    SearchMode::Files | SearchMode::Count => {
                        let hits = hit_count(content);
                        if hits > 0 {
                            out.push(if mode == SearchMode::Count {
                                format!("{label}:{hits}")
                            } else {
                                label.to_string()
                            });
                        }
                        out.len() < SEARCH_FILE_LIST_CAP
                    }
                };
            if root_is_file {
                // A file path searches that single file with the same matching semantics and
                // output format as a recursive search. The optional filter still applies.
                if let Some(ref fg) = file_glob {
                    let path = std::path::Path::new(&root);
                    let name = path.file_name().map(std::path::Path::new).unwrap_or(path);
                    if !fg.is_match(path) && !fg.is_match(name) {
                        return Ok("No matches found.".to_string());
                    }
                }
                // A read failure is a real error here (unlike the walk, where unreadable files
                // are skipped silently).
                let content = std::fs::read_to_string(&root)
                    .map_err(|e| ToolError::Failed(format!("can't read {root}: {e}")))?;
                let label = result_label(std::path::Path::new(&root), &root, base);
                report(&label, &content, usize::MAX, &mut matches);
            } else {
                let mut stack = vec![std::path::PathBuf::from(&root)];
                'walk: while let Some(dir) = stack.pop() {
                    let mut subdirs = Vec::new();
                    for entry in sorted_entries(&dir) {
                        let name = entry.file_name().to_string_lossy().into_owned();
                        if name.starts_with('.') {
                            continue; // hidden files + dirs (.git, .venv, …)
                        }
                        let path = entry.path();
                        let Ok(ft) = entry.file_type() else { continue };
                        if ft.is_dir() {
                            // Skip heavy vendor/build dirs so non-Rust repos (node_modules, venv,
                            // …) don't bury real results. (`target` is skipped only as a dir.)
                            if !SEARCH_SKIP_DIRS.contains(&name.as_str()) {
                                subdirs.push(path);
                            }
                            continue;
                        }
                        let rel = path.strip_prefix(&root).unwrap_or(&path);
                        if let Some(ref fg) = file_glob {
                            if !fg.is_match(rel) {
                                continue;
                            }
                        }
                        if let Ok(content) = std::fs::read_to_string(&path) {
                            let label = result_label(&path, &root, base);
                            if !report(&label, &content, SEARCH_PER_FILE_CAP, &mut matches) {
                                break 'walk; // an output cap was hit — stop searching
                            }
                        }
                    }
                    // Pushed in reverse so the stack pops them in name order.
                    stack.extend(subdirs.into_iter().rev());
                }
            }
            if matches.is_empty() {
                return Ok(format!("no matches for '{query}'"));
            }
            if let Some(limit) = head_limit.filter(|limit| matches.len() > *limit) {
                matches.truncate(limit);
                matches.push(format!("… (head_limit {limit} reached)"));
            }
            Ok(matches.join("\n"))
        })
        .await
        .map_err(|e| ToolError::Failed(format!("search task failed: {e}")))?
    }
}

/// Scan one file's `content` for `query` matches and append rendered output lines to `matches` —
/// the shared match/render core of [`SearchTool`] for both the directory walk and the single-file
/// path. `label` is the path column. Returns `false` when an output cap was hit (the caller must
/// stop searching; the cap note has already been appended).
fn append_search_matches(
    label: &str,
    content: &str,
    re: Option<&regex::Regex>,
    query: &str,
    context: usize,
    per_file_cap: usize,
    matches: &mut Vec<String>,
) -> bool {
    let lines: Vec<&str> = content.lines().collect();
    let hits: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| {
            if let Some(re) = re {
                re.is_match(line)
            } else {
                line.contains(query)
            }
        })
        .map(|(i, _)| i)
        .collect();
    if hits.is_empty() {
        return true;
    }
    if context == 0 {
        // One file full of hits (a log, a lockfile, generated code) used to fill the whole budget
        // and hide every other file. Show a sample per file and say how many more it has.
        let shown = hits.len().min(per_file_cap);
        for &i in &hits[..shown] {
            matches.push(format!(
                "{label}:{}: {}",
                i + 1,
                cap_line(lines[i].trim_end())
            ));
            if matches.len() >= SEARCH_MATCH_CAP {
                matches.push(format!("… (capped at {SEARCH_MATCH_CAP} matches)"));
                return false;
            }
            if matches.iter().map(String::len).sum::<usize>() >= SEARCH_OUTPUT_MAX_BYTES {
                matches.push("… (capped — narrow the query or file_pattern)".into());
                return false;
            }
        }
        if hits.len() > shown {
            matches.push(format!(
                "{label}: … {} more matches in this file (search it with a narrower query or \
                 read it directly)",
                hits.len() - shown
            ));
        }
    } else {
        for mut hunk in context_hunks(label, &lines, &hits, context) {
            if !matches.is_empty() {
                matches.push("--".into());
            }
            // Checked before pushing, not only after: one merged hunk over a dense file ran a
            // "64 KB" result to 272 KB.
            let room = SEARCH_OUTPUT_MAX_BYTES
                .saturating_sub(matches.iter().map(String::len).sum::<usize>());
            if hunk.len() > room {
                let mut end = room;
                while !hunk.is_char_boundary(end) {
                    end -= 1;
                }
                hunk.truncate(end);
                matches.push(hunk);
                matches.push("… (capped — narrow the query or file_pattern)".into());
                return false;
            }
            matches.push(hunk);
            if matches.iter().map(String::len).sum::<usize>() >= SEARCH_OUTPUT_MAX_BYTES {
                matches.push("… (capped — narrow the query or file_pattern)".into());
                return false;
            }
        }
    }
    true
}

/// Total byte budget for a `search` result, so no single call can flood the model's context
/// window. Once exceeded, remaining matches/hunks are dropped with a "narrow it" note.
///
/// This used to apply ONLY to the context mode, which left the DEFAULT mode (`context: 0`) bounded
/// by [`SEARCH_MATCH_CAP`] alone — a count, which says nothing about size. A real
/// `search{path: ".", query: "213"}` over a tree containing log files hit exactly 200 matches of
/// ~3 KB log lines and returned 651,214 bytes (~163k tokens) in one tool result, more input than
/// most whole sessions. A count cap cannot bound output; only a byte budget can.
pub(crate) const SEARCH_OUTPUT_MAX_BYTES: usize = 64 * 1024;

/// Most paths a `files_with_matches`/`count` search lists.
const SEARCH_FILE_LIST_CAP: usize = 500;

/// Most matching lines shown per file in the default mode; the rest are counted.
pub(crate) const SEARCH_PER_FILE_CAP: usize = 25;

/// Trim one matched line to [`SEARCH_LINE_MAX_BYTES`], on a char boundary, saying how much was
/// dropped so the model can tell a truncated line from a genuinely short one.
fn cap_line(line: &str) -> std::borrow::Cow<'_, str> {
    if line.len() <= SEARCH_LINE_MAX_BYTES {
        return std::borrow::Cow::Borrowed(line);
    }
    let mut end = SEARCH_LINE_MAX_BYTES;
    while end > 0 && !line.is_char_boundary(end) {
        end -= 1;
    }
    std::borrow::Cow::Owned(format!(
        "{}… (+{} bytes on this line)",
        &line[..end],
        line.len() - end
    ))
}

/// Build grep -C-style context hunks for one file: merge each match's `[i-ctx, i+ctx]` window with
/// adjacent/overlapping windows so a cluster of nearby hits prints as ONE block, then render with
/// ripgrep's convention — match lines as `path:lineno:`, context lines as `path:lineno-`, `--`
/// between non-contiguous hunks. `hits` must be sorted ascending (it is, by construction).
fn context_hunks(rel: &str, lines: &[&str], hits: &[usize], ctx: usize) -> Vec<String> {
    let hit_set: std::collections::HashSet<usize> = hits.iter().copied().collect();
    let mut windows: Vec<(usize, usize)> = Vec::new();
    for &i in hits {
        let lo = i.saturating_sub(ctx);
        let hi = (i + ctx).min(lines.len().saturating_sub(1));
        match windows.last_mut() {
            // Merge when this window touches or overlaps the previous one.
            Some((_, prev_hi)) if lo <= *prev_hi + 1 => *prev_hi = (*prev_hi).max(hi),
            _ => windows.push((lo, hi)),
        }
    }
    windows
        .into_iter()
        .map(|(lo, hi)| {
            (lo..=hi)
                .map(|n| {
                    let sep = if hit_set.contains(&n) { ':' } else { '-' };
                    format!("{rel}:{}{} {}", n + 1, sep, cap_line(lines[n].trim_end()))
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
        .collect()
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
        confine(root)?;
        let root_meta = std::fs::metadata(root).map_err(|e| {
            ToolError::Failed(format!(
                "path '{root}' does not exist or can't be read: {e}"
            ))
        })?;
        if !root_meta.is_dir() {
            return Err(ToolError::Failed(format!("{root} is not a directory")));
        }

        let matcher = Glob::new(pattern)
            .map_err(|e| ToolError::Failed(format!("invalid glob: {e}")))?
            .compile_matcher();

        let root = root.to_string();
        let pattern = pattern.to_string();
        let base = workspace_base();
        tokio::task::spawn_blocking(move || -> Result<String, ToolError> {
            let mut matches: Vec<String> = Vec::new();
            let mut stack = vec![std::path::PathBuf::from(&root)];
            while let Some(dir) = stack.pop() {
                for entry in sorted_entries(&dir) {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    if name.starts_with('.') {
                        continue; // hidden files + dirs (.git, .venv, …)
                    }
                    let path = entry.path();
                    let Ok(ft) = entry.file_type() else { continue };
                    if ft.is_dir() {
                        // Skip heavy vendor/build dirs so non-Rust repos (node_modules, venv, …)
                        // don't bury real results. (`target` is skipped only as a directory.)
                        if SEARCH_SKIP_DIRS.contains(&name.as_str()) {
                            continue;
                        }
                        stack.push(path);
                    } else {
                        let rel = path.strip_prefix(&root).unwrap_or(&path);
                        if matcher.is_match(rel) {
                            matches.push(result_label(&path, &root, base.as_deref()));
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
        })
        .await
        .map_err(|e| ToolError::Failed(format!("glob task failed: {e}")))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ToolRegistry;

    fn workspace() -> (tempfile::TempDir, ToolRegistry) {
        let dir = tempfile::tempdir().unwrap();
        for (path, text) in [
            ("crates/b/src/lib.rs", "Alpha one\nbeta\n"),
            ("crates/a/src/lib.rs", "alpha two\nalpha three\n"),
            ("crates/a/Cargo.toml", "name = \"a\"\n"),
            ("top.txt", "ALPHA\n"),
        ] {
            let path = dir.path().join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        let registry = ToolRegistry::with_core_tools_in(dir.path());
        (dir, registry)
    }

    async fn call(registry: &ToolRegistry, tool: &str, args: Value) -> String {
        registry.get(tool).unwrap().run(&args).await.unwrap()
    }

    #[tokio::test]
    async fn search_paths_are_workspace_relative_and_ordered_whatever_the_search_root() {
        let (_dir, registry) = workspace();
        let out = call(
            &registry,
            "search",
            json!({ "query": "alpha", "path": "crates" }),
        )
        .await;
        assert_eq!(
            out,
            "crates/a/src/lib.rs:1: alpha two\ncrates/a/src/lib.rs:2: alpha three"
        );
        let single = call(
            &registry,
            "search",
            json!({ "query": "beta", "path": "crates/b/src/lib.rs" }),
        )
        .await;
        assert_eq!(single, "crates/b/src/lib.rs:2: beta");
    }

    #[tokio::test]
    async fn search_supports_case_folding_file_lists_counts_and_claude_code_names() {
        let (_dir, registry) = workspace();
        let ci = call(
            &registry,
            "search",
            json!({ "query": "alpha", "case_insensitive": true, "output_mode": "files_with_matches" }),
        )
        .await;
        assert_eq!(ci, "top.txt\ncrates/a/src/lib.rs\ncrates/b/src/lib.rs");
        let count = call(
            &registry,
            "search",
            json!({ "pattern": "alpha", "-i": true, "output_mode": "count", "glob": "**/*.rs" }),
        )
        .await;
        assert_eq!(count, "crates/a/src/lib.rs:2\ncrates/b/src/lib.rs:1");
        let limited = call(
            &registry,
            "search",
            json!({ "query": "alpha", "head_limit": 1 }),
        )
        .await;
        assert_eq!(
            limited,
            "crates/a/src/lib.rs:1: alpha two\n… (head_limit 1 reached)"
        );
    }

    #[tokio::test]
    async fn glob_lists_workspace_relative_paths_from_a_sub_directory() {
        let (_dir, registry) = workspace();
        let out = call(
            &registry,
            "glob",
            json!({ "pattern": "**/lib.rs", "path": "crates" }),
        )
        .await;
        assert_eq!(out, "crates/a/src/lib.rs\ncrates/b/src/lib.rs");
    }
}
