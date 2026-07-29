//! Focused persistence operations.

use super::*;

impl Store {
    /// Apply staged portable history whose local dependency graph is already present.
    ///
    /// Session snapshots update only portable presentation metadata and never replace `cwd`,
    /// worktree, or permission fields. Missing sessions/messages remain staged for a later pass;
    /// stable-id or message-sequence collisions become durable conflicts instead of being remapped.
    pub fn apply_staged_history_records(
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
                "SELECT r.cursor, r.sender_device_id, r.record_kind, r.stable_id, r.operation,
                        r.logical_clock, r.content_hash, r.payload
                 FROM anywhere_sync_remote r
                 LEFT JOIN anywhere_sync_apply a ON a.cursor = r.cursor
                 WHERE r.record_kind IN
                       ('session', 'message', 'checkpoint', 'tool_call',
                        'routing_decision', 'usage', 'compaction')
                   AND a.cursor IS NULL
                 ORDER BY CASE r.record_kind
                            WHEN 'session' THEN 0
                            WHEN 'message' THEN 1
                            WHEN 'checkpoint' THEN 2
                            WHEN 'compaction' THEN 4
                            ELSE 3
                          END,
                          r.cursor
                 LIMIT ?1",
            )?;
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
        let mut records = Vec::with_capacity(raw_records.len());
        for (cursor, sender, kind, stable_id, operation, clock, hash, payload) in raw_records {
            records.push(StagedHistoryRecord {
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
            });
        }

        let mut summary = RemoteSyncApplySummary::default();
        for record in records {
            summary.inspected += 1;
            let mutation = match parse_history_mutation(&record) {
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

            if let HistoryMutation::Session(payload) = mutation {
                let exists = transaction.query_row(
                    "SELECT EXISTS(SELECT 1 FROM session WHERE id = ?1)",
                    [&record.stable_id],
                    |row| row.get::<_, bool>(0),
                )?;
                let (disposition, has_provenance) =
                    classify_mutable_sync_version(&transaction, &record, local_device_id)?;
                match disposition {
                    Some(SyncVersionDisposition::Conflict) => {
                        record_sync_apply_outcome(
                            &transaction,
                            record.cursor,
                            "conflict",
                            Some("equal session versions contain different content"),
                        )?;
                        summary.conflicts += 1;
                        continue;
                    }
                    Some(SyncVersionDisposition::Superseded) => {
                        record_sync_apply_outcome(
                            &transaction,
                            record.cursor,
                            "superseded",
                            Some("a deterministic newer session snapshot already exists"),
                        )?;
                        summary.superseded += 1;
                        continue;
                    }
                    None if exists && !has_provenance => {
                        record_sync_apply_outcome(
                            &transaction,
                            record.cursor,
                            "conflict",
                            Some("existing local session has no sync provenance"),
                        )?;
                        summary.conflicts += 1;
                        continue;
                    }
                    None => {}
                }
                if let Some(payload) = payload {
                    if exists {
                        transaction.execute(
                            "UPDATE session SET title = ?2, archived = ?3, view_snapshot = ?4,
                                 updated_at = strftime('%s','now') WHERE id = ?1",
                            rusqlite::params![
                                payload.id,
                                payload.title,
                                payload.archived,
                                payload.view_snapshot
                            ],
                        )?;
                    } else {
                        let local_cwd = std::env::current_dir()
                            .unwrap_or_else(|_| PathBuf::from("."))
                            .to_string_lossy()
                            .into_owned();
                        transaction.execute(
                            "INSERT INTO session
                             (id, title, cwd, permission_mode, archived, view_snapshot)
                             VALUES (?1, ?2, ?3, 'accept_edits', ?4, ?5)",
                            rusqlite::params![
                                payload.id,
                                payload.title,
                                local_cwd,
                                payload.archived,
                                payload.view_snapshot
                            ],
                        )?;
                    }
                } else if exists {
                    transaction
                        .execute("DELETE FROM session WHERE id = ?1", [&record.stable_id])?;
                }
                upsert_sync_materialized(&transaction, &record)?;
                record_sync_apply_outcome(&transaction, record.cursor, "applied", None)?;
                summary.applied += 1;
                continue;
            }

            if let HistoryMutation::Compaction(payload) = mutation {
                let exists = transaction.query_row(
                    "SELECT EXISTS(SELECT 1 FROM session WHERE id = ?1)",
                    [&record.stable_id],
                    |row| row.get::<_, bool>(0),
                )?;
                if !exists {
                    summary.deferred += 1;
                    continue;
                }
                let had_compaction = transaction.query_row(
                    "SELECT EXISTS(SELECT 1 FROM session_compaction WHERE session_id = ?1)",
                    [&record.stable_id],
                    |row| row.get::<_, bool>(0),
                )?;
                let (disposition, has_provenance) =
                    classify_mutable_sync_version(&transaction, &record, local_device_id)?;
                match disposition {
                    Some(SyncVersionDisposition::Conflict) => {
                        record_sync_apply_outcome(
                            &transaction,
                            record.cursor,
                            "conflict",
                            Some("equal compaction versions contain different content"),
                        )?;
                        summary.conflicts += 1;
                        continue;
                    }
                    Some(SyncVersionDisposition::Superseded) => {
                        record_sync_apply_outcome(
                            &transaction,
                            record.cursor,
                            "superseded",
                            Some("a deterministic newer compaction already exists"),
                        )?;
                        summary.superseded += 1;
                        continue;
                    }
                    None if had_compaction && !has_provenance => {
                        record_sync_apply_outcome(
                            &transaction,
                            record.cursor,
                            "conflict",
                            Some("existing local compaction has no sync provenance"),
                        )?;
                        summary.conflicts += 1;
                        continue;
                    }
                    None => {}
                }
                if let Some(payload) = payload {
                    let keep_count = i64::try_from(payload.keep_count).map_err(|_| {
                        StoreError::InvalidValue(
                            "remote compaction keep count exceeds SQLite range".into(),
                        )
                    })?;
                    if keep_count == 0 {
                        transaction.execute(
                            "UPDATE message SET active = 0, compacted = 1
                             WHERE session_id = ?1 AND active = 1",
                            [&payload.session_id],
                        )?;
                    } else {
                        transaction.execute(
                            "UPDATE message SET active = 0, compacted = 1
                             WHERE session_id = ?1 AND active = 1
                               AND seq < (
                                 SELECT seq FROM message
                                 WHERE session_id = ?1 AND active = 1
                                 ORDER BY seq DESC LIMIT 1 OFFSET ?2
                               )",
                            rusqlite::params![&payload.session_id, keep_count - 1],
                        )?;
                    }
                    transaction.execute(
                        "INSERT INTO session_compaction (session_id, summary) VALUES (?1, ?2)
                         ON CONFLICT(session_id) DO UPDATE SET
                           summary = excluded.summary,
                           created_at = strftime('%s','now')",
                        rusqlite::params![payload.session_id, payload.summary],
                    )?;
                } else {
                    transaction.execute(
                        "UPDATE message SET active = 1, compacted = 0
                         WHERE session_id = ?1 AND compacted = 1",
                        [&record.stable_id],
                    )?;
                    transaction.execute(
                        "DELETE FROM session_compaction WHERE session_id = ?1",
                        [&record.stable_id],
                    )?;
                }
                upsert_sync_materialized(&transaction, &record)?;
                record_sync_apply_outcome(&transaction, record.cursor, "applied", None)?;
                summary.applied += 1;
                continue;
            }

            if matches!(&mutation, HistoryMutation::Tombstone) {
                match classify_mutable_sync_version(&transaction, &record, local_device_id)?.0 {
                    Some(SyncVersionDisposition::Conflict) => {
                        record_sync_apply_outcome(
                            &transaction,
                            record.cursor,
                            "conflict",
                            Some("equal history tombstone versions contain different content"),
                        )?;
                        summary.conflicts += 1;
                        continue;
                    }
                    Some(SyncVersionDisposition::Superseded) => {
                        record_sync_apply_outcome(
                            &transaction,
                            record.cursor,
                            "superseded",
                            Some("a deterministic newer history revision already exists"),
                        )?;
                        summary.superseded += 1;
                        continue;
                    }
                    None => {}
                }
            }

            let local = transaction
                .query_row(
                    "SELECT operation, content_hash FROM sync_journal
                     WHERE record_kind = ?1 AND stable_id = ?2
                     ORDER BY logical_clock DESC, id DESC LIMIT 1",
                    (&record.record_kind, &record.stable_id),
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?)),
                )
                .optional()?;
            if let Some((operation, hash)) =
                local.filter(|_| !matches!(&mutation, HistoryMutation::Tombstone))
            {
                if operation == record.operation && hash == record.content_hash {
                    record_sync_apply_outcome(
                        &transaction,
                        record.cursor,
                        "superseded",
                        Some("the immutable record already exists locally"),
                    )?;
                    summary.superseded += 1;
                } else {
                    record_sync_apply_outcome(
                        &transaction,
                        record.cursor,
                        "conflict",
                        Some("an immutable local record has different content"),
                    )?;
                    summary.conflicts += 1;
                }
                continue;
            }

            let outcome = match mutation {
                HistoryMutation::Session(_) => unreachable!("session handled above"),
                HistoryMutation::Message(payload) => {
                    if payload.session_id.trim().is_empty()
                        || payload.seq < 0
                        || Role::parse(&payload.role).is_none()
                        || !matches!(payload.visibility.as_str(), "llm" | "llm_only" | "ui")
                    {
                        Err("message snapshot contains invalid fields")
                    } else if !transaction.query_row(
                        "SELECT EXISTS(SELECT 1 FROM session WHERE id = ?1)",
                        [&payload.session_id],
                        |row| row.get::<_, bool>(0),
                    )? {
                        Ok(false)
                    } else if transaction.query_row(
                        "SELECT EXISTS(SELECT 1 FROM message WHERE id = ?1 OR
                             (session_id = ?2 AND seq = ?3))",
                        rusqlite::params![&payload.id, &payload.session_id, payload.seq],
                        |row| row.get::<_, bool>(0),
                    )? {
                        Err("message id or session sequence already belongs to local data")
                    } else {
                        let tool_calls = if payload.tool_calls.is_empty() {
                            None
                        } else {
                            Some(
                                serde_json::to_string(&payload.tool_calls)
                                    .map_err(|error| StoreError::Json(error.to_string()))?,
                            )
                        };
                        transaction.execute(
                            "INSERT INTO message
                             (id, session_id, seq, role, content, model, tool_calls_json,
                              tool_call_id, visibility, active)
                             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                            rusqlite::params![
                                payload.id,
                                payload.session_id,
                                payload.seq,
                                payload.role,
                                payload.content,
                                payload.model,
                                tool_calls,
                                payload.tool_call_id,
                                payload.visibility,
                                payload.active
                            ],
                        )?;
                        Ok(true)
                    }
                }
                HistoryMutation::Checkpoint(payload) => {
                    if payload.session_id.trim().is_empty() || payload.seq < 0 {
                        Err("checkpoint snapshot contains invalid fields")
                    } else if !transaction.query_row(
                        "SELECT EXISTS(SELECT 1 FROM session WHERE id = ?1)",
                        [&payload.session_id],
                        |row| row.get::<_, bool>(0),
                    )? {
                        Ok(false)
                    } else if transaction.query_row(
                        "SELECT EXISTS(SELECT 1 FROM checkpoint WHERE id = ?1)",
                        [&payload.id],
                        |row| row.get::<_, bool>(0),
                    )? {
                        Err("checkpoint id already belongs to local data")
                    } else {
                        transaction.execute(
                            "INSERT INTO checkpoint (id, session_id, label, seq)
                             VALUES (?1, ?2, ?3, ?4)",
                            rusqlite::params![
                                payload.id,
                                payload.session_id,
                                payload.label,
                                payload.seq
                            ],
                        )?;
                        Ok(true)
                    }
                }
                HistoryMutation::ToolCall(payload) => {
                    if payload.message_id.trim().is_empty() || payload.tool_name.trim().is_empty() {
                        Err("tool-call snapshot contains invalid fields")
                    } else if !transaction.query_row(
                        "SELECT EXISTS(SELECT 1 FROM message WHERE id = ?1)",
                        [&payload.message_id],
                        |row| row.get::<_, bool>(0),
                    )? {
                        Ok(false)
                    } else if transaction.query_row(
                        "SELECT EXISTS(SELECT 1 FROM tool_call WHERE id = ?1)",
                        [&payload.id],
                        |row| row.get::<_, bool>(0),
                    )? {
                        Err("tool-call id already belongs to local data")
                    } else {
                        transaction.execute(
                            "INSERT INTO tool_call
                             (id, message_id, tool_name, args_json, result_json,
                              permission, status, path)
                             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                            rusqlite::params![
                                payload.id,
                                payload.message_id,
                                payload.tool_name,
                                payload.args_json,
                                payload.result_json,
                                payload.permission,
                                payload.status,
                                payload.path
                            ],
                        )?;
                        Ok(true)
                    }
                }
                HistoryMutation::Routing(payload) => {
                    if payload.message_id.trim().is_empty()
                        || payload.chosen_model.trim().is_empty()
                    {
                        Err("routing snapshot contains invalid fields")
                    } else if !transaction.query_row(
                        "SELECT EXISTS(SELECT 1 FROM message WHERE id = ?1)",
                        [&payload.message_id],
                        |row| row.get::<_, bool>(0),
                    )? {
                        Ok(false)
                    } else if transaction.query_row(
                        "SELECT EXISTS(SELECT 1 FROM routing_decision WHERE id = ?1)",
                        [&payload.id],
                        |row| row.get::<_, bool>(0),
                    )? {
                        Err("routing-decision id already belongs to local data")
                    } else {
                        transaction.execute(
                            "INSERT INTO routing_decision
                             (id, message_id, task_tier, chosen_model, rationale)
                             VALUES (?1, ?2, ?3, ?4, ?5)",
                            rusqlite::params![
                                payload.id,
                                payload.message_id,
                                payload.task_tier,
                                payload.chosen_model,
                                payload.rationale
                            ],
                        )?;
                        Ok(true)
                    }
                }
                HistoryMutation::Usage(payload) => {
                    if payload.message_id.trim().is_empty()
                        || payload.input_tokens < 0
                        || payload.cached_input_tokens < 0
                        || payload.output_tokens < 0
                        || !payload.cost_usd.is_finite()
                        || payload.cost_usd < 0.0
                    {
                        Err("usage snapshot contains invalid fields")
                    } else if !transaction.query_row(
                        "SELECT EXISTS(SELECT 1 FROM message WHERE id = ?1)",
                        [&payload.message_id],
                        |row| row.get::<_, bool>(0),
                    )? {
                        Ok(false)
                    } else if transaction.query_row(
                        "SELECT EXISTS(SELECT 1 FROM usage WHERE id = ?1)",
                        [&payload.id],
                        |row| row.get::<_, bool>(0),
                    )? {
                        Err("usage id already belongs to local data")
                    } else {
                        transaction.execute(
                            "INSERT INTO usage
                             (id, message_id, input_tokens, cached_input_tokens, output_tokens, cost_usd)
                             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                            rusqlite::params![
                                payload.id,
                                payload.message_id,
                                payload.input_tokens,
                                payload.cached_input_tokens,
                                payload.output_tokens,
                                payload.cost_usd
                            ],
                        )?;
                        transaction.execute(
                            "UPDATE session SET total_cost_usd = total_cost_usd + ?1,
                                 updated_at = strftime('%s','now')
                             WHERE id = (SELECT session_id FROM message WHERE id = ?2)",
                            rusqlite::params![payload.cost_usd, payload.message_id],
                        )?;
                        Ok(true)
                    }
                }
                HistoryMutation::Compaction(_) => unreachable!("compaction handled above"),
                HistoryMutation::Tombstone => {
                    let table = match record.record_kind.as_str() {
                        "message" => "message",
                        "checkpoint" => "checkpoint",
                        "tool_call" => "tool_call",
                        "routing_decision" => "routing_decision",
                        "usage" => "usage",
                        _ => {
                            return Err(StoreError::InvalidValue(
                                "unsupported history tombstone kind".into(),
                            ))
                        }
                    };
                    let has_provenance = transaction.query_row(
                        "SELECT EXISTS(SELECT 1 FROM anywhere_sync_materialized
                         WHERE record_kind = ?1 AND stable_id = ?2)",
                        (&record.record_kind, &record.stable_id),
                        |row| row.get::<_, bool>(0),
                    )? || transaction.query_row(
                        "SELECT EXISTS(SELECT 1 FROM sync_journal
                         WHERE record_kind = ?1 AND stable_id = ?2)",
                        (&record.record_kind, &record.stable_id),
                        |row| row.get::<_, bool>(0),
                    )?;
                    let exists = transaction.query_row(
                        &format!("SELECT EXISTS(SELECT 1 FROM {table} WHERE id = ?1)"),
                        [&record.stable_id],
                        |row| row.get::<_, bool>(0),
                    )?;
                    if exists && !has_provenance {
                        Err("existing local history row has no sync provenance")
                    } else {
                        transaction.execute(
                            &format!("DELETE FROM {table} WHERE id = ?1"),
                            [&record.stable_id],
                        )?;
                        Ok(true)
                    }
                }
            };
            match outcome {
                Ok(true) => {
                    upsert_sync_materialized(&transaction, &record)?;
                    record_sync_apply_outcome(&transaction, record.cursor, "applied", None)?;
                    summary.applied += 1;
                }
                Ok(false) => summary.deferred += 1,
                Err(detail) => {
                    record_sync_apply_outcome(
                        &transaction,
                        record.cursor,
                        "conflict",
                        Some(detail),
                    )?;
                    summary.conflicts += 1;
                }
            }
        }
        transaction.commit()?;
        Ok(summary)
    }
}
