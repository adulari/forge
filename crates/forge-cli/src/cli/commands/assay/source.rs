//! Assay source bundling and scope resolution.

use std::path::Path;

/// Concatenate the analyzable source under `root` (capped) with `// FILE:` headers, for the crew
/// prompt. Skips VCS/build/vendor dirs; deterministic order. A single file is bundled directly.
pub(crate) fn bundle_source(root: &Path, max_bytes: usize) -> String {
    fn is_skip_dir(name: &str) -> bool {
        matches!(
            name,
            ".git" | "target" | ".forge" | "node_modules" | "graphify-out" | ".idea" | ".vscode"
        )
    }
    fn is_source(ext: &str) -> bool {
        matches!(
            ext,
            "rs" | "toml"
                | "md"
                | "py"
                | "js"
                | "ts"
                | "tsx"
                | "go"
                | "java"
                | "c"
                | "cpp"
                | "h"
                | "hpp"
                | "sh"
                | "yaml"
                | "yml"
                | "json"
                | "sql"
        )
    }
    fn append(out: &mut String, path: &Path) {
        if let Ok(content) = std::fs::read_to_string(path) {
            out.push_str(&format!("// FILE: {}\n{content}\n\n", path.display()));
        }
    }

    let mut out = String::new();
    if root.is_file() {
        append(&mut out, root);
        out.truncate(floor_char_boundary(&out, max_bytes));
        return out;
    }
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if out.len() >= max_bytes {
            break;
        }
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut paths: Vec<_> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
        paths.sort();
        for p in paths {
            if out.len() >= max_bytes {
                break;
            }
            if p.is_dir() {
                if !p
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(is_skip_dir)
                    .unwrap_or(false)
                {
                    stack.push(p);
                }
            } else if p
                .extension()
                .and_then(|e| e.to_str())
                .map(is_source)
                .unwrap_or(false)
            {
                append(&mut out, &p);
            }
        }
    }
    out.truncate(floor_char_boundary(&out, max_bytes));
    out
}

/// Bundle source for the given scope. For git-backed scopes (Diff/Branch/Since) the changed-file
/// list is resolved via `git diff --name-only`; only those files are bundled. Returns an error
/// string when a git scope is requested outside a repo or the git command fails.
pub(crate) fn bundle_scoped_source(
    scope: &forge_types::AssayScope,
    max_bytes: usize,
) -> Result<String, String> {
    use forge_types::AssayScope::*;
    let git_files = |args: &[&str]| -> Result<Vec<std::path::PathBuf>, String> {
        let out = std::process::Command::new("git")
            .args(args)
            .output()
            .map_err(|e| format!("git: {e}"))?;
        if !out.status.success() {
            let msg = String::from_utf8_lossy(&out.stderr).trim().to_string();
            return Err(format!("git {}: {msg}", args.join(" ")));
        }
        Ok(String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter(|l| !l.is_empty())
            .map(std::path::PathBuf::from)
            .collect())
    };
    match scope {
        Repo => Ok(bundle_source(std::path::Path::new("."), max_bytes)),
        Path(p) => Ok(bundle_source(std::path::Path::new(p), max_bytes)),
        Diff => {
            // `diff HEAD` (not bare `diff`) so STAGED changes are included — a plain `git diff`
            // compares the working tree to the index and silently drops anything already `git add`ed,
            // so a fully-staged change looked like "no uncommitted changes".
            let files = git_files(&["diff", "HEAD", "--name-only"])?;
            if files.is_empty() {
                return Err(
                    "no uncommitted changes (git diff HEAD --name-only returned nothing)".into(),
                );
            }
            Ok(bundle_file_list(&files, max_bytes))
        }
        Branch(base) => {
            let files = git_files(&["diff", "--name-only", &format!("{base}...HEAD")])?;
            if files.is_empty() {
                return Err(format!(
                    "no changes vs {base} (git diff --name-only {base}...HEAD returned nothing)"
                ));
            }
            Ok(bundle_file_list(&files, max_bytes))
        }
        Since(ref_) => {
            let files = git_files(&["diff", "--name-only", ref_])?;
            if files.is_empty() {
                return Err(format!(
                    "no changes since {ref_} (git diff --name-only {ref_} returned nothing)"
                ));
            }
            Ok(bundle_file_list(&files, max_bytes))
        }
    }
}

/// Bundle a specific list of file paths (e.g. from a git diff) with `// FILE:` headers.
pub(crate) fn bundle_file_list(files: &[std::path::PathBuf], max_bytes: usize) -> String {
    let mut out = String::new();
    for p in files {
        if out.len() >= max_bytes {
            break;
        }
        if let Ok(content) = std::fs::read_to_string(p) {
            out.push_str(&format!("// FILE: {}\n{content}\n\n", p.display()));
            if out.len() > max_bytes {
                out.truncate(floor_char_boundary(&out, max_bytes));
                break;
            }
        }
    }
    out
}

/// Largest index ≤ `max` that is a char boundary (so truncation never splits a UTF-8 char).
pub(crate) fn floor_char_boundary(s: &str, max: usize) -> usize {
    if max >= s.len() {
        return s.len();
    }
    let mut i = max;
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}
