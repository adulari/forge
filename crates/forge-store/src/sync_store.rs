//! Store operations for Anywhere synchronization.

use super::*;

impl Store {
    /// Apply staged memory records using deterministic `(logical_clock, device_id)` ordering.
    ///
    /// Memory embeddings remain device-local: remote upserts preserve an existing embedding and
    /// never import one. Records without safe local sync provenance become durable conflicts rather
    /// than overwriting pre-Anywhere data. Other record kinds remain staged for their own policy.
    pub fn apply_staged_memory_records(
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
        let raw_records = {
            let mut statement = transaction.prepare(
                "SELECT r.cursor, r.sender_device_id, r.stable_id, r.operation,
                        r.logical_clock, r.content_hash, r.payload
                 FROM anywhere_sync_remote r
                 LEFT JOIN anywhere_sync_apply a ON a.cursor = r.cursor
                 WHERE r.record_kind = 'memory' AND a.cursor IS NULL
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
                        row.get::<_, Vec<u8>>(5)?,
                        row.get::<_, Vec<u8>>(6)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            records
        };
        let mut records = Vec::with_capacity(raw_records.len());
        for (cursor, sender, stable_id, operation, logical_clock, content_hash, payload) in
            raw_records
        {
            records.push(StagedMemoryRecord {
                cursor,
                sender_device_id: sender.try_into().map_err(|_| {
                    StoreError::InvalidValue("staged sync sender id has the wrong length".into())
                })?,
                stable_id,
                operation,
                logical_clock,
                content_hash: content_hash.try_into().map_err(|_| {
                    StoreError::InvalidValue("staged sync content hash has the wrong length".into())
                })?,
                payload,
            });
        }

        let mut summary = RemoteSyncApplySummary::default();
        for record in records {
            summary.inspected += 1;
            let mutation = match parse_memory_mutation(&record) {
                Ok(mutation) => mutation,
                Err(detail) => {
                    record_sync_apply_outcome(
                        &transaction,
                        record.cursor,
                        "conflict",
                        Some(&detail),
                    )?;
                    summary.conflicts += 1;
                    continue;
                }
            };

            let local_version = transaction
                .query_row(
                    "SELECT operation, logical_clock, content_hash FROM sync_journal
                     WHERE record_kind = 'memory' AND stable_id = ?1
                     ORDER BY logical_clock DESC, id DESC LIMIT 1",
                    [&record.stable_id],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, Vec<u8>>(2)?,
                        ))
                    },
                )
                .optional()?;
            let materialized_version = transaction
                .query_row(
                    "SELECT operation, logical_clock, sender_device_id, content_hash
                     FROM anywhere_sync_materialized
                     WHERE record_kind = 'memory' AND stable_id = ?1",
                    [&record.stable_id],
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

            let mut disposition = None;
            if let Some((operation, clock, hash)) = &local_version {
                disposition = compare_sync_version(
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
                );
            }
            if let Some((operation, clock, sender, hash)) = &materialized_version {
                let sender: [u8; 16] = sender.as_slice().try_into().map_err(|_| {
                    StoreError::InvalidValue(
                        "materialized sync sender id has the wrong length".into(),
                    )
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
                if matches!(candidate, Some(SyncVersionDisposition::Conflict))
                    || disposition.is_none()
                {
                    disposition = candidate.or(disposition);
                }
            }
            match disposition {
                Some(SyncVersionDisposition::Conflict) => {
                    record_sync_apply_outcome(
                        &transaction,
                        record.cursor,
                        "conflict",
                        Some("equal sync versions contain different content"),
                    )?;
                    summary.conflicts += 1;
                    continue;
                }
                Some(SyncVersionDisposition::Superseded) => {
                    record_sync_apply_outcome(
                        &transaction,
                        record.cursor,
                        "superseded",
                        Some("a deterministic newer local or remote version already exists"),
                    )?;
                    summary.superseded += 1;
                    continue;
                }
                None => {}
            }

            let primary_exists = transaction.query_row(
                "SELECT EXISTS(SELECT 1 FROM memory WHERE id = ?1)",
                [&record.stable_id],
                |row| row.get::<_, bool>(0),
            )?;
            if primary_exists && local_version.is_none() && materialized_version.is_none() {
                record_sync_apply_outcome(
                    &transaction,
                    record.cursor,
                    "conflict",
                    Some("existing local memory has no sync provenance"),
                )?;
                summary.conflicts += 1;
                continue;
            }

            match mutation {
                MemoryMutation::Upsert(payload) => {
                    transaction.execute(
                        "INSERT INTO memory
                         (id, scope, kind, text, source_session, created_at, updated_at, salience)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                         ON CONFLICT(id) DO UPDATE SET
                           scope = excluded.scope,
                           kind = excluded.kind,
                           text = excluded.text,
                           source_session = excluded.source_session,
                           created_at = excluded.created_at,
                           updated_at = excluded.updated_at,
                           salience = excluded.salience",
                        rusqlite::params![
                            payload.id,
                            payload.scope,
                            payload.kind,
                            payload.text,
                            payload.source_session,
                            payload.created_at,
                            payload.updated_at,
                            payload.salience
                        ],
                    )?;
                }
                MemoryMutation::Tombstone => {
                    transaction.execute("DELETE FROM memory WHERE id = ?1", [&record.stable_id])?;
                }
            }
            transaction.execute(
                "INSERT INTO anywhere_sync_materialized
                 (record_kind, stable_id, operation, logical_clock, sender_device_id, content_hash)
                 VALUES ('memory', ?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(record_kind, stable_id) DO UPDATE SET
                   operation = excluded.operation,
                   logical_clock = excluded.logical_clock,
                   sender_device_id = excluded.sender_device_id,
                   content_hash = excluded.content_hash",
                rusqlite::params![
                    &record.stable_id,
                    &record.operation,
                    record.logical_clock,
                    record.sender_device_id.as_slice(),
                    record.content_hash.as_slice()
                ],
            )?;
            record_sync_apply_outcome(&transaction, record.cursor, "applied", None)?;
            summary.applied += 1;
        }
        transaction.commit()?;
        Ok(summary)
    }

    /// Apply account-scoped settings, commands, skills, agents, and workflows.
    ///
    /// Payloads remain opaque in this layer and are never copied to `config.toml`, the keyring, or
    /// a workspace. Kind-specific consumers must validate the bytes before using them.
    pub fn apply_staged_portable_records(
        &self,
        local_device_id: [u8; 16],
        limit: usize,
    ) -> Result<RemoteSyncApplySummary> {
        const KINDS: &str = "'user_setting', 'command', 'skill', 'agent', 'workflow'";
        if limit == 0 {
            return Ok(RemoteSyncApplySummary::default());
        }
        let limit = i64::try_from(limit).map_err(|_| {
            StoreError::InvalidValue("sync apply limit exceeds SQLite range".into())
        })?;
        let mut conn = self.lock()?;
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let sql = format!(
            "SELECT r.cursor, r.sender_device_id, r.record_kind, r.stable_id, r.operation,
                    r.logical_clock, r.content_hash, r.payload
             FROM anywhere_sync_remote r
             LEFT JOIN anywhere_sync_apply a ON a.cursor = r.cursor
             WHERE r.record_kind IN ({KINDS}) AND a.cursor IS NULL
             ORDER BY r.cursor LIMIT ?1"
        );
        let raw = {
            let mut statement = transaction.prepare(&sql)?;
            let records = statement
                .query_map([limit], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, Vec<u8>>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, i64>(5)?,
                        row.get::<_, Vec<u8>>(6)?,
                        row.get::<_, Vec<u8>>(7)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            records
        };
        let mut summary = RemoteSyncApplySummary::default();
        for (cursor, sender, kind, stable_id, operation, clock, hash, payload) in raw {
            summary.inspected += 1;
            let record = StagedHistoryRecord {
                cursor,
                sender_device_id: sender.try_into().map_err(|_| {
                    StoreError::InvalidValue("staged sync sender id has the wrong length".into())
                })?,
                record_kind: kind,
                stable_id,
                operation,
                logical_clock: clock,
                content_hash: hash.try_into().map_err(|_| {
                    StoreError::InvalidValue("staged sync content hash has the wrong length".into())
                })?,
                payload,
            };
            let invalid = sync_payload_hash(&record.payload) != record.content_hash
                || (record.operation == "tombstone" && !record.payload.is_empty())
                || !matches!(record.operation.as_str(), "upsert" | "tombstone");
            if invalid {
                record_sync_apply_outcome(
                    &transaction,
                    record.cursor,
                    "conflict",
                    Some("portable record payload or operation is invalid"),
                )?;
                summary.conflicts += 1;
                continue;
            }
            match classify_mutable_sync_version(&transaction, &record, local_device_id)?.0 {
                Some(SyncVersionDisposition::Conflict) => {
                    record_sync_apply_outcome(
                        &transaction,
                        record.cursor,
                        "conflict",
                        Some("equal portable record versions contain different content"),
                    )?;
                    summary.conflicts += 1;
                    continue;
                }
                Some(SyncVersionDisposition::Superseded) => {
                    record_sync_apply_outcome(
                        &transaction,
                        record.cursor,
                        "superseded",
                        Some("a deterministic newer portable record already exists"),
                    )?;
                    summary.superseded += 1;
                    continue;
                }
                None => {}
            }
            transaction.execute(
                "INSERT INTO anywhere_sync_portable_record
                 (record_kind, stable_id, payload, deleted, logical_clock,
                  sender_device_id, content_hash)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(record_kind, stable_id) DO UPDATE SET
                   payload = excluded.payload, deleted = excluded.deleted,
                   logical_clock = excluded.logical_clock,
                   sender_device_id = excluded.sender_device_id,
                   content_hash = excluded.content_hash",
                rusqlite::params![
                    &record.record_kind,
                    &record.stable_id,
                    &record.payload,
                    record.operation == "tombstone",
                    record.logical_clock,
                    record.sender_device_id.as_slice(),
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

    /// Load one portable record, including its tombstone when `deleted` is true.
    pub fn portable_sync_record(
        &self,
        record_kind: &str,
        stable_id: &str,
    ) -> Result<Option<PortableSyncRecord>> {
        self.lock()?
            .query_row(
                "SELECT payload, deleted, logical_clock, sender_device_id, content_hash
                 FROM anywhere_sync_portable_record
                 WHERE record_kind = ?1 AND stable_id = ?2",
                (record_kind, stable_id),
                |row| {
                    let sender: Vec<u8> = row.get(3)?;
                    let hash: Vec<u8> = row.get(4)?;
                    Ok(PortableSyncRecord {
                        record_kind: record_kind.to_owned(),
                        stable_id: stable_id.to_owned(),
                        payload: row.get(0)?,
                        deleted: row.get(1)?,
                        logical_clock: row.get::<_, i64>(2)?.max(0) as u64,
                        sender_device_id: sender.try_into().map_err(|_| {
                            rusqlite::Error::FromSqlConversionFailure(
                                3,
                                rusqlite::types::Type::Blob,
                                "portable sender id is not 16 bytes".into(),
                            )
                        })?,
                        content_hash: hash.try_into().map_err(|_| {
                            rusqlite::Error::FromSqlConversionFailure(
                                4,
                                rusqlite::types::Type::Blob,
                                "portable content hash is not 32 bytes".into(),
                            )
                        })?,
                    })
                },
            )
            .optional()
            .map_err(StoreError::from)
    }

    /// Commit a local portable record and its encrypted-sync journal revision atomically.
    pub fn write_portable_sync_record(
        &self,
        local_device_id: [u8; 16],
        record_kind: &str,
        stable_id: &str,
        payload: Option<&[u8]>,
    ) -> Result<()> {
        if !matches!(
            record_kind,
            "user_setting" | "command" | "skill" | "agent" | "workflow"
        ) || stable_id.trim().is_empty()
        {
            return Err(StoreError::InvalidValue(
                "portable sync record kind or stable id is invalid".into(),
            ));
        }
        let normalized = stable_id.to_ascii_lowercase();
        if record_kind == "user_setting"
            && [
                "secret",
                "token",
                "password",
                "credential",
                "api_key",
                "private_key",
            ]
            .iter()
            .any(|needle| normalized.contains(needle))
        {
            return Err(StoreError::InvalidValue(
                "secret-bearing settings are not eligible for sync".into(),
            ));
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
        let next: i64 = transaction.query_row(
            "SELECT MAX(value) + 1 FROM (
               SELECT COALESCE(MAX(revision), 0) AS value FROM sync_journal
                WHERE record_kind = ?1 AND stable_id = ?2
               UNION ALL
               SELECT COALESCE(MAX(logical_clock), 0) AS value
                FROM anywhere_sync_portable_record
                WHERE record_kind = ?1 AND stable_id = ?2
             )",
            (record_kind, stable_id),
            |row| row.get(0),
        )?;
        let hash = sync_payload_hash(payload);
        transaction.execute(
            "INSERT INTO anywhere_sync_portable_record
             (record_kind, stable_id, payload, deleted, logical_clock,
              sender_device_id, content_hash)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(record_kind, stable_id) DO UPDATE SET
               payload = excluded.payload, deleted = excluded.deleted,
               logical_clock = excluded.logical_clock,
               sender_device_id = excluded.sender_device_id,
               content_hash = excluded.content_hash",
            rusqlite::params![
                record_kind,
                stable_id,
                payload,
                operation == SyncJournalOperation::Tombstone,
                next,
                local_device_id.as_slice(),
                hash.as_slice(),
            ],
        )?;
        if sync_enabled {
            insert_sync_journal_row(
                &transaction,
                record_kind,
                stable_id,
                operation,
                next,
                next,
                payload,
            )?;
        }
        transaction.commit()?;
        Ok(())
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
}
