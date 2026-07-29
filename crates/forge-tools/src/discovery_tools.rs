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

/// Search text files for a pattern, returning `path:lineno: line` matches. `path` may be a
/// directory (recursive walk) or a single file (models routinely pass a file path — the old
/// "is not a directory" error just burned a round-trip). Supports substring (default) or full
/// regex matching, and an optional file-path glob filter.
pub struct SearchTool;

const SEARCH_MATCH_CAP: usize = 200;

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
                }
            },
            "required": ["query"]
        })
    }
    async fn run(&self, args: &Value) -> Result<String, ToolError> {
        let query = str_arg(args, "query")?;
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
        let file_pattern = args.get("file_pattern").and_then(Value::as_str);
        let context = args
            .get("context")
            .and_then(Value::as_u64)
            .map(|n| n.min(10) as usize)
            .unwrap_or(0);

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

        // Offload the recursive walk + per-file reads to a blocking thread so a large-repo search
        // doesn't stall the async executor (and any concurrent subagents/streams) while it runs.
        let root = root.to_string();
        let query = query.to_string();
        tokio::task::spawn_blocking(move || -> Result<String, ToolError> {
            let mut matches: Vec<String> = Vec::new();
            if root_is_file {
                // A file path searches that single file — same matching semantics + output
                // format, the path column is the file as given. `file_pattern` is ignored:
                // the caller already named the exact file. A read failure is a real error
                // here (unlike the walk, where unreadable files are skipped silently).
                let content = std::fs::read_to_string(&root)
                    .map_err(|e| ToolError::Failed(format!("can't read {root}: {e}")))?;
                append_search_matches(&root, &content, re.as_ref(), &query, context, &mut matches);
            } else {
                let mut stack = vec![std::path::PathBuf::from(&root)];
                'walk: while let Some(dir) = stack.pop() {
                    let Ok(entries) = std::fs::read_dir(&dir) else {
                        continue;
                    };
                    for entry in entries.flatten() {
                        let name = entry.file_name().to_string_lossy().into_owned();
                        if name.starts_with('.') {
                            continue; // hidden files + dirs (.git, .venv, …)
                        }
                        let path = entry.path();
                        let Ok(ft) = entry.file_type() else { continue };
                        if ft.is_dir() {
                            // Skip heavy vendor/build dirs so non-Rust repos (node_modules, venv,
                            // …) don't bury real results. (`target` is skipped only as a dir.)
                            if SEARCH_SKIP_DIRS.contains(&name.as_str()) {
                                continue;
                            }
                            stack.push(path);
                        } else {
                            let rel = path.strip_prefix(&root).unwrap_or(&path);
                            if let Some(ref fg) = file_glob {
                                if !fg.is_match(rel) {
                                    continue;
                                }
                            }
                            if let Ok(content) = std::fs::read_to_string(&path) {
                                let label = rel.display().to_string();
                                if !append_search_matches(
                                    &label,
                                    &content,
                                    re.as_ref(),
                                    &query,
                                    context,
                                    &mut matches,
                                ) {
                                    break 'walk; // an output cap was hit — stop searching
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
        for &i in &hits {
            matches.push(format!("{label}:{}: {}", i + 1, lines[i].trim_end()));
            if matches.len() >= SEARCH_MATCH_CAP {
                matches.push(format!("… (capped at {SEARCH_MATCH_CAP} matches)"));
                return false;
            }
        }
    } else {
        for hunk in context_hunks(label, &lines, &hits, context) {
            if !matches.is_empty() {
                matches.push("--".into());
            }
            matches.push(hunk);
            if matches.iter().map(String::len).sum::<usize>() >= SEARCH_CONTEXT_OUTPUT_MAX_BYTES {
                matches.push("… (capped — narrow the query or file_pattern)".into());
                return false;
            }
        }
    }
    true
}

/// Total byte budget for a context-mode `search` result, so `context: N` over many hits can't flood
/// the model's context window. Once exceeded, remaining hunks are dropped with a "narrow it" note.
const SEARCH_CONTEXT_OUTPUT_MAX_BYTES: usize = 64 * 1024;

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
                    format!("{rel}:{}{} {}", n + 1, sep, lines[n].trim_end())
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
        tokio::task::spawn_blocking(move || -> Result<String, ToolError> {
            let mut matches: Vec<String> = Vec::new();
            let mut stack = vec![std::path::PathBuf::from(&root)];
            while let Some(dir) = stack.pop() {
                let Ok(entries) = std::fs::read_dir(&dir) else {
                    continue;
                };
                for entry in entries.flatten() {
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
        })
        .await
        .map_err(|e| ToolError::Failed(format!("glob task failed: {e}")))?
    }
}
