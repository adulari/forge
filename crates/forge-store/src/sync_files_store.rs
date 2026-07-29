//! Focused persistence operations.

use super::*;

impl Store {
    /// Apply staged file records into the content-addressed local cache.
    ///
    /// A logical file id is never opened as a path. If the authenticated base does not match the
    /// local winner, the incoming bytes become a durable conflict copy and the winner is untouched.
    pub fn apply_staged_file_records(
        &self,
        local_device_id: [u8; 16],
        limit: usize,
    ) -> Result<RemoteSyncApplySummary> {
        if limit == 0 {
            return Ok(RemoteSyncApplySummary::default());
        }
        let limit = i64::try_from(limit).map_err(|_| {
            StoreError::InvalidValue("sync apply limit exceeds SQLite range".into())
        })?;
        let mut conn = self.lock()?;
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let raw = {
            let mut statement = transaction.prepare(
                "SELECT r.cursor, r.sender_device_id, r.stable_id, r.operation,
                        r.logical_clock, r.base_hash, r.content_hash, r.payload
                 FROM anywhere_sync_remote r
                 LEFT JOIN anywhere_sync_apply a ON a.cursor = r.cursor
                 WHERE r.record_kind = 'file' AND a.cursor IS NULL
                 ORDER BY r.cursor LIMIT ?1",
            )?;
            let records = statement
                .query_map([limit], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, Vec<u8>>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, Option<Vec<u8>>>(5)?,
                        row.get::<_, Vec<u8>>(6)?,
                        row.get::<_, Vec<u8>>(7)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            records
        };
        let mut summary = RemoteSyncApplySummary::default();
        for (cursor, sender, stable_id, operation, clock, base, hash, payload) in raw {
            summary.inspected += 1;
            let record = StagedHistoryRecord {
                cursor,
                sender_device_id: sender.try_into().map_err(|_| {
                    StoreError::InvalidValue("staged sync sender id has the wrong length".into())
                })?,
                record_kind: "file".into(),
                stable_id,
                operation,
                logical_clock: clock,
                content_hash: hash.try_into().map_err(|_| {
                    StoreError::InvalidValue("staged sync content hash has the wrong length".into())
                })?,
                payload,
            };
            let base_hash: Option<[u8; 32]> = base
                .map(|value| {
                    value.try_into().map_err(|_| {
                        StoreError::InvalidValue(
                            "staged file base hash has the wrong length".into(),
                        )
                    })
                })
                .transpose()?;
            if sync_payload_hash(&record.payload) != record.content_hash
                || (record.operation == "upsert" && base_hash.is_none())
                || (record.operation == "tombstone" && !record.payload.is_empty())
                || !matches!(record.operation.as_str(), "upsert" | "tombstone")
            {
                record_sync_apply_outcome(
                    &transaction,
                    record.cursor,
                    "conflict",
                    Some("file record payload, base hash, or operation is invalid"),
                )?;
                summary.conflicts += 1;
                continue;
            }
            let current = transaction
                .query_row(
                    "SELECT deleted, content_hash FROM anywhere_sync_file WHERE stable_id = ?1",
                    [&record.stable_id],
                    |row| Ok((row.get::<_, bool>(0)?, row.get::<_, Vec<u8>>(1)?)),
                )
                .optional()?;
            if let Some((_, current_hash)) = &current {
                if current_hash.as_slice() == record.content_hash {
                    record_sync_apply_outcome(
                        &transaction,
                        record.cursor,
                        "superseded",
                        Some("the file content already exists locally"),
                    )?;
                    summary.superseded += 1;
                    continue;
                }
            }
            if record.operation == "upsert" {
                if let Some((false, current_hash)) = &current {
                    if base_hash
                        .as_ref()
                        .is_some_and(|base| base.as_slice() != current_hash)
                    {
                        transaction.execute(
                            "INSERT INTO anywhere_sync_file_conflict
                             (stable_id, sender_device_id, base_hash, content_hash, payload, detail)
                             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                             ON CONFLICT(stable_id, content_hash) DO NOTHING",
                            rusqlite::params![
                                &record.stable_id,
                                record.sender_device_id.as_slice(),
                                base_hash.as_ref().map(|hash| hash.as_slice()),
                                record.content_hash.as_slice(),
                                &record.payload,
                                "incoming file base differs from the local winner",
                            ],
                        )?;
                        record_sync_apply_outcome(
                            &transaction,
                            record.cursor,
                            "conflict",
                            Some("incoming file base differs from the local winner"),
                        )?;
                        summary.conflicts += 1;
                        continue;
                    }
                }
            }
            match classify_mutable_sync_version(&transaction, &record, local_device_id)?.0 {
                Some(SyncVersionDisposition::Conflict) => {
                    record_sync_apply_outcome(
                        &transaction,
                        record.cursor,
                        "conflict",
                        Some("equal file versions contain different content"),
                    )?;
                    summary.conflicts += 1;
                    continue;
                }
                Some(SyncVersionDisposition::Superseded) => {
                    record_sync_apply_outcome(
                        &transaction,
                        record.cursor,
                        "superseded",
                        Some("a deterministic newer file revision already exists"),
                    )?;
                    summary.superseded += 1;
                    continue;
                }
                None => {}
            }
            transaction.execute(
                "INSERT INTO anywhere_sync_file
                 (stable_id, payload, deleted, logical_clock, sender_device_id,
                  base_hash, content_hash)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(stable_id) DO UPDATE SET
                   payload = excluded.payload, deleted = excluded.deleted,
                   logical_clock = excluded.logical_clock,
                   sender_device_id = excluded.sender_device_id,
                   base_hash = excluded.base_hash, content_hash = excluded.content_hash",
                rusqlite::params![
                    &record.stable_id,
                    &record.payload,
                    record.operation == "tombstone",
                    record.logical_clock,
                    record.sender_device_id.as_slice(),
                    base_hash.as_ref().map(|hash| hash.as_slice()),
                    record.content_hash.as_slice(),
                ],
            )?;
            upsert_sync_materialized(&transaction, &record)?;
            record_sync_apply_outcome(&transaction, record.cursor, "applied", None)?;
            summary.applied += 1;
        }
        transaction.commit()?;
        Ok(summary)
    }

    /// Atomically update one local logical file and enqueue its authenticated base revision.
    pub fn write_sync_file(
        &self,
        local_device_id: [u8; 16],
        stable_id: &str,
        expected_base: [u8; 32],
        payload: Option<&[u8]>,
    ) -> Result<()> {
        if stable_id.trim().is_empty() {
            return Err(StoreError::InvalidValue("file stable id is empty".into()));
        }
        let operation = if payload.is_some() {
            SyncJournalOperation::Upsert
        } else {
            SyncJournalOperation::Tombstone
        };
        let payload = payload.unwrap_or_default();
        let mut conn = self.lock()?;
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let sync_enabled = transaction.query_row(
            "SELECT enabled FROM anywhere_sync_state WHERE singleton = 1",
            [],
            |row| row.get::<_, bool>(0),
        )?;
        let current = transaction
            .query_row(
                "SELECT deleted, content_hash FROM anywhere_sync_file WHERE stable_id = ?1",
                [stable_id],
                |row| Ok((row.get::<_, bool>(0)?, row.get::<_, Vec<u8>>(1)?)),
            )
            .optional()?;
        let empty_hash: [u8; 32] = sync_payload_hash(&[]);
        let actual_base = current
            .as_ref()
            .filter(|(deleted, _)| !deleted)
            .map_or(empty_hash.as_slice(), |(_, hash)| hash.as_slice());
        if actual_base != expected_base {
            return Err(StoreError::InvalidValue(
                "file changed since the supplied base revision".into(),
            ));
        }
        let next: i64 = transaction.query_row(
            "SELECT MAX(value) + 1 FROM (
               SELECT COALESCE(MAX(revision), 0) AS value FROM sync_journal
                WHERE record_kind = 'file' AND stable_id = ?1
               UNION ALL
               SELECT COALESCE(MAX(logical_clock), 0) AS value
                FROM anywhere_sync_file WHERE stable_id = ?1
             )",
            [stable_id],
            |row| row.get(0),
        )?;
        let content_hash = sync_payload_hash(payload);
        transaction.execute(
            "INSERT INTO anywhere_sync_file
             (stable_id, payload, deleted, logical_clock, sender_device_id,
              base_hash, content_hash)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(stable_id) DO UPDATE SET
               payload = excluded.payload, deleted = excluded.deleted,
               logical_clock = excluded.logical_clock,
               sender_device_id = excluded.sender_device_id,
               base_hash = excluded.base_hash, content_hash = excluded.content_hash",
            rusqlite::params![
                stable_id,
                payload,
                operation == SyncJournalOperation::Tombstone,
                next,
                local_device_id.as_slice(),
                expected_base.as_slice(),
                content_hash.as_slice(),
            ],
        )?;
        if sync_enabled {
            insert_sync_journal_row_with_base(
                &transaction,
                "file",
                stable_id,
                operation,
                next,
                next,
                Some(&expected_base),
                payload,
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    /// Load the current logical file payload; tombstones return `None`.
    pub fn sync_file(&self, stable_id: &str) -> Result<Option<Vec<u8>>> {
        self.lock()?
            .query_row(
                "SELECT payload FROM anywhere_sync_file
                 WHERE stable_id = ?1 AND deleted = 0",
                [stable_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(StoreError::from)
    }

    /// List durable file conflict copies for explicit user resolution.
    pub fn sync_file_conflicts(&self, stable_id: &str) -> Result<Vec<SyncFileConflict>> {
        let conn = self.lock()?;
        let mut statement = conn.prepare(
            "SELECT sender_device_id, base_hash, content_hash, payload, detail
             FROM anywhere_sync_file_conflict WHERE stable_id = ?1 ORDER BY id",
        )?;
        let conflicts = statement
            .query_map([stable_id], |row| {
                let sender: Vec<u8> = row.get(0)?;
                let base: Option<Vec<u8>> = row.get(1)?;
                let hash: Vec<u8> = row.get(2)?;
                Ok(SyncFileConflict {
                    stable_id: stable_id.to_owned(),
                    sender_device_id: sender.try_into().map_err(|_| {
                        rusqlite::Error::FromSqlConversionFailure(
                            0,
                            rusqlite::types::Type::Blob,
                            "file conflict sender id is not 16 bytes".into(),
                        )
                    })?,
                    base_hash: base
                        .map(|value| {
                            value.try_into().map_err(|_| {
                                rusqlite::Error::FromSqlConversionFailure(
                                    1,
                                    rusqlite::types::Type::Blob,
                                    "file conflict base hash is not 32 bytes".into(),
                                )
                            })
                        })
                        .transpose()?,
                    content_hash: hash.try_into().map_err(|_| {
                        rusqlite::Error::FromSqlConversionFailure(
                            2,
                            rusqlite::types::Type::Blob,
                            "file conflict content hash is not 32 bytes".into(),
                        )
                    })?,
                    payload: row.get(3)?,
                    detail: row.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(conflicts)
    }

    /// List durable terminal conflicts without exposing synchronized payload content.
    pub fn sync_apply_conflicts(&self, limit: usize) -> Result<Vec<SyncApplyConflict>> {
        let limit = i64::try_from(limit.clamp(1, 1000)).unwrap_or(1000);
        let conn = self.lock()?;
        let mut statement = conn.prepare(
            "SELECT a.cursor, r.record_kind, r.stable_id, COALESCE(a.detail, 'sync conflict')
             FROM anywhere_sync_apply a
             JOIN anywhere_sync_remote r ON r.cursor = a.cursor
             WHERE a.state = 'conflict' ORDER BY a.cursor DESC LIMIT ?1",
        )?;
        let conflicts = statement
            .query_map([limit], |row| {
                Ok(SyncApplyConflict {
                    cursor: row.get(0)?,
                    record_kind: row.get(1)?,
                    stable_id: row.get(2)?,
                    detail: row.get(3)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(conflicts)
    }

    /// Atomically mark acknowledged journal rows uploaded. Unknown IDs are harmless.
    pub fn mark_sync_journal_uploaded(&self, ids: &[i64], uploaded_at: i64) -> Result<usize> {
        let mut conn = self.lock()?;
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut changed = 0;
        for id in ids {
            changed += transaction.execute(
                "UPDATE sync_journal SET uploaded_at = ?1
                 WHERE id = ?2 AND uploaded_at IS NULL",
                (uploaded_at, id),
            )?;
            transaction.execute(
                "DELETE FROM anywhere_sync_upload
                 WHERE journal_id = ?1
                   AND EXISTS (SELECT 1 FROM sync_journal
                               WHERE id = ?1 AND uploaded_at IS NOT NULL)",
                [id],
            )?;
        }
        transaction.commit()?;
        Ok(changed)
    }
}
