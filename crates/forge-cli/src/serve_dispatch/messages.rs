//! The `[dispatch]` messages the daemon queues for a coordinator, and how they are delivered.

use super::*;

/// The `[dispatch]` approval message, from the row after the first scheduling pass.
pub(super) fn approval_text(row: &DispatchRow, advanced: &Advance) -> String {
    let title = |idx: i64| {
        row.items
            .iter()
            .find(|i| i.idx == idx)
            .map_or("", |i| i.title.as_str())
    };
    let satisfied = |idx: &i64| {
        row.items.iter().any(|i| {
            i.idx == *idx && (i.status == item_status::SUCCEEDED || i.status == item_status::MERGED)
        })
    };
    let started: Vec<(usize, &str, &str)> = advanced
        .started
        .iter()
        .map(|(idx, sid)| (*idx as usize, title(*idx), sid.as_str()))
        .collect();
    let waiting_deps: Vec<(usize, &str, Vec<usize>)> = row
        .items
        .iter()
        .filter(|i| i.status == item_status::QUEUED)
        .map(|i| {
            let deps = i
                .depends_on
                .iter()
                .filter(|d| !satisfied(d))
                .map(|d| *d as usize)
                .collect();
            (i.idx as usize, i.title.as_str(), deps)
        })
        .collect();
    let waiting: Vec<(usize, &str, &[usize])> = waiting_deps
        .iter()
        .map(|(n, t, d)| (*n, *t, d.as_slice()))
        .collect();
    let failed: Vec<i64> = advanced.start_failed.iter().map(|(idx, _)| *idx).collect();
    let not_started: Vec<(usize, &str)> = row
        .items
        .iter()
        .filter(|i| {
            i.status == item_status::SKIPPED
                || i.status == item_status::CANCELLED
                || failed.contains(&i.idx)
        })
        .map(|i| (i.idx as usize, i.title.as_str()))
        .collect();
    plan::approved_message(&started, &waiting, &not_started)
}

/// Queue `text` for the coordinator through the fleet message queue (it survives a busy or
/// not-yet-live coordinator), then deliver whatever is pending.
///
/// At the per-sender pending cap the newest undelivered dispatch message is folded together with
/// `text` into one new message instead of dropping the update: the old row is marked delivered to
/// free its slot, then the combined row is queued. The two writes are not atomic; a crash between
/// them loses that one older update, which is still better than losing every update past the cap.
pub(crate) async fn send_to_coordinator(
    store: &Store,
    registry: &SessionRegistry,
    coordinator_id: &str,
    text: &str,
) {
    let pending: Vec<_> = store
        .pending_fleet_messages_for(coordinator_id)
        .unwrap_or_default()
        .into_iter()
        .filter(|m| m.sender_label == SENDER_LABEL)
        .collect();
    let mut body = text.to_string();
    if pending.len() >= FLEET_PENDING_CAP {
        if let Some(newest) = pending.last() {
            let now = chrono::Utc::now().timestamp();
            if matches!(
                store.mark_fleet_message_delivered(&newest.id, now),
                Ok(true)
            ) {
                body = combine_messages(&newest.body, text);
            }
        }
    }
    let id = forge_types::new_id();
    if let Err(error) = store.enqueue_fleet_message(
        &id,
        SENDER_LABEL,
        None,
        SENDER_LABEL,
        coordinator_id,
        &body,
        "follow_up",
    ) {
        tracing::warn!(coordinator = %coordinator_id, %error, "dispatch: could not queue a coordinator message");
    }
    deliver_pending_fleet_messages(store, registry, coordinator_id).await;
}

/// Two queued updates as one message, newest kept whole. Over the fleet body limit the OLDER text
/// loses its beginning, since the latest state of the dispatch is what the coordinator needs.
pub(super) fn combine_messages(older: &str, newer: &str) -> String {
    const TRIMMED: &str = "[dispatch] (earlier updates were shortened to fit)\n\n";
    let joined = format!("{older}\n\n{newer}");
    if joined.len() <= FLEET_MESSAGE_MAX_BYTES {
        return joined;
    }
    let budget = FLEET_MESSAGE_MAX_BYTES.saturating_sub(TRIMMED.len() + newer.len() + 2);
    let mut start = older.len().saturating_sub(budget);
    while !older.is_char_boundary(start) {
        start += 1;
    }
    let kept = &older[start..];
    if kept.is_empty() {
        format!("{TRIMMED}{newer}")
    } else {
        format!("{TRIMMED}{kept}\n\n{newer}")
    }
}
