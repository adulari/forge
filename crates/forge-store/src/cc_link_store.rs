//! The Forge <-> Claude Code session link and the reads/writes a switch needs around it.
//!
//! See migration #40 for what a link row means. Everything that changes both the Forge rows and
//! the link happens in one transaction, so an interrupted switch leaves the link pointing at what
//! was really mirrored.

use super::session_import_store::{journal_enabled, upsert_compaction_summary};
use super::*;

/// One mirrored conversation: a Forge session and the Claude Code transcript that is its twin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CcLink {
    pub forge_session_id: String,
    pub cc_session_id: String,
    pub cc_path: String,
    /// Bytes of the Claude Code file already mirrored (always a line boundary).
    pub cc_synced_bytes: i64,
    /// The last Claude Code chain entry mirrored, in either direction.
    pub cc_last_uuid: Option<String>,
    /// The highest Forge message seq already mirrored, in either direction (-1 = none).
    pub forge_synced_seq: i64,
    /// The compaction summary already reflected on both sides.
    pub forge_summary: Option<String>,
    /// `claude` or `forge`: the side the last switch opened.
    pub last_target: Option<String>,
    pub updated_at: i64,
}

/// The Claude Code half of a link, as known when the file was last read or written.
#[derive(Debug, Clone, Copy)]
pub struct CcLinkUpdate<'a> {
    pub cc_session_id: &'a str,
    pub cc_path: &'a str,
    pub cc_synced_bytes: i64,
    pub cc_last_uuid: Option<&'a str>,
    pub last_target: Option<&'a str>,
}

/// One stored message, with the flags a transcript exporter needs.
#[derive(Debug, Clone)]
pub struct ExportRow {
    pub seq: i64,
    pub role: Role,
    pub content: String,
    pub model: Option<String>,
    pub tool_calls: Vec<ToolCall>,
    pub tool_call_id: Option<String>,
    pub visibility: Visibility,
    pub nudge: bool,
    pub active: bool,
    pub created_at: i64,
}

#[derive(Debug, Clone)]
pub struct SessionMeta {
    pub id: String,
    pub cwd: String,
    pub title: Option<String>,
    pub permission_mode: String,
    pub pinned_model: Option<String>,
    pub created_at: i64,
    pub last_activity: i64,
    /// Conversation turns (what the person typed and what came back), compacted ones included.
    pub message_count: i64,
    /// First thing the person typed, for sessions with no title.
    pub preview: Option<String>,
}

const LINK_COLUMNS: &str = "forge_session_id, cc_session_id, cc_path, cc_synced_bytes, \
     cc_last_uuid, forge_synced_seq, forge_summary, last_target, updated_at";

fn link_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<CcLink> {
    Ok(CcLink {
        forge_session_id: row.get(0)?,
        cc_session_id: row.get(1)?,
        cc_path: row.get(2)?,
        cc_synced_bytes: row.get(3)?,
        cc_last_uuid: row.get(4)?,
        forge_synced_seq: row.get(5)?,
        forge_summary: row.get(6)?,
        last_target: row.get(7)?,
        updated_at: row.get(8)?,
    })
}

/// Upsert a link inside a transaction, keeping the one-to-one invariant: any other row that
/// claims either id is dropped.
pub(crate) fn write_link_tx(tx: &rusqlite::Transaction<'_>, link: &CcLink) -> Result<()> {
    tx.execute(
        "DELETE FROM cc_session_link WHERE (cc_session_id = ?1 AND forge_session_id != ?2)",
        (&link.cc_session_id, &link.forge_session_id),
    )?;
    tx.execute(
        "INSERT INTO cc_session_link (forge_session_id, cc_session_id, cc_path, cc_synced_bytes,
             cc_last_uuid, forge_synced_seq, forge_summary, last_target, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, strftime('%s','now'))
         ON CONFLICT(forge_session_id) DO UPDATE SET
           cc_session_id = excluded.cc_session_id, cc_path = excluded.cc_path,
           cc_synced_bytes = excluded.cc_synced_bytes, cc_last_uuid = excluded.cc_last_uuid,
           forge_synced_seq = excluded.forge_synced_seq, forge_summary = excluded.forge_summary,
           last_target = excluded.last_target, updated_at = excluded.updated_at",
        rusqlite::params![
            link.forge_session_id,
            link.cc_session_id,
            link.cc_path,
            link.cc_synced_bytes,
            link.cc_last_uuid,
            link.forge_synced_seq,
            link.forge_summary,
            link.last_target,
        ],
    )?;
    Ok(())
}

impl Store {
    pub fn cc_link_for_forge(&self, forge_session_id: &str) -> Result<Option<CcLink>> {
        Ok(self
            .lock()?
            .query_row(
                &format!("SELECT {LINK_COLUMNS} FROM cc_session_link WHERE forge_session_id = ?1"),
                [forge_session_id],
                link_from_row,
            )
            .optional()?)
    }

    pub fn cc_link_for_cc(&self, cc_session_id: &str) -> Result<Option<CcLink>> {
        Ok(self
            .lock()?
            .query_row(
                &format!("SELECT {LINK_COLUMNS} FROM cc_session_link WHERE cc_session_id = ?1"),
                [cc_session_id],
                link_from_row,
            )
            .optional()?)
    }

    /// Create or replace a link (used after a Forge -> Claude Code export, which writes no Forge rows).
    pub fn write_cc_link(&self, link: &CcLink) -> Result<()> {
        let mut conn = self.lock()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        write_link_tx(&tx, link)?;
        tx.commit()?;
        Ok(())
    }

    pub fn delete_cc_link(&self, forge_session_id: &str) -> Result<()> {
        self.lock()?.execute(
            "DELETE FROM cc_session_link WHERE forge_session_id = ?1",
            [forge_session_id],
        )?;
        Ok(())
    }

    pub fn session_meta(&self, session_id: &str) -> Result<Option<SessionMeta>> {
        Ok(self
            .lock()?
            .query_row(
                "SELECT s.id, s.cwd, s.title, s.permission_mode, s.pinned_model, s.created_at,
                        COALESCE((SELECT MAX(m.created_at) FROM message m WHERE m.session_id = s.id),
                                 s.created_at),
                        (SELECT COUNT(*) FROM message m WHERE m.session_id = s.id
                           AND m.role IN ('user', 'assistant') AND m.visibility != 'ui' AND m.nudge = 0),
                        (SELECT substr(content, 1, 200) FROM message m WHERE m.session_id = s.id
                           AND m.role = 'user' AND m.active = 1 AND m.nudge = 0
                         ORDER BY m.seq LIMIT 1)
                 FROM session s WHERE s.id = ?1",
                [session_id],
                |row| {
                    Ok(SessionMeta {
                        id: row.get(0)?,
                        cwd: row.get(1)?,
                        title: row.get(2)?,
                        permission_mode: row.get(3)?,
                        pinned_model: row.get(4)?,
                        created_at: row.get(5)?,
                        last_activity: row.get(6)?,
                        message_count: row.get(7)?,
                        preview: row.get(8)?,
                    })
                },
            )
            .optional()?)
    }

    /// How many messages after `after_seq` the other side would be shown: active conversation
    /// rows, not Forge's own system notes, nudges or display-only copies.
    pub fn mirrorable_rows_after(&self, session_id: &str, after_seq: i64) -> Result<i64> {
        Ok(self.lock()?.query_row(
            "SELECT COUNT(*) FROM message
             WHERE session_id = ?1 AND seq > ?2 AND active = 1 AND nudge = 0
               AND role != 'system' AND visibility != 'ui'",
            (session_id, after_seq),
            |row| row.get(0),
        )?)
    }

    /// Highest message seq of a session, or -1 when it has none.
    pub fn session_max_seq(&self, session_id: &str) -> Result<i64> {
        Ok(self.lock()?.query_row(
            "SELECT COALESCE(MAX(seq), -1) FROM message WHERE session_id = ?1",
            [session_id],
            |row| row.get(0),
        )?)
    }

    /// The session's compaction summary and when it was stored.
    pub fn session_compaction_summary(&self, session_id: &str) -> Result<Option<(String, i64)>> {
        Ok(self
            .lock()?
            .query_row(
                "SELECT summary, created_at FROM session_compaction WHERE session_id = ?1",
                [session_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?)
    }

    /// Every message with `seq > after_seq`, oldest first, soft-deleted rows included (the
    /// caller decides what the other side should see).
    pub fn export_rows_after(&self, session_id: &str, after_seq: i64) -> Result<Vec<ExportRow>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT seq, role, content, model, tool_calls_json, tool_call_id, visibility, nudge,
                    active, created_at
             FROM message WHERE session_id = ?1 AND seq > ?2 ORDER BY seq",
        )?;
        let rows = stmt.query_map((session_id, after_seq), |row| {
            let role: String = row.get(1)?;
            let calls: Option<String> = row.get(4)?;
            let visibility: String = row.get(6)?;
            Ok(ExportRow {
                seq: row.get(0)?,
                role: Role::parse(&role).unwrap_or(Role::User),
                content: row.get(2)?,
                model: row.get(3)?,
                tool_calls: calls
                    .and_then(|c| serde_json::from_str(&c).ok())
                    .unwrap_or_default(),
                tool_call_id: row.get(5)?,
                visibility: Visibility::parse(&visibility),
                nudge: row.get::<_, i64>(7)? != 0,
                active: row.get::<_, i64>(8)? != 0,
                created_at: row.get(9)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    /// The most recently used top-level session working in `cwd` (or whose worktree is `cwd`).
    pub fn latest_session_for_cwd(&self, cwd: &str) -> Result<Option<SessionMeta>> {
        let id: Option<String> = self
            .lock()?
            .query_row(
                "SELECT s.id FROM session s
                 WHERE s.parent_session_id IS NULL AND s.archived = 0
                   AND (s.cwd = ?1 OR s.worktree_path = ?1)
                   AND EXISTS (SELECT 1 FROM message m WHERE m.session_id = s.id AND m.role = 'user')
                 ORDER BY COALESCE((SELECT MAX(m.created_at) FROM message m WHERE m.session_id = s.id),
                                   s.created_at) DESC, s.rowid DESC
                 LIMIT 1",
                [cwd],
                |row| row.get(0),
            )
            .optional()?;
        match id {
            Some(id) => self.session_meta(&id),
            None => Ok(None),
        }
    }

    /// Mirror new Claude Code rows into the linked Forge session in ONE transaction: optionally
    /// mark everything already there compacted (the Claude Code side compacted), push the rows
    /// through `fill`, store a returned compaction summary, and advance the link. Returns the
    /// updated link. Nothing is written if `fill` fails.
    pub fn mirror_into_session_with<F>(
        &self,
        link: &CcLink,
        update: &CcLinkUpdate<'_>,
        compact_existing: bool,
        fill: F,
    ) -> Result<CcLink>
    where
        F: FnOnce(&mut ImportSink<'_>) -> Result<Option<String>>,
    {
        let mut conn = self.lock()?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let id = link.forge_session_id.as_str();
        let max_seq: i64 = tx.query_row(
            "SELECT COALESCE(MAX(seq), -1) FROM message WHERE session_id = ?1",
            [id],
            |row| row.get(0),
        )?;
        if compact_existing {
            tx.execute(
                "UPDATE message SET active = 0, compacted = 1 WHERE session_id = ?1 AND active = 1",
                [id],
            )?;
        }
        let journal = journal_enabled(&tx)?;
        let mut sink = ImportSink::resume(&tx, id, max_seq + 1, journal);
        let summary = fill(&mut sink)?.filter(|s| !s.trim().is_empty());
        let (end_seq, last_ts) = (sink.next_seq(), sink.last_timestamp());
        if let Some(summary) = &summary {
            upsert_compaction_summary(&tx, id, summary)?;
        }
        if let Some(ts) = last_ts {
            tx.execute(
                "UPDATE session SET updated_at = MAX(updated_at, ?2) WHERE id = ?1",
                (id, ts),
            )?;
        }
        let next = CcLink {
            forge_session_id: link.forge_session_id.clone(),
            cc_session_id: update.cc_session_id.to_string(),
            cc_path: update.cc_path.to_string(),
            cc_synced_bytes: update.cc_synced_bytes,
            cc_last_uuid: update
                .cc_last_uuid
                .map(str::to_string)
                .or_else(|| link.cc_last_uuid.clone()),
            forge_synced_seq: if end_seq > max_seq + 1 {
                end_seq - 1
            } else {
                link.forge_synced_seq
            },
            forge_summary: summary.or_else(|| link.forge_summary.clone()),
            last_target: update.last_target.map(str::to_string),
            updated_at: 0,
        };
        write_link_tx(&tx, &next)?;
        append_session_snapshot(&tx, id)?;
        tx.commit()?;
        Ok(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(content: &str, active: bool) -> ImportedRow {
        ImportedRow {
            role: Role::User,
            content: content.into(),
            model: None,
            tool_calls: vec![],
            tool_call_id: None,
            active,
            created_at: Some(1_700_000_000),
        }
    }

    fn linked(store: &Store) -> (String, CcLink) {
        let meta = ImportedSessionMeta {
            cwd: "/w",
            mode: "Default",
            title: None,
            pinned_model: None,
        };
        let update = CcLinkUpdate {
            cc_session_id: "cc-1",
            cc_path: "/p/cc-1.jsonl",
            cc_synced_bytes: 10,
            cc_last_uuid: Some("u1"),
            last_target: None,
        };
        let id = store
            .import_session_linked_with(&meta, Some(&update), |sink| {
                sink.push(row("hello", true))?;
                Ok(None)
            })
            .unwrap();
        let link = store.cc_link_for_forge(&id).unwrap().unwrap();
        (id, link)
    }

    #[test]
    fn an_import_records_its_link_in_the_same_transaction() {
        let store = Store::open_in_memory().unwrap();
        let (id, link) = linked(&store);
        assert_eq!(link.cc_session_id, "cc-1");
        assert_eq!(link.cc_synced_bytes, 10);
        assert_eq!(link.forge_synced_seq, 0);
        assert_eq!(
            store
                .cc_link_for_cc("cc-1")
                .unwrap()
                .unwrap()
                .forge_session_id,
            id
        );
    }

    #[test]
    fn a_failed_mirror_leaves_rows_and_link_untouched() {
        let store = Store::open_in_memory().unwrap();
        let (id, link) = linked(&store);
        let update = CcLinkUpdate {
            cc_session_id: "cc-1",
            cc_path: "/p/cc-1.jsonl",
            cc_synced_bytes: 99,
            cc_last_uuid: Some("u2"),
            last_target: Some("forge"),
        };
        let err = store.mirror_into_session_with(&link, &update, true, |sink| {
            sink.push(row("new", true))?;
            Err(StoreError::InvalidValue("boom".into()))
        });
        assert!(err.is_err());
        assert_eq!(store.cc_link_for_forge(&id).unwrap().unwrap(), link);
        let rows = store.export_rows_after(&id, -1).unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].active, "the failed compaction was rolled back too");
    }

    #[test]
    fn a_mirror_appends_after_the_last_row_and_advances_the_link() {
        let store = Store::open_in_memory().unwrap();
        let (id, link) = linked(&store);
        let update = CcLinkUpdate {
            cc_session_id: "cc-1",
            cc_path: "/p/cc-1.jsonl",
            cc_synced_bytes: 50,
            cc_last_uuid: Some("u2"),
            last_target: Some("forge"),
        };
        let next = store
            .mirror_into_session_with(&link, &update, false, |sink| {
                sink.push(row("a", true))?;
                sink.push(row("b", true))?;
                Ok(None)
            })
            .unwrap();
        assert_eq!((next.forge_synced_seq, next.cc_synced_bytes), (2, 50));
        assert_eq!(next.cc_last_uuid.as_deref(), Some("u2"));
        let seqs: Vec<i64> = store
            .export_rows_after(&id, -1)
            .unwrap()
            .iter()
            .map(|r| r.seq)
            .collect();
        assert_eq!(seqs, [0, 1, 2]);
        // No rows pushed: the Forge side of the link does not move.
        let same = store
            .mirror_into_session_with(&next, &update, false, |_| Ok(None))
            .unwrap();
        assert_eq!(same.forge_synced_seq, 2);
    }

    #[test]
    fn mirrorable_rows_ignore_forge_notes_and_nudges() {
        let store = Store::open_in_memory().unwrap();
        let (id, _) = linked(&store);
        store
            .add_message(&id, 1, Role::System, "[time] now", None)
            .unwrap();
        store
            .add_ui_note(&id, 2, Role::Assistant, "display copy")
            .unwrap();
        store.add_nudge_message(&id, 3, "keep going").unwrap();
        assert_eq!(store.mirrorable_rows_after(&id, 0).unwrap(), 0);
        store.add_message(&id, 4, Role::User, "real", None).unwrap();
        assert_eq!(store.mirrorable_rows_after(&id, 0).unwrap(), 1);
    }
}
