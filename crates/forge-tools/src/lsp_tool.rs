//! The `lsp` tool: definition, hover and references from a language server (Python in v0).
//!
//! `ReadOnly`. The model names a position as path + 1-based line + the identifier as written;
//! Forge works out the UTF-16 column. A definition that lands outside the workspace (installed
//! packages, the stdlib) comes back with its body inlined, because `read_file` cannot reach it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use forge_lsp::nav::{Location, NavFailure, NavOp, NavPayload, NavRequest, NavResult};
use forge_lsp::LspRegistry;
use forge_types::SideEffect;
use serde_json::{json, Value};

use crate::{str_arg, Tool, ToolError, ToolRegistry};

/// Lines of a library definition's body that are inlined.
const INLINE_LINES: usize = 40;
const INLINE_BYTES: usize = 2560;
/// Lines shown for a definition inside the workspace; `read_file` gets the rest.
const WORKSPACE_WINDOW: usize = 3;
const MAX_DEFINITIONS: usize = 3;
const MAX_REFERENCE_ROWS: usize = 40;
const REFERENCE_LINE_CHARS: usize = 120;
const OUTPUT_CAP: usize = 6 * 1024;

type PathAllowed = Arc<dyn Fn(&Path) -> bool + Send + Sync>;

pub struct LspTool {
    lsp: Arc<LspRegistry>,
    path_allowed: PathAllowed,
}

/// Register `lsp` (deferred unless `mesh.lean_core_extra` lists it). `path_allowed` is the
/// session's read_file rule check: inlined source counts as a read.
pub fn register(
    tools: &mut ToolRegistry,
    lsp: Arc<LspRegistry>,
    path_allowed: impl Fn(&Path) -> bool + Send + Sync + 'static,
) {
    tools.register(Box::new(LspTool {
        lsp,
        path_allowed: Arc::new(path_allowed),
    }));
}

fn workspace() -> PathBuf {
    crate::SESSION_WORKSPACE
        .try_with(Clone::clone)
        .ok()
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_default()
}

fn shown(path: &Path, workspace: &Path) -> String {
    path.strip_prefix(workspace)
        .unwrap_or(path)
        .display()
        .to_string()
}

fn clip(line: &str, chars: usize) -> String {
    let line = line.trim();
    match line.char_indices().nth(chars) {
        Some((cut, _)) => format!("{}…", &line[..cut]),
        None => line.to_string(),
    }
}

fn read_lines(path: &Path) -> Option<Vec<String>> {
    let text = std::fs::read_to_string(path).ok()?;
    Some(text.lines().map(str::to_string).collect())
}

impl LspTool {
    fn definitions(&self, found: &[Location], workspace: &Path) -> String {
        if found.is_empty() {
            return "no definition found".to_string();
        }
        let mut out = String::new();
        for location in found.iter().take(MAX_DEFINITIONS) {
            let Some(path) = &location.path else {
                out.push_str(&format!("{} (not a file; nothing to read)\n", location.uri));
                continue;
            };
            let label = shown(path, workspace);
            out.push_str(&format!("{label}:{}:{}\n", location.line, location.col));
            let Some(lines) = read_lines(path) else {
                out.push_str("  (file not readable)\n");
                continue;
            };
            if path.starts_with(workspace) {
                let from = location.line as usize;
                for (offset, text) in lines
                    .iter()
                    .skip(from - 1)
                    .take(WORKSPACE_WINDOW)
                    .enumerate()
                {
                    out.push_str(&format!("  {:>5}  {text}\n", from + offset));
                }
                let end = (location.body_end as usize).min(from + 79).max(from);
                out.push_str(&format!(
                    "  read_file {label} start_line={from} end_line={end}\n"
                ));
            } else if (self.path_allowed)(path) {
                let start = location.body_start.max(1) as usize;
                let end = (location.body_end as usize).min(start + INLINE_LINES - 1);
                let mut used = 0;
                for (offset, text) in lines
                    .iter()
                    .skip(start - 1)
                    .take(end + 1 - start)
                    .enumerate()
                {
                    used += text.len() + 1;
                    if used > INLINE_BYTES {
                        out.push_str("  …\n");
                        break;
                    }
                    out.push_str(&format!("  {:>5}  {text}\n", start + offset));
                }
            } else {
                out.push_str("  (source withheld by path rules)\n");
            }
        }
        if found.len() > MAX_DEFINITIONS {
            out.push_str(&format!(
                "[{} more definitions not shown]\n",
                found.len() - MAX_DEFINITIONS
            ));
        }
        out
    }

    fn references(&self, found: &[Location], symbol: &str, workspace: &Path) -> String {
        if found.is_empty() {
            return "no references found".to_string();
        }
        let mut by_file: Vec<(PathBuf, Vec<&Location>)> = Vec::new();
        for location in found {
            let Some(path) = &location.path else { continue };
            match by_file.iter_mut().find(|(file, _)| file == path) {
                Some((_, rows)) => rows.push(location),
                None => by_file.push((path.clone(), vec![location])),
            }
        }
        let total_files = by_file.len();
        let mut out = String::new();
        let mut shown_rows = 0;
        let mut cache: HashMap<PathBuf, Option<Vec<String>>> = HashMap::new();
        'files: for (path, rows) in &by_file {
            out.push_str(&format!("{}\n", shown(path, workspace)));
            let allowed = (self.path_allowed)(path);
            let lines = cache.entry(path.clone()).or_insert_with(|| {
                if allowed {
                    read_lines(path)
                } else {
                    None
                }
            });
            for location in rows {
                if shown_rows == MAX_REFERENCE_ROWS {
                    break 'files;
                }
                shown_rows += 1;
                let source = lines
                    .as_ref()
                    .and_then(|lines| lines.get(location.line as usize - 1))
                    .map(|text| clip(text, REFERENCE_LINE_CHARS))
                    .unwrap_or_default();
                let is_def = ["def ", "class ", "async def "]
                    .iter()
                    .any(|kw| source.starts_with(&format!("{kw}{symbol}")));
                let tag = if is_def { " (def)" } else { "" };
                out.push_str(&format!(
                    "  L{}:{} {source}{tag}\n",
                    location.line, location.col
                ));
            }
        }
        if found.len() > shown_rows {
            out.push_str(&format!(
                "[{shown_rows} of {} refs in {total_files} files shown; narrow with search]\n",
                found.len()
            ));
        }
        out
    }

    fn render(&self, req: &NavRequest<'_>, result: NavResult, workspace: &Path) -> String {
        let origin = format!("{}:{}", shown(req.path, workspace), req.line);
        let mut out = format!(
            "[lsp:{}] {} of `{}` ← {origin}\n",
            result.server,
            req.op.label(),
            req.symbol
        );
        if let Some(line) = result.moved_to {
            out.push_str(&format!(
                "(symbol found on line {line}, not {})\n",
                req.line
            ));
        }
        match &result.payload {
            NavPayload::Locations(found) if req.op == NavOp::References => {
                out.push_str(&self.references(found, req.symbol, workspace));
            }
            NavPayload::Locations(found) => out.push_str(&self.definitions(found, workspace)),
            NavPayload::Hover(Some(text)) => {
                out.push_str(text);
                out.push('\n');
            }
            NavPayload::Hover(None) => out.push_str("no hover information\n"),
        }
        if let Some(note) = result.note {
            out.push_str(&format!("[{note}]\n"));
        }
        if out.len() > OUTPUT_CAP {
            let mut cut = OUTPUT_CAP;
            while !out.is_char_boundary(cut) {
                cut -= 1;
            }
            out.truncate(cut);
            out.push_str("\n[output truncated]\n");
        }
        out
    }
}

#[async_trait]
impl Tool for LspTool {
    fn name(&self) -> &str {
        "lsp"
    }

    fn description(&self) -> &str {
        "Semantic code navigation via the language server (Python). Symbol questions — where is it \
         defined, who uses it, what is its type/signature → lsp; literal text/regex → search. Give \
         path + line + symbol (the identifier exactly as written on that line); never a column. \
         definition and hover also resolve into installed libraries and the stdlib and return the \
         source inline. Read-only."
    }

    fn side_effect(&self) -> SideEffect {
        SideEffect::ReadOnly
    }

    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "op": { "type": "string", "enum": ["definition", "hover", "references"] },
                "path": { "type": "string", "description": "File the symbol is written in." },
                "line": { "type": "integer", "minimum": 1, "description": "1-based line of the symbol." },
                "symbol": { "type": "string", "description": "The identifier exactly as written on that line." },
                "occurrence": { "type": "integer", "minimum": 1, "description": "Which occurrence on the line. Default 1." }
            },
            "required": ["op", "path", "line", "symbol"]
        })
    }

    async fn run(&self, args: &Value) -> Result<String, ToolError> {
        let op = match str_arg(args, "op")? {
            "definition" => NavOp::Definition,
            "hover" => NavOp::Hover,
            "references" => NavOp::References,
            other => {
                return Err(ToolError::BadArgs(format!(
                    "unknown op '{other}' (expected definition|hover|references)"
                )))
            }
        };
        let path = str_arg(args, "path")?;
        let symbol = str_arg(args, "symbol")?;
        let line = args
            .get("line")
            .and_then(Value::as_u64)
            .filter(|line| *line >= 1)
            .ok_or_else(|| ToolError::BadArgs("expected integer 'line' >= 1".into()))?;
        let occurrence = args
            .get("occurrence")
            .and_then(Value::as_u64)
            .unwrap_or(1)
            .max(1);
        let workspace = workspace();
        let path = {
            let path = Path::new(path);
            if path.is_absolute() {
                path.to_path_buf()
            } else {
                workspace.join(path)
            }
        };
        let request = NavRequest {
            op,
            path: &path,
            workspace: &workspace,
            line: line as u32,
            symbol,
            occurrence: occurrence as usize,
        };
        match self.lsp.navigate(&request).await {
            Ok(result) => Ok(self.render(&request, result, &workspace)),
            Err(NavFailure::Unavailable(reason)) => {
                Ok(format!("[lsp:unavailable] {reason} — use search/read_file"))
            }
            Err(NavFailure::TimedOut(secs)) => Ok(format!(
                "[lsp:timeout] timed out after {secs}s (server cancelled) — use search"
            )),
            Err(NavFailure::Failed(message)) => {
                Ok(format!("[lsp:error] {message} — use search/read_file"))
            }
            Err(NavFailure::BadPosition(message)) => Err(ToolError::Failed(message)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_config::{LspConfig, LspServerEntry};

    static GATE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    fn frame(value: &Value) -> Vec<u8> {
        let body = serde_json::to_vec(value).unwrap();
        let mut out = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
        out.extend(body);
        out
    }

    fn tool(dir: &Path, answer: Value, allowed: bool) -> LspTool {
        let replay = dir.join("replay");
        let frames = [
            json!({"jsonrpc":"2.0","id":1,"result":{"capabilities":{}}}),
            json!({"jsonrpc":"2.0","id":2,"result":answer}),
        ];
        std::fs::write(&replay, frames.iter().flat_map(frame).collect::<Vec<u8>>()).unwrap();
        let mut config = LspConfig::default();
        config.servers.insert(
            "python".into(),
            LspServerEntry {
                command: "/bin/sh".into(),
                args: vec![
                    "-c".into(),
                    format!("cat {}; exec sleep 30", replay.display()),
                ],
            },
        );
        LspTool {
            lsp: Arc::new(LspRegistry::from_config_nav(&config)),
            path_allowed: Arc::new(move |_| allowed),
        }
    }

    fn link(uri: &str, start: u32, end: u32) -> Value {
        json!({
            "targetUri": uri,
            "targetRange": {"start": {"line": start, "character": 0}, "end": {"line": end, "character": 0}},
            "targetSelectionRange": {"start": {"line": start, "character": 4}, "end": {"line": start, "character": 9}}
        })
    }

    async fn call(tool: &LspTool, workspace: &Path, args: Value) -> Result<String, ToolError> {
        crate::SESSION_WORKSPACE
            .scope(workspace.to_path_buf(), tool.run(&args))
            .await
    }

    fn args(op: &str, line: u32, symbol: &str) -> Value {
        json!({"op": op, "path": "main.py", "line": line, "symbol": symbol})
    }

    #[tokio::test]
    async fn out_of_workspace_definition_inlines_its_body_unless_paths_deny_it() {
        let _gate = GATE.lock().await;
        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().join("ws");
        let lib = dir.path().join("site").join("lib.py");
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::create_dir_all(lib.parent().unwrap()).unwrap();
        std::fs::write(workspace.join("main.py"), "import lib\nlib.dumps(1)\n").unwrap();
        let body: String = (1..=60).map(|n| format!("line{n}\n")).collect();
        std::fs::write(&lib, body).unwrap();
        let uri = format!("file://{}", lib.display());
        let answer = json!([link(&uri, 4, 59)]);

        let allowed = tool(dir.path(), answer.clone(), true);
        let out = call(&allowed, &workspace, args("definition", 2, "lib.dumps"))
            .await
            .unwrap();
        assert!(
            out.starts_with("[lsp:sh] definition of `lib.dumps` ← main.py:2\n"),
            "{out}"
        );
        assert!(out.contains("line5"), "{out}");
        assert!(
            out.contains("line44") && !out.contains("line45"),
            "40 lines max: {out}"
        );
        drop(allowed);

        let denied = tool(dir.path(), answer, false);
        let out = call(&denied, &workspace, args("definition", 2, "lib.dumps"))
            .await
            .unwrap();
        assert!(
            out.contains("source withheld by path rules") && !out.contains("line5"),
            "{out}"
        );
    }

    #[tokio::test]
    async fn references_group_by_file_and_cap_rows() {
        let _gate = GATE.lock().await;
        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().to_path_buf();
        let text: String = (0..60).map(|n| format!("x{n} = f()\n")).collect();
        std::fs::write(workspace.join("main.py"), format!("def f(): ...\n{text}")).unwrap();
        let uri = format!("file://{}", workspace.join("main.py").display());
        let refs: Vec<Value> = (0..50)
            .map(|n| json!({"uri": uri, "range": {"start": {"line": n + 1, "character": 5}, "end": {"line": n + 1, "character": 6}}}))
            .collect();
        let tool = tool(dir.path(), Value::Array(refs), true);
        let out = call(&tool, &workspace, args("references", 1, "f"))
            .await
            .unwrap();
        assert!(out.contains("main.py\n  L2:6 x0 = f()"), "{out}");
        assert!(
            out.contains("[40 of 50 refs in 1 files shown; narrow with search]"),
            "{out}"
        );
    }

    #[tokio::test]
    async fn failures_are_reported_with_the_unavailable_header() {
        let _gate = GATE.lock().await;
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("main.py"), "a = 1\n").unwrap();
        let mut config = LspConfig::default();
        config.servers.insert(
            "python".into(),
            LspServerEntry {
                command: "/nonexistent/ty".into(),
                args: vec![],
            },
        );
        let missing = LspTool {
            lsp: Arc::new(LspRegistry::from_config_nav(&config)),
            path_allowed: Arc::new(|_| true),
        };
        let out = call(&missing, dir.path(), args("hover", 1, "a"))
            .await
            .unwrap();
        assert!(
            out.starts_with("[lsp:unavailable]") && out.ends_with("use search/read_file"),
            "{out}"
        );

        let err = call(&missing, dir.path(), args("hover", 1, "zzz"))
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("symbol 'zzz' not on line 1"),
            "{err}"
        );
    }

    #[test]
    fn output_is_capped() {
        let tool = LspTool {
            lsp: Arc::new(LspRegistry::from_config_nav(&LspConfig::default())),
            path_allowed: Arc::new(|_| true),
        };
        let request = NavRequest {
            op: NavOp::Hover,
            path: Path::new("/w/a.py"),
            workspace: Path::new("/w"),
            line: 1,
            symbol: "a",
            occurrence: 1,
        };
        let result = NavResult {
            server: "ty".into(),
            payload: NavPayload::Hover(Some("é".repeat(10_000))),
            note: None,
            moved_to: None,
        };
        let out = tool.render(&request, result, Path::new("/w"));
        assert!(out.len() < OUTPUT_CAP + 40 && out.ends_with("[output truncated]\n"));
    }
}
