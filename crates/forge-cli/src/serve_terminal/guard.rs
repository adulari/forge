//! Counts attached clients on a terminal for as long as the guard lives.

use super::TerminalHandle;
use std::sync::atomic::Ordering;
use std::sync::Arc;

pub(super) struct TerminalClientGuard(Arc<TerminalHandle>);

impl TerminalClientGuard {
    pub(super) fn new(handle: Arc<TerminalHandle>) -> Self {
        handle.clients.fetch_add(1, Ordering::AcqRel);
        handle.touch();
        Self(handle)
    }
}

impl Drop for TerminalClientGuard {
    fn drop(&mut self) {
        self.0.clients.fetch_sub(1, Ordering::AcqRel);
        self.0.touch();
    }
}
