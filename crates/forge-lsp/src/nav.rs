//! Code navigation (definition, hover, references) over the same server pool diagnostics uses.
//!
//! v0 serves Python only. ty and pyright analyse source without executing project code, so a
//! read-only tool may start them; rust-analyzer and gopls run cargo/go tooling that honours repo
//! config and need a project-trust gate before they can join [`NAV_LANGS`].

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tokio::time::timeout_at;
use tracing::{debug, warn};

use crate::position::{locate_symbol, uri_to_path};
use crate::registry::{
    arm_idle_timer, global_permits, lang_from_ext, path_to_uri, repo_root, stderr_clause, which,
    LspRegistry,
};
use crate::server::{LspServer, NavError};

/// Languages `lsp` serves. Other languages answer "unavailable" instead of spawning a server.
pub const NAV_LANGS: [&str; 1] = ["python"];
/// Everything a `definition` or `hover` call may spend: lock wait, spawn, initialize, resync, request.
const SHORT_BUDGET: Duration = Duration::from_secs(10);
/// The same for `references`, which walks the whole project.
const REFERENCES_BUDGET: Duration = Duration::from_secs(15);
/// How often an empty `references` answer is retried while the server reports indexing.
const INDEX_POLL: Duration = Duration::from_millis(250);
const HOVER_CHARS: usize = 1500;

/// Python servers tried in order when none is configured. The first one on `PATH` wins.
const PYTHON_CANDIDATES: [(&str, &[&str]); 3] = [
    ("ty", &["server"]),
    ("pyright-langserver", &["--stdio"]),
    ("basedpyright-langserver", &["--stdio"]),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavOp {
    Definition,
    Hover,
    References,
}

impl NavOp {
    pub fn label(self) -> &'static str {
        match self {
            Self::Definition => "definition",
            Self::Hover => "hover",
            Self::References => "references",
        }
    }
}

pub struct NavRequest<'a> {
    pub op: NavOp,
    /// Absolute path of the file the symbol is written in.
    pub path: &'a Path,
    pub workspace: &'a Path,
    /// 1-based line.
    pub line: u32,
    pub symbol: &'a str,
    pub occurrence: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Location {
    pub uri: String,
    /// `None` for a non-file URI (bundled typeshed); nothing can be read from it.
    pub path: Option<PathBuf>,
    /// 1-based line and column of the symbol itself.
    pub line: u32,
    pub col: u32,
    /// 1-based first and last line of the whole declaration (a definition's body).
    pub body_start: u32,
    pub body_end: u32,
}

#[derive(Debug)]
pub enum NavPayload {
    Locations(Vec<Location>),
    Hover(Option<String>),
}

#[derive(Debug)]
pub struct NavResult {
    /// Short server name for the output header (`ty`).
    pub server: String,
    pub payload: NavPayload,
    pub note: Option<String>,
    /// Set when the symbol was found on a different line than the one asked for.
    pub moved_to: Option<u32>,
}

#[derive(Debug)]
pub enum NavFailure {
    Unavailable(String),
    /// The symbol is not where the model said it is.
    BadPosition(String),
    TimedOut(u64),
    Failed(String),
}

/// The directory a nav server is rooted at: the project marker root when it lies inside the
/// session workspace, else the workspace itself (benchmark task dirs have no markers at all).
pub(crate) fn nav_root(path: &Path, workspace: &Path) -> PathBuf {
    repo_root(path)
        .filter(|root| root.starts_with(workspace))
        .unwrap_or_else(|| workspace.to_path_buf())
}

impl LspRegistry {
    fn nav_server(&self, lang: &str) -> Result<(String, Vec<String>), String> {
        if let Some(entry) = self.config.servers.get(lang) {
            return if which(&entry.command).is_some() {
                Ok((entry.command.clone(), entry.args.clone()))
            } else {
                Err(format!(
                    "language server `{}` is not on PATH",
                    entry.command
                ))
            };
        }
        if self.nav && lang == "python" {
            for (command, args) in PYTHON_CANDIDATES {
                if which(command).is_some() {
                    return Ok((
                        command.to_string(),
                        args.iter().map(|arg| arg.to_string()).collect(),
                    ));
                }
            }
        }
        Err("no Python language server found (looked for ty, pyright-langserver, basedpyright-langserver)".into())
    }

    /// The Python server `lsp` would use, for `forge doctor`.
    pub fn nav_candidate(&self) -> Option<String> {
        self.nav_server("python").ok().map(|(command, _)| command)
    }

    /// Answer one navigation request. Bounded end to end by the op's budget.
    pub async fn navigate(&self, req: &NavRequest<'_>) -> Result<NavResult, NavFailure> {
        let started = Instant::now();
        let budget = if req.op == NavOp::References {
            REFERENCES_BUDGET
        } else {
            SHORT_BUDGET
        };
        let deadline = tokio::time::Instant::now() + budget;
        let outcome = self.navigate_inner(req, deadline, budget).await;
        debug!(
            op = req.op.label(),
            ms = started.elapsed().as_millis() as u64,
            ok = outcome.is_ok(),
            "lsp nav"
        );
        outcome
    }

    async fn navigate_inner(
        &self,
        req: &NavRequest<'_>,
        deadline: tokio::time::Instant,
        budget: Duration,
    ) -> Result<NavResult, NavFailure> {
        let lang = lang_from_ext(req.path)
            .filter(|lang| NAV_LANGS.contains(lang))
            .ok_or_else(|| NavFailure::Unavailable("only Python files are supported".into()))?;
        let text = std::fs::read_to_string(req.path).map_err(|error| {
            NavFailure::Failed(format!("cannot read {}: {error}", req.path.display()))
        })?;
        let located =
            locate_symbol(&text, req.line, req.symbol, req.occurrence).map_err(|error| {
                NavFailure::BadPosition(format!(
                    "symbol '{}' not on line {} {error}",
                    req.symbol, req.line
                ))
            })?;

        let (cmd, args) = self.nav_server(lang).map_err(NavFailure::Unavailable)?;
        let server_name = Path::new(&cmd)
            .file_stem()
            .map_or_else(|| cmd.clone(), |stem| stem.to_string_lossy().into_owned());
        let root = nav_root(req.path, req.workspace);
        let ttl = self.idle_ttl();
        let Some(entry) = self.slot_for((lang.to_string(), root.clone()), ttl).await else {
            return Err(NavFailure::Unavailable(
                "all language-server slots are busy".into(),
            ));
        };
        let mut slot = timeout_at(deadline, entry.clone().lock_owned())
            .await
            .map_err(|_| {
                NavFailure::Unavailable("language server is busy with another request".into())
            })?;
        let generation = slot.touch();
        arm_idle_timer(&mut slot, entry.clone(), generation, ttl);

        if slot.server.is_none() {
            let now = Instant::now();
            if let Some(reason) = slot.abandoned.clone() {
                return Err(NavFailure::Unavailable(reason));
            }
            if slot.cooling_down(now) {
                return Err(NavFailure::Unavailable(format!(
                    "{lang} language server is recovering from a previous failure"
                )));
            }
            let Ok(permit) = global_permits().clone().try_acquire_owned() else {
                return Err(NavFailure::Unavailable(
                    "the language-server capacity is full".into(),
                ));
            };
            slot.permit = Some(permit);
            let mut server = match LspServer::spawn_with_memory_limit(
                &cmd,
                &args,
                self.memory_limit_bytes(),
                false,
            )
            .await
            {
                Ok(server) => server,
                Err(error) => {
                    slot.record_failure(now);
                    return Err(NavFailure::Unavailable(format!(
                        "could not start `{cmd}`: {error}"
                    )));
                }
            };
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if let Err(error) = server.initialize_nav(&path_to_uri(&root), remaining).await {
                let cause = server.stderr_summary().await;
                slot.record_failure(now);
                warn!(
                    "lsp: nav initialize failed for {cmd}: {error}{}",
                    stderr_clause(&cause)
                );
                return Err(NavFailure::Unavailable(format!(
                    "{lang} language server initialization failed: {error}"
                )));
            }
            slot.server = Some(server);
        }

        let uri = path_to_uri(req.path);
        let result = {
            let server = slot.server.as_mut().expect("server set above");
            let sync = async {
                server.resync_from_disk().await?;
                server.sync_if_changed(&uri, lang, &text).await
            };
            // A server that stopped reading stdin blocks these writes forever; the deadline covers
            // them and a stuck server is dropped (killed) rather than left holding the slot.
            let synced = match timeout_at(deadline, sync).await {
                Ok(done) => done,
                Err(_) => {
                    slot.record_failure(Instant::now());
                    return Err(NavFailure::Unavailable(
                        "language server stopped accepting documents (killed)".into(),
                    ));
                }
            };
            match synced {
                Ok(()) => request_op(server, req.op, &uri, &located, deadline).await,
                Err(error) => Err(NavError::Transport(error.to_string())),
            }
        };
        let mut note = None;
        let value = match result {
            Ok(value) => value,
            Err(NavError::TimedOut) => return Err(NavFailure::TimedOut(budget.as_secs())),
            Err(NavError::Server(message)) => return Err(NavFailure::Failed(message)),
            Err(NavError::Transport(message)) => {
                slot.record_failure(Instant::now());
                return Err(NavFailure::Unavailable(format!(
                    "{lang} language server failed: {message}"
                )));
            }
        };
        let mut value = value;
        if req.op == NavOp::References && parse_locations(&value).is_empty() {
            let server = slot.server.as_mut().expect("server set above");
            while server.is_indexing() && tokio::time::Instant::now() < deadline {
                tokio::time::sleep(INDEX_POLL).await;
                match request_op(server, req.op, &uri, &located, deadline).await {
                    Ok(next) => value = next,
                    Err(_) => break,
                }
                if !parse_locations(&value).is_empty() {
                    break;
                }
            }
            if parse_locations(&value).is_empty() && server.is_indexing() {
                note = Some("server still indexing; results may be incomplete".to_string());
            }
        }
        let payload = match req.op {
            NavOp::Hover => NavPayload::Hover(parse_hover(&value)),
            _ => NavPayload::Locations(parse_locations(&value)),
        };
        // An empty answer proves nothing about the server's health (see ServerEntry::clear_failure).
        if matches!(&payload, NavPayload::Locations(found) if !found.is_empty())
            || matches!(&payload, NavPayload::Hover(Some(_)))
        {
            slot.clear_failure();
        }
        let generation = slot.touch();
        arm_idle_timer(&mut slot, entry.clone(), generation, ttl);
        Ok(NavResult {
            server: server_name,
            payload,
            note,
            moved_to: located.moved_to,
        })
    }
}

async fn request_op(
    server: &mut LspServer,
    op: NavOp,
    uri: &str,
    located: &crate::position::Located,
    deadline: tokio::time::Instant,
) -> Result<Value, NavError> {
    let mut params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": located.line0, "character": located.col16 }
    });
    let method = match op {
        NavOp::Definition => "textDocument/definition",
        NavOp::Hover => "textDocument/hover",
        NavOp::References => {
            params["context"] = json!({ "includeDeclaration": true });
            "textDocument/references"
        }
    };
    server.request(method, params, deadline).await
}

fn position_line(range: &Value, edge: &str) -> Option<u32> {
    Some(range.get(edge)?.get("line")?.as_u64()? as u32 + 1)
}

fn parse_one(value: &Value) -> Option<Location> {
    let (uri, selection, whole) = match value.get("targetUri") {
        Some(uri) => {
            let whole = value.get("targetRange")?;
            (
                uri.as_str()?,
                value.get("targetSelectionRange").unwrap_or(whole),
                whole,
            )
        }
        None => {
            let range = value.get("range")?;
            (value.get("uri")?.as_str()?, range, range)
        }
    };
    let line = position_line(selection, "start")?;
    let col = selection.get("start")?.get("character")?.as_u64()? as u32 + 1;
    Some(Location {
        uri: uri.to_string(),
        path: uri_to_path(uri),
        line,
        col,
        body_start: position_line(whole, "start")?,
        body_end: position_line(whole, "end")?,
    })
}

/// `Location | Location[] | LocationLink[] | null`.
pub(crate) fn parse_locations(value: &Value) -> Vec<Location> {
    match value {
        Value::Array(items) => items.iter().filter_map(parse_one).collect(),
        Value::Object(_) => parse_one(value).into_iter().collect(),
        _ => Vec::new(),
    }
}

fn marked_string(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other
            .get("value")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
    }
}

/// Hover text as plain lines: markdown fences dropped, capped at [`HOVER_CHARS`].
pub(crate) fn parse_hover(value: &Value) -> Option<String> {
    let contents = value.get("contents")?;
    let raw = match contents {
        Value::Array(items) => items
            .iter()
            .map(marked_string)
            .collect::<Vec<_>>()
            .join("\n"),
        other => marked_string(other),
    };
    let text = raw
        .lines()
        .filter(|line| !line.trim_start().starts_with("```"))
        .collect::<Vec<_>>()
        .join("\n");
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    Some(match text.char_indices().nth(HOVER_CHARS) {
        Some((cut, _)) => format!("{}…", &text[..cut]),
        None => text.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_location_link_location_and_arrays() {
        let link = json!([{
            "targetUri": "file:///site/pkg/mod.py",
            "targetRange": {"start": {"line": 9, "character": 0}, "end": {"line": 20, "character": 4}},
            "targetSelectionRange": {"start": {"line": 10, "character": 4}, "end": {"line": 10, "character": 9}}
        }]);
        let found = parse_locations(&link);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].path, Some(PathBuf::from("/site/pkg/mod.py")));
        assert_eq!((found[0].line, found[0].col), (11, 5));
        assert_eq!((found[0].body_start, found[0].body_end), (10, 21));

        let plain = json!({"uri": "file:///a.py", "range": {"start": {"line": 0, "character": 2}, "end": {"line": 0, "character": 5}}});
        assert_eq!(parse_locations(&plain)[0].line, 1);
        assert_eq!(parse_locations(&json!([plain.clone(), plain])).len(), 2);
        assert!(parse_locations(&Value::Null).is_empty());
    }

    #[test]
    fn non_file_uri_has_no_path() {
        let value = json!([{"uri": "ty-builtins:///b.pyi", "range": {"start": {"line": 1, "character": 0}, "end": {"line": 1, "character": 1}}}]);
        assert_eq!(parse_locations(&value)[0].path, None);
    }

    #[test]
    fn hover_accepts_every_contents_shape_and_strips_fences() {
        let markup = json!({"contents": {"kind": "markdown", "value": "```python\ndef f(x) -> int\n```\ndocs"}});
        assert_eq!(
            parse_hover(&markup).as_deref(),
            Some("def f(x) -> int\ndocs")
        );
        assert_eq!(
            parse_hover(&json!({"contents": "plain"})).as_deref(),
            Some("plain")
        );
        let marked = json!({"contents": [{"language": "python", "value": "a: int"}, "more"]});
        assert_eq!(parse_hover(&marked).as_deref(), Some("a: int\nmore"));
        assert_eq!(parse_hover(&Value::Null), None);
        let long = json!({"contents": {"kind": "plaintext", "value": "x".repeat(4000)}});
        assert_eq!(parse_hover(&long).unwrap().chars().count(), HOVER_CHARS + 1);
    }

    #[test]
    fn nav_root_falls_back_to_the_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().canonicalize().unwrap();
        let file = workspace.join("app").join("main.py");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "").unwrap();
        // The tempdir sits under /tmp, which may itself hold a marker: stay inside the workspace.
        assert!(nav_root(&file, &workspace).starts_with(&workspace));
        std::fs::write(workspace.join("app").join("pyproject.toml"), "").unwrap();
        assert_eq!(nav_root(&file, &workspace), workspace.join("app"));
        // A marker root outside the workspace is replaced by the workspace.
        assert_eq!(
            nav_root(&file, &workspace.join("app").join("sub")),
            workspace.join("app").join("sub")
        );
    }

    use forge_config::{LspConfig, LspServerEntry};
    use serial_test::serial;

    fn frame(value: &Value) -> Vec<u8> {
        let body = serde_json::to_vec(value).unwrap();
        let mut out = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
        out.extend(body);
        out
    }

    /// A registry whose Python server replays `answers` (response to initialize first).
    fn fake_registry(dir: &Path, answers: &[Value]) -> LspRegistry {
        let replay = dir.join("replay");
        std::fs::write(&replay, answers.iter().flat_map(frame).collect::<Vec<u8>>()).unwrap();
        let script = format!("cat {}; exec sleep 30", replay.display());
        let mut config = LspConfig::default();
        config.servers.insert(
            "python".into(),
            LspServerEntry {
                command: "/bin/sh".into(),
                args: vec!["-c".into(), script],
            },
        );
        LspRegistry::from_config_nav(&config)
    }

    #[tokio::test]
    #[serial]
    async fn definition_roots_at_a_marker_less_workspace_and_parses_the_answer() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().canonicalize().unwrap();
        let file = workspace.join("main.py");
        std::fs::write(&file, "import json\njson.dumps({})\n").unwrap();
        let answer = json!([{
            "targetUri": "file:///usr/lib/python3/json/__init__.py",
            "targetRange": {"start": {"line": 182, "character": 0}, "end": {"line": 237, "character": 25}},
            "targetSelectionRange": {"start": {"line": 182, "character": 4}, "end": {"line": 182, "character": 9}}
        }]);
        let registry = fake_registry(
            dir.path(),
            &[
                json!({"jsonrpc":"2.0","id":1,"result":{"capabilities":{}}}),
                json!({"jsonrpc":"2.0","id":2,"result":answer}),
            ],
        );
        let result = registry
            .navigate(&NavRequest {
                op: NavOp::Definition,
                path: &file,
                workspace: &workspace,
                line: 2,
                symbol: "json.dumps",
                occurrence: 1,
            })
            .await
            .unwrap();
        assert_eq!(result.server, "sh");
        let NavPayload::Locations(found) = result.payload else {
            panic!("expected locations")
        };
        assert_eq!((found[0].line, found[0].body_end), (183, 238));
    }

    #[tokio::test]
    #[serial]
    async fn unsupported_files_and_missing_servers_are_unavailable() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().canonicalize().unwrap();
        let rust = workspace.join("lib.rs");
        std::fs::write(&rust, "fn a() {}\n").unwrap();
        let registry = LspRegistry::from_config_nav(&LspConfig::default());
        let err = registry
            .navigate(&NavRequest {
                op: NavOp::Hover,
                path: &rust,
                workspace: &workspace,
                line: 1,
                symbol: "a",
                occurrence: 1,
            })
            .await
            .unwrap_err();
        assert!(
            matches!(&err, NavFailure::Unavailable(m) if m.contains("Python")),
            "{err:?}"
        );

        let mut config = LspConfig::default();
        config.servers.insert(
            "python".into(),
            LspServerEntry {
                command: "/nonexistent/ty".into(),
                args: vec![],
            },
        );
        let py = workspace.join("a.py");
        std::fs::write(&py, "a = 1\n").unwrap();
        let err = LspRegistry::from_config_nav(&config)
            .navigate(&NavRequest {
                op: NavOp::Hover,
                path: &py,
                workspace: &workspace,
                line: 1,
                symbol: "a",
                occurrence: 1,
            })
            .await
            .unwrap_err();
        assert!(
            matches!(&err, NavFailure::Unavailable(m) if m.contains("not on PATH")),
            "{err:?}"
        );
    }

    #[tokio::test]
    #[serial]
    async fn a_server_that_stops_reading_is_dropped_at_the_deadline() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().canonicalize().unwrap();
        let file = workspace.join("big.py");
        std::fs::write(&file, format!("x = 1\n{}", "#".repeat(4 * 1024 * 1024))).unwrap();
        // Answers initialize, then never reads stdin again: the document write blocks.
        let registry = fake_registry(
            dir.path(),
            &[json!({"jsonrpc":"2.0","id":1,"result":{"capabilities":{}}})],
        );
        let started = Instant::now();
        let err = registry
            .navigate(&NavRequest {
                op: NavOp::Definition,
                path: &file,
                workspace: &workspace,
                line: 1,
                symbol: "x",
                occurrence: 1,
            })
            .await
            .unwrap_err();
        assert!(
            matches!(&err, NavFailure::Unavailable(m) if m.contains("killed")),
            "{err:?}"
        );
        assert!(
            started.elapsed() < Duration::from_secs(14),
            "{:?}",
            started.elapsed()
        );
        // The slot lock was released and the server dropped: the next call is not stuck behind it.
        let again = registry
            .navigate(&NavRequest {
                op: NavOp::Definition,
                path: &file,
                workspace: &workspace,
                line: 1,
                symbol: "x",
                occurrence: 1,
            })
            .await
            .unwrap_err();
        assert!(
            matches!(&again, NavFailure::Unavailable(m) if m.contains("recovering")),
            "{again:?}"
        );
    }

    #[test]
    fn candidate_fallback_applies_only_to_nav_registries() {
        let registry = LspRegistry::from_config(&LspConfig::default());
        assert!(registry.nav_server("python").is_err());
    }

    /// Needs a real `ty` (FORGE_TEST_TY or on PATH). Run with `--ignored`.
    #[tokio::test]
    #[serial]
    #[ignore = "needs ty"]
    async fn live_ty_smoke_on_a_marker_less_fixture() {
        let ty = std::env::var("FORGE_TEST_TY")
            .ok()
            .or_else(|| which("ty").map(|p| p.display().to_string()))
            .expect("ty on PATH or FORGE_TEST_TY");
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().canonicalize().unwrap();
        std::fs::write(ws.join("util.py"), "def helper(x):\n    return x\n").unwrap();
        std::fs::write(
            ws.join("a.py"),
            "import json\nfrom util import helper\njson.dumps(helper(1))\n",
        )
        .unwrap();
        std::fs::write(ws.join("b.py"), "from util import helper\nhelper(2)\n").unwrap();
        let mut config = LspConfig::default();
        config.servers.insert(
            "python".into(),
            LspServerEntry {
                command: ty,
                args: vec!["server".into()],
            },
        );
        let registry = LspRegistry::from_config_nav(&config);
        let ask = |op, file: &str, line, symbol: &'static str| {
            let path = ws.join(file);
            let registry = &registry;
            let ws = ws.clone();
            async move {
                registry
                    .navigate(&NavRequest {
                        op,
                        path: &path,
                        workspace: &ws,
                        line,
                        symbol,
                        occurrence: 1,
                    })
                    .await
            }
        };
        let locations = |result: NavResult| match result.payload {
            NavPayload::Locations(found) => found,
            other => panic!("expected locations, got {other:?}"),
        };

        let stdlib = locations(
            ask(NavOp::Definition, "a.py", 3, "json.dumps")
                .await
                .unwrap(),
        );
        assert!(
            stdlib[0]
                .path
                .as_ref()
                .unwrap()
                .ends_with("json/__init__.py"),
            "{stdlib:?}"
        );

        let local = locations(ask(NavOp::Definition, "a.py", 2, "helper").await.unwrap());
        assert!(
            local[0].path.as_ref().unwrap().ends_with("util.py"),
            "{local:?}"
        );

        let refs = locations(
            ask(NavOp::References, "util.py", 1, "helper")
                .await
                .unwrap(),
        );
        let files: std::collections::HashSet<_> =
            refs.iter().filter_map(|l| l.path.clone()).collect();
        assert!(files.len() >= 2, "{refs:?}");

        std::fs::write(ws.join("util.py"), "\n\n\ndef helper(x):\n    return x\n").unwrap();
        let moved = locations(ask(NavOp::Definition, "a.py", 2, "helper").await.unwrap());
        assert_eq!(
            moved[0].line, 4,
            "definition must follow an out-of-band edit: {moved:?}"
        );
    }
}
