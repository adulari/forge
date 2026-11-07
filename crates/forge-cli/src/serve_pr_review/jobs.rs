//! Reviews run as background tasks, one per (session, head SHA), so a slow reviewer never holds
//! up the PR Watch tick that polls every other watch. The tick starts a job and reads its result
//! on a later poll; a daemon restart loses the running task, and the watch's stored `running`
//! state tells the next tick to start it again.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};

use tokio::sync::Semaphore;

use super::ReviewState;

/// Reviews that may call a model at once; the rest queue and show as running.
pub(crate) const MAX_CONCURRENT: usize = 2;

enum Slot {
    Running,
    Done(ReviewState),
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
}

impl Jobs {
    fn new() -> Self {
        Self {
            slots: Mutex::new(HashMap::new()),
            permits: Arc::new(Semaphore::new(MAX_CONCURRENT)),
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
            Some(Slot::Running) => {
                slots.insert(key, Slot::Running);
                Poll::Running
            }
            Some(Slot::Done(state)) => Poll::Done(Box::new(state)),
        }
    }

    /// Drop what a superseded head left behind (a finished result nobody will read).
    pub(crate) fn forget_other_heads(&self, session: &str, head: &str) {
        self.lock()
            .retain(|(s, h), slot| s != session || h == head || matches!(slot, Slot::Running));
    }

    /// Start `work` in the background. `done` runs when it finishes, after the result is readable.
    pub(crate) fn start(
        self: &Arc<Self>,
        session: &str,
        head: &str,
        work: impl Future<Output = ReviewState> + Send + 'static,
        done: impl FnOnce() + Send + 'static,
    ) {
        let key = (session.to_string(), head.to_string());
        self.lock().insert(key.clone(), Slot::Running);
        let jobs = self.clone();
        tokio::spawn(async move {
            let _permit = jobs.permits.clone().acquire_owned().await;
            let state = work.await;
            jobs.lock().insert(key, Slot::Done(state));
            done();
        });
    }

    #[cfg(test)]
    pub(crate) fn running(&self) -> usize {
        self.lock()
            .values()
            .filter(|s| matches!(s, Slot::Running))
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
