//! The host-side inbox of notifications the model sent. Hosted push channels carry only a generic
//! alert, so the app fetches the real text from here (`GET /api/notices`) once it is open and
//! authenticated. In memory and bounded: a restart drops it, which is fine for a transient alert.

use std::collections::VecDeque;
use std::sync::Mutex;

use forge_core::push_notify::PushRequest;
use serde::Serialize;

const CAPACITY: usize = 50;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct Notice {
    pub(crate) seq: u64,
    pub(crate) at: i64,
    pub(crate) title: String,
    pub(crate) message: String,
    pub(crate) priority: &'static str,
    pub(crate) session_id: String,
}

#[derive(Default)]
pub(super) struct Log {
    next: u64,
    items: VecDeque<Notice>,
}

static LOG: Mutex<Log> = Mutex::new(Log {
    next: 0,
    items: VecDeque::new(),
});

impl Log {
    pub(super) fn push(&mut self, request: &PushRequest) -> u64 {
        self.next += 1;
        self.items.push_back(Notice {
            seq: self.next,
            at: chrono::Utc::now().timestamp(),
            title: request.title.clone().unwrap_or_else(|| "Forge".into()),
            message: request.message.clone(),
            priority: request.priority.as_str(),
            session_id: request.session_id.clone(),
        });
        while self.items.len() > CAPACITY {
            self.items.pop_front();
        }
        self.next
    }

    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.items.len()
    }
}

pub(crate) fn record(request: &PushRequest) -> u64 {
    LOG.lock().unwrap_or_else(|e| e.into_inner()).push(request)
}

/// Notices newer than `after`, oldest first, and the latest sequence number.
pub(crate) fn since(after: u64) -> (Vec<Notice>, u64) {
    let log = LOG.lock().unwrap_or_else(|e| e.into_inner());
    (
        log.items
            .iter()
            .filter(|n| n.seq > after)
            .cloned()
            .collect(),
        log.next,
    )
}
