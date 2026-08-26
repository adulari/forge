use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tokio::sync::{Mutex, Semaphore};
use tracing::{debug, warn};

use forge_config::LspConfig;

use crate::server::LspServer;
use crate::types::{Diagnostic, DiagnosticSeverity};

/// First cooldown after a language server fails to start or hand shake.
const BACKOFF_BASE: Duration = Duration::from_secs(30);
/// Ceiling for the doubling cooldown, so a permanently broken toolchain settles at one retry
/// every ten minutes instead of one process per edited file.
const BACKOFF_MAX: Duration = Duration::from_secs(600);
/// A daemon can visit many worktrees over its lifetime; never retain one heavyweight analyzer for
/// every root it has touched.
const MAX_SERVER_SLOTS: usize = 4;
/// Only one heavyweight analyzer process tree may run in Forge at once. Combined with the
/// per-process-tree resident-memory guard, this makes the configured per-server limit an aggregate
/// process-wide LSP ceiling as well.
const MAX_LIVE_SERVERS: usize = 1;
/// A lightweight rust-analyzer still needs a short cold-start window to load project metadata.
/// Hot diagnostic requests continue to use the operator-configured timeout.
const COLD_START_DIAGNOSTIC_TIMEOUT: Duration = Duration::from_secs(12);

/// A lazily-initialized language-server slot for one `(language, repo-root)` pair, behind its
/// own lock so a hung server only blocks callers waiting on that same pair.
type ServerSlot = Arc<Mutex<ServerEntry>>;

fn global_permits() -> &'static Arc<Semaphore> {
    static PERMITS: OnceLock<Arc<Semaphore>> = OnceLock::new();
    PERMITS.get_or_init(|| Arc::new(Semaphore::new(MAX_LIVE_SERVERS)))
}

/// The live server for one `(language, repo-root)` pair plus its failure backoff.
///
/// Without the backoff, a server that cannot start (a missing rustup component, a broken config)
/// is respawned on every single diagnostic request: one doomed process, one truncated handshake,
/// and one identical warning per edited file. Recording the failure lets Forge skip the attempt
/// until the cooldown expires, and a later success (once the toolchain is repaired) clears it, so
/// diagnostics come back on their own without restarting Forge.
struct ServerEntry {
    server: Option<LspServer>,
    consecutive_failures: u32,
    retry_at: Option<Instant>,
    last_used: Instant,
    idle_generation: u64,
    idle_timer: Option<tokio::task::AbortHandle>,
    permit: Option<tokio::sync::OwnedSemaphorePermit>,
}

impl Default for ServerEntry {
    fn default() -> Self {
        Self {
            server: None,
            consecutive_failures: 0,
            retry_at: None,
            last_used: Instant::now(),
            idle_generation: 0,
            idle_timer: None,
            permit: None,
        }
    }
}

impl ServerEntry {
    /// Whether a start attempt is still suppressed by the current cooldown.
    fn cooling_down(&self, now: Instant) -> bool {
        self.retry_at.is_some_and(|retry_at| now < retry_at)
    }

    /// Drop any live server, extend the cooldown, and return how long the next attempt waits.
    fn record_failure(&mut self, now: Instant) -> Duration {
        self.server = None;
        self.permit = None;
        self.consecutive_failures = self.consecutive_failures.saturating_add(1);
        let backoff = failure_backoff(self.consecutive_failures);
        self.retry_at = Some(now + backoff);
        backoff
    }

    fn clear_failure(&mut self) {
        self.consecutive_failures = 0;
        self.retry_at = None;
    }

    fn touch(&mut self) -> u64 {
        self.last_used = Instant::now();
        self.idle_generation = self.idle_generation.wrapping_add(1);
        self.idle_generation
    }

    fn is_idle_for(&self, now: Instant, ttl: Duration) -> bool {
        now.duration_since(self.last_used) >= ttl
    }
}

/// Exponential cooldown for the nth consecutive failure, capped at [`BACKOFF_MAX`].
fn failure_backoff(consecutive_failures: u32) -> Duration {
    let doublings = consecutive_failures.saturating_sub(1).min(16);
    BACKOFF_BASE
        .saturating_mul(1u32 << doublings)
        .min(BACKOFF_MAX)
}

/// Owns the live language-server processes and routes a file to the right one.
///
/// One server is spawned lazily per `(language, repo-root)` and reused across calls (kept in
/// `servers`), so repeated diagnostics on the same project don't pay startup each time.
///
/// Each entry has its own `Mutex` so a stalled/hung server for one `(language, repo-root)` only
/// blocks callers waiting on *that* entry; the outer `servers` lock is only ever held briefly to
/// look up or insert the entry itself, never across the slow spawn/initialize/diagnostics work.
pub struct LspRegistry {
    config: LspConfig,
    servers: Mutex<HashMap<(String, PathBuf), ServerSlot>>,
}

impl LspRegistry {
    /// Build a registry from the user's `[lsp]` config (which servers are enabled, their commands).
    pub fn from_config(config: &LspConfig) -> Self {
        Self {
            config: config.clone(),
            servers: Mutex::new(HashMap::new()),
        }
    }

    fn idle_ttl(&self) -> Duration {
        Duration::from_secs(self.config.idle_timeout_secs.max(1))
    }

    fn memory_limit_bytes(&self) -> Option<u64> {
        self.config
            .memory_limit_mb
            .checked_mul(1024 * 1024)
            .filter(|limit| *limit > 0)
    }

    /// Diagnostics for one file. Configuration that does not apply to the file remains an empty
    /// result, while an analyzer outage is returned as a synthetic informational diagnostic. The
    /// latter keeps callers' existing `Vec<Diagnostic>` API but prevents a broken analyzer from
    /// looking identical to a clean file.
    pub async fn diagnostics_for(&self, abs_path: &Path, timeout: Duration) -> Vec<Diagnostic> {
        let Some(lang) = lang_from_ext(abs_path) else {
            return vec![];
        };
        let Some(root) = repo_root(abs_path) else {
            return vec![];
        };
        let Some((cmd, args)) = self.server_for_lang(lang) else {
            return vec![];
        };
        if which(&cmd).is_none() {
            return Self::unavailable_diagnostic(
                abs_path,
                format!("language server `{cmd}` is not available on PATH"),
            );
        }

        let text = match std::fs::read_to_string(abs_path) {
            Ok(t) => t,
            Err(e) => {
                warn!("lsp: cannot read {}: {e}", abs_path.display());
                return Self::unavailable_diagnostic(
                    abs_path,
                    format!("cannot read source file: {e}"),
                );
            }
        };

        let uri = path_to_uri(abs_path);
        let root_uri = path_to_uri(&root);
        let key = (lang.to_string(), root.clone());
        let idle_ttl = self.idle_ttl();
        let rust_resource_profile = uses_rust_analyzer_profile(lang, &cmd);

        // Only the map lookup/insert happens under the registry-wide lock; the entry's own
        // lock (acquired below, after this guard is dropped) is what serializes the actual
        // spawn/initialize/diagnostics work for that one (language, repo-root) pair.
        let entry = {
            let mut servers = self.servers.lock().await;
            let now = Instant::now();
            // Do not wait behind a diagnostic request while holding the registry lock. Busy slots
            // stay alive; idle ones are dropped (and kill_on_drop reaps their child).
            servers.retain(|_, entry| {
                entry
                    .try_lock()
                    .map(|slot| !slot.is_idle_for(now, idle_ttl) || slot.cooling_down(now))
                    .unwrap_or(true)
            });
            if !servers.contains_key(&key) && servers.len() >= MAX_SERVER_SLOTS {
                let evict = servers
                    .iter()
                    .filter_map(|(key, entry)| {
                        entry
                            .try_lock()
                            .ok()
                            .map(|slot| (key.clone(), slot.last_used))
                    })
                    .min_by_key(|(_, last_used)| *last_used)
                    .map(|(key, _)| key);
                if let Some(evict) = evict {
                    debug!("lsp: evicting least-recently-used server slot to enforce root limit");
                    servers.remove(&evict);
                } else {
                    // All analyzers are busy. Diagnostics are best-effort; never overcommit.
                    return Self::unavailable_diagnostic(
                        abs_path,
                        "all language-server slots are busy",
                    );
                }
            }
            servers
                .entry(key)
                .or_insert_with(|| Arc::new(Mutex::new(ServerEntry::default())))
                .clone()
        };

        let mut slot = entry.lock().await;
        let idle_generation = slot.touch();
        arm_idle_timer(&mut slot, entry.clone(), idle_generation, idle_ttl);
        let mut cold_start = false;
        if slot.server.is_none() {
            let now = Instant::now();
            if slot.cooling_down(now) {
                debug!("lsp: {lang} ({cmd}) is in failure cooldown — skipping diagnostics");
                return Self::unavailable_diagnostic(
                    abs_path,
                    format!("{lang} language server is recovering from a previous failure"),
                );
            }
            let Some(permit) = global_permits().clone().try_acquire_owned().ok() else {
                debug!("lsp: global analyzer cap is full; deferring diagnostics");
                return Self::unavailable_diagnostic(
                    abs_path,
                    "the language-server capacity is full",
                );
            };
            slot.permit = Some(permit);
            match LspServer::spawn_with_memory_limit(
                &cmd,
                &args,
                self.memory_limit_bytes(),
                rust_resource_profile,
            )
            .await
            {
                Ok(mut srv) => match srv
                    .initialize_with_options(
                        &root_uri,
                        timeout,
                        initialization_options_for(rust_resource_profile),
                    )
                    .await
                {
                    Ok(()) => {
                        slot.clear_failure();
                        slot.server = Some(srv);
                        cold_start = true;
                    }
                    Err(e) => {
                        let cause = srv.stderr_summary().await;
                        slot.permit = None;
                        let backoff = slot.record_failure(now);
                        warn!(
                            "lsp: initialize failed for {lang} ({cmd}): {e}{} — retrying in {}s",
                            stderr_clause(&cause),
                            backoff.as_secs()
                        );
                        return Self::unavailable_diagnostic(
                            abs_path,
                            format!("{lang} language server initialization failed: {e}"),
                        );
                    }
                },
                Err(e) => {
                    slot.permit = None;
                    let backoff = slot.record_failure(now);
                    warn!(
                        "lsp: spawn failed for {lang} ({cmd}): {e} — retrying in {}s",
                        backoff.as_secs()
                    );
                    return Self::unavailable_diagnostic(
                        abs_path,
                        format!("could not start {lang} language server: {e}"),
                    );
                }
            }
        }
        let Some(server) = slot.server.as_mut() else {
            warn!("lsp: server unavailable for {lang} after spawn");
            return Self::unavailable_diagnostic(
                abs_path,
                format!("{lang} language server is unavailable"),
            );
        };

        let document_version = match server.sync_document(&uri, lang, &text).await {
            Ok(version) => version,
            Err(e) => {
                slot.server = None;
                let backoff = slot.record_failure(Instant::now());
                warn!(
                    "lsp: document sync failed for {lang} ({cmd}): {e} — retrying in {}s",
                    backoff.as_secs()
                );
                return Self::unavailable_diagnostic(
                    abs_path,
                    format!("{lang} language server could not sync the document: {e}"),
                );
            }
        };
        let diagnostic_timeout = if cold_start && rust_resource_profile {
            timeout.max(COLD_START_DIAGNOSTIC_TIMEOUT)
        } else {
            timeout
        };
        let diagnostics = match server
            .collect_diagnostics(&uri, document_version, diagnostic_timeout)
            .await
        {
            Ok(diagnostics) => diagnostics,
            Err(error) if error.kind() == std::io::ErrorKind::TimedOut => {
                debug!("lsp: diagnostics timed out for {lang} ({cmd}); keeping the server warm");
                let idle_generation = slot.touch();
                arm_idle_timer(&mut slot, entry.clone(), idle_generation, idle_ttl);
                return vec![];
            }
            Err(error) => {
                let backoff = slot.record_failure(Instant::now());
                warn!(
                    "lsp: diagnostics failed for {lang} ({cmd}): {error} — retrying in {}s",
                    backoff.as_secs()
                );
                return Self::unavailable_diagnostic(
                    abs_path,
                    format!("{lang} language server failed while collecting diagnostics: {error}"),
                );
            }
        };
        let idle_generation = slot.touch();
        arm_idle_timer(&mut slot, entry.clone(), idle_generation, idle_ttl);
        diagnostics
    }

    /// Keep analyzer failures visible to callers that intentionally use the historical
    /// `Vec<Diagnostic>` API. This is an informational hint, never a source-code error.
    fn unavailable_diagnostic(path: &Path, reason: impl Into<String>) -> Vec<Diagnostic> {
        vec![Diagnostic {
            severity: DiagnosticSeverity::Information,
            message: format!(
                "LSP diagnostics unavailable for {}: {}",
                path.display(),
                reason.into()
            ),
            line: 0,
            character: 0,
            code: Some("forge-lsp-unavailable".to_string()),
        }]
    }

    /// Forget every pending failure cooldown, as if it had elapsed.
    #[cfg(test)]
    async fn expire_cooldowns(&self) {
        for entry in self.servers.lock().await.values() {
            entry.lock().await.retry_at = None;
        }
    }

    #[cfg(test)]
    async fn server_slot_count(&self) -> usize {
        self.servers.lock().await.len()
    }

    fn server_for_lang(&self, lang: &str) -> Option<(String, Vec<String>)> {
        if let Some(entry) = self.config.servers.get(lang) {
            return Some((entry.command.clone(), entry.args.clone()));
        }
        match lang {
            "rust" => Some(("rust-analyzer".to_string(), vec![])),
            "typescript" | "javascript" => Some((
                "typescript-language-server".to_string(),
                vec!["--stdio".to_string()],
            )),
            "python" => Some((
                "pyright-langserver".to_string(),
                vec!["--stdio".to_string()],
            )),
            "go" => Some(("gopls".to_string(), vec![])),
            _ => None,
        }
    }
}

fn uses_rust_analyzer_profile(lang: &str, command: &str) -> bool {
    lang == "rust"
        && Path::new(command)
            .file_name()
            .is_some_and(|name| name.to_string_lossy().contains("rust-analyzer"))
}

fn initialization_options_for(rust_resource_profile: bool) -> Option<Value> {
    rust_resource_profile.then(|| {
        json!({
            // Forge requests diagnostics after edits; an additional full-workspace `cargo check`
            // duplicates the autofix/test pipeline and caused sustained CPU saturation.
            "checkOnSave": false,
            // Let rust-analyzer analyze opened files on demand instead of eagerly warming every
            // crate in a large workspace.
            "cachePriming": {
                "enable": false,
                "numThreads": 1
            },
            // Build-script discovery can launch dozens of cargo/rustc children during initialize.
            // Forge keeps syntax/type diagnostics available but leaves full macro/build validation
            // to its explicit check/test pipeline.
            "cargo": {
                "buildScripts": {
                    "enable": false
                }
            },
            "procMacro": {
                "enable": false
            },
            // With proc-macro expansion intentionally disabled, these notices are expected noise;
            // cargo check/Clippy remains the authoritative macro diagnostic path.
            "diagnostics": {
                "disabled": ["macro-error"]
            },
            // Forge opens files for diagnostics, not IDE-wide navigation. A smaller syntax-tree
            // cache prevents memory from ratcheting upward across long multi-file sessions.
            "lru": {
                "capacity": 32
            },
            // Keep semantic diagnostics usable while bounding its internal worker pool.
            "numThreads": 1
        })
    })
}

/// Process-wide capacity permits are held for the lifetime of each live analyzer.
fn arm_idle_timer(slot: &mut ServerEntry, entry: ServerSlot, generation: u64, delay: Duration) {
    if let Some(previous) = slot.idle_timer.take() {
        previous.abort();
    }
    let weak = Arc::downgrade(&entry);
    let task = tokio::spawn(async move {
        tokio::time::sleep(delay).await;
        let Some(entry) = weak.upgrade() else {
            debug!("lsp: idle timer expired after entry was dropped");
            return;
        };
        let mut slot = entry.lock().await;
        if slot.idle_generation == generation && slot.is_idle_for(Instant::now(), delay) {
            debug!(
                "lsp: stopping idle language server after {}s",
                delay.as_secs()
            );
            slot.server = None;
            slot.permit = None;
            slot.idle_timer = None;
        } else {
            debug!("lsp: idle timer expired but server was used again");
        }
    });
    slot.idle_timer = Some(task.abort_handle());
}

/// Render a captured stderr tail as a trailing clause, or nothing when the server was silent.
fn stderr_clause(cause: &str) -> String {
    if cause.is_empty() {
        String::new()
    } else {
        format!(" — server stderr: {cause}")
    }
}

/// Map a file extension to the language key used to look up its server (`None` = unsupported).
pub fn lang_from_ext(path: &Path) -> Option<&'static str> {
    match path.extension()?.to_str()? {
        "rs" => Some("rust"),
        "ts" | "tsx" => Some("typescript"),
        "js" | "jsx" => Some("javascript"),
        "py" => Some("python"),
        "go" => Some("go"),
        _ => None,
    }
}

/// Walk up from `path` to the nearest project root (a dir holding `Cargo.toml`, `package.json`,
/// `pyproject.toml`, `go.mod`, or `.git`) — the directory the language server is rooted at.
pub fn repo_root(path: &Path) -> Option<PathBuf> {
    let mut dir = path.parent()?;
    loop {
        for marker in &[
            "Cargo.toml",
            "package.json",
            "pyproject.toml",
            "go.mod",
            ".git",
        ] {
            if dir.join(marker).exists() {
                return Some(dir.to_path_buf());
            }
        }
        dir = dir.parent()?;
    }
}

/// Resolve a server command to an executable path (absolute path as-is, else searched on `PATH`).
pub fn which(cmd: &str) -> Option<PathBuf> {
    let p = Path::new(cmd);
    if p.is_absolute() {
        return p.exists().then(|| p.to_path_buf());
    }
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(cmd);
        if candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

fn path_to_uri(path: &Path) -> String {
    let p = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    };
    // RFC 8089 file URIs use forward slashes and a leading `/` before the path. On Unix
    // MAIN_SEPARATOR is `/` so this is a no-op; on Windows it turns `C:\a\b` into `/C:/a/b`,
    // yielding `file:///C:/a/b` instead of the malformed `file://C:\a\b`.
    let mut s = p.to_string_lossy().replace(std::path::MAIN_SEPARATOR, "/");
    if !s.starts_with('/') {
        s.insert(0, '/');
    }
    format!("file://{}", percent_encode_path(&s))
}

/// Percent-encode a URI path per RFC 3986, leaving `/` (segment separator) and `:` (needed for
/// Windows drive letters, e.g. `/C:/...`) unescaped. Without this, a path containing a space or
/// other reserved character never matches the (typically percent-encoded) URI a language server
/// echoes back in `publishDiagnostics`, silently dropping diagnostics for that file forever.
fn percent_encode_path(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' | b':' => {
                out.push(b as char);
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn rust_analyzer_uses_the_resource_safe_profile() {
        let defaults = LspConfig::default();
        assert_eq!(defaults.memory_limit_mb, 2048);
        assert_eq!(defaults.idle_timeout_secs, 120);

        assert!(uses_rust_analyzer_profile("rust", "rust-analyzer"));
        assert!(uses_rust_analyzer_profile(
            "rust",
            "/usr/local/bin/rust-analyzer-wrapper"
        ));
        assert!(!uses_rust_analyzer_profile("rust", "custom-rust-lsp"));
        assert!(!uses_rust_analyzer_profile("python", "rust-analyzer"));

        let options = initialization_options_for(true).unwrap();

        assert_eq!(options["checkOnSave"], false);
        assert_eq!(options["cachePriming"]["enable"], false);
        assert_eq!(options["cachePriming"]["numThreads"], 1);
        assert_eq!(options["cargo"]["buildScripts"]["enable"], false);
        assert_eq!(options["procMacro"]["enable"], false);
        assert_eq!(options["diagnostics"]["disabled"], json!(["macro-error"]));
        assert_eq!(options["lru"]["capacity"], 32);
        assert_eq!(options["numThreads"], 1);
        assert!(initialization_options_for(false).is_none());
    }

    #[test]
    fn lang_from_ext_table() {
        assert_eq!(lang_from_ext(Path::new("foo.rs")), Some("rust"));
        assert_eq!(lang_from_ext(Path::new("foo.ts")), Some("typescript"));
        assert_eq!(lang_from_ext(Path::new("foo.tsx")), Some("typescript"));
        assert_eq!(lang_from_ext(Path::new("foo.js")), Some("javascript"));
        assert_eq!(lang_from_ext(Path::new("foo.jsx")), Some("javascript"));
        assert_eq!(lang_from_ext(Path::new("foo.py")), Some("python"));
        assert_eq!(lang_from_ext(Path::new("foo.go")), Some("go"));
        assert_eq!(lang_from_ext(Path::new("foo.txt")), None);
        assert_eq!(lang_from_ext(Path::new("noext")), None);
    }

    #[test]
    fn repo_root_finds_cargo_toml() {
        let dir = TempDir::new().unwrap();
        let cargo = dir.path().join("Cargo.toml");
        fs::write(&cargo, "[package]").unwrap();
        let src = dir.path().join("src");
        fs::create_dir(&src).unwrap();
        let file = src.join("lib.rs");
        fs::write(&file, "").unwrap();
        let found = repo_root(&file).unwrap();
        assert_eq!(found, dir.path());
    }

    fn empty_config() -> LspConfig {
        LspConfig {
            enabled: true,
            timeout_ms: 100,
            servers: std::collections::HashMap::new(),
            ..LspConfig::default()
        }
    }

    #[test]
    fn server_for_lang_built_in_defaults() {
        let reg = LspRegistry::from_config(&empty_config());
        assert_eq!(
            reg.server_for_lang("rust"),
            Some(("rust-analyzer".to_string(), vec![]))
        );
        assert_eq!(
            reg.server_for_lang("typescript"),
            Some((
                "typescript-language-server".to_string(),
                vec!["--stdio".to_string()]
            ))
        );
        // typescript and javascript share the same server.
        assert_eq!(
            reg.server_for_lang("javascript"),
            reg.server_for_lang("typescript")
        );
        assert_eq!(
            reg.server_for_lang("python"),
            Some((
                "pyright-langserver".to_string(),
                vec!["--stdio".to_string()]
            ))
        );
        assert_eq!(
            reg.server_for_lang("go"),
            Some(("gopls".to_string(), vec![]))
        );
        assert_eq!(reg.server_for_lang("cobol"), None);
    }

    #[test]
    fn server_for_lang_config_overrides_default() {
        let mut servers = std::collections::HashMap::new();
        servers.insert(
            "rust".to_string(),
            forge_config::LspServerEntry {
                command: "my-analyzer".to_string(),
                args: vec!["--flag".to_string()],
            },
        );
        let cfg = LspConfig {
            enabled: true,
            timeout_ms: 100,
            servers,
            ..LspConfig::default()
        };
        let reg = LspRegistry::from_config(&cfg);
        assert_eq!(
            reg.server_for_lang("rust"),
            Some(("my-analyzer".to_string(), vec!["--flag".to_string()]))
        );
    }

    #[test]
    fn config_can_add_a_new_language() {
        let mut servers = std::collections::HashMap::new();
        servers.insert(
            "ruby".to_string(),
            forge_config::LspServerEntry {
                command: "solargraph".to_string(),
                args: vec!["stdio".to_string()],
            },
        );
        let cfg = LspConfig {
            enabled: true,
            timeout_ms: 100,
            servers,
            ..LspConfig::default()
        };
        let reg = LspRegistry::from_config(&cfg);
        assert_eq!(
            reg.server_for_lang("ruby"),
            Some(("solargraph".to_string(), vec!["stdio".to_string()]))
        );
    }

    #[test]
    fn repo_root_finds_git_dir() {
        let dir = TempDir::new().unwrap();
        fs::create_dir(dir.path().join(".git")).unwrap();
        let nested = dir.path().join("a").join("b");
        fs::create_dir_all(&nested).unwrap();
        let file = nested.join("x.rs");
        fs::write(&file, "").unwrap();
        assert_eq!(repo_root(&file).unwrap(), dir.path());
    }

    #[test]
    fn repo_root_none_without_marker() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("loose.rs");
        fs::write(&file, "").unwrap();
        // A bare TempDir has no project marker above it within the temp tree.
        assert!(repo_root(&file).is_none());
    }

    #[test]
    fn repo_root_picks_nearest_ancestor() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("Cargo.toml"), "[package]").unwrap();
        let inner = dir.path().join("sub");
        fs::create_dir(&inner).unwrap();
        fs::write(inner.join("package.json"), "{}").unwrap();
        let file = inner.join("app.ts");
        fs::write(&file, "").unwrap();
        // Walks up only to the closest marker, not the outer Cargo.toml.
        assert_eq!(repo_root(&file).unwrap(), inner);
    }

    #[test]
    fn which_resolves_absolute_path_when_present() {
        let dir = TempDir::new().unwrap();
        let bin = dir.path().join("fake-lsp");
        fs::write(&bin, "#!/bin/sh\n").unwrap();
        assert_eq!(which(bin.to_str().unwrap()).unwrap(), bin);
    }

    #[test]
    fn which_absolute_path_missing_is_none() {
        let dir = TempDir::new().unwrap();
        let missing = dir.path().join("does-not-exist");
        assert!(which(missing.to_str().unwrap()).is_none());
    }

    #[test]
    fn which_bare_nonexistent_command_is_none() {
        assert!(which("__forge_definitely_not_a_real_binary_zzz__").is_none());
    }

    #[cfg(unix)]
    #[test]
    fn path_to_uri_absolute_has_file_scheme() {
        let uri = path_to_uri(Path::new("/home/user/project/main.rs"));
        assert_eq!(uri, "file:///home/user/project/main.rs");
    }

    #[cfg(windows)]
    #[test]
    fn path_to_uri_absolute_has_file_scheme() {
        // RFC 8089: a Windows drive path becomes file:///C:/... with forward slashes and a
        // leading slash before the drive letter, never file://C:\... .
        let uri = path_to_uri(Path::new(r"C:\home\user\main.rs"));
        assert_eq!(uri, "file:///C:/home/user/main.rs");
    }

    #[cfg(unix)]
    #[test]
    fn path_to_uri_percent_encodes_reserved_characters() {
        // Spaces and other reserved/unsafe URI characters must be percent-encoded, or the
        // language server's own (encoded) echoed URI in publishDiagnostics never matches ours.
        let uri = path_to_uri(Path::new("/home/user/My Project/a#b%c?.rs"));
        assert_eq!(uri, "file:///home/user/My%20Project/a%23b%25c%3F.rs");
    }

    #[test]
    fn path_to_uri_relative_is_anchored_to_absolute() {
        let uri = path_to_uri(Path::new("rel/file.rs"));
        assert!(uri.starts_with("file:///"), "uri was: {uri}");
        // Output URIs always use forward slashes, regardless of the host separator.
        assert!(uri.ends_with("rel/file.rs"), "uri was: {uri}");
        assert!(
            !uri.contains('\\'),
            "uri must not contain backslashes: {uri}"
        );
    }

    #[serial_test::serial]
    #[tokio::test]
    async fn diagnostics_for_returns_empty_when_no_lang() {
        let cfg = LspConfig {
            enabled: true,
            timeout_ms: 100,
            servers: std::collections::HashMap::new(),
            ..LspConfig::default()
        };
        let reg = LspRegistry::from_config(&cfg);
        let tmp = TempDir::new().unwrap();
        let f = tmp.path().join("test.txt");
        fs::write(&f, "hello").unwrap();
        let diags = reg.diagnostics_for(&f, Duration::from_millis(100)).await;
        assert!(diags.is_empty());
    }

    #[test]
    fn backoff_doubles_and_is_capped() {
        assert_eq!(failure_backoff(0), BACKOFF_BASE);
        assert_eq!(failure_backoff(1), BACKOFF_BASE);
        assert_eq!(failure_backoff(2), BACKOFF_BASE * 2);
        assert_eq!(failure_backoff(3), BACKOFF_BASE * 4);
        assert_eq!(failure_backoff(99), BACKOFF_MAX);
        assert_eq!(failure_backoff(u32::MAX), BACKOFF_MAX);
    }

    #[test]
    #[serial_test::serial]
    fn a_failure_releases_its_global_permit() {
        let mut entry = ServerEntry {
            permit: Some(global_permits().clone().try_acquire_owned().unwrap()),
            ..Default::default()
        };
        entry.server = None;
        entry.record_failure(Instant::now());
        assert!(entry.permit.is_none());
    }

    #[test]
    fn a_success_clears_the_cooldown() {
        let now = Instant::now();
        let mut entry = ServerEntry::default();
        assert!(!entry.cooling_down(now));
        let first = entry.record_failure(now);
        assert_eq!(first, BACKOFF_BASE);
        assert!(entry.cooling_down(now));
        assert!(!entry.cooling_down(now + first));
        assert_eq!(entry.record_failure(now), BACKOFF_BASE * 2);
        entry.clear_failure();
        assert!(!entry.cooling_down(now));
        assert_eq!(entry.record_failure(now), BACKOFF_BASE);
    }

    /// Write a fake language server that records every start, fails the handshake until `fixed`
    /// exists, and afterwards answers `initialize` and publishes one diagnostic. Callers must run
    /// it as an argument to `/bin/sh` (see `fake_server_entry`) rather than exec'ing it directly.
    #[cfg(unix)]
    fn write_fake_server(script: &Path, attempts: &Path, fixed: &Path, uri: &str) {
        let body = format!(
            "echo $$ >> {attempts}\n\
             if [ ! -f {fixed} ]; then\n\
             echo \"error: 'rust-analyzer' is not installed for the toolchain\" 1>&2\n\
             exit 1\n\
             fi\n\
             init='{{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{{\"capabilities\":{{}}}}}}'\n\
             diag='{{\"jsonrpc\":\"2.0\",\"method\":\"textDocument/publishDiagnostics\",\"params\":{{\"uri\":\"{uri}\",\"diagnostics\":[{{\"message\":\"boom\",\"severity\":1,\"range\":{{\"start\":{{\"line\":0,\"character\":0}},\"end\":{{\"line\":0,\"character\":1}}}}}}]}}}}'\n\
             printf 'Content-Length: %d\\r\\n\\r\\n%s' ${{#init}} \"$init\"\n\
             printf 'Content-Length: %d\\r\\n\\r\\n%s' ${{#diag}} \"$diag\"\n\
             exec sleep 5\n",
            attempts = attempts.display(),
            fixed = fixed.display(),
            uri = uri,
        );
        fs::write(script, body).unwrap();
    }

    /// A config entry that runs `script` under `/bin/sh`. Exec'ing a file the test process just
    /// wrote races every other test that spawns a child: a concurrent fork duplicates the still-open
    /// write fd, and exec on that inode fails ETXTBSY until that child execs. `sh` only ever *reads*
    /// the path, so no such race applies.
    #[cfg(unix)]
    fn fake_server_entry(script: &Path) -> forge_config::LspServerEntry {
        forge_config::LspServerEntry {
            command: "/bin/sh".to_string(),
            args: vec![script.to_string_lossy().into_owned()],
        }
    }

    #[cfg(unix)]
    fn starts_recorded(attempts: &Path) -> usize {
        fs::read_to_string(attempts)
            .map(|t| t.lines().count())
            .unwrap_or(0)
    }

    #[cfg(unix)]
    fn started_pids(attempts: &Path) -> Vec<u32> {
        fs::read_to_string(attempts)
            .unwrap_or_default()
            .lines()
            .filter_map(|pid| pid.parse().ok())
            .collect()
    }

    #[cfg(unix)]
    fn process_is_alive(pid: u32) -> bool {
        std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    }

    #[cfg(unix)]
    async fn wait_for_process_exit(pid: u32) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
        while process_is_alive(pid) && tokio::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    #[cfg(unix)]
    async fn wait_for_starts(attempts: &Path, expected: usize) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
        while starts_recorded(attempts) < expected && tokio::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// A server that cannot start (the reported `rust-analyzer` component was missing) must not be
    /// respawned once per diagnostic request, and must recover on its own once it is repaired.
    #[cfg(unix)]
    #[serial_test::serial]
    #[tokio::test]
    async fn a_failing_server_backs_off_then_recovers_after_the_cooldown() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("Cargo.toml"), "[package]").unwrap();
        let file = tmp.path().join("main.rs");
        fs::write(&file, "fn main() {}").unwrap();
        let attempts = tmp.path().join("attempts");
        let fixed = tmp.path().join("fixed");
        let script = tmp.path().join("fake-lsp.sh");
        write_fake_server(&script, &attempts, &fixed, &path_to_uri(&file));

        let mut servers = std::collections::HashMap::new();
        servers.insert("rust".to_string(), fake_server_entry(&script));
        let reg = LspRegistry::from_config(&LspConfig {
            enabled: true,
            timeout_ms: 500,
            servers,
            ..LspConfig::default()
        });
        let timeout = Duration::from_millis(500);

        let unavailable = reg.diagnostics_for(&file, timeout).await;
        assert_eq!(
            unavailable[0].code.as_deref(),
            Some("forge-lsp-unavailable")
        );
        assert!(unavailable[0].message.contains("initialization failed"));
        wait_for_starts(&attempts, 1).await;
        assert_eq!(starts_recorded(&attempts), 1);

        // Still inside the cooldown: no second process, but keep the stale-analysis signal.
        let unavailable = reg.diagnostics_for(&file, timeout).await;
        assert_eq!(
            unavailable[0].code.as_deref(),
            Some("forge-lsp-unavailable")
        );
        assert!(unavailable[0].message.contains("recovering"));
        assert_eq!(
            starts_recorded(&attempts),
            1,
            "cooldown must suppress the respawn"
        );

        // The toolchain is repaired and the cooldown elapses: diagnostics come back by themselves.
        fs::write(&fixed, "").unwrap();
        reg.expire_cooldowns().await;
        let diags = reg.diagnostics_for(&file, timeout).await;
        wait_for_starts(&attempts, 2).await;
        assert_eq!(
            starts_recorded(&attempts),
            2,
            "the cooldown must expire, not latch"
        );
        assert_eq!(diags.len(), 1, "diagnostics were: {diags:?}");
        assert_eq!(diags[0].message, "boom");

        // The healthy server is reused rather than restarted (the fake publishes only once, so
        // the second request's result is empty — what matters is that no third process started).
        let _ = reg.diagnostics_for(&file, timeout).await;
        assert_eq!(
            starts_recorded(&attempts),
            2,
            "a live server must be reused"
        );
    }

    /// An idle server is reaped by its own timer; no later diagnostics call is needed to trigger it.
    #[cfg(unix)]
    #[serial_test::serial]
    #[tokio::test]
    async fn idle_server_is_terminated_without_a_follow_up_request() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("Cargo.toml"), "[package]").unwrap();
        let file = tmp.path().join("main.rs");
        fs::write(&file, "fn main() {}").unwrap();
        let attempts = tmp.path().join("attempts");
        let fixed = tmp.path().join("fixed");
        fs::write(&fixed, "").unwrap();
        let script = tmp.path().join("fake-lsp.sh");
        write_fake_server(&script, &attempts, &fixed, &path_to_uri(&file));
        let mut servers = std::collections::HashMap::new();
        servers.insert("rust".to_string(), fake_server_entry(&script));
        let reg = LspRegistry::from_config(&LspConfig {
            enabled: true,
            timeout_ms: 500,
            servers,
            ..LspConfig::default()
        });
        let _ = reg.diagnostics_for(&file, Duration::from_millis(500)).await;
        wait_for_starts(&attempts, 1).await;
        let pid = started_pids(&attempts)[0];
        let key = ("rust".to_string(), tmp.path().canonicalize().unwrap());
        let entry = reg.servers.lock().await.get(&key).unwrap().clone();
        {
            let mut slot = entry.lock().await;
            let generation = slot.idle_generation;
            arm_idle_timer(
                &mut slot,
                entry.clone(),
                generation,
                Duration::from_millis(25),
            );
        }
        wait_for_process_exit(pid).await;
        assert!(
            !process_is_alive(pid),
            "idle analyzer must be terminated by its timer"
        );
    }

    #[cfg(unix)]
    #[serial_test::serial]
    #[tokio::test]
    async fn diagnostic_timeout_keeps_a_healthy_server_warm() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("Cargo.toml"), "[package]").unwrap();
        let file = tmp.path().join("main.rs");
        fs::write(&file, "fn main() {}").unwrap();
        let attempts = tmp.path().join("attempts");
        let fixed = tmp.path().join("fixed");
        fs::write(&fixed, "").unwrap();
        let script = tmp.path().join("fake-lsp.sh");
        write_fake_server(&script, &attempts, &fixed, &path_to_uri(&file));
        let mut servers = std::collections::HashMap::new();
        servers.insert("rust".to_string(), fake_server_entry(&script));
        let reg = LspRegistry::from_config(&LspConfig {
            enabled: true,
            timeout_ms: 20,
            servers,
            ..LspConfig::default()
        });

        let first = reg.diagnostics_for(&file, Duration::from_millis(20)).await;
        assert_eq!(first.len(), 1);
        let diagnostics = reg.diagnostics_for(&file, Duration::from_millis(20)).await;

        assert!(diagnostics.is_empty());
        let key = ("rust".to_string(), tmp.path().canonicalize().unwrap());
        let entry = reg.servers.lock().await.get(&key).unwrap().clone();
        let slot = entry.lock().await;
        assert!(
            slot.server.is_some(),
            "a timeout must not kill a live server"
        );
        assert!(
            slot.retry_at.is_none(),
            "a timeout must not trigger backoff"
        );
    }

    /// The live-process cap is process-wide, not one allowance per session registry.
    #[cfg(unix)]
    #[serial_test::serial]
    #[tokio::test]
    async fn live_servers_are_bounded_across_registries() {
        let tmp = TempDir::new().unwrap();
        let attempts = tmp.path().join("attempts");
        let fixed = tmp.path().join("fixed");
        fs::write(&fixed, "").unwrap();
        let script = tmp.path().join("fake-lsp.sh");
        write_fake_server(&script, &attempts, &fixed, "file:///unused.rs");
        let mut registries = Vec::new();
        for index in 0..=MAX_SERVER_SLOTS {
            let root = tmp.path().join(format!("registry-root-{index}"));
            fs::create_dir(&root).unwrap();
            fs::write(root.join("Cargo.toml"), "[package]").unwrap();
            let file = root.join("main.rs");
            fs::write(&file, "fn main() {}").unwrap();
            let mut servers = std::collections::HashMap::new();
            servers.insert("rust".to_string(), fake_server_entry(&script));
            let reg = Arc::new(LspRegistry::from_config(&LspConfig {
                enabled: true,
                timeout_ms: 200,
                servers,
                ..LspConfig::default()
            }));
            let _ = reg.diagnostics_for(&file, Duration::from_millis(200)).await;
            registries.push(reg);
        }
        wait_for_starts(&attempts, MAX_LIVE_SERVERS).await;
        let pids = started_pids(&attempts);
        assert_eq!(pids.len(), MAX_LIVE_SERVERS);
        assert!(
            pids.iter().all(|pid| process_is_alive(*pid)),
            "the global permit cap must refuse new analyzers rather than exceed the limit"
        );
    }

    /// A daemon may work across many project roots, but must retain only a fixed number of
    /// heavyweight analyzer processes. Evicted roots are lazy: touching one again can restart it.
    #[cfg(unix)]
    #[serial_test::serial]
    #[tokio::test]
    async fn live_servers_are_bounded_across_project_roots() {
        let tmp = TempDir::new().unwrap();
        let attempts = tmp.path().join("attempts");
        let fixed = tmp.path().join("fixed");
        fs::write(&fixed, "").unwrap();
        let script = tmp.path().join("fake-lsp.sh");
        write_fake_server(&script, &attempts, &fixed, "file:///unused.rs");
        let mut servers = std::collections::HashMap::new();
        servers.insert("rust".to_string(), fake_server_entry(&script));
        let reg = LspRegistry::from_config(&LspConfig {
            enabled: true,
            timeout_ms: 200,
            servers,
            ..LspConfig::default()
        });

        for root_number in 0..=MAX_SERVER_SLOTS {
            let root = tmp.path().join(format!("root-{root_number}"));
            fs::create_dir(&root).unwrap();
            fs::write(root.join("Cargo.toml"), "[package]").unwrap();
            let file = root.join("main.rs");
            fs::write(&file, "fn main() {}").unwrap();
            let _ = reg.diagnostics_for(&file, Duration::from_millis(200)).await;
        }

        assert_eq!(reg.server_slot_count().await, MAX_SERVER_SLOTS);
        let pids = started_pids(&attempts);
        assert_eq!(pids.len(), MAX_LIVE_SERVERS + 1);
        wait_for_process_exit(pids[0]).await;
        assert!(
            !process_is_alive(pids[0]),
            "the evicted server process must be killed, not retained by an idle timer"
        );
    }

    /// The cause a bare "server closed stdout" hides must survive to the failure path.
    #[cfg(unix)]
    #[serial_test::serial]
    #[tokio::test]
    async fn a_dying_server_reports_its_stderr() {
        let tmp = TempDir::new().unwrap();
        let attempts = tmp.path().join("attempts");
        let script = tmp.path().join("fake-lsp.sh");
        write_fake_server(&script, &attempts, &tmp.path().join("never"), "file:///x");

        let mut srv = LspServer::spawn("/bin/sh", &[script.to_string_lossy().into_owned()])
            .await
            .unwrap();
        let err = srv
            .initialize("file:///tmp", Duration::from_millis(500))
            .await
            .expect_err("the fake server exits before answering");
        assert!(
            matches!(
                err.kind(),
                std::io::ErrorKind::UnexpectedEof | std::io::ErrorKind::BrokenPipe
            ),
            "unexpected initialize failure: {err}"
        );
        let cause = srv.stderr_summary().await;
        assert!(
            cause.contains("'rust-analyzer' is not installed"),
            "stderr summary was: {cause:?}"
        );
    }

    #[serial_test::serial]
    #[tokio::test]
    async fn diagnostics_for_reports_when_binary_not_found() {
        let mut servers = std::collections::HashMap::new();
        servers.insert(
            "rust".to_string(),
            forge_config::LspServerEntry {
                command: "__forge_lsp_nonexistent_binary_xyz__".to_string(),
                args: vec![],
            },
        );
        let cfg = LspConfig {
            enabled: true,
            timeout_ms: 100,
            servers,
            ..LspConfig::default()
        };
        let reg = LspRegistry::from_config(&cfg);
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("Cargo.toml"), "[package]").unwrap();
        let f = tmp.path().join("main.rs");
        fs::write(&f, "fn main() {}").unwrap();
        let diags = reg.diagnostics_for(&f, Duration::from_millis(100)).await;
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code.as_deref(), Some("forge-lsp-unavailable"));
        assert!(diags[0].message.contains("not available on PATH"));
    }
}
