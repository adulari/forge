//! Local SQLite persistence (ADR-0005), via rusqlite with bundled SQLite. The store owns
//! a single connection behind a mutex; SQLite is in WAL mode for crash-resilient writes.
//! All persistence in Forge goes through this crate.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Datelike, Duration as ChronoDuration, Local, TimeZone};
use forge_types::{Role, TaskTier, ToolCall, Usage, Visibility};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior};
use sha2::{Digest, Sha256};

mod assay_store;
mod checkpoint_store;
mod duel_store;
mod handoff;
mod handoff_types;
mod lattice_store;
mod memory;
mod migrations;
mod model_health_store;
use migrations::{
    add_column_if_missing, migrate_subscription_usage, run_migrations, seed_singleton_rows,
};
#[cfg(test)]
use migrations::{
    migration_0001, ANYWHERE_PRERELEASE_MAX_VERSION, ANYWHERE_PRERELEASE_MIN_VERSION, MIGRATIONS,
};
mod quota_store;
mod schema;
mod session_usage_store;
mod spending_store;
mod sync_files_store;
mod sync_history_store;
mod sync_journal;
mod sync_store;
mod task_store;
mod usage_store;

pub use handoff_types::{
    HandoffCheckpoint, HandoffImportProvenance, HandoffMessage, HandoffSessionExport,
    HandoffSessionImport, ImportedSessionMetadata,
};
pub use memory::Memory;

/// Current schema version this build understands. Bumped whenever a new entry is added to
/// [`migrations::MIGRATIONS`]; persisted in the DB via `PRAGMA user_version`. A DB whose `user_version`
/// exceeds this (written by a NEWER Forge) is refused, rather than silently misread.
const SCHEMA_VERSION: i64 = 19;

/// Max attempts a critical write makes when SQLite reports the database is busy/locked. The single
/// WAL writer lock can be briefly held by another connection (TUI vs mcp-serve, or the indexer);
/// `busy_timeout` covers ordinary lock waits but NOT `SQLITE_BUSY_SNAPSHOT`, so we retry the whole
/// transaction a bounded number of times with a short backoff rather than dropping the row.
const BUSY_RETRY_MAX: u32 = 8;

/// How large `forge.db-wal` may remain after a checkpoint.
///
/// WAL is reused in place: SQLite's passive autocheckpoint (default ~1000 pages) copies committed
/// frames into the main database but leaves the `-wal` file at its high-water mark, and the
/// last-connection-close checkpoint+delete never fires while `forge serve` holds a connection open.
/// One outsized write transaction — a lattice PageRank pass, a bulk prune — therefore pins hundreds
/// of megabytes of disk for the life of the install (656 MB observed on a 2.8 GB database).
/// `journal_size_limit` makes each checkpoint truncate the file back down to this size.
///
/// Deliberately set FAR above the autocheckpoint threshold (~4 MB at the default page size): in
/// steady state the WAL never reaches 64 MB, so the truncate never runs and no checkpoint pays for
/// it. It only fires after an abnormal spike — exactly when the space should come back. Reclaiming
/// the *database* file is a separate, far more expensive operation that must stay explicit; see
/// [`Store::vacuum`].
///
/// The pragma is per-connection and is NOT persisted in the file, so it has to be applied on every
/// connection the pool opens rather than once at migration time.
const WAL_SIZE_LIMIT_BYTES: i64 = 64 * 1024 * 1024;

/// Oversized `tool_call.args_json`/`result_json` (e.g. a full large-file read or write) is truncated
/// to this many bytes at insert time, with a marker. Keeps the append-only global DB from growing
/// without bound while preserving the head of the args/output for audit/replay.
const MAX_RESULT_JSON_BYTES: usize = 64 * 1024;

/// How long a `workflow_run` row may sit in `running` before a reader treats it as `interrupted`.
/// A run is closed out by the process that opened it (normally, on Esc, and on turn abort — see
/// `WorkflowRunGuard` in forge-core), so only a hard crash/kill can leave one open; without a
/// horizon such a row would claim "running" forever. Deliberately far longer than any real
/// workflow: agent concurrency, `mesh.workflows.max_total_agents` and the budget cap all bound a
/// run, and calling a live one dead would be the worse lie.
const WORKFLOW_RUN_STALE_SECS: i64 = 12 * 60 * 60;

/// Max characters of a tool call's arguments carried in a synthesized call row's content. Long
/// enough to identify the call (path, command, pattern), short enough that a `write_file` body or
/// a pasted document can never ride a transcript row. See [`tool_call_args_summary`].
const MAX_CALL_ARGS_CHARS: usize = 200;

/// Max characters of any single string VALUE inside those arguments. Applied before the whole-
/// summary cap so one huge field can't crowd out the other keys — the shape of the call stays
/// readable even when one argument is a file body.
const MAX_CALL_ARG_VALUE_CHARS: usize = 80;

/// Default retention horizon: sessions untouched for longer than this are eligible for opportunistic
/// pruning (cascading to their messages/usage/routing/tool_calls/live_events). ~90 days.
pub const RETENTION_HORIZON_SECS: i64 = 90 * 24 * 60 * 60;

/// How many old sessions a single opportunistic [`Store::prune`] pass removes — bounded so the prune
/// piggy-backed on session open stays cheap.
const PRUNE_BATCH: usize = 50;

/// How long a session with zero real (user) messages is kept before being eligible for
/// [`Store::prune_empty`] — much shorter than [`RETENTION_HORIZON_SECS`] since an empty session
/// carries nothing worth retaining. Long enough that the session currently being opened (which
/// hasn't sent its first message yet) is never swept out from under itself.
const EMPTY_SESSION_HORIZON_SECS: i64 = 10 * 60;

/// Same cap rationale as [`PRUNE_BATCH`], applied to the empty-session sweep.
const EMPTY_PRUNE_BATCH: usize = 200;

/// Run live-event ring-buffer pruning only once every this many appends, instead of on every insert
/// (the old per-insert correlated-subquery DELETE was O(n) on a hot path).
const LIVE_EVENT_PRUNE_EVERY: u64 = 256;

/// Max live events kept per session (ring buffer). The actual count drifts up to this plus at most
/// [`LIVE_EVENT_PRUNE_EVERY`] between prunes.
const LIVE_EVENT_KEEP: i64 = 2000;

/// Whether a rusqlite error is a transient busy/locked condition worth retrying (covers plain
/// `SQLITE_BUSY`, `SQLITE_BUSY_SNAPSHOT`, and `SQLITE_LOCKED`).
fn is_busy(e: &rusqlite::Error) -> bool {
    matches!(
        e.sqlite_error_code(),
        Some(rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked)
    )
}

/// Whether a rusqlite error is specifically a UNIQUE / PRIMARY KEY constraint violation (so the seq
/// allocator can retry with the next seq, without also catching unrelated FK/NOT NULL violations).
fn is_unique_violation(e: &rusqlite::Error) -> bool {
    matches!(
        e,
        rusqlite::Error::SqliteFailure(err, _)
            if err.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE
                || err.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_PRIMARYKEY
    )
}

/// Run a critical write, retrying the WHOLE closure on a transient busy/locked error up to
/// [`BUSY_RETRY_MAX`] times with a short exponential backoff. Each attempt re-acquires its
/// connection and re-runs its transaction from scratch (a failed IMMEDIATE txn is rolled back on
/// drop), so a transcript/usage row isn't lost just because another writer briefly held the lock.
fn with_busy_retry<T>(mut f: impl FnMut() -> Result<T>) -> Result<T> {
    let mut attempt = 0u32;
    loop {
        let r = f();
        if attempt < BUSY_RETRY_MAX {
            if let Err(StoreError::Sqlite(ref e)) = r {
                if is_busy(e) {
                    let backoff = 2u64.saturating_pow(attempt.min(6));
                    std::thread::sleep(std::time::Duration::from_millis(5 * backoff));
                    attempt += 1;
                    continue;
                }
            }
        }
        return r;
    }
}

/// Truncate an oversized tool args/result string to [`MAX_RESULT_JSON_BYTES`] on a char boundary,
/// appending a marker noting how many bytes were elided. Returns the input unchanged when within
/// the cap.
fn cap_result_json(s: &str) -> std::borrow::Cow<'_, str> {
    if s.len() <= MAX_RESULT_JSON_BYTES {
        return std::borrow::Cow::Borrowed(s);
    }
    let mut end = MAX_RESULT_JSON_BYTES;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    std::borrow::Cow::Owned(format!("{}…[truncated {} bytes]", &s[..end], s.len() - end))
}

/// Pull the top-level `"path"` string out of a tool call's `args_json`, for `forge blame`
/// (docs/features/forge-blame.md). Cheap best-effort: only `write_file`/`edit_file` carry a
/// `path` arg today, but this is generic over any tool's args so it doesn't need updating if
/// another file-touching tool is added. Returns `None` on unparseable/truncated JSON or a
/// missing/non-string `path` key, rather than erroring the caller.
fn extract_path_arg(args_json: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(args_json)
        .ok()?
        .get("path")?
        .as_str()
        .map(str::to_string)
}

/// Escape `\`, `%`, and `_` in a caller-supplied string so it can be safely embedded in a SQL
/// `LIKE` pattern (with `ESCAPE '\'`) as literal text rather than as wildcards.
fn escape_like_pattern(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

/// How long a permanently-failed model (a [`Store::exclude_model`] capability exclusion) stays out
/// of routing before it's re-probed: 24 hours. Long enough to stop the per-session churn of
/// re-trying models that can't do tool calling, short enough that a transient misclassification or
/// a provider adding support is picked up the next day (was 7 days — too sticky).
const CAPABILITY_EXCLUSION_SECS: i64 = 24 * 60 * 60;

/// Half-open `[start, end)` epoch-second bounds of `now`'s **local** calendar day. Computed
/// in Rust (not SQLite `strftime`) so the day rolls at the user's midnight and survives DST.
pub fn day_bounds_local(now: DateTime<Local>) -> (i64, i64) {
    let midnight = now
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .expect("valid midnight");
    let start = Local
        .from_local_datetime(&midnight)
        .earliest()
        .unwrap_or(now);
    let end = start + ChronoDuration::days(1);
    (start.timestamp(), end.timestamp())
}

/// Half-open `[start, end)` covering the last `hours` hours ending at `now`.
pub fn rolling_hours_bounds(now: DateTime<Local>, hours: i64) -> (i64, i64) {
    let end = now.timestamp() + 1;
    let start = end - hours * 3600;
    (start, end)
}

/// Half-open `[start, end)` epoch-second bounds of `now`'s **local** ISO calendar week
/// (Monday 00:00 local → 7 days later).
pub fn week_bounds_local(now: DateTime<Local>) -> (i64, i64) {
    use chrono::Datelike;
    let days_since_monday = now.weekday().num_days_from_monday() as i64;
    let monday = now.date_naive() - ChronoDuration::days(days_since_monday);
    let start = Local
        .from_local_datetime(&monday.and_hms_opt(0, 0, 0).expect("valid midnight"))
        .earliest()
        .unwrap_or(now);
    let end = start + ChronoDuration::weeks(1);
    (start.timestamp(), end.timestamp())
}

/// Half-open `[start, end)` epoch-second bounds of `now`'s **local** calendar month.
pub fn month_bounds_local(now: DateTime<Local>) -> (i64, i64) {
    let first = now
        .date_naive()
        .with_day(1)
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .expect("valid first-of-month");
    let start = Local.from_local_datetime(&first).earliest().unwrap_or(now);
    let next_first = if first.month() == 12 {
        first
            .with_year(first.year() + 1)
            .and_then(|d| d.with_month(1))
    } else {
        first.with_month(first.month() + 1)
    }
    .expect("valid next month");
    let end = Local
        .from_local_datetime(&next_first)
        .earliest()
        .unwrap_or(now);
    (start.timestamp(), end.timestamp())
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error("connection pool: {0}")]
    Pool(String),
    #[error("portable metadata JSON: {0}")]
    Json(String),
    #[error("invalid store value: {0}")]
    InvalidValue(String),
    #[error(
        "database schema version {found} is newer than this build supports ({supported}); \
         upgrade Forge to open it"
    )]
    SchemaTooNew { found: i64, supported: i64 },
}

type Result<T> = std::result::Result<T, StoreError>;

/// Tables safe to carry in a `forge migrate` bundle: model metadata only (cooldowns, context
/// windows, pricing). Deliberately EXCLUDES every session/message/usage/routing/lattice table so a
/// metadata export can never leak private history. The set is an allow-list on both export and
/// import — a tampered bundle naming other tables is ignored.
const PORTABLE_METADATA_TABLES: &[&str] = &["model_health", "model_context", "model_pricing"];

/// SQLite value (as read via `get_ref`) → JSON, for the portable-metadata dump.
fn value_ref_to_json(v: rusqlite::types::ValueRef<'_>) -> serde_json::Value {
    use rusqlite::types::ValueRef;
    match v {
        ValueRef::Null => serde_json::Value::Null,
        ValueRef::Integer(i) => serde_json::Value::from(i),
        ValueRef::Real(f) => serde_json::Value::from(f),
        ValueRef::Text(t) => serde_json::Value::from(String::from_utf8_lossy(t).into_owned()),
        ValueRef::Blob(b) => serde_json::Value::from(String::from_utf8_lossy(b).into_owned()),
    }
}

/// JSON → SQLite bind value, the inverse of [`value_ref_to_json`] for the portable-metadata import.
fn json_to_sql_value(v: &serde_json::Value) -> rusqlite::types::Value {
    use rusqlite::types::Value;
    match v {
        serde_json::Value::Null => Value::Null,
        serde_json::Value::Bool(b) => Value::Integer(*b as i64),
        serde_json::Value::Number(n) if n.is_i64() => Value::Integer(n.as_i64().unwrap()),
        serde_json::Value::Number(n) => Value::Real(n.as_f64().unwrap_or(0.0)),
        serde_json::Value::String(s) => Value::Text(s.clone()),
        other => Value::Text(other.to_string()),
    }
}

/// A fetched per-model price row: `(model, input_per_1k, output_per_1k, cache_read_per_1k)` in USD.
pub type ModelPriceRow = (String, f64, f64, Option<f64>);

/// Forge-daemon-process-wide active completion reservations keyed by durable store identity and
/// model. Stores opened over the same database coordinate; separate databases and in-memory stores
/// remain isolated. It intentionally does not coordinate separate Forge processes; cross-process
/// leases are a later wave-two concern.
fn in_flight_models() -> &'static std::sync::Mutex<std::collections::HashSet<(String, String)>> {
    static MODELS: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashSet<(String, String)>>,
    > = std::sync::OnceLock::new();
    MODELS.get_or_init(|| std::sync::Mutex::new(std::collections::HashSet::new()))
}

fn next_memory_store_id() -> String {
    static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    format!(
        "memory:{}",
        NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    )
}

/// An active per-model completion reservation. Dropping it makes the model eligible for another
/// concurrent session.
pub struct ModelReservation {
    store_id: String,
    model: String,
}

impl Drop for ModelReservation {
    fn drop(&mut self) {
        if let Ok(mut in_flight) = in_flight_models().lock() {
            in_flight.remove(&(self.store_id.clone(), self.model.clone()));
        }
    }
}

pub struct Store {
    pool: r2d2::Pool<SqliteManager>,
    reservation_store_id: String,
    /// Append counter for `live_event`, so the ring-buffer prune runs once every
    /// [`LIVE_EVENT_PRUNE_EVERY`] inserts instead of on every append (the old per-insert
    /// correlated-subquery DELETE was O(n) on a hot path).
    live_event_writes: std::sync::atomic::AtomicU64,
}

/// SQL fragment: derives a usage row's provider from its message model (aliased `m`).
///
/// Ordinary completions retain their routed model. Synthetic side-call messages (compact,
/// diagnose) have no model, so inherit the most recent routed model in their session instead of
/// appearing in a misleading shared `other` bucket. The earliest following model covers legacy
/// rows that precede every routed turn; a session with no routed model still falls back to `other`.
///
/// Keep outer-row references in each subquery's `WHERE` clause. SQLite 3.51 rejects a correlated
/// outer column used directly by a scalar subquery's `ORDER BY` (`no such column: m.seq`).
const USAGE_PROVIDER_EXPR: &str = "COALESCE(NULLIF(CASE WHEN instr(m.model, '::') > 0 THEN substr(m.model, 1, instr(m.model, '::') - 1) ELSE m.model END, ''), (SELECT CASE WHEN instr(pm.model, '::') > 0 THEN substr(pm.model, 1, instr(pm.model, '::') - 1) ELSE pm.model END FROM message pm WHERE pm.session_id = m.session_id AND pm.model IS NOT NULL AND pm.seq <= m.seq ORDER BY pm.seq DESC LIMIT 1), (SELECT CASE WHEN instr(pm.model, '::') > 0 THEN substr(pm.model, 1, instr(pm.model, '::') - 1) ELSE pm.model END FROM message pm WHERE pm.session_id = m.session_id AND pm.model IS NOT NULL AND pm.seq > m.seq ORDER BY pm.seq ASC LIMIT 1), 'other')";

#[derive(Debug, Clone, PartialEq)]
pub struct ProviderUsage {
    pub provider: String,
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SubscriptionWindow {
    pub provider: String,
    pub window_kind: String,
    pub status: String,
    pub resets_at: Option<i64>,
    pub fraction: Option<f64>,
}

/// How the pool opens a fresh connection. `:memory:` makes a DISTINCT empty DB on every open, so an
/// in-memory pool is pinned to a single never-recycled connection (see [`Store::build`]).
#[derive(Clone)]
enum ConnSource {
    File(std::path::PathBuf),
    Memory,
}

/// An [`r2d2::ManageConnection`] over OUR `rusqlite` 0.40. Hand-rolled instead of pulling
/// `r2d2_sqlite`, which pins an older `rusqlite`/`libsqlite3-sys` and would link a SECOND bundled
/// SQLite (symbol clash). Applies the per-connection pragmas (busy_timeout, foreign_keys) every time
/// the pool opens a connection, so a pooled read carries the same settings the old single conn did.
struct SqliteManager {
    source: ConnSource,
}

impl r2d2::ManageConnection for SqliteManager {
    type Connection = Connection;
    type Error = rusqlite::Error;

    fn connect(&self) -> std::result::Result<Connection, rusqlite::Error> {
        let conn = match &self.source {
            ConnSource::File(p) => Connection::open(p)?,
            ConnSource::Memory => Connection::open_in_memory()?,
        };
        // WAL still allows only one writer; without a busy_timeout a concurrent writer (the TUI vs
        // the mcp-serve bridge, or now two pooled connections) hits SQLITE_BUSY immediately.
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        // Performance pragmas — safe with WAL mode:
        //   synchronous=NORMAL: WAL already guarantees crash recovery; FULL adds extra fsyncs
        //   with no benefit here. Reduces write latency on every INSERT/UPDATE.
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        //   32 MB page cache (default ~2 MB) — cuts disk reads for hot queries like spend_summary
        //   and load_messages on large sessions.
        conn.pragma_update(None, "cache_size", -32_000_i64)?;
        //   Sort/group-by temp tables in memory — no tmp file for our aggregation queries.
        conn.pragma_update(None, "temp_store", "MEMORY")?;
        // Bound the WAL. Without this SQLite reuses the -wal file in place after every checkpoint
        // and it never shrinks again; see [`WAL_SIZE_LIMIT_BYTES`]. Per-connection and not
        // persisted, hence set here rather than once during migration.
        conn.pragma_update(None, "journal_size_limit", WAL_SIZE_LIMIT_BYTES)?;
        Ok(conn)
    }

    fn is_valid(&self, conn: &mut Connection) -> std::result::Result<(), rusqlite::Error> {
        conn.execute_batch("SELECT 1")
    }

    fn has_broken(&self, _conn: &mut Connection) -> bool {
        false
    }
}

/// One stored Web Push subscription (the browser's `PushSubscription.toJSON()` fields):
/// `endpoint` is the vendor push URL, `p256dh`/`auth` are the RFC 8291 client keys, both
/// base64url as handed out by the browser. Deduplicated by endpoint on write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushSubscription {
    pub id: String,
    pub endpoint: String,
    pub p256dh: String,
    pub auth: String,
}

/// One stored APNs (native iOS) push subscription: a device token plus which APNs environment
/// it belongs to ("sandbox" for Xcode/TestFlight debug builds, "production" for App Store
/// builds) — Apple routes each to a different host, and a token from one is rejected by the
/// other. Deduplicated by `device_token` on write, mirroring [`PushSubscription`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApnsSubscription {
    pub id: String,
    pub device_token: String,
    pub environment: String,
}

/// The Live Activity remote-update push token for one session's ActivityKit activity (see
/// migration_0010). Keyed by session, not deduplicated by token — starting a new Live Activity
/// for the same session replaces the old token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveActivityToken {
    pub session_id: String,
    pub push_token: String,
    pub environment: String,
}

/// Provider aliases that bill the SAME underlying subscription account, so their
/// `subscription_usage`/`quota_history` rows must be read as one shared bucket (never summed) —
/// see [`Store::quota_at`]. `codex-cli` (the Codex CLI bridge) and `codex-oauth` (Forge's direct
/// ChatGPT OAuth provider) both draw on one ChatGPT account's server-reported usage.
const QUOTA_ALIAS_GROUPS: &[&[&str]] = &[&["codex-cli", "codex-oauth"]];

/// Every provider `p` should be treated as equivalent to for quota purposes: the full alias group
/// containing `p`, or just `[p]` when it isn't in any group (the common case — a no-op merge).
fn quota_alias_members(provider: &str) -> Vec<&str> {
    for group in QUOTA_ALIAS_GROUPS {
        if group.contains(&provider) {
            return group.to_vec();
        }
    }
    vec![provider]
}

/// Codex OAuth and the Codex CLI spend the same ChatGPT subscription outside Forge as well as
/// inside it. Their usage snapshots therefore expire quickly; retaining an old high-water mark
/// would incorrectly make the mesh avoid both surfaces. Other providers retain their existing
/// reset-window semantics.
fn codex_quota_is_fresh(provider: &str, updated_at: i64, now: i64) -> bool {
    !matches!(provider, "codex-oauth" | "codex-cli")
        || now.saturating_sub(updated_at) <= forge_types::CODEX_QUOTA_FRESHNESS_SECS
}

fn quota_status_from_str(status: &str) -> forge_types::QuotaStatus {
    match status {
        "exhausted" => forge_types::QuotaStatus::Exhausted,
        "warning" => forge_types::QuotaStatus::Warning,
        _ => forge_types::QuotaStatus::Ok,
    }
}

/// One model-call outcome recorded by the Mesh.  This deliberately excludes user content and
/// response text; it is operational telemetry stored locally with the session metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeshOutcome {
    pub session_id: String,
    pub model: String,
    pub tier: TaskTier,
    pub started_at: i64,
    pub completed_at: i64,
    pub latency_ms: u64,
    pub outcome: String,
    pub error_kind: Option<String>,
    pub failover_hop: u32,
    pub tool_calls: u32,
    pub verified_completion: bool,
}

/// Bounded aggregate evidence used by Mesh as a small tie-breaker after enough observations.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelOutcomeCalibration {
    pub model: String,
    pub samples: u32,
    pub success_rate: f64,
    pub mean_latency_ms: f64,
}

/// One pending local record for the encrypted Anywhere sync worker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncJournalEntry {
    pub id: i64,
    pub record_kind: String,
    pub stable_id: String,
    pub operation: String,
    pub revision: u64,
    pub logical_clock: u64,
    pub base_hash: Option<[u8; 32]>,
    pub content_hash: Vec<u8>,
    /// Immutable plaintext snapshot; the sync worker encrypts it before it leaves the host.
    pub payload: Vec<u8>,
    pub created_at: i64,
}

/// Durable ciphertext associated with one pending sync journal revision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncUploadEnvelope {
    pub envelope: Vec<u8>,
    pub ciphertext_sha256: [u8; 32],
}

/// One authenticated and decrypted change staged from the Anywhere service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteSyncRecord {
    pub cursor: i64,
    pub sender_device_id: [u8; 16],
    pub record_kind: String,
    pub stable_id: String,
    pub operation: String,
    pub revision: u64,
    pub logical_clock: u64,
    pub base_hash: Option<[u8; 32]>,
    pub content_hash: [u8; 32],
    pub payload: Vec<u8>,
}

/// Outcomes from one bounded pass applying staged mutable records to local primary tables.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RemoteSyncApplySummary {
    pub inspected: usize,
    pub applied: usize,
    pub superseded: usize,
    pub conflicts: usize,
    pub deferred: usize,
}

/// One safely materialized account record used by local settings/command/extension consumers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortableSyncRecord {
    pub record_kind: String,
    pub stable_id: String,
    pub payload: Vec<u8>,
    pub deleted: bool,
    pub logical_clock: u64,
    pub sender_device_id: [u8; 16],
    pub content_hash: [u8; 32],
}

/// A content-divergent file revision retained instead of overwriting the local winner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncFileConflict {
    pub stable_id: String,
    pub sender_device_id: [u8; 16],
    pub base_hash: Option<[u8; 32]>,
    pub content_hash: [u8; 32],
    pub payload: Vec<u8>,
    pub detail: String,
}

/// A terminal sync conflict retained for status/UI reporting and explicit resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncApplyConflict {
    pub cursor: i64,
    pub record_kind: String,
    pub stable_id: String,
    pub detail: String,
}

#[derive(Debug)]
struct StagedMemoryRecord {
    cursor: i64,
    sender_device_id: [u8; 16],
    stable_id: String,
    operation: String,
    logical_clock: i64,
    content_hash: [u8; 32],
    payload: Vec<u8>,
}

#[derive(Debug, serde::Deserialize)]
struct RemoteMemoryPayload {
    id: String,
    scope: String,
    kind: String,
    text: String,
    source_session: String,
    created_at: i64,
    updated_at: i64,
    salience: f64,
}

#[derive(Debug)]
struct StagedHistoryRecord {
    cursor: i64,
    sender_device_id: [u8; 16],
    record_kind: String,
    stable_id: String,
    operation: String,
    logical_clock: i64,
    content_hash: [u8; 32],
    payload: Vec<u8>,
}

#[derive(Debug, serde::Deserialize)]
struct RemoteSessionPayload {
    id: String,
    title: Option<String>,
    archived: bool,
    view_snapshot: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct RemoteMessagePayload {
    id: String,
    session_id: String,
    seq: i64,
    role: String,
    content: String,
    model: Option<String>,
    #[serde(default)]
    tool_calls: Vec<ToolCall>,
    tool_call_id: Option<String>,
    visibility: String,
    #[serde(default = "default_true")]
    active: bool,
}

#[derive(Debug, serde::Deserialize)]
struct RemoteCheckpointPayload {
    id: String,
    session_id: String,
    label: Option<String>,
    seq: i64,
}

#[derive(Debug, serde::Deserialize)]
struct RemoteToolCallPayload {
    id: String,
    message_id: String,
    tool_name: String,
    args_json: String,
    result_json: String,
    permission: String,
    status: String,
    path: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct RemoteRoutingPayload {
    id: String,
    message_id: String,
    task_tier: String,
    chosen_model: String,
    rationale: String,
}

#[derive(Debug, serde::Deserialize)]
struct RemoteUsagePayload {
    id: String,
    message_id: String,
    input_tokens: i64,
    #[serde(default)]
    cached_input_tokens: i64,
    output_tokens: i64,
    cost_usd: f64,
}

#[derive(Debug, serde::Deserialize)]
struct RemoteCompactionPayload {
    session_id: String,
    summary: String,
    keep_count: usize,
}

enum HistoryMutation {
    Session(Option<RemoteSessionPayload>),
    Message(RemoteMessagePayload),
    Checkpoint(RemoteCheckpointPayload),
    ToolCall(RemoteToolCallPayload),
    Routing(RemoteRoutingPayload),
    Usage(RemoteUsagePayload),
    Compaction(Option<RemoteCompactionPayload>),
    Tombstone,
}

fn default_true() -> bool {
    true
}

enum MemoryMutation {
    Upsert(RemoteMemoryPayload),
    Tombstone,
}

enum SyncVersionDisposition {
    Superseded,
    Conflict,
}

#[derive(Clone, Copy)]
struct SyncVersion<'a> {
    operation: &'a str,
    logical_clock: i64,
    device_id: [u8; 16],
    content_hash: &'a [u8],
}

/// Operation recorded in the local Anywhere sync outbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncJournalOperation {
    Upsert,
    Tombstone,
}

impl SyncJournalOperation {
    fn as_str(self) -> &'static str {
        match self {
            Self::Upsert => "upsert",
            Self::Tombstone => "tombstone",
        }
    }
}

fn sync_payload_hash(payload: &[u8]) -> [u8; 32] {
    Sha256::digest(payload).into()
}

fn sync_json(value: serde_json::Value) -> Result<Vec<u8>> {
    serde_json::to_vec(&value).map_err(|error| StoreError::Json(error.to_string()))
}

fn parse_memory_mutation(
    record: &StagedMemoryRecord,
) -> std::result::Result<MemoryMutation, String> {
    if sync_payload_hash(&record.payload) != record.content_hash {
        return Err("decrypted payload does not match its authenticated content hash".into());
    }
    match record.operation.as_str() {
        "tombstone" if record.payload.is_empty() => Ok(MemoryMutation::Tombstone),
        "tombstone" => Err("memory tombstone payload must be empty".into()),
        "upsert" => {
            let payload: RemoteMemoryPayload = serde_json::from_slice(&record.payload)
                .map_err(|error| format!("invalid memory snapshot: {error}"))?;
            if payload.id != record.stable_id {
                return Err("memory snapshot id does not match its stable id".into());
            }
            if payload.scope.trim().is_empty()
                || payload.kind.trim().is_empty()
                || payload.text.trim().is_empty()
                || payload.source_session.trim().is_empty()
            {
                return Err("memory snapshot contains an empty required field".into());
            }
            if !payload.salience.is_finite() || !(0.0..=1.0).contains(&payload.salience) {
                return Err("memory snapshot salience must be between zero and one".into());
            }
            Ok(MemoryMutation::Upsert(payload))
        }
        _ => Err("memory operation is not supported".into()),
    }
}

fn parse_history_mutation(
    record: &StagedHistoryRecord,
) -> std::result::Result<HistoryMutation, String> {
    if sync_payload_hash(&record.payload) != record.content_hash {
        return Err("decrypted payload does not match its authenticated content hash".into());
    }
    if matches!(record.record_kind.as_str(), "session" | "compaction") {
        return match record.operation.as_str() {
            "tombstone" if record.payload.is_empty() && record.record_kind == "session" => {
                Ok(HistoryMutation::Session(None))
            }
            "tombstone" if record.payload.is_empty() => Ok(HistoryMutation::Compaction(None)),
            "tombstone" => Err("mutable history tombstone payload must be empty".into()),
            "upsert" => {
                if record.record_kind == "session" {
                    let payload: RemoteSessionPayload = serde_json::from_slice(&record.payload)
                        .map_err(|error| format!("invalid session snapshot: {error}"))?;
                    if payload.id != record.stable_id {
                        return Err("session snapshot id does not match its stable id".into());
                    }
                    Ok(HistoryMutation::Session(Some(payload)))
                } else {
                    let payload: RemoteCompactionPayload = serde_json::from_slice(&record.payload)
                        .map_err(|error| format!("invalid compaction snapshot: {error}"))?;
                    if payload.session_id != record.stable_id {
                        return Err("compaction session id does not match its stable id".into());
                    }
                    Ok(HistoryMutation::Compaction(Some(payload)))
                }
            }
            _ => Err("mutable history operation is not supported".into()),
        };
    }
    if record.operation == "tombstone" {
        return if record.payload.is_empty() {
            Ok(HistoryMutation::Tombstone)
        } else {
            Err("history tombstone payload must be empty".into())
        };
    }
    if record.operation != "upsert" {
        return Err("history operation is not supported".into());
    }
    macro_rules! parse {
        ($ty:ty, $variant:ident, $label:literal) => {{
            let payload: $ty = serde_json::from_slice(&record.payload)
                .map_err(|error| format!(concat!("invalid ", $label, " snapshot: {}"), error))?;
            if payload.id != record.stable_id {
                return Err(concat!($label, " snapshot id does not match its stable id").into());
            }
            HistoryMutation::$variant(payload)
        }};
    }
    Ok(match record.record_kind.as_str() {
        "message" => parse!(RemoteMessagePayload, Message, "message"),
        "checkpoint" => parse!(RemoteCheckpointPayload, Checkpoint, "checkpoint"),
        "tool_call" => parse!(RemoteToolCallPayload, ToolCall, "tool call"),
        "routing_decision" => parse!(RemoteRoutingPayload, Routing, "routing decision"),
        "usage" => parse!(RemoteUsagePayload, Usage, "usage"),
        _ => return Err("history record kind is not supported".into()),
    })
}

fn compare_sync_version(
    remote: SyncVersion<'_>,
    existing: SyncVersion<'_>,
) -> Option<SyncVersionDisposition> {
    match (existing.logical_clock, existing.device_id)
        .cmp(&(remote.logical_clock, remote.device_id))
    {
        std::cmp::Ordering::Greater => Some(SyncVersionDisposition::Superseded),
        std::cmp::Ordering::Equal
            if existing.operation == remote.operation
                && existing.content_hash == remote.content_hash =>
        {
            Some(SyncVersionDisposition::Superseded)
        }
        std::cmp::Ordering::Equal => Some(SyncVersionDisposition::Conflict),
        std::cmp::Ordering::Less => None,
    }
}

fn record_sync_apply_outcome(
    transaction: &rusqlite::Transaction<'_>,
    cursor: i64,
    state: &str,
    detail: Option<&str>,
) -> rusqlite::Result<()> {
    transaction.execute(
        "INSERT INTO anywhere_sync_apply (cursor, state, detail) VALUES (?1, ?2, ?3)",
        rusqlite::params![cursor, state, detail],
    )?;
    Ok(())
}

fn classify_mutable_sync_version(
    transaction: &rusqlite::Transaction<'_>,
    record: &StagedHistoryRecord,
    local_device_id: [u8; 16],
) -> Result<(Option<SyncVersionDisposition>, bool)> {
    let local = transaction
        .query_row(
            "SELECT operation, logical_clock, content_hash FROM sync_journal
             WHERE record_kind = ?1 AND stable_id = ?2
             ORDER BY logical_clock DESC, id DESC LIMIT 1",
            (&record.record_kind, &record.stable_id),
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Vec<u8>>(2)?,
                ))
            },
        )
        .optional()?;
    let materialized = transaction
        .query_row(
            "SELECT operation, logical_clock, sender_device_id, content_hash
             FROM anywhere_sync_materialized WHERE record_kind = ?1 AND stable_id = ?2",
            (&record.record_kind, &record.stable_id),
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Vec<u8>>(2)?,
                    row.get::<_, Vec<u8>>(3)?,
                ))
            },
        )
        .optional()?;
    let mut disposition = local.as_ref().and_then(|(operation, clock, hash)| {
        compare_sync_version(
            SyncVersion {
                operation: &record.operation,
                logical_clock: record.logical_clock,
                device_id: record.sender_device_id,
                content_hash: &record.content_hash,
            },
            SyncVersion {
                operation,
                logical_clock: *clock,
                device_id: local_device_id,
                content_hash: hash,
            },
        )
    });
    if let Some((operation, clock, sender, hash)) = &materialized {
        let sender = sender.as_slice().try_into().map_err(|_| {
            StoreError::InvalidValue("materialized sync sender id has the wrong length".into())
        })?;
        let candidate = compare_sync_version(
            SyncVersion {
                operation: &record.operation,
                logical_clock: record.logical_clock,
                device_id: record.sender_device_id,
                content_hash: &record.content_hash,
            },
            SyncVersion {
                operation,
                logical_clock: *clock,
                device_id: sender,
                content_hash: hash,
            },
        );
        if matches!(candidate, Some(SyncVersionDisposition::Conflict)) || disposition.is_none() {
            disposition = candidate.or(disposition);
        }
    }
    Ok((disposition, local.is_some() || materialized.is_some()))
}

fn upsert_sync_materialized(
    transaction: &rusqlite::Transaction<'_>,
    record: &StagedHistoryRecord,
) -> rusqlite::Result<()> {
    transaction.execute(
        "INSERT INTO anywhere_sync_materialized
         (record_kind, stable_id, operation, logical_clock, sender_device_id, content_hash)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(record_kind, stable_id) DO UPDATE SET
           operation = excluded.operation,
           logical_clock = excluded.logical_clock,
           sender_device_id = excluded.sender_device_id,
           content_hash = excluded.content_hash",
        rusqlite::params![
            &record.record_kind,
            &record.stable_id,
            &record.operation,
            record.logical_clock,
            record.sender_device_id.as_slice(),
            record.content_hash.as_slice()
        ],
    )?;
    Ok(())
}

pub(crate) fn insert_sync_journal_row(
    conn: &Connection,
    record_kind: &str,
    stable_id: &str,
    operation: SyncJournalOperation,
    revision: i64,
    logical_clock: i64,
    payload: &[u8],
) -> Result<bool> {
    insert_sync_journal_row_with_base(
        conn,
        record_kind,
        stable_id,
        operation,
        revision,
        logical_clock,
        None,
        payload,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn insert_sync_journal_row_with_base(
    conn: &Connection,
    record_kind: &str,
    stable_id: &str,
    operation: SyncJournalOperation,
    revision: i64,
    logical_clock: i64,
    base_hash: Option<&[u8; 32]>,
    payload: &[u8],
) -> Result<bool> {
    if record_kind.trim().is_empty() || stable_id.trim().is_empty() {
        return Err(StoreError::InvalidValue(
            "sync record kind and stable id must be non-empty".into(),
        ));
    }
    if !matches!(
        record_kind,
        "session"
            | "message"
            | "checkpoint"
            | "tool_call"
            | "routing_decision"
            | "usage"
            | "compaction"
            | "memory"
            | "user_setting"
            | "command"
            | "skill"
            | "agent"
            | "workflow"
            | "file"
    ) {
        return Err(StoreError::InvalidValue(
            "record kind is not eligible for Anywhere sync".into(),
        ));
    }
    if operation == SyncJournalOperation::Tombstone && !payload.is_empty() {
        return Err(StoreError::InvalidValue(
            "sync tombstones must have an empty payload".into(),
        ));
    }
    if record_kind == "file" && operation == SyncJournalOperation::Upsert && base_hash.is_none() {
        return Err(StoreError::InvalidValue(
            "file upserts require a base content hash".into(),
        ));
    }
    let content_hash = sync_payload_hash(payload);
    let changed = conn.execute(
        "INSERT INTO sync_journal
             (record_kind, stable_id, operation, revision, logical_clock, base_hash,
              content_hash, payload)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(record_kind, stable_id, revision) DO NOTHING",
        rusqlite::params![
            record_kind,
            stable_id,
            operation.as_str(),
            revision,
            logical_clock,
            base_hash.map(|hash| hash.as_slice()),
            content_hash.as_slice(),
            payload,
        ],
    )?;
    if changed == 1 {
        return Ok(true);
    }
    let existing = conn.query_row(
        "SELECT operation, logical_clock, base_hash, content_hash, payload
         FROM sync_journal WHERE record_kind = ?1 AND stable_id = ?2 AND revision = ?3",
        (record_kind, stable_id, revision),
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, Option<Vec<u8>>>(2)?,
                row.get::<_, Vec<u8>>(3)?,
                row.get::<_, Vec<u8>>(4)?,
            ))
        },
    )?;
    let same = existing.0 == operation.as_str()
        && existing.1 == logical_clock
        && existing.2.as_deref() == base_hash.map(|hash| hash.as_slice())
        && existing.3 == content_hash
        && existing.4 == payload;
    if same {
        Ok(false)
    } else {
        Err(StoreError::InvalidValue(
            "sync revision already exists with different content".into(),
        ))
    }
}

/// Append the next immutable snapshot for a local record inside the caller's transaction.
pub(crate) fn append_sync_revision(
    conn: &Connection,
    record_kind: &str,
    stable_id: &str,
    operation: SyncJournalOperation,
    payload: &[u8],
) -> Result<()> {
    let enabled = conn
        .query_row(
            "SELECT enabled FROM anywhere_sync_state WHERE singleton = 1",
            [],
            |row| row.get::<_, bool>(0),
        )
        .optional()?
        .unwrap_or(false);
    if !enabled {
        return Ok(());
    }
    let (revision, logical_clock) = conn.query_row(
        "SELECT COALESCE(MAX(revision), 0) + 1, COALESCE(MAX(logical_clock), 0) + 1
         FROM sync_journal WHERE record_kind = ?1 AND stable_id = ?2",
        (record_kind, stable_id),
        |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
    )?;
    if !insert_sync_journal_row(
        conn,
        record_kind,
        stable_id,
        operation,
        revision,
        logical_clock,
        payload,
    )? {
        return Err(StoreError::InvalidValue(
            "sync journal revision collision".into(),
        ));
    }
    Ok(())
}

pub(crate) fn append_session_snapshot(conn: &Connection, session_id: &str) -> Result<()> {
    let payload = conn.query_row(
        "SELECT id, title, cwd, permission_mode, total_cost_usd, parent_session_id,
                forked_from, forked_at_seq, worktree_path, archived, view_snapshot
         FROM session WHERE id = ?1",
        [session_id],
        |row| {
            Ok(serde_json::json!({
                "id": row.get::<_, String>(0)?,
                "title": row.get::<_, Option<String>>(1)?,
                "cwd": row.get::<_, String>(2)?,
                "permission_mode": row.get::<_, String>(3)?,
                "total_cost_usd": row.get::<_, f64>(4)?,
                "parent_session_id": row.get::<_, Option<String>>(5)?,
                "forked_from": row.get::<_, Option<String>>(6)?,
                "forked_at_seq": row.get::<_, Option<i64>>(7)?,
                "worktree_path": row.get::<_, Option<String>>(8)?,
                "archived": row.get::<_, bool>(9)?,
                "view_snapshot": row.get::<_, Option<String>>(10)?,
            }))
        },
    )?;
    let payload = sync_json(payload)?;
    append_sync_revision(
        conn,
        "session",
        session_id,
        SyncJournalOperation::Upsert,
        &payload,
    )
}

impl Store {
    /// Open (creating if needed) a database file and run migrations.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::build(ConnSource::File(path.as_ref().to_path_buf()))
    }

    /// In-memory store, primarily for tests.
    pub fn open_in_memory() -> Result<Self> {
        Self::build(ConnSource::Memory)
    }

    fn build(source: ConnSource) -> Result<Self> {
        let reservation_source = source.clone();
        let in_memory = matches!(source, ConnSource::Memory);
        let manager = SqliteManager { source };
        let builder = r2d2::Pool::builder().test_on_check_out(false);
        // A small pool lets WAL reads run concurrently instead of serializing behind a single mutex
        // (the TUI run loop, subagents, and the lattice indexer all touch the store). An in-memory
        // store, by contrast, is pinned to ONE connection that is never recycled — every `:memory:`
        // open is a fresh empty DB, so dropping/recreating it would silently lose all data.
        let pool = if in_memory {
            builder
                .max_size(1)
                .min_idle(Some(1))
                .idle_timeout(None)
                .max_lifetime(None)
                .build(manager)
        } else {
            builder.max_size(8).build(manager)
        }
        .map_err(|e| StoreError::Pool(e.to_string()))?;

        // Run migrations ONCE on a single pooled connection (for in-memory this is THE connection).
        // File DBs persist `journal_mode = WAL` and the schema, so later pooled connections inherit
        // them; per-connection pragmas (busy_timeout, foreign_keys) are set in `SqliteManager`.
        {
            let conn = pool.get().map_err(|e| StoreError::Pool(e.to_string()))?;
            if !in_memory {
                conn.pragma_update(None, "journal_mode", "WAL")?;
            }
            // Every step below is idempotent, so the whole group can be retried as a unit — and it
            // needs to be, because a database that DOES have work to do here writes while another
            // process may be holding the single WAL writer for tens of seconds (a lattice PageRank
            // pass, a bulk prune). Without the retry, `Store::open` fails outright with "database is
            // locked" rather than waiting its turn. On an already-initialized database this whole
            // block performs no write at all, so it never contends in the first place.
            with_busy_retry(|| {
                // Migrate before schema so old DBs get the composite PK before CREATE TABLE IF NOT EXISTS no-ops.
                migrate_subscription_usage(&conn)?;
                conn.execute_batch(schema::SCHEMA)?;
                seed_singleton_rows(&conn)?;
                // Versioned migrations (PRAGMA user_version). Folds the historic ad-hoc ADD COLUMN
                // migrations and the UNIQUE(session_id, seq) index; refuses a DB from a newer build.
                run_migrations(&conn)?;
                // Forge Anywhere's payload snapshot column was added while schema v18 was still on an
                // unreleased feature branch. Repair databases opened by that branch without consuming
                // a permanent migration number; released schemas must use MIGRATIONS instead.
                add_column_if_missing(
                    &conn,
                    "ALTER TABLE sync_journal ADD COLUMN payload BLOB NOT NULL DEFAULT X''",
                )?;
                add_column_if_missing(&conn, "ALTER TABLE sync_journal ADD COLUMN base_hash BLOB")?;
                Ok(())
            })?;
        }
        let reservation_store_id = match reservation_source {
            ConnSource::File(path) => std::fs::canonicalize(&path)
                .unwrap_or(path)
                .to_string_lossy()
                .into_owned(),
            ConnSource::Memory => next_memory_store_id(),
        };
        Ok(Self {
            pool,
            reservation_store_id,
            live_event_writes: std::sync::atomic::AtomicU64::new(0),
        })
    }

    /// Atomically reserve a model for an active completion. The returned guard releases the
    /// reservation on every normal return, error, and cancellation path.
    pub fn try_reserve_model(&self, model: &str) -> Option<ModelReservation> {
        let mut in_flight = in_flight_models().lock().ok()?;
        if !in_flight.insert((self.reservation_store_id.clone(), model.to_string())) {
            return None;
        }
        Some(ModelReservation {
            store_id: self.reservation_store_id.clone(),
            model: model.to_string(),
        })
    }

    /// Check whether a model has an active completion reservation.
    pub fn is_model_reserved(&self, model: &str) -> bool {
        in_flight_models().lock().is_ok_and(|in_flight| {
            in_flight.contains(&(self.reservation_store_id.clone(), model.to_string()))
        })
    }

    /// Check out a pooled connection. Named `lock` for continuity with the call sites; the returned
    /// `PooledConnection` derefs to `rusqlite::Connection` and returns itself to the pool on drop.
    pub(crate) fn lock(&self) -> Result<r2d2::PooledConnection<SqliteManager>> {
        self.pool.get().map_err(|e| StoreError::Pool(e.to_string()))
    }

    /// Number of messages in a session.
    pub fn message_count(&self, session_id: &str) -> Result<i64> {
        Ok(self.lock()?.query_row(
            // `active = 1` only — soft-deleted (undone/compacted) rows must not inflate the count
            // shown in the session picker / `forge sessions`, which `load_messages` also excludes.
            "SELECT COUNT(*) FROM message WHERE session_id = ?1 AND active = 1",
            [session_id],
            |row| row.get(0),
        )?)
    }

    /// The id of the most-recent top-level session (excludes subagent children), or `None` if
    /// there are no sessions yet.
    pub fn most_recent_session_id(&self) -> Result<Option<String>> {
        let conn = self.lock()?;
        // Order by LAST ACTIVITY (newest message), not creation time, so `--continue` reattaches
        // the session the user actually used most recently — not whichever was created last.
        let result = conn
            .query_row(
                "SELECT s.id FROM session s WHERE s.parent_session_id IS NULL \
                 ORDER BY COALESCE( \
                   (SELECT MAX(m.created_at) FROM message m WHERE m.session_id = s.id), \
                   s.created_at) DESC, s.rowid DESC LIMIT 1",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        Ok(result)
    }

    /// Past sessions, **most-recently-used first** (by newest message, falling back to creation
    /// time), so the picker lists the sessions you're likely to resume at the top. Excludes
    /// subagent child sessions (`parent_session_id IS NOT NULL`) so the picker and the
    /// `forge sessions` command only surface top-level sessions. Also excludes sessions that
    /// never received a real (role='user') message — checked regardless of `active`, so a
    /// session whose sole user message was later soft-deleted by `/undo` or a checkpoint restore
    /// still counts as used — a session row is created eagerly at process start (before
    /// [`Store::prune_empty`] has a chance to sweep it, and for a session still in its first
    /// few minutes of life), so without this filter a process that opens a session and
    /// exits/crashes before any prompt is sent — including one stuck in a spawn loop, the
    /// original trigger for this — fills the picker with blank, useless entries.
    pub fn list_sessions(&self) -> Result<Vec<SessionSummary>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT s.id, s.cwd, s.permission_mode, s.created_at, s.total_cost_usd,
                    (SELECT COUNT(*) FROM message m WHERE m.session_id = s.id AND m.active = 1),
                    (SELECT content FROM message m WHERE m.session_id = s.id
                       AND m.role = 'user' AND m.active = 1 ORDER BY m.seq LIMIT 1),
                    COALESCE((SELECT MAX(m.created_at) FROM message m WHERE m.session_id = s.id),
                             s.created_at) AS last_activity,
                    s.title, s.worktree_path
             FROM session s WHERE s.parent_session_id IS NULL \
             AND s.archived = 0 \
             AND EXISTS ( \
               SELECT 1 FROM message m \
               WHERE m.session_id = s.id AND m.role = 'user' \
             ) \
             ORDER BY last_activity DESC, s.rowid DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(SessionSummary {
                id: row.get(0)?,
                cwd: row.get(1)?,
                permission_mode: row.get(2)?,
                created_at: row.get(3)?,
                total_cost_usd: row.get(4)?,
                message_count: row.get(5)?,
                preview: row.get(6)?,
                last_activity: row.get(7)?,
                title: row.get(8)?,
                worktree_path: row.get(9)?,
                archived: false, // filtered to archived = 0 above
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    /// Like [`Store::list_sessions`] but INCLUDES archived sessions (flagged via
    /// [`SessionSummary::archived`]) instead of hiding them. Used by `forge serve`'s
    /// past-sessions browser (`GET /api/sessions/past`) so a session the user explicitly
    /// archived is still browsable and resumable — just visibly marked — rather than only
    /// surfacing sessions orphaned by a daemon restart. Same MRU ordering, same exclusion of
    /// subagent children and sessions that never received a real user message.
    pub fn list_sessions_for_resume(&self) -> Result<Vec<SessionSummary>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT s.id, s.cwd, s.permission_mode, s.created_at, s.total_cost_usd,
                    (SELECT COUNT(*) FROM message m WHERE m.session_id = s.id AND m.active = 1),
                    (SELECT content FROM message m WHERE m.session_id = s.id
                       AND m.role = 'user' AND m.active = 1 ORDER BY m.seq LIMIT 1),
                    COALESCE((SELECT MAX(m.created_at) FROM message m WHERE m.session_id = s.id),
                             s.created_at) AS last_activity,
                    s.title, s.worktree_path, s.archived
             FROM session s WHERE s.parent_session_id IS NULL \
             AND EXISTS ( \
               SELECT 1 FROM message m \
               WHERE m.session_id = s.id AND m.role = 'user' \
             ) \
             ORDER BY last_activity DESC, s.rowid DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(SessionSummary {
                id: row.get(0)?,
                cwd: row.get(1)?,
                permission_mode: row.get(2)?,
                created_at: row.get(3)?,
                total_cost_usd: row.get(4)?,
                message_count: row.get(5)?,
                preview: row.get(6)?,
                last_activity: row.get(7)?,
                title: row.get(8)?,
                worktree_path: row.get(9)?,
                archived: row.get::<_, i64>(10)? != 0,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    /// Archive a session (`forge serve`): hidden from [`Store::list_sessions`] and the daemon's
    /// session list, but its full history stays intact (nothing is deleted).
    pub fn archive_session(&self, session_id: &str) -> Result<()> {
        self.lock()?.execute(
            "UPDATE session SET archived = 1 WHERE id = ?1",
            [session_id],
        )?;
        Ok(())
    }

    /// Whether a session is archived. `Ok(false)` for unknown ids (nothing to un-hide).
    pub fn session_archived(&self, session_id: &str) -> Result<bool> {
        let n: i64 = self.lock()?.query_row(
            "SELECT COUNT(*) FROM session WHERE id = ?1 AND archived = 1",
            [session_id],
            |row| row.get(0),
        )?;
        Ok(n > 0)
    }

    /// Un-archive a session: reverses [`Store::archive_session`]. `forge serve` calls this when
    /// resuming a session from the past-sessions browser — resurrecting an archived session is
    /// an explicit choice to bring it back, so it should reappear in [`Store::list_sessions`]
    /// and the fleet list once it stops running again, rather than immediately re-hiding itself.
    pub fn unarchive_session(&self, session_id: &str) -> Result<()> {
        let conn = self.lock()?;
        let blocked: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM anywhere_handoff_session_state WHERE session_id=?1)",
            [session_id],
            |row| row.get(0),
        )?;
        if blocked {
            return Err(StoreError::InvalidValue(
                "session is frozen by an Anywhere handoff".into(),
            ));
        }
        conn.execute(
            "UPDATE session SET archived = 0 WHERE id = ?1",
            [session_id],
        )?;
        Ok(())
    }

    /// Record the isolated worktree a daemon session runs in (`forge serve` with `worktree:true`).
    pub fn set_session_worktree(&self, session_id: &str, path: &str) -> Result<()> {
        self.lock()?.execute(
            "UPDATE session SET worktree_path = ?2 WHERE id = ?1",
            (session_id, path),
        )?;
        Ok(())
    }

    /// The isolated worktree recorded for a session, if any.
    pub fn session_worktree(&self, session_id: &str) -> Result<Option<String>> {
        Ok(self
            .lock()?
            .query_row(
                "SELECT worktree_path FROM session WHERE id = ?1",
                [session_id],
                |row| row.get(0),
            )
            .optional()?
            .flatten())
    }

    /// Store (or refresh) a Web Push subscription, deduplicating by `endpoint` — a browser
    /// re-subscribing after a permission round-trip must update its keys in place, never pile
    /// up duplicate rows that would each receive (and each decrypt-fail or double-notify) every
    /// push. Atomic: a single `INSERT … ON CONFLICT(endpoint) DO UPDATE` against the UNIQUE index
    /// `idx_push_subscription_endpoint` (migration #13), so concurrent callers can't race a
    /// duplicate in between a SELECT and an INSERT. Returns the row id (existing or new).
    pub fn upsert_push_subscription(
        &self,
        endpoint: &str,
        p256dh: &str,
        auth: &str,
    ) -> Result<String> {
        let conn = self.lock()?;
        let id = forge_types::new_id();
        let row_id = conn.query_row(
            "INSERT INTO push_subscription (id, endpoint, p256dh, auth) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(endpoint) DO UPDATE SET p256dh = excluded.p256dh, auth = excluded.auth
             RETURNING id",
            (&id, endpoint, p256dh, auth),
            |row| row.get::<_, String>(0),
        )?;
        Ok(row_id)
    }

    /// Remove a Web Push subscription by its endpoint (unsubscribe, or a push service answering
    /// 404/410). `Ok(true)` when a row was actually deleted.
    pub fn delete_push_subscription(&self, endpoint: &str) -> Result<bool> {
        let n = self.lock()?.execute(
            "DELETE FROM push_subscription WHERE endpoint = ?1",
            [endpoint],
        )?;
        Ok(n > 0)
    }

    /// Every stored Web Push subscription, oldest first (delivery order is stable and boring).
    pub fn list_push_subscriptions(&self) -> Result<Vec<PushSubscription>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT id, endpoint, p256dh, auth FROM push_subscription ORDER BY created_at, id",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok(PushSubscription {
                    id: row.get(0)?,
                    endpoint: row.get(1)?,
                    p256dh: row.get(2)?,
                    auth: row.get(3)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Store (or refresh) an APNs subscription, deduplicating by `device_token` — Apple may
    /// reissue the same device a new token, but a re-registration with an unchanged token must
    /// update the row in place, never pile up duplicates. Returns the row id (existing or new).
    pub fn upsert_apns_subscription(
        &self,
        device_token: &str,
        environment: &str,
    ) -> Result<String> {
        let conn = self.lock()?;
        if let Some(id) = conn
            .query_row(
                "SELECT id FROM apns_subscription WHERE device_token = ?1",
                [device_token],
                |row| row.get::<_, String>(0),
            )
            .optional()?
        {
            conn.execute(
                "UPDATE apns_subscription SET environment = ?2 WHERE id = ?1",
                (&id, environment),
            )?;
            return Ok(id);
        }
        let id = forge_types::new_id();
        conn.execute(
            "INSERT INTO apns_subscription (id, device_token, environment) VALUES (?1, ?2, ?3)",
            (&id, device_token, environment),
        )?;
        Ok(id)
    }

    /// Remove an APNs subscription by its device token (unsubscribe, or APNs answering
    /// `BadDeviceToken`/`Unregistered`). `Ok(true)` when a row was actually deleted.
    pub fn delete_apns_subscription(&self, device_token: &str) -> Result<bool> {
        let n = self.lock()?.execute(
            "DELETE FROM apns_subscription WHERE device_token = ?1",
            [device_token],
        )?;
        Ok(n > 0)
    }

    /// Every stored APNs subscription, oldest first.
    pub fn list_apns_subscriptions(&self) -> Result<Vec<ApnsSubscription>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT id, device_token, environment FROM apns_subscription ORDER BY created_at, id",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok(ApnsSubscription {
                    id: row.get(0)?,
                    device_token: row.get(1)?,
                    environment: row.get(2)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Store (or refresh) a session's Live Activity remote-update push token. Keyed by
    /// `session_id` (the table's primary key), so a re-registration for the same session
    /// replaces the existing token/environment in place rather than adding a row.
    pub fn upsert_live_activity_token(
        &self,
        session_id: &str,
        push_token: &str,
        environment: &str,
    ) -> Result<()> {
        self.lock()?.execute(
            "INSERT INTO live_activity_token (session_id, push_token, environment)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(session_id) DO UPDATE SET
                push_token = excluded.push_token,
                environment = excluded.environment,
                updated_at = strftime('%s','now')",
            (session_id, push_token, environment),
        )?;
        Ok(())
    }

    /// Remove a session's Live Activity push token (the activity ended). `Ok(true)` when a row
    /// was actually deleted.
    pub fn delete_live_activity_token(&self, session_id: &str) -> Result<bool> {
        let n = self.lock()?.execute(
            "DELETE FROM live_activity_token WHERE session_id = ?1",
            [session_id],
        )?;
        Ok(n > 0)
    }

    /// A session's stored Live Activity push token, if any.
    pub fn get_live_activity_token(&self, session_id: &str) -> Result<Option<LiveActivityToken>> {
        self.lock()?
            .query_row(
                "SELECT session_id, push_token, environment FROM live_activity_token
                 WHERE session_id = ?1",
                [session_id],
                |row| {
                    Ok(LiveActivityToken {
                        session_id: row.get(0)?,
                        push_token: row.get(1)?,
                        environment: row.get(2)?,
                    })
                },
            )
            .optional()
            .map_err(StoreError::from)
    }

    /// A session's stored title, if any.
    pub fn session_title(&self, session_id: &str) -> Result<Option<String>> {
        Ok(self
            .lock()?
            .query_row(
                "SELECT title FROM session WHERE id = ?1",
                [session_id],
                |row| row.get(0),
            )
            .optional()?
            .flatten())
    }

    /// Full session ids whose id starts with `prefix` (git-style abbreviation). `prefix` is
    /// matched literally: any `%`/`_`/`\` it contains is escaped so it can't act as a SQL LIKE
    /// wildcard and broaden the match beyond a literal prefix.
    pub fn matching_session_ids(&self, prefix: &str) -> Result<Vec<String>> {
        let conn = self.lock()?;
        let escaped = escape_like_pattern(prefix);
        let mut stmt =
            conn.prepare("SELECT id FROM session WHERE id LIKE ?1 || '%' ESCAPE '\\'")?;
        let rows = stmt.query_map([escaped], |row| row.get::<_, String>(0))?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    /// Whether a session with this id exists.
    pub fn session_exists(&self, session_id: &str) -> Result<bool> {
        let n: i64 = self.lock()?.query_row(
            "SELECT COUNT(*) FROM session WHERE id = ?1",
            [session_id],
            |row| row.get(0),
        )?;
        Ok(n > 0)
    }

    /// All *active* messages of a session, in turn order (by seq). Soft-deleted rows (those a
    /// `/undo` rewound past) are excluded — they remain in the table for audit/redo. If a
    /// compaction summary exists (written by [`compact_session_store`](Self::compact_session_store)),
    /// a synthetic System message is prepended so a resumed session sees the compacted view.
    pub fn load_messages(&self, session_id: &str) -> Result<Vec<StoredMessage>> {
        let conn = self.lock()?;
        // Read compaction summary before the message prepare (both are &self borrows; ordering
        // keeps the non-mut borrow from query_row from conflicting with the stmt lifetime).
        let summary: Option<String> = conn
            .query_row(
                "SELECT summary FROM session_compaction WHERE session_id = ?1",
                [session_id],
                |row| row.get(0),
            )
            .optional()?;
        let mut stmt = conn.prepare_cached(
            "SELECT role, content, model, tool_calls_json, tool_call_id, visibility
             FROM message WHERE session_id = ?1 AND active = 1 ORDER BY seq",
        )?;
        let rows = stmt.query_map([session_id], |row| {
            let role: String = row.get(0)?;
            let tool_calls_json: Option<String> = row.get(3)?;
            let tool_calls = tool_calls_json
                .and_then(|j| serde_json::from_str(&j).ok())
                .unwrap_or_default();
            let visibility: String = row.get(5)?;
            Ok(StoredMessage {
                role: Role::parse(&role).unwrap_or(Role::User),
                content: row.get(1)?,
                model: row.get(2)?,
                tool_calls,
                tool_call_id: row.get(4)?,
                visibility: Visibility::parse(&visibility),
            })
        })?;
        let mut msgs = rows
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(StoreError::from)?;
        if let Some(s) = summary {
            msgs.insert(
                0,
                StoredMessage {
                    role: Role::System,
                    content: format!(
                        "[Earlier conversation summarized to save context]\n{}",
                        s.trim()
                    ),
                    model: None,
                    tool_calls: vec![],
                    tool_call_id: None,
                    visibility: Visibility::Llm,
                },
            );
        }
        Ok(msgs)
    }

    /// ALL messages of a session in turn order, INCLUDING soft-deleted rows (compacted-away or
    /// `/undo`-rewound) and WITHOUT prepending the summary marker — the genuine, untouched
    /// conversation. The model only ever sees the compacted view ([`load_messages`](Self::load_messages)),
    /// but this lets the USER still read the FULL original history in scrollback after a resume.
    pub fn load_all_messages(&self, session_id: &str) -> Result<Vec<StoredMessage>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT role, content, model, tool_calls_json, tool_call_id, visibility
             FROM message WHERE session_id = ?1 ORDER BY seq",
        )?;
        let rows = stmt.query_map([session_id], |row| {
            let role: String = row.get(0)?;
            let tool_calls_json: Option<String> = row.get(3)?;
            let tool_calls = tool_calls_json
                .and_then(|j| serde_json::from_str(&j).ok())
                .unwrap_or_default();
            let visibility: String = row.get(5)?;
            Ok(StoredMessage {
                role: Role::parse(&role).unwrap_or(Role::User),
                content: row.get(1)?,
                model: row.get(2)?,
                tool_calls,
                tool_call_id: row.get(4)?,
                visibility: Visibility::parse(&visibility),
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    /// One page of a session's user-facing transcript, NEWEST first — the remote-control
    /// scrollback pagination seam (docs/features/remote-control.md). Returns user + assistant
    /// turns plus `visibility='ui'` notes (they are part of the visible conversation); tool
    /// results, tool-call carrier rows (empty content), and system prompts are harness plumbing
    /// and excluded. Soft-deleted (`active=0`) rows are INCLUDED, like
    /// [`load_all_messages`](Self::load_all_messages) — this is the user's history, not the
    /// model's context. `before_seq` restricts to rows with `seq < before_seq` (pass `None` for
    /// the newest page); `limit` caps the page size.
    pub fn load_history_page(
        &self,
        session_id: &str,
        before_seq: Option<i64>,
        limit: usize,
    ) -> Result<Vec<HistoryRow>> {
        self.load_history_page_with(session_id, before_seq, limit, false)
    }

    /// [`load_history_page`](Self::load_history_page) with the tool rows opted in.
    ///
    /// `include_tools = false` is byte-for-byte the historical page (the filter below collapses to
    /// the original predicate), so every existing caller and the legacy PWA see no change.
    ///
    /// `include_tools = true` additionally returns the persisted `role='tool'` rows — the tool
    /// RESULT rows written by `Session::invoke_tool` — with [`HistoryRow::tool_name`] resolved.
    /// Note what that is and isn't: the tool row itself stores only the result text and its
    /// `tool_call_id`; the NAME lives in the assistant carrier's `tool_calls_json`, so it is
    /// recovered from the nearest preceding row that has one and left `None` when that carrier is
    /// gone or does not contain the id. `llm_only` tool rows stay excluded — like their
    /// user/assistant siblings they are provider-continuity plumbing, not conversation.
    ///
    /// It also SURFACES the tool CALLS, which are persisted but were previously unreachable: a
    /// carrier row's `content` is empty, so the `content != ''` filter dropped it and the
    /// `[{id, name, args}]` it carries went with it. With tools opted in, each carrier is expanded
    /// into one row per declared call ([`ToolPhase::Call`], content = a capped args summary), at
    /// the CARRIER's `seq`/`created_at` — so every call sorts before the result row that answers
    /// it (results always carry a later seq), and a call whose result never arrived (an
    /// interrupted turn) still shows up instead of vanishing. A carrier that also wrote prose
    /// keeps its own assistant row, ordered before its calls. Nothing here is reconstructed: the
    /// carrier row and its JSON are real persisted data.
    pub fn load_history_page_with(
        &self,
        session_id: &str,
        before_seq: Option<i64>,
        limit: usize,
        include_tools: bool,
    ) -> Result<Vec<HistoryRow>> {
        let conn = self.lock()?;
        // The carrier lookup sits behind `CASE WHEN ?4 = 1 AND m.role = 'tool'` rather than in the
        // projection unconditionally: SQLite short-circuits the CASE, so the default page runs the
        // same work it always did and pays nothing for a subquery it never needs. Same for the
        // row's OWN `tool_calls_json` (the carrier expansion below) and for the widened content
        // filter, which collapses back to `m.content != ''` when tools aren't asked for.
        let mut stmt = conn.prepare(
            "SELECT m.seq, m.role, m.content, m.model, m.created_at, m.visibility,
                    m.tool_call_id,
                    CASE WHEN ?4 = 1 AND m.role = 'tool' THEN (
                        SELECT c.tool_calls_json FROM message c
                         WHERE c.session_id = m.session_id
                           AND c.seq < m.seq
                           AND c.tool_calls_json IS NOT NULL
                         ORDER BY c.seq DESC LIMIT 1
                    ) END,
                    CASE WHEN ?4 = 1 THEN m.tool_calls_json END
             FROM message m
             WHERE m.session_id = ?1
               AND (?2 IS NULL OR m.seq < ?2)
               AND (((m.role IN ('user', 'assistant') AND m.visibility != 'llm_only') OR m.visibility = 'ui')
                    OR (?4 = 1 AND m.role = 'tool' AND m.visibility != 'llm_only'))
               AND (m.content != ''
                    OR (?4 = 1 AND m.role = 'assistant' AND m.visibility != 'llm_only'
                        AND m.tool_calls_json IS NOT NULL))
             ORDER BY m.seq DESC LIMIT ?3",
        )?;
        let rows = stmt.query_map(
            rusqlite::params![
                session_id,
                before_seq,
                limit as i64,
                i64::from(include_tools)
            ],
            |row| {
                let role: String = row.get(1)?;
                let visibility: String = row.get(5)?;
                let tool_call_id: Option<String> = row.get(6)?;
                let carrier_json: Option<String> = row.get(7)?;
                let own_calls_json: Option<String> = row.get(8)?;
                let role = Role::parse(&role).unwrap_or(Role::User);
                Ok((
                    HistoryRow {
                        seq: row.get(0)?,
                        role,
                        content: row.get(2)?,
                        model: row.get(3)?,
                        created_at: row.get(4)?,
                        visibility: Visibility::parse(&visibility),
                        tool_name: carrier_json.as_deref().and_then(|carrier| {
                            tool_name_from_carrier(carrier, tool_call_id.as_deref())
                        }),
                        tool_phase: (role == Role::Tool).then_some(ToolPhase::Result),
                    },
                    own_calls_json,
                ))
            },
        )?;
        // `limit` bounds the DB rows read, not the rows returned: one carrier expands into as many
        // call rows as it declares. Pagination is unaffected — every returned row carries a REAL
        // `seq`, so `before_seq` from the oldest of them opens the next window with no gap and no
        // repeat.
        let mut out = Vec::new();
        for row in rows {
            let (row, own_calls_json) = row?;
            // Newest-first order, so a carrier's calls are pushed in reverse declaration order and
            // its own prose last: reversed by the client, that reads prose → call 1 → call 2.
            if let Some(json) = own_calls_json.as_deref() {
                for call in parse_tool_calls(json).into_iter().rev() {
                    out.push(HistoryRow {
                        seq: row.seq,
                        // A call is tool activity, like the result it precedes — the carrier's
                        // `assistant` role belongs to its prose row, not to the calls.
                        role: Role::Tool,
                        content: tool_call_args_summary(&call.args),
                        // Only a provider round-trip has a model; the persisted result rows this
                        // sits next to carry none either.
                        model: None,
                        created_at: row.created_at,
                        visibility: row.visibility,
                        tool_name: Some(call.name),
                        tool_phase: Some(ToolPhase::Call),
                    });
                }
            }
            if !row.content.is_empty() {
                out.push(row);
            }
        }
        Ok(out)
    }

    /// When the session's FIRST user-facing row was written — the zero point a transcript replay
    /// measures its offsets from (raw `created_at` epochs give a scrubber nothing to anchor on).
    /// Same row filter as [`load_history_page`](Self::load_history_page), so the epoch is the
    /// first row a client can actually page to. `None` for a session with no visible rows yet.
    pub fn history_epoch(&self, session_id: &str) -> Result<Option<i64>> {
        self.history_epoch_with(session_id, false)
    }

    /// [`history_epoch`](Self::history_epoch) over the row set
    /// [`load_history_page_with`](Self::load_history_page_with) returns for the same `include_tools`.
    ///
    /// The two MUST be asked the same question: a session whose first row is a tool result has one
    /// epoch with tools included and a later one without, and mixing them would slide every
    /// `elapsed_ms` on the wire — the scrubber's zero point would jump the moment a client toggled
    /// tool rows on.
    pub fn history_epoch_with(&self, session_id: &str, include_tools: bool) -> Result<Option<i64>> {
        let conn = self.lock()?;
        // Must mirror `load_history_page_with`'s filter exactly, INCLUDING the carrier rows it
        // expands into call rows: a carrier can be the oldest row of an include_tools page (a turn
        // that opened with a tool call), and measuring against a later row would slide every
        // `elapsed_ms` on the wire.
        let epoch = conn.query_row(
            "SELECT MIN(created_at)
             FROM message
             WHERE session_id = ?1
               AND (((role IN ('user', 'assistant') AND visibility != 'llm_only') OR visibility = 'ui')
                    OR (?2 = 1 AND role = 'tool' AND visibility != 'llm_only'))
               AND (content != ''
                    OR (?2 = 1 AND role = 'assistant' AND visibility != 'llm_only'
                        AND tool_calls_json IS NOT NULL))",
            rusqlite::params![session_id, i64::from(include_tools)],
            |row| row.get::<_, Option<i64>>(0),
        )?;
        Ok(epoch)
    }

    /// Whether this session has a stored compaction summary (was compacted at least once) — the
    /// signal for offering "compact first vs continue uncompacted" when resuming it.
    pub fn session_has_compaction(&self, session_id: &str) -> Result<bool> {
        let n: i64 = self.lock()?.query_row(
            "SELECT COUNT(*) FROM session_compaction WHERE session_id = ?1",
            [session_id],
            |row| row.get(0),
        )?;
        Ok(n > 0)
    }

    /// Persist the compacted view of a session: soft-delete the oldest active messages (keeping
    /// the last `keep_count`) and upsert `summary` into `session_compaction`. On the next resume,
    /// [`load_messages`](Self::load_messages) prepends a System message with the summary so the
    /// session rehydrates the compacted state instead of the full transcript.
    pub fn compact_session_store(
        &self,
        session_id: &str,
        summary: &str,
        keep_count: usize,
    ) -> Result<()> {
        let mut conn = self.lock()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if keep_count == 0 {
            tx.execute(
                "UPDATE message SET active = 0, compacted = 1 WHERE session_id = ?1 AND active = 1",
                [session_id],
            )?;
        } else {
            // Soft-delete every active message whose seq is below the (keep_count)-th newest.
            // LIMIT 1 OFFSET (keep_count-1) on DESC order gives the oldest row to KEEP.
            tx.execute(
                "UPDATE message SET active = 0, compacted = 1
                 WHERE session_id = ?1 AND active = 1
                 AND seq < (
                     SELECT seq FROM message
                     WHERE session_id = ?1 AND active = 1
                     ORDER BY seq DESC
                     LIMIT 1 OFFSET ?2
                 )",
                (session_id, keep_count as i64 - 1),
            )?;
        }
        tx.execute(
            "INSERT INTO session_compaction (session_id, summary) VALUES (?1, ?2)
             ON CONFLICT(session_id) DO UPDATE SET
               summary = excluded.summary,
               created_at = strftime('%s','now')",
            (session_id, summary),
        )?;
        let payload = sync_json(serde_json::json!({
            "session_id": session_id,
            "summary": summary,
            "keep_count": keep_count,
        }))?;
        append_sync_revision(
            &tx,
            "compaction",
            session_id,
            SyncJournalOperation::Upsert,
            &payload,
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Undo a compaction: reactivate the messages THIS compaction soft-deleted (`compacted = 1`)
    /// and drop the stored summary row. Rows `/undo` soft-deleted (`active = 0`, `compacted = 0`)
    /// stay removed — resurrecting them was a bug. Returns `false` (no-op) if the session was never
    /// compacted (no `session_compaction` row).
    pub fn uncompact_session_store(&self, session_id: &str) -> Result<bool> {
        let mut conn = self.lock()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let had_compaction: bool = tx.query_row(
            "SELECT COUNT(*) FROM session_compaction WHERE session_id = ?1",
            [session_id],
            |row| row.get::<_, i64>(0),
        )? > 0;
        if !had_compaction {
            tx.commit()?;
            return Ok(false);
        }
        tx.execute(
            "UPDATE message SET active = 1, compacted = 0 WHERE session_id = ?1 AND compacted = 1",
            [session_id],
        )?;
        tx.execute(
            "DELETE FROM session_compaction WHERE session_id = ?1",
            [session_id],
        )?;
        append_sync_revision(
            &tx,
            "compaction",
            session_id,
            SyncJournalOperation::Tombstone,
            &[],
        )?;
        tx.commit()?;
        Ok(true)
    }

    /// Every active message of a session in turn order, each joined to its usage row so a
    /// replay can show the model, token counts, cost, and wall-clock time of each turn
    /// (docs/features/session-replay.md). Unlike [`load_messages`](Self::load_messages) this
    /// is for auditing a finished session, not rebuilding live state.
    pub fn load_replay(&self, session_id: &str) -> Result<Vec<ReplayEntry>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT m.seq, m.role, m.content, m.model, m.created_at, m.tool_calls_json,
                    u.input_tokens, u.output_tokens, u.cost_usd
             FROM message m LEFT JOIN usage u ON u.message_id = m.id
             WHERE m.session_id = ?1 AND m.active = 1 ORDER BY m.seq",
        )?;
        let rows = stmt.query_map([session_id], |row| {
            let role: String = row.get(1)?;
            let tool_calls_json: Option<String> = row.get(5)?;
            let tool_calls = tool_calls_json
                .and_then(|j| serde_json::from_str(&j).ok())
                .unwrap_or_default();
            Ok(ReplayEntry {
                seq: row.get(0)?,
                role: Role::parse(&role).unwrap_or(Role::User),
                content: row.get(2)?,
                model: row.get(3)?,
                created_at: row.get(4)?,
                tool_calls,
                input_tokens: row.get(6)?,
                output_tokens: row.get(7)?,
                cost_usd: row.get(8)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    /// Every recorded `write_file`/`edit_file` call whose `path` matches (as a suffix) the given
    /// `filename_suffix`, oldest first — the raw material `forge blame` (docs/features/forge-blame.md)
    /// attributes source lines from. Joined to the owning session (for `cwd`, to resolve a relative
    /// `path` the same way the tool did) and to the assistant message that made the call (for
    /// `model`); `routing_decision.chosen_model` fills in when the message's own `model` is NULL
    /// (older rows, or a message that predates routing being recorded for it).
    pub fn file_edits(&self, filename_suffix: &str) -> Result<Vec<FileEditRow>> {
        let conn = self.lock()?;
        let pattern = escape_like_pattern(filename_suffix);
        let mut stmt = conn.prepare(
            "SELECT tc.tool_name, tc.args_json, tc.path, m.session_id, s.cwd,
                    COALESCE(m.model, r.chosen_model), m.seq, tc.created_at
             FROM tool_call tc
             JOIN message m ON m.id = tc.message_id
             JOIN session s ON s.id = m.session_id
             LEFT JOIN routing_decision r ON r.message_id = m.id
             WHERE tc.path IS NOT NULL
               AND tc.tool_name IN ('write_file', 'edit_file')
               AND tc.status = 'ok'
               AND tc.path LIKE '%' || ?1 ESCAPE '\\'
             ORDER BY tc.created_at ASC",
        )?;
        let rows = stmt.query_map([pattern], |row| {
            Ok(FileEditRow {
                tool_name: row.get(0)?,
                args_json: row.get(1)?,
                path: row.get(2)?,
                session_id: row.get(3)?,
                session_cwd: row.get(4)?,
                model: row.get(5)?,
                seq: row.get(6)?,
                created_at: row.get(7)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    /// The provenance context of one turn: the nearest user prompt at or before `seq`, and the
    /// content of the assistant message AT `seq` (the one that made the edit `forge blame` is
    /// explaining). Either half is `None` if no matching row exists — e.g. `seq` is a virtual
    /// subagent turn with no direct user prompt in this session.
    pub fn turn_context(&self, session_id: &str, seq: i64) -> Result<TurnContext> {
        let conn = self.lock()?;
        let user_prompt = conn
            .query_row(
                "SELECT content FROM message WHERE session_id = ?1 AND role = 'user' AND seq <= ?2 \
                 ORDER BY seq DESC LIMIT 1",
                (session_id, seq),
                |r| r.get(0),
            )
            .optional()?;
        let assistant_content = conn
            .query_row(
                "SELECT content FROM message WHERE session_id = ?1 AND role = 'assistant' AND seq = ?2",
                (session_id, seq),
                |r| r.get(0),
            )
            .optional()?;
        Ok(TurnContext {
            user_prompt,
            assistant_content,
        })
    }

    /// The next `seq` to assign for a session: `MAX(seq) + 1` over ALL rows (active or soft-deleted),
    /// or 0 for a fresh session. Must be used instead of an in-memory message COUNT when resuming a
    /// session that may have been COMPACTED — `load_messages` returns only the active tail (+ a
    /// synthetic summary), so its length is far below the real max seq, and reusing low seqs makes a
    /// later `/undo` deactivate pre-compaction survivors (data loss).
    pub fn next_seq_for_session(&self, session_id: &str) -> Result<i64> {
        Ok(self.lock()?.query_row(
            "SELECT COALESCE(MAX(seq), -1) + 1 FROM message WHERE session_id = ?1",
            [session_id],
            |row| row.get(0),
        )?)
    }
}

/// A persisted message, as read back from the store.
#[derive(Debug, Clone)]
pub struct StoredMessage {
    pub role: Role,
    pub content: String,
    pub model: Option<String>,
    pub tool_calls: Vec<ToolCall>,
    pub tool_call_id: Option<String>,
    /// `UiOnly` rows are user-facing notes; the context pipeline strips them from provider calls.
    pub visibility: Visibility,
}

/// One row of a user-facing transcript page (see [`Store::load_history_page`]) — the
/// remote-control full-scrollback seam.
#[derive(Debug, Clone)]
pub struct HistoryRow {
    pub seq: i64,
    pub role: Role,
    pub content: String,
    pub model: Option<String>,
    pub created_at: i64,
    /// `UiOnly` rows are user-facing notes; they belong in the visible conversation.
    pub visibility: Visibility,
    /// The tool behind a `Role::Tool` row, resolved from the assistant carrier that made the call
    /// (see [`Store::load_history_page_with`]). Always `None` on the default page (which has no
    /// tool rows) and `None` on a tool row whose carrier is no longer recoverable — never guessed.
    pub tool_name: Option<String>,
    /// Which half of a tool interaction this row is (see [`Store::load_history_page_with`]).
    /// `None` on every non-tool row, and on the whole default page.
    pub tool_phase: Option<ToolPhase>,
}

/// Which half of a tool interaction a `Role::Tool` [`HistoryRow`] is: the CALL the model made
/// (synthesized from the assistant carrier's `tool_calls_json`, content = an args summary) or the
/// RESULT row that answered it. The two are otherwise indistinguishable on the wire — same kind,
/// same tool name — and a client that renders a call as a result would be showing arguments as
/// output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolPhase {
    Call,
    Result,
}

impl ToolPhase {
    pub fn as_str(self) -> &'static str {
        match self {
            ToolPhase::Call => "call",
            ToolPhase::Result => "result",
        }
    }
}

/// Resolve a tool row's name out of the assistant carrier's `tool_calls_json`, matching on the
/// row's `tool_call_id`. The `tool_call` audit table records names too, but only keyed by the
/// carrier's message id — with parallel calls in one turn it cannot say WHICH result row is which,
/// while the carrier's `[{id, name, args}]` can. `None` on a missing/unparseable carrier or an id
/// that isn't in it: an unnamed tool row is honest, a mis-paired name is not.
fn tool_name_from_carrier(carrier_json: &str, tool_call_id: Option<&str>) -> Option<String> {
    let id = tool_call_id?;
    serde_json::from_str::<Vec<ToolCall>>(carrier_json)
        .ok()?
        .into_iter()
        .find(|call| call.id == id)
        .map(|call| call.name)
}

/// The calls an assistant carrier declared, in the order the model made them. An unparseable
/// carrier yields none rather than erroring the page: the rest of the transcript is still true.
fn parse_tool_calls(carrier_json: &str) -> Vec<ToolCall> {
    serde_json::from_str::<Vec<ToolCall>>(carrier_json).unwrap_or_default()
}

/// A tool call's arguments, compacted for a transcript row: the same JSON the model sent, with
/// every string value capped at [`MAX_CALL_ARG_VALUE_CHARS`] and the whole line at
/// [`MAX_CALL_ARGS_CHARS`]. Value-level capping comes first deliberately — truncating the raw JSON
/// alone would spend the whole budget on a `write_file` body and hide the `path` that says what
/// the call actually did. Elisions are marked with `…` so a reader can tell a capped value from a
/// short one; nothing is summarized or reworded.
fn tool_call_args_summary(args: &serde_json::Value) -> String {
    let capped = cap_json_strings(args);
    let line = serde_json::to_string(&capped).unwrap_or_else(|_| "{}".to_string());
    let mut chars = line.chars();
    let head: String = chars.by_ref().take(MAX_CALL_ARGS_CHARS).collect();
    if chars.next().is_some() {
        format!("{head}…")
    } else {
        head
    }
}

/// Recursively cap every string in a JSON value to [`MAX_CALL_ARG_VALUE_CHARS`] characters,
/// marking each cap with `…`. Keys are left alone (they are short and identify the argument).
fn cap_json_strings(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::String(s) => {
            let mut chars = s.chars();
            let head: String = chars.by_ref().take(MAX_CALL_ARG_VALUE_CHARS).collect();
            if chars.next().is_some() {
                serde_json::Value::from(format!("{head}…"))
            } else {
                serde_json::Value::from(head)
            }
        }
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.iter().map(cap_json_strings).collect())
        }
        serde_json::Value::Object(fields) => serde_json::Value::Object(
            fields
                .iter()
                .map(|(k, v)| (k.clone(), cap_json_strings(v)))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// One `forge tree` row: a session's display metadata and fork linkage.
#[derive(Debug, Clone)]
pub struct ForkNode {
    pub id: String,
    pub title: Option<String>,
    pub forked_from: Option<String>,
    pub forked_at_seq: Option<i64>,
    pub created_at: i64,
}

/// One message of a session enriched with its usage row, for `forge replay`. The token/cost
/// fields are `None` for messages that never produced a usage record (user/tool messages, or
/// assistant turns from before usage tracking existed).
#[derive(Debug, Clone)]
pub struct ReplayEntry {
    pub seq: i64,
    pub role: Role,
    pub content: String,
    pub model: Option<String>,
    pub created_at: i64,
    pub tool_calls: Vec<ToolCall>,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub cost_usd: Option<f64>,
}

/// One recorded `write_file`/`edit_file` tool call touching a file, as read back for
/// `forge blame` (docs/features/forge-blame.md). `path` is exactly what the model passed —
/// possibly relative to `session_cwd`, which the caller resolves the same way the tool did.
#[derive(Debug, Clone)]
pub struct FileEditRow {
    pub tool_name: String,
    pub args_json: String,
    pub path: String,
    pub session_id: String,
    pub session_cwd: String,
    pub model: Option<String>,
    pub seq: i64,
    pub created_at: i64,
}

/// The provenance context of a single turn, for `forge blame --line` (docs/features/forge-blame.md).
#[derive(Debug, Clone, Default)]
pub struct TurnContext {
    /// The nearest user prompt at or before the turn's `seq`.
    pub user_prompt: Option<String>,
    /// The assistant message's own content at that `seq` (its reasoning/summary text).
    pub assistant_content: Option<String>,
}

/// A persisted checkpoint (rewind point) of a session.
#[derive(Debug, Clone)]
pub struct CheckpointRow {
    pub id: String,
    /// User-given name, or `None` for an auto per-turn checkpoint.
    pub label: Option<String>,
    /// Transcript boundary: messages with `seq < this` survive a rewind to here.
    pub seq: i64,
    pub created_at: i64,
}

/// A one-line summary of a past session, for `forge sessions`.
#[derive(Debug, Clone)]
pub struct SessionSummary {
    pub id: String,
    pub cwd: String,
    pub permission_mode: String,
    pub created_at: i64,
    pub total_cost_usd: f64,
    pub message_count: i64,
    /// First user message, if any.
    pub preview: Option<String>,
    /// Unix seconds of the newest message (falls back to the session's creation time when the
    /// session has no messages). Drives the most-recently-used ordering + the picker's age column.
    pub last_activity: i64,
    /// Stored session title (`forge serve` names sessions; subagents store their agent name here).
    pub title: Option<String>,
    /// The isolated worktree this session runs in, if created with `worktree:true` (migration_0008).
    pub worktree_path: Option<String>,
    /// Whether the session has been explicitly archived ([`Store::archive_session`]). Always
    /// `false` from [`Store::list_sessions`] (which filters archived rows out); set from
    /// [`Store::list_sessions_for_resume`], which includes them.
    pub archived: bool,
}

// ---- Lattice: code-intelligence graph (code-intelligence.md) ----

/// A persisted source-file row in the Lattice graph.
#[derive(Debug, Clone)]
pub struct LatticeFileRow {
    pub id: String,
    pub repo_root: String,
    pub rel_path: String,
    pub lang: String,
    pub content_hash: String,
    pub parse_status: String,
}

/// A persisted symbol node.
#[derive(Debug, Clone)]
pub struct LatticeNodeRow {
    pub id: String,
    pub file_id: String,
    pub kind: String,
    pub name: String,
    pub qualname: Option<String>,
    pub signature: Option<String>,
    pub span_start: i64,
    pub span_end: i64,
    pub line_start: i64,
    pub pagerank: f64,
}

/// A persisted relationship edge.
#[derive(Debug, Clone)]
pub struct LatticeEdgeRow {
    pub id: String,
    pub src_id: String,
    pub dst_id: String,
    pub kind: String,
    pub unresolved_name: Option<String>,
}

/// A persisted reference / call site (resolved to a node by name-join at query time).
#[derive(Debug, Clone)]
pub struct LatticeRefRow {
    pub id: String,
    pub src_id: String,
    pub name: String,
    pub kind: String,
    pub line: i64,
}

/// Read a [`LatticeNodeRow`] from the first 10 columns of a row (id, file_id, kind, name, qualname,
/// signature, span_start, span_end, line_start, pagerank).
fn lattice_node_from_row(r: &rusqlite::Row) -> rusqlite::Result<LatticeNodeRow> {
    Ok(LatticeNodeRow {
        id: r.get(0)?,
        file_id: r.get(1)?,
        kind: r.get(2)?,
        name: r.get(3)?,
        qualname: r.get(4)?,
        signature: r.get(5)?,
        span_start: r.get(6)?,
        span_end: r.get(7)?,
        line_start: r.get(8)?,
        pagerank: r.get(9).unwrap_or(0.0),
    })
}

impl Store {
    /// Write an event for an active MCP agent session. Keeps only the last 2000 events per
    /// session (ring buffer) to bound disk usage on long runs.
    pub fn append_live_event(&self, session_id: &str, payload_json: &str) -> Result<()> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO live_event (session_id, payload_json) VALUES (?1, ?2)",
            (session_id, payload_json),
        )?;
        // Prune to the ring-buffer cap only once every LIVE_EVENT_PRUNE_EVERY appends. The old code
        // ran the correlated-subquery DELETE on EVERY insert (an O(n) scan per append on the hottest
        // write path); amortizing it keeps the buffer bounded without the per-event cost.
        let n = self
            .live_event_writes
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if n.is_multiple_of(LIVE_EVENT_PRUNE_EVERY) {
            conn.execute(
                "DELETE FROM live_event WHERE session_id = ?1 AND id <= (
                    SELECT id FROM live_event WHERE session_id = ?1 ORDER BY id DESC LIMIT 1 OFFSET ?2
                 )",
                (session_id, LIVE_EVENT_KEEP),
            )?;
        }
        Ok(())
    }

    /// Fetch all events for `session_id` with `id > after_id`, in order.
    pub fn live_events_after(&self, session_id: &str, after_id: i64) -> Result<Vec<(i64, String)>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare_cached(
            "SELECT id, payload_json FROM live_event WHERE session_id = ?1 AND id > ?2 ORDER BY id",
        )?;
        let rows = stmt.query_map((session_id, after_id), |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut res = Vec::new();
        for r in rows {
            res.push(r?);
        }
        Ok(res)
    }

    /// Mark a session as having an active MCP agent.
    pub fn set_session_agent_active(&self, session_id: &str, active: bool) -> Result<()> {
        self.lock()?.execute(
            "UPDATE session SET agent_active = ?1 WHERE id = ?2",
            (active as i64, session_id),
        )?;
        Ok(())
    }

    /// Clear agent_active on all sessions. Called at MCP server startup to reset flags left
    /// by processes that were SIGKILLed before their Drop guard could run.
    pub fn clear_all_agent_active(&self) -> Result<()> {
        self.lock()?
            .execute("UPDATE session SET agent_active = 0", [])?;
        Ok(())
    }

    /// Session IDs with agent_active = 1.
    pub fn active_agent_session_ids(&self) -> Result<Vec<String>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT id FROM session WHERE agent_active = 1 AND parent_session_id IS NULL",
        )?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        let mut res = Vec::new();
        for r in rows {
            res.push(r?);
        }
        Ok(res)
    }

    // --- forge schedule: recurring OS-timer-driven `forge run` registry ---

    /// Register a new schedule row. `id` is the caller-generated [`forge_types::new_id`] so the CLI
    /// can print/use it before (and regardless of) the store round-trip.
    #[allow(clippy::too_many_arguments)]
    pub fn add_schedule(
        &self,
        id: &str,
        task: &str,
        cwd: &str,
        mode: Option<&str>,
        model: Option<&str>,
        cron: &str,
    ) -> Result<()> {
        self.lock()?.execute(
            "INSERT INTO schedule (id, task, cwd, mode, model, cron) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            (id, task, cwd, mode, model, cron),
        )?;
        Ok(())
    }

    /// All registered schedules, oldest first.
    pub fn list_schedules(&self) -> Result<Vec<Schedule>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT id, task, cwd, mode, model, cron, enabled, created_at, last_run \
             FROM schedule ORDER BY created_at",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(Schedule {
                    id: r.get(0)?,
                    task: r.get(1)?,
                    cwd: r.get(2)?,
                    mode: r.get(3)?,
                    model: r.get(4)?,
                    cron: r.get(5)?,
                    enabled: r.get::<_, i64>(6)? != 0,
                    created_at: r.get(7)?,
                    last_run: r.get(8)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Schedule ids whose id starts with `prefix` (git-style prefix resolution, mirrors
    /// [`Store::matching_session_ids`]).
    pub fn matching_schedule_ids(&self, prefix: &str) -> Result<Vec<String>> {
        let conn = self.lock()?;
        let escaped = escape_like_pattern(prefix);
        let mut stmt =
            conn.prepare("SELECT id FROM schedule WHERE id LIKE ?1 || '%' ESCAPE '\\'")?;
        let rows = stmt.query_map([escaped], |row| row.get::<_, String>(0))?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    /// Delete a schedule row by its exact id. Returns `false` if no row matched.
    pub fn remove_schedule(&self, id: &str) -> Result<bool> {
        let n = self
            .lock()?
            .execute("DELETE FROM schedule WHERE id = ?1", [id])?;
        Ok(n > 0)
    }

    /// Flip a schedule's `enabled` flag. Pausing does NOT stop the OS timer by itself — the caller
    /// must uninstall/reinstall it (see `forge serve`'s schedules API); this only records the
    /// state `forge schedule list` reports. Returns `false` if no row matched.
    pub fn set_schedule_enabled(&self, id: &str, enabled: bool) -> Result<bool> {
        let n = self.lock()?.execute(
            "UPDATE schedule SET enabled = ?1 WHERE id = ?2",
            (i64::from(enabled), id),
        )?;
        Ok(n > 0)
    }

    /// Record the epoch-seconds timestamp of a schedule's most recent tick.
    pub fn set_schedule_last_run(&self, id: &str, at: i64) -> Result<()> {
        self.lock()?
            .execute("UPDATE schedule SET last_run = ?1 WHERE id = ?2", (at, id))?;
        Ok(())
    }

    // --- forge queue: the overnight-autopilot task queue ---

    /// Enqueue a task. `id` is caller-generated ([`forge_types::new_id`]) so the CLI can print it
    /// immediately; the row starts in `pending`.
    pub fn add_queue_task(
        &self,
        id: &str,
        task: &str,
        cwd: &str,
        mode: Option<&str>,
        model: Option<&str>,
        budget_usd: Option<f64>,
    ) -> Result<()> {
        self.lock()?.execute(
            "INSERT INTO queue_task (id, task, cwd, mode, model, budget_usd) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            (id, task, cwd, mode, model, budget_usd),
        )?;
        Ok(())
    }

    /// All queue tasks, oldest first. `cwd` filters to one project when given (a drain only runs
    /// the current repo's tasks; `forge queue list` shows everything with `None`).
    pub fn list_queue_tasks(&self, cwd: Option<&str>) -> Result<Vec<QueueTask>> {
        let conn = self.lock()?;
        let sql = "SELECT id, task, cwd, mode, model, budget_usd, status, created_at, \
                   started_at, finished_at, session_id, branch, summary, cost_usd, gate \
                   FROM queue_task";
        let map = |r: &rusqlite::Row<'_>| {
            Ok(QueueTask {
                id: r.get(0)?,
                task: r.get(1)?,
                cwd: r.get(2)?,
                mode: r.get(3)?,
                model: r.get(4)?,
                budget_usd: r.get(5)?,
                status: r.get(6)?,
                created_at: r.get(7)?,
                started_at: r.get(8)?,
                finished_at: r.get(9)?,
                session_id: r.get(10)?,
                branch: r.get(11)?,
                summary: r.get(12)?,
                cost_usd: r.get(13)?,
                gate: r.get(14)?,
            })
        };
        let rows = match cwd {
            Some(dir) => {
                let mut stmt =
                    conn.prepare(&format!("{sql} WHERE cwd = ?1 ORDER BY created_at"))?;
                let rows = stmt.query_map([dir], map)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()?
            }
            None => {
                let mut stmt = conn.prepare(&format!("{sql} ORDER BY created_at"))?;
                let rows = stmt.query_map([], map)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()?
            }
        };
        Ok(rows)
    }

    /// Queue-task ids starting with `prefix` (git-style prefix resolution).
    pub fn matching_queue_task_ids(&self, prefix: &str) -> Result<Vec<String>> {
        let conn = self.lock()?;
        let escaped = escape_like_pattern(prefix);
        let mut stmt =
            conn.prepare("SELECT id FROM queue_task WHERE id LIKE ?1 || '%' ESCAPE '\\'")?;
        let rows = stmt.query_map([escaped], |row| row.get::<_, String>(0))?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    /// Delete a queue task by exact id, but never one mid-run. Returns `false` if nothing matched
    /// (wrong id, or the row is `running`).
    pub fn remove_queue_task(&self, id: &str) -> Result<bool> {
        let n = self.lock()?.execute(
            "DELETE FROM queue_task WHERE id = ?1 AND status != 'running'",
            [id],
        )?;
        Ok(n > 0)
    }

    /// Move a pending task to `running`, stamping `started_at`. Returns `false` when the row was
    /// not pending (already claimed by a concurrent drain, or finished) — the caller skips it.
    pub fn claim_queue_task(&self, id: &str, at: i64) -> Result<bool> {
        let n = self.lock()?.execute(
            "UPDATE queue_task SET status = 'running', started_at = ?1 \
             WHERE id = ?2 AND status = 'pending'",
            (at, id),
        )?;
        Ok(n > 0)
    }

    // --- forge fork / forge tree: counterfactual session branching ---
    // (ForkNode is defined next to the other read-side row types below.)

    /// Branch a session at a turn boundary: create a new top-level session (same cwd + mode)
    /// carrying a copy of `src`'s *active* messages with `seq < at_seq`, linked back via
    /// `forked_from`/`forked_at_seq`. The re-asked prompt itself is NOT copied — the fork's next
    /// turn supplies it (possibly against a different model), which is the whole point.
    pub fn fork_session(&self, src: &str, at_seq: i64) -> Result<String> {
        let mut conn = self.lock()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (cwd, mode): (String, String) = tx.query_row(
            "SELECT cwd, permission_mode FROM session WHERE id = ?1",
            [src],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let new_id = forge_types::new_id();
        tx.execute(
            "INSERT INTO session (id, cwd, permission_mode, total_cost_usd, forked_from, forked_at_seq) \
             VALUES (?1, ?2, ?3, 0, ?4, ?5)",
            (&new_id, &cwd, &mode, src, at_seq),
        )?;
        {
            let mut read = tx.prepare(
                "SELECT seq, role, content, model, tool_calls_json, tool_call_id, visibility \
                 FROM message WHERE session_id = ?1 AND active = 1 AND seq < ?2 ORDER BY seq",
            )?;
            let mut write = tx.prepare(
                "INSERT INTO message (id, session_id, seq, role, content, model, tool_calls_json, tool_call_id, visibility) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            )?;
            let rows = read.query_map((src, at_seq), |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, Option<String>>(4)?,
                    r.get::<_, Option<String>>(5)?,
                    r.get::<_, String>(6)?,
                ))
            })?;
            for row in rows {
                let (seq, role, content, model, tcj, tcid, vis) = row?;
                write.execute((
                    forge_types::new_id(),
                    &new_id,
                    seq,
                    role,
                    content,
                    model,
                    tcj,
                    tcid,
                    vis,
                ))?;
            }
        }
        tx.commit()?;
        Ok(new_id)
    }

    /// `forge tree` shows conversations, not worker fan-out.
    pub fn fork_nodes(&self) -> Result<Vec<ForkNode>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT id, title, forked_from, forked_at_seq, created_at FROM session \
             WHERE parent_session_id IS NULL ORDER BY created_at, id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(ForkNode {
                id: r.get(0)?,
                title: r.get(1)?,
                forked_from: r.get(2)?,
                forked_at_seq: r.get(3)?,
                created_at: r.get(4)?,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(StoreError::from)
    }

    /// Record a finished task's outcome in one write.
    #[allow(clippy::too_many_arguments)]
    pub fn finish_queue_task(
        &self,
        id: &str,
        status: &str,
        at: i64,
        session_id: Option<&str>,
        branch: Option<&str>,
        summary: Option<&str>,
        cost_usd: Option<f64>,
        gate: Option<&str>,
    ) -> Result<()> {
        self.lock()?.execute(
            "UPDATE queue_task SET status = ?1, finished_at = ?2, session_id = ?3, \
             branch = ?4, summary = ?5, cost_usd = ?6, gate = ?7 WHERE id = ?8",
            (status, at, session_id, branch, summary, cost_usd, gate, id),
        )?;
        Ok(())
    }

    // --- saved-workflow run history (`/workflow run <name>`) ---

    /// Open a `workflow_run` row for a starting saved workflow. `id` is caller-generated
    /// ([`forge_types::new_id`]) so the caller can close the row out later without a round-trip.
    /// `cwd` is the workspace root the script runs against — the same key the workflow library
    /// screen lists scripts by, so one project's history never shows up under another's.
    ///
    /// Also sweeps this session's own long-dead `running` rows (see [`WORKFLOW_RUN_STALE_SECS`]):
    /// a row left open by a killed process is repaired the next time a workflow runs, so the
    /// projection [`list_workflow_runs`](Self::list_workflow_runs) applies on read doesn't have to
    /// be re-derived forever.
    pub fn start_workflow_run(
        &self,
        id: &str,
        name: &str,
        session_id: &str,
        cwd: &str,
    ) -> Result<()> {
        let stale_before = chrono::Utc::now().timestamp() - WORKFLOW_RUN_STALE_SECS;
        let conn = self.lock()?;
        conn.execute(
            "UPDATE workflow_run SET status = 'interrupted' \
             WHERE status = 'running' AND started_at < ?1",
            [stale_before],
        )?;
        conn.execute(
            "INSERT INTO workflow_run (id, name, session_id, cwd) VALUES (?1, ?2, ?3, ?4)",
            (id, name, session_id, cwd),
        )?;
        Ok(())
    }

    /// Close a run out with what it ended up doing. `ok` distinguishes `ok` from `failed`; the
    /// counts and cost are the run's own observed totals, not estimates (see
    /// `Session::run_saved_workflow`). No-op if the row is gone (its session was pruned).
    pub fn finish_workflow_run(
        &self,
        id: &str,
        ok: bool,
        summary: &str,
        phases: i64,
        agents: i64,
        cost_usd: f64,
    ) -> Result<()> {
        self.lock()?.execute(
            "UPDATE workflow_run \
             SET status = ?1, finished_at = ?2, summary = ?3, phases = ?4, agents = ?5, \
                 cost_usd = ?6 \
             WHERE id = ?7 AND status = 'running'",
            (
                if ok { "ok" } else { "failed" },
                chrono::Utc::now().timestamp(),
                summary,
                phases,
                agents,
                cost_usd,
                id,
            ),
        )?;
        Ok(())
    }

    /// Mark a run interrupted — the turn was aborted (Esc) or the process is shutting down, so no
    /// outcome exists. Unlike a crash this DOES know when the run stopped, so `finished_at` is
    /// recorded; a crash-interrupted row keeps a NULL `finished_at` because that moment was never
    /// observed. Guarded on `status = 'running'` so it can never overwrite a real outcome.
    pub fn interrupt_workflow_run(&self, id: &str) -> Result<()> {
        self.lock()?.execute(
            "UPDATE workflow_run SET status = 'interrupted', finished_at = ?1 \
             WHERE id = ?2 AND status = 'running'",
            (chrono::Utc::now().timestamp(), id),
        )?;
        Ok(())
    }

    /// The newest `limit` recorded runs of one workflow in one workspace, newest first.
    ///
    /// A `running` row older than [`WORKFLOW_RUN_STALE_SECS`] is REPORTED as `interrupted` (its
    /// `finished_at` stays NULL — the end time was never observed and is not invented). That is
    /// the read-side half of the staleness rule; [`start_workflow_run`](Self::start_workflow_run)
    /// writes the same verdict back to disk on the next run.
    pub fn list_workflow_runs(
        &self,
        name: &str,
        cwd: &str,
        limit: usize,
    ) -> Result<Vec<WorkflowRun>> {
        let stale_before = chrono::Utc::now().timestamp() - WORKFLOW_RUN_STALE_SECS;
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT id, name, session_id, cwd, started_at, finished_at, status, summary, \
                    phases, agents, cost_usd \
             FROM workflow_run WHERE name = ?1 AND cwd = ?2 \
             ORDER BY started_at DESC, rowid DESC LIMIT ?3",
        )?;
        let rows = stmt.query_map(rusqlite::params![name, cwd, limit as i64], |r| {
            let status: String = r.get(6)?;
            let started_at: i64 = r.get(4)?;
            Ok(WorkflowRun {
                id: r.get(0)?,
                name: r.get(1)?,
                session_id: r.get(2)?,
                cwd: r.get(3)?,
                started_at,
                finished_at: r.get(5)?,
                status: if status == "running" && started_at < stale_before {
                    "interrupted".to_string()
                } else {
                    status
                },
                summary: r.get(7)?,
                phases: r.get(8)?,
                agents: r.get(9)?,
                cost_usd: r.get(10)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }
}

/// One registered `forge schedule` row: a task, its working directory, and the cron/interval spec
/// driving the OS timer that fires `forge run <task>`.
#[derive(Debug, Clone, PartialEq)]
pub struct Schedule {
    pub id: String,
    pub task: String,
    pub cwd: String,
    pub mode: Option<String>,
    pub model: Option<String>,
    pub cron: String,
    pub enabled: bool,
    pub created_at: i64,
    pub last_run: Option<i64>,
}

/// One `forge queue` row: a queued headless task plus, once drained, its recorded outcome.
/// `status` lifecycle: `pending` → `running` → `done` / `empty` (ran clean but changed nothing) /
/// `gated` (assay gate tripped) / `over-budget` (killed at the cost cap, partial work kept) /
/// `failed`. `gate` holds the assay verdict line when a gate ran.
#[derive(Debug, Clone, PartialEq)]
pub struct QueueTask {
    pub id: String,
    pub task: String,
    pub cwd: String,
    pub mode: Option<String>,
    pub model: Option<String>,
    pub budget_usd: Option<f64>,
    pub status: String,
    pub created_at: i64,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub session_id: Option<String>,
    pub branch: Option<String>,
    pub summary: Option<String>,
    pub cost_usd: Option<f64>,
    pub gate: Option<String>,
}

/// One recorded run of a saved workflow (`/workflow run <name>`). `status` lifecycle:
/// `running` → `ok` / `failed` (the script returned) / `interrupted` (the turn was aborted, or the
/// process died mid-run — see [`Store::list_workflow_runs`]). `phases`/`agents`/`cost_usd` are the
/// run's own observed totals: phases announced, agents started, and cost summed over the agents
/// that reported one — never estimates, and 0 on a run that reported none.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowRun {
    pub id: String,
    pub name: String,
    pub session_id: String,
    /// Workspace root the script ran against, so a same-named workflow in another project is a
    /// different history.
    pub cwd: String,
    pub started_at: i64,
    /// `None` while running, and on a run interrupted by a crash (the end moment was never seen).
    pub finished_at: Option<i64>,
    pub status: String,
    pub summary: Option<String>,
    pub phases: i64,
    pub agents: i64,
    pub cost_usd: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_reservation_conflicts_for_shared_database_but_not_distinct_contexts() {
        let root = std::env::temp_dir();
        let shared_path = root.join(format!("forge-store-shared-{}.db", std::process::id()));
        let other_path = root.join(format!("forge-store-other-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&shared_path);
        let _ = std::fs::remove_file(&other_path);
        let first_store = Store::open(&shared_path).unwrap();
        let second_store = Store::open(&shared_path).unwrap();
        let other_store = Store::open(&other_path).unwrap();
        let reservation = first_store.try_reserve_model("openai::gpt-4o").unwrap();

        assert!(second_store.try_reserve_model("openai::gpt-4o").is_none());
        assert!(other_store.try_reserve_model("openai::gpt-4o").is_some());

        let first_memory = Store::open_in_memory().unwrap();
        let second_memory = Store::open_in_memory().unwrap();
        let memory_reservation = first_memory.try_reserve_model("openai::gpt-4o").unwrap();
        assert!(second_memory.try_reserve_model("openai::gpt-4o").is_some());

        drop(memory_reservation);
        drop(reservation);
        let _ = std::fs::remove_file(&shared_path);
        let _ = std::fs::remove_file(&other_path);
    }

    #[test]
    fn model_reservation_is_atomic_and_released_on_drop() {
        let store = Store::open_in_memory().unwrap();
        let first = store.try_reserve_model("openai::gpt-4o").unwrap();
        assert!(store.is_model_reserved("openai::gpt-4o"));
        assert!(store.try_reserve_model("openai::gpt-4o").is_none());

        drop(first);
        assert!(!store.is_model_reserved("openai::gpt-4o"));
        assert!(store.try_reserve_model("openai::gpt-4o").is_some());
    }

    #[test]
    fn view_snapshot_persists_per_session() {
        let store = Store::open_in_memory().unwrap();
        let sid = store.create_session("/tmp", "default").unwrap();
        // Absent until written.
        assert_eq!(store.session_view_snapshot(&sid).unwrap(), None);
        store
            .update_session_view_snapshot(&sid, r#"{"viewer":{"selected":2}}"#)
            .unwrap();
        assert_eq!(
            store.session_view_snapshot(&sid).unwrap().as_deref(),
            Some(r#"{"viewer":{"selected":2}}"#)
        );
    }

    #[test]
    fn persist_a_turn() {
        let store = Store::open_in_memory().unwrap();
        let sid = store.create_session("/tmp", "default").unwrap();

        let mid = store
            .add_message(&sid, 0, Role::User, "hello", None)
            .unwrap();
        store
            .record_routing(
                &mid,
                TaskTier::Standard,
                "openai::gpt-4o-mini",
                "medium prompt",
            )
            .unwrap();
        store
            .record_usage(
                &sid,
                &mid,
                &Usage {
                    input_tokens: 10,
                    output_tokens: 5,
                    cached_input_tokens: 7,
                    cost_usd: 0.02,
                },
            )
            .unwrap();
        store
            .record_tool_call(&mid, "read_file", "{}", "ok", "allowed", "ok")
            .unwrap();

        assert_eq!(store.message_count(&sid).unwrap(), 1);
        assert!((store.session_cost(&sid).unwrap() - 0.02).abs() < 1e-9);
        assert_eq!(store.session_cached_input_tokens(&sid).unwrap(), 7);
    }

    fn record_cost(store: &Store, cost: f64) {
        let sid = store.create_session("/tmp", "default").unwrap();
        let mid = store.add_message(&sid, 0, Role::User, "x", None).unwrap();
        store
            .record_usage(
                &sid,
                &mid,
                &Usage {
                    input_tokens: 1,
                    output_tokens: 1,
                    cached_input_tokens: 0,
                    cost_usd: cost,
                },
            )
            .unwrap();
    }

    #[test]
    fn spend_today_sums_across_sessions() {
        // AC-1: the day total aggregates usage across DIFFERENT sessions, not one session's
        // running total.
        let store = Store::open_in_memory().unwrap();
        record_cost(&store, 0.06);
        record_cost(&store, 0.05);
        let today = store.spend_today_usd().unwrap();
        assert!(
            (today - 0.11).abs() < 1e-9,
            "summed across sessions: {today}"
        );
    }

    #[test]
    fn spend_between_excludes_out_of_window_rows() {
        let store = Store::open_in_memory().unwrap();
        record_cost(&store, 0.03);
        assert_eq!(
            store.spend_between(0, 1).unwrap(),
            0.0,
            "a 1970 window excludes today's row"
        );
        let (s, e) = day_bounds_local(Local::now());
        assert!(
            store.spend_between(s, e).unwrap() > 0.0,
            "today's window includes it"
        );
    }

    #[test]
    fn day_bounds_are_24h_and_exclude_prior_day() {
        let now = Local.with_ymd_and_hms(2026, 6, 15, 13, 30, 0).unwrap();
        let (s, e) = day_bounds_local(now);
        assert_eq!(e - s, 86_400, "a day is 24h (no DST on this date)");
        assert!(now.timestamp() >= s && now.timestamp() < e);
        let prev = Local.with_ymd_and_hms(2026, 6, 14, 23, 0, 0).unwrap();
        assert!(prev.timestamp() < s, "yesterday is excluded (AC-4)");
    }

    #[test]
    fn month_bounds_exclude_prior_month() {
        let now = Local.with_ymd_and_hms(2026, 6, 15, 12, 0, 0).unwrap();
        let (s, e) = month_bounds_local(now);
        assert!(now.timestamp() >= s && now.timestamp() < e);
        let jun1 = Local.with_ymd_and_hms(2026, 6, 1, 0, 0, 0).unwrap();
        assert_eq!(
            s,
            jun1.timestamp(),
            "window starts at the first of the month"
        );
        let may = Local.with_ymd_and_hms(2026, 5, 31, 23, 0, 0).unwrap();
        assert!(may.timestamp() < s, "May is excluded from June (AC-3)");
    }

    #[test]
    fn tool_linkage_round_trips() {
        let store = Store::open_in_memory().unwrap();
        let sid = store.create_session("/tmp", "default").unwrap();
        let calls = vec![ToolCall {
            id: "c1".into(),
            name: "read_file".into(),
            args: serde_json::json!({ "path": "x" }),
        }];
        store
            .add_message_full(&sid, 0, Role::Assistant, "calling", Some("m"), &calls, None)
            .unwrap();
        store
            .add_message_full(&sid, 1, Role::Tool, "result", None, &[], Some("c1"))
            .unwrap();

        let msgs = store.load_messages(&sid).unwrap();
        assert_eq!(msgs[0].tool_calls.len(), 1);
        assert_eq!(msgs[0].tool_calls[0].name, "read_file");
        assert_eq!(msgs[1].tool_call_id.as_deref(), Some("c1"));
    }

    #[test]
    fn load_replay_joins_usage_and_orders_by_seq() {
        let store = Store::open_in_memory().unwrap();
        let sid = store.create_session("/tmp", "default").unwrap();
        store.add_message(&sid, 0, Role::User, "ask", None).unwrap();
        let mid = store
            .add_message(&sid, 1, Role::Assistant, "answer", Some("openai::gpt-4o"))
            .unwrap();
        store
            .record_usage(
                &sid,
                &mid,
                &Usage {
                    input_tokens: 12,
                    output_tokens: 7,
                    cached_input_tokens: 0,
                    cost_usd: 0.03,
                },
            )
            .unwrap();

        let entries = store.load_replay(&sid).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].seq, 0);
        assert_eq!(entries[0].role, Role::User);
        assert!(entries[0].cost_usd.is_none(), "user turn has no usage row");
        assert_eq!(entries[1].model.as_deref(), Some("openai::gpt-4o"));
        assert_eq!(entries[1].input_tokens, Some(12));
        assert!((entries[1].cost_usd.unwrap() - 0.03).abs() < 1e-9);
    }

    #[test]
    fn load_messages_returns_seq_order() {
        let store = Store::open_in_memory().unwrap();
        let sid = store.create_session("/tmp", "default").unwrap();
        // Insert out of order; load must sort by seq.
        store
            .add_message(&sid, 2, Role::Tool, "tool result", None)
            .unwrap();
        store
            .add_message(&sid, 0, Role::User, "do the thing", None)
            .unwrap();
        store
            .add_message(&sid, 1, Role::Assistant, "on it", Some("opus"))
            .unwrap();

        let msgs = store.load_messages(&sid).unwrap();
        let roles: Vec<_> = msgs.iter().map(|m| m.role).collect();
        assert_eq!(roles, vec![Role::User, Role::Assistant, Role::Tool]);
        assert_eq!(msgs[0].content, "do the thing");
        assert_eq!(msgs[1].model.as_deref(), Some("opus"));
    }

    #[test]
    fn ui_notes_round_trip_their_visibility() {
        let store = Store::open_in_memory().unwrap();
        let sid = store.create_session("/tmp", "default").unwrap();
        store
            .add_message(&sid, 0, Role::User, "do the thing", None)
            .unwrap();
        store
            .add_ui_note(&sid, 1, Role::System, "⚠ budget cap reached")
            .unwrap();

        let msgs = store.load_messages(&sid).unwrap();
        assert_eq!(msgs[0].visibility, Visibility::Llm);
        assert_eq!(msgs[1].visibility, Visibility::UiOnly);
        // The full-history read keeps the tag too (scrollback still shows the note).
        let all = store.load_all_messages(&sid).unwrap();
        assert_eq!(all[1].visibility, Visibility::UiOnly);

        // Forks copy the tag: a UI note in the prefix must not become model context in the fork.
        let fork = store.fork_session(&sid, 2).unwrap();
        let forked = store.load_messages(&fork).unwrap();
        assert_eq!(forked[1].visibility, Visibility::UiOnly);
    }

    #[test]
    fn list_sessions_newest_first_with_preview_and_count() {
        let store = Store::open_in_memory().unwrap();

        let a = store.create_session("/a", "default").unwrap();
        store
            .add_message(&a, 0, Role::User, "first task", None)
            .unwrap();

        let b = store.create_session("/b", "plan").unwrap();
        store
            .add_message(&b, 0, Role::User, "second task", None)
            .unwrap();
        store
            .add_message(&b, 1, Role::Assistant, "working", Some("opus"))
            .unwrap();

        let sessions = store.list_sessions().unwrap();
        assert_eq!(sessions.len(), 2);
        // Newest (b) first.
        assert_eq!(sessions[0].id, b);
        assert_eq!(sessions[0].preview.as_deref(), Some("second task"));
        assert_eq!(sessions[0].message_count, 2);
        assert_eq!(sessions[1].id, a);
        assert_eq!(sessions[1].message_count, 1);
    }

    #[test]
    fn history_page_is_newest_first_windowed_and_user_facing() {
        let store = Store::open_in_memory().unwrap();
        let sid = store.create_session("/x", "default").unwrap();
        // seq 0..=9: user/assistant alternating, plus a tool row, an empty assistant
        // (tool-call carrier), a system row, and a ui note — only the user-facing rows page.
        store.add_message(&sid, 0, Role::User, "q0", None).unwrap();
        store
            .add_message(&sid, 1, Role::Assistant, "a1", Some("m1"))
            .unwrap();
        store
            .add_message(&sid, 2, Role::Tool, "tool output — plumbing", None)
            .unwrap();
        store
            .add_message(&sid, 3, Role::Assistant, "", None)
            .unwrap();
        store
            .add_message(&sid, 4, Role::System, "system prompt — plumbing", None)
            .unwrap();
        store
            .add_ui_note(&sid, 5, Role::System, "⚠ budget note")
            .unwrap();
        store.add_message(&sid, 6, Role::User, "q6", None).unwrap();
        store
            .add_message(&sid, 7, Role::Assistant, "a7", None)
            .unwrap();

        // Newest page: newest first, plumbing rows (tool / empty / system) excluded, ui included.
        let page = store.load_history_page(&sid, None, 10).unwrap();
        let seqs: Vec<i64> = page.iter().map(|r| r.seq).collect();
        assert_eq!(seqs, vec![7, 6, 5, 1, 0], "newest-first, user-facing only");
        assert_eq!(
            page[2].visibility,
            Visibility::UiOnly,
            "ui notes ride along"
        );
        assert_eq!(page[3].model.as_deref(), Some("m1"));
        assert!(
            page.iter().all(|r| r.created_at > 0),
            "created_at populated"
        );

        // `limit` caps the page; `before` opens the next window strictly below it.
        let first = store.load_history_page(&sid, None, 2).unwrap();
        assert_eq!(first.iter().map(|r| r.seq).collect::<Vec<_>>(), vec![7, 6]);
        let next = store
            .load_history_page(&sid, Some(first.last().unwrap().seq), 2)
            .unwrap();
        assert_eq!(next.iter().map(|r| r.seq).collect::<Vec<_>>(), vec![5, 1]);
        let last = store.load_history_page(&sid, Some(1), 10).unwrap();
        assert_eq!(last.iter().map(|r| r.seq).collect::<Vec<_>>(), vec![0]);
        assert!(store
            .load_history_page(&sid, Some(0), 10)
            .unwrap()
            .is_empty());

        // Session scoping: another session's rows never leak into the page.
        let other = store.create_session("/y", "default").unwrap();
        store
            .add_message(&other, 0, Role::User, "other q", None)
            .unwrap();
        let page = store.load_history_page(&sid, None, 10).unwrap();
        assert_eq!(page.len(), 5, "other session's rows excluded");
    }

    #[test]
    fn history_page_includes_tool_rows_only_when_asked_and_names_them() {
        let store = Store::open_in_memory().unwrap();
        let sid = store.create_session("/x", "default").unwrap();
        // One realistic turn: a user prompt, an assistant carrier announcing TWO parallel calls,
        // and the two result rows that answer them.
        store.add_message(&sid, 0, Role::User, "q", None).unwrap();
        let calls = [
            ToolCall {
                id: "call-a".into(),
                name: "read_file".into(),
                args: serde_json::json!({"path": "a.rs"}),
            },
            ToolCall {
                id: "call-b".into(),
                name: "shell".into(),
                args: serde_json::json!({"command": "ls"}),
            },
        ];
        store
            .add_message_full(&sid, 1, Role::Assistant, "looking…", None, &calls, None)
            .unwrap();
        store
            .add_message_full(
                &sid,
                2,
                Role::Tool,
                "a.rs contents",
                None,
                &[],
                Some("call-a"),
            )
            .unwrap();
        store
            .add_message_full(&sid, 3, Role::Tool, "a.rs  b.rs", None, &[], Some("call-b"))
            .unwrap();
        // An orphan: the carrier that requested it is gone (pruned/never written), so its name is
        // not recoverable and must not be guessed.
        store
            .add_message_full(&sid, 4, Role::Tool, "???", None, &[], Some("call-gone"))
            .unwrap();
        // Provider-continuity plumbing stays out of BOTH modes.
        store
            .add_llm_only_message_full(
                &sid,
                5,
                Role::Tool,
                "provisional",
                None,
                &[],
                Some("call-a"),
            )
            .unwrap();
        store
            .add_message(&sid, 6, Role::Assistant, "done", None)
            .unwrap();

        // Default: byte-for-byte the old page — no tool rows, no names.
        let plain = store.load_history_page(&sid, None, 20).unwrap();
        assert_eq!(
            plain.iter().map(|r| r.seq).collect::<Vec<_>>(),
            vec![6, 1, 0],
            "tool rows stay out of the default page"
        );
        assert!(plain.iter().all(|r| r.tool_name.is_none()));
        assert!(
            plain.iter().all(|r| r.tool_phase.is_none()),
            "and neither half of a tool interaction is even describable there"
        );

        // Opted in: the result rows appear, each named from its carrier's tool_calls_json — and
        // the carrier's own calls become rows of their own, at the carrier's seq (so each sorts
        // before the result answering it) and after its prose.
        let with_tools = store.load_history_page_with(&sid, None, 20, true).unwrap();
        assert_eq!(
            with_tools.iter().map(|r| r.seq).collect::<Vec<_>>(),
            vec![6, 4, 3, 2, 1, 1, 1, 0],
            "tool rows join the page; the llm_only one is still plumbing"
        );
        let named: Vec<(i64, Option<&str>)> = with_tools
            .iter()
            .filter(|r| r.role == Role::Tool && r.tool_phase == Some(ToolPhase::Result))
            .map(|r| (r.seq, r.tool_name.as_deref()))
            .collect();
        assert_eq!(
            named,
            vec![(4, None), (3, Some("shell")), (2, Some("read_file"))],
            "parallel calls resolve per tool_call_id, and an orphan stays unnamed"
        );

        // Newest-first, so reversing gives the chronological read: prose, then the calls in the
        // order the model made them, then each result.
        let chronological: Vec<(i64, &str, Option<&str>, Option<ToolPhase>)> = with_tools
            .iter()
            .rev()
            .map(|r| {
                (
                    r.seq,
                    r.content.as_str(),
                    r.tool_name.as_deref(),
                    r.tool_phase,
                )
            })
            .collect();
        assert_eq!(
            chronological,
            vec![
                (0, "q", None, None),
                (1, "looking…", None, None),
                (
                    1,
                    r#"{"path":"a.rs"}"#,
                    Some("read_file"),
                    Some(ToolPhase::Call)
                ),
                (
                    1,
                    r#"{"command":"ls"}"#,
                    Some("shell"),
                    Some(ToolPhase::Call)
                ),
                (
                    2,
                    "a.rs contents",
                    Some("read_file"),
                    Some(ToolPhase::Result)
                ),
                (3, "a.rs  b.rs", Some("shell"), Some(ToolPhase::Result)),
                (4, "???", None, Some(ToolPhase::Result)),
                (6, "done", None, None),
            ],
            "every call precedes the result that answers it"
        );
        // A call row is tool activity, not the carrier's assistant turn, and carries no model —
        // it renders exactly like the result row next to it.
        let calls: Vec<&HistoryRow> = with_tools
            .iter()
            .filter(|r| r.tool_phase == Some(ToolPhase::Call))
            .collect();
        assert!(calls
            .iter()
            .all(|r| r.role == Role::Tool && r.model.is_none()));

        // Pagination still windows correctly with tools in the set. `limit` bounds the DB rows
        // read, so a page containing a carrier hands back MORE rows than it asked for — but every
        // one carries a real seq, so walking `before` from the oldest of them neither skips nor
        // repeats a row.
        let first = store.load_history_page_with(&sid, None, 2, true).unwrap();
        assert_eq!(first.iter().map(|r| r.seq).collect::<Vec<_>>(), vec![6, 4]);
        let next = store
            .load_history_page_with(&sid, Some(4), 2, true)
            .unwrap();
        assert_eq!(next.iter().map(|r| r.seq).collect::<Vec<_>>(), vec![3, 2]);
        let last = store
            .load_history_page_with(&sid, Some(2), 2, true)
            .unwrap();
        assert_eq!(
            last.iter().map(|r| r.seq).collect::<Vec<_>>(),
            vec![1, 1, 1, 0],
            "the carrier's two calls and its prose all belong to seq 1"
        );
        assert!(store
            .load_history_page_with(&sid, Some(0), 2, true)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn history_page_keeps_a_call_whose_result_never_arrived() {
        // An interrupted turn: the carrier is persisted, the tool never got to answer. The call
        // is real (the model made it) and is the only trace of it, so dropping it would erase the
        // most interesting row of the transcript.
        let store = Store::open_in_memory().unwrap();
        let sid = store.create_session("/x", "default").unwrap();
        store.add_message(&sid, 0, Role::User, "q", None).unwrap();
        store
            .add_message_full(
                &sid,
                1,
                Role::Assistant,
                "",
                Some("m1"),
                &[ToolCall {
                    id: "call-a".into(),
                    name: "shell".into(),
                    args: serde_json::json!({"command": "cargo test"}),
                }],
                None,
            )
            .unwrap();

        // The empty carrier still contributes nothing to the default page.
        assert_eq!(
            store
                .load_history_page(&sid, None, 10)
                .unwrap()
                .iter()
                .map(|r| r.seq)
                .collect::<Vec<_>>(),
            vec![0]
        );

        let page = store.load_history_page_with(&sid, None, 10, true).unwrap();
        assert_eq!(
            page.iter()
                .map(|r| (r.seq, r.tool_name.as_deref(), r.tool_phase))
                .collect::<Vec<_>>(),
            vec![(1, Some("shell"), Some(ToolPhase::Call)), (0, None, None)],
            "the unanswered call rides alone; the empty carrier itself never becomes a row"
        );
        assert_eq!(page[0].content, r#"{"command":"cargo test"}"#);
    }

    #[test]
    fn tool_call_args_are_summarized_without_leaking_a_file_body() {
        // A `write_file` call is the worst case: the body is arbitrarily large and the `path` is
        // the part that says what happened. Capping values first keeps the path visible.
        let body = "x".repeat(10_000);
        let summary = tool_call_args_summary(&serde_json::json!({
            "path": "src/lib.rs",
            "content": body,
        }));
        assert!(summary.chars().count() <= MAX_CALL_ARGS_CHARS + 1);
        assert!(summary.contains(r#""path":"src/lib.rs""#));
        assert!(
            summary.matches('x').count() <= MAX_CALL_ARG_VALUE_CHARS,
            "no more of the body than one capped value: {summary}"
        );
        assert!(summary.contains('…'), "the elision is visible: {summary}");

        // Short args survive verbatim — a summary that rewrote them would misreport the call.
        assert_eq!(
            tool_call_args_summary(&serde_json::json!({"path": "a.rs"})),
            r#"{"path":"a.rs"}"#
        );
        assert_eq!(tool_call_args_summary(&serde_json::json!({})), "{}");
        // Nested and non-object args are capped the same way, never dropped.
        let nested = tool_call_args_summary(&serde_json::json!({
            "edits": [{"old": "y".repeat(500), "new": "z"}],
        }));
        assert!(nested.contains(r#""new":"z""#), "{nested}");
        assert!(nested.matches('y').count() <= MAX_CALL_ARG_VALUE_CHARS);
    }

    #[test]
    fn history_epoch_matches_the_row_set_the_page_returns() {
        // The scrubber's zero point must be the first row of the SAME set — otherwise including
        // tools would shift every elapsed_ms on the wire.
        let store = Store::open_in_memory().unwrap();
        let sid = store.create_session("/x", "default").unwrap();
        let stamp = |seq: i64, at: i64| {
            store
                .lock()
                .unwrap()
                .execute(
                    "UPDATE message SET created_at = ?1 WHERE session_id = ?2 AND seq = ?3",
                    rusqlite::params![at, &sid, seq],
                )
                .unwrap();
        };
        // A tool row is the OLDEST visible row here (a resumed session whose earlier turns were
        // pruned), so the two epochs genuinely differ.
        store
            .add_message_full(&sid, 0, Role::Tool, "result", None, &[], Some("call-a"))
            .unwrap();
        stamp(0, 1_000);
        store.add_message(&sid, 1, Role::User, "q", None).unwrap();
        stamp(1, 1_010);

        assert_eq!(
            store.history_epoch(&sid).unwrap(),
            Some(1_010),
            "the default page starts at the first user/assistant row"
        );
        assert_eq!(
            store.history_epoch_with(&sid, true).unwrap(),
            Some(1_000),
            "an include_tools page starts at the tool row it actually returns"
        );
        // And each epoch really is the minimum of its own page.
        for tools in [false, true] {
            let page = store.load_history_page_with(&sid, None, 20, tools).unwrap();
            assert_eq!(
                page.iter().map(|r| r.created_at).min(),
                store.history_epoch_with(&sid, tools).unwrap(),
                "epoch must be the oldest row of the set it describes"
            );
        }

        // The widened set includes the call rows, which sit at their CARRIER's timestamp — an
        // empty carrier is invisible to the default page but can be the oldest row of an
        // include_tools one, and the epoch has to follow.
        let other = store.create_session("/y", "default").unwrap();
        other_stamped_carrier(&store, &other);
        assert_eq!(
            store.history_epoch(&other).unwrap(),
            Some(2_020),
            "the carrier is invisible without tools"
        );
        assert_eq!(
            store.history_epoch_with(&other, true).unwrap(),
            Some(2_000),
            "with tools the call row is the zero point"
        );
        for tools in [false, true] {
            let page = store
                .load_history_page_with(&other, None, 20, tools)
                .unwrap();
            assert_eq!(
                page.iter().map(|r| r.created_at).min(),
                store.history_epoch_with(&other, tools).unwrap()
            );
        }
    }

    /// A session that OPENS with a tool call: an empty carrier at t=2000, its result at t=2010,
    /// and the model's reply at t=2020 (the only row the default page can see).
    fn other_stamped_carrier(store: &Store, sid: &str) {
        store
            .add_message_full(
                sid,
                0,
                Role::Assistant,
                "",
                None,
                &[ToolCall {
                    id: "call-a".into(),
                    name: "read_file".into(),
                    args: serde_json::json!({"path": "a.rs"}),
                }],
                None,
            )
            .unwrap();
        store
            .add_message_full(sid, 1, Role::Tool, "contents", None, &[], Some("call-a"))
            .unwrap();
        store
            .add_message(sid, 2, Role::Assistant, "done", None)
            .unwrap();
        let conn = store.lock().unwrap();
        for (seq, at) in [(0_i64, 2_000_i64), (1, 2_010), (2, 2_020)] {
            conn.execute(
                "UPDATE message SET created_at = ?1 WHERE session_id = ?2 AND seq = ?3",
                rusqlite::params![at, sid, seq],
            )
            .unwrap();
        }
    }

    #[test]
    fn workflow_runs_record_their_outcome_and_stay_scoped_to_one_workspace() {
        let store = Store::open_in_memory().unwrap();
        let sid = store.create_session("/repo", "default").unwrap();
        let other_sid = store.create_session("/other", "default").unwrap();

        store
            .start_workflow_run("run-1", "audit", &sid, "/repo")
            .unwrap();
        // Still open: no outcome to report yet, and none invented.
        let live = store.list_workflow_runs("audit", "/repo", 10).unwrap();
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].status, "running");
        assert_eq!(live[0].finished_at, None);
        assert_eq!(live[0].summary, None);
        assert_eq!(live[0].session_id, sid);

        store
            .finish_workflow_run("run-1", true, "AUDIT_OK", 3, 5, 0.42)
            .unwrap();
        let done = store.list_workflow_runs("audit", "/repo", 10).unwrap();
        assert_eq!(done[0].status, "ok");
        assert_eq!(done[0].summary.as_deref(), Some("AUDIT_OK"));
        assert_eq!((done[0].phases, done[0].agents), (3, 5));
        assert!((done[0].cost_usd - 0.42).abs() < f64::EPSILON);
        assert!(done[0].finished_at.is_some());

        // A finished run is terminal: a late writer can't reopen or relabel it.
        store
            .finish_workflow_run("run-1", false, "later", 0, 0, 0.0)
            .unwrap();
        store.interrupt_workflow_run("run-1").unwrap();
        assert_eq!(
            store.list_workflow_runs("audit", "/repo", 10).unwrap()[0].status,
            "ok"
        );

        // Same workflow NAME in another project is a different history.
        store
            .start_workflow_run("run-2", "audit", &other_sid, "/other")
            .unwrap();
        store
            .finish_workflow_run("run-2", false, "boom", 1, 1, 0.0)
            .unwrap();
        assert_eq!(
            store
                .list_workflow_runs("audit", "/repo", 10)
                .unwrap()
                .iter()
                .map(|r| r.id.as_str())
                .collect::<Vec<_>>(),
            vec!["run-1"]
        );
        assert_eq!(
            store.list_workflow_runs("audit", "/other", 10).unwrap()[0].status,
            "failed"
        );
        assert!(store
            .list_workflow_runs("never-run", "/repo", 10)
            .unwrap()
            .is_empty());

        // Newest first, capped.
        for i in 3..8 {
            let id = format!("run-{i}");
            store
                .start_workflow_run(&id, "audit", &sid, "/repo")
                .unwrap();
            store
                .finish_workflow_run(&id, true, "ok", 0, 0, 0.0)
                .unwrap();
        }
        let capped = store.list_workflow_runs("audit", "/repo", 3).unwrap();
        assert_eq!(
            capped.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            vec!["run-7", "run-6", "run-5"],
            "newest first (same-second runs break the tie by insertion order)"
        );

        // The run belongs to its session's history: pruning the session takes it along rather
        // than leaving a row pointing at a transcript that no longer exists.
        store
            .lock()
            .unwrap()
            .execute("DELETE FROM session WHERE id = ?1", [&sid])
            .unwrap();
        assert!(store
            .list_workflow_runs("audit", "/repo", 10)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn an_interrupted_workflow_run_never_keeps_claiming_it_is_running() {
        let store = Store::open_in_memory().unwrap();
        let sid = store.create_session("/repo", "default").unwrap();

        // Esc mid-run (or shutdown): the end moment IS observed, so it is recorded.
        store
            .start_workflow_run("run-esc", "audit", &sid, "/repo")
            .unwrap();
        store.interrupt_workflow_run("run-esc").unwrap();
        let esc = store.list_workflow_runs("audit", "/repo", 10).unwrap();
        assert_eq!(esc[0].status, "interrupted");
        assert!(esc[0].finished_at.is_some());
        assert_eq!(esc[0].summary, None, "there is no outcome to summarize");

        // A killed process leaves the row open. Backdate it past the horizon: the reader reports
        // `interrupted` WITHOUT inventing a finish time it never saw.
        store
            .start_workflow_run("run-crash", "build", &sid, "/repo")
            .unwrap();
        store
            .lock()
            .unwrap()
            .execute(
                "UPDATE workflow_run SET started_at = ?1 WHERE id = 'run-crash'",
                [chrono::Utc::now().timestamp() - WORKFLOW_RUN_STALE_SECS - 1],
            )
            .unwrap();
        let crashed = store.list_workflow_runs("build", "/repo", 10).unwrap();
        assert_eq!(crashed[0].status, "interrupted");
        assert_eq!(crashed[0].finished_at, None);

        // A run that started moments ago is genuinely live and must be left alone.
        store
            .start_workflow_run("run-live", "build", &sid, "/repo")
            .unwrap();
        assert_eq!(
            store.list_workflow_runs("build", "/repo", 10).unwrap()[0].status,
            "running"
        );
        // …and starting it swept the crashed row's verdict to disk, so the projection isn't the
        // only thing keeping the table honest.
        let on_disk: String = store
            .lock()
            .unwrap()
            .query_row(
                "SELECT status FROM workflow_run WHERE id = 'run-crash'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(on_disk, "interrupted");
    }

    #[test]
    fn history_page_keeps_compacted_away_rows_for_the_user() {
        // Compaction soft-deletes old rows from the MODEL's view; the user's scrollback (and so
        // the remote history page) still shows them.
        let store = Store::open_in_memory().unwrap();
        let sid = store.create_session("/x", "default").unwrap();
        for i in 0_i64..6 {
            let role = if i.rem_euclid(2) == 0 {
                Role::User
            } else {
                Role::Assistant
            };
            store
                .add_message(&sid, i, role, &format!("m{i}"), None)
                .unwrap();
        }
        store.compact_session_store(&sid, "SUMMARY", 2).unwrap();
        let page = store.load_history_page(&sid, None, 10).unwrap();
        assert_eq!(page.len(), 6, "soft-deleted rows still page for the user");
    }

    #[test]
    fn usage_by_provider_derives_provider_from_message_model() {
        // Regression: `usage.provider` is never written at insert. Aggregation must derive the
        // provider from the linked `message.model` namespace, not group on the NULL column (which
        // collapsed every row into one bucket and read back as "no usage yet").
        let store = Store::open_in_memory().unwrap();
        let sid = store.create_session("/x", "default").unwrap();
        let usage = |i: u64, o: u64, cached: u64| Usage {
            input_tokens: i,
            output_tokens: o,
            cached_input_tokens: cached,
            cost_usd: 0.0,
        };
        let m0 = store
            .add_message(
                &sid,
                0,
                Role::Assistant,
                "a",
                Some("codex-oauth::gpt-5.6-terra"),
            )
            .unwrap();
        store.record_usage(&sid, &m0, &usage(100, 10, 60)).unwrap();
        let m1 = store
            .add_message(
                &sid,
                1,
                Role::Assistant,
                "b",
                Some("codex-oauth::gpt-5.6-terra"),
            )
            .unwrap();
        store.record_usage(&sid, &m1, &usage(50, 5, 25)).unwrap();
        let m2 = store
            .add_message(&sid, 2, Role::Assistant, "c", Some("nvidia::z-ai/glm-5.2"))
            .unwrap();
        store.record_usage(&sid, &m2, &usage(30, 3, 15)).unwrap();
        store
            .record_side_call_usage(&sid, "compact", &usage(20, 2, 10))
            .unwrap();

        let week = store.usage_by_provider_since(0).unwrap();
        let terra = week
            .iter()
            .find(|r| r.provider == "codex-oauth")
            .expect("codex-oauth provider derived from model namespace");
        assert_eq!(terra.input_tokens, 150, "both terra turns summed");
        assert_eq!(terra.cached_input_tokens, 85);
        assert_eq!(terra.output_tokens, 15);
        let nvidia = week
            .iter()
            .find(|r| r.provider == "nvidia")
            .expect("nvidia derived as a distinct provider, not collapsed into one bucket");
        assert_eq!(
            nvidia.input_tokens, 50,
            "side-call usage inherits nearest provider"
        );
        assert_eq!(nvidia.output_tokens, 5);
        assert_eq!(nvidia.cached_input_tokens, 25);

        let session = store.usage_by_provider_for_session(&sid).unwrap();
        assert_eq!(session.len(), 2, "two distinct providers in the session");
        assert!(
            !session.iter().any(|r| r.provider == "other"),
            "side-call usage inherits its session provider instead of creating an other bucket"
        );
    }

    #[test]
    fn load_all_messages_keeps_compacted_away_history_for_the_user() {
        let store = Store::open_in_memory().unwrap();
        let sid = store.create_session("/x", "default").unwrap();
        for i in 0_i64..10 {
            let role = if i.rem_euclid(2) == 0 {
                Role::User
            } else {
                Role::Assistant
            };
            store
                .add_message(&sid, i, role, &format!("message {i}"), None)
                .unwrap();
        }
        assert!(!store.session_has_compaction(&sid).unwrap());
        // Compact: keep the 3 most-recent, summarize (soft-delete) the rest.
        store.compact_session_store(&sid, "SUMMARY", 3).unwrap();
        assert!(store.session_has_compaction(&sid).unwrap());

        // The model view is the summary + the 3 recent messages…
        let model_view = store.load_messages(&sid).unwrap();
        assert_eq!(model_view.len(), 4, "summary + 3 recent");
        // …but the USER's full view still has every original message, with no summary marker.
        let full = store.load_all_messages(&sid).unwrap();
        assert_eq!(
            full.len(),
            10,
            "full history retains every original message"
        );
        assert_eq!(
            full[0].content, "message 0",
            "the compacted-away first turn survives"
        );
    }

    #[test]
    fn uncompact_session_store_reactivates_messages_and_drops_the_summary() {
        let store = Store::open_in_memory().unwrap();
        let sid = store.create_session("/x", "default").unwrap();
        for i in 0_i64..10 {
            let role = if i.rem_euclid(2) == 0 {
                Role::User
            } else {
                Role::Assistant
            };
            store
                .add_message(&sid, i, role, &format!("message {i}"), None)
                .unwrap();
        }
        store.compact_session_store(&sid, "SUMMARY", 3).unwrap();
        assert_eq!(
            store.load_messages(&sid).unwrap().len(),
            4,
            "summary + 3 recent"
        );

        assert!(store.uncompact_session_store(&sid).unwrap());
        assert!(!store.session_has_compaction(&sid).unwrap());
        assert_eq!(
            store.load_messages(&sid).unwrap().len(),
            10,
            "every message reactivated, no summary marker"
        );
    }

    #[test]
    fn uncompact_session_store_does_not_resurrect_undone_messages() {
        // Both /undo and /compact soft-delete (active = 0). Uncompact must reactivate only the
        // rows /compact removed, never the ones /undo removed.
        let store = Store::open_in_memory().unwrap();
        let sid = store.create_session("/x", "default").unwrap();
        for i in 0_i64..10 {
            let role = if i.rem_euclid(2) == 0 {
                Role::User
            } else {
                Role::Assistant
            };
            store
                .add_message(&sid, i, role, &format!("message {i}"), None)
                .unwrap();
        }
        // /undo the last two turns (seq 8, 9) — they must stay gone across a compact/uncompact.
        assert_eq!(store.deactivate_messages_from(&sid, 8).unwrap(), 2);
        // Compact the remaining active messages, keeping the 3 most recent (seq 5,6,7).
        store.compact_session_store(&sid, "SUMMARY", 3).unwrap();

        assert!(store.uncompact_session_store(&sid).unwrap());
        assert!(!store.session_has_compaction(&sid).unwrap());
        let restored = store.load_messages(&sid).unwrap();
        assert_eq!(
            restored.len(),
            8,
            "compaction undone (seq 0..7 back) but the two /undo'd rows stay removed"
        );
        assert!(
            restored
                .iter()
                .all(|m| m.content != "message 8" && m.content != "message 9"),
            "the /undo'd messages were not resurrected"
        );
    }

    #[test]
    fn uncompact_session_store_is_a_no_op_without_a_prior_compaction() {
        let store = Store::open_in_memory().unwrap();
        let sid = store.create_session("/x", "default").unwrap();
        store
            .add_message(&sid, 0, Role::User, "hello", None)
            .unwrap();
        assert!(!store.uncompact_session_store(&sid).unwrap());
        assert_eq!(store.load_messages(&sid).unwrap().len(), 1);
    }

    #[test]
    fn session_exists_reports_presence() {
        let store = Store::open_in_memory().unwrap();
        let id = store.create_session("/x", "default").unwrap();
        assert!(store.session_exists(&id).unwrap());
        assert!(!store.session_exists("nope").unwrap());
    }

    #[test]
    fn ensure_session_restores_missing_parent_without_overwriting_existing_metadata() {
        let store = Store::open_in_memory().unwrap();
        store.ensure_session("recoverable", "/x", "Bypass").unwrap();
        assert!(store.session_exists("recoverable").unwrap());
        assert_eq!(
            store.session_cwd("recoverable").unwrap().as_deref(),
            Some("/x")
        );
        assert_eq!(store.session_mode("recoverable").unwrap(), "Bypass");

        store
            .ensure_session("recoverable", "/other", "Plan")
            .unwrap();
        assert_eq!(
            store.session_cwd("recoverable").unwrap().as_deref(),
            Some("/x")
        );
        assert_eq!(store.session_mode("recoverable").unwrap(), "Bypass");
    }

    #[test]
    fn matching_session_ids_resolves_a_prefix() {
        let store = Store::open_in_memory().unwrap();
        let id = store.create_session("/x", "default").unwrap();
        let prefix: String = id.chars().take(8).collect();

        let matches = store.matching_session_ids(&prefix).unwrap();
        assert_eq!(matches, vec![id]);
        assert!(store.matching_session_ids("zzzzzzzz").unwrap().is_empty());
    }

    #[test]
    fn matching_session_ids_treats_percent_and_underscore_as_literal() {
        // `%` and `_` are SQL LIKE metacharacters; a prefix containing them must be matched
        // literally, not as wildcards, or a lookup with those characters could match unrelated
        // session ids.
        let store = Store::open_in_memory().unwrap();
        let _other = store.create_session("/other", "default").unwrap();
        assert!(store.matching_session_ids("a%").unwrap().is_empty());
        assert!(store.matching_session_ids("a_").unwrap().is_empty());
    }

    // --- Assay runs + findings ---

    fn assay_finding(
        severity: forge_types::Severity,
        confidence: forge_types::Confidence,
        title: &str,
    ) -> forge_types::Finding {
        forge_types::Finding {
            id: forge_types::new_id(),
            category: forge_types::FindingCategory::Correctness,
            severity,
            confidence,
            file: "core/lib.rs".into(),
            line: None,
            title: title.into(),
            rationale: "reason".into(),
            suggested_fix: "fix".into(),
            effort: forge_types::Effort::Small,
            lens: "correctness".into(),
            verified: true,
        }
    }

    #[test]
    fn assay_findings_are_ranked_by_severity_then_confidence() {
        use forge_types::{Confidence, Severity};
        let store = Store::open_in_memory().unwrap();
        let run = store.create_assay_run("repo", 0.0).unwrap();
        for finding in [
            assay_finding(Severity::Low, Confidence::High, "low"),
            assay_finding(Severity::High, Confidence::Low, "high-low-confidence"),
            assay_finding(Severity::High, Confidence::High, "high-high-confidence"),
            assay_finding(Severity::Critical, Confidence::Low, "critical"),
        ] {
            store.add_finding(&run, &finding).unwrap();
        }

        let titles: Vec<_> = store
            .load_findings(&run)
            .unwrap()
            .into_iter()
            .map(|finding| finding.title)
            .collect();
        assert_eq!(
            titles,
            [
                "critical",
                "high-high-confidence",
                "high-low-confidence",
                "low"
            ]
        );
    }

    #[test]
    fn assay_history_is_scope_specific_and_excludes_the_current_run() {
        let store = Store::open_in_memory().unwrap();
        let first_repo = store.create_assay_run("repo", 0.1).unwrap();
        let path_run = store.create_assay_run("path src", 0.2).unwrap();
        let current_repo = store.create_assay_run("repo", 0.3).unwrap();

        assert_eq!(
            store
                .latest_run_for_scope("repo", &current_repo)
                .unwrap()
                .as_deref(),
            Some(first_repo.as_str())
        );
        assert_eq!(
            store.latest_run_for_scope("path src", &path_run).unwrap(),
            None
        );
        let runs = store.list_assay_runs().unwrap();
        assert_eq!(
            runs.first().map(|row| row.0.as_str()),
            Some(current_repo.as_str())
        );
    }

    #[test]
    fn assay_run_and_findings_round_trip() {
        use forge_types::{Confidence, Effort, Finding, FindingCategory, Severity};
        let store = Store::open_in_memory().unwrap();
        let run = store.create_assay_run("repo", 0.12).unwrap();
        let f = Finding {
            id: forge_types::new_id(),
            category: FindingCategory::Correctness,
            severity: Severity::Critical,
            confidence: Confidence::High,
            file: "core/lib.rs".into(),
            line: Some(204),
            title: "unwrap on provider result panics the turn".into(),
            rationale: "a transient 5xx aborts the session".into(),
            suggested_fix: "propagate via ?".into(),
            effort: Effort::Small,
            lens: "correctness".into(),
            verified: true,
        };
        store.add_finding(&run, &f).unwrap();

        let loaded = store.load_findings(&run).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0], f, "finding round-trips through the store");

        let runs = store.list_assay_runs().unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].0, run);
        assert_eq!(runs[0].1, "repo");
        assert!((runs[0].2 - 0.12).abs() < 1e-9);
    }

    // --- Conversation checkpoints / undo (PR2) ---

    #[test]
    fn deactivate_excludes_messages_from_load_but_keeps_earlier_ones() {
        let store = Store::open_in_memory().unwrap();
        let sid = store.create_session("/tmp", "default").unwrap();
        store
            .add_message(&sid, 0, Role::User, "turn 1", None)
            .unwrap();
        store
            .add_message(&sid, 1, Role::Assistant, "reply 1", Some("m"))
            .unwrap();
        store
            .add_message(&sid, 2, Role::User, "turn 2", None)
            .unwrap();
        store
            .add_message(&sid, 3, Role::Assistant, "reply 2", Some("m"))
            .unwrap();

        // Rewind to the start of turn 2 (seq 2): turn 2's two messages drop out.
        let n = store.deactivate_messages_from(&sid, 2).unwrap();
        assert_eq!(n, 2, "two messages deactivated");

        let msgs = store.load_messages(&sid).unwrap();
        let contents: Vec<_> = msgs.iter().map(|m| m.content.as_str()).collect();
        assert_eq!(
            contents,
            vec!["turn 1", "reply 1"],
            "only the surviving turn loads"
        );
        // message_count must also exclude the soft-deleted rows (it used to count all 4, inflating
        // the session picker).
        assert_eq!(
            store.message_count(&sid).unwrap(),
            2,
            "message_count excludes soft-deleted messages"
        );
    }

    #[test]
    fn session_tokens_and_step_count_exclude_deactivated_messages() {
        let store = Store::open_in_memory().unwrap();
        let sid = store.create_session("/tmp", "default").unwrap();
        let m1 = store.add_message(&sid, 0, Role::User, "q", None).unwrap();
        store
            .record_usage(
                &sid,
                &m1,
                &Usage {
                    input_tokens: 10,
                    output_tokens: 5,
                    ..Default::default()
                },
            )
            .unwrap();
        let m2 = store
            .add_message(&sid, 1, Role::Assistant, "a", Some("m"))
            .unwrap();
        store
            .record_usage(
                &sid,
                &m2,
                &Usage {
                    input_tokens: 20,
                    output_tokens: 10,
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(store.session_step_count(&sid).unwrap(), 2);
        assert_eq!(store.session_tokens(&sid).unwrap(), (30, 15));

        // Undo turn 2: its usage must drop out of both the token counter and the steps metric.
        store.deactivate_messages_from(&sid, 1).unwrap();
        assert_eq!(store.session_step_count(&sid).unwrap(), 1);
        assert_eq!(store.session_tokens(&sid).unwrap(), (10, 5));

        // Consumed usage is an accounting ledger: undo does not refund provider quota, and
        // inactive synthetic side calls must also remain visible there without entering the
        // active transcript counter.
        let consumed = store.session_token_usage(&sid).unwrap();
        assert_eq!((consumed.input_tokens, consumed.output_tokens), (30, 15));
        store
            .record_side_call_usage(
                &sid,
                "memory",
                &Usage {
                    input_tokens: 7,
                    cached_input_tokens: 2,
                    output_tokens: 3,
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(store.session_tokens(&sid).unwrap(), (10, 5));
        let consumed = store.session_token_usage(&sid).unwrap();
        assert_eq!((consumed.input_tokens, consumed.output_tokens), (37, 18));
        assert_eq!(consumed.cached_input_tokens, 2);
    }

    #[test]
    fn session_tasks_round_trip_and_replace() {
        use forge_types::{TodoItem, TodoStatus};
        let store = Store::open_in_memory().unwrap();
        let sid = store.create_session("/tmp", "default").unwrap();
        assert!(store.tasks(&sid).unwrap().is_empty(), "none initially");

        let tasks = vec![
            TodoItem {
                title: "write the parser".into(),
                status: TodoStatus::Done,
                assignee: None,
            },
            TodoItem {
                title: "wire it up".into(),
                status: TodoStatus::InProgress,
                assignee: None,
            },
        ];
        store.set_tasks(&sid, &tasks).unwrap();
        assert_eq!(store.tasks(&sid).unwrap(), tasks, "round-trips");

        // A second write replaces the list wholesale.
        let next = vec![TodoItem {
            title: "ship".into(),
            status: TodoStatus::Pending,
            assignee: None,
        }];
        store.set_tasks(&sid, &next).unwrap();
        assert_eq!(store.tasks(&sid).unwrap(), next, "replaced, not appended");
    }

    #[test]
    fn session_tasks_carry_an_assignee_and_load_rows_written_without_one() {
        use forge_types::{TodoItem, TodoStatus};
        let store = Store::open_in_memory().unwrap();
        let sid = store.create_session("/tmp", "default").unwrap();

        let delegated = vec![TodoItem {
            title: "port the store".into(),
            status: TodoStatus::InProgress,
            assignee: Some("builder-a3".into()),
        }];
        store.set_tasks(&sid, &delegated).unwrap();
        assert_eq!(store.tasks(&sid).unwrap(), delegated, "owner round-trips");

        // A row written by an older build has no `assignee` key at all — it must still load
        // (serde default), not drop the whole list on the floor and silently show no tasks.
        store
            .lock()
            .unwrap()
            .execute(
                "UPDATE session_tasks SET tasks_json = ?1 WHERE session_id = ?2",
                rusqlite::params![r#"[{"title":"legacy","status":"pending"}]"#, &sid],
            )
            .unwrap();
        let loaded = store.tasks(&sid).unwrap();
        assert_eq!(loaded.len(), 1, "pre-assignee rows still parse");
        assert_eq!(loaded[0].title, "legacy");
        assert_eq!(loaded[0].assignee, None);
    }

    #[test]
    fn compact_session_store_prepends_summary_on_resume() {
        let store = Store::open_in_memory().unwrap();
        let sid = store.create_session("/tmp", "default").unwrap();
        for i in 0..8i64 {
            store
                .add_message(&sid, i, Role::User, &format!("msg {i}"), None)
                .unwrap();
        }

        // Keep the last 3, summarize the first 5.
        store
            .compact_session_store(&sid, "Summary of first 5 messages.", 3)
            .unwrap();

        let msgs = store.load_messages(&sid).unwrap();
        // 1 summary + 3 kept = 4
        assert_eq!(msgs.len(), 4, "summary + 3 kept messages");
        assert_eq!(
            msgs[0].role,
            Role::System,
            "prepended summary is a System message"
        );
        assert!(
            msgs[0].content.contains("Summary of first 5 messages."),
            "summary content preserved"
        );
        assert_eq!(msgs[1].content, "msg 5");
        assert_eq!(msgs[2].content, "msg 6");
        assert_eq!(msgs[3].content, "msg 7");
    }

    #[test]
    fn compact_session_store_upserts_summary_on_second_compact() {
        let store = Store::open_in_memory().unwrap();
        let sid = store.create_session("/tmp", "default").unwrap();
        for i in 0..6i64 {
            store
                .add_message(&sid, i, Role::User, &format!("msg {i}"), None)
                .unwrap();
        }
        store
            .compact_session_store(&sid, "First summary.", 3)
            .unwrap();
        // Add 3 more messages (simulate new turns after first compact).
        for i in 6..9i64 {
            store
                .add_message(&sid, i, Role::User, &format!("msg {i}"), None)
                .unwrap();
        }
        store
            .compact_session_store(&sid, "Second summary.", 3)
            .unwrap();

        let msgs = store.load_messages(&sid).unwrap();
        assert_eq!(msgs.len(), 4, "summary + 3 kept after second compact");
        assert!(
            msgs[0].content.contains("Second summary."),
            "upserted summary"
        );
        assert_eq!(msgs[1].content, "msg 6");
        assert_eq!(msgs[2].content, "msg 7");
        assert_eq!(msgs[3].content, "msg 8");
    }

    #[test]
    fn checkpoints_round_trip_newest_first() {
        let store = Store::open_in_memory().unwrap();
        let sid = store.create_session("/tmp", "default").unwrap();
        store
            .add_checkpoint(&sid, Some("before refactor"), 2)
            .unwrap();
        store.add_checkpoint(&sid, None, 5).unwrap();

        let cps = store.list_checkpoints(&sid).unwrap();
        assert_eq!(cps.len(), 2);
        assert_eq!(cps[0].seq, 5, "newest (highest seq) first");
        assert_eq!(cps[0].label, None, "auto checkpoint has no label");
        assert_eq!(cps[1].label.as_deref(), Some("before refactor"));
    }

    // --- Model health / failover ---

    #[test]
    fn benched_model_is_in_snapshot_until_cooldown_elapses() {
        let store = Store::open_in_memory().unwrap();
        store
            .bench_model("gemini::antigravity", 1000, "rate-limited")
            .unwrap();
        // now=500 < cooldown 1000 → still benched (AC-3).
        assert!(store
            .benched_models(500)
            .unwrap()
            .is_benched("gemini::antigravity"));
        // now=1001 > cooldown → eligible again (AC-4).
        assert!(!store
            .benched_models(1001)
            .unwrap()
            .is_benched("gemini::antigravity"));
    }

    #[test]
    fn provider_auth_exclusion_benches_all_of_its_model_aliases() {
        let store = Store::open_in_memory().unwrap();
        store.exclude_provider("agy-cli", "login required").unwrap();
        let health = store.current_benched().unwrap();
        assert!(health.is_benched("agy-cli::gemini-3.1-pro"));
        assert!(health.is_benched("agy-cli::gemini-3.5-flash"));
        assert!(!health.is_benched("groq::qwen/qwen3.6-27b"));
        store.clear_provider_health("agy-cli").unwrap();
        assert!(!store
            .current_benched()
            .unwrap()
            .is_benched("agy-cli::gemini-3.1-pro"));
    }

    #[test]
    fn quota_is_upserted_and_expires_when_the_window_resets() {
        let store = Store::open_in_memory().unwrap();
        let hint = |status, resets_at| forge_types::QuotaHint {
            provider: "claude-cli".into(),
            window: "five_hour".into(),
            status,
            resets_at,
            fraction_used: None,
        };
        // A warning that resets at t=1000.
        store
            .record_quota(&hint(forge_types::QuotaStatus::Warning, Some(1000)))
            .unwrap();
        assert!(store.quota_at(500).unwrap().is_pressured("claude-cli"));
        // Past the reset → no longer constraining.
        assert!(!store.quota_at(2000).unwrap().is_pressured("claude-cli"));

        // Upsert to exhausted; an Ok provider isn't carried at all.
        store
            .record_quota(&hint(forge_types::QuotaStatus::Exhausted, Some(3000)))
            .unwrap();
        assert!(store.quota_at(500).unwrap().is_exhausted("claude-cli"));
        store
            .record_quota(&forge_types::QuotaHint {
                provider: "codex-cli".into(),
                window: String::new(),
                status: forge_types::QuotaStatus::Ok,
                resets_at: Some(9999),
                fraction_used: None,
            })
            .unwrap();
        assert!(!store.quota_at(500).unwrap().is_pressured("codex-cli"));
    }

    #[test]
    fn record_quota_also_appends_history_when_fraction_is_known() {
        // record_quota's history side-effect is additive: subscription_usage still upserts to one
        // row per (provider, window), but quota_history grows one row per call.
        let store = Store::open_in_memory().unwrap();
        let hint = |fraction| forge_types::QuotaHint {
            provider: "claude-cli".into(),
            window: "five_hour".into(),
            status: forge_types::QuotaStatus::Ok,
            resets_at: Some(9999),
            fraction_used: Some(fraction),
        };
        store.record_quota(&hint(0.1)).unwrap();
        store.record_quota(&hint(0.2)).unwrap();

        let history = store
            .quota_history_since("claude-cli", "five_hour", 0)
            .unwrap();
        assert_eq!(history.len(), 2, "one history row per record_quota call");
        assert_eq!(history[0].fraction_used, 0.1);
        assert_eq!(history[1].fraction_used, 0.2);

        // subscription_usage itself still reflects only the latest snapshot.
        assert!(store.quota_at(0).unwrap().is_empty(), "Ok isn't carried");
    }

    #[test]
    fn record_quota_skips_history_when_fraction_is_unknown() {
        let store = Store::open_in_memory().unwrap();
        store
            .record_quota(&forge_types::QuotaHint {
                provider: "codex-cli".into(),
                window: "weekly".into(),
                status: forge_types::QuotaStatus::Ok,
                resets_at: None,
                fraction_used: None,
            })
            .unwrap();
        assert!(store
            .quota_history_since("codex-cli", "weekly", 0)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn quota_history_since_filters_by_cutoff_and_orders_oldest_first() {
        let store = Store::open_in_memory().unwrap();
        store
            .record_quota_history_at("claude-cli", "five_hour", 0.05, None, 100)
            .unwrap();
        store
            .record_quota_history_at("claude-cli", "five_hour", 0.50, None, 300)
            .unwrap();
        store
            .record_quota_history_at("claude-cli", "five_hour", 0.90, None, 500)
            .unwrap();

        let all = store
            .quota_history_since("claude-cli", "five_hour", 0)
            .unwrap();
        assert_eq!(
            all.iter().map(|p| p.observed_at).collect::<Vec<_>>(),
            vec![100, 300, 500]
        );

        let recent = store
            .quota_history_since("claude-cli", "five_hour", 200)
            .unwrap();
        assert_eq!(
            recent.iter().map(|p| p.observed_at).collect::<Vec<_>>(),
            vec![300, 500],
            "cutoff excludes the earlier point"
        );

        // A different provider/window is isolated.
        assert!(store
            .quota_history_since("codex-cli", "five_hour", 0)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn quota_at_attaches_a_pace_projection_from_history() {
        // Fast-climbing history (>5min apart, per QUOTA_PACE_MIN_ELAPSED_SECS) plus a
        // subscription_usage row carrying resets_at → quota_at must attach a pace whose
        // projected_fraction_at_reset is higher than the current (low) fraction, so a
        // fast-burning-but-early window isn't under-protected by the plain fraction alone.
        //
        // `record_quota` stamps its own quota_history row with the real wall clock (it has no
        // testable-clock variant), so this seeds the earlier point at `now - 1200` and lets
        // `record_quota` supply the latest point at (approximately) `now` — unlike
        // `record_quota_history_at`, which would need a fully synthetic `now` that the real
        // wall-clock row would then fall outside of.
        let store = Store::open_in_memory().unwrap();
        let now = chrono::Utc::now().timestamp();
        let resets_at = now + 3600; // 1 hour left in the window
        store
            .record_quota_history_at("claude-cli", "five_hour", 0.10, Some(resets_at), now - 1200)
            .unwrap();
        store
            .record_quota(&forge_types::QuotaHint {
                provider: "claude-cli".into(),
                window: "five_hour".into(),
                status: forge_types::QuotaStatus::Ok,
                resets_at: Some(resets_at),
                fraction_used: Some(0.30),
            })
            .unwrap();

        let quota = store.quota_at(now).unwrap();
        let current = quota.fraction_for("claude-cli");
        let pace = quota
            .pace_for("claude-cli")
            .expect("enough history to derive a pace");
        let projected = pace
            .projected_fraction_at_reset
            .expect("resets_at is known");
        assert!(
            projected > current,
            "fast pace should project above the current fraction: current={current} projected={projected}"
        );
        assert!(
            quota.effective_fraction_for("claude-cli") > current,
            "the conservation input must reflect the pace, not just the point-in-time fraction"
        );
    }

    #[test]
    fn quota_at_has_no_pace_without_history() {
        // A subscription_usage row with a fraction but no quota_history rows at all (e.g. a
        // single record_quota call) must not attach a pace — everything else about quota_at
        // stays as before.
        let store = Store::open_in_memory().unwrap();
        store
            .record_quota(&forge_types::QuotaHint {
                provider: "codex-cli".into(),
                window: "weekly".into(),
                status: forge_types::QuotaStatus::Ok,
                resets_at: Some(999_999),
                fraction_used: Some(0.15),
            })
            .unwrap();

        let quota = store.quota_at(0).unwrap();
        assert!(
            quota.pace_for("codex-cli").is_none(),
            "a single sample is not enough history for a pace"
        );
        assert!((quota.fraction_for("codex-cli") - 0.15).abs() < 1e-9);
        assert!((quota.effective_fraction_for("codex-cli") - 0.15).abs() < 1e-9);
    }

    // --- Shared codex quota bucket (codex-cli / codex-oauth alias group) ---

    #[test]
    fn codex_alias_group_surfaces_oauth_only_usage_under_both_providers() {
        let store = Store::open_in_memory().unwrap();
        store
            .record_quota(&forge_types::QuotaHint {
                provider: "codex-oauth".into(),
                window: "five_hour".into(),
                status: forge_types::QuotaStatus::Ok,
                resets_at: Some(999_999),
                fraction_used: Some(0.5),
            })
            .unwrap();

        let quota = store.quota_at(0).unwrap();
        assert!((quota.fraction_for("codex-cli") - 0.5).abs() < 1e-9);
        assert!((quota.fraction_for("codex-oauth") - 0.5).abs() < 1e-9);
    }

    #[test]
    fn bridge_fractions_share_the_latest_codex_window_with_both_surfaces() {
        let store = Store::open_in_memory().unwrap();
        let now = chrono::Utc::now().timestamp();
        store
            .record_quota_at(
                &forge_types::QuotaHint {
                    provider: "codex-cli".into(),
                    window: "weekly".into(),
                    status: forge_types::QuotaStatus::Ok,
                    resets_at: None,
                    fraction_used: Some(0.27),
                },
                now,
            )
            .unwrap();

        let fractions = store.bridge_fractions().unwrap();
        for provider in ["codex-cli", "codex-oauth"] {
            assert_eq!(
                fractions
                    .get(provider)
                    .and_then(|windows| windows.get("weekly")),
                Some(&0.27),
                "{provider} must display the same shared account snapshot"
            );
            assert!(
                fractions
                    .get(provider)
                    .is_none_or(|windows| !windows.contains_key("five_hour")),
                "an absent 5h window must not be invented"
            );
        }
    }

    #[test]
    fn live_codex_account_updates_aliases_plan_and_freshness_together() {
        let store = Store::open_in_memory().unwrap();
        let hint = forge_types::QuotaHint {
            provider: "codex-oauth".to_string(),
            window: "five_hour".to_string(),
            status: forge_types::QuotaStatus::Warning,
            resets_at: None,
            fraction_used: Some(0.85),
        };

        store.record_live_codex_account(&hint, Some("pro")).unwrap();

        let fractions = store.bridge_fractions().unwrap();
        for provider in ["codex-oauth", "codex-cli"] {
            assert_eq!(fractions[provider]["five_hour"], 0.85);
            assert_eq!(
                store.fresh_subscription_plan(provider).as_deref(),
                Some("pro")
            );
        }
        assert!(store.codex_oauth_quota_age_secs().unwrap().is_some());
    }

    #[test]
    fn fresh_codex_plan_observation_is_shared_by_the_oauth_and_cli_aliases() {
        let store = Store::open_in_memory().unwrap();
        store
            .record_subscription_plan("codex-oauth", "pro")
            .unwrap();
        assert_eq!(
            store.fresh_subscription_plan("codex-cli").as_deref(),
            Some("pro")
        );
        assert_eq!(
            store.fresh_subscription_plan("codex-oauth").as_deref(),
            Some("pro")
        );
    }

    #[test]
    fn codex_alias_group_latest_updated_at_wins_never_sums() {
        let store = Store::open_in_memory().unwrap();
        let hint = |provider: &str, fraction| forge_types::QuotaHint {
            provider: provider.into(),
            window: "five_hour".into(),
            status: forge_types::QuotaStatus::Ok,
            resets_at: Some(999_999),
            fraction_used: Some(fraction),
        };
        // codex-cli recorded at t=100, codex-oauth LATER at t=200, same window.
        store
            .record_quota_at(&hint("codex-cli", 0.30), 100)
            .unwrap();
        store
            .record_quota_at(&hint("codex-oauth", 0.60), 200)
            .unwrap();

        let quota = store.quota_at(0).unwrap();
        assert!(
            (quota.fraction_for("codex-cli") - 0.6).abs() < 1e-9,
            "latest wins, not summed to 0.9"
        );
        assert!((quota.fraction_for("codex-oauth") - 0.6).abs() < 1e-9);

        // Reverse order: an OLDER codex-oauth write after a NEWER codex-cli write must not win.
        store
            .record_quota_at(&hint("codex-cli", 0.70), 500)
            .unwrap();
        store
            .record_quota_at(&hint("codex-oauth", 0.20), 300)
            .unwrap();
        let quota = store.quota_at(0).unwrap();
        assert!(
            (quota.fraction_for("codex-cli") - 0.7).abs() < 1e-9,
            "the stale (lower updated_at) write must not override the newer one"
        );
        assert!((quota.fraction_for("codex-oauth") - 0.7).abs() < 1e-9);
    }

    #[test]
    fn codex_alias_group_merges_per_window_across_providers() {
        let store = Store::open_in_memory().unwrap();
        store
            .record_quota(&forge_types::QuotaHint {
                provider: "codex-cli".into(),
                window: "five_hour".into(),
                status: forge_types::QuotaStatus::Warning,
                resets_at: Some(999_999),
                fraction_used: Some(0.85),
            })
            .unwrap();
        store
            .record_quota(&forge_types::QuotaHint {
                provider: "codex-oauth".into(),
                window: "weekly".into(),
                status: forge_types::QuotaStatus::Ok,
                resets_at: Some(999_999),
                fraction_used: Some(0.40),
            })
            .unwrap();

        // Both surfaces see both windows: the strictest (five_hour, 0.85) drives the fraction.
        let quota = store.quota_at(0).unwrap();
        assert!((quota.fraction_for("codex-cli") - 0.85).abs() < 1e-9);
        assert!((quota.fraction_for("codex-oauth") - 0.85).abs() < 1e-9);
        assert!(quota.is_pressured("codex-cli"));
        assert!(quota.is_pressured("codex-oauth"), "merged status is shared");
    }

    #[test]
    fn codex_alias_group_exhausted_threshold_shared_across_both_surfaces() {
        let store = Store::open_in_memory().unwrap();
        store
            .record_quota(&forge_types::QuotaHint {
                provider: "codex-oauth".into(),
                window: "five_hour".into(),
                status: forge_types::QuotaStatus::Exhausted,
                resets_at: Some(999_999),
                fraction_used: Some(0.99),
            })
            .unwrap();

        let quota = store.quota_at(0).unwrap();
        assert!(quota.is_exhausted("codex-cli"));
        assert!(quota.is_exhausted("codex-oauth"));
    }

    #[test]
    fn stale_seed_after_newer_header_reading_does_not_lower_merged_fraction() {
        // The live failure this reproduces: codex-oauth records 1% from fresh x-codex headers;
        // 30 seconds LATER a `forge mesh` run re-seeds codex-cli from an hours-old rollout file
        // reading 0%. With the seed stamped at its OBSERVATION time (older), the merged bucket
        // must keep the fresher 1% — not reset to the stale 0%.
        let store = Store::open_in_memory().unwrap();
        let hint = |provider: &str, fraction| forge_types::QuotaHint {
            provider: provider.into(),
            window: "five_hour".into(),
            status: forge_types::QuotaStatus::Ok,
            resets_at: Some(999_999),
            fraction_used: Some(fraction),
        };
        store
            .record_quota_at(&hint("codex-oauth", 0.01), 1000)
            .unwrap();
        // Rollout observation from long before the header reading, recorded after it.
        store.record_quota_at(&hint("codex-cli", 0.0), 500).unwrap();

        let quota = store.quota_at(0).unwrap();
        assert!(
            (quota.fraction_for("codex-oauth") - 0.01).abs() < 1e-9,
            "stale rollout seed must not mask the fresher header reading"
        );
        assert!((quota.fraction_for("codex-cli") - 0.01).abs() < 1e-9);
    }

    #[test]
    fn reset_inference_stamped_at_reset_beats_prereset_loses_to_postreset() {
        // The "window just reset → 0%" inference is stamped at the reset instant itself. Ordering
        // consequences: it must overwrite a STALE pre-reset reading of the old window, but lose
        // to any REAL observation made after the reset (e.g. a fresher x-codex header reading of
        // the new window via codex-oauth).
        let store = Store::open_in_memory().unwrap();
        let hint = |provider: &str, fraction| forge_types::QuotaHint {
            provider: provider.into(),
            window: "five_hour".into(),
            status: forge_types::QuotaStatus::Ok,
            resets_at: None,
            fraction_used: Some(fraction),
        };
        let reset_at = 1000;

        // Old-window reading observed BEFORE the reset.
        store
            .record_quota_at(&hint("codex-cli", 0.80), 900)
            .unwrap();
        // The reset inference, stamped AT the reset instant — wins over the pre-reset reading.
        store
            .record_quota_at(&hint("codex-cli", 0.0), reset_at)
            .unwrap();
        let quota = store.quota_at(0).unwrap();
        assert!(
            quota.fraction_for("codex-cli").abs() < 1e-9,
            "the window DID reset — the inference beats the pre-reset reading"
        );

        // A real post-reset observation (header reading on the oauth surface) — beats the
        // inference in the merged bucket, on both surfaces.
        store
            .record_quota_at(&hint("codex-oauth", 0.01), 1100)
            .unwrap();
        let quota = store.quota_at(0).unwrap();
        assert!(
            (quota.fraction_for("codex-cli") - 0.01).abs() < 1e-9,
            "newer real knowledge of the new window beats the reset inference"
        );
        assert!((quota.fraction_for("codex-oauth") - 0.01).abs() < 1e-9);

        // And a re-seeded inference (same reset instant) can no longer clobber it.
        store
            .record_quota_at(&hint("codex-cli", 0.0), reset_at)
            .unwrap();
        let quota = store.quota_at(0).unwrap();
        assert!(
            (quota.fraction_for("codex-cli") - 0.01).abs() < 1e-9,
            "re-seeding the inference is a no-op against fresher data"
        );
    }

    #[test]
    fn record_quota_at_older_timestamp_is_a_noop_newer_overwrites() {
        let store = Store::open_in_memory().unwrap();
        let hint = |fraction| forge_types::QuotaHint {
            provider: "codex-cli".into(),
            window: "five_hour".into(),
            status: forge_types::QuotaStatus::Ok,
            resets_at: Some(999_999),
            fraction_used: Some(fraction),
        };
        store.record_quota_at(&hint(0.5), 1000).unwrap();
        // Older observation arriving late: complete no-op — snapshot AND history untouched.
        store.record_quota_at(&hint(0.3), 500).unwrap();
        let quota = store.quota_at(0).unwrap();
        assert!(
            (quota.fraction_for("codex-cli") - 0.5).abs() < 1e-9,
            "an older observation must not overwrite a newer row"
        );
        let history = store
            .quota_history_since("codex-cli", "five_hour", 0)
            .unwrap();
        assert_eq!(history.len(), 1, "stale write appends no history");

        // Newer observation overwrites.
        store.record_quota_at(&hint(0.7), 1500).unwrap();
        let quota = store.quota_at(0).unwrap();
        assert!((quota.fraction_for("codex-cli") - 0.7).abs() < 1e-9);

        // Re-seeding the SAME observation (same timestamp) doesn't duplicate history.
        store.record_quota_at(&hint(0.7), 1500).unwrap();
        let history = store
            .quota_history_since("codex-cli", "five_hour", 0)
            .unwrap();
        assert_eq!(
            history.len(),
            2,
            "one point per distinct observation, not per re-seed"
        );
    }

    #[test]
    fn non_grouped_provider_is_unaffected_by_alias_merge() {
        let store = Store::open_in_memory().unwrap();
        store
            .record_quota(&forge_types::QuotaHint {
                provider: "claude-cli".into(),
                window: "five_hour".into(),
                status: forge_types::QuotaStatus::Warning,
                resets_at: Some(999_999),
                fraction_used: Some(0.85),
            })
            .unwrap();
        store
            .record_quota(&forge_types::QuotaHint {
                provider: "codex-oauth".into(),
                window: "five_hour".into(),
                status: forge_types::QuotaStatus::Ok,
                resets_at: Some(999_999),
                fraction_used: Some(0.10),
            })
            .unwrap();

        let quota = store.quota_at(0).unwrap();
        assert!(
            (quota.fraction_for("claude-cli") - 0.85).abs() < 1e-9,
            "claude-cli reads only its own row, untouched by the codex alias group"
        );
        assert!(quota.is_pressured("claude-cli"));
        assert!(!quota.is_pressured("codex-oauth"));
        assert!((quota.fraction_for("codex-cli") - 0.10).abs() < 1e-9);
    }

    #[test]
    fn stale_codex_quota_does_not_pressure_either_shared_surface() {
        let store = Store::open_in_memory().unwrap();
        let now = 10_000;
        let hint = |provider: &str, fraction| forge_types::QuotaHint {
            provider: provider.into(),
            window: "five_hour".into(),
            status: if fraction >= 0.80 {
                forge_types::QuotaStatus::Warning
            } else {
                forge_types::QuotaStatus::Ok
            },
            resets_at: Some(now + 10_000),
            fraction_used: Some(fraction),
        };

        // A historical rollout may correctly report 92%, but it cannot describe a shared
        // subscription after five minutes of external Codex usage.
        store
            .record_quota_at(
                &hint("codex-cli", 0.92),
                now - forge_types::CODEX_QUOTA_FRESHNESS_SECS - 1,
            )
            .unwrap();
        let quota = store.quota_at(now).unwrap();
        assert!(!quota.is_pressured("codex-cli"));
        assert!(!quota.is_pressured("codex-oauth"));
        assert_eq!(quota.fraction_for("codex-cli"), 0.0);
        assert_eq!(quota.fraction_for("codex-oauth"), 0.0);

        // A current OAuth header observation becomes authoritative for BOTH names.
        store
            .record_quota_at(&hint("codex-oauth", 0.25), now)
            .unwrap();
        let quota = store.quota_at(now).unwrap();
        assert!((quota.fraction_for("codex-cli") - 0.25).abs() < 1e-9);
        assert!((quota.fraction_for("codex-oauth") - 0.25).abs() < 1e-9);
        assert!(!quota.is_pressured("codex-cli"));
    }

    #[test]
    fn bench_is_upsert_and_clear_removes_it() {
        let store = Store::open_in_memory().unwrap();
        store.bench_model("m", 1000, "rate-limited").unwrap();
        store.bench_model("m", 2000, "auth failed").unwrap(); // upsert, no PK clash
        let report = store.benched_report(500).unwrap();
        assert_eq!(report.len(), 1);
        assert_eq!(
            report[0],
            ("m".to_string(), 2000, "auth failed".to_string())
        );
        store.clear_model_health("m").unwrap();
        assert!(store.benched_models(500).unwrap().is_empty());
    }

    #[test]
    fn transient_benches_are_ordered_by_recovery_time() {
        let store = Store::open_in_memory().unwrap();
        store.bench_model("late", 300, "rate limited").unwrap();
        store.bench_model("early", 100, "unavailable").unwrap();
        store
            .bench_model("excluded", 50, "excluded: no tools")
            .unwrap();

        assert_eq!(
            store.transient_benched_ordered().unwrap(),
            ["early", "late"]
        );
    }

    #[test]
    fn all_model_contexts_returns_every_discovered_window() {
        let store = Store::open_in_memory().unwrap();
        store.set_model_context("small", 8_192).unwrap();
        store.set_model_context("large", 200_000).unwrap();

        let contexts = store.all_model_contexts().unwrap();
        assert_eq!(contexts["small"], 8_192);
        assert_eq!(contexts["large"], 200_000);
    }

    #[test]
    fn model_context_round_trips_and_upserts() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(store.model_context("openrouter::x:free").unwrap(), None);
        store
            .set_model_context("openrouter::x:free", 131_072)
            .unwrap();
        assert_eq!(
            store.model_context("openrouter::x:free").unwrap(),
            Some(131_072)
        );
        // Upsert: a later fetch refreshes the window.
        store
            .set_model_context("openrouter::x:free", 65_536)
            .unwrap();
        assert_eq!(
            store.model_context("openrouter::x:free").unwrap(),
            Some(65_536)
        );
    }

    #[test]
    fn model_pricing_round_trips_and_upserts() {
        let store = Store::open_in_memory().unwrap();
        assert!(store.all_model_pricing().unwrap().is_empty());
        store
            .set_model_pricing("openrouter::vendor/m", 0.0002, 0.0008, Some(0.00005))
            .unwrap();
        let rows = store.all_model_pricing().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].0, "openrouter::vendor/m");
        assert!((rows[0].1 - 0.0002).abs() < 1e-12);
        assert!((rows[0].2 - 0.0008).abs() < 1e-12);
        assert!((rows[0].3.unwrap() - 0.00005).abs() < 1e-12);
        // Upsert refreshes in place, including clearing the cache-read rate.
        store
            .set_model_pricing("openrouter::vendor/m", 0.001, 0.002, None)
            .unwrap();
        let rows = store.all_model_pricing().unwrap();
        assert_eq!(rows.len(), 1);
        assert!((rows[0].1 - 0.001).abs() < 1e-12);
        assert!(rows[0].3.is_none());
    }

    #[test]
    fn exclude_model_benches_long_and_soonest_skips_exclusions() {
        let store = Store::open_in_memory().unwrap();
        let now = chrono::Utc::now().timestamp();

        // A permanent exclusion: benched far into the future, reason prefixed "excluded:".
        store
            .exclude_model("dead::no-tools", "no tool calling")
            .unwrap();
        assert!(
            store
                .current_benched()
                .unwrap()
                .is_benched("dead::no-tools"),
            "excluded model is benched now"
        );
        let report = store.current_benched_report().unwrap();
        let row = report
            .iter()
            .find(|(m, _, _)| m == "dead::no-tools")
            .unwrap();
        assert!(
            row.1 > now + 23 * 60 * 60 && row.1 <= now + 25 * 60 * 60,
            "exclusion window is ~24 hours"
        );
        assert!(row.2.starts_with("excluded:"));

        // A transient bench alongside it.
        store
            .bench_for(
                "rl::model",
                std::time::Duration::from_secs(120),
                "rate-limited",
            )
            .unwrap();

        // soonest_unbenched returns the transient one, never the permanent exclusion.
        assert_eq!(
            store.soonest_unbenched().unwrap().as_deref(),
            Some("rl::model")
        );

        // With only exclusions left, there's no last-resort candidate.
        store.clear_model_health("rl::model").unwrap();
        assert_eq!(store.soonest_unbenched().unwrap(), None);
    }

    #[test]
    fn lattice_embedding_round_trips_and_upserts() {
        let store = Store::open_in_memory().unwrap();
        // A node row is required (FK). Insert one via the file-replace path.
        let file = LatticeFileRow {
            id: "f1".into(),
            repo_root: "/r".into(),
            rel_path: "a.rs".into(),
            lang: "rust".into(),
            content_hash: "h".into(),
            parse_status: "ok".into(),
        };
        let node = LatticeNodeRow {
            id: "n1".into(),
            file_id: "f1".into(),
            kind: "function".into(),
            name: "foo".into(),
            qualname: None,
            signature: None,
            span_start: 0,
            span_end: 1,
            line_start: 1,
            pagerank: 0.0,
        };
        store
            .replace_lattice_file(&file, &[node], &[], &[])
            .unwrap();

        store
            .put_lattice_embedding("n1", &[1.0, -0.5, 0.25])
            .unwrap();
        assert_eq!(store.lattice_embedding_count("/r").unwrap(), 1);
        let all = store.lattice_embeddings("/r").unwrap();
        assert_eq!(all, vec![("n1".to_string(), vec![1.0, -0.5, 0.25])]);
        // Upsert replaces, not duplicates.
        store.put_lattice_embedding("n1", &[2.0, 2.0]).unwrap();
        assert_eq!(store.lattice_embedding_count("/r").unwrap(), 1);
        assert_eq!(store.lattice_embeddings("/r").unwrap()[0].1, vec![2.0, 2.0]);
    }

    #[test]
    fn most_recent_session_id_empty_store_returns_none() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(store.most_recent_session_id().unwrap(), None);
    }

    #[test]
    fn most_recent_session_id_returns_newest_top_level() {
        let store = Store::open_in_memory().unwrap();
        let a = store.create_session("/a", "default").unwrap();
        let b = store.create_session("/b", "default").unwrap();
        // b was created last → should be most recent
        assert_eq!(store.most_recent_session_id().unwrap(), Some(b));
        // a is still there but not most recent
        assert_ne!(store.most_recent_session_id().unwrap(), Some(a));
    }

    #[test]
    fn most_recent_session_id_skips_child_sessions() {
        let store = Store::open_in_memory().unwrap();
        let parent = store.create_session("/parent", "default").unwrap();
        // Create a child session after the parent — it must not appear as most-recent
        let _child = store
            .create_child_session("/child", "default", &parent)
            .unwrap();
        assert_eq!(
            store.most_recent_session_id().unwrap(),
            Some(parent.clone())
        );
    }

    #[test]
    fn list_sessions_excludes_child_sessions() {
        let store = Store::open_in_memory().unwrap();
        let parent = store.create_session("/parent", "default").unwrap();
        store
            .add_message(&parent, 0, Role::User, "do the thing", None)
            .unwrap();
        let _child = store
            .create_child_session("/child", "default", &parent)
            .unwrap();
        let sessions = store.list_sessions().unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, parent);
    }

    #[test]
    fn list_sessions_excludes_sessions_with_no_user_message() {
        // A session row is created eagerly at process start, before any prompt is sent. A
        // process that opens a session and exits/crashes before the user ever types anything
        // (e.g. an `mcp agent` connection that's never used, or one caught in a spawn loop)
        // must not pollute the picker with a blank entry.
        let store = Store::open_in_memory().unwrap();
        let _empty = store.create_session("/empty", "default").unwrap();
        let used = store.create_session("/used", "default").unwrap();
        store
            .add_message(&used, 0, Role::User, "real prompt", None)
            .unwrap();
        let sessions = store.list_sessions().unwrap();
        assert_eq!(sessions.len(), 1, "the empty session must not be listed");
        assert_eq!(sessions[0].id, used);
    }

    #[test]
    fn list_sessions_excludes_a_session_with_only_assistant_messages() {
        // An assistant-only session (no role='user' row) is just as "never actually used" as a
        // fully empty one — the filter checks role='user' specifically, not just any message.
        let store = Store::open_in_memory().unwrap();
        let sid = store.create_session("/x", "default").unwrap();
        store
            .add_message(&sid, 0, Role::Assistant, "unsolicited", Some("opus"))
            .unwrap();
        assert!(store.list_sessions().unwrap().is_empty());
    }

    #[test]
    fn prune_empty_removes_old_unused_sessions_but_keeps_recent_and_used_ones() {
        let store = Store::open_in_memory().unwrap();
        let old_empty = store.create_session("/old-empty", "default").unwrap();
        let recent_empty = store.create_session("/recent-empty", "default").unwrap();
        let old_used = store.create_session("/old-used", "default").unwrap();
        store
            .add_message(&old_used, 0, Role::User, "kept", None)
            .unwrap();

        // Backdate `old_empty` and `old_used` past the horizon; `recent_empty` stays fresh (as if
        // just created this instant) so it must survive the sweep.
        let past = chrono::Utc::now().timestamp() - 3600;
        store
            .lock()
            .unwrap()
            .execute(
                "UPDATE session SET created_at = ?1 WHERE id IN (?2, ?3)",
                rusqlite::params![past, old_empty, old_used],
            )
            .unwrap();

        let removed = store.prune_empty(600, 50).unwrap();
        assert_eq!(removed, 1, "only the old + empty session is eligible");

        let remaining: Vec<String> = store
            .list_sessions()
            .unwrap()
            .into_iter()
            .map(|s| s.id)
            .collect();
        // recent_empty is filtered from list_sessions (no user message) but must still EXIST —
        // prune_empty shouldn't have touched it.
        assert!(remaining.contains(&old_used));
        assert!(!remaining.contains(&old_empty));
        let ids_after: Vec<String> = {
            let conn = store.lock().unwrap();
            let mut stmt = conn.prepare("SELECT id FROM session").unwrap();
            stmt.query_map([], |r| r.get::<_, String>(0))
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap()
        };
        assert!(ids_after.contains(&recent_empty), "too young to prune");
        assert!(!ids_after.contains(&old_empty), "old + empty: pruned");
        assert!(ids_after.contains(&old_used), "has a real message: kept");
    }

    #[test]
    fn prune_empty_keeps_session_whose_user_message_was_soft_deleted() {
        // /undo and checkpoint-restore soft-delete a message (active = 0) without removing the
        // row (deactivate_messages_from). A session that genuinely received a user message and
        // then had it rewound must NOT look "never used" to prune_empty — otherwise it gets
        // permanently hard-deleted, taking real (soft-deleted) transcript + checkpoints with it.
        let store = Store::open_in_memory().unwrap();
        let sid = store.create_session("/undone", "default").unwrap();
        store
            .add_message(&sid, 0, Role::User, "real prompt", None)
            .unwrap();
        store.deactivate_messages_from(&sid, 0).unwrap();

        // Backdate past the empty-session horizon, as if the sweep ran much later.
        let past = chrono::Utc::now().timestamp() - 3600;
        store
            .lock()
            .unwrap()
            .execute(
                "UPDATE session SET created_at = ?1 WHERE id = ?2",
                rusqlite::params![past, sid],
            )
            .unwrap();

        let removed = store.prune_empty(600, 50).unwrap();
        assert_eq!(
            removed, 0,
            "a session with a soft-deleted user message was actually used and must survive"
        );
        assert!(store.session_exists(&sid).unwrap());
    }

    #[test]
    fn list_sessions_includes_session_whose_user_message_was_soft_deleted() {
        // Same "actually used" bar as list_sessions_excludes_sessions_with_no_user_message, but
        // for a message that was soft-deleted after the fact — it must still count as used.
        let store = Store::open_in_memory().unwrap();
        let sid = store.create_session("/undone", "default").unwrap();
        store
            .add_message(&sid, 0, Role::User, "real prompt", None)
            .unwrap();
        store.deactivate_messages_from(&sid, 0).unwrap();
        let sessions = store.list_sessions().unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, sid);
    }

    #[test]
    fn duel_outcome_roundtrips_and_boost_math_is_correct() {
        let store = Store::open_in_memory().unwrap();
        let repo = "/home/user/proj";

        // No history yet → empty boosts.
        assert!(store.duel_boosts(repo).unwrap().is_empty());

        // model A: 2 wins, 0 losses -> boost = min(2*0.5, 2.0) = 1.0
        store
            .record_duel_outcome(repo, "provA::one", true, "task 1")
            .unwrap();
        store
            .record_duel_outcome(repo, "provA::one", true, "task 2")
            .unwrap();
        // model B: 1 win, 1 loss -> boost = 0*0.5 = 0.0
        store
            .record_duel_outcome(repo, "provB::two", true, "task 1")
            .unwrap();
        store
            .record_duel_outcome(repo, "provB::two", false, "task 2")
            .unwrap();
        // model C: 0 wins, 1 loss -> boost = -1*0.5 = -0.5
        store
            .record_duel_outcome(repo, "provC::three", false, "task 1")
            .unwrap();

        let boosts = store.duel_boosts(repo).unwrap();
        assert_eq!(boosts.get("provA::one").copied(), Some(1.0));
        assert_eq!(boosts.get("provB::two").copied(), Some(0.0));
        assert_eq!(boosts.get("provC::three").copied(), Some(-0.5));

        // Boosts are scoped per-repo: a different repo sees nothing.
        assert!(store.duel_boosts("/some/other/repo").unwrap().is_empty());
    }

    #[test]
    fn schedule_roundtrips_list_last_run_and_remove() {
        let store = Store::open_in_memory().unwrap();
        assert!(store.list_schedules().unwrap().is_empty());

        let id = forge_types::new_id();
        store
            .add_schedule(
                &id,
                "check the deploy",
                "/home/user/proj",
                Some("bypass"),
                None,
                "every:30m",
            )
            .unwrap();

        let rows = store.list_schedules().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, id);
        assert_eq!(rows[0].task, "check the deploy");
        assert_eq!(rows[0].cwd, "/home/user/proj");
        assert_eq!(rows[0].mode.as_deref(), Some("bypass"));
        assert_eq!(rows[0].model, None);
        assert_eq!(rows[0].cron, "every:30m");
        assert!(rows[0].enabled);
        assert_eq!(rows[0].last_run, None);

        let prefix: String = id.chars().take(8).collect();
        assert_eq!(
            store.matching_schedule_ids(&prefix).unwrap(),
            vec![id.clone()]
        );

        store.set_schedule_last_run(&id, 12345).unwrap();
        assert_eq!(store.list_schedules().unwrap()[0].last_run, Some(12345));

        assert!(store.remove_schedule(&id).unwrap());
        assert!(store.list_schedules().unwrap().is_empty());
        assert!(!store.remove_schedule(&id).unwrap());
    }

    #[test]
    fn fork_copies_the_prefix_and_links_back() {
        let store = Store::open_in_memory().unwrap();
        let src = store.create_session("/repo", "default").unwrap();
        // Two full turns: (user, assistant) at seqs 0..=3.
        store
            .add_message(&src, 0, Role::User, "first prompt", None)
            .unwrap();
        store
            .add_message(&src, 1, Role::Assistant, "first answer", Some("m::a"))
            .unwrap();
        store
            .add_message(&src, 2, Role::User, "second prompt", None)
            .unwrap();
        store
            .add_message(&src, 3, Role::Assistant, "second answer", Some("m::a"))
            .unwrap();

        // Fork BEFORE the second prompt: the fork carries turn 1 only.
        let fork = store.fork_session(&src, 2).unwrap();
        let msgs = store.load_messages(&fork).unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].content, "first prompt");
        assert_eq!(msgs[1].content, "first answer");

        // Linkage visible to `forge tree`; the source is untouched.
        let nodes = store.fork_nodes().unwrap();
        let node = nodes.iter().find(|node| node.id == fork).unwrap();
        assert_eq!(node.forked_from.as_deref(), Some(src.as_str()));
        assert_eq!(node.forked_at_seq, Some(2));
        assert_eq!(store.load_messages(&src).unwrap().len(), 4);
    }

    #[test]
    fn queue_task_roundtrips_claim_finish_and_remove() {
        let store = Store::open_in_memory().unwrap();
        assert!(store.list_queue_tasks(None).unwrap().is_empty());

        let id = forge_types::new_id();
        store
            .add_queue_task(
                &id,
                "migrate the auth module",
                "/home/user/proj",
                Some("accept-edits"),
                None,
                Some(2.5),
            )
            .unwrap();
        let other = forge_types::new_id();
        store
            .add_queue_task(&other, "other project task", "/elsewhere", None, None, None)
            .unwrap();

        // cwd filter separates projects; None sees both.
        assert_eq!(store.list_queue_tasks(None).unwrap().len(), 2);
        let rows = store.list_queue_tasks(Some("/home/user/proj")).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, id);
        assert_eq!(rows[0].status, "pending");
        assert_eq!(rows[0].budget_usd, Some(2.5));

        let prefix: String = id.chars().take(8).collect();
        assert_eq!(
            store.matching_queue_task_ids(&prefix).unwrap(),
            vec![id.clone()]
        );

        // Claim is single-shot: the second attempt (a concurrent drain) loses.
        assert!(store.claim_queue_task(&id, 100).unwrap());
        assert!(!store.claim_queue_task(&id, 101).unwrap());
        // A running task refuses removal.
        assert!(!store.remove_queue_task(&id).unwrap());

        store
            .finish_queue_task(
                &id,
                "done",
                200,
                Some("sess-1"),
                Some("autopilot/migrate-auth"),
                Some("moved auth to the new module"),
                Some(1.25),
                None,
            )
            .unwrap();
        let row = &store.list_queue_tasks(Some("/home/user/proj")).unwrap()[0];
        assert_eq!(row.status, "done");
        assert_eq!(row.started_at, Some(100));
        assert_eq!(row.finished_at, Some(200));
        assert_eq!(row.branch.as_deref(), Some("autopilot/migrate-auth"));
        assert_eq!(row.cost_usd, Some(1.25));

        assert!(store.remove_queue_task(&id).unwrap());
        assert_eq!(store.list_queue_tasks(None).unwrap().len(), 1);
    }

    #[test]
    fn scoreboard_mirrors_duel_boost_math_and_sorts_by_boost() {
        let store = Store::open_in_memory().unwrap();
        let repo = "/repo/x";
        assert!(store.model_scoreboard(repo).unwrap().is_empty());
        for _ in 0..3 {
            store
                .record_duel_outcome(repo, "free::fast", true, "t")
                .unwrap();
        }
        store
            .record_duel_outcome(repo, "free::fast", false, "t")
            .unwrap();
        store
            .record_duel_outcome(repo, "paid::big", false, "t")
            .unwrap();

        let rows = store.model_scoreboard(repo).unwrap();
        assert_eq!(rows.len(), 2);
        // (3 wins - 1 loss) * 0.5 = +1.0, sorted first; (0 - 1) * 0.5 = -0.5 second.
        assert_eq!(rows[0], ("free::fast".into(), 3, 1, 1.0));
        assert_eq!(rows[1], ("paid::big".into(), 0, 1, -0.5));
        // The scoreboard's boost equals what routing actually receives.
        let boosts = store.duel_boosts(repo).unwrap();
        assert_eq!(boosts.get("free::fast"), Some(&1.0));
        assert_eq!(boosts.get("paid::big"), Some(&-0.5));
    }

    #[test]
    fn duel_boost_clamps_at_the_bound_for_a_long_streak() {
        let store = Store::open_in_memory().unwrap();
        let repo = "/home/user/proj";
        for i in 0..20 {
            store
                .record_duel_outcome(repo, "provA::one", true, &format!("task {i}"))
                .unwrap();
        }
        let boosts = store.duel_boosts(repo).unwrap();
        assert_eq!(
            boosts.get("provA::one").copied(),
            Some(2.0),
            "a long win streak must clamp at +2.0, not grow unbounded"
        );
    }

    #[test]
    fn clear_all_model_health_wipes_every_bench() {
        let store = Store::open_in_memory().unwrap();
        store.bench_model("a", 2000, "rate-limited").unwrap();
        store.bench_model("b", 2000, "auth failed").unwrap();
        assert_eq!(store.clear_all_model_health().unwrap(), 2);
        assert!(store.benched_models(500).unwrap().is_empty());
        assert_eq!(store.clear_all_model_health().unwrap(), 0, "idempotent");
    }

    #[test]
    fn bench_persists_across_reopen() {
        // Same file → a daily-quota bench survives a Forge restart (AC-3).
        let dir = std::env::temp_dir().join(forge_types::new_id());
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("forge.db");
        {
            let store = Store::open(&path).unwrap();
            store
                .bench_model("m", 9_999_999_999, "probe: quota 0")
                .unwrap();
        }
        let store = Store::open(&path).unwrap();
        assert!(store.benched_models(500).unwrap().is_benched("m"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn pool_handles_concurrent_threads_on_a_file_db() {
        // The connection pool must let several threads touch the store at once (file DB + WAL +
        // busy_timeout) without "database is locked" — the point of moving off one Mutex<Connection>.
        use std::sync::Arc;
        let dir = std::env::temp_dir().join(forge_types::new_id());
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("forge.db");
        let store = Arc::new(Store::open(&path).unwrap());
        let sid = store.create_session("/tmp", "default").unwrap();

        let mut handles = Vec::new();
        for t in 0..8i64 {
            let s = Arc::clone(&store);
            let sid = sid.clone();
            handles.push(std::thread::spawn(move || {
                for j in 0..20i64 {
                    s.list_sessions().unwrap();
                    s.message_count(&sid).unwrap();
                    // Unique seq per (thread, iter) so concurrent writers don't collide on the PK.
                    s.add_message(&sid, t * 100 + j, Role::User, "x", None)
                        .unwrap();
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(
            store.message_count(&sid).unwrap(),
            160,
            "all 8×20 concurrent writes landed"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn portable_metadata_round_trips_health_only() {
        // Source store with a real health row + a private message that must NOT be exported.
        let src = Store::open_in_memory().unwrap();
        src.bench_model("gemini::x", 9_999_999_999, "rate-limited")
            .unwrap();
        let sid = src.create_session(".", "default").unwrap();
        src.add_message(&sid, 0, Role::User, "SECRET_PRIVATE_CHAT", None)
            .unwrap();

        let json = src.export_portable_metadata().unwrap();
        assert!(json.contains("gemini::x"), "health row exported: {json}");
        assert!(
            !json.contains("SECRET_PRIVATE_CHAT"),
            "allow-list: messages must never be in the metadata export"
        );
        assert!(!json.contains("\"message\""), "no session tables exported");

        // Import into a fresh store reconstructs the health row; an injected non-allow-listed table
        // in the JSON is ignored (only the 1 health row is written).
        let tampered = json.replace(
            "\"model_health\"",
            "\"message\":{\"columns\":[\"id\",\"content\"],\"rows\":[[\"m\",\"EVIL\"]]},\"model_health\"",
        );
        let dst = Store::open_in_memory().unwrap();
        let n = dst.import_portable_metadata(&tampered).unwrap();
        assert_eq!(n, 1, "only the allow-listed model_health row is imported");
        assert!(dst.benched_models(500).unwrap().is_benched("gemini::x"));
    }

    #[test]
    fn portable_metadata_rejects_injected_column_names() {
        // A tampered bundle names an allow-listed table but smuggles a SQL-injection column name
        // that would be `format!`-interpolated into the INSERT. It must be rejected outright — and a
        // legitimate column set must still import (regression guard for the validation).
        let store = Store::open_in_memory().unwrap();

        let evil = serde_json::json!({
            "model_health": {
                "columns": ["model", "x); DROP TABLE message;--"],
                "rows": [["m", 1]]
            }
        })
        .to_string();
        let err = store
            .import_portable_metadata(&evil)
            .expect_err("an injected column name must be rejected");
        assert!(
            matches!(err, StoreError::Json(_)),
            "expected a rejection error, got {err:?}"
        );
        // The `message` table is untouched (no DROP executed) — a normal write still works.
        let sid = store.create_session(".", "default").unwrap();
        store
            .add_message(&sid, 0, Role::User, "still here", None)
            .unwrap();

        // A legitimate export round-trips cleanly through the now-stricter import.
        let src = Store::open_in_memory().unwrap();
        src.bench_model("openai::y", 9_999_999_999, "rate-limited")
            .unwrap();
        let good = src.export_portable_metadata().unwrap();
        let n = store.import_portable_metadata(&good).unwrap();
        assert_eq!(n, 1, "a legitimate column set still imports");
    }

    #[test]
    fn mcp_live_observer_events() {
        let store = Store::open_in_memory().unwrap();
        let sid = store.create_session("/tmp", "default").unwrap();

        // Initially inactive
        let active = store.active_agent_session_ids().unwrap();
        assert!(active.is_empty());

        // Make active
        store.set_session_agent_active(&sid, true).unwrap();
        let active = store.active_agent_session_ids().unwrap();
        assert_eq!(active, vec![sid.clone()]);

        // Append events
        store
            .append_live_event(&sid, "{\"type\":\"Text\",\"delta\":\"hello\"}")
            .unwrap();
        store
            .append_live_event(&sid, "{\"type\":\"Done\"}")
            .unwrap();

        let events = store.live_events_after(&sid, 0).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].1, "{\"type\":\"Text\",\"delta\":\"hello\"}");
        assert_eq!(events[1].1, "{\"type\":\"Done\"}");

        // Test filtering by after_id
        let last_id = events[0].0;
        let events_filtered = store.live_events_after(&sid, last_id).unwrap();
        assert_eq!(events_filtered.len(), 1);
        assert_eq!(events_filtered[0].1, "{\"type\":\"Done\"}");

        // Make inactive
        store.set_session_agent_active(&sid, false).unwrap();
        let active = store.active_agent_session_ids().unwrap();
        assert!(active.is_empty());
    }

    // --- Concurrency + integrity hardening (v2.0) -------------------------------------------

    use std::sync::atomic::{AtomicUsize, Ordering};
    static DB_N: AtomicUsize = AtomicUsize::new(0);

    /// A unique temp DB path (file-backed, so two `Store` handles share ONE database — in-memory
    /// stores can't, every `:memory:` open is a distinct DB). Cleaned up by the caller's process.
    fn temp_db_path() -> std::path::PathBuf {
        let n = DB_N.fetch_add(1, Ordering::SeqCst);
        std::env::temp_dir().join(format!("forge-store-test-{}-{n}.db", std::process::id()))
    }

    fn cleanup(p: &std::path::Path) {
        let _ = std::fs::remove_file(p);
        let _ = std::fs::remove_file(p.with_extension("db-wal"));
        let _ = std::fs::remove_file(p.with_extension("db-shm"));
    }

    #[test]
    fn concurrent_writers_dont_drop_rows_or_dup_seqs() {
        // Two independent Store handles (two pools) on the SAME file DB, several threads each, all
        // appending to ONE session. With IMMEDIATE txns + busy-retry no append is lost, and the
        // UNIQUE(session_id, seq) index + atomic re-allocation keeps every seq distinct — so the
        // final next_seq equals the row count (seqs are a gapless 0..N), proving no collision.
        let path = temp_db_path();
        let store_a = std::sync::Arc::new(Store::open(&path).unwrap());
        let store_b = std::sync::Arc::new(Store::open(&path).unwrap());
        let sid = store_a.create_session("/tmp", "default").unwrap();

        const THREADS: usize = 6;
        const PER: usize = 40;
        let mut handles = Vec::new();
        for t in 0..THREADS {
            let store = if t.is_multiple_of(2) {
                std::sync::Arc::clone(&store_a)
            } else {
                std::sync::Arc::clone(&store_b)
            };
            let sid = sid.clone();
            handles.push(std::thread::spawn(move || {
                for i in 0..PER {
                    // Deliberately race the read-then-write: every thread reads next_seq then
                    // appends, so threads frequently compute the SAME seq.
                    let seq = store.next_seq_for_session(&sid).unwrap();
                    store
                        .add_message(&sid, seq, Role::User, &format!("{t}-{i}"), None)
                        .expect("append must not be dropped under contention");
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }

        let total = (THREADS * PER) as i64;
        assert_eq!(
            store_a.message_count(&sid).unwrap(),
            total,
            "no appended message was lost"
        );
        assert_eq!(
            store_a.next_seq_for_session(&sid).unwrap(),
            total,
            "seqs are gapless and unique (no two writers shared a seq)"
        );
        cleanup(&path);
    }

    #[test]
    fn two_writers_cannot_produce_a_duplicate_seq() {
        // Force the collision directly: both writers pass the SAME explicit seq. The unique index
        // rejects the second and add_message re-allocates, so both rows land with distinct seqs.
        let path = temp_db_path();
        let store = std::sync::Arc::new(Store::open(&path).unwrap());
        let sid = store.create_session("/tmp", "default").unwrap();

        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let mut handles = Vec::new();
        for w in 0..2 {
            let store = std::sync::Arc::clone(&store);
            let sid = sid.clone();
            let barrier = std::sync::Arc::clone(&barrier);
            handles.push(std::thread::spawn(move || {
                barrier.wait();
                store
                    .add_message(&sid, 0, Role::User, &format!("writer-{w}"), None)
                    .unwrap();
            }));
        }
        for h in handles {
            h.join().unwrap();
        }

        assert_eq!(store.message_count(&sid).unwrap(), 2, "both rows persisted");
        assert_eq!(
            store.next_seq_for_session(&sid).unwrap(),
            2,
            "the two writers got distinct seqs (0 and 1), not a duplicate"
        );
        cleanup(&path);
    }

    #[test]
    fn side_call_usage_survives_concurrent_writers() {
        // record_side_call_usage SELECTs MAX(seq) then writes — the read-then-write path that a
        // DEFERRED txn would lose to SQLITE_BUSY_SNAPSHOT. With IMMEDIATE + retry every cost row
        // lands, so the summed session cost equals the number of calls.
        let path = temp_db_path();
        let store_a = std::sync::Arc::new(Store::open(&path).unwrap());
        let store_b = std::sync::Arc::new(Store::open(&path).unwrap());
        let sid = store_a.create_session("/tmp", "default").unwrap();

        const THREADS: usize = 6;
        const PER: usize = 20;
        let mut handles = Vec::new();
        for t in 0..THREADS {
            let store = if t.is_multiple_of(2) {
                std::sync::Arc::clone(&store_a)
            } else {
                std::sync::Arc::clone(&store_b)
            };
            let sid = sid.clone();
            handles.push(std::thread::spawn(move || {
                for _ in 0..PER {
                    store
                        .record_side_call_usage(
                            &sid,
                            "compact",
                            &Usage {
                                input_tokens: 1,
                                output_tokens: 1,
                                cached_input_tokens: 0,
                                cost_usd: 0.01,
                            },
                        )
                        .expect("usage row must not be lost");
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }

        let expected = (THREADS * PER) as f64 * 0.01;
        assert!(
            (store_a.session_cost(&sid).unwrap() - expected).abs() < 1e-6,
            "every side-call cost row was recorded under contention"
        );
        cleanup(&path);
    }

    #[test]
    fn rejects_db_from_a_newer_build() {
        // A DB whose user_version exceeds what this build supports must be refused, not misread.
        let path = temp_db_path();
        Store::open(&path).unwrap(); // create at the current version
        {
            let conn = Connection::open(&path).unwrap();
            conn.pragma_update(None, "user_version", SCHEMA_VERSION + 5)
                .unwrap();
        }
        match Store::open(&path) {
            Err(StoreError::SchemaTooNew { found, supported }) => {
                assert_eq!(found, SCHEMA_VERSION + 5);
                assert_eq!(supported, SCHEMA_VERSION);
            }
            Err(e) => panic!("expected SchemaTooNew, got {e:?}"),
            Ok(_) => panic!("expected SchemaTooNew, but the DB opened"),
        }
        cleanup(&path);
    }

    #[test]
    fn migration_0008_applies_to_a_v7_db_and_is_idempotent() {
        // A v7 DB (session table without worktree_path/archived, no push_subscription table)
        // must upgrade to v8 with exactly migration_0008's changes — and a second open must be
        // a clean no-op (idempotence).
        let path = temp_db_path();
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE session (
                     id TEXT PRIMARY KEY, title TEXT, cwd TEXT NOT NULL,
                     permission_mode TEXT NOT NULL,
                     created_at INTEGER NOT NULL DEFAULT (strftime('%s','now')),
                     updated_at INTEGER NOT NULL DEFAULT (strftime('%s','now')),
                     total_cost_usd REAL NOT NULL DEFAULT 0,
                     parent_session_id TEXT, forked_from TEXT, forked_at_seq INTEGER,
                     view_snapshot TEXT, agent_active INTEGER NOT NULL DEFAULT 0
                 );
                 INSERT INTO session (id, cwd, permission_mode) VALUES ('s7', '/tmp', 'default');",
            )
            .unwrap();
            conn.pragma_update(None, "user_version", 7).unwrap();
        }
        for pass in ["first open (migrates)", "second open (idempotent)"] {
            let store = Store::open(&path).unwrap_or_else(|e| panic!("{pass}: {e:?}"));
            let conn = store.lock().unwrap();
            assert_eq!(
                conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
                    .unwrap(),
                SCHEMA_VERSION,
                "{pass}: at v8"
            );
            // The new columns exist, defaulted, on the pre-existing row.
            let (wt, archived): (Option<String>, i64) = conn
                .query_row(
                    "SELECT worktree_path, archived FROM session WHERE id = 's7'",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .unwrap();
            assert_eq!(wt, None, "{pass}: worktree_path defaults NULL");
            assert_eq!(archived, 0, "{pass}: archived defaults 0");
            // The push_subscription table exists and is writable (pre-added for Phase 5).
            conn.execute(
                "INSERT OR REPLACE INTO push_subscription (id, endpoint, p256dh, auth)
                 VALUES ('p1', 'https://push.example/x', 'key', 'auth')",
                [],
            )
            .unwrap();
        }
        cleanup(&path);
    }

    /// A `usage` table shaped as it was at schema v17 — before migration_0018 added
    /// `cached_input_tokens`. Stamping a version from the ambiguous 18..=21 window on top of this is
    /// what a database written by the unreleased Forge Anywhere branch looks like.
    fn write_pre_v18_usage_table(path: &std::path::Path, stamp_version: i64) {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(
            "CREATE TABLE usage (
                 id TEXT PRIMARY KEY,
                 message_id TEXT NOT NULL,
                 provider TEXT,
                 model TEXT,
                 input_tokens INTEGER NOT NULL,
                 output_tokens INTEGER NOT NULL,
                 cost_usd REAL NOT NULL,
                 created_at INTEGER NOT NULL DEFAULT (strftime('%s','now'))
             );
             INSERT INTO usage
                 (id, message_id, provider, model, input_tokens, output_tokens, cost_usd)
             VALUES ('u-pre', 'm-pre', 'provider', 'model', 7, 8, 0.9);",
        )
        .unwrap();
        conn.pragma_update(None, "user_version", stamp_version)
            .unwrap();
    }

    /// The regression: `run_migrations` used to rewind `user_version` 18..=21 to 17 unconditionally,
    /// so opening an ALREADY-CURRENT database wrote page 1 twice and re-ran two migrations — turning
    /// every short-lived Forge process (CLI subcommand, statusline probe, mcp-serve) into a writer
    /// that failed with "database is locked" whenever a long lattice write held the WAL writer.
    ///
    /// `PRAGMA query_only` makes any write attempt fail with SQLITE_READONLY, so this asserts the
    /// zero-write property directly rather than inferring it.
    #[test]
    fn run_migrations_writes_nothing_when_the_database_is_already_current() {
        let path = temp_db_path();
        Store::open(&path).unwrap(); // create + migrate to SCHEMA_VERSION, then close
        let conn = Connection::open(&path).unwrap();
        assert_eq!(
            conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            SCHEMA_VERSION,
        );
        conn.pragma_update(None, "query_only", true).unwrap();
        // Prove the probe is real: the write the old code performed is rejected under query_only.
        assert!(
            conn.pragma_update(None, "user_version", 17i64).is_err(),
            "query_only must reject the rewind the old code did on every open"
        );
        run_migrations(&conn).expect("a fully-migrated database must open without writing");
        conn.pragma_update(None, "query_only", false).unwrap();
        assert_eq!(
            conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            SCHEMA_VERSION,
            "the version is left exactly as found"
        );
        drop(conn);
        cleanup(&path);
    }

    /// The user-visible half of the same bug: because opening the store wrote (a `user_version`
    /// rewind plus two unconditional singleton `INSERT`s), a freshly launched Forge process could not
    /// open an already-migrated database at all while a long lattice write held the single WAL
    /// writer — it failed with "database is locked" instead of just reading. Opening must now
    /// succeed with another connection sitting in an exclusive write transaction the whole time.
    #[test]
    fn opening_a_current_database_succeeds_while_another_writer_holds_the_lock() {
        let path = temp_db_path();
        Store::open(&path).unwrap(); // create + migrate, then close
        let blocker = Connection::open(&path).unwrap();
        blocker
            .busy_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        blocker.execute_batch("BEGIN EXCLUSIVE").unwrap();

        let store =
            Store::open(&path).expect("opening a migrated database must not need the writer");
        {
            let conn = store.lock().unwrap();
            assert_eq!(
                conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
                    .unwrap(),
                SCHEMA_VERSION,
            );
        }
        blocker.execute_batch("ROLLBACK").unwrap();
        drop(blocker);
        drop(store);
        cleanup(&path);
    }

    /// The seeded singleton rows still exist (moving them out of `schema::SCHEMA` must not have lost
    /// them), and re-seeding an initialized database is a no-op.
    #[test]
    fn singleton_rows_are_seeded_once_and_never_duplicated() {
        let store = Store::open_in_memory().unwrap();
        let conn = store.lock().unwrap();
        for table in ["anywhere_sync_state", "anywhere_sync_cursor"] {
            let n: i64 = conn
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
                .unwrap();
            assert_eq!(n, 1, "{table} is seeded exactly once");
        }
        seed_singleton_rows(&conn).unwrap();
        seed_singleton_rows(&conn).unwrap();
        for table in ["anywhere_sync_state", "anywhere_sync_cursor"] {
            let n: i64 = conn
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
                .unwrap();
            assert_eq!(n, 1, "{table} stays at one row");
        }
    }

    /// The compatibility mechanism this repair exists for: a database stamped inside 18..=21 by the
    /// unreleased Anywhere branch never ran the PUBLIC steps 18+, so those steps have to be replayed
    /// and the number renumbered down to what this build ships. Covers every version in the window.
    #[test]
    fn prerelease_anywhere_versions_are_repaired_and_renumbered() {
        for stamped in ANYWHERE_PRERELEASE_MIN_VERSION..=ANYWHERE_PRERELEASE_MAX_VERSION {
            let path = temp_db_path();
            write_pre_v18_usage_table(&path, stamped);
            for pass in ["first open (repairs)", "second open (idempotent)"] {
                let store = Store::open(&path)
                    .unwrap_or_else(|e| panic!("stamped {stamped}, {pass}: {e:?}"));
                let conn = store.lock().unwrap();
                assert_eq!(
                    conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
                        .unwrap(),
                    SCHEMA_VERSION,
                    "stamped {stamped}, {pass}: renumbered to the public version"
                );
                // Public migration 18's column was applied despite the number claiming otherwise…
                let (input, cached): (i64, i64) = conn
                    .query_row(
                        "SELECT input_tokens, cached_input_tokens FROM usage WHERE id = 'u-pre'",
                        [],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .unwrap_or_else(|e| panic!("stamped {stamped}, {pass}: {e:?}"));
                assert_eq!((input, cached), (7, 0), "stamped {stamped}, {pass}");
                // …and so was public migration 19's table.
                assert!(
                    conn.prepare("SELECT 1 FROM workflow_run")
                        .unwrap()
                        .exists([])
                        .is_ok(),
                    "stamped {stamped}, {pass}: workflow_run exists"
                );
            }
            cleanup(&path);
        }
    }

    /// The other half of the window's ambiguity: a database at 20/21 that DOES carry public
    /// migration 18's column cannot have come from the pre-release branch (that branch forked at
    /// SCHEMA_VERSION 17, before the column existed), so it was written by a newer public build and
    /// must be REFUSED. The old code renumbered it down and opened it, silently bypassing the
    /// `SchemaTooNew` guard for exactly the two versions above what it supported.
    #[test]
    fn a_newer_public_database_inside_the_window_is_refused_not_downgraded() {
        if SCHEMA_VERSION >= ANYWHERE_PRERELEASE_MAX_VERSION {
            return; // the window is fully consumed by public versions; nothing ambiguous is left
        }
        let path = temp_db_path();
        Store::open(&path).unwrap(); // fully migrated, so the public v18 marker is present
        {
            let conn = Connection::open(&path).unwrap();
            conn.pragma_update(None, "user_version", ANYWHERE_PRERELEASE_MAX_VERSION)
                .unwrap();
        }
        match Store::open(&path) {
            Err(StoreError::SchemaTooNew { found, supported }) => {
                assert_eq!(found, ANYWHERE_PRERELEASE_MAX_VERSION);
                assert_eq!(supported, SCHEMA_VERSION);
            }
            Err(e) => panic!("expected SchemaTooNew, got {e:?}"),
            Ok(_) => panic!("a database from a newer build must not be silently downgraded"),
        }
        cleanup(&path);
    }

    /// Forward/backward compatibility with an OLDER binary. The previous public build repairs the
    /// ambiguous window by rewinding `user_version` to 17 and re-running every step from there, so a
    /// database this build wrote must survive that round trip: the rewind must not lose data, and
    /// re-running the in-window steps must stay a clean no-op. That "additive and idempotent from 18
    /// on" contract is what makes the whole mechanism safe in both directions.
    #[test]
    fn a_database_written_by_this_build_survives_an_older_binarys_rewind_to_17() {
        let path = temp_db_path();
        let sid = {
            let store = Store::open(&path).unwrap();
            let sid = store.create_session("/tmp/proj", "default").unwrap();
            store
                .add_message(&sid, 1, Role::User, "keep me", None)
                .unwrap();
            sid
        };
        // What the older binary's repair does on open.
        {
            let conn = Connection::open(&path).unwrap();
            conn.pragma_update(None, "user_version", 17i64).unwrap();
        }
        let store = Store::open(&path).expect("reopening after a rewind must succeed");
        {
            let conn = store.lock().unwrap();
            assert_eq!(
                conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
                    .unwrap(),
                SCHEMA_VERSION,
                "the rewind is re-applied up to the current version"
            );
            // Every in-window step is re-runnable, twice, with no error — the contract that lets an
            // older binary rewind at all.
            for _ in 0..2 {
                for step in ANYWHERE_PRERELEASE_MIN_VERSION..=SCHEMA_VERSION {
                    MIGRATIONS[(step - 1) as usize](&conn)
                        .unwrap_or_else(|e| panic!("migration {step} is not idempotent: {e:?}"));
                }
            }
        }
        assert_eq!(
            store.load_messages(&sid).unwrap().len(),
            1,
            "the transcript survived the round trip"
        );
        cleanup(&path);
    }

    /// WAL growth is bounded automatically. `journal_size_limit` is per-connection and is NOT stored
    /// in the file, so the load-bearing assertion is that EVERY connection the pool hands out carries
    /// it — a single connection missing it is a checkpoint that leaves the `-wal` at its high-water
    /// mark forever (656 MB was observed in the wild).
    #[test]
    fn wal_growth_is_bounded_on_every_pooled_connection() {
        let path = temp_db_path();
        let store = Store::open(&path).unwrap();
        // Held simultaneously, so the pool is forced to open four DISTINCT connections rather than
        // handing the same one back each time.
        let held: Vec<_> = (0..4).map(|_| store.lock().unwrap()).collect();
        for (i, conn) in held.iter().enumerate() {
            let limit: i64 = conn
                .query_row("PRAGMA journal_size_limit", [], |r| r.get(0))
                .unwrap();
            assert_eq!(
                limit, WAL_SIZE_LIMIT_BYTES,
                "pooled connection {i} must bound the WAL"
            );
        }
        drop(held);
        // Write enough to cross the autocheckpoint threshold several times, then check the file.
        let sid = store.create_session("/tmp/proj", "default").unwrap();
        let big = "x".repeat(4096);
        for i in 0..400 {
            store.add_message(&sid, i, Role::User, &big, None).unwrap();
        }
        let wal = path.with_extension("db-wal");
        let wal_len = std::fs::metadata(&wal).map(|m| m.len()).unwrap_or(0);
        assert!(
            wal_len <= WAL_SIZE_LIMIT_BYTES as u64,
            "-wal grew to {wal_len} bytes, past the {WAL_SIZE_LIMIT_BYTES}-byte limit"
        );
        drop(store);
        cleanup(&path);
    }

    /// The explicit, user-initiated reclaim (`forge lattice prune --vacuum`) still shrinks the WAL to
    /// nothing. Deliberately NOT wired to run automatically — see [`Store::vacuum`].
    #[test]
    fn vacuum_truncates_the_wal() {
        let path = temp_db_path();
        let store = Store::open(&path).unwrap();
        let sid = store.create_session("/tmp/proj", "default").unwrap();
        for i in 0..200 {
            store
                .add_message(&sid, i, Role::User, &format!("m{i}"), None)
                .unwrap();
        }
        store.vacuum().unwrap();
        let wal_len = std::fs::metadata(path.with_extension("db-wal"))
            .map(|m| m.len())
            .unwrap_or(0);
        assert_eq!(wal_len, 0, "a truncating checkpoint leaves an empty -wal");
        drop(store);
        cleanup(&path);
    }

    /// Every lattice count/embedding query must report only its own repo root. The store is global —
    /// one database across every project and every bench clone — so an unscoped query made
    /// `forge lattice status` print another project's totals as this project's, spent this project's
    /// embedding quota on another project's nodes, and cosine-ranked another project's symbols
    /// against this project's prompt.
    #[test]
    fn lattice_counts_and_embeddings_are_scoped_to_their_repo_root() {
        let store = Store::open_in_memory().unwrap();
        let mk = |root: &str, tag: &str, nodes: usize| {
            let file = LatticeFileRow {
                id: format!("f-{tag}"),
                repo_root: root.into(),
                rel_path: "a.rs".into(),
                lang: "rust".into(),
                content_hash: "h".into(),
                parse_status: "ok".into(),
            };
            let rows: Vec<LatticeNodeRow> = (0..nodes)
                .map(|i| LatticeNodeRow {
                    id: format!("n-{tag}-{i}"),
                    file_id: format!("f-{tag}"),
                    kind: "function".into(),
                    name: format!("sym_{tag}_{i}"),
                    qualname: None,
                    signature: None,
                    span_start: 0,
                    span_end: 1,
                    line_start: 1,
                    pagerank: 0.0,
                })
                .collect();
            let edges = vec![LatticeEdgeRow {
                id: format!("e-{tag}"),
                src_id: format!("n-{tag}-0"),
                dst_id: format!("n-{tag}-1"),
                kind: "calls".into(),
                unresolved_name: None,
            }];
            let refs = vec![LatticeRefRow {
                id: format!("r-{tag}"),
                src_id: format!("n-{tag}-0"),
                name: format!("sym_{tag}_1"),
                kind: "call".into(),
                line: 1,
            }];
            store
                .replace_lattice_file(&file, &rows, &edges, &refs)
                .unwrap();
        };
        mk("/repo/a", "a", 2);
        mk("/repo/b", "b", 3);
        // One embedding in each root, so neither can satisfy the other's queries by accident.
        store.put_lattice_embedding("n-a-0", &[1.0, 0.0]).unwrap();
        store.put_lattice_embedding("n-b-0", &[0.0, 1.0]).unwrap();

        assert_eq!(store.lattice_counts("/repo/a").unwrap(), (1, 2, 1));
        assert_eq!(store.lattice_counts("/repo/b").unwrap(), (1, 3, 1));
        assert_eq!(store.lattice_counts("/repo/none").unwrap(), (0, 0, 0));

        assert_eq!(store.lattice_ref_count("/repo/a").unwrap(), 1);
        assert_eq!(store.lattice_ref_count("/repo/b").unwrap(), 1);
        assert_eq!(store.lattice_ref_count("/repo/none").unwrap(), 0);

        assert_eq!(store.lattice_embedding_count("/repo/a").unwrap(), 1);
        assert_eq!(store.lattice_embedding_count("/repo/none").unwrap(), 0);
        assert_eq!(
            store.lattice_embeddings("/repo/a").unwrap(),
            vec![("n-a-0".to_string(), vec![1.0, 0.0])],
            "only this root's vectors are cosine-ranked"
        );
        assert!(store.lattice_embeddings("/repo/none").unwrap().is_empty());

        let pending: Vec<String> = store
            .lattice_nodes_without_embedding("/repo/a", 50)
            .unwrap()
            .into_iter()
            .map(|n| n.id)
            .collect();
        assert_eq!(
            pending,
            vec!["n-a-1".to_string()],
            "embed_pending must never spend calls on another project's nodes"
        );
        assert!(store
            .lattice_nodes_without_embedding("/repo/none", 50)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn migration_0018_preserves_v17_usage_and_adds_cached_input_tokens() {
        let path = temp_db_path();
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE usage (
                     id TEXT PRIMARY KEY,
                     message_id TEXT NOT NULL,
                     provider TEXT,
                     model TEXT,
                     input_tokens INTEGER NOT NULL,
                     output_tokens INTEGER NOT NULL,
                     cost_usd REAL NOT NULL,
                     created_at INTEGER NOT NULL DEFAULT (strftime('%s','now'))
                 );
                 INSERT INTO usage
                     (id, message_id, provider, model, input_tokens, output_tokens, cost_usd)
                 VALUES ('u17', 'm17', 'provider', 'model', 123, 45, 0.67);",
            )
            .unwrap();
            conn.pragma_update(None, "user_version", 17i64).unwrap();
        }

        for pass in ["first open (migrates)", "second open (idempotent)"] {
            let store = Store::open(&path).unwrap_or_else(|e| panic!("{pass}: {e:?}"));
            let conn = store.lock().unwrap();
            assert_eq!(
                conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
                    .unwrap(),
                SCHEMA_VERSION,
                "{pass}: upgraded to the current version"
            );
            let (input, cached, output, cost): (i64, i64, i64, f64) = conn
                .query_row(
                    "SELECT input_tokens, cached_input_tokens, output_tokens, cost_usd
                     FROM usage WHERE id = 'u17'",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                )
                .unwrap();
            assert_eq!((input, cached, output), (123, 0, 45), "{pass}");
            assert!((cost - 0.67).abs() < f64::EPSILON, "{pass}");
        }
        cleanup(&path);
    }

    #[test]
    fn push_subscription_crud_dedupes_by_endpoint() {
        let store = Store::open_in_memory().unwrap();
        assert!(store.list_push_subscriptions().unwrap().is_empty());

        let id1 = store
            .upsert_push_subscription("https://push.example/a", "keyA", "authA")
            .unwrap();
        let id2 = store
            .upsert_push_subscription("https://push.example/b", "keyB", "authB")
            .unwrap();
        assert_ne!(id1, id2);
        assert_eq!(store.list_push_subscriptions().unwrap().len(), 2);

        // Re-subscribing the SAME endpoint refreshes the keys in place — no duplicate row.
        let id1b = store
            .upsert_push_subscription("https://push.example/a", "keyA2", "authA2")
            .unwrap();
        assert_eq!(id1, id1b, "same endpoint keeps its row id");
        let subs = store.list_push_subscriptions().unwrap();
        assert_eq!(subs.len(), 2, "dedupe by endpoint");
        let a = subs.iter().find(|s| s.id == id1).unwrap();
        assert_eq!((a.p256dh.as_str(), a.auth.as_str()), ("keyA2", "authA2"));
        assert_eq!(a.endpoint, "https://push.example/a");

        // Delete by endpoint; deleting again reports nothing removed.
        assert!(store
            .delete_push_subscription("https://push.example/a")
            .unwrap());
        assert!(!store
            .delete_push_subscription("https://push.example/a")
            .unwrap());
        let left = store.list_push_subscriptions().unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].endpoint, "https://push.example/b");
    }

    #[test]
    fn push_subscription_endpoint_has_a_unique_index() {
        // migration_0013 builds a UNIQUE index on endpoint, so a raw duplicate INSERT is rejected
        // at the DB level — the atomic upsert can rely on ON CONFLICT(endpoint) resolving it.
        let store = Store::open_in_memory().unwrap();
        let conn = store.lock().unwrap();
        conn.execute(
            "INSERT INTO push_subscription (id, endpoint, p256dh, auth)
             VALUES ('p1', 'https://push.example/dup', 'k', 'a')",
            [],
        )
        .unwrap();
        let dup = conn.execute(
            "INSERT INTO push_subscription (id, endpoint, p256dh, auth)
             VALUES ('p2', 'https://push.example/dup', 'k', 'a')",
            [],
        );
        assert!(dup.is_err(), "duplicate endpoint rejected by UNIQUE index");
    }

    #[test]
    fn apns_subscription_crud_dedupes_by_device_token() {
        let store = Store::open_in_memory().unwrap();
        assert!(store.list_apns_subscriptions().unwrap().is_empty());

        let id1 = store.upsert_apns_subscription("tokenA", "sandbox").unwrap();
        let id2 = store
            .upsert_apns_subscription("tokenB", "production")
            .unwrap();
        assert_ne!(id1, id2);
        assert_eq!(store.list_apns_subscriptions().unwrap().len(), 2);

        // Re-registering the SAME device token refreshes its environment in place — no
        // duplicate row (e.g. a debug build reinstalled as a TestFlight build, same device).
        let id1b = store
            .upsert_apns_subscription("tokenA", "production")
            .unwrap();
        assert_eq!(id1, id1b, "same device token keeps its row id");
        let subs = store.list_apns_subscriptions().unwrap();
        assert_eq!(subs.len(), 2, "dedupe by device token");
        let a = subs.iter().find(|s| s.id == id1).unwrap();
        assert_eq!(a.environment, "production");
        assert_eq!(a.device_token, "tokenA");

        // Delete by device token; deleting again reports nothing removed.
        assert!(store.delete_apns_subscription("tokenA").unwrap());
        assert!(!store.delete_apns_subscription("tokenA").unwrap());
        let left = store.list_apns_subscriptions().unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].device_token, "tokenB");
    }

    #[test]
    fn live_activity_token_upserts_and_replaces_by_session() {
        let store = Store::open_in_memory().unwrap();
        assert!(store.get_live_activity_token("sess1").unwrap().is_none());

        store
            .upsert_live_activity_token("sess1", "tokA", "sandbox")
            .unwrap();
        let got = store.get_live_activity_token("sess1").unwrap().unwrap();
        assert_eq!(got.session_id, "sess1");
        assert_eq!(got.push_token, "tokA");
        assert_eq!(got.environment, "sandbox");

        // Re-registering the SAME session replaces the token/environment in place — no
        // duplicate row (e.g. the OS reissues a fresh token for a still-running activity).
        store
            .upsert_live_activity_token("sess1", "tokB", "production")
            .unwrap();
        let got = store.get_live_activity_token("sess1").unwrap().unwrap();
        assert_eq!(got.push_token, "tokB");
        assert_eq!(got.environment, "production");

        // A different session's token doesn't collide with sess1's row.
        store
            .upsert_live_activity_token("sess2", "tokC", "sandbox")
            .unwrap();
        let sess1_again = store.get_live_activity_token("sess1").unwrap().unwrap();
        assert_eq!(sess1_again.push_token, "tokB", "sess1 unaffected by sess2");
        let sess2 = store.get_live_activity_token("sess2").unwrap().unwrap();
        assert_eq!(sess2.push_token, "tokC");

        assert!(store.delete_live_activity_token("sess1").unwrap());
        assert!(!store.delete_live_activity_token("sess1").unwrap());
        assert!(store.get_live_activity_token("sess1").unwrap().is_none());
    }

    #[test]
    fn archived_sessions_are_hidden_from_list_sessions() {
        let store = Store::open_in_memory().unwrap();
        let a = store.create_session("/tmp", "default").unwrap();
        let b = store.create_session("/tmp", "default").unwrap();
        store.add_message(&a, 0, Role::User, "hi a", None).unwrap();
        store.add_message(&b, 0, Role::User, "hi b", None).unwrap();
        let ids: Vec<String> = store
            .list_sessions()
            .unwrap()
            .into_iter()
            .map(|s| s.id)
            .collect();
        assert!(ids.contains(&a) && ids.contains(&b), "both listed: {ids:?}");

        store.archive_session(&a).unwrap();
        assert!(store.session_archived(&a).unwrap());
        assert!(!store.session_archived(&b).unwrap());
        let ids: Vec<String> = store
            .list_sessions()
            .unwrap()
            .into_iter()
            .map(|s| s.id)
            .collect();
        assert!(!ids.contains(&a), "archived session hidden: {ids:?}");
        assert!(ids.contains(&b), "live session still listed");
        assert_eq!(store.load_messages(&a).unwrap().len(), 1);

        let archived = store.list_sessions_for_resume().unwrap();
        assert!(
            archived.iter().any(|s| s.id == a && s.archived),
            "archived session remains resumable"
        );
        store.unarchive_session(&a).unwrap();
        assert!(
            store.list_sessions().unwrap().iter().any(|s| s.id == a),
            "unarchived session returns to normal list"
        );
    }

    #[test]
    fn session_worktree_and_title_round_trip() {
        let store = Store::open_in_memory().unwrap();
        let sid = store.create_session("/repo", "default").unwrap();
        assert_eq!(store.session_worktree(&sid).unwrap(), None);
        store
            .set_session_worktree(&sid, "/repo/.forge/worktrees/abc")
            .unwrap();
        assert_eq!(
            store.session_worktree(&sid).unwrap().as_deref(),
            Some("/repo/.forge/worktrees/abc")
        );
        assert_eq!(store.session_title(&sid).unwrap(), None);
        store.set_session_title(&sid, "fix the parser").unwrap();
        assert_eq!(
            store.session_title(&sid).unwrap().as_deref(),
            Some("fix the parser")
        );
        // list_sessions surfaces both (once the session has a user message).
        store.add_message(&sid, 0, Role::User, "go", None).unwrap();
        let row = store
            .list_sessions()
            .unwrap()
            .into_iter()
            .find(|s| s.id == sid)
            .unwrap();
        assert_eq!(row.title.as_deref(), Some("fix the parser"));
        assert_eq!(
            row.worktree_path.as_deref(),
            Some("/repo/.forge/worktrees/abc")
        );
    }

    #[test]
    fn old_schema_db_upgrades_cleanly() {
        // An existing user DB on the pre-migration schema (no user_version, missing the columns the
        // ad-hoc ALTERs added, and carrying duplicate (session_id, seq) rows from the old seq race)
        // must open, upgrade to the current version, repair the duplicates, and stay usable.
        let path = temp_db_path();
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE session (
                     id TEXT PRIMARY KEY, title TEXT, cwd TEXT NOT NULL,
                     permission_mode TEXT NOT NULL,
                     created_at INTEGER NOT NULL DEFAULT (strftime('%s','now')),
                     updated_at INTEGER NOT NULL DEFAULT (strftime('%s','now')),
                     total_cost_usd REAL NOT NULL DEFAULT 0
                 );
                 CREATE TABLE message (
                     id TEXT PRIMARY KEY,
                     session_id TEXT NOT NULL REFERENCES session(id) ON DELETE CASCADE,
                     seq INTEGER NOT NULL, role TEXT NOT NULL, content TEXT NOT NULL,
                     model TEXT,
                     created_at INTEGER NOT NULL DEFAULT (strftime('%s','now'))
                 );
                 INSERT INTO session (id, cwd, permission_mode) VALUES ('s1', '/tmp', 'default');
                 INSERT INTO message (id, session_id, seq, role, content)
                     VALUES ('m1', 's1', 0, 'user', 'a');
                 INSERT INTO message (id, session_id, seq, role, content)
                     VALUES ('m2', 's1', 0, 'user', 'b');",
            )
            .unwrap();
            assert_eq!(
                conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
                    .unwrap(),
                0,
                "starts as a version-0 DB"
            );
        }

        let store = Store::open(&path).unwrap();
        {
            let conn = store.lock().unwrap();
            assert_eq!(
                conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
                    .unwrap(),
                SCHEMA_VERSION,
                "upgraded to the current schema version"
            );
        }
        // Both pre-existing rows survived; the duplicate seq was repaired to a distinct value.
        assert_eq!(store.message_count("s1").unwrap(), 2);
        // The unique index now blocks a fresh duplicate and the store still appends fine.
        let seq = store.next_seq_for_session("s1").unwrap();
        assert_eq!(
            seq, 2,
            "repair renumbered the duplicate; next seq is gapless"
        );
        store.add_message("s1", seq, Role::User, "c", None).unwrap();
        assert_eq!(store.message_count("s1").unwrap(), 3);
        cleanup(&path);
    }

    #[test]
    fn oversized_tool_result_is_capped() {
        let store = Store::open_in_memory().unwrap();
        let sid = store.create_session("/tmp", "default").unwrap();
        let mid = store
            .add_message(&sid, 0, Role::Assistant, "x", None)
            .unwrap();
        let huge = "A".repeat(MAX_RESULT_JSON_BYTES * 3);
        store
            .record_tool_call(&mid, "read_file", "{}", &huge, "allowed", "ok")
            .unwrap();
        let conn = store.lock().unwrap();
        let stored: String = conn
            .query_row(
                "SELECT result_json FROM tool_call WHERE message_id = ?1",
                [&mid],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            stored.len() < huge.len(),
            "oversized result was truncated ({} < {})",
            stored.len(),
            huge.len()
        );
        assert!(stored.contains("truncated"), "carries a truncation marker");
    }

    #[test]
    fn oversized_tool_args_are_capped() {
        // args_json (e.g. a write_file/edit tool call passing the new file body as an argument)
        // must be bounded the same way result_json is, or the same unbounded-growth problem the
        // result cap exists to prevent recurs on the args side.
        let store = Store::open_in_memory().unwrap();
        let sid = store.create_session("/tmp", "default").unwrap();
        let mid = store
            .add_message(&sid, 0, Role::Assistant, "x", None)
            .unwrap();
        let huge_args = "B".repeat(MAX_RESULT_JSON_BYTES * 3);
        store
            .record_tool_call(&mid, "write_file", &huge_args, "ok", "allowed", "ok")
            .unwrap();
        let conn = store.lock().unwrap();
        let stored: String = conn
            .query_row(
                "SELECT args_json FROM tool_call WHERE message_id = ?1",
                [&mid],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            stored.len() < huge_args.len(),
            "oversized args were truncated ({} < {})",
            stored.len(),
            huge_args.len()
        );
        assert!(stored.contains("truncated"), "carries a truncation marker");
    }

    #[test]
    fn record_tool_call_populates_path_for_write_and_edit_but_not_others() {
        let store = Store::open_in_memory().unwrap();
        let sid = store.create_session("/tmp", "default").unwrap();
        let mid = store
            .add_message(&sid, 0, Role::Assistant, "x", None)
            .unwrap();
        store
            .record_tool_call(
                &mid,
                "write_file",
                r#"{"path":"src/a.rs","content":"fn a() {}"}"#,
                "wrote 10 bytes",
                "allowed",
                "ok",
            )
            .unwrap();
        store
            .record_tool_call(
                &mid,
                "read_file",
                r#"{"path":"src/b.rs"}"#,
                "ok",
                "allowed",
                "ok",
            )
            .unwrap();
        let conn = store.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT tool_name, path FROM tool_call ORDER BY rowid")
            .unwrap();
        let rows: Vec<(String, Option<String>)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(
            rows[0],
            ("write_file".to_string(), Some("src/a.rs".to_string()))
        );
        // read_file isn't in file_edits' tool_name filter, but the path column itself is
        // populated generically from any args carrying a top-level "path" string.
        assert_eq!(
            rows[1],
            ("read_file".to_string(), Some("src/b.rs".to_string()))
        );
    }

    #[test]
    fn migration_0002_backfills_path_on_pre_existing_rows() {
        // A DB written before `tool_call.path` existed must have its historic write_file/edit_file
        // rows backfilled from their args_json on upgrade, not just left NULL forever. Build the
        // pre-0002 DB by hand: base schema + migration_0001 only, insert a row, THEN open it
        // through `Store::open` so `migration_0002` runs and backfills it.
        let path = temp_db_path();
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(schema::SCHEMA).unwrap();
            migration_0001(&conn).unwrap();
            conn.pragma_update(None, "user_version", 1i64).unwrap();
            conn.execute(
                "INSERT INTO session (id, cwd, permission_mode) VALUES ('s1', '/tmp', 'default')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO message (id, session_id, seq, role, content) \
                 VALUES ('m1', 's1', 0, 'assistant', 'x')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO tool_call (id, message_id, tool_name, args_json, result_json, permission, status) \
                 VALUES ('tc1', 'm1', 'write_file', '{\"path\":\"src/old.rs\",\"content\":\"x\"}', 'ok', 'allowed', 'ok')",
                [],
            )
            .unwrap();
        }
        let store = Store::open(&path).unwrap();
        let conn = store.lock().unwrap();
        let backfilled: String = conn
            .query_row("SELECT path FROM tool_call WHERE id = 'tc1'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(backfilled, "src/old.rs");
        cleanup(&path);
    }

    #[test]
    fn file_edits_joins_model_session_and_matches_by_path_suffix() {
        let store = Store::open_in_memory().unwrap();
        let sid = store.create_session("/repo", "default").unwrap();
        let mid = store
            .add_message(
                &sid,
                0,
                Role::Assistant,
                "wrote it",
                Some("anthropic::claude"),
            )
            .unwrap();
        store
            .record_tool_call(
                &mid,
                "write_file",
                r#"{"path":"src/main.rs","content":"fn main() {}"}"#,
                "wrote 14 bytes",
                "allowed",
                "ok",
            )
            .unwrap();
        // A non-matching file and a failed call must not show up.
        store
            .record_tool_call(
                &mid,
                "write_file",
                r#"{"path":"src/other.rs","content":"fn other() {}"}"#,
                "wrote 16 bytes",
                "allowed",
                "ok",
            )
            .unwrap();
        store
            .record_tool_call(
                &mid,
                "write_file",
                r#"{"path":"src/main.rs","content":"broken"}"#,
                "permission denied",
                "denied",
                "error",
            )
            .unwrap();

        let rows = store.file_edits("main.rs").unwrap();
        assert_eq!(rows.len(), 1, "only the ok write_file to main.rs matches");
        assert_eq!(rows[0].path, "src/main.rs");
        assert_eq!(rows[0].session_cwd, "/repo");
        assert_eq!(rows[0].model.as_deref(), Some("anthropic::claude"));
        assert_eq!(rows[0].session_id, sid);
    }

    #[test]
    fn file_edits_falls_back_to_routing_decision_when_message_model_is_null() {
        let store = Store::open_in_memory().unwrap();
        let sid = store.create_session("/repo", "default").unwrap();
        let mid = store
            .add_message(&sid, 0, Role::Assistant, "wrote it", None)
            .unwrap();
        store
            .record_routing(&mid, TaskTier::Standard, "openai::gpt-4o", "rationale")
            .unwrap();
        store
            .record_tool_call(
                &mid,
                "edit_file",
                r#"{"path":"src/main.rs","old":"a","new":"b"}"#,
                "ok",
                "allowed",
                "ok",
            )
            .unwrap();
        let rows = store.file_edits("main.rs").unwrap();
        assert_eq!(rows[0].model.as_deref(), Some("openai::gpt-4o"));
    }

    #[test]
    fn turn_context_finds_nearest_user_prompt_and_the_assistant_reply() {
        let store = Store::open_in_memory().unwrap();
        let sid = store.create_session("/repo", "default").unwrap();
        store
            .add_message(&sid, 0, Role::User, "add a counter", None)
            .unwrap();
        store
            .add_message(&sid, 1, Role::Assistant, "adding it now", Some("m"))
            .unwrap();
        let turn = store.turn_context(&sid, 1).unwrap();
        assert_eq!(turn.user_prompt.as_deref(), Some("add a counter"));
        assert_eq!(turn.assistant_content.as_deref(), Some("adding it now"));
    }

    #[test]
    fn prune_removes_only_old_idle_sessions() {
        let store = Store::open_in_memory().unwrap();
        let a = store.create_session("/tmp", "default").unwrap();
        let b = store.create_session("/tmp", "default").unwrap();
        // Recent sessions are untouched by a normal-horizon prune.
        assert_eq!(store.prune(RETENTION_HORIZON_SECS, 50).unwrap(), 0);
        assert!(store.session_cost(&a).is_ok());
        // A horizon in the future treats every idle session as stale → both are pruned (cascade).
        assert_eq!(store.prune(-1, 50).unwrap(), 2);
        assert!(store.session_cost(&a).is_err(), "old session gone");
        assert!(store.session_cost(&b).is_err());
    }

    #[test]
    fn mesh_outcomes_are_bounded_and_aggregate_without_prompt_content() {
        let store = Store::open_in_memory().unwrap();
        for n in 0..130 {
            store
                .record_mesh_outcome(&MeshOutcome {
                    session_id: "session".into(),
                    model: "codex-oauth::gpt-5.4-mini".into(),
                    tier: TaskTier::Standard,
                    started_at: n,
                    completed_at: n,
                    latency_ms: 100 + n as u64,
                    outcome: if n < 10 { "failure" } else { "success" }.into(),
                    error_kind: None,
                    failover_hop: 0,
                    tool_calls: 0,
                    verified_completion: true,
                })
                .unwrap();
        }
        let rows = store.model_outcome_calibration().unwrap();
        let row = rows
            .iter()
            .find(|row| row.model == "codex-oauth::gpt-5.4-mini")
            .unwrap();
        assert_eq!(
            row.samples, 120,
            "only the bounded recent tail is considered"
        );
        assert_eq!(
            row.success_rate, 1.0,
            "old failures decay out of the calibration"
        );
        assert!(row.mean_latency_ms > 100.0);
    }

    #[test]
    fn only_a_live_oauth_codex_snapshot_advances_the_canonical_freshness_gate() {
        let store = Store::open_in_memory().unwrap();
        let hint = forge_types::QuotaHint {
            provider: "codex-cli".into(),
            window: "weekly".into(),
            status: forge_types::QuotaStatus::Ok,
            resets_at: None,
            fraction_used: Some(0.27),
        };
        store.record_codex_quota(&hint).unwrap();
        assert_eq!(store.codex_oauth_quota_age_secs().unwrap(), None);
        store.record_live_codex_oauth_quota(&hint).unwrap();
        assert!(
            store
                .codex_oauth_quota_age_secs()
                .unwrap()
                .is_some_and(|age| age <= 1),
            "a direct header observation, not a bridge alias, owns the refresh gate"
        );
    }

    #[test]
    fn handoff_session_import_remaps_collisions_and_rolls_back_cleanly() {
        let source = Store::open_in_memory().unwrap();
        let session_id = source.create_session("/source", "default").unwrap();
        source
            .add_message(&session_id, 0, Role::User, "portable", None)
            .unwrap();
        source.add_checkpoint(&session_id, Some("idle"), 0).unwrap();
        let export = source.export_handoff_session(&session_id).unwrap();

        let destination = Store::open_in_memory().unwrap();
        destination
            .ensure_session(&session_id, "/existing", "default")
            .unwrap();
        let imported = destination
            .import_handoff_session(&export, "/handoffs/capsule")
            .unwrap();
        assert!(imported.remapped);
        assert_ne!(imported.session_id, session_id);
        assert_eq!(
            destination.load_messages(&imported.session_id).unwrap()[0].content,
            "portable"
        );
        assert_eq!(
            destination
                .list_checkpoints(&imported.session_id)
                .unwrap()
                .len(),
            1
        );
        assert!(destination
            .rollback_handoff_session(&imported.session_id)
            .unwrap());
        assert!(!destination.session_exists(&imported.session_id).unwrap());
        assert!(destination.session_exists(&session_id).unwrap());
    }

    #[test]
    fn source_handoff_freeze_survives_archive_controls_and_transfer() {
        let store = Store::open_in_memory().unwrap();
        let session = store.create_session("/repo", "default").unwrap();
        let capsule = "ab".repeat(16);

        store.begin_source_handoff(&session, &capsule).unwrap();
        assert!(store.session_handoff_blocked(&session).unwrap());
        assert!(store.session_archived(&session).unwrap());
        assert!(store.unarchive_session(&session).is_err());
        store.begin_source_handoff(&session, &capsule).unwrap();

        store
            .mark_source_handoff_transferred(&session, &capsule)
            .unwrap();
        assert!(!store.cancel_source_handoff(&session, &capsule).unwrap());
        assert!(store.unarchive_session(&session).is_err());
        assert!(store.session_archived(&session).unwrap());
    }

    #[test]
    fn cancelled_source_handoff_becomes_resumable() {
        let store = Store::open_in_memory().unwrap();
        let session = store.create_session("/repo", "default").unwrap();
        let capsule = "cd".repeat(16);
        store.begin_source_handoff(&session, &capsule).unwrap();
        assert!(store.cancel_source_handoff(&session, &capsule).unwrap());
        assert!(!store.session_handoff_blocked(&session).unwrap());
        assert!(!store.session_archived(&session).unwrap());
        store.unarchive_session(&session).unwrap();
    }

    #[test]
    fn destination_import_stays_quarantined_until_explicit_activation() {
        let source = Store::open_in_memory().unwrap();
        let source_session = source.create_session("/source", "default").unwrap();
        source
            .add_message(&source_session, 0, Role::User, "portable", None)
            .unwrap();
        let export = source.export_handoff_session(&source_session).unwrap();
        let destination = Store::open_in_memory().unwrap();
        let capsule = "ef".repeat(16);
        let imported = destination
            .import_handoff_session_with_provenance(
                &export,
                "/handoffs/quarantine",
                &HandoffImportProvenance {
                    source_device_id: [7; 16],
                    capsule_id: capsule.clone(),
                    base_commit: "1".repeat(40),
                    imported_at: 1,
                },
            )
            .unwrap();
        assert!(destination.session_archived(&imported.session_id).unwrap());
        assert!(destination
            .session_handoff_blocked(&imported.session_id)
            .unwrap());
        assert!(destination.unarchive_session(&imported.session_id).is_err());
        destination
            .activate_destination_handoff(&imported.session_id, &capsule)
            .unwrap();
        assert!(!destination.session_archived(&imported.session_id).unwrap());
        assert!(!destination
            .session_handoff_blocked(&imported.session_id)
            .unwrap());
    }
}
