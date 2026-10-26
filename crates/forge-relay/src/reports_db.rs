//! The reports SQLite: one row per distinct problem, per-version/OS counts, and hashed install
//! activity. Counts are written synchronously on every accepted report so they survive a GitHub
//! outage; the GitHub issue is reconciled later from this state (see `reports_sync.rs`).

use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;
use std::sync::Mutex;

pub(crate) struct ReportsDb {
    conn: Mutex<Connection>,
}

#[derive(Debug, Clone)]
pub(crate) struct Problem {
    pub(crate) fingerprint: String,
    pub(crate) manual: bool,
    pub(crate) issue_number: Option<u64>,
    pub(crate) title: String,
    pub(crate) first_seen: i64,
    pub(crate) last_seen: i64,
    pub(crate) count: i64,
    pub(crate) closed_in_version: Option<String>,
    pub(crate) representative: String,
    pub(crate) max_version: String,
    pub(crate) synced_max_version: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct VersionCount {
    pub(crate) version: String,
    pub(crate) os: String,
    pub(crate) count: i64,
    pub(crate) commented: bool,
}

/// What `record` stores for one accepted report.
pub(crate) struct NewReport<'a> {
    pub(crate) key: &'a str,
    pub(crate) manual: bool,
    pub(crate) title: &'a str,
    pub(crate) representative: &'a str,
    pub(crate) version: &'a str,
    pub(crate) os: &'a str,
    pub(crate) install_hash: &'a str,
    pub(crate) now: i64,
}

impl ReportsDb {
    pub(crate) fn open(path: &Path) -> anyhow::Result<Self> {
        let conn = Connection::open(path)?;
        Self::init(conn)
    }

    fn init(conn: Connection) -> anyhow::Result<Self> {
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=NORMAL;
             CREATE TABLE IF NOT EXISTS problems(
               fingerprint TEXT PRIMARY KEY,
               manual INTEGER NOT NULL DEFAULT 0,
               issue_number INTEGER,
               title TEXT NOT NULL,
               first_seen INTEGER NOT NULL,
               last_seen INTEGER NOT NULL,
               count INTEGER NOT NULL,
               state TEXT NOT NULL DEFAULT 'open',
               closed_in_version TEXT,
               representative TEXT NOT NULL,
               max_version TEXT NOT NULL,
               synced_max_version TEXT,
               dirty INTEGER NOT NULL DEFAULT 1,
               attempts INTEGER NOT NULL DEFAULT 0,
               next_sync_at INTEGER NOT NULL DEFAULT 0,
               last_body_hash TEXT);
             CREATE TABLE IF NOT EXISTS version_counts(
               fingerprint TEXT NOT NULL,
               version TEXT NOT NULL,
               os TEXT NOT NULL,
               count INTEGER NOT NULL,
               commented INTEGER NOT NULL DEFAULT 0,
               PRIMARY KEY(fingerprint, version, os));
             CREATE TABLE IF NOT EXISTS installs(
               hash TEXT PRIMARY KEY,
               first_seen INTEGER NOT NULL,
               last_seen INTEGER NOT NULL,
               total INTEGER NOT NULL);",
        )?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub(crate) fn record(&self, r: &NewReport<'_>) -> anyhow::Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let existing: Option<String> = tx
            .query_row(
                "SELECT max_version FROM problems WHERE fingerprint=?1",
                [r.key],
                |row| row.get(0),
            )
            .optional()?;
        match existing {
            None => {
                tx.execute(
                    "INSERT INTO problems(fingerprint,manual,title,first_seen,last_seen,count,representative,max_version)
                     VALUES(?1,?2,?3,?4,?4,1,?5,?6)",
                    params![r.key, r.manual, r.title, r.now, r.representative, r.version],
                )?;
            }
            Some(max) => {
                let newest = if crate::github::version_newer(r.version, &max) {
                    r.version.to_string()
                } else {
                    max
                };
                tx.execute(
                    "UPDATE problems SET count=count+1,last_seen=?2,dirty=1,max_version=?3 WHERE fingerprint=?1",
                    params![r.key, r.now, newest],
                )?;
            }
        }
        tx.execute(
            "INSERT INTO version_counts(fingerprint,version,os,count) VALUES(?1,?2,?3,1)
             ON CONFLICT(fingerprint,version,os) DO UPDATE SET count=count+1",
            params![r.key, r.version, r.os],
        )?;
        tx.execute(
            "INSERT INTO installs(hash,first_seen,last_seen,total) VALUES(?1,?2,?2,1)
             ON CONFLICT(hash) DO UPDATE SET last_seen=?2,total=total+1",
            params![r.install_hash, r.now],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn problem(&self, key: &str) -> anyhow::Result<Option<Problem>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT fingerprint,manual,issue_number,title,first_seen,last_seen,count,closed_in_version,
                        representative,max_version,synced_max_version
                 FROM problems WHERE fingerprint=?1",
                [key],
                row_to_problem,
            )
            .optional()?)
    }

    pub(crate) fn due(&self, now: i64) -> anyhow::Result<Vec<String>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT fingerprint FROM problems WHERE dirty=1 AND next_sync_at<=?1 ORDER BY first_seen LIMIT 25",
        )?;
        let rows = stmt.query_map([now], |row| row.get(0))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub(crate) fn versions(&self, key: &str) -> anyhow::Result<Vec<VersionCount>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT version,os,count,commented FROM version_counts WHERE fingerprint=?1",
        )?;
        let rows = stmt.query_map([key], |row| {
            Ok(VersionCount {
                version: row.get(0)?,
                os: row.get(1)?,
                count: row.get(2)?,
                commented: row.get::<_, i64>(3)? != 0,
            })
        })?;
        let mut all: Vec<VersionCount> = rows.collect::<Result<_, _>>()?;
        all.sort_by(|a, b| {
            crate::github::version_cmp(&b.version, &a.version).then_with(|| a.os.cmp(&b.os))
        });
        Ok(all)
    }

    pub(crate) fn last_body_hash(&self, key: &str) -> Option<String> {
        self.conn()
            .query_row(
                "SELECT last_body_hash FROM problems WHERE fingerprint=?1",
                [key],
                |row| row.get(0),
            )
            .ok()
            .flatten()
    }

    pub(crate) fn set_issue_number(&self, key: &str, number: u64) -> anyhow::Result<()> {
        self.conn().execute(
            "UPDATE problems SET issue_number=?2 WHERE fingerprint=?1",
            params![key, number as i64],
        )?;
        Ok(())
    }

    pub(crate) fn set_closed_in(&self, key: &str, version: Option<&str>) -> anyhow::Result<()> {
        self.conn().execute(
            "UPDATE problems SET closed_in_version=?2 WHERE fingerprint=?1",
            params![key, version],
        )?;
        Ok(())
    }

    pub(crate) fn mark_commented(&self, key: &str, version: &str) -> anyhow::Result<()> {
        self.conn().execute(
            "UPDATE version_counts SET commented=1 WHERE fingerprint=?1 AND version=?2",
            params![key, version],
        )?;
        Ok(())
    }

    /// The issue now reflects everything up to `max_version` and `body_hash`.
    pub(crate) fn mark_synced(
        &self,
        key: &str,
        max_version: &str,
        body_hash: &str,
        next_sync_at: i64,
        reopened_state: &str,
    ) -> anyhow::Result<()> {
        self.conn().execute(
            "UPDATE problems SET dirty=0,attempts=0,synced_max_version=?2,last_body_hash=?3,
                    next_sync_at=?4,state=?5 WHERE fingerprint=?1",
            params![key, max_version, body_hash, next_sync_at, reopened_state],
        )?;
        Ok(())
    }

    /// A report arrived between the snapshot and the write-back: keep the problem dirty.
    pub(crate) fn still_dirty_since(&self, key: &str, count: i64) -> bool {
        self.conn()
            .query_row(
                "SELECT count>?2 FROM problems WHERE fingerprint=?1",
                params![key, count],
                |row| row.get::<_, bool>(0),
            )
            .unwrap_or(false)
    }

    pub(crate) fn redirty(&self, key: &str) {
        let _ = self
            .conn()
            .execute("UPDATE problems SET dirty=1 WHERE fingerprint=?1", [key]);
    }

    pub(crate) fn mark_failed(&self, key: &str, now: i64) -> anyhow::Result<()> {
        let conn = self.conn();
        let attempts: i64 = conn.query_row(
            "SELECT attempts FROM problems WHERE fingerprint=?1",
            [key],
            |row| row.get(0),
        )?;
        let wait = (30_i64 << attempts.min(7)).min(3600);
        conn.execute(
            "UPDATE problems SET attempts=attempts+1,next_sync_at=?2 WHERE fingerprint=?1",
            params![key, now + wait],
        )?;
        Ok(())
    }
}

fn row_to_problem(row: &rusqlite::Row<'_>) -> rusqlite::Result<Problem> {
    Ok(Problem {
        fingerprint: row.get(0)?,
        manual: row.get::<_, i64>(1)? != 0,
        issue_number: row.get::<_, Option<i64>>(2)?.map(|n| n as u64),
        title: row.get(3)?,
        first_seen: row.get(4)?,
        last_seen: row.get(5)?,
        count: row.get(6)?,
        closed_in_version: row.get(7)?,
        representative: row.get(8)?,
        max_version: row.get(9)?,
        synced_max_version: row.get(10)?,
    })
}
