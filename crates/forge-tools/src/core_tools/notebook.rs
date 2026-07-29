//! Cell-level editing of Jupyter notebooks.
//!
//! An `.ipynb` is JSON, so the generic text tools would happily corrupt it: a plain-text edit
//! rewrites the structure the format depends on. This module is the safe path — it addresses
//! cells by index, keeps nbformat's line-list `source` shape, and drops a replaced code cell's
//! stale outputs and execution count, which no longer describe the code that is there.

use async_trait::async_trait;
use forge_types::{DiffKind, FileDiff, SideEffect};
use serde_json::{json, Value};

use super::{check_readable_size, confine};
use crate::{str_arg, Tool, ToolError};

/// Edit a Jupyter notebook (`.ipynb`) at the cell level: replace a cell's source, insert a new
/// cell, or delete one. Mutates the workspace.
pub struct NotebookEditTool;

#[derive(Clone, Copy, PartialEq)]
enum NotebookMode {
    Replace,
    Insert,
    Delete,
}

/// nbformat stores a cell's `source` as a list of lines, each retaining its trailing `\n`.
fn notebook_source_array(source: &str) -> Value {
    Value::Array(
        source
            .split_inclusive('\n')
            .map(|l| Value::String(l.to_string()))
            .collect(),
    )
}

/// Apply one cell-level edit to a notebook's JSON text. Pure (no disk) so it can be unit-tested;
/// returns the rewritten notebook JSON or a human-readable error. Editing a code cell's source
/// clears its stale `outputs`/`execution_count` (they no longer correspond to the new code).
fn apply_notebook_edit(
    content: &str,
    cell: usize,
    source: Option<&str>,
    cell_type: &str,
    mode: NotebookMode,
) -> Result<String, String> {
    let mut nb: Value =
        serde_json::from_str(content).map_err(|e| format!("not valid JSON: {e}"))?;
    let cells = nb
        .get_mut("cells")
        .and_then(Value::as_array_mut)
        .ok_or("not a Jupyter notebook (no `cells` array)")?;
    let n = cells.len();
    match mode {
        NotebookMode::Delete => {
            if cell >= n {
                return Err(format!("cell {cell} out of range (notebook has {n} cells)"));
            }
            cells.remove(cell);
        }
        NotebookMode::Insert => {
            let source = source.ok_or("`source` is required to insert a cell")?;
            if cell > n {
                return Err(format!(
                    "insert index {cell} out of range (notebook has {n} cells)"
                ));
            }
            let new_cell = if cell_type == "markdown" {
                json!({ "cell_type": "markdown", "metadata": {}, "source": notebook_source_array(source) })
            } else {
                json!({ "cell_type": "code", "metadata": {}, "execution_count": null, "outputs": [], "source": notebook_source_array(source) })
            };
            cells.insert(cell, new_cell);
        }
        NotebookMode::Replace => {
            let source = source.ok_or("`source` is required to replace a cell")?;
            if cell >= n {
                return Err(format!("cell {cell} out of range (notebook has {n} cells)"));
            }
            let c = &mut cells[cell];
            c["source"] = notebook_source_array(source);
            if c.get("cell_type").and_then(Value::as_str) == Some("code") {
                if let Some(obj) = c.as_object_mut() {
                    obj.insert("outputs".into(), Value::Array(Vec::new()));
                    obj.insert("execution_count".into(), Value::Null);
                }
            }
        }
    }
    let mut out = serde_json::to_string_pretty(&nb).map_err(|e| e.to_string())?;
    out.push('\n'); // .ipynb files conventionally end with a trailing newline
    Ok(out)
}

#[async_trait]
impl Tool for NotebookEditTool {
    fn name(&self) -> &str {
        "notebook_edit"
    }
    fn description(&self) -> &str {
        "Edit a Jupyter notebook (.ipynb) by cell index (0-based). mode=replace (default) swaps a \
         cell's source; mode=insert adds a new cell at the index; mode=delete removes it. \
         `source` is the full new cell text (required for replace/insert). For insert, `cell_type` \
         is \"code\" (default) or \"markdown\". Replacing a code cell clears its stale outputs and \
         execution count. Use this for .ipynb files — edit_file would corrupt the JSON structure."
    }
    fn side_effect(&self) -> SideEffect {
        SideEffect::Write
    }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Notebook (.ipynb) file." },
                "cell": { "type": "integer", "description": "0-based cell index.", "minimum": 0 },
                "mode": {
                    "type": "string",
                    "enum": ["replace", "insert", "delete"],
                    "description": "replace (default), insert, or delete."
                },
                "source": {
                    "type": "string",
                    "description": "New cell source (required for replace/insert)."
                },
                "cell_type": {
                    "type": "string",
                    "enum": ["code", "markdown"],
                    "description": "Cell type for insert (default code)."
                }
            },
            "required": ["path", "cell"]
        })
    }
    async fn run(&self, args: &Value) -> Result<String, ToolError> {
        let path = str_arg(args, "path")?;
        confine(path)?;
        let cell = args
            .get("cell")
            .and_then(Value::as_u64)
            .ok_or_else(|| ToolError::BadArgs("expected integer 'cell'".into()))?
            as usize;
        let mode = match args
            .get("mode")
            .and_then(Value::as_str)
            .unwrap_or("replace")
        {
            "replace" => NotebookMode::Replace,
            "insert" => NotebookMode::Insert,
            "delete" => NotebookMode::Delete,
            other => return Err(ToolError::BadArgs(format!("unknown mode '{other}'"))),
        };
        let source = args.get("source").and_then(Value::as_str);
        let cell_type = args
            .get("cell_type")
            .and_then(Value::as_str)
            .unwrap_or("code");

        check_readable_size(path).await?;
        let content = tokio::fs::read_to_string(path).await?;
        let updated = apply_notebook_edit(&content, cell, source, cell_type, mode)
            .map_err(|e| ToolError::Failed(format!("{e} (in {path})")))?;
        tokio::fs::write(path, &updated).await?;
        let verb = match mode {
            NotebookMode::Replace => "replaced",
            NotebookMode::Insert => "inserted",
            NotebookMode::Delete => "deleted",
        };
        Ok(format!("{verb} cell {cell} in {path}"))
    }

    async fn preview(&self, args: &Value) -> Option<FileDiff> {
        let path = str_arg(args, "path").ok()?;
        confine(path).ok()?;
        let cell = args.get("cell").and_then(Value::as_u64)? as usize;
        let mode = match args
            .get("mode")
            .and_then(Value::as_str)
            .unwrap_or("replace")
        {
            "insert" => NotebookMode::Insert,
            "delete" => NotebookMode::Delete,
            _ => NotebookMode::Replace,
        };
        let source = args.get("source").and_then(Value::as_str);
        let cell_type = args
            .get("cell_type")
            .and_then(Value::as_str)
            .unwrap_or("code");
        check_readable_size(path).await.ok()?;
        let content = tokio::fs::read_to_string(path).await.ok()?;
        let updated = apply_notebook_edit(&content, cell, source, cell_type, mode).ok()?;
        Some(FileDiff {
            path: path.to_string(),
            kind: DiffKind::Modified,
            old: Some(content),
            new: Some(updated),
            lang: Some("json".to_string()),
            binary: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NB: &str = r##"{"cells":[{"cell_type":"code","execution_count":3,"metadata":{},"outputs":[{"x":1}],"source":["print(1)\n"]},{"cell_type":"markdown","metadata":{},"source":["# title\n"]}],"metadata":{},"nbformat":4,"nbformat_minor":5}"##;

    #[test]
    fn notebook_replace_rewrites_source_and_clears_code_outputs() {
        let out = apply_notebook_edit(NB, 0, Some("print(2)\n"), "code", NotebookMode::Replace)
            .expect("replace");
        let nb: Value = serde_json::from_str(&out).unwrap();
        let c0 = &nb["cells"][0];
        assert_eq!(c0["source"], json!(["print(2)\n"]));
        // Stale execution artifacts are cleared on a source edit.
        assert_eq!(c0["outputs"], json!([]));
        assert_eq!(c0["execution_count"], Value::Null);
        // The other cell is untouched.
        assert_eq!(nb["cells"][1]["source"], json!(["# title\n"]));
    }

    #[test]
    fn notebook_insert_and_delete_change_cell_count() {
        let inserted =
            apply_notebook_edit(NB, 1, Some("import os\n"), "code", NotebookMode::Insert).unwrap();
        let nb: Value = serde_json::from_str(&inserted).unwrap();
        assert_eq!(nb["cells"].as_array().unwrap().len(), 3);
        assert_eq!(nb["cells"][1]["source"], json!(["import os\n"]));

        let deleted = apply_notebook_edit(NB, 0, None, "code", NotebookMode::Delete).unwrap();
        let nb: Value = serde_json::from_str(&deleted).unwrap();
        assert_eq!(nb["cells"].as_array().unwrap().len(), 1);
        assert_eq!(nb["cells"][0]["cell_type"], "markdown");
    }

    #[test]
    fn notebook_edit_rejects_bad_index_and_non_notebook() {
        assert!(apply_notebook_edit(NB, 9, Some("x"), "code", NotebookMode::Replace).is_err());
        assert!(
            apply_notebook_edit("{\"foo\":1}", 0, Some("x"), "code", NotebookMode::Replace)
                .is_err()
        );
        assert!(apply_notebook_edit("not json", 0, None, "code", NotebookMode::Delete).is_err());
    }
}
