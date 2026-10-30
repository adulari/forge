//! File-backed items: a skill directory, a command, an agent, AGENTS.md or keybindings.json,
//! carried as a bundle of base64 files so one record always holds one coherent item.

use std::path::{Component, Path, PathBuf};

use anyhow::{bail, Context, Result};
use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use serde_json::{json, Value};

use super::MAX_ITEM_BYTES;

/// Reads `path` (a file, or a directory walked recursively) into `{"files":[{path,b64}]}`.
/// `Ok(None)` when it would exceed [`MAX_ITEM_BYTES`].
pub(super) fn bundle(path: &Path) -> Result<Option<Value>> {
    let mut files = Vec::new();
    let mut total = 0usize;
    if path.is_dir() {
        let mut stack = vec![PathBuf::new()];
        while let Some(rel) = stack.pop() {
            for entry in std::fs::read_dir(path.join(&rel)).context("read synced directory")? {
                let entry = entry?;
                let name = entry.file_name().to_string_lossy().into_owned();
                let child = rel.join(&name);
                let kind = entry.file_type()?;
                if kind.is_symlink() || name == ".git" || name.ends_with(".conflict") {
                    continue;
                }
                if kind.is_dir() {
                    stack.push(child);
                } else if kind.is_file() {
                    let bytes = std::fs::read(entry.path())?;
                    total += bytes.len();
                    if total > MAX_ITEM_BYTES {
                        return Ok(None);
                    }
                    files.push((child.to_string_lossy().replace('\\', "/"), bytes));
                }
            }
        }
    } else {
        let bytes = std::fs::read(path)?;
        if bytes.len() > MAX_ITEM_BYTES {
            return Ok(None);
        }
        files.push((String::new(), bytes));
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));
    let files: Vec<Value> = files
        .into_iter()
        .map(|(path, bytes)| json!({ "path": path, "b64": STANDARD.encode(bytes) }))
        .collect();
    Ok(Some(json!({ "files": files })))
}

fn decode(value: &Value) -> Result<Vec<(String, Vec<u8>)>> {
    let files = value
        .get("files")
        .and_then(Value::as_array)
        .context("synced item has no files")?;
    let mut out = Vec::new();
    let mut total = 0usize;
    for file in files {
        let rel = file.get("path").and_then(Value::as_str).unwrap_or_default();
        let bytes = STANDARD
            .decode(file.get("b64").and_then(Value::as_str).unwrap_or_default())
            .context("synced file is not valid base64")?;
        total += bytes.len();
        if total > MAX_ITEM_BYTES {
            bail!(
                "synced item is larger than the {} MiB cap",
                MAX_ITEM_BYTES / (1024 * 1024)
            );
        }
        if !rel.is_empty() {
            let clean = Path::new(rel)
                .components()
                .all(|c| matches!(c, Component::Normal(_)));
            if !clean || rel.starts_with('.') {
                bail!("synced item names a path outside its directory");
            }
        }
        out.push((rel.to_string(), bytes));
    }
    Ok(out)
}

/// A single path segment that cannot climb out of its parent directory.
pub(super) fn plain_name(name: &str) -> bool {
    !name.is_empty() && !name.starts_with('.') && !name.contains(['/', '\\', '\0']) && name != ".."
}

/// Writes a bundle to `target`: a file for a single unnamed entry, otherwise a directory that is
/// replaced wholesale. When `keep_conflict` is set the previous content is kept in `<target>.conflict`.
pub(super) fn write_bundle(target: &Path, value: &Value, keep_conflict: bool) -> Result<()> {
    let files = decode(value)?;
    if keep_conflict && target.exists() {
        if let Some(previous) = bundle(target)? {
            let name = target
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let copy = target.with_file_name(format!("{name}.conflict"));
            write_decoded(&copy, &decode(&previous)?)?;
        }
    }
    if target.is_dir() {
        std::fs::remove_dir_all(target)?;
    }
    write_decoded(target, &files)
}

fn write_decoded(target: &Path, files: &[(String, Vec<u8>)]) -> Result<()> {
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    for (rel, bytes) in files {
        let path = if rel.is_empty() {
            target.to_path_buf()
        } else {
            target.join(rel)
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        forge_config::write_atomic(&path, bytes)?;
    }
    Ok(())
}

pub(super) fn remove(target: &Path) -> Result<()> {
    if target.is_dir() {
        std::fs::remove_dir_all(target)?;
    } else if target.exists() {
        std::fs::remove_file(target)?;
    }
    Ok(())
}

pub(super) fn modified_ms(path: &Path) -> Option<u64> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    modified
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_millis() as u64)
}
