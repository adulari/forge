//! A host-wide ceiling on subagent helpers (`[host] max_helpers`).
//!
//! Each batch of subagents already has its own concurrency limit, but ten sessions each running
//! their own batch still add up. The daemon sets one process-wide limit here; a helper takes a
//! slot before it starts and gives it back when it ends. A helper that finds no free slot waits
//! for one, so a busy host slows down instead of failing the turn that spawned the helper.

use std::sync::{Arc, OnceLock};

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

static HELPER_SLOTS: OnceLock<Arc<Semaphore>> = OnceLock::new();

/// Set the ceiling once, at daemon start. A later call is ignored: permits already handed out
/// belong to the first semaphore.
pub fn set_helper_limit(max: usize) {
    let _ = HELPER_SLOTS.set(Arc::new(Semaphore::new(max.max(1))));
}

/// A held helper slot, or `None` when no ceiling is set.
pub async fn acquire_helper_slot() -> Option<OwnedSemaphorePermit> {
    acquire(HELPER_SLOTS.get()).await
}

async fn acquire(slots: Option<&Arc<Semaphore>>) -> Option<OwnedSemaphorePermit> {
    match slots {
        Some(slots) => slots.clone().acquire_owned().await.ok(),
        None => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn unset_means_no_ceiling_and_a_set_ceiling_makes_the_next_helper_wait() {
        assert!(acquire(None).await.is_none());
        let slots = Arc::new(Semaphore::new(1));
        let first = acquire(Some(&slots)).await.expect("one slot");
        let second =
            tokio::time::timeout(std::time::Duration::from_millis(60), acquire(Some(&slots))).await;
        assert!(
            second.is_err(),
            "the second helper waits while the first holds the slot"
        );
        drop(first);
        assert!(acquire(Some(&slots)).await.is_some(), "freed on drop");
    }
}
