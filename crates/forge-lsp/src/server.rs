use std::collections::{HashMap, HashSet};
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
                        Ok(0) => {
                            debug!("lsp: stderr reader reached EOF");
                            break;
                        }
                        Err(error) => {
                            warn!("lsp: stderr reader failed: {error}");
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
        let id = self.new_id();
        let req = initialize_request(id, root_uri, initialization_options);
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
                Ok(Some(msg)) => {
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
                        break;
                    }
                }
                Ok(None) => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        "lsp: server closed stdout during initialize",
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
        write_msg(&mut self.stdin, &notif).await?;
        Ok(current_version)
    }

    pub async fn collect_diagnostics(
        &mut self,
        uri: &str,
        minimum_version: i32,
        timeout: Duration,
    ) -> std::io::Result<Vec<Diagnostic>> {
        let mut diags = Vec::new();
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                break;
            }
            let msg = match tokio::time::timeout(remaining, read_msg(&mut self.stdout)).await {
                Ok(Some(m)) => m,
                Ok(None) => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        "lsp: server closed stdout while collecting diagnostics",
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
                        break;
                    }
                }
            }
        }
        Ok(diags)
    }
}

fn initialize_request(id: u64, root_uri: &str, initialization_options: Option<Value>) -> Value {
    let mut params = json!({
        "processId": std::process::id(),
        "rootUri": root_uri,
        "capabilities": {
            "textDocument": {
                "publishDiagnostics": {}
            }
        }
    });
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
            let relationships = system
                .processes()
                .iter()
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

    #[test]
    fn initialize_request_carries_resource_safe_options() {
        let request = initialize_request(
            7,
            "file:///workspace",
            Some(json!({"numThreads": 1, "checkOnSave": false})),
        );

        assert_eq!(request["id"], 7);
        assert_eq!(request["params"]["rootUri"], "file:///workspace");
        assert_eq!(
            request["params"]["initializationOptions"],
            json!({"numThreads": 1, "checkOnSave": false})
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
