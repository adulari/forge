//! Persistence of an assistant message's provider-native items (migration #38).

use super::*;

impl Store {
    /// Attach `items` to an already-stored message so a resumed session replays them. Takes the
    /// message id rather than widening the insert signatures every caller shares.
    pub fn set_message_provider_items(
        &self,
        message_id: &str,
        items: &forge_types::ProviderItems,
    ) -> Result<()> {
        let json = serde_json::to_string(items).map_err(|e| StoreError::Json(e.to_string()))?;
        self.lock()?.execute(
            "UPDATE message SET provider_items_json = ?2 WHERE id = ?1",
            (message_id, json),
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_items_round_trip_through_the_store() {
        let store = Store::open_in_memory().unwrap();
        let sid = store.create_session("/tmp", "default").unwrap();
        store.add_message(&sid, 0, Role::User, "hi", None).unwrap();
        let reply = store
            .add_message(&sid, 1, Role::Assistant, "ok", Some("codex-oauth::m"))
            .unwrap();
        let items = forge_types::ProviderItems {
            source: "codex-oauth::m".into(),
            items: vec![serde_json::json!({"type": "reasoning", "encrypted_content": "x"})],
        };
        store.set_message_provider_items(&reply, &items).unwrap();

        let loaded = store.load_messages(&sid).unwrap();
        assert!(loaded[0].provider_items.is_none());
        assert_eq!(loaded[1].provider_items.as_ref(), Some(&items));
        let all = store.load_all_messages(&sid).unwrap();
        assert_eq!(all[1].provider_items.as_ref(), Some(&items));
    }
}
