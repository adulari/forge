//! Durable encrypted Anywhere sync journal and remote staging.
//!
//! This owner keeps cursor advancement and idempotent journal/envelope writes transactional.

use super::*;

impl Store {
    /// Enable or disable creation of new local Anywhere sync snapshots.
    ///
    /// Existing pending rows are retained when disabled so logout/service outages cannot destroy
    /// unsynchronized history. Ordinary Forge installs default to disabled and incur no duplicate
    /// payload writes.
    pub fn set_sync_journal_enabled(&self, enabled: bool) -> Result<()> {
        self.lock()?.execute(
            "UPDATE anywhere_sync_state SET enabled = ?1 WHERE singleton = 1",
            [enabled],
        )?;
        Ok(())
    }

    /// Idempotently append a record to the durable encrypted-sync outbox.
    ///
    /// Callers that own a larger store transaction should add the journal row in that same
    /// transaction; this public boundary is for already-committed records and import workers.
    pub fn append_sync_journal(
        &self,
        record_kind: &str,
        stable_id: &str,
        operation: SyncJournalOperation,
        revision: u64,
        logical_clock: u64,
        payload: &[u8],
    ) -> Result<bool> {
        let revision = i64::try_from(revision)
            .map_err(|_| StoreError::InvalidValue("sync revision exceeds SQLite range".into()))?;
        let logical_clock = i64::try_from(logical_clock).map_err(|_| {
            StoreError::InvalidValue("sync logical clock exceeds SQLite range".into())
        })?;
        let conn = self.lock()?;
        insert_sync_journal_row(
            &conn,
            record_kind,
            stable_id,
            operation,
            revision,
            logical_clock,
            payload,
        )
    }

    /// Append a file revision with its explicit base content hash.
    ///
    /// The base is authenticated in the encrypted record and is required for remote divergence
    /// detection. `stable_id` is a logical id and is never interpreted as a host filesystem path.
    pub fn append_sync_file_journal(
        &self,
        stable_id: &str,
        operation: SyncJournalOperation,
        revision: u64,
        logical_clock: u64,
        base_hash: Option<[u8; 32]>,
        payload: &[u8],
    ) -> Result<bool> {
        if operation == SyncJournalOperation::Upsert && base_hash.is_none() {
            return Err(StoreError::InvalidValue(
                "file upserts require a base content hash".into(),
            ));
        }
        let revision = i64::try_from(revision)
            .map_err(|_| StoreError::InvalidValue("sync revision exceeds SQLite range".into()))?;
        let logical_clock = i64::try_from(logical_clock).map_err(|_| {
            StoreError::InvalidValue("sync logical clock exceeds SQLite range".into())
        })?;
        let conn = self.lock()?;
        insert_sync_journal_row_with_base(
            &conn,
            "file",
            stable_id,
            operation,
            revision,
            logical_clock,
            base_hash.as_ref(),
            payload,
        )
    }

    /// Return pending sync records in durable cursor order.
    pub fn pending_sync_journal(&self, limit: usize) -> Result<Vec<SyncJournalEntry>> {
        let limit = i64::try_from(limit.clamp(1, 1000)).unwrap_or(1000);
        let conn = self.lock()?;
        let mut statement = conn.prepare(
            "SELECT id, record_kind, stable_id, operation, revision, logical_clock,
                    base_hash, content_hash, payload, created_at
             FROM sync_journal WHERE uploaded_at IS NULL ORDER BY id LIMIT ?1",
        )?;
        let rows = statement.query_map([limit], |row| {
            Ok(SyncJournalEntry {
                id: row.get(0)?,
                record_kind: row.get(1)?,
                stable_id: row.get(2)?,
                operation: row.get(3)?,
                revision: row.get::<_, i64>(4)?.max(0) as u64,
                logical_clock: row.get::<_, i64>(5)?.max(0) as u64,
                base_hash: row
                    .get::<_, Option<Vec<u8>>>(6)?
                    .map(|hash| {
                        hash.try_into().map_err(|_| {
                            rusqlite::Error::FromSqlConversionFailure(
                                6,
                                rusqlite::types::Type::Blob,
                                "sync base hash is not 32 bytes".into(),
                            )
                        })
                    })
                    .transpose()?,
                content_hash: row.get(7)?,
                payload: row.get(8)?,
                created_at: row.get(9)?,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(StoreError::from)
    }

    /// Return previously sealed ciphertext for a journal row, if upload preparation completed.
    pub fn sync_upload_envelope(&self, journal_id: i64) -> Result<Option<SyncUploadEnvelope>> {
        self.lock()?
            .query_row(
                "SELECT envelope, ciphertext_sha256 FROM anywhere_sync_upload
                 WHERE journal_id = ?1",
                [journal_id],
                |row| {
                    let hash: Vec<u8> = row.get(1)?;
                    let ciphertext_sha256 = hash.try_into().map_err(|_| {
                        rusqlite::Error::InvalidColumnType(
                            1,
                            "ciphertext_sha256".into(),
                            rusqlite::types::Type::Blob,
                        )
                    })?;
                    Ok(SyncUploadEnvelope {
                        envelope: row.get(0)?,
                        ciphertext_sha256,
                    })
                },
            )
            .optional()
            .map_err(StoreError::from)
    }

    /// Persist ciphertext once and return the authoritative bytes if another worker won the race.
    pub fn store_sync_upload_envelope(
        &self,
        journal_id: i64,
        envelope: &[u8],
        ciphertext_sha256: [u8; 32],
    ) -> Result<SyncUploadEnvelope> {
        let mut conn = self.lock()?;
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            "INSERT INTO anywhere_sync_upload (journal_id, envelope, ciphertext_sha256)
             VALUES (?1, ?2, ?3) ON CONFLICT(journal_id) DO NOTHING",
            (journal_id, envelope, ciphertext_sha256.as_slice()),
        )?;
        let stored = transaction.query_row(
            "SELECT envelope, ciphertext_sha256 FROM anywhere_sync_upload WHERE journal_id = ?1",
            [journal_id],
            |row| {
                let hash: Vec<u8> = row.get(1)?;
                let ciphertext_sha256 = hash.try_into().map_err(|_| {
                    rusqlite::Error::InvalidColumnType(
                        1,
                        "ciphertext_sha256".into(),
                        rusqlite::types::Type::Blob,
                    )
                })?;
                Ok(SyncUploadEnvelope {
                    envelope: row.get(0)?,
                    ciphertext_sha256,
                })
            },
        )?;
        transaction.commit()?;
        Ok(stored)
    }

    /// Last service cursor durably staged by the remote sync worker.
    pub fn sync_download_cursor(&self) -> Result<i64> {
        self.lock()?
            .query_row(
                "SELECT cursor FROM anywhere_sync_cursor WHERE singleton = 1",
                [],
                |row| row.get(0),
            )
            .map_err(StoreError::from)
    }

    /// Advance past service-side changes whose ciphertext was deliberately deleted.
    pub fn advance_sync_download_cursor(&self, cursor: i64) -> Result<()> {
        if cursor < 0 {
            return Err(StoreError::InvalidValue(
                "sync cursor must not be negative".into(),
            ));
        }
        self.lock()?.execute(
            "UPDATE anywhere_sync_cursor SET cursor = MAX(cursor, ?1) WHERE singleton = 1",
            [cursor],
        )?;
        Ok(())
    }

    /// Stage one verified remote record and advance its cursor in the same transaction.
    pub fn stage_remote_sync_record(&self, record: &RemoteSyncRecord) -> Result<bool> {
        if record.cursor <= 0
            || record.record_kind.trim().is_empty()
            || record.stable_id.trim().is_empty()
            || !matches!(record.operation.as_str(), "upsert" | "tombstone")
        {
            return Err(StoreError::InvalidValue(
                "remote sync record metadata is invalid".into(),
            ));
        }
        let revision = i64::try_from(record.revision)
            .map_err(|_| StoreError::InvalidValue("sync revision exceeds SQLite range".into()))?;
        let logical_clock = i64::try_from(record.logical_clock).map_err(|_| {
            StoreError::InvalidValue("sync logical clock exceeds SQLite range".into())
        })?;
        let mut conn = self.lock()?;
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current: i64 = transaction.query_row(
            "SELECT cursor FROM anywhere_sync_cursor WHERE singleton = 1",
            [],
            |row| row.get(0),
        )?;
        if record.cursor <= current {
            transaction.commit()?;
            return Ok(false);
        }
        let existing = transaction
            .query_row(
                "SELECT sender_device_id, operation, logical_clock, base_hash, content_hash, payload
                 FROM anywhere_sync_remote
                 WHERE record_kind = ?1 AND stable_id = ?2 AND revision = ?3",
                (&record.record_kind, &record.stable_id, revision),
                |row| {
                    Ok((
                        row.get::<_, Vec<u8>>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, Option<Vec<u8>>>(3)?,
                        row.get::<_, Vec<u8>>(4)?,
                        row.get::<_, Vec<u8>>(5)?,
                    ))
                },
            )
            .optional()?;
        if let Some(existing) = existing {
            let same = existing.0 == record.sender_device_id
                && existing.1 == record.operation
                && existing.2 == logical_clock
                && existing.3.as_deref() == record.base_hash.as_ref().map(|hash| hash.as_slice())
                && existing.4 == record.content_hash
                && existing.5 == record.payload;
            if !same {
                return Err(StoreError::InvalidValue(
                    "remote sync revision conflicts with staged content".into(),
                ));
            }
        } else {
            transaction.execute(
                "INSERT INTO anywhere_sync_remote
                 (cursor, sender_device_id, record_kind, stable_id, operation, revision,
                  logical_clock, base_hash, content_hash, payload)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                rusqlite::params![
                    record.cursor,
                    record.sender_device_id.as_slice(),
                    &record.record_kind,
                    &record.stable_id,
                    &record.operation,
                    revision,
                    logical_clock,
                    record.base_hash.as_ref().map(|hash| hash.as_slice()),
                    record.content_hash.as_slice(),
                    &record.payload
                ],
            )?;
        }
        transaction.execute(
            "UPDATE anywhere_sync_cursor SET cursor = ?1 WHERE singleton = 1",
            [record.cursor],
        )?;
        transaction.commit()?;
        Ok(true)
    }

    /// Remove bounded encrypted-sync staging data that no longer owns recovery state.
    ///
    /// Local rows are eligible only after the service acknowledged them, and the newest
    /// revision for every `(record_kind, stable_id)` is retained so revision clocks and
    /// idempotent retries remain anchored. Remote rows are eligible only after materialization
    /// ended in `applied` or `superseded`; unresolved conflicts remain available to the UI.
    /// The durable download cursor and materialized/domain rows are independent of this staging
    /// data, so deleting a terminal remote row cannot replay or roll back a change.
    pub fn prune_terminal_sync_rows(&self, max_rows: usize) -> Result<SyncPruneSummary> {
        if max_rows == 0 {
            return Ok(SyncPruneSummary::default());
        }
        let limit = i64::try_from(max_rows).map_err(|_| {
            StoreError::InvalidValue("sync prune limit exceeds SQLite range".into())
        })?;
        let mut conn = self.lock()?;
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;

        let local_revisions = transaction.execute(
            "WITH doomed AS (
                 SELECT older.id
                 FROM sync_journal older
                 WHERE older.uploaded_at IS NOT NULL
                   AND EXISTS (
                       SELECT 1
                       FROM sync_journal newer
                       WHERE newer.record_kind = older.record_kind
                         AND newer.stable_id = older.stable_id
                         AND newer.revision > older.revision
                   )
                 ORDER BY older.id
                 LIMIT ?1
             )
             DELETE FROM sync_journal WHERE id IN (SELECT id FROM doomed)",
            [limit],
        )?;

        let remote_records = transaction.execute(
            "WITH doomed AS (
                 SELECT remote.cursor
                 FROM anywhere_sync_remote remote
                 JOIN anywhere_sync_apply apply ON apply.cursor = remote.cursor
                 WHERE apply.state IN ('applied', 'superseded')
                 ORDER BY remote.cursor
                 LIMIT ?1
             )
             DELETE FROM anywhere_sync_remote WHERE cursor IN (SELECT cursor FROM doomed)",
            [limit],
        )?;

        transaction.commit()?;
        Ok(SyncPruneSummary {
            local_revisions,
            remote_records,
        })
    }
}
