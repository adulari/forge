//! Durable Forge Anywhere handoff lifecycle.
//!
//! Store keeps the complete export/import, provenance, quarantine, and lease
//! transition protocol here so each handoff state change remains transactional.

use forge_types::{Role, Visibility};
use rusqlite::{OptionalExtension, TransactionBehavior};

use crate::{
    append_session_snapshot, HandoffCheckpoint, HandoffImportProvenance, HandoffMessage,
    HandoffSessionExport, HandoffSessionImport, ImportedSessionMetadata, Result, Store, StoreError,
};

impl Store {
    /// Record immutable handoff provenance after the destination session import succeeds.
    pub fn record_imported_session(&self, metadata: &ImportedSessionMetadata) -> Result<()> {
        self.lock()?.execute(
            "INSERT INTO imported_session
                 (session_id, source_session_id, source_device_id, capsule_id, base_commit,
                  worktree_path, imported_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            (
                &metadata.session_id,
                &metadata.source_session_id,
                metadata.source_device_id.as_slice(),
                &metadata.capsule_id,
                &metadata.base_commit,
                &metadata.worktree_path,
                metadata.imported_at,
            ),
        )?;
        Ok(())
    }

    /// Load imported-session provenance, if this session originated from a capsule.
    pub fn imported_session_metadata(
        &self,
        session_id: &str,
    ) -> Result<Option<ImportedSessionMetadata>> {
        let conn = self.lock()?;
        conn.query_row(
            "SELECT session_id, source_session_id, source_device_id, capsule_id, base_commit,
                    worktree_path, imported_at
             FROM imported_session WHERE session_id = ?1",
            [session_id],
            |row| {
                let source_device_id: Vec<u8> = row.get(2)?;
                let source_device_id = source_device_id.try_into().map_err(|_| {
                    rusqlite::Error::FromSqlConversionFailure(
                        2,
                        rusqlite::types::Type::Blob,
                        "source device id is not 16 bytes".into(),
                    )
                })?;
                Ok(ImportedSessionMetadata {
                    session_id: row.get(0)?,
                    source_session_id: row.get(1)?,
                    source_device_id,
                    capsule_id: row.get(3)?,
                    base_commit: row.get(4)?,
                    worktree_path: row.get(5)?,
                    imported_at: row.get(6)?,
                })
            },
        )
        .optional()
        .map_err(StoreError::from)
    }

    /// Load handoff provenance by capsule id for crash-safe destination retries.
    pub fn imported_session_by_capsule(
        &self,
        capsule_id: &str,
    ) -> Result<Option<ImportedSessionMetadata>> {
        let conn = self.lock()?;
        conn.query_row(
            "SELECT session_id, source_session_id, source_device_id, capsule_id, base_commit,
                    worktree_path, imported_at
             FROM imported_session WHERE capsule_id = ?1",
            [capsule_id],
            |row| {
                let source_device_id: Vec<u8> = row.get(2)?;
                let source_device_id = source_device_id.try_into().map_err(|_| {
                    rusqlite::Error::FromSqlConversionFailure(
                        2,
                        rusqlite::types::Type::Blob,
                        "source device id is not 16 bytes".into(),
                    )
                })?;
                Ok(ImportedSessionMetadata {
                    session_id: row.get(0)?,
                    source_session_id: row.get(1)?,
                    source_device_id,
                    capsule_id: row.get(3)?,
                    base_commit: row.get(4)?,
                    worktree_path: row.get(5)?,
                    imported_at: row.get(6)?,
                })
            },
        )
        .optional()
        .map_err(StoreError::from)
    }

    /// Export the portable transcript and rewind points needed to resume a session after handoff.
    /// Provider credentials, indexes, caches, schedules, and queue internals are never included.
    pub fn export_handoff_session(&self, session_id: &str) -> Result<HandoffSessionExport> {
        let conn = self.lock()?;
        let (title, permission_mode) = conn
            .query_row(
                "SELECT title, permission_mode FROM session WHERE id = ?1",
                [session_id],
                |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?
            .ok_or_else(|| StoreError::InvalidValue("handoff session does not exist".into()))?;
        let messages = {
            let mut statement = conn.prepare(
                "SELECT seq, role, content, model, tool_calls_json, tool_call_id, visibility, active
                 FROM message WHERE session_id = ?1 ORDER BY seq, created_at",
            )?;
            let rows = statement
                .query_map([session_id], |row| {
                    let role: String = row.get(1)?;
                    let tool_calls_json: Option<String> = row.get(4)?;
                    Ok(HandoffMessage {
                        seq: row.get(0)?,
                        role: Role::parse(&role).unwrap_or(Role::User),
                        content: row.get(2)?,
                        model: row.get(3)?,
                        tool_calls: tool_calls_json
                            .and_then(|json| serde_json::from_str(&json).ok())
                            .unwrap_or_default(),
                        tool_call_id: row.get(5)?,
                        visibility: Visibility::parse(&row.get::<_, String>(6)?),
                        active: row.get(7)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            rows
        };
        let checkpoints = {
            let mut statement = conn.prepare(
                "SELECT label, seq FROM checkpoint WHERE session_id = ?1 ORDER BY seq, created_at",
            )?;
            let rows = statement
                .query_map([session_id], |row| {
                    Ok(HandoffCheckpoint {
                        label: row.get(0)?,
                        seq: row.get(1)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            rows
        };
        Ok(HandoffSessionExport {
            version: 1,
            source_session_id: session_id.to_owned(),
            title,
            permission_mode,
            messages,
            checkpoints,
        })
    }

    /// Import a handoff session into `worktree_path`, remapping its id on a local collision.
    /// The complete import is one transaction; callers record provenance before acknowledging the
    /// remote lease and delete the session/worktree if that later step fails.
    pub fn import_handoff_session(
        &self,
        export: &HandoffSessionExport,
        worktree_path: &str,
    ) -> Result<HandoffSessionImport> {
        self.import_handoff_session_inner(export, worktree_path, None)
    }

    /// Atomically import a handoff session and its immutable capsule provenance.
    pub fn import_handoff_session_with_provenance(
        &self,
        export: &HandoffSessionExport,
        worktree_path: &str,
        provenance: &HandoffImportProvenance,
    ) -> Result<HandoffSessionImport> {
        self.import_handoff_session_inner(export, worktree_path, Some(provenance))
    }

    fn import_handoff_session_inner(
        &self,
        export: &HandoffSessionExport,
        worktree_path: &str,
        provenance: Option<&HandoffImportProvenance>,
    ) -> Result<HandoffSessionImport> {
        if export.version != 1 || export.source_session_id.trim().is_empty() {
            return Err(StoreError::InvalidValue(
                "unsupported or invalid handoff session export".into(),
            ));
        }
        let mut conn = self.lock()?;
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let collision = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM session WHERE id = ?1)",
            [&export.source_session_id],
            |row| row.get::<_, bool>(0),
        )?;
        let session_id = if collision {
            forge_types::new_id()
        } else {
            export.source_session_id.clone()
        };
        transaction.execute(
            "INSERT INTO session
                 (id, title, cwd, permission_mode, worktree_path, archived)
             VALUES (?1, ?2, ?3, ?4, ?3, ?5)",
            (
                &session_id,
                &export.title,
                worktree_path,
                &export.permission_mode,
                i64::from(provenance.is_some()),
            ),
        )?;
        for message in &export.messages {
            let id = forge_types::new_id();
            let tool_calls_json = if message.tool_calls.is_empty() {
                None
            } else {
                Some(
                    serde_json::to_string(&message.tool_calls)
                        .map_err(|error| StoreError::Json(error.to_string()))?,
                )
            };
            transaction.execute(
                "INSERT INTO message
                     (id, session_id, seq, role, content, model, tool_calls_json, tool_call_id,
                      visibility, active)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                rusqlite::params![
                    id,
                    session_id,
                    message.seq,
                    message.role.as_str(),
                    message.content,
                    message.model,
                    tool_calls_json,
                    message.tool_call_id,
                    message.visibility.as_str(),
                    message.active,
                ],
            )?;
        }
        for checkpoint in &export.checkpoints {
            transaction.execute(
                "INSERT INTO checkpoint (id, session_id, label, seq) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![
                    forge_types::new_id(),
                    session_id,
                    checkpoint.label,
                    checkpoint.seq,
                ],
            )?;
        }
        if let Some(provenance) = provenance {
            transaction.execute(
                "INSERT INTO imported_session
                     (session_id, source_session_id, source_device_id, capsule_id, base_commit,
                      worktree_path, imported_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                (
                    &session_id,
                    &export.source_session_id,
                    provenance.source_device_id.as_slice(),
                    &provenance.capsule_id,
                    &provenance.base_commit,
                    worktree_path,
                    provenance.imported_at,
                ),
            )?;
            transaction.execute(
                "INSERT INTO anywhere_handoff_session_state (session_id, capsule_id, state)
                 VALUES (?1, ?2, 'destination_quarantined')",
                (&session_id, &provenance.capsule_id),
            )?;
        }
        append_session_snapshot(&transaction, &session_id)?;
        transaction.commit()?;
        Ok(HandoffSessionImport {
            session_id,
            remapped: collision,
        })
    }

    /// Remove a locally imported session during a failed handoff acknowledgement rollback.
    pub fn rollback_handoff_session(&self, session_id: &str) -> Result<bool> {
        Ok(self
            .lock()?
            .execute("DELETE FROM session WHERE id = ?1", [session_id])?
            > 0)
    }

    /// Freeze a source session before any handoff network request. Repeating the exact operation
    /// is idempotent; a different capsule cannot replace a pending or transferred operation.
    pub fn begin_source_handoff(&self, session_id: &str, capsule_id: &str) -> Result<()> {
        let mut conn = self.lock()?;
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing = transaction
            .query_row(
                "SELECT capsule_id, state FROM anywhere_handoff_session_state WHERE session_id=?1",
                [session_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?;
        match existing {
            Some((existing, state)) if existing == capsule_id && state == "source_pending" => {}
            Some(_) => {
                return Err(StoreError::InvalidValue(
                    "session already has a different or terminal handoff".into(),
                ));
            }
            None => {
                let exists: bool = transaction.query_row(
                    "SELECT EXISTS(SELECT 1 FROM session WHERE id=?1)",
                    [session_id],
                    |row| row.get(0),
                )?;
                if !exists {
                    return Err(StoreError::InvalidValue(
                        "handoff session does not exist".into(),
                    ));
                }
                transaction.execute(
                    "INSERT INTO anywhere_handoff_session_state (session_id, capsule_id, state)
                     VALUES (?1, ?2, 'source_pending')",
                    (session_id, capsule_id),
                )?;
            }
        }
        transaction.execute("UPDATE session SET archived=1 WHERE id=?1", [session_id])?;
        transaction.commit()?;
        Ok(())
    }

    /// Confirm that the service cancelled a pending source handoff and make it resumable again.
    pub fn cancel_source_handoff(&self, session_id: &str, capsule_id: &str) -> Result<bool> {
        let mut conn = self.lock()?;
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let removed = transaction.execute(
            "DELETE FROM anywhere_handoff_session_state
             WHERE session_id=?1 AND capsule_id=?2 AND state='source_pending'",
            (session_id, capsule_id),
        )? > 0;
        if removed {
            transaction.execute("UPDATE session SET archived=0 WHERE id=?1", [session_id])?;
        }
        transaction.commit()?;
        Ok(removed)
    }

    /// Permanently record that this source lease moved away. Ordinary archive controls cannot
    /// resurrect a transferred session.
    pub fn mark_source_handoff_transferred(
        &self,
        session_id: &str,
        capsule_id: &str,
    ) -> Result<()> {
        let changed = self.lock()?.execute(
            "UPDATE anywhere_handoff_session_state
             SET state='source_transferred', updated_at=strftime('%s','now')
             WHERE session_id=?1 AND capsule_id=?2 AND state IN
                   ('source_pending','source_transferred')",
            (session_id, capsule_id),
        )?;
        if changed == 0 {
            return Err(StoreError::InvalidValue(
                "source handoff is not pending on this machine".into(),
            ));
        }
        Ok(())
    }

    /// Activate a quarantined destination only after the service accepted its acknowledgement.
    pub fn activate_destination_handoff(&self, session_id: &str, capsule_id: &str) -> Result<()> {
        let mut conn = self.lock()?;
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let removed = transaction.execute(
            "DELETE FROM anywhere_handoff_session_state
             WHERE session_id=?1 AND capsule_id=?2 AND state='destination_quarantined'",
            (session_id, capsule_id),
        )?;
        if removed == 0 {
            return Err(StoreError::InvalidValue(
                "destination handoff is not quarantined".into(),
            ));
        }
        transaction.execute("UPDATE session SET archived=0 WHERE id=?1", [session_id])?;
        transaction.commit()?;
        Ok(())
    }

    /// Whether local input/resume must be rejected to preserve a handoff lease invariant.
    pub fn session_handoff_blocked(&self, session_id: &str) -> Result<bool> {
        let blocked: bool = self.lock()?.query_row(
            "SELECT EXISTS(SELECT 1 FROM anywhere_handoff_session_state WHERE session_id=?1)",
            [session_id],
            |row| row.get(0),
        )?;
        Ok(blocked)
    }
}
