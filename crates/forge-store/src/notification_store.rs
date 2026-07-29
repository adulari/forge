//! Web Push, APNs, and Live Activity destination persistence.

use super::*;

impl Store {
    /// Store (or refresh) a Web Push subscription, deduplicating by `endpoint` — a browser
    /// re-subscribing after a permission round-trip must update its keys in place, never pile
    /// up duplicate rows that would each receive (and each decrypt-fail or double-notify) every
    /// push. Atomic: a single `INSERT … ON CONFLICT(endpoint) DO UPDATE` against the UNIQUE index
    /// `idx_push_subscription_endpoint` (migration #13), so concurrent callers can't race a
    /// duplicate in between a SELECT and an INSERT. Returns the row id (existing or new).
    pub fn upsert_push_subscription(
        &self,
        endpoint: &str,
        p256dh: &str,
        auth: &str,
    ) -> Result<String> {
        let conn = self.lock()?;
        let id = forge_types::new_id();
        let row_id = conn.query_row(
            "INSERT INTO push_subscription (id, endpoint, p256dh, auth) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(endpoint) DO UPDATE SET p256dh = excluded.p256dh, auth = excluded.auth
             RETURNING id",
            (&id, endpoint, p256dh, auth),
            |row| row.get::<_, String>(0),
        )?;
        Ok(row_id)
    }

    /// Remove a Web Push subscription by its endpoint (unsubscribe, or a push service answering
    /// 404/410). `Ok(true)` when a row was actually deleted.
    pub fn delete_push_subscription(&self, endpoint: &str) -> Result<bool> {
        let n = self.lock()?.execute(
            "DELETE FROM push_subscription WHERE endpoint = ?1",
            [endpoint],
        )?;
        Ok(n > 0)
    }

    /// Every stored Web Push subscription, oldest first (delivery order is stable and boring).
    pub fn list_push_subscriptions(&self) -> Result<Vec<PushSubscription>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT id, endpoint, p256dh, auth FROM push_subscription ORDER BY created_at, id",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok(PushSubscription {
                    id: row.get(0)?,
                    endpoint: row.get(1)?,
                    p256dh: row.get(2)?,
                    auth: row.get(3)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Store (or refresh) an APNs subscription, deduplicating by `device_token`. Atomic: the
    /// unique device-token index makes concurrent registration update one row in place.
    pub fn upsert_apns_subscription(
        &self,
        device_token: &str,
        environment: &str,
    ) -> Result<String> {
        let conn = self.lock()?;
        let id = forge_types::new_id();
        let row_id = conn.query_row(
            "INSERT INTO apns_subscription (id, device_token, environment) VALUES (?1, ?2, ?3)
             ON CONFLICT(device_token) DO UPDATE SET environment = excluded.environment
             RETURNING id",
            (&id, device_token, environment),
            |row| row.get::<_, String>(0),
        )?;
        Ok(row_id)
    }

    /// Remove an APNs subscription by its device token (unsubscribe, or APNs answering
    /// `BadDeviceToken`/`Unregistered`). `Ok(true)` when a row was actually deleted.
    pub fn delete_apns_subscription(&self, device_token: &str) -> Result<bool> {
        let n = self.lock()?.execute(
            "DELETE FROM apns_subscription WHERE device_token = ?1",
            [device_token],
        )?;
        Ok(n > 0)
    }

    /// Every stored APNs subscription, oldest first.
    pub fn list_apns_subscriptions(&self) -> Result<Vec<ApnsSubscription>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT id, device_token, environment FROM apns_subscription ORDER BY created_at, id",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok(ApnsSubscription {
                    id: row.get(0)?,
                    device_token: row.get(1)?,
                    environment: row.get(2)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Store (or refresh) a session's Live Activity remote-update push token. Keyed by
    /// `session_id` (the table's primary key), so a re-registration for the same session
    /// replaces the existing token/environment in place rather than adding a row.
    pub fn upsert_live_activity_token(
        &self,
        session_id: &str,
        push_token: &str,
        environment: &str,
    ) -> Result<()> {
        self.lock()?.execute(
            "INSERT INTO live_activity_token (session_id, push_token, environment)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(session_id) DO UPDATE SET
                push_token = excluded.push_token,
                environment = excluded.environment,
                updated_at = strftime('%s','now')",
            (session_id, push_token, environment),
        )?;
        Ok(())
    }

    /// Remove a session's Live Activity push token (the activity ended). `Ok(true)` when a row
    /// was actually deleted.
    pub fn delete_live_activity_token(&self, session_id: &str) -> Result<bool> {
        let n = self.lock()?.execute(
            "DELETE FROM live_activity_token WHERE session_id = ?1",
            [session_id],
        )?;
        Ok(n > 0)
    }

    /// A session's stored Live Activity push token, if any.
    pub fn get_live_activity_token(&self, session_id: &str) -> Result<Option<LiveActivityToken>> {
        self.lock()?
            .query_row(
                "SELECT session_id, push_token, environment FROM live_activity_token
                 WHERE session_id = ?1",
                [session_id],
                |row| {
                    Ok(LiveActivityToken {
                        session_id: row.get(0)?,
                        push_token: row.get(1)?,
                        environment: row.get(2)?,
                    })
                },
            )
            .optional()
            .map_err(StoreError::from)
    }
}
