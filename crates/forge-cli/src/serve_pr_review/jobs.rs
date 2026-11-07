//! Reviews run as background tasks, one per (session, head SHA), so a slow reviewer never holds
//! up the PR Watch tick that polls every other watch. The tick starts a job and reads its result
//! on a later poll; a daemon restart loses the running task, and the watch's stored `running`
//! state tells the next tick to start it again. A job follows its watch when a relay moves the
//! watch to a fresh worker (`rekey`), and is aborted once its head is superseded.

use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};

use tokio::sync::Semaphore;
use tokio::task::AbortHandle;

use super::ReviewState;

/// Reviews that may call a model at once; the rest queue and show as running.
pub(crate) const MAX_CONCURRENT: usize = 2;

enum Slot {
    /// The task finds its slot by `id` when it finishes, wherever a rekey moved it.
    Running {
        id: u64,
        abort: AbortHandle,
    },
    Done(ReviewState),
}

impl Slot {
    fn cancel(&self) {
        if let Slot::Running { abort, .. } = self {
            abort.abort();
        }
    }
}

pub(crate) enum Poll {
    /// No job for this head in this daemon (never started, or lost in a restart).
    Missing,
    Running,
    Done(Box<ReviewState>),
}

pub(crate) struct Jobs {
    slots: Mutex<HashMap<(String, String), Slot>>,
    permits: Arc<Semaphore>,
    next_id: std::sync::atomic::AtomicU64,
}

impl Jobs {
    fn new() -> Self {
        Self {
            slots: Mutex::new(HashMap::new()),
            permits: Arc::new(Semaphore::new(MAX_CONCURRENT)),
            next_id: std::sync::atomic::AtomicU64::new(0),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<(String, String), Slot>> {
        self.slots
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The job's state; a finished result is handed over once.
    pub(crate) fn poll(&self, session: &str, head: &str) -> Poll {
        let mut slots = self.lock();
        let key = (session.to_string(), head.to_string());
        match slots.remove(&key) {
            None => Poll::Missing,
            Some(running @ Slot::Running { .. }) => {
                slots.insert(key, running);
                Poll::Running
            }
            Some(Slot::Done(state)) => Poll::Done(Box::new(state)),
        }
    }

    /// Drop what a superseded head left behind: a finished result nobody will read, or a review
    /// still spending on it, which is aborted so its permit goes to the head that matters.
    pub(crate) fn forget_other_heads(&self, session: &str, head: &str) {
        self.lock().retain(|(s, h), slot| {
            let keep = s != session || h == head;
            if !keep {
                slot.cancel();
            }
            keep
        });
    }

    /// The watch moved from `from` to `to` (a goal relay): its jobs go with it, so the next tick
    /// reads the running review instead of paying for a second one. When `to` already has a job
    /// for a head, `from`'s is aborted.
    pub(crate) fn rekey(&self, from: &str, to: &str) {
        let mut slots = self.lock();
        let moving: Vec<(String, String)> =
            slots.keys().filter(|(s, _)| s == from).cloned().collect();
        for key in moving {
            let slot = slots.remove(&key).expect("listed above");
            let target = (to.to_string(), key.1);
            match slots.entry(target) {
                Entry::Vacant(free) => {
                    free.insert(slot);
                }
                Entry::Occupied(_) => slot.cancel(),
            }
        }
    }

    /// The watch is gone: abort its reviews and drop their results.
    pub(crate) fn forget_session(&self, session: &str) {
        self.lock().retain(|(s, _), slot| {
            if s == session {
                slot.cancel();
            }
            s != session
        });
    }

    /// Start `work` in the background. `done` runs with the job's current session when it
    /// finishes, after the result is readable; a job forgotten meanwhile drops its result.
    pub(crate) fn start(
        self: &Arc<Self>,
        session: &str,
        head: &str,
        work: impl Future<Output = ReviewState> + Send + 'static,
        done: impl FnOnce(&str) + Send + 'static,
    ) {
        let key = (session.to_string(), head.to_string());
        let id = self
            .next_id
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        // Held across the spawn so the task cannot finish before its slot exists.
        let mut slots = self.lock();
        let jobs = self.clone();
        let task = tokio::spawn(async move {
            let _permit = jobs.permits.clone().acquire_owned().await;
            let state = work.await;
            let mut slots = jobs.lock();
            let Some(key) = slots
                .iter()
                .find(|(_, slot)| matches!(slot, Slot::Running { id: own, .. } if *own == id))
                .map(|(key, _)| key.clone())
            else {
                return;
            };
            slots.insert(key.clone(), Slot::Done(state));
            drop(slots);
            done(&key.0);
        });
        let abort = task.abort_handle();
        if let Some(old) = slots.insert(key, Slot::Running { id, abort }) {
            old.cancel();
        }
    }

    #[cfg(test)]
    pub(crate) fn running(&self) -> usize {
        self.lock()
            .values()
            .filter(|s| matches!(s, Slot::Running { .. }))
            .count()
    }
}

#[cfg(not(test))]
pub(crate) fn jobs() -> Arc<Jobs> {
    static JOBS: std::sync::OnceLock<Arc<Jobs>> = std::sync::OnceLock::new();
    JOBS.get_or_init(|| Arc::new(Jobs::new())).clone()
}

// Tests run one per thread on a current-thread runtime, so each gets its own table.
#[cfg(test)]
pub(crate) fn jobs() -> Arc<Jobs> {
    thread_local! {
        static JOBS: Arc<Jobs> = Arc::new(Jobs::new());
    }
    JOBS.with(Arc::clone)
}
