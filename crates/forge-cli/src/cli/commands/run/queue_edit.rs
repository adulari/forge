//! Editing the visible follow-up queue from a remote client: move one prompt, or pull it to the
//! head so it runs next.
//!
//! Both are addressed by `index` + `text`, the row the client rendered, so an edit made against a
//! queue that has since shifted (a turn took the head) is refused instead of moving a different
//! prompt. The steer inbox mirrors the queue's prompts, so it is reordered to match; otherwise the
//! running turn would still absorb them in the old order.

use forge_core::steer::SteerInbox;

pub(crate) const STALE_QUEUE_EDIT_NOTE: &str =
    "⚠ stale queue edit ignored — the queue changed; review the current list";

/// Move the prompt the client saw at `index` to position `to` (clamped to the end). False when the
/// queue no longer has `text` at `index`.
pub(crate) fn move_queued(
    queue: &mut Vec<String>,
    steer: &SteerInbox,
    index: u64,
    text: &str,
    to: u64,
) -> bool {
    let from = index as usize;
    if queue.get(from).map(String::as_str) != Some(text) {
        return false;
    }
    let item = queue.remove(from);
    queue.insert((to as usize).min(queue.len()), item);
    steer.reorder_pending(queue);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn queue(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_move_reorders_the_queue_and_the_steer_inbox_together() {
        let steer = SteerInbox::default();
        let mut q = queue(&["one", "/compact", "two", "three"]);
        for t in ["one", "two", "three"] {
            steer.push(t);
        }
        assert!(move_queued(&mut q, &steer, 3, "three", 0));
        assert_eq!(q, ["three", "one", "/compact", "two"]);
        assert_eq!(steer.drain(), ["three", "one", "two"]);
    }

    #[test]
    fn a_move_past_the_end_lands_last() {
        let steer = SteerInbox::default();
        let mut q = queue(&["a", "b", "c"]);
        assert!(move_queued(&mut q, &steer, 0, "a", 99));
        assert_eq!(q, ["b", "c", "a"]);
    }

    #[test]
    fn a_stale_edit_moves_nothing() {
        let steer = SteerInbox::default();
        let mut q = queue(&["a", "b"]);
        assert!(
            !move_queued(&mut q, &steer, 1, "a", 0),
            "text is not at that index"
        );
        assert!(
            !move_queued(&mut q, &steer, 5, "a", 0),
            "index is past the end"
        );
        assert_eq!(q, ["a", "b"]);
    }
}
