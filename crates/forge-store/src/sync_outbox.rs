//! Outbox backlog accounting and recovery for journal rows the service can never accept as sealed.

use super::*;

/// What the outbox holds, across every record kind (not only the account-sync ones).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SyncOutboxBacklog {
    pub pending: usize,
    /// `created_at` (epoch seconds) of the oldest row still waiting, which is the row blocking the
    /// strictly ordered drain.
    pub oldest_created_at: Option<i64>,
    pub oldest_id: Option<i64>,
}

/// Result of [`Store::rejournal_sync_entry`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RejournalOutcome {
    /// The entry was retired and its payload journaled again at this new journal id.
    Rejournaled { new_id: i64, revision: u64 },
    /// A newer revision of the same record already exists, so the entry was only retired.
    Superseded,
    /// The entry was already uploaded or no longer exists.
    Gone,
}

impl Store {
    pub fn sync_outbox_backlog(&self) -> Result<SyncOutboxBacklog> {
        let conn = self.lock()?;
        let (pending, oldest_id, oldest_created_at) = conn.query_row(
            "SELECT COUNT(*), MIN(id), MIN(created_at) FROM sync_journal WHERE uploaded_at IS NULL",
            [],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, Option<i64>>(1)?,
                    row.get::<_, Option<i64>>(2)?,
                ))
            },
        )?;
        Ok(SyncOutboxBacklog {
            pending: pending.max(0) as usize,
            oldest_created_at,
            oldest_id,
        })
    }

    /// Forget the sealed envelope of a row that has not uploaded, so the next attempt seals afresh.
    pub fn discard_sync_upload_envelope(&self, journal_id: i64) -> Result<bool> {
        Ok(self.lock()?.execute(
            "DELETE FROM anywhere_sync_upload WHERE journal_id = ?1",
            [journal_id],
        )? > 0)
    }

    /// Retire a row the service has poisoned (its revision holds different or unusable bytes) and
    /// journal the same payload again at the record's next revision, in one transaction. The
    /// service treats a reserved (kind, id, revision) as immutable, so a new revision is the only
    /// way the same content can reach it. Nothing is lost (the payload is carried over) and nothing
    /// is duplicated (the retired row never uploads).
    pub fn rejournal_sync_entry(&self, journal_id: i64, now_secs: i64) -> Result<RejournalOutcome> {
        let mut conn = self.lock()?;
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let row = transaction
            .query_row(
                "SELECT record_kind, stable_id, operation, revision, logical_clock, base_hash,
                        payload
                 FROM sync_journal WHERE id = ?1 AND uploaded_at IS NULL",
                [journal_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, Option<Vec<u8>>>(5)?,
                        row.get::<_, Vec<u8>>(6)?,
                    ))
                },
            )
            .optional()?;
        let Some((kind, stable_id, operation, revision, clock, base_hash, payload)) = row else {
            transaction.commit()?;
            return Ok(RejournalOutcome::Gone);
        };
        let (max_revision, max_clock): (i64, i64) = transaction.query_row(
            "SELECT COALESCE(MAX(revision), 0), COALESCE(MAX(logical_clock), 0)
             FROM sync_journal WHERE record_kind = ?1 AND stable_id = ?2",
            (&kind, &stable_id),
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let outcome = if max_revision > revision {
            RejournalOutcome::Superseded
        } else {
            let content_hash = sync_payload_hash(&payload);
            transaction.execute(
                "INSERT INTO sync_journal
                     (record_kind, stable_id, operation, revision, logical_clock, base_hash,
                      content_hash, payload)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                rusqlite::params![
                    kind,
                    stable_id,
                    operation,
                    max_revision + 1,
                    max_clock.max(clock) + 1,
                    base_hash,
                    content_hash.as_slice(),
                    payload,
                ],
            )?;
            RejournalOutcome::Rejournaled {
                new_id: transaction.last_insert_rowid(),
                revision: (max_revision + 1) as u64,
            }
        };
        transaction.execute(
            "UPDATE sync_journal SET uploaded_at = ?1 WHERE id = ?2",
            (now_secs, journal_id),
        )?;
        transaction.execute(
            "DELETE FROM anywhere_sync_upload WHERE journal_id = ?1",
            [journal_id],
        )?;
        transaction.commit()?;
        Ok(outcome)
    }
}
