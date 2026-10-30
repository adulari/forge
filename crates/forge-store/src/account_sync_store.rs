//! Reconcile bookkeeping for Anywhere account sync. Rows hold labels and keyed hashes, never values.

use super::*;

/// One tracked account-sync item on this device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountSyncItem {
    pub category: String,
    pub item_id: String,
    pub item: String,
    pub label: String,
    pub local_hash: String,
    pub remote_clock: u64,
    pub applied_at: i64,
    pub status: String,
    pub detail: String,
}

impl Store {
    pub fn account_sync_items(&self, category: Option<&str>) -> Result<Vec<AccountSyncItem>> {
        let conn = self.lock()?;
        let mut statement = conn.prepare(
            "SELECT category, item_id, item, label, local_hash, remote_clock, applied_at, status, detail
             FROM anywhere_account_sync_item WHERE (?1 IS NULL OR category = ?1)
             ORDER BY category, label, item_id",
        )?;
        let rows = statement
            .query_map([category], |row| {
                Ok(AccountSyncItem {
                    category: row.get(0)?,
                    item_id: row.get(1)?,
                    item: row.get(2)?,
                    label: row.get(3)?,
                    local_hash: row.get(4)?,
                    remote_clock: row.get::<_, i64>(5)?.max(0) as u64,
                    applied_at: row.get(6)?,
                    status: row.get(7)?,
                    detail: row.get(8)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn upsert_account_sync_item(&self, item: &AccountSyncItem) -> Result<()> {
        self.lock()?.execute(
            "INSERT INTO anywhere_account_sync_item
             (category, item_id, item, label, local_hash, remote_clock, applied_at, status, detail)
             VALUES (?1, ?2, ?9, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(category, item_id) DO UPDATE SET
               item = excluded.item, label = excluded.label, local_hash = excluded.local_hash,
               remote_clock = excluded.remote_clock, applied_at = excluded.applied_at,
               status = excluded.status, detail = excluded.detail",
            rusqlite::params![
                item.category,
                item.item_id,
                item.label,
                item.local_hash,
                item.remote_clock as i64,
                item.applied_at,
                item.status,
                item.detail,
                item.item
            ],
        )?;
        Ok(())
    }

    pub fn delete_account_sync_item(&self, category: &str, item_id: &str) -> Result<()> {
        self.lock()?.execute(
            "DELETE FROM anywhere_account_sync_item WHERE category = ?1 AND item_id = ?2",
            (category, item_id),
        )?;
        Ok(())
    }

    /// Every portable record of one kind, tombstones included.
    pub fn portable_sync_records(&self, record_kind: &str) -> Result<Vec<PortableSyncRecord>> {
        let ids: Vec<String> = {
            let conn = self.lock()?;
            let mut statement = conn.prepare(
                "SELECT stable_id FROM anywhere_sync_portable_record WHERE record_kind = ?1",
            )?;
            let ids = statement
                .query_map([record_kind], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            ids
        };
        let mut records = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(record) = self.portable_sync_record(record_kind, &id)? {
                records.push(record);
            }
        }
        Ok(records)
    }

    /// Cleartext record-kind counts per sending device, from the staged remote feed. Used to tell
    /// the user what other devices sync without decrypting anything new.
    pub fn account_sync_remote_kinds(&self) -> Result<Vec<(String, i64)>> {
        let conn = self.lock()?;
        let mut statement = conn.prepare(
            "SELECT record_kind, COUNT(DISTINCT stable_id) FROM anywhere_sync_remote
             WHERE record_kind IN ('user_setting', 'skill', 'command', 'agent')
             GROUP BY record_kind",
        )?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Whether the sync journal is recording (Anywhere sync on) and how many rows await upload.
    pub fn pending_account_sync_uploads(&self) -> Result<usize> {
        let conn = self.lock()?;
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM sync_journal WHERE uploaded_at IS NULL
             AND record_kind IN ('user_setting', 'skill', 'command', 'agent')",
            [],
            |row| row.get(0),
        )?;
        Ok(count.max(0) as usize)
    }
}
