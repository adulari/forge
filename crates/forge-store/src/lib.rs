//! Local SQLite persistence (ADR-0005), via rusqlite with bundled SQLite. The store owns
//! a single connection behind a mutex; SQLite is in WAL mode for crash-resilient writes.
//! All persistence in Forge goes through this crate.

use std::path::Path;

use chrono::{DateTime, Datelike, Duration as ChronoDuration, Local, TimeZone};
use forge_types::{Role, TaskTier, ToolCall, Usage, Visibility};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior};

mod memory;
mod schema;

pub use memory::Memory;

/// Current schema version this build understands. Bumped whenever a new entry is added to
/// [`MIGRATIONS`]; persisted in the DB via `PRAGMA user_version`. A DB whose `user_version`
/// exceeds this (written by a NEWER Forge) is refused, rather than silently misread.
const SCHEMA_VERSION: i64 = 13;

/// Max attempts a critical write makes when SQLite reports the database is busy/locked. The single
/// WAL writer lock can be briefly held by another connection (TUI vs mcp-serve, or the indexer);
/// `busy_timeout` covers ordinary lock waits but NOT `SQLITE_BUSY_SNAPSHOT`, so we retry the whole
/// transaction a bounded number of times with a short backoff rather than dropping the row.
const BUSY_RETRY_MAX: u32 = 8;

/// Oversized `tool_call.args_json`/`result_json` (e.g. a full large-file read or write) is truncated
/// to this many bytes at insert time, with a marker. Keeps the append-only global DB from growing
/// without bound while preserving the head of the args/output for audit/replay.
const MAX_RESULT_JSON_BYTES: usize = 64 * 1024;

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

/// Process-wide active per-model completion reservations. Stores are opened independently by
/// daemon sessions, so this registry must not live on a single [`Store`] instance.
fn in_flight_models() -> &'static std::sync::Mutex<std::collections::HashSet<String>> {
    static MODELS: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<String>>> =
        std::sync::OnceLock::new();
    MODELS.get_or_init(|| std::sync::Mutex::new(std::collections::HashSet::new()))
}

/// An active per-model completion reservation. Dropping it makes the model eligible for another
/// concurrent session.
pub struct ModelReservation {
    model: String,
}

impl Drop for ModelReservation {
    fn drop(&mut self) {
        if let Ok(mut in_flight) = in_flight_models().lock() {
            in_flight.remove(&self.model);
        }
    }
}

pub struct Store {
    pool: r2d2::Pool<SqliteManager>,
    /// Append counter for `live_event`, so the ring-buffer prune runs once every
    /// [`LIVE_EVENT_PRUNE_EVERY`] inserts instead of on every append (the old per-insert
    /// correlated-subquery DELETE was O(n) on a hot path).
    live_event_writes: std::sync::atomic::AtomicU64,
}

/// SQL fragment: derives a usage row's provider from its message model (aliased `m`).
///
/// Ordinary completions retain their routed model. Synthetic side-call messages (compact,
/// diagnose) have no model, so inherit the nearest routed model in their session instead of
/// appearing in a misleading shared `other` bucket. A session containing no routed model at all
/// still falls back to `other`.
const USAGE_PROVIDER_EXPR: &str = "COALESCE(NULLIF(CASE WHEN instr(m.model, '::') > 0 THEN substr(m.model, 1, instr(m.model, '::') - 1) ELSE m.model END, ''), (SELECT CASE WHEN instr(pm.model, '::') > 0 THEN substr(pm.model, 1, instr(pm.model, '::') - 1) ELSE pm.model END FROM message pm WHERE pm.session_id = m.session_id AND pm.model IS NOT NULL ORDER BY ABS(pm.seq - m.seq), pm.seq DESC LIMIT 1), 'other')";

#[derive(Debug, Clone, PartialEq)]
pub struct ProviderUsage {
    pub provider: String,
    pub input_tokens: u64,
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
        Ok(conn)
    }

    fn is_valid(&self, conn: &mut Connection) -> std::result::Result<(), rusqlite::Error> {
        conn.execute_batch("SELECT 1")
    }

    fn has_broken(&self, _conn: &mut Connection) -> bool {
        false
    }
}

/// Migrate `subscription_usage` from its old single-column PK to the composite
/// `(provider, window_kind)` PK. Safe to call on any DB version: a no-op when the table
/// doesn't exist yet (schema will create it correctly) or already has the composite key.
fn migrate_subscription_usage(conn: &Connection) -> rusqlite::Result<()> {
    let exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='subscription_usage'",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    if exists == 0 {
        return Ok(()); // table not yet created; schema will handle it
    }
    let pk_cols: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('subscription_usage') WHERE pk > 0",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    if pk_cols >= 2 {
        return Ok(()); // already on composite PK
    }
    // Old single-column PK — recreate with composite key.
    // subscription_usage is a transient cache; data loss on migration is acceptable.
    conn.execute_batch(
        "DROP TABLE IF EXISTS subscription_usage_new;
         CREATE TABLE subscription_usage_new (
             provider    TEXT NOT NULL,
             window_kind TEXT NOT NULL,
             status      TEXT NOT NULL,
             resets_at   INTEGER,
             fraction    REAL,
             updated_at  INTEGER NOT NULL DEFAULT (strftime('%s','now')),
             PRIMARY KEY (provider, window_kind)
         );
         DROP TABLE subscription_usage;
         ALTER TABLE subscription_usage_new RENAME TO subscription_usage;",
    )
}

/// Run `ALTER TABLE ADD COLUMN`, treating an "already present" error as success but surfacing any
/// OTHER failure. Replaces the old `let _ = conn.execute(...)` that swallowed every error, so a
/// genuine migration failure is no longer indistinguishable from "column already exists".
fn add_column_if_missing(conn: &Connection, sql: &str) -> rusqlite::Result<()> {
    match conn.execute(sql, []) {
        Ok(_) => Ok(()),
        Err(rusqlite::Error::SqliteFailure(_, Some(ref msg)))
            if msg.contains("duplicate column name") =>
        {
            Ok(())
        }
        Err(e) => Err(e),
    }
}

/// Migration #1: fold the historic ad-hoc `ADD COLUMN` migrations into the versioned runner and add
/// the `UNIQUE(session_id, seq)` index that makes seq allocation collision-proof. On an existing DB
/// these ALTERs are idempotent (the columns usually exist); on a fresh DB `schema::SCHEMA` already
/// created them so the ALTERs no-op. Pre-existing duplicate `(session_id, seq)` rows (from the old
/// non-atomic seq race) are repaired before the unique index is built so the migration can't fail.
fn migration_0001(conn: &Connection) -> rusqlite::Result<()> {
    for stmt in [
        "ALTER TABLE message ADD COLUMN tool_calls_json TEXT",
        "ALTER TABLE message ADD COLUMN tool_call_id TEXT",
        "ALTER TABLE message ADD COLUMN active INTEGER NOT NULL DEFAULT 1",
        "ALTER TABLE session ADD COLUMN parent_session_id TEXT",
        "ALTER TABLE session ADD COLUMN view_snapshot TEXT",
        "ALTER TABLE lattice_node ADD COLUMN pagerank REAL NOT NULL DEFAULT 0.0",
        "ALTER TABLE session ADD COLUMN agent_active INTEGER NOT NULL DEFAULT 0",
    ] {
        add_column_if_missing(conn, stmt)?;
    }
    // These depend on the `active` column the ALTER above adds, so they live here (not in the base
    // schema batch, which can't add columns to a pre-existing message table).
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_message_session_active ON message(session_id, active, seq)",
    )?;
    repair_duplicate_seqs(conn)?;
    conn.execute_batch(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_message_session_seq_unique \
         ON message(session_id, seq)",
    )
}

/// Reassign fresh per-session `seq` values to any duplicate `(session_id, seq)` rows left by the old
/// non-atomic allocator, keeping the earliest row (lowest rowid) at its original seq. Runs before
/// the unique index so building it can't fail on a legacy DB. A no-op when there are no duplicates.
fn repair_duplicate_seqs(conn: &Connection) -> rusqlite::Result<()> {
    let dups: Vec<(String, String)> = {
        let mut stmt = conn.prepare(
            "SELECT id, session_id FROM message WHERE rowid NOT IN (
                 SELECT MIN(rowid) FROM message GROUP BY session_id, seq
             ) ORDER BY session_id, seq, rowid",
        )?;
        let v = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        v
    };
    for (id, session_id) in dups {
        let next: i64 = conn.query_row(
            "SELECT COALESCE(MAX(seq), -1) + 1 FROM message WHERE session_id = ?1",
            [&session_id],
            |r| r.get(0),
        )?;
        conn.execute("UPDATE message SET seq = ?1 WHERE id = ?2", (next, &id))?;
    }
    Ok(())
}

/// Migration #2: add `tool_call.path` (`forge blame`, docs/features/forge-blame.md) so a
/// write/edit tool call can be traced back to the file it touched without re-parsing
/// `args_json` at query time. Backfills existing `write_file`/`edit_file` rows best-effort —
/// a row whose `args_json` was truncated (see `MAX_RESULT_JSON_BYTES`) before reaching the
/// `path` key, or that fails to parse, is left NULL rather than erroring the migration.
fn migration_0002(conn: &Connection) -> rusqlite::Result<()> {
    add_column_if_missing(conn, "ALTER TABLE tool_call ADD COLUMN path TEXT")?;
    conn.execute_batch("CREATE INDEX IF NOT EXISTS idx_tool_call_path ON tool_call(path)")?;

    let rows: Vec<(String, String)> = {
        let mut stmt = conn.prepare(
            "SELECT id, args_json FROM tool_call \
             WHERE tool_name IN ('write_file', 'edit_file') AND path IS NULL",
        )?;
        let v = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        v
    };
    for (id, args_json) in rows {
        if let Some(path) = extract_path_arg(&args_json) {
            conn.execute("UPDATE tool_call SET path = ?1 WHERE id = ?2", (path, id))?;
        }
    }
    Ok(())
}

/// Migration #3: `/duel` outcome history (docs/features/duel.md): one row per candidate in
/// every duel run, per repo. `duel_boosts` aggregates wins-minus-losses per model, per repo,
/// into the soft routing boost `HeuristicRouter::with_repo_boosts` consumes.
fn migration_0003(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS duel_outcome (
            id TEXT PRIMARY KEY,
            repo_key TEXT NOT NULL,
            model TEXT NOT NULL,
            won INTEGER NOT NULL,
            task TEXT NOT NULL,
            created_at INTEGER NOT NULL DEFAULT (strftime('%s','now'))
         );
         CREATE INDEX IF NOT EXISTS idx_duel_outcome_repo ON duel_outcome(repo_key)",
    )
}

/// Migration #4: `forge schedule` registry — recurring OS-timer-driven `forge run` tasks
/// (feature: forge-schedule). Local machine state (deliberately NOT in
/// [`PORTABLE_METADATA_TABLES`] — a `cwd`/OS-timer install doesn't travel with `forge migrate`).
fn migration_0004(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schedule (
            id         TEXT PRIMARY KEY,
            task       TEXT NOT NULL,
            cwd        TEXT NOT NULL,
            mode       TEXT,
            model      TEXT,
            cron       TEXT NOT NULL,
            enabled    INTEGER NOT NULL DEFAULT 1,
            created_at INTEGER NOT NULL DEFAULT (strftime('%s','now')),
            last_run   INTEGER
         )",
    )
}

/// Migration #5: `forge queue` — the overnight-autopilot task queue (feature: queue-autopilot).
/// Each row is one queued headless task; a drain (`forge queue run`) executes them in isolated
/// worktrees and records the outcome (branch, cost, summary) back onto the row. Local machine
/// state like `schedule` (cwd + branches don't travel), so NOT in [`PORTABLE_METADATA_TABLES`].
fn migration_0005(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS queue_task (
            id          TEXT PRIMARY KEY,
            task        TEXT NOT NULL,
            cwd         TEXT NOT NULL,
            mode        TEXT,
            model       TEXT,
            budget_usd  REAL,
            status      TEXT NOT NULL DEFAULT 'pending',
            created_at  INTEGER NOT NULL DEFAULT (strftime('%s','now')),
            started_at  INTEGER,
            finished_at INTEGER,
            session_id  TEXT,
            branch      TEXT,
            summary     TEXT,
            cost_usd    REAL,
            gate        TEXT
         );
         CREATE INDEX IF NOT EXISTS idx_queue_task_status ON queue_task(status)",
    )
}

/// Migration #6: counterfactual forks (`forge fork` / `forge tree`) — a session can branch off
/// another at a turn boundary. `forked_from` points at the source session, `forked_at_seq` is
/// the message seq the copied prefix stops BEFORE (the re-asked prompt's original seq).
fn migration_0006(conn: &Connection) -> rusqlite::Result<()> {
    // Idempotent: on a fresh DB the base schema already carries these columns.
    add_column_if_missing(conn, "ALTER TABLE session ADD COLUMN forked_from TEXT")?;
    add_column_if_missing(conn, "ALTER TABLE session ADD COLUMN forked_at_seq INTEGER")
}

/// Migration #7: two-phase context pipeline — a message carries who it's for. `'llm'` (default)
/// rows are sent to the model; `'ui'` rows are user-facing notes the context pipeline strips
/// before every provider call (and after a resume).
fn migration_0007(conn: &Connection) -> rusqlite::Result<()> {
    // Idempotent: on a fresh DB the base schema already carries this column.
    add_column_if_missing(
        conn,
        "ALTER TABLE message ADD COLUMN visibility TEXT NOT NULL DEFAULT 'llm'",
    )
}

/// Migration #8: the `forge serve` multi-session daemon (docs/features/remote-control.md).
/// `session.worktree_path` records the isolated worktree a daemon session runs in;
/// `session.archived` hides a session from lists without deleting its history. The
/// `push_subscription` table is pre-added for actionable web push (Phase 5) so enabling it
/// later needs no migration. (`session.title` already exists — base schema.)
fn migration_0008(conn: &Connection) -> rusqlite::Result<()> {
    // Idempotent: on a fresh DB the base schema already carries these columns + table.
    add_column_if_missing(conn, "ALTER TABLE session ADD COLUMN worktree_path TEXT")?;
    add_column_if_missing(
        conn,
        "ALTER TABLE session ADD COLUMN archived INTEGER NOT NULL DEFAULT 0",
    )?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS push_subscription (
            id         TEXT PRIMARY KEY,
            endpoint   TEXT NOT NULL,
            p256dh     TEXT NOT NULL,
            auth       TEXT NOT NULL,
            created_at INTEGER NOT NULL DEFAULT (strftime('%s','now'))
         )",
    )
}

/// Migration #9: append-only quota usage history (mesh-routing.md, extends L3 in
/// docs/features/mesh-routing.md). `subscription_usage` only ever holds the LATEST
/// snapshot per (provider, window) — there's no way to derive a rate of consumption from it. This
/// table keeps every observation so [`forge_types::compute_quota_pace`] can project where a
/// window is headed. Deliberately NOT touching `subscription_usage`'s schema or upsert behavior —
/// the mesh router depends on that table staying "latest row per provider/window".
fn migration_0009(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS quota_history (
            id            INTEGER PRIMARY KEY AUTOINCREMENT,
            provider      TEXT NOT NULL,
            window_kind   TEXT NOT NULL,
            fraction_used REAL NOT NULL,
            resets_at     INTEGER,
            observed_at   INTEGER NOT NULL DEFAULT (strftime('%s','now'))
         );
         CREATE INDEX IF NOT EXISTS idx_quota_history_lookup
             ON quota_history(provider, window_kind, observed_at)",
    )
}

/// Migration #10: native (APNs) push subscriptions, alongside the existing Web Push table —
/// iOS/Android have no `PushManager`, so a device token + environment ("sandbox" vs
/// "production", since Apple routes each to a different host and a token from one is rejected by
/// the other) is the native equivalent of a `push_subscription` row.
fn migration_0010(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS apns_subscription (
            id           TEXT PRIMARY KEY,
            device_token TEXT NOT NULL,
            environment  TEXT NOT NULL,
            created_at   INTEGER NOT NULL DEFAULT (strftime('%s','now'))
         )",
    )
}

/// Migration #11: Live Activity remote-update push tokens (ActivityKit). A Live Activity has
/// its own push token, separate from the device's general APNs token (`apns_subscription`) —
/// Apple issues a fresh one per activity instance via `Activity.pushTokenUpdates`. At most one
/// active Live Activity token per session, so this is keyed by `session_id`, not an id+list.
fn migration_0011(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS live_activity_token (
            session_id   TEXT PRIMARY KEY,
            push_token   TEXT NOT NULL,
            environment  TEXT NOT NULL,
            updated_at   INTEGER NOT NULL DEFAULT (strftime('%s','now'))
         )",
    )
}

/// Migration #12: distinguish compaction soft-deletes from `/undo` soft-deletes. Both set
/// `message.active = 0`, so the old "reactivate every inactive row" uncompact resurrected rows
/// `/undo` had removed. `compacted = 1` marks the rows a `/compact` deactivated; uncompact now
/// reactivates only those, leaving `/undo` rows untouched.
fn migration_0012(conn: &Connection) -> rusqlite::Result<()> {
    // Idempotent: on a fresh DB the base schema already carries this column.
    add_column_if_missing(
        conn,
        "ALTER TABLE message ADD COLUMN compacted INTEGER NOT NULL DEFAULT 0",
    )
}

/// Migration #13: enforce one `push_subscription` row per `endpoint`. `upsert_push_subscription`
/// deduped application-side with a non-atomic SELECT-then-INSERT, so concurrent callers could
/// still pile up duplicate rows. De-dupe any existing duplicates (keep the earliest rowid) before
/// building the UNIQUE index the upsert's `ON CONFLICT(endpoint)` now resolves against. Kept out
/// of the base schema (like `idx_message_session_seq_unique`) because schema runs before this
/// de-dupe, so a legacy DB with duplicates would fail to build the index there.
fn migration_0013(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute(
        "DELETE FROM push_subscription WHERE rowid NOT IN (
             SELECT MIN(rowid) FROM push_subscription GROUP BY endpoint
         )",
        [],
    )?;
    conn.execute_batch(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_push_subscription_endpoint \
         ON push_subscription(endpoint)",
    )
}

/// Ordered migration steps. Index `i` upgrades the DB from `user_version = i` to `i + 1`. Append
/// new steps here and bump [`SCHEMA_VERSION`]; never reorder or rewrite an already-shipped step.
const MIGRATIONS: &[fn(&Connection) -> rusqlite::Result<()>] = &[
    migration_0001,
    migration_0002,
    migration_0003,
    migration_0004,
    migration_0005,
    migration_0006,
    migration_0007,
    migration_0008,
    migration_0009,
    migration_0010,
    migration_0011,
    migration_0012,
    migration_0013,
];

/// Apply every migration the DB hasn't seen yet, bumping `PRAGMA user_version` after each so a
/// crash mid-run resumes cleanly. Refuses (with [`StoreError::SchemaTooNew`]) to open a DB written
/// by a newer build, rather than silently misreading it.
fn run_migrations(conn: &Connection) -> Result<()> {
    debug_assert_eq!(MIGRATIONS.len() as i64, SCHEMA_VERSION);
    let current: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if current > SCHEMA_VERSION {
        return Err(StoreError::SchemaTooNew {
            found: current,
            supported: SCHEMA_VERSION,
        });
    }
    for (v, migrate) in MIGRATIONS.iter().enumerate().skip(current as usize) {
        migrate(conn)?;
        conn.pragma_update(None, "user_version", (v + 1) as i64)?;
    }
    Ok(())
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

fn quota_status_from_str(status: &str) -> forge_types::QuotaStatus {
    match status {
        "exhausted" => forge_types::QuotaStatus::Exhausted,
        "warning" => forge_types::QuotaStatus::Warning,
        _ => forge_types::QuotaStatus::Ok,
    }
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
            // Migrate before schema so old DBs get the composite PK before CREATE TABLE IF NOT EXISTS no-ops.
            migrate_subscription_usage(&conn)?;
            conn.execute_batch(schema::SCHEMA)?;
            // Versioned migrations (PRAGMA user_version). Folds the historic ad-hoc ADD COLUMN
            // migrations and the UNIQUE(session_id, seq) index; refuses a DB from a newer build.
            run_migrations(&conn)?;
        }
        Ok(Self {
            pool,
            live_event_writes: std::sync::atomic::AtomicU64::new(0),
        })
    }

    /// Atomically reserve a model for an active completion. The returned guard releases the
    /// reservation on every normal return, error, and cancellation path.
    pub fn try_reserve_model(&self, model: &str) -> Option<ModelReservation> {
        let mut in_flight = in_flight_models().lock().ok()?;
        if !in_flight.insert(model.to_string()) {
            return None;
        }
        Some(ModelReservation {
            model: model.to_string(),
        })
    }

    /// Check whether a model has an active completion reservation.
    pub fn is_model_reserved(&self, model: &str) -> bool {
        in_flight_models()
            .lock()
            .is_ok_and(|in_flight| in_flight.contains(model))
    }

    /// Check out a pooled connection. Named `lock` for continuity with the call sites; the returned
    /// `PooledConnection` derefs to `rusqlite::Connection` and returns itself to the pool on drop.
    fn lock(&self) -> Result<r2d2::PooledConnection<SqliteManager>> {
        self.pool.get().map_err(|e| StoreError::Pool(e.to_string()))
    }

    /// Export machine-agnostic model metadata (health cooldowns, context windows, pricing) as JSON,
    /// for `forge migrate`. ONLY the allow-listed [`PORTABLE_METADATA_TABLES`] are dumped — it
    /// contains NO session, message, usage, or routing data, so it is safe to put in a bundle that
    /// deliberately excludes history. Column order is preserved so the import is schema-faithful.
    pub fn export_portable_metadata(&self) -> Result<String> {
        let conn = self.lock()?;
        let mut out = serde_json::Map::new();
        for table in PORTABLE_METADATA_TABLES {
            let mut stmt = conn.prepare(&format!("SELECT * FROM {table}"))?;
            let cols: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();
            let ncol = cols.len();
            let rows = stmt
                .query_map([], |row| {
                    let mut vals = Vec::with_capacity(ncol);
                    for i in 0..ncol {
                        vals.push(value_ref_to_json(row.get_ref(i)?));
                    }
                    Ok(serde_json::Value::Array(vals))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            out.insert(
                (*table).to_string(),
                serde_json::json!({ "columns": cols, "rows": rows }),
            );
        }
        Ok(serde_json::Value::Object(out).to_string())
    }

    /// Import metadata produced by [`export_portable_metadata`], upserting (`INSERT OR REPLACE`).
    /// Only the allow-listed portable tables are touched; any other key in the JSON is ignored, so
    /// a tampered bundle cannot write arbitrary tables. Returns the number of rows written.
    pub fn import_portable_metadata(&self, json: &str) -> Result<usize> {
        let parsed: serde_json::Value =
            serde_json::from_str(json).map_err(|e| StoreError::Json(e.to_string()))?;
        let conn = self.lock()?;
        let mut written = 0usize;
        for table in PORTABLE_METADATA_TABLES {
            let Some(t) = parsed.get(*table) else {
                continue;
            };
            let (Some(cols), Some(rows)) = (
                t.get("columns").and_then(|c| c.as_array()),
                t.get("rows").and_then(|r| r.as_array()),
            ) else {
                continue;
            };
            let col_names: Vec<&str> = cols.iter().filter_map(|c| c.as_str()).collect();
            if col_names.is_empty() {
                continue;
            }
            // The `table` is allow-listed, but `col_names` come straight from the (untrusted)
            // migrate-bundle JSON and are `format!`-interpolated into the INSERT below. A tampered
            // bundle could inject SQL via a crafted column name (e.g. `x); DROP TABLE message;--`).
            // Validate every incoming column against the table's REAL schema (`pragma_table_info`)
            // so the interpolated identifiers are provably members of the table — reject otherwise.
            let valid_cols: std::collections::HashSet<String> = {
                let mut info = conn.prepare(&format!("PRAGMA table_info({table})"))?;
                let cols = info
                    .query_map([], |row| row.get::<_, String>(1))?
                    .collect::<rusqlite::Result<std::collections::HashSet<String>>>()?;
                cols
            };
            if let Some(bad) = col_names.iter().find(|c| !valid_cols.contains(**c)) {
                return Err(StoreError::Json(format!(
                    "portable metadata for `{table}` names unknown column `{bad}` (rejected)"
                )));
            }
            let placeholders = vec!["?"; col_names.len()].join(",");
            let sql = format!(
                "INSERT OR REPLACE INTO {table} ({}) VALUES ({placeholders})",
                col_names.join(",")
            );
            let mut stmt = conn.prepare(&sql)?;
            for row in rows {
                let Some(arr) = row.as_array() else { continue };
                if arr.len() != col_names.len() {
                    continue;
                }
                let params: Vec<rusqlite::types::Value> =
                    arr.iter().map(json_to_sql_value).collect();
                stmt.execute(rusqlite::params_from_iter(params.iter()))?;
                written += 1;
            }
        }
        Ok(written)
    }

    /// Create a new session row and return its id.
    pub fn create_session(&self, cwd: &str, mode: &str) -> Result<String> {
        let id = forge_types::new_id();
        self.lock()?.execute(
            "INSERT INTO session (id, cwd, permission_mode, total_cost_usd) VALUES (?1, ?2, ?3, 0)",
            (&id, cwd, mode),
        )?;
        // Opportunistic, bounded retention sweep so the global append-only DB doesn't grow forever.
        // Best-effort: a prune failure must never block opening a session.
        let _ = self.prune(RETENTION_HORIZON_SECS, PRUNE_BATCH);
        let _ = self.prune_empty(EMPTY_SESSION_HORIZON_SECS, EMPTY_PRUNE_BATCH);
        Ok(id)
    }

    /// The working directory recorded for a session at creation time, or `None` if no such
    /// session exists. Unlike `SessionRegistry::get` (the daemon's in-memory map of currently
    /// running drivers), this reads straight from the store — like [`Store::load_history_page`]
    /// it works for ANY persisted session, live or not, so a historical image can still be served
    /// after a daemon restart or once the session's driver has wound down.
    pub fn session_cwd(&self, id: &str) -> Result<Option<String>> {
        let conn = self.lock()?;
        match conn.query_row("SELECT cwd FROM session WHERE id = ?1", [id], |r| r.get(0)) {
            Ok(cwd) => Ok(Some(cwd)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Delete up to `max_sessions` sessions that have NEVER received a real (role='user') message —
    /// checked regardless of `active`, so a session whose sole user message was later soft-deleted
    /// by `/undo` or a checkpoint restore (which only flips `active`, it never removes the row) is
    /// still correctly recognized as having been used — and were created more than `horizon_secs`
    /// ago (oldest first) — separate from [`Store::prune`]'s much longer general retention horizon,
    /// since an empty session carries nothing worth keeping at all. A process that spawns a session
    /// and exits (or crashes) before the user ever sends a prompt — e.g. an `mcp agent` connection
    /// that's opened and torn down without being used — otherwise leaves a permanent, empty row that
    /// clutters `forge sessions` / the resume picker forever. Returns the number removed.
    pub fn prune_empty(&self, horizon_secs: i64, max_sessions: usize) -> Result<usize> {
        if max_sessions == 0 {
            return Ok(0);
        }
        let cutoff = chrono::Utc::now().timestamp() - horizon_secs;
        with_busy_retry(|| {
            let conn = self.lock()?;
            let ids: Vec<String> = {
                let mut stmt = conn.prepare(
                    "SELECT id FROM session s WHERE s.created_at < ?1 AND s.agent_active = 0 \
                     AND NOT EXISTS ( \
                       SELECT 1 FROM message m \
                       WHERE m.session_id = s.id AND m.role = 'user' \
                     ) ORDER BY s.created_at LIMIT ?2",
                )?;
                let v = stmt
                    .query_map((cutoff, max_sessions as i64), |r| r.get::<_, String>(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                v
            };
            for id in &ids {
                // ON DELETE CASCADE clears the dependent rows (messages → usage/routing/tool_call).
                conn.execute("DELETE FROM session WHERE id = ?1", [id])?;
            }
            Ok(ids.len())
        })
    }

    /// Delete up to `max_sessions` sessions whose `updated_at` is older than `horizon_secs` ago
    /// (oldest first), cascading to their messages/usage/routing/tool_calls/live_events/tasks. The
    /// retention unit is a whole stale session, never individual rows of a live one, so an active
    /// transcript is never partially pruned. Returns the number of sessions removed.
    pub fn prune(&self, horizon_secs: i64, max_sessions: usize) -> Result<usize> {
        if max_sessions == 0 {
            return Ok(0);
        }
        let cutoff = chrono::Utc::now().timestamp() - horizon_secs;
        with_busy_retry(|| {
            let conn = self.lock()?;
            let ids: Vec<String> = {
                let mut stmt = conn.prepare(
                    "SELECT id FROM session WHERE updated_at < ?1 AND agent_active = 0 \
                     ORDER BY updated_at LIMIT ?2",
                )?;
                let v = stmt
                    .query_map((cutoff, max_sessions as i64), |r| r.get::<_, String>(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                v
            };
            for id in &ids {
                // ON DELETE CASCADE clears the dependent rows (messages → usage/routing/tool_call).
                conn.execute("DELETE FROM session WHERE id = ?1", [id])?;
            }
            Ok(ids.len())
        })
    }

    /// Reclaim free pages and checkpoint the WAL — the periodic compaction a host can call (e.g. on
    /// a timer or at shutdown) after retention pruning. `VACUUM` rebuilds the file; the truncating
    /// checkpoint then shrinks the WAL. Both are safe in WAL mode with no open write transaction.
    pub fn vacuum(&self) -> Result<()> {
        let conn = self.lock()?;
        conn.execute_batch("VACUUM")?;
        // Truncating WAL checkpoint to shrink the -wal file (no-op / harmless on in-memory DBs).
        let _ = conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()));
        Ok(())
    }

    /// Create a subagent child session linked to `parent_id` (RFC subagent-orchestration).
    pub fn create_child_session(&self, cwd: &str, mode: &str, parent_id: &str) -> Result<String> {
        let id = forge_types::new_id();
        self.lock()?.execute(
            "INSERT INTO session (id, cwd, permission_mode, total_cost_usd, parent_session_id) \
             VALUES (?1, ?2, ?3, 0, ?4)",
            (&id, cwd, mode, parent_id),
        )?;
        Ok(id)
    }

    /// A session's stored permission mode (temper) string.
    pub fn session_mode(&self, session_id: &str) -> Result<String> {
        Ok(self.lock()?.query_row(
            "SELECT permission_mode FROM session WHERE id = ?1",
            [session_id],
            |row| row.get(0),
        )?)
    }

    /// Update a session's permission mode (temper) — persisted when the user switches it live.
    pub fn update_session_mode(&self, session_id: &str, mode: &str) -> Result<()> {
        self.lock()?.execute(
            "UPDATE session SET permission_mode = ?2, updated_at = strftime('%s','now') WHERE id = ?1",
            (session_id, mode),
        )?;
        Ok(())
    }

    /// A session's persisted TUI view snapshot (opaque JSON), if one was saved. Used to restore the
    /// exact on-screen state (activity panel, viewer, scroll) when the session is resumed.
    pub fn session_view_snapshot(&self, session_id: &str) -> Result<Option<String>> {
        Ok(self.lock()?.query_row(
            "SELECT view_snapshot FROM session WHERE id = ?1",
            [session_id],
            |row| row.get(0),
        )?)
    }

    /// Persist a session's TUI view snapshot (opaque JSON). Written at the end of each completed
    /// turn and on clean exit so a resume restores the screen as of the last prompt.
    pub fn update_session_view_snapshot(&self, session_id: &str, json: &str) -> Result<()> {
        self.lock()?.execute(
            "UPDATE session SET view_snapshot = ?2, updated_at = strftime('%s','now') WHERE id = ?1",
            (session_id, json),
        )?;
        Ok(())
    }

    /// Ids of the subagent child sessions spawned by `parent_id`, oldest first.
    pub fn child_sessions(&self, parent_id: &str) -> Result<Vec<String>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT id FROM session WHERE parent_session_id = ?1 ORDER BY created_at, id",
        )?;
        let rows = stmt.query_map([parent_id], |r| r.get::<_, String>(0))?;
        Ok(rows.filter_map(std::result::Result::ok).collect())
    }

    /// Name a session — for a subagent child, the resolved agent name (`title` doubles as the
    /// child's address for `send_to_agent`; top-level sessions title themselves elsewhere).
    pub fn set_session_title(&self, session_id: &str, title: &str) -> Result<()> {
        self.lock()?.execute(
            "UPDATE session SET title = ?2 WHERE id = ?1",
            (session_id, title),
        )?;
        Ok(())
    }

    /// `(id, title)` of `parent_id`'s child sessions, oldest first — the address book
    /// `send_to_agent` resolves against (title = the agent name recorded at spawn).
    pub fn named_child_sessions(&self, parent_id: &str) -> Result<Vec<(String, Option<String>)>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT id, title FROM session WHERE parent_session_id = ?1 ORDER BY created_at, id",
        )?;
        let rows = stmt.query_map([parent_id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(StoreError::from)
    }

    /// The next free message `seq` for a session (0 for an empty one) — lets a follow-up turn
    /// append to a child session without replaying its whole insert history.
    pub fn next_message_seq(&self, session_id: &str) -> Result<i64> {
        Ok(self.lock()?.query_row(
            "SELECT COALESCE(MAX(seq) + 1, 0) FROM message WHERE session_id = ?1",
            [session_id],
            |row| row.get(0),
        )?)
    }

    /// Append a message to a session and return its id.
    pub fn add_message(
        &self,
        session_id: &str,
        seq: i64,
        role: Role,
        content: &str,
        model: Option<&str>,
    ) -> Result<String> {
        self.add_message_full(session_id, seq, role, content, model, &[], None)
    }

    /// Append a UI-only note: persisted (so it survives resume and shows in replay/scrollback)
    /// but tagged `visibility='ui'` so the context pipeline never sends it to a model.
    pub fn add_ui_note(
        &self,
        session_id: &str,
        seq: i64,
        role: Role,
        content: &str,
    ) -> Result<String> {
        self.insert_message(
            session_id,
            seq,
            role,
            content,
            None,
            &[],
            None,
            Visibility::UiOnly,
        )
    }

    /// Append a message, including any tool-call linkage (assistant tool calls / tool
    /// result ids), so the transcript round-trips faithfully on resume.
    #[allow(clippy::too_many_arguments)]
    pub fn add_message_full(
        &self,
        session_id: &str,
        seq: i64,
        role: Role,
        content: &str,
        model: Option<&str>,
        tool_calls: &[ToolCall],
        tool_call_id: Option<&str>,
    ) -> Result<String> {
        self.insert_message(
            session_id,
            seq,
            role,
            content,
            model,
            tool_calls,
            tool_call_id,
            Visibility::Llm,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn insert_message(
        &self,
        session_id: &str,
        seq: i64,
        role: Role,
        content: &str,
        model: Option<&str>,
        tool_calls: &[ToolCall],
        tool_call_id: Option<&str>,
        visibility: Visibility,
    ) -> Result<String> {
        let id = forge_types::new_id();
        let tool_calls_json = if tool_calls.is_empty() {
            None
        } else {
            Some(serde_json::to_string(tool_calls).unwrap_or_default())
        };
        // IMMEDIATE so the write lock is taken up front (no read-snapshot upgrade), bounded-retried
        // on transient busy, and self-healing on a seq collision: if `seq` is already taken (two
        // writers raced on `next_seq_for_session`), the UNIQUE(session_id, seq) index rejects it and
        // we re-allocate MAX(seq)+1 inside the same transaction rather than scrambling order.
        with_busy_retry(|| {
            let mut conn = self.lock()?;
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let mut s = seq;
            loop {
                let r = tx.execute(
                    "INSERT INTO message (id, session_id, seq, role, content, model, tool_calls_json, tool_call_id, visibility)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                     ON CONFLICT(session_id, seq) DO NOTHING",
                    (&id, session_id, s, role.as_str(), content, model, &tool_calls_json, tool_call_id, visibility.as_str()),
                );
                match r {
                    Ok(1) => break,
                    Ok(0) => {
                        s = tx.query_row(
                            "SELECT COALESCE(MAX(seq), -1) + 1 FROM message WHERE session_id = ?1",
                            [session_id],
                            |row| row.get(0),
                        )?;
                    }
                    Ok(_) => unreachable!("INSERT can affect at most one row"),
                    Err(e) => return Err(StoreError::Sqlite(e)),
                }
            }
            tx.commit()?;
            Ok(())
        })?;
        Ok(id)
    }

    /// Record the Mesh's routing decision for a message.
    pub fn record_routing(
        &self,
        message_id: &str,
        tier: TaskTier,
        chosen_model: &str,
        rationale: &str,
    ) -> Result<()> {
        with_busy_retry(|| {
            let conn = self.lock()?;
            conn.prepare_cached(
                "INSERT INTO routing_decision (id, message_id, task_tier, chosen_model, rationale)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )?
            .execute((
                forge_types::new_id(),
                message_id,
                tier.as_str(),
                chosen_model,
                rationale,
            ))?;
            Ok(())
        })
    }

    /// Record token usage/cost for a message and bump the session's running total.
    /// Batched in one explicit transaction so the INSERT + UPDATE land in a single WAL commit.
    pub fn record_usage(&self, session_id: &str, message_id: &str, usage: &Usage) -> Result<()> {
        with_busy_retry(|| {
            let mut conn = self.lock()?;
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            tx.execute(
                "INSERT INTO usage (id, message_id, input_tokens, output_tokens, cost_usd)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                (
                    forge_types::new_id(),
                    message_id,
                    usage.input_tokens as i64,
                    usage.output_tokens as i64,
                    usage.cost_usd,
                ),
            )?;
            tx.execute(
                "UPDATE session SET total_cost_usd = total_cost_usd + ?1,
                 updated_at = strftime('%s','now') WHERE id = ?2",
                (usage.cost_usd, session_id),
            )?;
            tx.commit()?;
            Ok(())
        })
    }

    /// Record usage for a side call (compact, diagnose) that has no corresponding agent message.
    /// Inserts a synthetic inactive system message as the FK anchor, then the usage row, and
    /// bumps the session total so daily/monthly budget queries (which read `usage`) stay accurate.
    pub fn record_side_call_usage(
        &self,
        session_id: &str,
        label: &str,
        usage: &Usage,
    ) -> Result<()> {
        let msg_id = forge_types::new_id();
        // IMMEDIATE: this SELECTs MAX(seq) then writes. A DEFERRED txn would take a read snapshot
        // first and, if another connection committed in between, fail the upgrade with
        // SQLITE_BUSY_SNAPSHOT (which busy_timeout does NOT cover) — silently losing the usage/cost
        // row. Taking the write lock up front avoids the snapshot conflict entirely.
        with_busy_retry(|| {
            let mut conn = self.lock()?;
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let mut s: i64 = tx
                .query_row(
                    "SELECT COALESCE(MAX(seq), -1) + 1 FROM message WHERE session_id = ?1",
                    [session_id],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            loop {
                let r = tx.execute(
                    "INSERT INTO message (id, session_id, seq, role, content, active) \
                     VALUES (?1, ?2, ?3, 'system', ?4, 0)",
                    (msg_id.as_str(), session_id, s, label),
                );
                match r {
                    Ok(_) => break,
                    Err(ref e) if is_unique_violation(e) => s += 1,
                    Err(e) => return Err(StoreError::Sqlite(e)),
                }
            }
            tx.execute(
                "INSERT INTO usage (id, message_id, input_tokens, output_tokens, cost_usd) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                (
                    forge_types::new_id(),
                    msg_id.as_str(),
                    usage.input_tokens as i64,
                    usage.output_tokens as i64,
                    usage.cost_usd,
                ),
            )?;
            tx.execute(
                "UPDATE session SET total_cost_usd = total_cost_usd + ?1, \
                 updated_at = strftime('%s','now') WHERE id = ?2",
                (usage.cost_usd, session_id),
            )?;
            tx.commit()?;
            Ok(())
        })
    }

    /// Record a tool call and its permission outcome.
    pub fn record_tool_call(
        &self,
        message_id: &str,
        tool_name: &str,
        args_json: &str,
        result: &str,
        permission: &str,
        status: &str,
    ) -> Result<()> {
        // Extracted from the UNCAPPED args string, before the cap below can truncate the tail
        // and clip a late `path` key out of the JSON.
        let path = extract_path_arg(args_json);
        // Cap oversized args/results (full file writes/reads etc.) so the append-only global DB
        // can't grow without bound; the head is preserved with a truncation marker for audit/replay.
        let args_json = cap_result_json(args_json);
        let result = cap_result_json(result);
        with_busy_retry(|| {
            let conn = self.lock()?;
            conn.prepare_cached(
                "INSERT INTO tool_call (id, message_id, tool_name, args_json, result_json, permission, status, path)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            )?
            .execute((forge_types::new_id(), message_id, tool_name, args_json.as_ref(), result.as_ref(), permission, status, path.as_deref()))?;
            Ok(())
        })
    }

    /// Current running cost of a session (the per-session meter — unchanged).
    pub fn session_cost(&self, session_id: &str) -> Result<f64> {
        Ok(self.lock()?.query_row(
            "SELECT total_cost_usd FROM session WHERE id = ?1",
            [session_id],
            |row| row.get(0),
        )?)
    }

    /// `(input_tokens, output_tokens)` summed across a session's `usage` rows — the live token
    /// counter (tui-token-counter.md).
    pub fn session_tokens(&self, session_id: &str) -> Result<(u64, u64)> {
        let conn = self.lock()?;
        let (i, o): (i64, i64) = conn.query_row(
            "SELECT COALESCE(SUM(u.input_tokens), 0), COALESCE(SUM(u.output_tokens), 0)
             FROM usage u JOIN message m ON m.id = u.message_id
             WHERE m.session_id = ?1 AND m.active = 1",
            [session_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        Ok((i.max(0) as u64, o.max(0) as u64))
    }

    /// Number of provider calls (model steps) recorded in a session — one `usage` row per call.
    /// The Lattice benchmark uses this as the "steps" metric: fewer tool-exploration round-trips
    /// means fewer steps and fewer tokens.
    pub fn session_step_count(&self, session_id: &str) -> Result<u64> {
        let conn = self.lock()?;
        let n: i64 = conn.query_row(
            "SELECT COUNT(*) FROM usage u JOIN message m ON m.id = u.message_id
             WHERE m.session_id = ?1 AND m.active = 1",
            [session_id],
            |r| r.get(0),
        )?;
        Ok(n.max(0) as u64)
    }

    /// Models the Mesh routed to within a session (chosen_model per routing_decision), oldest
    /// first. Used to verify subagents route independently of the parent.
    pub fn session_models(&self, session_id: &str) -> Result<Vec<String>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT r.chosen_model FROM routing_decision r \
             JOIN message m ON m.id = r.message_id \
             WHERE m.session_id = ?1 ORDER BY m.seq",
        )?;
        let rows = stmt.query_map([session_id], |r| r.get::<_, String>(0))?;
        Ok(rows.filter_map(std::result::Result::ok).collect())
    }

    /// Per-provider usage since a rolling epoch timestamp.
    pub fn usage_by_provider_since(&self, since_epoch: i64) -> Result<Vec<ProviderUsage>> {
        self.usage_query("WHERE u.created_at >= ?1", [since_epoch])
    }

    pub fn usage_by_provider_for_session(&self, session_id: &str) -> Result<Vec<ProviderUsage>> {
        let conn = self.lock()?;
        // Provider derived from `message.model` (see `usage_query`) — `usage.provider` is NULL.
        let sql = format!(
            "SELECT {USAGE_PROVIDER_EXPR} AS prov, COALESCE(SUM(u.input_tokens), 0), COALESCE(SUM(u.output_tokens), 0), COALESCE(SUM(u.cost_usd), 0.0) \
             FROM usage u JOIN message m ON m.id = u.message_id WHERE m.session_id = ?1 GROUP BY prov \
             ORDER BY SUM(u.cost_usd) DESC, SUM(u.input_tokens + u.output_tokens) DESC"
        );
        let mut stmt = conn.prepare_cached(&sql)?;
        let rows = stmt.query_map([session_id], |r| {
            Ok(ProviderUsage {
                provider: r.get(0)?,
                input_tokens: r.get::<_, i64>(1)? as u64,
                output_tokens: r.get::<_, i64>(2)? as u64,
                cost_usd: r.get(3)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    fn usage_query<P: rusqlite::Params>(
        &self,
        predicate: &str,
        params: P,
    ) -> Result<Vec<ProviderUsage>> {
        let conn = self.lock()?;
        // Derive the provider from the routed model on the linked message: the `usage.provider`
        // column is never populated at insert, so grouping on it collapsed every row into one NULL
        // bucket (and `r.get::<String>` then failed on NULL → the whole page read as "no usage").
        // `message.model` IS populated (e.g. `codex-oauth::gpt-5.6-terra`); the namespace before
        // `::` is the provider. GROUP BY the alias `prov`, never a bare `provider` (that binds to
        // the still-NULL column, not this expression).
        let sql = format!("SELECT {USAGE_PROVIDER_EXPR} AS prov, COALESCE(SUM(u.input_tokens), 0), COALESCE(SUM(u.output_tokens), 0), COALESCE(SUM(u.cost_usd), 0.0) FROM usage u JOIN message m ON m.id = u.message_id {predicate} GROUP BY prov ORDER BY SUM(u.cost_usd) DESC, SUM(u.input_tokens + u.output_tokens) DESC");
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params, |r| {
            Ok(ProviderUsage {
                provider: r.get(0)?,
                input_tokens: r.get::<_, i64>(1)? as u64,
                output_tokens: r.get::<_, i64>(2)? as u64,
                cost_usd: r.get(3)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    pub fn subscription_windows(&self) -> Result<Vec<SubscriptionWindow>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare("SELECT provider, window_kind, status, resets_at, fraction FROM subscription_usage WHERE resets_at IS NULL OR resets_at > ?1")?;
        let rows = stmt.query_map([chrono::Utc::now().timestamp()], |r| {
            Ok(SubscriptionWindow {
                provider: r.get(0)?,
                window_kind: r.get(1)?,
                status: r.get(2)?,
                resets_at: r.get(3)?,
                fraction: r.get(4)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// This is the authoritative budget figure (FR-5): it aggregates `usage.cost_usd` across
    /// every session, not one session's running total.
    pub fn spend_between(&self, start: i64, end: i64) -> Result<f64> {
        Ok(self.lock()?.query_row(
            "SELECT COALESCE(SUM(cost_usd), 0.0) FROM usage \
             WHERE created_at >= ?1 AND created_at < ?2",
            (start, end),
            |row| row.get(0),
        )?)
    }

    /// Spend across all sessions in the current local calendar day.
    pub fn spend_today_usd(&self) -> Result<f64> {
        let (s, e) = day_bounds_local(chrono::Local::now());
        self.spend_between(s, e)
    }

    /// Spend across all sessions in the current local calendar month.
    pub fn spend_this_month_usd(&self) -> Result<f64> {
        let (s, e) = month_bounds_local(chrono::Local::now());
        self.spend_between(s, e)
    }

    /// Per-model spend + token counts for the current calendar day.
    /// Returns `Vec<(model, cost_usd, input_tokens, output_tokens)>`, sorted by cost desc.
    /// Rows where `message.model` is NULL (side calls like compact/diagnose) are grouped under
    /// the empty string.
    pub fn spend_by_model_today(&self) -> Result<Vec<(String, f64, u64, u64)>> {
        let (s, e) = day_bounds_local(chrono::Local::now());
        let conn = self.lock()?;
        let mut stmt = conn.prepare_cached(
            "SELECT COALESCE(m.model, '') as mdl,
                    COALESCE(SUM(u.cost_usd), 0.0),
                    COALESCE(SUM(u.input_tokens), 0),
                    COALESCE(SUM(u.output_tokens), 0)
             FROM usage u JOIN message m ON m.id = u.message_id
             WHERE u.created_at >= ?1 AND u.created_at < ?2
             GROUP BY mdl
             ORDER BY SUM(u.cost_usd) DESC, SUM(u.input_tokens) DESC",
        )?;
        let rows = stmt.query_map((s, e), |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, f64>(1)?,
                r.get::<_, i64>(2)? as u64,
                r.get::<_, i64>(3)? as u64,
            ))
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Spend in the last 5 hours (rolling, not calendar-day-aligned).
    pub fn spend_last_5h_usd(&self) -> Result<f64> {
        let (s, e) = rolling_hours_bounds(chrono::Local::now(), 5);
        self.spend_between(s, e)
    }

    /// Spend in the current local ISO calendar week (Monday 00:00 → now).
    pub fn spend_this_week_usd(&self) -> Result<f64> {
        let (s, e) = week_bounds_local(chrono::Local::now());
        self.spend_between(s, e)
    }

    /// Today / week / month spend in a single query — 3× cheaper than calling the three
    /// individual helpers. Uses conditional aggregation over the widest window (month) so
    /// only one table scan runs; the `created_at` index makes it sub-millisecond.
    /// Uses prepare_cached so the statement is compiled once per connection, not once per call.
    pub fn spend_summary_usd(&self) -> Result<(f64, f64, f64)> {
        let now = chrono::Local::now();
        let (day_s, day_e) = day_bounds_local(now);
        let (week_s, _) = week_bounds_local(now);
        let (month_s, month_e) = month_bounds_local(now);
        let conn = self.lock()?;
        let mut stmt = conn.prepare_cached(
            "SELECT
               COALESCE(SUM(CASE WHEN created_at >= ?1 AND created_at < ?2 THEN cost_usd ELSE 0 END), 0.0),
               COALESCE(SUM(CASE WHEN created_at >= ?3 THEN cost_usd ELSE 0 END), 0.0),
               COALESCE(SUM(cost_usd), 0.0)
             FROM usage
             WHERE created_at >= ?4 AND created_at < ?5",
        )?;
        Ok(
            stmt.query_row((day_s, day_e, week_s, month_s, month_e), |row| {
                Ok((
                    row.get::<_, f64>(0)?,
                    row.get::<_, f64>(1)?,
                    row.get::<_, f64>(2)?,
                ))
            })?,
        )
    }

    /// Per-model spend + token counts for the last 5 hours.
    pub fn spend_by_model_5h(&self) -> Result<Vec<(String, f64, u64, u64)>> {
        let (s, e) = rolling_hours_bounds(chrono::Local::now(), 5);
        let conn = self.lock()?;
        let mut stmt = conn.prepare_cached(
            "SELECT COALESCE(m.model, '') as mdl,
                    COALESCE(SUM(u.cost_usd), 0.0),
                    COALESCE(SUM(u.input_tokens), 0),
                    COALESCE(SUM(u.output_tokens), 0)
             FROM usage u JOIN message m ON m.id = u.message_id
             WHERE u.created_at >= ?1 AND u.created_at < ?2
             GROUP BY mdl
             ORDER BY SUM(u.cost_usd) DESC, SUM(u.input_tokens) DESC",
        )?;
        let rows = stmt.query_map((s, e), |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, f64>(1)?,
                r.get::<_, i64>(2)? as u64,
                r.get::<_, i64>(3)? as u64,
            ))
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Per-model spend + token counts for the current ISO week.
    pub fn spend_by_model_week(&self) -> Result<Vec<(String, f64, u64, u64)>> {
        let (s, e) = week_bounds_local(chrono::Local::now());
        let conn = self.lock()?;
        let mut stmt = conn.prepare_cached(
            "SELECT COALESCE(m.model, '') as mdl,
                    COALESCE(SUM(u.cost_usd), 0.0),
                    COALESCE(SUM(u.input_tokens), 0),
                    COALESCE(SUM(u.output_tokens), 0)
             FROM usage u JOIN message m ON m.id = u.message_id
             WHERE u.created_at >= ?1 AND u.created_at < ?2
             GROUP BY mdl
             ORDER BY SUM(u.cost_usd) DESC, SUM(u.input_tokens) DESC",
        )?;
        let rows = stmt.query_map((s, e), |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, f64>(1)?,
                r.get::<_, i64>(2)? as u64,
                r.get::<_, i64>(3)? as u64,
            ))
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Per-model spend + token counts for the current calendar month.
    pub fn spend_by_model_month(&self) -> Result<Vec<(String, f64, u64, u64)>> {
        let (s, e) = month_bounds_local(chrono::Local::now());
        let conn = self.lock()?;
        let mut stmt = conn.prepare_cached(
            "SELECT COALESCE(m.model, '') as mdl,
                    COALESCE(SUM(u.cost_usd), 0.0),
                    COALESCE(SUM(u.input_tokens), 0),
                    COALESCE(SUM(u.output_tokens), 0)
             FROM usage u JOIN message m ON m.id = u.message_id
             WHERE u.created_at >= ?1 AND u.created_at < ?2
             GROUP BY mdl
             ORDER BY SUM(u.cost_usd) DESC, SUM(u.input_tokens) DESC",
        )?;
        let rows = stmt.query_map((s, e), |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, f64>(1)?,
                r.get::<_, i64>(2)? as u64,
                r.get::<_, i64>(3)? as u64,
            ))
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    // --- Model health / failover (docs/features/mesh-routing.md) ---

    /// Bench a model until `cooldown_until` (epoch secs), recording why. Upsert: a fresh failure
    /// or probe overwrites any prior bench.
    pub fn bench_model(&self, model: &str, cooldown_until: i64, reason: &str) -> Result<()> {
        self.lock()?.execute(
            "INSERT INTO model_health (model, cooldown_until, reason, updated_at)
             VALUES (?1, ?2, ?3, strftime('%s','now'))
             ON CONFLICT(model) DO UPDATE SET
               cooldown_until = excluded.cooldown_until,
               reason = excluded.reason,
               updated_at = excluded.updated_at",
            (model, cooldown_until, reason),
        )?;
        Ok(())
    }

    /// Bench a model for `cooldown` from now (convenience over [`bench_model`] that owns the
    /// clock, like [`spend_today_usd`](Self::spend_today_usd)).
    pub fn bench_for(
        &self,
        model: &str,
        cooldown: std::time::Duration,
        reason: &str,
    ) -> Result<()> {
        let until = chrono::Utc::now().timestamp() + cooldown.as_secs() as i64;
        self.bench_model(model, until, reason)
    }

    /// Exclude a model that failed *permanently* (no tool-calling support, unaffordable, malformed
    /// tool payload — see [`ProviderError::Capability`](forge_provider::ProviderError::Capability)).
    /// Modeled as a long bench window so it reuses the `model_health` table and naturally
    /// *re-probes* after the window elapses (a provider may add tool support later). The reason is
    /// prefixed `excluded:` so the UI / report can distinguish it from a transient bench.
    pub fn exclude_model(&self, model: &str, reason: &str) -> Result<()> {
        let until = chrono::Utc::now().timestamp() + CAPABILITY_EXCLUSION_SECS;
        self.bench_model(model, until, &format!("excluded: {reason}"))
    }

    /// The non-excluded model whose bench expires soonest (the "least dead" model), as a
    /// last-resort fallback when every routable model is currently benched but none is a permanent
    /// capability exclusion. `None` when nothing is benched or all benches are permanent
    /// exclusions. Used by the core loop so a turn never hard-fails while a transient bench exists.
    pub fn soonest_unbenched(&self) -> Result<Option<String>> {
        let conn = self.lock()?;
        let row = conn
            .query_row(
                "SELECT model FROM model_health
                 WHERE reason NOT LIKE 'excluded:%'
                 ORDER BY cooldown_until ASC LIMIT 1",
                [],
                |r| r.get::<_, String>(0),
            )
            .optional()?;
        Ok(row)
    }

    /// All transiently-benched (non-excluded) models, soonest-recovering first. The caller
    /// applies its own filter (e.g. drop providers with no key) before picking a last-resort
    /// model — `soonest_unbenched` can't, since the store has no notion of key presence.
    pub fn transient_benched_ordered(&self) -> Result<Vec<String>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT model FROM model_health
             WHERE reason NOT LIKE 'excluded:%'
             ORDER BY cooldown_until ASC",
        )?;
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?;
        Ok(rows)
    }

    /// Currently-benched snapshot as of *now* (convenience over [`benched_models`]).
    pub fn current_benched(&self) -> Result<forge_types::ModelHealth> {
        self.benched_models(chrono::Utc::now().timestamp())
    }

    /// Currently-benched detailed report as of *now* (convenience over [`benched_report`]).
    pub fn current_benched_report(&self) -> Result<Vec<(String, i64, String)>> {
        self.benched_report(chrono::Utc::now().timestamp())
    }

    /// Clear any bench on a model (e.g. a healthy probe). No-op if it wasn't benched.
    pub fn clear_model_health(&self, model: &str) -> Result<()> {
        self.lock()?
            .execute("DELETE FROM model_health WHERE model = ?1", [model])?;
        Ok(())
    }

    /// Persist a model's fetched context window (tokens), from a provider's model API. Upsert so a
    /// later discovery refreshes it.
    pub fn set_model_context(&self, model: &str, window: u32) -> Result<()> {
        self.lock()?.execute(
            "INSERT INTO model_context (model, window, updated_at) VALUES (?1, ?2, strftime('%s','now'))
             ON CONFLICT(model) DO UPDATE SET window = excluded.window, updated_at = excluded.updated_at",
            (model, window),
        )?;
        Ok(())
    }

    /// A model's fetched context window (tokens), or `None` if we never stored one. The core
    /// prefers this over the family heuristic when bounding a turn's transcript.
    pub fn model_context(&self, model: &str) -> Result<Option<u32>> {
        let row = self
            .lock()?
            .query_row(
                "SELECT window FROM model_context WHERE model = ?1",
                [model],
                |r| r.get::<_, i64>(0),
            )
            .optional()?;
        Ok(row.map(|w| w.max(0) as u32))
    }

    /// Every known context-window size: `model -> tokens`. Fed into the mesh router so it can skip
    /// models whose window is smaller than the current transcript.
    pub fn all_model_contexts(&self) -> Result<std::collections::HashMap<String, u32>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare("SELECT model, window FROM model_context")?;
        let map = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?
            .filter_map(|r| r.ok())
            .map(|(model, w)| (model, w.max(0) as u32))
            .collect();
        Ok(map)
    }

    /// Persist a model's fetched USD price (per 1k tokens), from a provider's model API. Upsert so a
    /// later discovery refreshes it. `cache_read_per_1k` is the discounted prompt-cache-read rate
    /// (None if the provider didn't report one).
    pub fn set_model_pricing(
        &self,
        model: &str,
        input_per_1k: f64,
        output_per_1k: f64,
        cache_read_per_1k: Option<f64>,
    ) -> Result<()> {
        self.lock()?.execute(
            "INSERT INTO model_pricing (model, input_per_1k, output_per_1k, cache_read_per_1k, updated_at)
             VALUES (?1, ?2, ?3, ?4, strftime('%s','now'))
             ON CONFLICT(model) DO UPDATE SET input_per_1k = excluded.input_per_1k,
                 output_per_1k = excluded.output_per_1k, cache_read_per_1k = excluded.cache_read_per_1k,
                 updated_at = excluded.updated_at",
            (model, input_per_1k, output_per_1k, cache_read_per_1k),
        )?;
        Ok(())
    }

    /// Every fetched per-model price: `model -> (input_per_1k, output_per_1k, cache_read_per_1k)` in
    /// USD. Fed into the mesh's `Pricing` as overrides so gateway/credit spend is tracked, not $0.
    pub fn all_model_pricing(&self) -> Result<Vec<ModelPriceRow>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT model, input_per_1k, output_per_1k, cache_read_per_1k FROM model_pricing",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, f64>(1)?,
                    r.get::<_, f64>(2)?,
                    r.get::<_, Option<f64>>(3)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Clear every model bench (the `forge models --clear` rescan reset). Returns the number of
    /// benched rows removed so the caller can report it.
    pub fn clear_all_model_health(&self) -> Result<usize> {
        Ok(self.lock()?.execute("DELETE FROM model_health", [])?)
    }

    // --- /duel: model arena outcomes + routing-learning boosts (feature: duel) ---

    /// Record one `/duel` candidate's outcome (won or lost) for `repo_key` (the canonicalized repo
    /// root). Called once per candidate every time a duel resolves, so a model's full win/loss
    /// history in this repo can be reconstructed and aggregated by [`Store::duel_boosts`].
    pub fn record_duel_outcome(
        &self,
        repo_key: &str,
        model: &str,
        won: bool,
        task: &str,
    ) -> Result<()> {
        self.lock()?.execute(
            "INSERT INTO duel_outcome (id, repo_key, model, won, task) VALUES (?1, ?2, ?3, ?4, ?5)",
            (forge_types::new_id(), repo_key, model, won as i64, task),
        )?;
        Ok(())
    }

    /// Per-model routing boost for `repo_key`, learned from past `/duel` outcomes: `(wins - losses)
    /// as f64 * 0.5`, clamped to `[-2.0, 2.0]` so a long streak can't permanently dominate routing.
    /// Feeds `HeuristicRouter::with_repo_boosts`. Empty when the repo has no duel history.
    pub fn duel_boosts(&self, repo_key: &str) -> Result<std::collections::HashMap<String, f64>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT model, SUM(won), COUNT(*) FROM duel_outcome
             WHERE repo_key = ?1 GROUP BY model",
        )?;
        let rows = stmt
            .query_map([repo_key], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows
            .into_iter()
            .map(|(model, wins, total)| {
                let losses = total - wins;
                let boost = ((wins - losses) as f64 * 0.5).clamp(-2.0, 2.0);
                (model, boost)
            })
            .collect())
    }

    /// The per-model win/loss ledger behind [`Store::duel_boosts`], for the scoreboard view:
    /// `(model, wins, losses, boost)`, most-boosted first. Same source (`duel_outcome`) and the
    /// same boost math, so what the scoreboard shows is exactly what routing applies.
    pub fn model_scoreboard(&self, repo_key: &str) -> Result<Vec<(String, i64, i64, f64)>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT model, SUM(won), COUNT(*) FROM duel_outcome
             WHERE repo_key = ?1 GROUP BY model",
        )?;
        let rows = stmt
            .query_map([repo_key], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut out: Vec<(String, i64, i64, f64)> = rows
            .into_iter()
            .map(|(model, wins, total)| {
                let losses = total - wins;
                let boost = ((wins - losses) as f64 * 0.5).clamp(-2.0, 2.0);
                (model, wins, losses, boost)
            })
            .collect();
        out.sort_by(|a, b| b.3.partial_cmp(&a.3).unwrap_or(std::cmp::Ordering::Equal));
        Ok(out)
    }

    /// Snapshot of models still benched as of `now` (epoch secs) — cooldown not yet elapsed.
    pub fn benched_models(&self, now: i64) -> Result<forge_types::ModelHealth> {
        let conn = self.lock()?;
        let mut stmt =
            conn.prepare_cached("SELECT model FROM model_health WHERE cooldown_until > ?1")?;
        let set = stmt
            .query_map([now], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<std::collections::HashSet<_>, _>>()?;
        Ok(forge_types::ModelHealth::new(set))
    }

    /// Detailed view of currently-benched models (model, cooldown_until, reason) for the CLI /
    /// startup hint, newest cooldown first.
    pub fn benched_report(&self, now: i64) -> Result<Vec<(String, i64, String)>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT model, cooldown_until, reason FROM model_health
             WHERE cooldown_until > ?1 ORDER BY cooldown_until DESC",
        )?;
        let rows = stmt
            .query_map([now], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Record the latest subscription quota observation (quota-aware routing, L3). One row per
    /// bridge provider, upserted — the most recent `rate_limit_event` wins.
    ///
    /// Also appends the observation to the append-only `quota_history` table (mesh-routing.md)
    /// when a `fraction_used` is reported, so [`quota_history_since`](Self::quota_history_since) can
    /// later derive a consumption rate. This does NOT change `subscription_usage`'s upsert
    /// (latest-snapshot-only) semantics — it's a pure addition alongside it.
    pub fn record_quota(&self, hint: &forge_types::QuotaHint) -> Result<()> {
        let status = match hint.status {
            forge_types::QuotaStatus::Ok => "ok",
            forge_types::QuotaStatus::Warning => "warning",
            forge_types::QuotaStatus::Exhausted => "exhausted",
        };
        with_busy_retry(|| {
            let mut conn = self.lock()?;
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            tx.execute(
                "INSERT INTO subscription_usage (provider, window_kind, status, resets_at, fraction, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, strftime('%s','now'))
                 ON CONFLICT(provider, window_kind) DO UPDATE SET
                   status = excluded.status,
                   resets_at = excluded.resets_at,
                   fraction = excluded.fraction,
                   updated_at = excluded.updated_at",
                (
                    hint.provider.as_str(),
                    hint.window.as_str(),
                    status,
                    hint.resets_at,
                    hint.fraction_used,
                ),
            )?;
            if let Some(fraction_used) = hint.fraction_used {
                tx.execute(
                    "INSERT INTO quota_history (provider, window_kind, fraction_used, resets_at, observed_at)
                     VALUES (?1, ?2, ?3, ?4, strftime('%s','now'))",
                    (hint.provider.as_str(), hint.window.as_str(), fraction_used, hint.resets_at),
                )?;
            }
            tx.commit()?;
            Ok(())
        })
    }

    /// Append one observation to the quota usage history (mesh-routing.md). Called by
    /// [`record_quota`](Self::record_quota) for every hint that carries a `fraction_used`; exposed
    /// separately so callers/tests can seed history points directly (e.g. with a fixed
    /// `observed_at` via [`Self::record_quota_history_at`]).
    pub fn record_quota_history(
        &self,
        provider: &str,
        window: &str,
        fraction_used: f64,
        resets_at: Option<i64>,
    ) -> Result<()> {
        self.lock()?.execute(
            "INSERT INTO quota_history (provider, window_kind, fraction_used, resets_at, observed_at)
             VALUES (?1, ?2, ?3, ?4, strftime('%s','now'))",
            (provider, window, fraction_used, resets_at),
        )?;
        Ok(())
    }

    /// [`record_quota`](Self::record_quota) with an explicit `updated_at` (epoch secs) — the
    /// OBSERVATION time, not the recording time. The seeding paths (codex rollout files, `forge
    /// mesh`) re-record old observations whenever their freshness gate reopens; stamping those
    /// with `now()` would let hours-old rollout data continually mask fresher `x-codex-*` header
    /// readings in the alias-group merge ([`quota_at`](Self::quota_at)'s latest-wins is only
    /// correct when `updated_at` means observation time).
    ///
    /// Guard: an incoming observation OLDER than the row's existing `updated_at` is a complete
    /// no-op (upsert rejected via the `ON CONFLICT ... WHERE`, history skipped) — a late-arriving
    /// stale observation can never regress a fresher reading, regardless of caller discipline.
    /// A duplicate history point (same provider/window/`observed_at`) is also skipped, so
    /// re-seeding the same rollout observation every few minutes doesn't grow `quota_history`.
    pub fn record_quota_at(&self, hint: &forge_types::QuotaHint, updated_at: i64) -> Result<()> {
        let status = match hint.status {
            forge_types::QuotaStatus::Ok => "ok",
            forge_types::QuotaStatus::Warning => "warning",
            forge_types::QuotaStatus::Exhausted => "exhausted",
        };
        let conn = self.lock()?;
        let changed = conn.execute(
            "INSERT INTO subscription_usage (provider, window_kind, status, resets_at, fraction, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(provider, window_kind) DO UPDATE SET
               status = excluded.status,
               resets_at = excluded.resets_at,
               fraction = excluded.fraction,
               updated_at = excluded.updated_at
             WHERE excluded.updated_at >= subscription_usage.updated_at",
            (
                hint.provider.as_str(),
                hint.window.as_str(),
                status,
                hint.resets_at,
                hint.fraction_used,
                updated_at,
            ),
        )?;
        if changed == 0 {
            // Stale: the existing row was observed more recently than this hint.
            return Ok(());
        }
        if let Some(fraction_used) = hint.fraction_used {
            conn.execute(
                "INSERT INTO quota_history (provider, window_kind, fraction_used, resets_at, observed_at)
                 SELECT ?1, ?2, ?3, ?4, ?5
                 WHERE NOT EXISTS (
                     SELECT 1 FROM quota_history
                     WHERE provider = ?1 AND window_kind = ?2 AND observed_at = ?5
                 )",
                (
                    hint.provider.as_str(),
                    hint.window.as_str(),
                    fraction_used,
                    hint.resets_at,
                    updated_at,
                ),
            )?;
        }
        Ok(())
    }

    /// [`record_quota_history`](Self::record_quota_history) with an explicit `observed_at` (epoch
    /// secs) — a testable clock, mirroring [`quota_at`](Self::quota_at) for `current_quota`.
    pub fn record_quota_history_at(
        &self,
        provider: &str,
        window: &str,
        fraction_used: f64,
        resets_at: Option<i64>,
        observed_at: i64,
    ) -> Result<()> {
        self.lock()?.execute(
            "INSERT INTO quota_history (provider, window_kind, fraction_used, resets_at, observed_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            (provider, window, fraction_used, resets_at, observed_at),
        )?;
        Ok(())
    }

    /// History points for one provider+window, observed at or after `since` (epoch secs),
    /// oldest first — the input [`forge_types::compute_quota_pace`] needs to derive a rate.
    pub fn quota_history_since(
        &self,
        provider: &str,
        window: &str,
        since: i64,
    ) -> Result<Vec<forge_types::QuotaHistoryPoint>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT observed_at, fraction_used FROM quota_history
             WHERE provider = ?1 AND window_kind = ?2 AND observed_at >= ?3
             ORDER BY observed_at ASC",
        )?;
        let rows = stmt
            .query_map((provider, window, since), |row| {
                Ok(forge_types::QuotaHistoryPoint {
                    observed_at: row.get(0)?,
                    fraction_used: row.get(1)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Replace a session's task list (the `update_tasks` tool). Stored as one JSON row so a
    /// resumed session restores its tasks. An empty list clears it.
    pub fn set_tasks(&self, session_id: &str, tasks: &[forge_types::TodoItem]) -> Result<()> {
        let json = serde_json::to_string(tasks).unwrap_or_else(|_| "[]".to_string());
        self.lock()?.execute(
            "INSERT INTO session_tasks (session_id, tasks_json, updated_at)
             VALUES (?1, ?2, strftime('%s','now'))
             ON CONFLICT(session_id) DO UPDATE SET
               tasks_json = excluded.tasks_json, updated_at = excluded.updated_at",
            (session_id, json),
        )?;
        Ok(())
    }

    /// The session's persisted task list (empty if none/unparseable).
    pub fn tasks(&self, session_id: &str) -> Result<Vec<forge_types::TodoItem>> {
        let conn = self.lock()?;
        let json: Option<String> = conn
            .query_row(
                "SELECT tasks_json FROM session_tasks WHERE session_id = ?1",
                [session_id],
                |row| row.get(0),
            )
            .optional()?;
        Ok(json
            .and_then(|j| serde_json::from_str(&j).ok())
            .unwrap_or_default())
    }

    /// Snapshot of currently-constraining subscription quotas (rows whose window hasn't reset),
    /// for the router. Only `Warning`/`Exhausted` providers are carried — `Ok` is the default.
    pub fn current_quota(&self) -> Result<forge_types::SubscriptionQuota> {
        self.quota_at(chrono::Utc::now().timestamp())
    }

    /// Seconds since the most recent quota update for `provider` (`None` if never recorded). Used
    /// to gate the on-demand claude rate-limit probe so it refreshes at most every few minutes.
    pub fn subscription_age_secs(&self, provider: &str) -> Option<i64> {
        let conn = self.lock().ok()?;
        let updated: Option<i64> = conn
            .query_row(
                "SELECT MAX(updated_at) FROM subscription_usage WHERE provider = ?1",
                [provider],
                |r| r.get(0),
            )
            .ok()
            .flatten();
        updated.map(|u| chrono::Utc::now().timestamp() - u)
    }

    /// Per-provider, per-window fraction from `subscription_usage` (for display).
    /// Only returns non-stale rows (window hasn't reset yet or has no reset time).
    /// Returns `HashMap<provider, HashMap<window_kind, fraction>>`.
    pub fn bridge_fractions(
        &self,
    ) -> Result<std::collections::HashMap<String, std::collections::HashMap<String, f64>>> {
        let now = chrono::Utc::now().timestamp();
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT provider, window_kind, fraction FROM subscription_usage
         WHERE fraction IS NOT NULL AND (resets_at IS NULL OR resets_at > ?1)",
        )?;
        let mut out: std::collections::HashMap<String, std::collections::HashMap<String, f64>> =
            std::collections::HashMap::new();
        let rows = stmt.query_map([now], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, f64>(2)?,
            ))
        })?;
        for row in rows.flatten() {
            out.entry(row.0).or_default().insert(row.1, row.2);
        }
        Ok(out)
    }

    /// [`current_quota`](Self::current_quota) at an explicit `now` (epoch secs) — testable clock.
    ///
    /// `codex-cli` and `codex-oauth` bill the SAME ChatGPT account (mesh-routing.md), so their
    /// `subscription_usage`/`quota_history` rows are merged here at read time via
    /// [`QUOTA_ALIAS_GROUPS`] before the status/fraction/pace rollups run — see
    /// [`quota_alias_members`]. Non-grouped providers (e.g. `claude-cli`) are unaffected: a
    /// provider outside any group only ever merges with itself, which is a no-op.
    pub fn quota_at(&self, now: i64) -> Result<forge_types::SubscriptionQuota> {
        let conn = self.lock()?;

        struct UsageRow {
            provider: String,
            window: String,
            status: String,
            fraction: Option<f64>,
            resets_at: Option<i64>,
            updated_at: i64,
        }
        let raw_rows: Vec<UsageRow> = {
            let mut stmt = conn.prepare(
                "SELECT provider, window_kind, status, fraction, resets_at, updated_at
                 FROM subscription_usage
                 WHERE resets_at IS NULL OR resets_at > ?1",
            )?;
            let rows = stmt
                .query_map([now], |row| {
                    Ok(UsageRow {
                        provider: row.get(0)?,
                        window: row.get(1)?,
                        status: row.get(2)?,
                        fraction: row.get(3)?,
                        resets_at: row.get(4)?,
                        updated_at: row.get(5)?,
                    })
                })?
                .filter_map(std::result::Result::ok)
                .collect();
            rows
        };

        // Every distinct provider seen, expanded to its full alias-group membership — so a group
        // member with zero rows of its own (e.g. codex-cli when only codex-oauth has reported
        // usage) still surfaces the group's shared reading.
        let mut output_providers: std::collections::BTreeSet<&str> =
            std::collections::BTreeSet::new();
        for row in &raw_rows {
            for m in quota_alias_members(&row.provider) {
                output_providers.insert(m);
            }
        }

        let mut map = std::collections::HashMap::new();
        let mut fractions = std::collections::HashMap::new();
        let mut paces = std::collections::HashMap::new();
        let since = now - forge_types::QUOTA_PACE_LOOKBACK_SECS;

        for provider in output_providers {
            let members = quota_alias_members(provider);

            // Merge per-window rows across every group member — these are server-authoritative
            // snapshots of the SAME account for a grouped provider, so the row with the latest
            // `updated_at` wins per window. NEVER summed (that would double-count headroom).
            let mut by_window: std::collections::HashMap<&str, &UsageRow> =
                std::collections::HashMap::new();
            for row in &raw_rows {
                if !members.contains(&row.provider.as_str()) {
                    continue;
                }
                by_window
                    .entry(row.window.as_str())
                    .and_modify(|existing| {
                        if row.updated_at > existing.updated_at {
                            *existing = row;
                        }
                    })
                    .or_insert(row);
            }
            if by_window.is_empty() {
                continue;
            }

            let worst_status = by_window
                .values()
                .map(|r| quota_status_from_str(&r.status))
                .max()
                .unwrap_or_default();
            if worst_status != forge_types::QuotaStatus::Ok {
                map.insert(provider.to_string(), worst_status);
            }

            // Strictest (max-fraction) window with a known fraction — also carried for still-Ok
            // providers so the router's graduated conservation can spread ahead of Warning. The
            // pace projection below must be derived for this SAME window, not just any window.
            if let Some(strictest) =
                by_window
                    .values()
                    .filter(|r| r.fraction.is_some())
                    .max_by(|a, b| {
                        a.fraction
                            .partial_cmp(&b.fraction)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })
            {
                let fraction = strictest.fraction.unwrap_or(0.0);
                fractions.insert(provider.to_string(), fraction);

                // Pace projection off that same strictest window's history, UNIONED across every
                // alias-group member (both surfaces may have recorded points for the same shared
                // account) — a subscription burning fast early in its window is otherwise
                // under-protected by `fractions` alone (mesh-routing.md).
                let history =
                    self.quota_history_union(&conn, &members, &strictest.window, since)?;
                if let Some(pace) =
                    forge_types::compute_quota_pace(&history, strictest.resets_at, now)
                {
                    paces.insert(provider.to_string(), pace);
                }
            }
        }

        Ok(forge_types::SubscriptionQuota::new(map)
            .with_fractions(fractions)
            .with_paces(paces))
    }

    /// History points for `window`, observed at or after `since`, unioned across every provider
    /// in `members` (ascending `observed_at`) — the shared-account merge [`quota_at`] needs so a
    /// grouped provider's pace reflects history recorded under either surface name.
    fn quota_history_union(
        &self,
        conn: &Connection,
        members: &[&str],
        window: &str,
        since: i64,
    ) -> Result<Vec<forge_types::QuotaHistoryPoint>> {
        let placeholders = (0..members.len())
            .map(|i| format!("?{}", i + 3))
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT observed_at, fraction_used FROM quota_history
             WHERE window_kind = ?1 AND observed_at >= ?2 AND provider IN ({placeholders})
             ORDER BY observed_at ASC"
        );
        let mut stmt = conn.prepare(&sql)?;
        let mut params: Vec<&dyn rusqlite::ToSql> = vec![&window, &since];
        for m in members {
            params.push(m);
        }
        let rows = stmt
            .query_map(params.as_slice(), |row| {
                Ok(forge_types::QuotaHistoryPoint {
                    observed_at: row.get(0)?,
                    fraction_used: row.get(1)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
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
        self.lock()?.execute(
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
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT seq, role, content, model, created_at, visibility
             FROM message
             WHERE session_id = ?1
               AND (?2 IS NULL OR seq < ?2)
               AND (role IN ('user', 'assistant') OR visibility = 'ui')
               AND content != ''
             ORDER BY seq DESC LIMIT ?3",
        )?;
        let rows = stmt.query_map(
            rusqlite::params![session_id, before_seq, limit as i64],
            |row| {
                let role: String = row.get(1)?;
                let visibility: String = row.get(5)?;
                Ok(HistoryRow {
                    seq: row.get(0)?,
                    role: Role::parse(&role).unwrap_or(Role::User),
                    content: row.get(2)?,
                    model: row.get(3)?,
                    created_at: row.get(4)?,
                    visibility: Visibility::parse(&visibility),
                })
            },
        )?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(StoreError::from)
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

    // --- Assay runs + findings (docs/features/analysis-mode.md) ---

    /// Persist an assay run; returns its id. Add findings with [`add_finding`](Self::add_finding).
    pub fn create_assay_run(&self, scope: &str, cost_usd: f64) -> Result<String> {
        let id = forge_types::new_id();
        self.lock()?.execute(
            "INSERT INTO assay_run (id, scope, cost_usd) VALUES (?1, ?2, ?3)",
            (&id, scope, cost_usd),
        )?;
        Ok(id)
    }

    /// Persist one finding under a run.
    pub fn add_finding(&self, run_id: &str, f: &forge_types::Finding) -> Result<()> {
        self.lock()?.execute(
            "INSERT INTO finding (id, run_id, category, severity, confidence, file, line, title,
             rationale, suggested_fix, effort, lens, verified)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            rusqlite::params![
                f.id,
                run_id,
                f.category.as_str(),
                f.severity.as_str(),
                f.confidence.as_str(),
                f.file,
                f.line,
                f.title,
                f.rationale,
                f.suggested_fix,
                f.effort.as_str(),
                f.lens,
                f.verified as i64,
            ],
        )?;
        Ok(())
    }

    /// Findings of a run, ranked (severity, confidence) at read time.
    pub fn load_findings(&self, run_id: &str) -> Result<Vec<forge_types::Finding>> {
        use forge_types::{Confidence, Effort, FindingCategory, Severity};
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            // Actually rank by (severity, confidence) as the doc promises — the query had no ORDER BY,
            // so SQLite returned insertion order and the UI showed the least-important finding first.
            "SELECT id, category, severity, confidence, file, line, title, rationale,
                    suggested_fix, effort, lens, verified
             FROM finding WHERE run_id = ?1
             ORDER BY CASE severity WHEN 'critical' THEN 0 WHEN 'high' THEN 1 WHEN 'medium' THEN 2 ELSE 3 END,
                      CASE confidence WHEN 'high' THEN 0 WHEN 'medium' THEN 1 ELSE 2 END",
        )?;
        let rows = stmt.query_map([run_id], |row| {
            let category: String = row.get(1)?;
            let severity: String = row.get(2)?;
            let confidence: String = row.get(3)?;
            let effort: String = row.get(9)?;
            Ok(forge_types::Finding {
                id: row.get(0)?,
                category: FindingCategory::parse(&category).unwrap_or(FindingCategory::Correctness),
                severity: Severity::parse(&severity).unwrap_or(Severity::Low),
                confidence: Confidence::parse(&confidence).unwrap_or(Confidence::Low),
                file: row.get(4)?,
                line: row.get(5)?,
                title: row.get(6)?,
                rationale: row.get(7)?,
                suggested_fix: row.get(8)?,
                effort: Effort::parse(&effort).unwrap_or(Effort::Small),
                lens: row.get(10)?,
                verified: row.get::<_, i64>(11)? != 0,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    /// The most recent assay run for `scope`, excluding `exclude_id` (the just-created run).
    /// Returns `None` when this is the first run for this scope.
    pub fn latest_run_for_scope(&self, scope: &str, exclude_id: &str) -> Result<Option<String>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT id FROM assay_run WHERE scope = ?1 AND id != ?2
             ORDER BY created_at DESC, rowid DESC LIMIT 1",
        )?;
        let mut rows = stmt.query([scope, exclude_id])?;
        Ok(rows.next()?.map(|r| r.get(0)).transpose()?)
    }

    /// Past assay runs, newest first: `(id, scope, cost_usd, created_at)`.
    pub fn list_assay_runs(&self) -> Result<Vec<(String, String, f64, i64)>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT id, scope, cost_usd, created_at FROM assay_run ORDER BY created_at DESC, rowid DESC",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(StoreError::from)
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

    // --- Conversation checkpoints / undo (RFC session-management-and-commands, PR2) ---

    /// Soft-delete every message of a session with `seq >= from_seq` (an `/undo` / checkpoint
    /// rewind). The rows stay in the table (`active = 0`) for audit/redo; [`load_messages`]
    /// excludes them. Returns the number of messages deactivated.
    pub fn deactivate_messages_from(&self, session_id: &str, from_seq: i64) -> Result<usize> {
        Ok(self.lock()?.execute(
            "UPDATE message SET active = 0 WHERE session_id = ?1 AND seq >= ?2 AND active = 1",
            (session_id, from_seq),
        )?)
    }

    /// Save a checkpoint (rewind point) at `seq`. `label` NULL = an auto per-turn checkpoint.
    pub fn add_checkpoint(
        &self,
        session_id: &str,
        label: Option<&str>,
        seq: i64,
    ) -> Result<String> {
        let id = forge_types::new_id();
        self.lock()?.execute(
            "INSERT INTO checkpoint (id, session_id, label, seq) VALUES (?1, ?2, ?3, ?4)",
            (&id, session_id, label, seq),
        )?;
        Ok(id)
    }

    /// A session's named checkpoints, newest first.
    pub fn list_checkpoints(&self, session_id: &str) -> Result<Vec<CheckpointRow>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT id, label, seq, created_at FROM checkpoint
             WHERE session_id = ?1 ORDER BY seq DESC, created_at DESC",
        )?;
        let rows = stmt.query_map([session_id], |row| {
            Ok(CheckpointRow {
                id: row.get(0)?,
                label: row.get(1)?,
                seq: row.get(2)?,
                created_at: row.get(3)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(StoreError::from)
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
    /// The stored content hash for a file, or `None` if it hasn't been indexed — the
    /// incremental-update gate (skip files whose hash is unchanged).
    pub fn lattice_file_hash(&self, repo_root: &str, rel_path: &str) -> Result<Option<String>> {
        let conn = self.lock()?;
        let hash = conn
            .query_row(
                "SELECT content_hash FROM lattice_file WHERE repo_root = ?1 AND rel_path = ?2",
                (repo_root, rel_path),
                |r| r.get::<_, String>(0),
            )
            .optional()?;
        Ok(hash)
    }

    /// Insert or replace a file's row and its symbol nodes + edges atomically: the file's prior
    /// nodes are deleted first (cascading their edges), so re-indexing is idempotent.
    pub fn replace_lattice_file(
        &self,
        file: &LatticeFileRow,
        nodes: &[LatticeNodeRow],
        edges: &[LatticeEdgeRow],
        refs: &[LatticeRefRow],
    ) -> Result<()> {
        let mut conn = self.lock()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "INSERT INTO lattice_file (id, repo_root, rel_path, lang, content_hash, parse_status)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(id) DO UPDATE SET
                content_hash = excluded.content_hash,
                lang = excluded.lang,
                parse_status = excluded.parse_status,
                indexed_at = strftime('%s','now')",
            (
                &file.id,
                &file.repo_root,
                &file.rel_path,
                &file.lang,
                &file.content_hash,
                &file.parse_status,
            ),
        )?;
        // Replace the file's symbols (FK ON DELETE CASCADE clears their edges too).
        tx.execute("DELETE FROM lattice_node WHERE file_id = ?1", (&file.id,))?;
        for n in nodes {
            tx.execute(
                "INSERT INTO lattice_node
                   (id, file_id, kind, name, qualname, signature, span_start, span_end, line_start, pagerank)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0.0)",
                rusqlite::params![
                    n.id,
                    n.file_id,
                    n.kind,
                    n.name,
                    n.qualname,
                    n.signature,
                    n.span_start,
                    n.span_end,
                    n.line_start,
                ],
            )?;
        }
        for e in edges {
            tx.execute(
                "INSERT INTO lattice_edge (id, src_id, dst_id, kind, unresolved_name)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![e.id, e.src_id, e.dst_id, e.kind, e.unresolved_name],
            )?;
        }
        for r in refs {
            tx.execute(
                "INSERT INTO lattice_ref (id, src_id, name, kind, line)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![r.id, r.src_id, r.name, r.kind, r.line],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Distinct definitions that reference `name` — the direct callers/dependents of a symbol
    /// (one hop of `impact`). Resolves the name-keyed `lattice_ref` rows back to their src nodes.
    pub fn lattice_callers_by_name(
        &self,
        repo_root: &str,
        name: &str,
        limit: usize,
    ) -> Result<Vec<LatticeNodeRow>> {
        let conn = self.lock()?;
        // Scoped to `repo_root`: the store is global (one DB across every project + bench clone), so
        // an unscoped name match returns cross-repo collisions (a `Command` in a vendored django/ or
        // another crate). The caller's Lattice is bound to one repo_root; only its rows are relevant.
        let mut stmt = conn.prepare(
            "SELECT DISTINCT n.id, n.file_id, n.kind, n.name, n.qualname, n.signature,
                    n.span_start, n.span_end, n.line_start, n.pagerank
             FROM lattice_ref r
             JOIN lattice_node n ON n.id = r.src_id
             JOIN lattice_file f ON f.id = n.file_id
             WHERE r.name = ?1 AND n.name <> ?1 AND f.repo_root = ?2
             ORDER BY n.name
             LIMIT ?3",
        )?;
        let rows = stmt
            .query_map(
                rusqlite::params![name, repo_root, limit as i64],
                lattice_node_from_row,
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Distinct identifier names referenced *by* definitions named `name` — one forward hop for
    /// `path` BFS (what the symbol calls/uses).
    pub fn lattice_callees_of_name(&self, repo_root: &str, name: &str) -> Result<Vec<String>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT DISTINCT r.name
             FROM lattice_ref r
             JOIN lattice_node n ON n.id = r.src_id
             JOIN lattice_file f ON f.id = n.file_id
             WHERE n.name = ?1 AND r.name <> ?1 AND f.repo_root = ?2",
        )?;
        let rows = stmt
            .query_map(rusqlite::params![name, repo_root], |r| {
                r.get::<_, String>(0)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Total reference rows — completes the `status` summary.
    pub fn lattice_ref_count(&self) -> Result<i64> {
        let conn = self.lock()?;
        Ok(conn.query_row("SELECT COUNT(*) FROM lattice_ref", [], |r| r.get(0))?)
    }

    /// Symbols whose name contains `query` (case-insensitive), best-first: exact name, then
    /// prefix, then substring; capped at `limit`.
    pub fn lattice_nodes_by_name(
        &self,
        repo_root: &str,
        query: &str,
        limit: usize,
    ) -> Result<Vec<LatticeNodeRow>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT n.id, n.file_id, n.kind, n.name, n.qualname, n.signature,
                    n.span_start, n.span_end, n.line_start, n.pagerank,
                    CASE
                        WHEN lower(n.name) = lower(?1) THEN 0
                        WHEN lower(n.name) LIKE lower(?1) || '%' THEN 1
                        ELSE 2
                    END AS rank
             FROM lattice_node n
             JOIN lattice_file f ON f.id = n.file_id
             WHERE lower(n.name) LIKE '%' || lower(?1) || '%' AND f.repo_root = ?3
             ORDER BY rank, length(n.name), n.name
             LIMIT ?2",
        )?;
        let rows = stmt
            .query_map(rusqlite::params![query, limit as i64, repo_root], |r| {
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
                    pagerank: r.get(9)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// A single node row by id — used to resolve embedding-ranked node ids back to nodes.
    pub fn lattice_node_by_id(&self, id: &str) -> Result<Option<LatticeNodeRow>> {
        let conn = self.lock()?;
        match conn.query_row(
            "SELECT id, file_id, kind, name, qualname, signature, span_start, span_end, line_start, pagerank
             FROM lattice_node WHERE id = ?1",
            [id],
            lattice_node_from_row,
        ) {
            Ok(r) => Ok(Some(r)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Delete every indexed file under `repo_root` whose `rel_path` is NOT in `keep` — and, via
    /// `ON DELETE CASCADE`, all of its symbols/edges/refs. Called after a full `update` walk to
    /// purge files that were removed or are now skipped (deleted files, nested git repos / vendored
    /// trees), so stale symbols don't linger in queries or bloat the store. Returns the count pruned.
    pub fn prune_lattice_files_except(
        &self,
        repo_root: &str,
        keep: &std::collections::HashSet<String>,
    ) -> Result<usize> {
        let mut conn = self.lock()?;
        // IMMEDIATE: SELECTs then DELETEs — a DEFERRED read snapshot could fail to upgrade with
        // SQLITE_BUSY_SNAPSHOT if the indexer committed concurrently.
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let stale: Vec<String> = {
            let mut stmt =
                tx.prepare("SELECT id, rel_path FROM lattice_file WHERE repo_root = ?1")?;
            let rows = stmt.query_map([repo_root], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })?;
            rows.filter_map(|r| r.ok())
                .filter(|(_, rel)| !keep.contains(rel))
                .map(|(id, _)| id)
                .collect()
        };
        for id in &stale {
            tx.execute("DELETE FROM lattice_file WHERE id = ?1", (id,))?;
        }
        tx.commit()?;
        Ok(stale.len())
    }

    /// Every distinct `repo_root` with indexed files. The store is global (shared across projects
    /// and bench clones), so this surfaces orphan roots — e.g. a deleted `/tmp/swe-*/django` scratch
    /// checkout — that `update` can prune.
    pub fn lattice_repo_roots(&self) -> Result<Vec<String>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare("SELECT DISTINCT repo_root FROM lattice_file")?;
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Delete every indexed file under `repo_root` (cascading to its symbols/edges/refs). Used to
    /// drop an orphan root whose directory no longer exists on disk. Returns the count removed.
    pub fn prune_lattice_repo(&self, repo_root: &str) -> Result<usize> {
        let conn = self.lock()?;
        Ok(conn.execute("DELETE FROM lattice_file WHERE repo_root = ?1", [repo_root])?)
    }

    /// Delete a single indexed file's row (cascading to its symbols/edges/refs). Called by the file
    /// watcher when a source file is removed on disk, so its nodes don't linger as phantom symbols
    /// in `query`/`impact`. Returns 1 if a row was removed, 0 if it wasn't indexed.
    pub fn delete_lattice_file(&self, repo_root: &str, rel_path: &str) -> Result<usize> {
        Ok(self.lock()?.execute(
            "DELETE FROM lattice_file WHERE repo_root = ?1 AND rel_path = ?2",
            (repo_root, rel_path),
        )?)
    }

    /// The `rel_path` of an indexed file by its id (for rendering a node's location).
    pub fn lattice_file_path(&self, file_id: &str) -> Result<Option<String>> {
        let conn = self.lock()?;
        Ok(conn
            .query_row(
                "SELECT rel_path FROM lattice_file WHERE id = ?1",
                (file_id,),
                |r| r.get::<_, String>(0),
            )
            .ok())
    }

    /// `(files, nodes, edges)` row counts — the `forge lattice status` summary.
    pub fn lattice_counts(&self) -> Result<(i64, i64, i64)> {
        let conn = self.lock()?;
        let files = conn.query_row("SELECT COUNT(*) FROM lattice_file", [], |r| r.get(0))?;
        let nodes = conn.query_row("SELECT COUNT(*) FROM lattice_node", [], |r| r.get(0))?;
        let edges = conn.query_row("SELECT COUNT(*) FROM lattice_edge", [], |r| r.get(0))?;
        Ok((files, nodes, edges))
    }

    /// Upsert a node's embedding vector (semantic retrieval, code-intelligence.md §5.6). `vec` is
    /// stored as little-endian f32 components.
    pub fn put_lattice_embedding(&self, node_id: &str, vec: &[f32]) -> Result<()> {
        let mut bytes = Vec::with_capacity(vec.len() * 4);
        for f in vec {
            bytes.extend_from_slice(&f.to_le_bytes());
        }
        self.lock()?.execute(
            "INSERT INTO lattice_embedding (node_id, dim, vec) VALUES (?1, ?2, ?3)
             ON CONFLICT(node_id) DO UPDATE SET dim = excluded.dim, vec = excluded.vec",
            rusqlite::params![node_id, vec.len() as i64, bytes],
        )?;
        Ok(())
    }

    /// Nodes that don't yet have an embedding — the work list for incremental `embed_pending`.
    pub fn lattice_nodes_without_embedding(&self, limit: usize) -> Result<Vec<LatticeNodeRow>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT n.id, n.file_id, n.kind, n.name, n.qualname, n.signature,
                    n.span_start, n.span_end, n.line_start, n.pagerank
             FROM lattice_node n
             LEFT JOIN lattice_embedding e ON e.node_id = n.id
             WHERE e.node_id IS NULL
             LIMIT ?1",
        )?;
        let rows = stmt
            .query_map([limit as i64], lattice_node_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// All stored `(node_id, vector)` embeddings — loaded once to cosine-rank a query vector.
    pub fn lattice_embeddings(&self) -> Result<Vec<(String, Vec<f32>)>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare("SELECT node_id, vec FROM lattice_embedding")?;
        let rows = stmt.query_map([], |r| {
            let id: String = r.get(0)?;
            let blob: Vec<u8> = r.get(1)?;
            let vec = blob
                .chunks_exact(4)
                .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                .collect();
            Ok((id, vec))
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    /// How many nodes currently have an embedding (`forge lattice status`: "embeddings: N").
    pub fn lattice_embedding_count(&self) -> Result<i64> {
        Ok(self
            .lock()?
            .query_row("SELECT COUNT(*) FROM lattice_embedding", [], |r| r.get(0))?)
    }

    /// All (src_id, dst_name) pairs from lattice_ref — the directed reference graph for PageRank.
    /// `src_id` is the referencing node's id; `dst_name` is the referenced identifier (resolved to
    /// node ids by name-join at call time). Returns (src_node_id, referenced_name) pairs.
    /// Scoped to `repo_root` — the store is global (one DB across every project), so an unscoped
    /// scan would mix another project's refs into THIS repo's PageRank (cross-repo contamination).
    pub fn lattice_ref_edges(&self, repo_root: &str) -> Result<Vec<(String, String)>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT r.src_id, r.name FROM lattice_ref r
             JOIN lattice_node n ON n.id = r.src_id
             JOIN lattice_file f ON f.id = n.file_id
             WHERE f.repo_root = ?1",
        )?;
        let rows = stmt
            .query_map([repo_root], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// All nodes ordered by pagerank descending, capped at `limit` — the repo-map selection query.
    /// Returns the top-N most important symbols across all files in the index; the caller applies
    /// a token-budget cutoff. Use `usize::MAX` to retrieve every node (for small repos).
    pub fn lattice_nodes_ranked(
        &self,
        repo_root: &str,
        limit: usize,
    ) -> Result<Vec<LatticeNodeRow>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT n.id, n.file_id, n.kind, n.name, n.qualname, n.signature,
                    n.span_start, n.span_end, n.line_start, n.pagerank
             FROM lattice_node n
             JOIN lattice_file f ON f.id = n.file_id
             WHERE f.repo_root = ?1
             ORDER BY n.pagerank DESC
             LIMIT ?2",
        )?;
        let rows = stmt
            .query_map(
                rusqlite::params![repo_root, limit as i64],
                lattice_node_from_row,
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// All (node_id, node_name) pairs for `repo_root` — needed to resolve reference names to node ids
    /// for PageRank. Scoped so a sibling project's nodes don't absorb this repo's reference rank.
    pub fn lattice_node_ids_and_names(&self, repo_root: &str) -> Result<Vec<(String, String)>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT n.id, n.name FROM lattice_node n
             JOIN lattice_file f ON f.id = n.file_id
             WHERE f.repo_root = ?1",
        )?;
        let rows = stmt
            .query_map([repo_root], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Batch-update pagerank scores: for each `(node_id, score)` pair, set `pagerank = score`.
    /// Committed in chunks (each its own IMMEDIATE transaction) rather than one giant write-txn, so
    /// the single WAL writer lock is released between batches — a full-table update used to hold it
    /// long enough to starve a concurrent critical write (transcript/usage) past `busy_timeout`.
    pub fn set_lattice_pageranks(&self, scores: &[(String, f64)]) -> Result<()> {
        if scores.is_empty() {
            return Ok(());
        }
        const CHUNK: usize = 500;
        for chunk in scores.chunks(CHUNK) {
            with_busy_retry(|| {
                let mut conn = self.lock()?;
                let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
                {
                    let mut stmt =
                        tx.prepare("UPDATE lattice_node SET pagerank = ?2 WHERE id = ?1")?;
                    for (id, score) in chunk {
                        stmt.execute(rusqlite::params![id, score])?;
                    }
                }
                tx.commit()?;
                Ok(())
            })?;
        }
        Ok(())
    }

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
        if n % LIVE_EVENT_PRUNE_EVERY == 0 {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_reservation_conflicts_across_independently_opened_stores() {
        let first_store = Store::open_in_memory().unwrap();
        let second_store = Store::open_in_memory().unwrap();
        let reservation = first_store.try_reserve_model("openai::gpt-4o").unwrap();

        assert!(second_store.try_reserve_model("openai::gpt-4o").is_none());

        drop(reservation);
        assert!(second_store.try_reserve_model("openai::gpt-4o").is_some());
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
                    cached_input_tokens: 0,
                    cost_usd: 0.02,
                },
            )
            .unwrap();
        store
            .record_tool_call(&mid, "read_file", "{}", "ok", "allowed", "ok")
            .unwrap();

        assert_eq!(store.message_count(&sid).unwrap(), 1);
        assert!((store.session_cost(&sid).unwrap() - 0.02).abs() < 1e-9);
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
    fn history_page_keeps_compacted_away_rows_for_the_user() {
        // Compaction soft-deletes old rows from the MODEL's view; the user's scrollback (and so
        // the remote history page) still shows them.
        let store = Store::open_in_memory().unwrap();
        let sid = store.create_session("/x", "default").unwrap();
        for i in 0..6 {
            let role = if i % 2 == 0 {
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
        let usage = |i: u64, o: u64| Usage {
            input_tokens: i,
            output_tokens: o,
            cached_input_tokens: 0,
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
        store.record_usage(&sid, &m0, &usage(100, 10)).unwrap();
        let m1 = store
            .add_message(
                &sid,
                1,
                Role::Assistant,
                "b",
                Some("codex-oauth::gpt-5.6-terra"),
            )
            .unwrap();
        store.record_usage(&sid, &m1, &usage(50, 5)).unwrap();
        let m2 = store
            .add_message(&sid, 2, Role::Assistant, "c", Some("nvidia::z-ai/glm-5.2"))
            .unwrap();
        store.record_usage(&sid, &m2, &usage(30, 3)).unwrap();
        store
            .record_side_call_usage(&sid, "compact", &usage(20, 2))
            .unwrap();

        let week = store.usage_by_provider_since(0).unwrap();
        let terra = week
            .iter()
            .find(|r| r.provider == "codex-oauth")
            .expect("codex-oauth provider derived from model namespace");
        assert_eq!(terra.input_tokens, 150, "both terra turns summed");
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
        for i in 0..10 {
            let role = if i % 2 == 0 {
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
        for i in 0..10 {
            let role = if i % 2 == 0 {
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
        for i in 0..10 {
            let role = if i % 2 == 0 {
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
            },
            TodoItem {
                title: "wire it up".into(),
                status: TodoStatus::InProgress,
            },
        ];
        store.set_tasks(&sid, &tasks).unwrap();
        assert_eq!(store.tasks(&sid).unwrap(), tasks, "round-trips");

        // A second write replaces the list wholesale.
        let next = vec![TodoItem {
            title: "ship".into(),
            status: TodoStatus::Pending,
        }];
        store.set_tasks(&sid, &next).unwrap();
        assert_eq!(store.tasks(&sid).unwrap(), next, "replaced, not appended");
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
        assert_eq!(store.lattice_embedding_count().unwrap(), 1);
        let all = store.lattice_embeddings().unwrap();
        assert_eq!(all, vec![("n1".to_string(), vec![1.0, -0.5, 0.25])]);
        // Upsert replaces, not duplicates.
        store.put_lattice_embedding("n1", &[2.0, 2.0]).unwrap();
        assert_eq!(store.lattice_embedding_count().unwrap(), 1);
        assert_eq!(store.lattice_embeddings().unwrap()[0].1, vec![2.0, 2.0]);
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
            let store = if t % 2 == 0 {
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
            let store = if t % 2 == 0 {
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
}
