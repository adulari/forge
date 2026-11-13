//! Persisted key verdicts: which providers have refused their credential, why, and since when.
//! The status is the opaque word `forge_provider::KeyFailure` writes; the store only keeps it.

use crate::{Result, Store};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ProviderVerdict {
    pub provider: String,
    pub status: String,
    pub reason: String,
    pub first_seen: i64,
    pub last_seen: i64,
}

/// What recording a verdict did, so callers log only a change, not every round.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerdictChange {
    New,
    Changed,
    Unchanged,
}

impl Store {
    /// Record that `provider` refused its key. `first_seen` survives repeats of the same status;
    /// a different status starts a new verdict.
    pub fn set_provider_verdict(
        &self,
        provider: &str,
        status: &str,
        reason: &str,
        now: i64,
    ) -> Result<VerdictChange> {
        let conn = self.lock()?;
        let existing: Option<(String, String)> = conn
            .query_row(
                "SELECT status, reason FROM provider_verdict WHERE provider = ?1",
                [provider],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .ok();
        let change = match &existing {
            None => VerdictChange::New,
            Some((s, r)) if s == status && r == reason => VerdictChange::Unchanged,
            Some(_) => VerdictChange::Changed,
        };
        match existing {
            Some((s, _)) if s == status => {
                conn.execute(
                    "UPDATE provider_verdict SET reason = ?2, last_seen = ?3 WHERE provider = ?1",
                    rusqlite::params![provider, reason, now],
                )?;
            }
            _ => {
                conn.execute(
                    "INSERT INTO provider_verdict (provider, status, reason, first_seen, last_seen)
                     VALUES (?1, ?2, ?3, ?4, ?4)
                     ON CONFLICT(provider) DO UPDATE SET status = excluded.status,
                       reason = excluded.reason, first_seen = excluded.first_seen,
                       last_seen = excluded.last_seen",
                    rusqlite::params![provider, status, reason, now],
                )?;
            }
        }
        Ok(change)
    }

    /// Drop `provider`'s verdict; whether there was one.
    pub fn clear_provider_verdict(&self, provider: &str) -> Result<bool> {
        Ok(self.lock()?.execute(
            "DELETE FROM provider_verdict WHERE provider = ?1",
            [provider],
        )? > 0)
    }

    pub fn provider_verdicts(&self) -> Result<Vec<ProviderVerdict>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT provider, status, reason, first_seen, last_seen FROM provider_verdict
             ORDER BY provider",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(ProviderVerdict {
                provider: row.get(0)?,
                status: row.get(1)?,
                reason: row.get(2)?,
                first_seen: row.get(3)?,
                last_seen: row.get(4)?,
            })
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_verdict_keeps_its_first_seen_until_the_status_changes_or_it_clears() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(
            store
                .set_provider_verdict("opencode", "expired", "key expired", 100)
                .unwrap(),
            VerdictChange::New
        );
        assert_eq!(
            store
                .set_provider_verdict("opencode", "expired", "key expired", 200)
                .unwrap(),
            VerdictChange::Unchanged
        );
        let v = &store.provider_verdicts().unwrap()[0];
        assert_eq!((v.first_seen, v.last_seen), (100, 200));
        assert_eq!(
            store
                .set_provider_verdict("opencode", "rejected", "revoked", 300)
                .unwrap(),
            VerdictChange::Changed
        );
        let v = &store.provider_verdicts().unwrap()[0];
        assert_eq!((v.status.as_str(), v.first_seen), ("rejected", 300));
        assert!(store.clear_provider_verdict("opencode").unwrap());
        assert!(!store.clear_provider_verdict("opencode").unwrap());
        assert!(store.provider_verdicts().unwrap().is_empty());
    }
}
