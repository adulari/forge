//! `multi_edit`: ordered edits to one file, or to many files in a single atomic call.

use async_trait::async_trait;
use forge_types::{DiffKind, FileDiff, SideEffect};
use serde_json::{json, Value};

use super::edits::{apply_edits, EditStep};
use super::{check_readable_size, confine, lang_from_path, shown};
use crate::{Tool, ToolError};

/// The file each step targets, in first-appearance order. A step's own `path` wins; a step with
/// none uses the call's top-level `path`.
fn plan(args: &Value) -> Result<Vec<(String, Vec<EditStep>)>, ToolError> {
    let items = args
        .get("edits")
        .and_then(Value::as_array)
        .ok_or_else(|| ToolError::Failed("`edits` must be an array of {old, new}".to_string()))?;
    if items.is_empty() {
        return Err(ToolError::Failed("`edits` is empty".to_string()));
    }
    let default_path = args.get("path").and_then(Value::as_str);
    let mut files: Vec<(String, Vec<EditStep>)> = Vec::new();
    for (index, item) in items.iter().enumerate() {
        let (Some(old), Some(new)) = (
            item.get("old").and_then(Value::as_str),
            item.get("new").and_then(Value::as_str),
        ) else {
            return Err(ToolError::Failed(
                "each edit needs string `old` and `new`".to_string(),
            ));
        };
        let path = item
            .get("path")
            .and_then(Value::as_str)
            .or(default_path)
            .ok_or_else(|| {
                ToolError::BadArgs(format!(
                    "edit #{} has no `path` and the call has no top-level `path`",
                    index + 1
                ))
            })?;
        let step = EditStep {
            old: old.to_string(),
            new: new.to_string(),
            replace_all: item
                .get("replace_all")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        };
        match files.iter_mut().find(|(existing, _)| existing == path) {
            Some((_, steps)) => steps.push(step),
            None => files.push((path.to_string(), vec![step])),
        }
    }
    Ok(files)
}

/// Apply several `old → new` edits in a single call, to one file or across many. Mutates the
/// workspace.
pub struct MultiEditTool;

#[async_trait]
impl Tool for MultiEditTool {
    fn name(&self) -> &str {
        "multi_edit"
    }
    fn description(&self) -> &str {
        "Apply several edits in ONE call, in order — to one file (top-level `path`) or across MANY \
         files (give each edit its own `path`, e.g. to rename a symbol everywhere). Each edit is \
         {old, new} with exactly edit_file's rules (each `old` exact + unique, with a \
         whitespace-insensitive fallback, or set that edit's `replace_all: true` to change every \
         occurrence in its file). ATOMIC across every file: nothing is written unless every edit \
         applies, and a failure names the edit by index and file. Prefer this over repeated \
         edit_file calls, and over shell `sed`, for any change touching more than one place."
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
                    "description": "Default file for edits that carry no `path` of their own."
                },
                "edits": {
                    "type": "array",
                    "description": "Edits applied in order; each is {old, new} (same rules as \
                                    edit_file), optionally with its own `path`.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "path": {
                                "type": "string",
                                "description": "File this edit applies to (default: the \
                                                top-level `path`)."
                            },
                            "old": { "type": "string" },
                            "new": { "type": "string" },
                            "replace_all": {
                                "type": "boolean",
                                "description": "Replace every occurrence of this edit's `old` \
                                 instead of requiring a unique match. Defaults to false."
                            }
                        },
                        "required": ["old", "new"]
                    }
                }
            },
            "required": ["edits"]
        })
    }
    async fn run(&self, args: &Value) -> Result<String, ToolError> {
        let files = plan(args)?;
        let multi = files.len() > 1;
        let mut staged = Vec::with_capacity(files.len());
        for (path, steps) in &files {
            confine(path)?;
            check_readable_size(path).await?;
            let original = tokio::fs::read_to_string(path).await?;
            let updated = apply_edits(&original, steps).map_err(|e| {
                ToolError::Failed(format!("{e} (in {path}; no edits applied to any file)"))
            })?;
            staged.push((path, original, updated, steps.len()));
        }
        for (written, (path, _, updated, _)) in staged.iter().enumerate() {
            if let Err(error) = tokio::fs::write(path, updated).await {
                for (path, original, _, _) in &staged[..written] {
                    let _ = tokio::fs::write(path, original).await;
                }
                return Err(error.into());
            }
        }
        let total: usize = staged.iter().map(|(.., n)| n).sum();
        if !multi {
            return Ok(format!(
                "edited {} ({total} edits applied)",
                shown(staged[0].0)
            ));
        }
        let each = staged
            .iter()
            .map(|(path, _, _, n)| format!("{} ({n})", shown(path)))
            .collect::<Vec<_>>()
            .join(", ");
        Ok(format!(
            "edited {} files ({total} edits applied): {each}",
            staged.len()
        ))
    }

    async fn preview(&self, args: &Value) -> Option<FileDiff> {
        let files = plan(args).ok()?;
        let [(path, steps)] = files.as_slice() else {
            return None;
        };
        confine(path).ok()?;
        check_readable_size(path).await.ok()?;
        let original = tokio::fs::read_to_string(path).await.ok()?;
        let updated = apply_edits(&original, steps).ok()?;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn dir() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[tokio::test]
    async fn one_call_edits_many_files_atomically() {
        let d = dir();
        let (a, b) = (d.path().join("a.rs"), d.path().join("b.rs"));
        std::fs::write(&a, "fn old_name() {}\nold_name();\n").unwrap();
        std::fs::write(&b, "use old_name;\n").unwrap();
        let out = MultiEditTool
            .run(&json!({ "edits": [
                { "path": a, "old": "old_name", "new": "new_name", "replace_all": true },
                { "path": b, "old": "old_name", "new": "new_name" },
            ]}))
            .await
            .unwrap();
        assert!(out.starts_with("edited 2 files (2 edits applied)"), "{out}");
        assert_eq!(
            std::fs::read_to_string(&a).unwrap(),
            "fn new_name() {}\nnew_name();\n"
        );
        assert_eq!(std::fs::read_to_string(&b).unwrap(), "use new_name;\n");
    }

    #[tokio::test]
    async fn a_failing_edit_in_a_later_file_writes_nothing_anywhere() {
        let d = dir();
        let (a, b) = (d.path().join("a.txt"), d.path().join("b.txt"));
        std::fs::write(&a, "one\n").unwrap();
        std::fs::write(&b, "two\n").unwrap();
        let err = MultiEditTool
            .run(&json!({ "edits": [
                { "path": a, "old": "one", "new": "1" },
                { "path": b, "old": "missing", "new": "2" },
            ]}))
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("no edits applied to any file"), "{err}");
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "one\n");
        assert_eq!(std::fs::read_to_string(&b).unwrap(), "two\n");
    }

    #[tokio::test]
    async fn top_level_path_stays_the_default_and_steps_apply_in_order() {
        let d = dir();
        let a = d.path().join("a.txt");
        std::fs::write(&a, "x\n").unwrap();
        let out = MultiEditTool
            .run(&json!({ "path": a, "edits": [
                { "old": "x", "new": "y" },
                { "old": "y", "new": "z" },
            ]}))
            .await
            .unwrap();
        assert_eq!(out, format!("edited {} (2 edits applied)", a.display()));
        assert_eq!(std::fs::read_to_string(&a).unwrap(), "z\n");
    }

    #[tokio::test]
    async fn an_edit_with_no_path_anywhere_is_a_bad_call() {
        let err = MultiEditTool
            .run(&json!({ "edits": [{ "old": "x", "new": "y" }] }))
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("has no `path`"), "{err}");
    }
}
