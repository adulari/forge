use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};
use sysinfo::{Pid, ProcessesToUpdate, System};
use tokio::io::{AsyncReadExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout};
use tracing::{debug, warn};

use crate::rpc::{read_msg, write_msg};
use crate::types::{Diagnostic, DiagnosticSeverity};

/// How much of the server's stderr is retained (the tail — the last thing it said before dying).
const STDERR_TAIL_BYTES: usize = 2048;
/// How long the reader task is given to finish draining stderr once a handshake has failed.
const STDERR_DRAIN_GRACE: Duration = Duration::from_millis(250);
/// Upper bound on the single-line cause attached to a failure message.
const STDERR_SUMMARY_CHARS: usize = 300;
/// Poll slowly enough to stay negligible while still stopping sustained allocation bursts.
const RESOURCE_SAMPLE_INTERVAL: Duration = Duration::from_millis(250);

pub struct LspServer {
    _child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
    documents: HashMap<String, i32>,
    /// Hash of the text last sent for each open document, so nav can tell a changed file from an
    /// unchanged one without resending it.
    doc_hashes: HashMap<String, u64>,
    /// Server capabilities from the initialize result (nav servers only read it).
    pub(crate) capabilities: Value,
    /// Open `$/progress` tokens; a non-empty set means the server is still indexing.
    progress_active: HashSet<String>,
    /// Bounded tail of the server's stderr. A server that dies during the handshake reports the
    /// real cause here (a missing rustup component, a bad config) while stdout only shows EOF.
    stderr: Arc<Mutex<String>>,
    stderr_reader: Option<tokio::task::JoinHandle<()>>,
    memory_guard: Option<tokio::task::JoinHandle<()>>,
}

impl Drop for LspServer {
    fn drop(&mut self) {
        if let Some(reader) = self.stderr_reader.take() {
            reader.abort();
        }
        if let Some(guard) = self.memory_guard.take() {
            guard.abort();
        }
    }
}

impl LspServer {
    pub async fn spawn(cmd: &str, args: &[String]) -> std::io::Result<Self> {
        Self::spawn_with_memory_limit(cmd, args, None, false).await
    }

    pub(crate) async fn spawn_with_memory_limit(
        cmd: &str,
        args: &[String],
        memory_limit_bytes: Option<u64>,
        rust_resource_profile: bool,
    ) -> std::io::Result<Self> {
        let mut command = tokio::process::Command::new(cmd);
        command.args(args);
        if rust_resource_profile {
            command
                .env("CARGO_BUILD_JOBS", "1")
                .env("RAYON_NUM_THREADS", "1");
        }
        let mut child = command
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;
        let stdin = child.stdin.take().expect("stdin piped");
        let stdout = BufReader::new(child.stdout.take().expect("stdout piped"));
        let stderr = Arc::new(Mutex::new(String::new()));
        let memory_guard = child.id().and_then(|pid| {
            memory_limit_bytes.map(|limit| {
                let stderr = stderr.clone();
                tokio::spawn(async move {
                    match monitor_process_tree_memory(pid, limit, stderr).await {
                        Ok(()) => debug!("lsp: memory guard exited for process {pid}"),
                        Err(error) => {
                            warn!("lsp: memory guard failed for process {pid}: {error}")
                        }
                    }
                })
            })
        });
        let stderr_reader = child.stderr.take().map(|mut pipe| {
            let sink = stderr.clone();
            tokio::spawn(async move {
                let mut buf = [0u8; 1024];
                loop {
                    match pipe.read(&mut buf).await {
                        Ok(0) => break,
                        Err(error) => {
                            if let Ok(mut sink) = sink.lock() {
                                append_stderr_read_error(&mut sink, &error);
                            }
                            break;
                        }
                        Ok(n) => {
                            let chunk = String::from_utf8_lossy(&buf[..n]).into_owned();
                            if let Ok(mut sink) = sink.lock() {
                                append_bounded(&mut sink, &chunk, STDERR_TAIL_BYTES);
                            }
                        }
                    }
                }
            })
        });
        Ok(Self {
            _child: child,
            stdin,
            stdout,
            next_id: 1,
            documents: HashMap::new(),
            doc_hashes: HashMap::new(),
            capabilities: Value::Null,
            progress_active: HashSet::new(),
            stderr,
            stderr_reader,
            memory_guard,
        })
    }

    /// A short, single-line tail of what the server printed on stderr, or `""` if it said nothing.
    /// Called on the failure path only: it waits briefly for the reader task to drain a stderr pipe
    /// the dying process has already closed, so the cause isn't lost to a race with its exit.
    pub async fn stderr_summary(&mut self) -> String {
        if let Some(mut handle) = self.stderr_reader.take() {
            if tokio::time::timeout(STDERR_DRAIN_GRACE, &mut handle)
                .await
                .is_err()
            {
                handle.abort();
                let _ = handle.await;
            }
        }
        let raw = self
            .stderr
            .lock()
            .map(|tail| tail.clone())
            .unwrap_or_default();
        summarize_stderr(&raw, STDERR_SUMMARY_CHARS)
    }

    fn new_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    pub async fn initialize(&mut self, root_uri: &str, timeout: Duration) -> std::io::Result<()> {
        self.initialize_with_options(root_uri, timeout, None).await
    }

    pub(crate) async fn initialize_with_options(
        &mut self,
        root_uri: &str,
        timeout: Duration,
        initialization_options: Option<Value>,
    ) -> std::io::Result<()> {
        self.initialize_inner(root_uri, timeout, initialization_options, false)
            .await
    }

    /// Initialize declaring the navigation capabilities (definition, references, hover).
    pub(crate) async fn initialize_nav(
        &mut self,
        root_uri: &str,
        timeout: Duration,
    ) -> std::io::Result<()> {
        self.initialize_inner(root_uri, timeout, None, true).await
    }

    async fn initialize_inner(
        &mut self,
        root_uri: &str,
        timeout: Duration,
        initialization_options: Option<Value>,
        nav: bool,
    ) -> std::io::Result<()> {
        let id = self.new_id();
        let req = initialize_request(id, root_uri, initialization_options, nav);
        write_msg(&mut self.stdin, &req).await?;
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "lsp: initialize response timed out",
                ));
            }
            match tokio::time::timeout(remaining, read_msg(&mut self.stdout)).await {
                Ok(Ok(Some(msg))) => {
                    if msg.get("id").and_then(|v| v.as_u64()) == Some(id) {
                        if let Some(error) = msg.get("error") {
                            let message = error
                                .get("message")
                                .and_then(Value::as_str)
                                .unwrap_or("unknown initialization error");
                            return Err(std::io::Error::other(format!(
                                "lsp: initialize rejected: {}",
                                sanitize_log_text(message)
                            )));
                        }
                        self.capabilities = msg
                            .pointer("/result/capabilities")
                            .cloned()
                            .unwrap_or(Value::Null);
                        break;
                    }
                }
                Ok(Ok(None)) => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        "lsp: server closed stdout during initialize",
                    ))
                }
                Ok(Err(error)) => {
                    return Err(std::io::Error::new(
                        error.kind(),
                        format!("lsp: failed to read initialize response: {error}"),
                    ))
                }
                Err(_) => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "lsp: initialize response timed out",
                    ))
                }
            }
        }
        let notif = json!({
            "jsonrpc": "2.0",
            "method": "initialized",
            "params": {}
        });
        write_msg(&mut self.stdin, &notif).await?;
        Ok(())
    }

    pub async fn sync_document(
        &mut self,
        uri: &str,
        lang: &str,
        text: &str,
    ) -> std::io::Result<i32> {
        let version = self.documents.entry(uri.to_string()).or_insert(0);
        *version += 1;
        let current_version = *version;
        let notif = if *version == 1 {
            json!({
                "jsonrpc": "2.0",
                "method": "textDocument/didOpen",
                "params": { "textDocument": {
                    "uri": uri, "languageId": lang, "version": *version, "text": text
                }}
            })
        } else {
            json!({
                "jsonrpc": "2.0",
                "method": "textDocument/didChange",
                "params": {
                    "textDocument": { "uri": uri, "version": *version },
                    "contentChanges": [{ "text": text }]
                }
            })
        };
        self.doc_hashes.insert(uri.to_string(), text_hash(text));
        write_msg(&mut self.stdin, &notif).await?;
        Ok(current_version)
    }

    /// [`Self::sync_document`], skipped when the server already holds exactly this text.
    pub(crate) async fn sync_if_changed(
        &mut self,
        uri: &str,
        lang: &str,
        text: &str,
    ) -> std::io::Result<()> {
        if self.doc_hashes.get(uri) == Some(&text_hash(text)) {
            return Ok(());
        }
        self.sync_document(uri, lang, text).await.map(|_| ())
    }

    /// Bring every document this server holds in line with the disk. Shell and script edits never
    /// pass through a dispatcher hook, so nav calls would otherwise answer from stale text.
    pub(crate) async fn resync_from_disk(&mut self) -> std::io::Result<()> {
        let mut uris: Vec<String> = self.documents.keys().cloned().collect();
        uris.sort();
        uris.truncate(RESYNC_DOCUMENT_CAP);
        for uri in uris {
            let Some(path) = crate::position::uri_to_path(&uri) else {
                continue;
            };
            match std::fs::read_to_string(&path) {
                Ok(text) => {
                    if self.doc_hashes.get(&uri) != Some(&text_hash(&text)) {
                        self.sync_document(&uri, "", &text).await?;
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    self.documents.remove(&uri);
                    self.doc_hashes.remove(&uri);
                    let close = json!({
                        "jsonrpc": "2.0",
                        "method": "textDocument/didClose",
                        "params": { "textDocument": { "uri": uri } }
                    });
                    write_msg(&mut self.stdin, &close).await?;
                }
                Err(_) => {}
            }
        }
        Ok(())
    }

    /// Whether the server has an open `$/progress` task (still indexing).
    pub(crate) fn is_indexing(&self) -> bool {
        !self.progress_active.is_empty()
    }

    /// Send one request and wait for its response, answering the server's own requests and
    /// discarding everything else. A passed `deadline` sends `$/cancelRequest`.
    pub(crate) async fn request(
        &mut self,
        method: &str,
        params: Value,
        deadline: tokio::time::Instant,
    ) -> Result<Value, NavError> {
        let mut retried = false;
        let mut id = self.new_id();
        self.send_request(id, method, &params).await?;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            let read = if remaining.is_zero() {
                None
            } else {
                tokio::time::timeout(remaining, read_msg(&mut self.stdout))
                    .await
                    .ok()
            };
            let msg = match read {
                None => {
                    let cancel = json!({
                        "jsonrpc": "2.0",
                        "method": "$/cancelRequest",
                        "params": { "id": id }
                    });
                    let _ = write_msg(&mut self.stdin, &cancel).await;
                    return Err(NavError::TimedOut);
                }
                Some(Ok(Some(msg))) => msg,
                Some(Ok(None)) => return Err(NavError::Transport("server closed stdout".into())),
                Some(Err(error)) => return Err(NavError::Transport(error.to_string())),
            };
            if msg.get("method").is_some() {
                self.answer_incoming(&msg).await?;
                continue;
            }
            if msg.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }
            if let Some(error) = msg.get("error") {
                let code = error.get("code").and_then(Value::as_i64).unwrap_or(0);
                if code == CONTENT_MODIFIED && !retried {
                    retried = true;
                    id = self.new_id();
                    self.send_request(id, method, &params).await?;
                    continue;
                }
                let text = error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("request failed");
                return Err(NavError::Server(sanitize_log_text(text)));
            }
            return Ok(msg.get("result").cloned().unwrap_or(Value::Null));
        }
    }

    async fn send_request(
        &mut self,
        id: u64,
        method: &str,
        params: &Value,
    ) -> Result<(), NavError> {
        let req = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        write_msg(&mut self.stdin, &req)
            .await
            .map_err(|error| NavError::Transport(error.to_string()))
    }

    /// Handle a server-initiated message: answer its requests, track progress, drop the rest.
    async fn answer_incoming(&mut self, msg: &Value) -> Result<(), NavError> {
        let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
        if method == "$/progress" {
            let token = msg
                .pointer("/params/token")
                .map(Value::to_string)
                .unwrap_or_default();
            match msg.pointer("/params/value/kind").and_then(Value::as_str) {
                Some("begin") => {
                    self.progress_active.insert(token);
                }
                Some("end") => {
                    self.progress_active.remove(&token);
                }
                _ => {}
            }
            return Ok(());
        }
        let Some(id) = msg.get("id") else {
            return Ok(());
        };
        let result = if method == "workspace/configuration" {
            let items = msg
                .pointer("/params/items")
                .and_then(Value::as_array)
                .map_or(0, Vec::len);
            Value::Array(vec![Value::Null; items])
        } else {
            Value::Null
        };
        let reply = json!({ "jsonrpc": "2.0", "id": id, "result": result });
        write_msg(&mut self.stdin, &reply)
            .await
            .map_err(|error| NavError::Transport(error.to_string()))
    }

    pub async fn collect_diagnostics(
        &mut self,
        uri: &str,
        minimum_version: i32,
        timeout: Duration,
    ) -> std::io::Result<Vec<Diagnostic>> {
        let mut diags = Vec::new();
        let mut deadline = tokio::time::Instant::now() + timeout;
        let hard_deadline = deadline;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                break;
            }
            let msg = match tokio::time::timeout(remaining, read_msg(&mut self.stdout)).await {
                Ok(Ok(Some(m))) => m,
                Ok(Ok(None)) => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        "lsp: server closed stdout while collecting diagnostics",
                    ))
                }
                Ok(Err(error)) => {
                    return Err(std::io::Error::new(
                        error.kind(),
                        format!("lsp: failed to read diagnostics message: {error}"),
                    ))
                }
                Err(_) => break,
            };
            if msg.get("method").and_then(|v| v.as_str()) == Some("textDocument/publishDiagnostics")
            {
                if let Some(params) = msg.get("params") {
                    let msg_uri = params.get("uri").and_then(|v| v.as_str()).unwrap_or("");
                    if msg_uri == uri {
                        if !diagnostic_version_is_current(params, minimum_version) {
                            continue;
                        }
                        if let Some(arr) = params.get("diagnostics").and_then(|v| v.as_array()) {
                            diags = arr.iter().filter_map(parse_diagnostic).collect();
                        }
                        // Servers like rust-analyzer publish an empty set first and the real one
                        // moments later: keep the latest publish once the stream goes quiet.
                        deadline =
                            hard_deadline.min(tokio::time::Instant::now() + DIAGNOSTICS_SETTLE);
                        continue;
                    }
                }
            }
        }
        Ok(diags)
    }
}

/// Most documents re-read from disk before one nav request.
const RESYNC_DOCUMENT_CAP: usize = 64;
/// LSP `ContentModified`: the server's state changed under the request; one retry is allowed.
const CONTENT_MODIFIED: i64 = -32801;

/// Why a navigation request produced no result.
#[derive(Debug)]
pub enum NavError {
    TimedOut,
    Transport(String),
    Server(String),
}

fn text_hash(text: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

/// Quiet time after a matching publishDiagnostics before it is taken as the final set.
const DIAGNOSTICS_SETTLE: Duration = Duration::from_millis(750);

fn initialize_request(
    id: u64,
    root_uri: &str,
    initialization_options: Option<Value>,
    nav: bool,
) -> Value {
    let mut params = json!({
        "processId": std::process::id(),
        "rootUri": root_uri,
        "capabilities": {
            "textDocument": {
                "publishDiagnostics": {}
            }
        }
    });
    if nav {
        params["capabilities"] = json!({
            "textDocument": {
                "publishDiagnostics": {},
                "definition": { "linkSupport": true },
                "references": {},
                "hover": { "contentFormat": ["plaintext", "markdown"] }
            },
            "general": { "positionEncodings": ["utf-16"] },
            "workspace": { "configuration": true },
            "window": { "workDoneProgress": true }
        });
        params["workspaceFolders"] = json!([{ "uri": root_uri, "name": "workspace" }]);
    }
    if let Some(options) = initialization_options {
        params
            .as_object_mut()
            .expect("initialize params are an object")
            .insert("initializationOptions".to_string(), options);
    }
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "initialize",
        "params": params
    })
}

async fn monitor_process_tree_memory(
    root_pid: u32,
    limit_bytes: u64,
    stderr: Arc<Mutex<String>>,
) -> Result<(), String> {
    let root = Pid::from_u32(root_pid);
    let mut system = System::new();

    loop {
        let sample = tokio::task::spawn_blocking(move || {
            system.refresh_processes(ProcessesToUpdate::All, true);
            let root_exists = system.process(root).is_some();
            // Linux lists every thread as a process entry repeating its process's RSS.
            let relationships = system
                .processes()
                .iter()
                .filter(|(_, process)| process.thread_kind().is_none())
                .map(|(pid, process)| (*pid, process.parent()))
                .collect::<Vec<_>>();
            let members = process_tree_pids(root, &relationships);
            let resident_bytes = members.iter().fold(0u64, |total, pid| {
                total.saturating_add(
                    system
                        .process(*pid)
                        .map(sysinfo::Process::memory)
                        .unwrap_or(0),
                )
            });
            (system, root_exists, members, resident_bytes)
        })
        .await;

        let (next_system, root_exists, members, resident_bytes) = match sample {
            Ok(sample) => sample,
            Err(error) => return Err(format!("sampling process tree: {error}")),
        };
        system = next_system;
        if !root_exists {
            return Ok(());
        }

        if resident_bytes > limit_bytes {
            let resident_mib = resident_bytes / (1024 * 1024);
            let limit_mib = limit_bytes / (1024 * 1024);
            let message = format!(
                "forge lsp memory guard stopped process tree {root_pid}: \
                 {resident_mib} MiB exceeded {limit_mib} MiB"
            );
            warn!("{message}");
            if let Ok(mut sink) = stderr.lock() {
                append_bounded(&mut sink, &message, STDERR_TAIL_BYTES);
            }
            let failed_kills = tokio::task::spawn_blocking(move || {
                // Kill children before the server so cargo/rustc helpers cannot be orphaned.
                let mut failed = 0;
                for pid in members.iter().filter(|pid| **pid != root) {
                    if let Some(process) = system.process(*pid) {
                        failed += usize::from(!process.kill());
                    }
                }
                if let Some(process) = system.process(root) {
                    failed += usize::from(!process.kill());
                }
                failed
            })
            .await
            .map_err(|error| format!("stopping process tree: {error}"))?;
            if failed_kills > 0 {
                return Err(format!(
                    "stopping process tree: {failed_kills} process kill(s) failed"
                ));
            }
            return Ok(());
        }

        tokio::time::sleep(RESOURCE_SAMPLE_INTERVAL).await;
    }
}

fn process_tree_pids(root: Pid, relationships: &[(Pid, Option<Pid>)]) -> HashSet<Pid> {
    let mut members = HashSet::from([root]);
    loop {
        let before = members.len();
        for (pid, parent) in relationships {
            if parent.is_some_and(|parent| members.contains(&parent)) {
                members.insert(*pid);
            }
        }
        if members.len() == before {
            return members;
        }
    }
}

fn diagnostic_version_is_current(params: &Value, minimum_version: i32) -> bool {
    params
        .get("version")
        .and_then(Value::as_i64)
        .is_none_or(|version| version >= i64::from(minimum_version))
}

/// Append `chunk`, keeping at most the last `cap` bytes (trimmed forward to a char boundary).
fn append_bounded(buf: &mut String, chunk: &str, cap: usize) {
    buf.push_str(chunk);
    if buf.len() <= cap {
        return;
    }
    let mut cut = buf.len() - cap;
    while cut < buf.len() && !buf.is_char_boundary(cut) {
        cut += 1;
    }
    buf.drain(..cut);
}

fn append_stderr_read_error(buf: &mut String, error: &std::io::Error) {
    let message = format!("\nstderr reader error: {error}");
    append_bounded(buf, &message, STDERR_TAIL_BYTES);
}

fn sanitize_log_text(raw: &str) -> String {
    let mut output = String::new();
    let mut chars = raw.chars().peekable();
    while let Some(character) = chars.next() {
        if character == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for next in chars.by_ref() {
                    if ('@'..='~').contains(&next) {
                        break;
                    }
                }
            }
            continue;
        }
        if character.is_control() {
            output.push(' ');
        } else {
            output.push(character);
        }
    }
    output.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Collapse a captured stderr tail into one log-safe line, clipped to `max_chars` from the end
/// (the tail carries the fatal message; earlier progress chatter is the part worth dropping).
fn summarize_stderr(raw: &str, max_chars: usize) -> String {
    let joined = sanitize_log_text(raw);
    let count = joined.chars().count();
    if count <= max_chars {
        return joined;
    }
    let start = joined
        .char_indices()
        .nth(count - max_chars)
        .map(|(i, _)| i)
        .unwrap_or(0);
    format!("…{}", &joined[start..])
}

fn parse_diagnostic(v: &Value) -> Option<Diagnostic> {
    let message = v.get("message")?.as_str()?.to_string();
    let range = v.get("range")?;
    let start = range.get("start")?;
    let line = start.get("line")?.as_u64()? as u32;
    let character = start.get("character")?.as_u64()? as u32;
    let severity = v
        .get("severity")
        .and_then(|s| s.as_u64())
        .map(DiagnosticSeverity::from_lsp_int)
        .unwrap_or(DiagnosticSeverity::Error);
    let code = v.get("code").and_then(|c| {
        if let Some(s) = c.as_str() {
            Some(s.to_string())
        } else {
            c.as_u64().map(|n| n.to_string())
        }
    });
    Some(Diagnostic {
        severity,
        message,
        line,
        character,
        code,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[cfg(unix)]
    #[tokio::test]
    async fn diagnostics_wait_past_an_empty_first_publish() {
        let uri = "file:///w/main.rs";
        let empty = format!(
            r#"{{"jsonrpc":"2.0","method":"textDocument/publishDiagnostics","params":{{"uri":"{uri}","diagnostics":[]}}}}"#
        );
        let full = format!(
            r#"{{"jsonrpc":"2.0","method":"textDocument/publishDiagnostics","params":{{"uri":"{uri}","diagnostics":[{{"message":"mismatched types","severity":1,"range":{{"start":{{"line":0,"character":0}},"end":{{"line":0,"character":1}}}}}}]}}}}"#
        );
        let script = format!(
            "printf 'Content-Length: %d\\r\\n\\r\\n%s' {} '{empty}'; sleep 0.3; \
             printf 'Content-Length: %d\\r\\n\\r\\n%s' {} '{full}'; sleep 5",
            empty.len(),
            full.len()
        );
        let mut server = LspServer::spawn("/bin/sh", &["-c".to_string(), script])
            .await
            .unwrap();
        let diags = server
            .collect_diagnostics(uri, 0, Duration::from_secs(5))
            .await
            .unwrap();
        assert_eq!(diags.len(), 1);
    }

    #[test]
    fn initialize_request_carries_resource_safe_options() {
        let request = initialize_request(
            7,
            "file:///workspace",
            Some(json!({"numThreads": 1, "checkOnSave": false})),
            false,
        );

        assert_eq!(request["id"], 7);
        assert_eq!(request["params"]["rootUri"], "file:///workspace");
        assert_eq!(
            request["params"]["initializationOptions"],
            json!({"numThreads": 1, "checkOnSave": false})
        );
    }

    /// A fake server that echoes everything it is sent into `log` and replays `frames` first.
    #[cfg(unix)]
    async fn fake_server(
        frames: &[Value],
        dir: &std::path::Path,
    ) -> (LspServer, std::path::PathBuf) {
        let mut bytes = Vec::new();
        for frame in frames {
            let body = serde_json::to_vec(frame).unwrap();
            bytes.extend(format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes());
            bytes.extend(body);
        }
        let replay = dir.join("replay");
        let log = dir.join("log");
        std::fs::write(&replay, bytes).unwrap();
        let script = format!(
            "cat {}; cat <&0 >{} & exec sleep 30",
            replay.display(),
            log.display()
        );
        let server = LspServer::spawn("/bin/sh", &["-c".to_string(), script])
            .await
            .unwrap();
        (server, log)
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn request_skips_noise_answers_server_requests_and_returns_its_result() {
        let dir = tempfile::tempdir().unwrap();
        let frames = [
            json!({"jsonrpc":"2.0","method":"textDocument/publishDiagnostics","params":{"uri":"file:///x","diagnostics":[]}}),
            json!({"jsonrpc":"2.0","id":99,"result":"late reply to a cancelled request"}),
            json!({"jsonrpc":"2.0","id":7,"method":"workspace/configuration","params":{"items":[{},{}]}}),
            json!({"jsonrpc":"2.0","method":"$/progress","params":{"token":"t","value":{"kind":"begin"}}}),
            json!({"jsonrpc":"2.0","id":1,"result":{"ok":true}}),
        ];
        let (mut server, log) = fake_server(&frames, dir.path()).await;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        let result = server
            .request("textDocument/definition", json!({"a": 1}), deadline)
            .await
            .unwrap();
        assert_eq!(result, json!({"ok": true}));
        assert!(server.is_indexing());
        tokio::time::sleep(Duration::from_millis(200)).await;
        let sent = std::fs::read_to_string(log).unwrap();
        assert!(
            sent.contains(r#""method":"textDocument/definition""#),
            "{sent}"
        );
        assert!(
            sent.contains(r#""id":7,"jsonrpc":"2.0","result":[null,null]"#),
            "{sent}"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn request_cancels_on_deadline() {
        let dir = tempfile::tempdir().unwrap();
        let (mut server, log) = fake_server(&[], dir.path()).await;
        let deadline = tokio::time::Instant::now() + Duration::from_millis(150);
        let error = server
            .request("textDocument/hover", json!({}), deadline)
            .await
            .unwrap_err();
        assert!(matches!(error, NavError::TimedOut), "{error:?}");
        tokio::time::sleep(Duration::from_millis(200)).await;
        let sent = std::fs::read_to_string(log).unwrap();
        assert!(
            sent.contains(r#""method":"$/cancelRequest","params":{"id":1}"#),
            "{sent}"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn resync_sends_changes_only_for_edited_and_deleted_files() {
        let dir = tempfile::tempdir().unwrap();
        let (mut server, log) = fake_server(&[], dir.path()).await;
        let a = dir.path().join("a.py");
        let b = dir.path().join("b.py");
        let c = dir.path().join("c.py");
        for path in [&a, &b, &c] {
            std::fs::write(path, "one").unwrap();
            let uri = format!("file://{}", path.display());
            server.sync_document(&uri, "python", "one").await.unwrap();
        }
        std::fs::write(&b, "two").unwrap();
        std::fs::remove_file(&c).unwrap();
        server.resync_from_disk().await.unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
        let sent = std::fs::read_to_string(log).unwrap();
        assert_eq!(sent.matches("textDocument/didOpen").count(), 3, "{sent}");
        assert_eq!(sent.matches("textDocument/didChange").count(), 1, "{sent}");
        assert!(sent.contains(&format!("file://{}", b.display())));
        assert_eq!(sent.matches("textDocument/didClose").count(), 1, "{sent}");
    }

    #[test]
    fn non_nav_initialize_request_is_unchanged() {
        let plain = initialize_request(1, "file:///w", None, false);
        assert_eq!(
            plain["params"]["capabilities"],
            json!({"textDocument": {"publishDiagnostics": {}}})
        );
        assert!(plain["params"].get("workspaceFolders").is_none());
        let nav = initialize_request(1, "file:///w", None, true);
        assert_eq!(
            nav["params"]["capabilities"]["general"]["positionEncodings"],
            json!(["utf-16"])
        );
        assert_eq!(
            nav["params"]["capabilities"]["workspace"]["configuration"],
            true
        );
    }

    #[test]
    fn process_tree_membership_excludes_unrelated_processes() {
        let root = Pid::from_u32(10);
        let relationships = [
            (root, Some(Pid::from_u32(1))),
            (Pid::from_u32(11), Some(root)),
            (Pid::from_u32(12), Some(Pid::from_u32(11))),
            (Pid::from_u32(20), Some(Pid::from_u32(1))),
        ];

        let members = process_tree_pids(root, &relationships);

        assert_eq!(members.len(), 3);
        assert!(members.contains(&root));
        assert!(members.contains(&Pid::from_u32(11)));
        assert!(members.contains(&Pid::from_u32(12)));
        assert!(!members.contains(&Pid::from_u32(20)));
    }

    #[test]
    fn stale_published_diagnostics_are_rejected_by_version() {
        assert!(!diagnostic_version_is_current(&json!({"version": 2}), 3));
        assert!(diagnostic_version_is_current(&json!({"version": 3}), 3));
        assert!(diagnostic_version_is_current(&json!({"version": 4}), 3));
        assert!(diagnostic_version_is_current(&json!({}), 3));
    }

    fn full_diag() -> Value {
        json!({
            "message": "cannot find value `x`",
            "severity": 1,
            "code": "E0425",
            "range": {
                "start": {"line": 4, "character": 8},
                "end": {"line": 4, "character": 9}
            }
        })
    }

    #[test]
    fn parse_full_diagnostic() {
        let d = parse_diagnostic(&full_diag()).expect("valid diagnostic");
        assert_eq!(d.severity, DiagnosticSeverity::Error);
        assert_eq!(d.message, "cannot find value `x`");
        assert_eq!(d.line, 4);
        assert_eq!(d.character, 8);
        assert_eq!(d.code.as_deref(), Some("E0425"));
    }

    #[test]
    fn severity_int_maps_to_enum() {
        let mut v = full_diag();
        v["severity"] = json!(2);
        assert_eq!(
            parse_diagnostic(&v).unwrap().severity,
            DiagnosticSeverity::Warning
        );
        v["severity"] = json!(3);
        assert_eq!(
            parse_diagnostic(&v).unwrap().severity,
            DiagnosticSeverity::Information
        );
        v["severity"] = json!(4);
        assert_eq!(
            parse_diagnostic(&v).unwrap().severity,
            DiagnosticSeverity::Hint
        );
    }

    #[test]
    fn missing_severity_defaults_to_error() {
        // LSP allows omitting severity; Forge treats an unlabeled diagnostic as an error.
        let mut v = full_diag();
        v.as_object_mut().unwrap().remove("severity");
        assert_eq!(
            parse_diagnostic(&v).unwrap().severity,
            DiagnosticSeverity::Error
        );
    }

    #[test]
    fn code_can_be_integer() {
        let mut v = full_diag();
        v["code"] = json!(2304);
        assert_eq!(parse_diagnostic(&v).unwrap().code.as_deref(), Some("2304"));
    }

    #[test]
    fn missing_code_is_none() {
        let mut v = full_diag();
        v.as_object_mut().unwrap().remove("code");
        assert_eq!(parse_diagnostic(&v).unwrap().code, None);
    }

    #[test]
    fn non_scalar_code_is_none() {
        // A float/object code is not representable; degrade to None rather than panic.
        let mut v = full_diag();
        v["code"] = json!(1.5);
        assert_eq!(parse_diagnostic(&v).unwrap().code, None);
    }

    #[test]
    fn missing_message_is_rejected() {
        let mut v = full_diag();
        v.as_object_mut().unwrap().remove("message");
        assert!(parse_diagnostic(&v).is_none());
    }

    #[test]
    fn non_string_message_is_rejected() {
        let mut v = full_diag();
        v["message"] = json!(42);
        assert!(parse_diagnostic(&v).is_none());
    }

    #[test]
    fn missing_range_is_rejected() {
        let mut v = full_diag();
        v.as_object_mut().unwrap().remove("range");
        assert!(parse_diagnostic(&v).is_none());
    }

    #[test]
    fn missing_start_position_is_rejected() {
        let mut v = full_diag();
        v["range"]["start"] = Value::Null;
        assert!(parse_diagnostic(&v).is_none());
    }

    #[test]
    fn empty_object_is_rejected() {
        assert!(parse_diagnostic(&json!({})).is_none());
    }

    #[test]
    fn non_numeric_line_is_rejected() {
        let mut v = full_diag();
        v["range"]["start"]["line"] = json!("oops");
        assert!(parse_diagnostic(&v).is_none());
    }

    #[test]
    fn stderr_tail_keeps_the_last_bytes_on_a_char_boundary() {
        let mut buf = String::new();
        append_bounded(&mut buf, "first\n", 8);
        assert_eq!(buf, "first\n");
        append_bounded(&mut buf, "second\n", 8);
        assert_eq!(buf, "\nsecond\n", "only the last {} bytes are kept", 8);
        // A multi-byte character straddling the cut is dropped whole, never split.
        let mut wide = String::new();
        append_bounded(&mut wide, "ééééé", 5);
        assert!(wide.chars().all(|c| c == 'é'), "tail was: {wide:?}");
        assert!(wide.len() <= 5);
    }

    #[test]
    fn stderr_reader_errors_are_retained_in_the_bounded_tail() {
        let mut buf = String::from("server output");
        append_stderr_read_error(
            &mut buf,
            &std::io::Error::new(std::io::ErrorKind::BrokenPipe, "pipe closed unexpectedly"),
        );

        let summary = summarize_stderr(&buf, STDERR_SUMMARY_CHARS);
        assert!(summary.contains("stderr reader error"));
        assert!(summary.contains("pipe closed unexpectedly"));
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn initialize_rejects_json_rpc_errors() {
        let tmp = tempfile::TempDir::new().unwrap();
        let script = tmp.path().join("reject-lsp.sh");
        std::fs::write(
            &script,
            "IFS= read -r header\nlen=${header#Content-Length: }\nIFS= read -r blank\ndd bs=1 count=$len of=/dev/null 2>/dev/null\nbody='{\"jsonrpc\":\"2.0\",\"id\":1,\"error\":{\"code\":-32603,\"message\":\"bad init\"}}'\nprintf 'Content-Length: %d\\r\\n\\r\\n%s' ${#body} \"$body\"\nsleep 1\n",
        )
        .unwrap();
        // Run the script as an argument to /bin/sh rather than exec'ing it directly. Exec'ing a
        // file this process just wrote races every other test that spawns a child: a concurrent
        // fork duplicates the still-open write fd, and exec on that inode fails ETXTBSY until the
        // child execs. `sh` only ever *reads* this path, which no such race affects.
        let mut server = LspServer::spawn("/bin/sh", &[script.to_string_lossy().into_owned()])
            .await
            .unwrap();
        let error = server
            .initialize("file:///tmp", Duration::from_secs(1))
            .await
            .expect_err("JSON-RPC initialize error must fail startup");
        assert!(error.to_string().contains("bad init"));
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn memory_guard_stops_an_over_budget_process_tree() {
        let args = vec!["-c".to_owned(), "sleep 30".to_owned()];
        let mut server = LspServer::spawn_with_memory_limit("/bin/sh", &args, Some(1), false)
            .await
            .unwrap();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        while server._child.try_wait().unwrap().is_none() && tokio::time::Instant::now() < deadline
        {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(
            server._child.try_wait().unwrap().is_some(),
            "the guard must terminate an over-budget server"
        );
        let cause = server.stderr_summary().await;
        assert!(
            cause.contains("forge lsp memory guard stopped process tree"),
            "stderr summary was: {cause:?}"
        );
    }

    #[test]
    fn stderr_summary_is_one_clipped_line() {
        assert_eq!(summarize_stderr("", 100), "");
        assert_eq!(summarize_stderr("   \n\n  \n", 100), "");
        assert_eq!(
            summarize_stderr(
                "error: 'rust-analyzer' is not installed\n  help: run rustup\n",
                100
            ),
            "error: 'rust-analyzer' is not installed help: run rustup"
        );
        // Clipping keeps the END of the tail — the fatal line, not the startup chatter.
        let long = summarize_stderr("aaaaaaaaaa\nfatal", 5);
        assert_eq!(long, "…fatal");
        assert_eq!(
            summarize_stderr("\u{1b}[31mERROR\u{1b}[0m\r\nnext\u{7}line", 100),
            "ERROR next line"
        );
    }
}
