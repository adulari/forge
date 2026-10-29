//! The extra shapes of `GET /api/history` the app's Find, prompt rail and jump-to-message use,
//! kept out of `serve.rs`: a text/role filter, a window centred on a row, and the integrity probe.

/// How many sequence numbers past the requested row an `around` window reaches, so the row sits
/// a little above the bottom of the page instead of on its last line.
const AROUND_AHEAD: i64 = 12;
/// Longest search text honoured; anything past it cannot be a sensible query.
const NEEDLE_CHARS: usize = 200;

/// The `before` cursor a request resolves to. `around=<seq>` asks for the page that ends just
/// past `seq`, which is the same newest-first page `before` opens, so a client pages onward from
/// it with the ordinary cursor.
pub(crate) fn window_before(before: Option<i64>, around: Option<i64>) -> Option<i64> {
    match around {
        Some(seq) => Some(seq.saturating_add(AROUND_AHEAD + 1)),
        None => before,
    }
}

/// The search text, trimmed and bounded; `None` when there is nothing to search for.
pub(crate) fn needle(raw: Option<&str>) -> Option<String> {
    let text = raw?.trim();
    (!text.is_empty()).then(|| text.chars().take(NEEDLE_CHARS).collect())
}

/// The stored role a request may narrow to. Only conversation roles; `tool` and `system` rows are
/// not something a client searches or outlines.
pub(crate) fn role(raw: Option<&str>) -> Option<&'static str> {
    match raw?.trim() {
        "user" => Some("user"),
        "assistant" => Some("assistant"),
        _ => None,
    }
}

/// The body of `?integrity=1`: `history_gap` is null for a whole session.
pub(crate) fn integrity_body(missing_rows: u64) -> serde_json::Value {
    let gap = (missing_rows > 0)
        .then(|| serde_json::json!({ "count": missing_rows, "reason": "missing_rows" }));
    serde_json::json!({ "history_gap": gap })
}

/// `?integrity=1`: counts the session's missing rows on the blocking pool.
pub(crate) async fn integrity(
    store: std::sync::Arc<forge_store::Store>,
    sid: String,
) -> axum::response::Response {
    let missing = tokio::task::spawn_blocking(move || store.history_missing_rows(&sid))
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or(0);
    crate::serve::json_response(&integrity_body(missing))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn around_opens_the_page_that_ends_past_the_row() {
        assert_eq!(window_before(Some(7), None), Some(7));
        assert_eq!(window_before(Some(7), Some(100)), Some(113));
        assert_eq!(window_before(None, Some(i64::MAX)), Some(i64::MAX));
    }

    #[test]
    fn an_empty_or_blank_needle_is_no_search() {
        assert_eq!(needle(None), None);
        assert_eq!(needle(Some("   ")), None);
        assert_eq!(needle(Some("  bug ")).as_deref(), Some("bug"));
        assert_eq!(needle(Some(&"x".repeat(500))).unwrap().chars().count(), 200);
    }

    #[test]
    fn only_conversation_roles_narrow_a_page() {
        assert_eq!(role(Some("user")), Some("user"));
        assert_eq!(role(Some("assistant")), Some("assistant"));
        assert_eq!(role(Some("tool")), None);
        assert_eq!(role(Some("system")), None);
    }

    #[test]
    fn the_integrity_probe_names_missing_rows_only_when_there_are_some() {
        assert!(integrity_body(0)["history_gap"].is_null());
        let body = integrity_body(3);
        assert_eq!(body["history_gap"]["count"], 3);
        assert_eq!(body["history_gap"]["reason"], "missing_rows");
    }
}
