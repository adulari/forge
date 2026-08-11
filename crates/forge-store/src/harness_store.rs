//! Continual Harness (`/refine`, port of prime-agent's `/refine`): durable prompt/skill/subagent
//! entries the agent proposes about itself, plus the append-only refinement journal that makes
//! every batch of edits inspectable and reversible. The `harness_entry` / `harness_refinement`
//! tables are declared in `migrations.rs` (migration #25).

use rusqlite::{Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};

use crate::{
    append_sync_revision, sync_json, with_busy_retry, Store, StoreError, SyncJournalOperation,
};

type Result<T> = std::result::Result<T, StoreError>;

/// A durable harness artifact: a prompt, skill, or subagent definition scoped to `global`, a
/// `project:<abs path>`, or a `session:<session id>`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HarnessEntry {
    pub id: String,
    pub scope: String,
    pub kind: String,
    pub title: String,
    pub content: String,
    pub source: String,
    pub version: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

/// One requested change against a scope's harness entries, as proposed by a refinement pass.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HarnessEdit {
    /// `"create"` | `"update"` | `"delete"`.
    pub action: String,
    pub kind: String,
    /// Required for `update`/`delete`; ignored for `create`.
    pub id: Option<String>,
    pub title: Option<String>,
    pub content: Option<String>,
    pub reason: Option<String>,
}

/// The outcome of applying one [`HarnessEdit`]: the entry id it touched, its before/after
/// snapshots, and whether it actually applied. Kept even when `applied` is false (e.g. an
/// `update`/`delete` naming an id that doesn't exist in the target scope) so a caller can see
/// exactly which edits in a batch landed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppliedHarnessEdit {
    pub edit: HarnessEdit,
    pub id: String,
    pub before: Option<HarnessEntry>,
    pub after: Option<HarnessEntry>,
    pub applied: bool,
    pub error: Option<String>,
}

/// One journaled batch of harness edits: what triggered it, why, and the full before/after
/// snapshot of every entry it touched (so [`Store::rollback_harness_refinement`] never needs to
/// consult `harness_entry`'s current state to invert it).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HarnessRefinement {
    pub id: String,
    pub session_id: String,
    /// `"manual"` | `"auto_turns"` | `"auto_compact"` | `"rollback"`.
    pub trigger: String,
    pub summary: String,
    pub rationale: String,
    pub expected_outcome: String,
    pub edits: Vec<AppliedHarnessEdit>,
    pub created_at: i64,
}

/// Every edit applied through [`Store::apply_harness_edits`] is attributed to this source; there
/// is currently no caller-supplied alternative (a manually authored entry would need a separate
/// entry point, not yet built).
const HARNESS_EDIT_SOURCE: &str = "refine";

impl Store {
    /// Every harness entry across `scopes` (typically `["global", "project:<cwd>",
    /// "session:<id>"]`), ordered by `kind` then most-recently-updated first within each kind —
    /// the order a context-injection pass wants to read them in.
    pub fn harness_entries(&self, scopes: &[&str]) -> Result<Vec<HarnessEntry>> {
        if scopes.is_empty() {
            return Ok(Vec::new());
        }
        let placeholders = (0..scopes.len())
            .map(|i| format!("?{}", i + 1))
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT id, scope, kind, title, content, source, version, created_at, updated_at \
             FROM harness_entry WHERE scope IN ({placeholders}) ORDER BY kind, updated_at DESC"
        );
        let conn = self.lock()?;
        let mut stmt = conn.prepare(&sql)?;
        let params: Vec<&dyn rusqlite::ToSql> =
            scopes.iter().map(|s| s as &dyn rusqlite::ToSql).collect();
        let rows = stmt.query_map(params.as_slice(), row_to_harness_entry)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(StoreError::from)
    }

    /// Apply a batch of edits against `scope` in one transaction and journal the result as a
    /// [`HarnessRefinement`]. Each edit is applied independently: an `update`/`delete` naming an
    /// unknown id records `applied: false` with an error instead of aborting the rest of the
    /// batch, so one bad edit in a model-proposed batch doesn't lose the others.
    #[allow(clippy::too_many_arguments)]
    pub fn apply_harness_edits(
        &self,
        scope: &str,
        session_id: &str,
        trigger: &str,
        summary: &str,
        rationale: &str,
        expected_outcome: &str,
        edits: Vec<HarnessEdit>,
    ) -> Result<HarnessRefinement> {
        with_busy_retry(|| {
            let mut conn = self.lock()?;
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let mut applied = Vec::with_capacity(edits.len());
            for edit in &edits {
                applied.push(apply_one_harness_edit(&tx, scope, edit)?);
            }
            let refinement = insert_harness_refinement(
                &tx,
                session_id,
                trigger,
                summary,
                rationale,
                expected_outcome,
                applied,
            )?;
            tx.commit()?;
            Ok(refinement)
        })
    }

    /// Refinements newest-first, optionally filtered to one session. `None` returns refinements
    /// across every session (e.g. for a global refinement-history view). `rowid DESC` breaks ties
    /// within the same second — `created_at` has 1-second resolution, so a fast burst of
    /// refinements (e.g. a rollback immediately following the refinement it reverts) would
    /// otherwise sort arbitrarily.
    pub fn harness_refinements(
        &self,
        session_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<HarnessRefinement>> {
        let conn = self.lock()?;
        let raw = if let Some(sid) = session_id {
            let mut stmt = conn.prepare(
                "SELECT id, session_id, trigger, summary, rationale, expected_outcome, \
                 edits_json, created_at FROM harness_refinement \
                 WHERE session_id = ?1 ORDER BY created_at DESC, rowid DESC LIMIT ?2",
            )?;
            let rows = stmt
                .query_map(rusqlite::params![sid, limit as i64], row_to_raw_refinement)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            rows
        } else {
            let mut stmt = conn.prepare(
                "SELECT id, session_id, trigger, summary, rationale, expected_outcome, \
                 edits_json, created_at FROM harness_refinement \
                 ORDER BY created_at DESC, rowid DESC LIMIT ?1",
            )?;
            let rows = stmt
                .query_map([limit as i64], row_to_raw_refinement)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            rows
        };
        raw.into_iter().map(raw_into_refinement).collect()
    }

    /// Invert every applied edit of a past refinement — from its journaled before/after
    /// snapshots, never from `harness_entry`'s current state — and journal the inversion as a new
    /// refinement with `trigger = "rollback"`.
    pub fn rollback_harness_refinement(
        &self,
        refinement_id: &str,
        session_id: &str,
    ) -> Result<HarnessRefinement> {
        with_busy_retry(|| {
            let mut conn = self.lock()?;
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let source = get_harness_refinement_on(&tx, refinement_id)?.ok_or_else(|| {
                StoreError::InvalidValue(format!("unknown harness refinement id '{refinement_id}'"))
            })?;
            // Reverse order: if the same entry was created then updated within one refinement,
            // undoing the update before the create keeps every intermediate state consistent.
            let mut inverted = Vec::with_capacity(source.edits.len());
            for applied in source.edits.iter().rev() {
                if !applied.applied {
                    continue;
                }
                inverted.push(invert_harness_edit(&tx, applied)?);
            }
            let refinement = insert_harness_refinement(
                &tx,
                session_id,
                "rollback",
                &format!("rollback of refinement {refinement_id}"),
                &format!("revert the edits applied by refinement {refinement_id}"),
                "harness entries restored to their state before that refinement",
                inverted,
            )?;
            tx.commit()?;
            Ok(refinement)
        })
    }
}

fn apply_one_harness_edit(
    conn: &Connection,
    scope: &str,
    edit: &HarnessEdit,
) -> Result<AppliedHarnessEdit> {
    match edit.action.as_str() {
        "create" => {
            let id = forge_types::new_id();
            let title = edit.title.clone().unwrap_or_default();
            let content = edit.content.clone().unwrap_or_default();
            conn.execute(
                "INSERT INTO harness_entry (id, scope, kind, title, content, source, version) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1)",
                rusqlite::params![
                    &id,
                    scope,
                    &edit.kind,
                    &title,
                    &content,
                    HARNESS_EDIT_SOURCE
                ],
            )?;
            let after = get_harness_entry_on(conn, &id)?;
            append_harness_entry_sync(conn, after.as_ref())?;
            Ok(AppliedHarnessEdit {
                edit: edit.clone(),
                id,
                before: None,
                after,
                applied: true,
                error: None,
            })
        }
        "update" => {
            let Some(id) = edit.id.clone() else {
                return Ok(unapplied(edit, String::new(), "update requires an id"));
            };
            let Some(before) = get_harness_entry_scoped(conn, &id, scope)? else {
                return Ok(unapplied(
                    edit,
                    id,
                    "unknown harness entry id in this scope",
                ));
            };
            let title = edit.title.clone().unwrap_or_else(|| before.title.clone());
            let content = edit
                .content
                .clone()
                .unwrap_or_else(|| before.content.clone());
            conn.execute(
                "UPDATE harness_entry SET title = ?1, content = ?2, version = version + 1, \
                 updated_at = strftime('%s','now') WHERE id = ?3",
                rusqlite::params![&title, &content, &id],
            )?;
            let after = get_harness_entry_on(conn, &id)?;
            append_harness_entry_sync(conn, after.as_ref())?;
            Ok(AppliedHarnessEdit {
                edit: edit.clone(),
                id,
                before: Some(before),
                after,
                applied: true,
                error: None,
            })
        }
        "delete" => {
            let Some(id) = edit.id.clone() else {
                return Ok(unapplied(edit, String::new(), "delete requires an id"));
            };
            let Some(before) = get_harness_entry_scoped(conn, &id, scope)? else {
                return Ok(unapplied(
                    edit,
                    id,
                    "unknown harness entry id in this scope",
                ));
            };
            conn.execute("DELETE FROM harness_entry WHERE id = ?1", [&id])?;
            append_sync_revision(
                conn,
                "harness_entry",
                &id,
                SyncJournalOperation::Tombstone,
                &[],
            )?;
            Ok(AppliedHarnessEdit {
                edit: edit.clone(),
                id,
                before: Some(before),
                after: None,
                applied: true,
                error: None,
            })
        }
        other => Ok(unapplied(
            edit,
            edit.id.clone().unwrap_or_default(),
            &format!("unknown edit action '{other}'"),
        )),
    }
}

fn unapplied(edit: &HarnessEdit, id: String, error: &str) -> AppliedHarnessEdit {
    AppliedHarnessEdit {
        edit: edit.clone(),
        id,
        before: None,
        after: None,
        applied: false,
        error: Some(error.to_string()),
    }
}

/// Invert one applied edit from its journaled snapshot: a create is undone by deleting, an
/// update by restoring the exact `before` snapshot (including its version), a delete by
/// recreating the entry verbatim at its original id.
fn invert_harness_edit(
    conn: &Connection,
    applied: &AppliedHarnessEdit,
) -> Result<AppliedHarnessEdit> {
    match (&applied.before, &applied.after) {
        (None, Some(created)) => {
            conn.execute("DELETE FROM harness_entry WHERE id = ?1", [&applied.id])?;
            append_sync_revision(
                conn,
                "harness_entry",
                &applied.id,
                SyncJournalOperation::Tombstone,
                &[],
            )?;
            Ok(AppliedHarnessEdit {
                edit: HarnessEdit {
                    action: "delete".to_string(),
                    kind: created.kind.clone(),
                    id: Some(applied.id.clone()),
                    title: None,
                    content: None,
                    reason: Some("rollback".to_string()),
                },
                id: applied.id.clone(),
                before: Some(created.clone()),
                after: None,
                applied: true,
                error: None,
            })
        }
        (Some(before), Some(current)) => {
            conn.execute(
                "UPDATE harness_entry SET title = ?1, content = ?2, version = ?3, \
                 updated_at = strftime('%s','now') WHERE id = ?4",
                rusqlite::params![&before.title, &before.content, before.version, &applied.id],
            )?;
            let restored = get_harness_entry_on(conn, &applied.id)?;
            append_harness_entry_sync(conn, restored.as_ref())?;
            Ok(AppliedHarnessEdit {
                edit: HarnessEdit {
                    action: "update".to_string(),
                    kind: before.kind.clone(),
                    id: Some(applied.id.clone()),
                    title: Some(before.title.clone()),
                    content: Some(before.content.clone()),
                    reason: Some("rollback".to_string()),
                },
                id: applied.id.clone(),
                before: Some(current.clone()),
                after: restored,
                applied: true,
                error: None,
            })
        }
        (Some(before), None) => {
            conn.execute(
                "INSERT INTO harness_entry \
                 (id, scope, kind, title, content, source, version, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                rusqlite::params![
                    &applied.id,
                    &before.scope,
                    &before.kind,
                    &before.title,
                    &before.content,
                    &before.source,
                    before.version,
                    before.created_at,
                    before.updated_at,
                ],
            )?;
            let after = get_harness_entry_on(conn, &applied.id)?;
            append_harness_entry_sync(conn, after.as_ref())?;
            Ok(AppliedHarnessEdit {
                edit: HarnessEdit {
                    action: "create".to_string(),
                    kind: before.kind.clone(),
                    id: Some(applied.id.clone()),
                    title: Some(before.title.clone()),
                    content: Some(before.content.clone()),
                    reason: Some("rollback".to_string()),
                },
                id: applied.id.clone(),
                before: None,
                after,
                applied: true,
                error: None,
            })
        }
        (None, None) => Ok(unapplied(
            &applied.edit,
            applied.id.clone(),
            "nothing to invert",
        )),
    }
}

fn insert_harness_refinement(
    conn: &Connection,
    session_id: &str,
    trigger: &str,
    summary: &str,
    rationale: &str,
    expected_outcome: &str,
    edits: Vec<AppliedHarnessEdit>,
) -> Result<HarnessRefinement> {
    let id = forge_types::new_id();
    let edits_json = serde_json::to_string(&edits).map_err(|e| StoreError::Json(e.to_string()))?;
    conn.execute(
        "INSERT INTO harness_refinement \
         (id, session_id, trigger, summary, rationale, expected_outcome, edits_json) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        rusqlite::params![
            &id,
            session_id,
            trigger,
            summary,
            rationale,
            expected_outcome,
            &edits_json
        ],
    )?;
    let created_at: i64 = conn.query_row(
        "SELECT created_at FROM harness_refinement WHERE id = ?1",
        [&id],
        |r| r.get(0),
    )?;
    let payload = sync_json(serde_json::json!({
        "id": id,
        "session_id": session_id,
        "trigger": trigger,
        "summary": summary,
        "rationale": rationale,
        "expected_outcome": expected_outcome,
        "edits_json": edits_json,
        "created_at": created_at,
    }))?;
    append_sync_revision(
        conn,
        "harness_refinement",
        &id,
        SyncJournalOperation::Upsert,
        &payload,
    )?;
    Ok(HarnessRefinement {
        id,
        session_id: session_id.to_string(),
        trigger: trigger.to_string(),
        summary: summary.to_string(),
        rationale: rationale.to_string(),
        expected_outcome: expected_outcome.to_string(),
        edits,
        created_at,
    })
}

fn append_harness_entry_sync(conn: &Connection, entry: Option<&HarnessEntry>) -> Result<()> {
    let Some(entry) = entry else { return Ok(()) };
    let payload = sync_json(serde_json::json!({
        "id": entry.id,
        "scope": entry.scope,
        "kind": entry.kind,
        "title": entry.title,
        "content": entry.content,
        "source": entry.source,
        "version": entry.version,
        "created_at": entry.created_at,
        "updated_at": entry.updated_at,
    }))?;
    append_sync_revision(
        conn,
        "harness_entry",
        &entry.id,
        SyncJournalOperation::Upsert,
        &payload,
    )
}

fn get_harness_entry_on(conn: &Connection, id: &str) -> Result<Option<HarnessEntry>> {
    conn.query_row(
        "SELECT id, scope, kind, title, content, source, version, created_at, updated_at \
         FROM harness_entry WHERE id = ?1",
        [id],
        row_to_harness_entry,
    )
    .optional()
    .map_err(StoreError::from)
}

fn get_harness_entry_scoped(
    conn: &Connection,
    id: &str,
    scope: &str,
) -> Result<Option<HarnessEntry>> {
    conn.query_row(
        "SELECT id, scope, kind, title, content, source, version, created_at, updated_at \
         FROM harness_entry WHERE id = ?1 AND scope = ?2",
        rusqlite::params![id, scope],
        row_to_harness_entry,
    )
    .optional()
    .map_err(StoreError::from)
}

fn row_to_harness_entry(r: &rusqlite::Row) -> rusqlite::Result<HarnessEntry> {
    Ok(HarnessEntry {
        id: r.get(0)?,
        scope: r.get(1)?,
        kind: r.get(2)?,
        title: r.get(3)?,
        content: r.get(4)?,
        source: r.get(5)?,
        version: r.get(6)?,
        created_at: r.get(7)?,
        updated_at: r.get(8)?,
    })
}

/// The raw row shape of `harness_refinement`, before `edits_json` is decoded — decoding can fail
/// (`StoreError::Json`), which `rusqlite::Row` mapping closures can't return, so it happens as a
/// separate step in [`raw_into_refinement`].
struct RawHarnessRefinement {
    id: String,
    session_id: String,
    trigger: String,
    summary: String,
    rationale: String,
    expected_outcome: String,
    edits_json: String,
    created_at: i64,
}

fn row_to_raw_refinement(r: &rusqlite::Row) -> rusqlite::Result<RawHarnessRefinement> {
    Ok(RawHarnessRefinement {
        id: r.get(0)?,
        session_id: r.get(1)?,
        trigger: r.get(2)?,
        summary: r.get(3)?,
        rationale: r.get(4)?,
        expected_outcome: r.get(5)?,
        edits_json: r.get(6)?,
        created_at: r.get(7)?,
    })
}

fn raw_into_refinement(raw: RawHarnessRefinement) -> Result<HarnessRefinement> {
    let edits: Vec<AppliedHarnessEdit> =
        serde_json::from_str(&raw.edits_json).map_err(|e| StoreError::Json(e.to_string()))?;
    Ok(HarnessRefinement {
        id: raw.id,
        session_id: raw.session_id,
        trigger: raw.trigger,
        summary: raw.summary,
        rationale: raw.rationale,
        expected_outcome: raw.expected_outcome,
        edits,
        created_at: raw.created_at,
    })
}

fn get_harness_refinement_on(conn: &Connection, id: &str) -> Result<Option<HarnessRefinement>> {
    let raw = conn
        .query_row(
            "SELECT id, session_id, trigger, summary, rationale, expected_outcome, edits_json, \
             created_at FROM harness_refinement WHERE id = ?1",
            [id],
            row_to_raw_refinement,
        )
        .optional()?;
    raw.map(raw_into_refinement).transpose()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Store;

    fn store() -> Store {
        Store::open_in_memory().unwrap()
    }

    fn create_edit(kind: &str, title: &str, content: &str) -> HarnessEdit {
        HarnessEdit {
            action: "create".to_string(),
            kind: kind.to_string(),
            id: None,
            title: Some(title.to_string()),
            content: Some(content.to_string()),
            reason: Some("test".to_string()),
        }
    }

    #[test]
    fn create_update_delete_round_trip() {
        let s = store();
        let refinement = s
            .apply_harness_edits(
                "global",
                "sess1",
                "manual",
                "seed a prompt",
                "because",
                "a durable prompt entry",
                vec![create_edit("prompt", "greeting", "say hello")],
            )
            .unwrap();
        assert_eq!(refinement.edits.len(), 1);
        assert!(refinement.edits[0].applied);
        let id = refinement.edits[0].id.clone();

        let entries = s.harness_entries(&["global"]).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].version, 1);
        assert_eq!(entries[0].content, "say hello");

        let update = HarnessEdit {
            action: "update".to_string(),
            kind: "prompt".to_string(),
            id: Some(id.clone()),
            title: None,
            content: Some("say hello warmly".to_string()),
            reason: None,
        };
        let refinement2 = s
            .apply_harness_edits(
                "global",
                "sess1",
                "manual",
                "tune the greeting",
                "because",
                "a warmer prompt",
                vec![update],
            )
            .unwrap();
        assert!(refinement2.edits[0].applied);
        let after_update = s.harness_entries(&["global"]).unwrap();
        assert_eq!(after_update.len(), 1);
        assert_eq!(after_update[0].version, 2);
        assert_eq!(after_update[0].content, "say hello warmly");
        assert_eq!(after_update[0].title, "greeting");

        let delete = HarnessEdit {
            action: "delete".to_string(),
            kind: "prompt".to_string(),
            id: Some(id),
            title: None,
            content: None,
            reason: None,
        };
        s.apply_harness_edits(
            "global",
            "sess1",
            "manual",
            "remove the greeting",
            "because",
            "no more greeting",
            vec![delete],
        )
        .unwrap();
        assert!(s.harness_entries(&["global"]).unwrap().is_empty());
    }

    #[test]
    fn scope_isolation() {
        let s = store();
        s.apply_harness_edits(
            "global",
            "sess1",
            "manual",
            "global entry",
            "because",
            "outcome",
            vec![create_edit("prompt", "g", "global content")],
        )
        .unwrap();
        s.apply_harness_edits(
            "session:sess1",
            "sess1",
            "manual",
            "session entry",
            "because",
            "outcome",
            vec![create_edit("prompt", "s", "session content")],
        )
        .unwrap();

        assert_eq!(s.harness_entries(&["global"]).unwrap().len(), 1);
        assert_eq!(s.harness_entries(&["session:sess1"]).unwrap().len(), 1);
        assert_eq!(
            s.harness_entries(&["global", "session:sess1"])
                .unwrap()
                .len(),
            2
        );

        // An update against the wrong scope must not cross into another scope's entry.
        let global_id = s.harness_entries(&["global"]).unwrap()[0].id.clone();
        let cross_scope_update = HarnessEdit {
            action: "update".to_string(),
            kind: "prompt".to_string(),
            id: Some(global_id),
            title: None,
            content: Some("hijacked".to_string()),
            reason: None,
        };
        let refinement = s
            .apply_harness_edits(
                "session:sess1",
                "sess1",
                "manual",
                "attempted cross-scope update",
                "because",
                "outcome",
                vec![cross_scope_update],
            )
            .unwrap();
        assert!(!refinement.edits[0].applied);
        assert!(refinement.edits[0].error.is_some());
        assert_eq!(
            s.harness_entries(&["global"]).unwrap()[0].content,
            "global content"
        );
    }

    #[test]
    fn rollback_restores_exact_prior_state_including_version() {
        let s = store();
        let created = s
            .apply_harness_edits(
                "global",
                "sess1",
                "manual",
                "create",
                "because",
                "outcome",
                vec![create_edit("skill", "title", "v1 content")],
            )
            .unwrap();
        let id = created.edits[0].id.clone();

        let update = HarnessEdit {
            action: "update".to_string(),
            kind: "skill".to_string(),
            id: Some(id.clone()),
            title: None,
            content: Some("v2 content".to_string()),
            reason: None,
        };
        let updated = s
            .apply_harness_edits(
                "global",
                "sess1",
                "manual",
                "update",
                "because",
                "outcome",
                vec![update],
            )
            .unwrap();
        assert_eq!(s.harness_entries(&["global"]).unwrap()[0].version, 2);

        let rollback = s.rollback_harness_refinement(&updated.id, "sess1").unwrap();
        assert_eq!(rollback.trigger, "rollback");
        assert!(rollback.edits[0].applied);

        let restored = s.harness_entries(&["global"]).unwrap();
        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].id, id);
        assert_eq!(
            restored[0].version, 1,
            "rollback must restore the exact prior version"
        );
        assert_eq!(restored[0].content, "v1 content");
    }

    #[test]
    fn rollback_of_create_deletes_the_entry() {
        let s = store();
        let created = s
            .apply_harness_edits(
                "global",
                "sess1",
                "manual",
                "create",
                "because",
                "outcome",
                vec![create_edit("subagent", "title", "content")],
            )
            .unwrap();
        assert_eq!(s.harness_entries(&["global"]).unwrap().len(), 1);

        s.rollback_harness_refinement(&created.id, "sess1").unwrap();
        assert!(s.harness_entries(&["global"]).unwrap().is_empty());
    }

    #[test]
    fn rollback_of_delete_recreates_the_entry_verbatim() {
        let s = store();
        let created = s
            .apply_harness_edits(
                "global",
                "sess1",
                "manual",
                "create",
                "because",
                "outcome",
                vec![create_edit("subagent", "title", "content")],
            )
            .unwrap();
        let id = created.edits[0].id.clone();
        let original = s.harness_entries(&["global"]).unwrap()[0].clone();

        let delete = HarnessEdit {
            action: "delete".to_string(),
            kind: "subagent".to_string(),
            id: Some(id.clone()),
            title: None,
            content: None,
            reason: None,
        };
        let deleted = s
            .apply_harness_edits(
                "global",
                "sess1",
                "manual",
                "delete",
                "because",
                "outcome",
                vec![delete],
            )
            .unwrap();
        assert!(s.harness_entries(&["global"]).unwrap().is_empty());

        s.rollback_harness_refinement(&deleted.id, "sess1").unwrap();
        let restored = s.harness_entries(&["global"]).unwrap();
        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].id, id);
        assert_eq!(restored[0].version, original.version);
        assert_eq!(restored[0].content, original.content);
        assert_eq!(restored[0].created_at, original.created_at);
    }

    #[test]
    fn unknown_id_edit_is_recorded_but_does_not_abort_the_batch() {
        let s = store();
        let bogus_update = HarnessEdit {
            action: "update".to_string(),
            kind: "prompt".to_string(),
            id: Some("does-not-exist".to_string()),
            title: None,
            content: Some("x".to_string()),
            reason: None,
        };
        let refinement = s
            .apply_harness_edits(
                "global",
                "sess1",
                "manual",
                "mixed batch",
                "because",
                "outcome",
                vec![bogus_update, create_edit("prompt", "ok", "ok content")],
            )
            .unwrap();
        assert_eq!(refinement.edits.len(), 2);
        assert!(!refinement.edits[0].applied);
        assert!(refinement.edits[0].error.is_some());
        assert!(refinement.edits[1].applied);
        assert_eq!(s.harness_entries(&["global"]).unwrap().len(), 1);
    }

    #[test]
    fn refinements_listing_order_and_session_filter() {
        let s = store();
        s.apply_harness_edits(
            "global",
            "sess1",
            "manual",
            "first",
            "because",
            "outcome",
            vec![create_edit("prompt", "a", "a content")],
        )
        .unwrap();
        s.apply_harness_edits(
            "global",
            "sess2",
            "manual",
            "second",
            "because",
            "outcome",
            vec![create_edit("prompt", "b", "b content")],
        )
        .unwrap();
        s.apply_harness_edits(
            "global",
            "sess1",
            "manual",
            "third",
            "because",
            "outcome",
            vec![create_edit("prompt", "c", "c content")],
        )
        .unwrap();

        let all = s.harness_refinements(None, 10).unwrap();
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].summary, "third", "newest first");

        let sess1_only = s.harness_refinements(Some("sess1"), 10).unwrap();
        assert_eq!(sess1_only.len(), 2);
        assert!(sess1_only.iter().all(|r| r.session_id == "sess1"));

        let limited = s.harness_refinements(None, 1).unwrap();
        assert_eq!(limited.len(), 1);
        assert_eq!(limited[0].summary, "third");
    }
}
