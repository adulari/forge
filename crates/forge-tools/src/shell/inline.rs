//! Pids whose exit the starting call is still waiting on itself, so no wake notice is queued for
//! them (see `autobackground`).

static INLINE: std::sync::Mutex<Vec<u32>> = std::sync::Mutex::new(Vec::new());

pub(super) fn mark(pid: u32) {
    INLINE.lock().unwrap().push(pid);
}

pub(super) fn release(pid: u32) {
    INLINE.lock().unwrap().retain(|p| *p != pid);
}

/// Remove `pid` and report whether it was marked: the exit watcher's one-lock check-and-release.
pub(super) fn take(pid: u32) -> bool {
    let mut inline = INLINE.lock().unwrap();
    let before = inline.len();
    inline.retain(|p| *p != pid);
    inline.len() != before
}

/// Release `pid` unless the job has already exited. An exited job stays marked until its watcher
/// has looked, so the watcher cannot queue a notice for a result the caller is returning itself.
/// Returns whether the pid was released.
pub(super) fn release_unless(pid: u32, exited: impl FnOnce() -> bool) -> bool {
    let mut inline = INLINE.lock().unwrap();
    if exited() {
        return false;
    }
    inline.retain(|p| *p != pid);
    true
}

#[cfg(test)]
pub(super) fn pids() -> Vec<u32> {
    INLINE.lock().unwrap().clone()
}
