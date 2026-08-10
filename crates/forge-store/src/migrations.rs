//! Versioned SQLite schema migrations and compatibility repairs.

use super::*;

/// Migrate `subscription_usage` from its old single-column PK to the composite
/// `(provider, window_kind)` PK. Safe to call on any DB version: a no-op when the table
/// doesn't exist yet (schema will create it correctly) or already has the composite key.
pub(super) fn migrate_subscription_usage(conn: &Connection) -> rusqlite::Result<()> {
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
pub(super) fn add_column_if_missing(conn: &Connection, sql: &str) -> rusqlite::Result<()> {
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
pub(super) fn migration_0001(conn: &Connection) -> rusqlite::Result<()> {
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

/// Migration #14: the live Codex backend plan header is a short-lived account observation,
/// persisted so a new `forge mesh` / TUI process does not fall back to an older JWT claim.
fn migration_0014(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS subscription_plan_observation (
             provider TEXT PRIMARY KEY,
             plan TEXT NOT NULL,
             observed_at INTEGER NOT NULL
         )",
    )
}

/// Migration #15: v14 briefly translated OpenAI's exact `pro` header to Forge's own
/// `pro-20x` label. This table is only a short-lived backend observation, so repair that value
/// in place and preserve the provider's spelling going forward.
fn migration_0015(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE subscription_plan_observation SET plan = 'pro' WHERE plan = 'pro-20x'",
        [],
    )?;
    Ok(())
}

/// Privacy-preserving route-attempt outcomes.  Prompts and model output never enter this table:
/// it is strictly the small amount of operational evidence needed to correct persistent provider
/// quality/latency drift without letting one transient failure rewrite Mesh ranking.
fn migration_0016(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS mesh_outcome (
             id INTEGER PRIMARY KEY,
             session_id TEXT NOT NULL,
             model TEXT NOT NULL,
             tier TEXT NOT NULL,
             started_at INTEGER NOT NULL,
             completed_at INTEGER NOT NULL,
             latency_ms INTEGER NOT NULL,
             outcome TEXT NOT NULL,
             error_kind TEXT,
             failover_hop INTEGER NOT NULL DEFAULT 0,
             tool_calls INTEGER NOT NULL DEFAULT 0,
             verified_completion INTEGER NOT NULL DEFAULT 0
         );
         CREATE INDEX IF NOT EXISTS idx_mesh_outcome_model_completed
             ON mesh_outcome(model, completed_at DESC);",
    )
}

/// Source freshness for the authoritative direct Codex OAuth probe.  Subscription usage rows are
/// deliberately shared with the CLI bridge, so their timestamp alone cannot prove that a live
/// OAuth header has been observed; without this marker a fresh-but-stale bridge rollout could
/// suppress the preferred probe indefinitely.
fn migration_0017(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS codex_oauth_quota_observation (
             singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
             observed_at INTEGER NOT NULL
         )",
        [],
    )?;
    Ok(())
}

/// Preserve provider-reported prompt-cache reads. `Usage` has carried this field across every
/// provider and sync payload, but the SQL row historically discarded it, making real cache hits
/// invisible to the CLI/TUI and remote usage surfaces.
fn migration_0018(conn: &Connection) -> rusqlite::Result<()> {
    add_column_if_missing(
        conn,
        "ALTER TABLE usage ADD COLUMN cached_input_tokens INTEGER NOT NULL DEFAULT 0",
    )
}

/// Migration #19: saved-workflow run history (`/workflow run <name>`, docs/rfcs/forge-workflow.md).
/// One row per run of a `.forge/workflows/<name>.js` script, so the app's workflow library can show
/// what a workflow has actually done instead of an always-empty strip. Local machine state like
/// `schedule`/`queue_task` — `cwd` and session ids don't travel — so deliberately NOT in
/// [`PORTABLE_METADATA_TABLES`]. Cascades with its session: the run is part of that session's
/// history, and a pruned session should not leave orphan rows pointing at a transcript that's gone.
///
/// Must stay idempotent (like every step from 18 on): [`run_migrations`]'s pre-release Anywhere
/// repair rewinds a `user_version` of 18-21 to 17, so a genuine v19 DB re-runs this step on open.
fn migration_0019(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS workflow_run (
            id          TEXT PRIMARY KEY,
            name        TEXT NOT NULL,
            session_id  TEXT NOT NULL REFERENCES session(id) ON DELETE CASCADE,
            cwd         TEXT NOT NULL,
            started_at  INTEGER NOT NULL DEFAULT (strftime('%s','now')),
            finished_at INTEGER,
            status      TEXT NOT NULL DEFAULT 'running',
            summary     TEXT,
            phases      INTEGER NOT NULL DEFAULT 0,
            agents      INTEGER NOT NULL DEFAULT 0,
            cost_usd    REAL NOT NULL DEFAULT 0
         );
         CREATE INDEX IF NOT EXISTS idx_workflow_run_lookup
             ON workflow_run(name, cwd, started_at DESC)",
    )
}

/// Migration #20: APNs registration identity is the device token. The original application-side
/// SELECT-then-INSERT could race across pooled connections and create duplicate deliveries. Keep
/// the earliest row, then enforce the identity at the database boundary for atomic upserts.
pub(super) fn migration_0020(conn: &Connection) -> rusqlite::Result<()> {
    let exists: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='apns_subscription'",
        [],
        |row| row.get(0),
    )?;
    if exists == 0 {
        return Ok(());
    }
    let valid_index: i64 = conn.query_row(
        "SELECT COUNT(*) FROM pragma_index_list('apns_subscription') AS indexes
         WHERE indexes.name = 'idx_apns_subscription_device_token'
           AND indexes.[unique] = 1
           AND (SELECT COUNT(*) FROM pragma_index_info(indexes.name)) = 1
           AND (SELECT name FROM pragma_index_info(indexes.name) LIMIT 1) = 'device_token'",
        [],
        |row| row.get(0),
    )?;
    if valid_index > 0 {
        return Ok(());
    }
    conn.execute_batch(
        "BEGIN IMMEDIATE;
         DROP INDEX IF EXISTS idx_apns_subscription_device_token;
         DELETE FROM apns_subscription WHERE rowid NOT IN (
             SELECT MIN(rowid) FROM apns_subscription GROUP BY device_token
         );
         CREATE UNIQUE INDEX idx_apns_subscription_device_token
             ON apns_subscription(device_token);
         COMMIT;",
    )
}

/// Migration #21: durable per-session live-event append counters. A counter in the database keeps
/// amortized ring-buffer pruning independent across Store handles and sessions.
fn migration_0021(conn: &Connection) -> rusqlite::Result<()> {
    add_column_if_missing(
        conn,
        "ALTER TABLE session ADD COLUMN live_event_writes INTEGER NOT NULL DEFAULT 0",
    )
}

/// Migration #22: the model a session is pinned to. The pin lived only in the running driver, so
/// resuming a session silently dropped it and the mesh routed the resumed turns by classification
/// instead — a pinned session could come back on a different model. Storing it lets a resume
/// restore the pin the session was created with.
fn migration_0022(conn: &Connection) -> rusqlite::Result<()> {
    add_column_if_missing(conn, "ALTER TABLE session ADD COLUMN pinned_model TEXT")
}

/// Migration #23: the reasoning-effort level a session is pinned to. Like the model pin this only
/// existed in the running session, so resuming silently dropped it back to the provider default —
/// a session driven at `whitehot` came back at `medium` without saying so.
fn migration_0023(conn: &Connection) -> rusqlite::Result<()> {
    add_column_if_missing(conn, "ALTER TABLE session ADD COLUMN pinned_effort TEXT")
}

/// Migration #24: whether a session belonged to the daemon's live fleet when it was last seen.
///
/// Fleet membership lived only in the running `SessionRegistry`, so restarting `forge serve` came
/// back with an empty fleet: `GET /api/sessions` returned `[]` and mid-task sessions were invisible
/// until someone re-added them by hand with an explicit resume. Persisting the flag lets startup
/// resurrect exactly the sessions that were live, and nothing the user had finished with.
fn migration_0024(conn: &Connection) -> rusqlite::Result<()> {
    add_column_if_missing(
        conn,
        "ALTER TABLE session ADD COLUMN daemon_live INTEGER NOT NULL DEFAULT 0",
    )
}

/// Migration #25: fleet agent-to-agent messaging (`forge send`, the `message_session` virtual
/// tool). One row per message queued for a daemon-hosted (fleet) session, so a message that
/// arrives while its target is offline — or sitting undelivered when the daemon itself restarts
/// — is not lost. `delivered_at IS NULL` is the pending set; `forge serve` drains it into the
/// target's live input queue as soon as that session (re)joins the registry. The two indexes
/// serve the daemon's own hot paths: draining one target's backlog, and enforcing the
/// per-sender-per-target pending cap at enqueue time.
fn migration_0025(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS fleet_message (
            id                TEXT PRIMARY KEY,
            sender_kind       TEXT NOT NULL,   -- 'cli' | 'session'
            sender_id         TEXT,            -- sending session id, when sender_kind = 'session'
            sender_label      TEXT NOT NULL,   -- display name: 'cli', or the sender session's name/id
            target_session_id TEXT NOT NULL,
            body              TEXT NOT NULL,
            mode              TEXT NOT NULL,   -- 'follow_up' | 'steer'
            created_at        INTEGER NOT NULL DEFAULT (strftime('%s','now')),
            delivered_at      INTEGER
         );
         CREATE INDEX IF NOT EXISTS idx_fleet_message_target
             ON fleet_message(target_session_id, delivered_at);
         CREATE INDEX IF NOT EXISTS idx_fleet_message_cap
             ON fleet_message(sender_label, target_session_id, delivered_at)",
    )
}

/// Migration #26: terminal-local session presence — whether a plain `forge` chat process
/// (not `forge serve`, not an MCP agent) currently has this session open, whether it is
/// mid-turn, and when it last checked in.
///
/// This is what lets a local terminal session surface (read-only) in the Anywhere/tunnelled
/// daemon's fleet list alongside `forge serve`-hosted sessions: `local_live` marks membership,
/// `local_busy` mirrors the turn state, and `local_last_seen` is a heartbeat the read side ages
/// out (`LOCAL_PRESENCE_STALE_SECS`) so a killed terminal — which never gets to clear its own
/// row — silently drops out of the fleet instead of appearing live forever.
fn migration_0026(conn: &Connection) -> rusqlite::Result<()> {
    for stmt in [
        "ALTER TABLE session ADD COLUMN local_live INTEGER NOT NULL DEFAULT 0",
        "ALTER TABLE session ADD COLUMN local_busy INTEGER NOT NULL DEFAULT 0",
        "ALTER TABLE session ADD COLUMN local_last_seen INTEGER",
    ] {
        add_column_if_missing(conn, stmt)?;
    }
    Ok(())
}

/// Migration #27: Continual Harness (`/refine`, port of prime-agent's `/refine`) persistence.
///
/// `harness_entry` holds the durable prompt/skill/subagent artifacts the agent proposes about
/// itself, scoped to `global`, a `project:<abs path>`, or a `session:<session id>`. `kind` +
/// `updated_at` are the hot lookup path (context injection reads the freshest entries per scope),
/// hence the composite index. `harness_refinement` is the append-only audit log of every batch of
/// edits applied to `harness_entry`: `edits_json` carries each edit's full before/after entry
/// snapshot, so [`Store::rollback_harness_refinement`] can invert a refinement from the journal
/// alone, without depending on `harness_entry`'s current (possibly further-mutated) state.
fn migration_0027(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS harness_entry (
            id         TEXT PRIMARY KEY,
            scope      TEXT NOT NULL,
            kind       TEXT NOT NULL,
            title      TEXT NOT NULL,
            content    TEXT NOT NULL,
            source     TEXT NOT NULL,
            version    INTEGER NOT NULL DEFAULT 1,
            created_at INTEGER NOT NULL DEFAULT (strftime('%s','now')),
            updated_at INTEGER NOT NULL DEFAULT (strftime('%s','now'))
         );
         CREATE INDEX IF NOT EXISTS idx_harness_entry_scope_kind ON harness_entry(scope, kind);

         CREATE TABLE IF NOT EXISTS harness_refinement (
            id               TEXT PRIMARY KEY,
            session_id       TEXT NOT NULL,
            trigger          TEXT NOT NULL,
            summary          TEXT NOT NULL,
            rationale        TEXT NOT NULL,
            expected_outcome TEXT NOT NULL,
            edits_json       TEXT NOT NULL,
            created_at       INTEGER NOT NULL DEFAULT (strftime('%s','now'))
         );
         CREATE INDEX IF NOT EXISTS idx_harness_refinement_session
             ON harness_refinement(session_id, created_at DESC)",
    )
}

/// Ordered migration steps. Index `i` upgrades the DB from `user_version = i` to `i + 1`. Append
/// new steps here and bump [`SCHEMA_VERSION`]; never reorder or rewrite an already-shipped step.
pub(super) const MIGRATIONS: &[fn(&Connection) -> rusqlite::Result<()>] = &[
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
    migration_0014,
    migration_0015,
    migration_0016,
    migration_0017,
    migration_0018,
    migration_0019,
    migration_0020,
    migration_0021,
    migration_0022,
    migration_0023,
    migration_0024,
    migration_0025,
    migration_0026,
    migration_0027,
];

/// Create the singleton rows the Anywhere sync state machine expects, if they are missing.
///
/// These were `INSERT OR IGNORE` statements at the end of [`schema::SCHEMA`]. An `INSERT` takes the
/// single WAL writer lock even when the row already exists and nothing changes, so they made merely
/// OPENING the store a write on every one of Forge's many short-lived processes (CLI subcommands,
/// statusline probes, `mcp-serve`) — the same startup contention the `user_version` repair used to
/// cause, see [`run_migrations`]. Reading first keeps opening an initialized database write-free.
pub(super) fn seed_singleton_rows(conn: &Connection) -> rusqlite::Result<()> {
    for (table, insert) in [
        (
            "anywhere_sync_state",
            "INSERT OR IGNORE INTO anywhere_sync_state (singleton, enabled) VALUES (1, 0)",
        ),
        (
            "anywhere_sync_cursor",
            "INSERT OR IGNORE INTO anywhere_sync_cursor (singleton, cursor) VALUES (1, 0)",
        ),
    ] {
        let present = conn
            .prepare(&format!("SELECT 1 FROM {table} WHERE singleton = 1"))?
            .exists([])?;
        if !present {
            conn.execute(insert, [])?;
        }
    }
    Ok(())
}

/// Lowest `user_version` inside the ambiguous pre-release window (see [`run_migrations`]). Equals
/// the first version the unreleased Forge Anywhere branch stamped, i.e. one past the last
/// unambiguously public version at the time that branch forked.
pub(super) const ANYWHERE_PRERELEASE_MIN_VERSION: i64 = 18;

/// Highest `user_version` the unreleased Forge Anywhere branch stamped.
pub(super) const ANYWHERE_PRERELEASE_MAX_VERSION: i64 = 21;

/// Whether public migration 18's column is present.
///
/// This is the evidence that separates a database written by a PUBLIC build from one stamped by the
/// unreleased Forge Anywhere branch inside the ambiguous window: that branch forked while
/// [`SCHEMA_VERSION`] was still 17, before [`migration_0018`] existed, and `CREATE TABLE IF NOT
/// EXISTS usage` in the base schema cannot add a column to a table that already exists. So no
/// pre-release database can carry this column, and every public database at 18 or above must.
fn public_v18_marker_present(conn: &Connection) -> rusqlite::Result<bool> {
    conn.prepare("SELECT 1 FROM pragma_table_info('usage') WHERE name = 'cached_input_tokens'")?
        .exists([])
}

/// Apply every migration the DB hasn't seen yet, bumping `PRAGMA user_version` after each so a
/// crash mid-run resumes cleanly. Refuses (with [`StoreError::SchemaTooNew`]) to open a DB written
/// by a newer build, rather than silently misreading it.
///
/// Opening an already-current database performs NO write at all, which matters because Forge runs
/// many short-lived processes (CLI subcommands, statusline probes, `mcp-serve`) that would otherwise
/// each contend for the single WAL writer just to open a database they have no work to do on.
pub(super) fn run_migrations(conn: &Connection) -> Result<()> {
    debug_assert_eq!(MIGRATIONS.len() as i64, SCHEMA_VERSION);
    let mut current: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    // Forge Anywhere's additive tables briefly consumed versions 18-21 on its unreleased feature
    // branch. They now live in the idempotent base schema so the previous public binary can open a
    // database after rollback. A number in that window is therefore AMBIGUOUS: it may come from that
    // branch (which never ran the public steps 18+) or from a public build that legitimately reached
    // it. Resolve it by replaying the public steps rather than by trusting the number.
    if (ANYWHERE_PRERELEASE_MIN_VERSION..=ANYWHERE_PRERELEASE_MAX_VERSION).contains(&current) {
        // Read the marker BEFORE the replay below adds the column it looks for.
        let from_public_build = public_v18_marker_present(conn)?;
        // Every step from 18 on is additive and idempotent by contract (`CREATE ... IF NOT EXISTS` /
        // `add_column_if_missing`), so replaying them is a pure no-op — and writes nothing — on a
        // database that really is at `current`, while a pre-release database that merely reused these
        // numbers gets the public tables/columns it is missing. The previous form rewound
        // `user_version` to 17 instead, which wrote page 1 twice on EVERY open of an already-migrated
        // database and made startup fail with "database is locked" whenever a long lattice write held
        // the writer.
        for step in ANYWHERE_PRERELEASE_MIN_VERSION..=current.min(SCHEMA_VERSION) {
            MIGRATIONS[(step - 1) as usize](conn)?;
        }
        // A version above what this build ships, inside the window, and WITHOUT the public marker can
        // only have come from the pre-release branch — renumber it down once so it stops tripping the
        // refusal below. With the marker present the number is trustworthy, so a database from a
        // genuinely newer public build is refused instead of being silently downgraded.
        if current > SCHEMA_VERSION && !from_public_build {
            conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
            current = SCHEMA_VERSION;
        }
    }
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
