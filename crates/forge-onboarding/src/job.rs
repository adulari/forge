//! Action jobs: long-running work (a verification, an install, an import) that reports progress as
//! serializable events. A front end polls [`Jobs::poll`] with a cursor, or a daemon serves the
//! same events over HTTP; the engine does not care which.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use crate::state::VerifyStatus;

pub type JobId = u64;

/// How many events a job keeps. Older output lines are dropped first; lifecycle events stay.
const EVENT_CAP: usize = 400;

/// What a job needs from the person while it runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PromptKind {
    /// A page the person should open (sign-in, key page).
    OpenUrl { url: String },
    /// A code to type at a page.
    DeviceCode { url: String, code: String },
    /// The step needs a real terminal; the host suspends its screen and hands it over.
    Terminal,
}

/// What a finished job tells the front end beyond its text, so it can update its state without
/// parsing prose.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ActionResult {
    /// A model source was checked live.
    Verified {
        source: String,
        status: VerifyStatus,
        detail: Option<String>,
    },
    /// Something was saved; the machine facts should be read again.
    Changed,
}

/// The successful end of a job.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Success {
    pub summary: String,
    pub result: Option<ActionResult>,
}

impl Success {
    pub fn text(summary: impl Into<String>) -> Self {
        Self {
            summary: summary.into(),
            result: None,
        }
    }

    pub fn with(summary: impl Into<String>, result: ActionResult) -> Self {
        Self {
            summary: summary.into(),
            result: Some(result),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum JobEvent {
    Started {
        job: JobId,
        action: String,
    },
    /// One line of output, already scrubbed of the action's secrets.
    Output {
        job: JobId,
        line: String,
    },
    Progress {
        job: JobId,
        label: String,
        /// 0.0 to 1.0 when known.
        fraction: Option<f32>,
    },
    Prompt {
        job: JobId,
        prompt: PromptKind,
        message: String,
    },
    Done {
        job: JobId,
        summary: String,
        result: Option<ActionResult>,
    },
    Failed {
        job: JobId,
        error: String,
    },
}

impl JobEvent {
    pub fn job(&self) -> JobId {
        match self {
            JobEvent::Started { job, .. }
            | JobEvent::Output { job, .. }
            | JobEvent::Progress { job, .. }
            | JobEvent::Prompt { job, .. }
            | JobEvent::Done { job, .. }
            | JobEvent::Failed { job, .. } => *job,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Running,
    Succeeded,
    Failed,
}

/// The reply to a poll: events after `cursor`, and the cursor to pass next time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JobPoll {
    pub job: JobId,
    pub action: String,
    pub state: JobState,
    pub events: Vec<JobEvent>,
    pub next: usize,
}

struct Record {
    action: String,
    state: JobState,
    events: Vec<JobEvent>,
    /// Events dropped from the front, so cursors stay valid as absolute positions.
    dropped: usize,
}

#[derive(Default)]
struct Inner {
    next_id: JobId,
    jobs: HashMap<JobId, Record>,
}

/// The job table. Cheap to clone; every clone sees the same jobs.
#[derive(Clone, Default)]
pub struct Jobs {
    inner: Arc<Mutex<Inner>>,
}

/// What a running job writes to. Cloneable so a job can hand it to helper threads.
#[derive(Clone)]
pub struct JobHandle {
    id: JobId,
    jobs: Jobs,
    secrets: Arc<Vec<String>>,
}

impl JobHandle {
    pub fn id(&self) -> JobId {
        self.id
    }

    /// One line of output. Every secret the action carries is replaced before it is stored.
    pub fn output(&self, line: impl AsRef<str>) {
        let line = scrub(line.as_ref(), &self.secrets);
        self.jobs
            .push(self.id, JobEvent::Output { job: self.id, line });
    }

    pub fn progress(&self, label: impl Into<String>, fraction: Option<f32>) {
        self.jobs.push(
            self.id,
            JobEvent::Progress {
                job: self.id,
                label: scrub(&label.into(), &self.secrets),
                fraction,
            },
        );
    }

    pub fn prompt(&self, prompt: PromptKind, message: impl Into<String>) {
        self.jobs.push(
            self.id,
            JobEvent::Prompt {
                job: self.id,
                prompt,
                message: scrub(&message.into(), &self.secrets),
            },
        );
    }
}

fn scrub(line: &str, secrets: &[String]) -> String {
    let mut out = line.to_string();
    for secret in secrets.iter().filter(|s| s.len() >= 4) {
        out = out.replace(secret.as_str(), "***");
    }
    out
}

impl Jobs {
    pub fn new() -> Self {
        Self::default()
    }

    /// Start `work` on its own thread. Its `Ok` text becomes the `Done` summary and its `Err`
    /// text the `Failed` error; both are scrubbed of `secrets` like every other line.
    pub fn start<F>(&self, action: impl Into<String>, secrets: Vec<String>, work: F) -> JobId
    where
        F: FnOnce(&JobHandle) -> Result<Success, String> + Send + 'static,
    {
        let action = action.into();
        let id = {
            let mut inner = self.lock();
            inner.next_id += 1;
            let id = inner.next_id;
            inner.jobs.insert(
                id,
                Record {
                    action: action.clone(),
                    state: JobState::Running,
                    events: Vec::new(),
                    dropped: 0,
                },
            );
            id
        };
        self.push(id, JobEvent::Started { job: id, action });
        let handle = JobHandle {
            id,
            jobs: self.clone(),
            secrets: Arc::new(secrets),
        };
        std::thread::spawn(move || {
            let result = work(&handle);
            let event = match result {
                Ok(success) => JobEvent::Done {
                    job: handle.id,
                    summary: scrub(&success.summary, &handle.secrets),
                    result: success.result,
                },
                Err(error) => JobEvent::Failed {
                    job: handle.id,
                    error: scrub(&error, &handle.secrets),
                },
            };
            handle.jobs.push(handle.id, event);
        });
        id
    }

    /// A job that has already finished, for work that never needed a thread (a refusal).
    pub fn finished(&self, action: impl Into<String>, result: Result<String, String>) -> JobId {
        let action = action.into();
        let id = {
            let mut inner = self.lock();
            inner.next_id += 1;
            let id = inner.next_id;
            inner.jobs.insert(
                id,
                Record {
                    action: action.clone(),
                    state: JobState::Running,
                    events: Vec::new(),
                    dropped: 0,
                },
            );
            id
        };
        self.push(id, JobEvent::Started { job: id, action });
        self.push(
            id,
            match result {
                Ok(summary) => JobEvent::Done {
                    job: id,
                    summary,
                    result: None,
                },
                Err(error) => JobEvent::Failed { job: id, error },
            },
        );
        id
    }

    /// Events after `cursor` (0 for all). `None` for an unknown job.
    pub fn poll(&self, id: JobId, cursor: usize) -> Option<JobPoll> {
        let inner = self.lock();
        let record = inner.jobs.get(&id)?;
        let total = record.dropped + record.events.len();
        let from = cursor
            .saturating_sub(record.dropped)
            .min(record.events.len());
        Some(JobPoll {
            job: id,
            action: record.action.clone(),
            state: record.state,
            events: record.events[from..].to_vec(),
            next: total,
        })
    }

    pub fn state(&self, id: JobId) -> Option<JobState> {
        self.lock().jobs.get(&id).map(|r| r.state)
    }

    fn push(&self, id: JobId, event: JobEvent) {
        let mut inner = self.lock();
        let Some(record) = inner.jobs.get_mut(&id) else {
            return;
        };
        match &event {
            JobEvent::Done { .. } => record.state = JobState::Succeeded,
            JobEvent::Failed { .. } => record.state = JobState::Failed,
            _ => {}
        }
        record.events.push(event);
        if record.events.len() > EVENT_CAP {
            if let Some(i) = record
                .events
                .iter()
                .position(|e| matches!(e, JobEvent::Output { .. } | JobEvent::Progress { .. }))
            {
                record.events.remove(i);
                record.dropped += 1;
            }
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn wait_done(jobs: &Jobs, id: JobId) {
        let start = Instant::now();
        while jobs.state(id) == Some(JobState::Running) {
            assert!(
                start.elapsed() < Duration::from_secs(5),
                "job never finished"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn a_job_reports_start_output_progress_and_done_in_order() {
        let jobs = Jobs::new();
        let id = jobs.start("Install service", vec![], |h| {
            h.output("writing unit");
            h.progress("starting", Some(0.5));
            Ok(Success::text("installed"))
        });
        wait_done(&jobs, id);
        let poll = jobs.poll(id, 0).unwrap();
        assert_eq!(poll.state, JobState::Succeeded);
        assert!(matches!(poll.events[0], JobEvent::Started { .. }));
        assert!(matches!(&poll.events[1], JobEvent::Output { line, .. } if line == "writing unit"));
        assert!(matches!(poll.events[2], JobEvent::Progress { fraction: Some(f), .. } if f == 0.5));
        assert!(
            matches!(&poll.events[3], JobEvent::Done { summary, .. } if summary == "installed")
        );
    }

    #[test]
    fn a_failed_job_ends_in_a_failed_event() {
        let jobs = Jobs::new();
        let id = jobs.start("x", vec![], |_| Err("boom".into()));
        wait_done(&jobs, id);
        let poll = jobs.poll(id, 0).unwrap();
        assert_eq!(poll.state, JobState::Failed);
        assert!(
            matches!(poll.events.last(), Some(JobEvent::Failed { error, .. }) if error == "boom")
        );
    }

    #[test]
    fn polling_with_a_cursor_returns_only_new_events() {
        let jobs = Jobs::new();
        let id = jobs.start("x", vec![], |h| {
            h.output("one");
            Ok(Success::default())
        });
        wait_done(&jobs, id);
        let first = jobs.poll(id, 0).unwrap();
        let again = jobs.poll(id, first.next).unwrap();
        assert!(again.events.is_empty());
        assert_eq!(again.next, first.next);
    }

    #[test]
    fn the_actions_secrets_never_reach_the_event_log() {
        let jobs = Jobs::new();
        let id = jobs.start("store key", vec!["sk-live-1234567890".into()], |h| {
            h.output("verifying sk-live-1234567890 against the API");
            h.progress("checking sk-live-1234567890", None);
            h.prompt(PromptKind::Terminal, "paste sk-live-1234567890");
            Err("rejected sk-live-1234567890".into())
        });
        wait_done(&jobs, id);
        let json = serde_json::to_string(&jobs.poll(id, 0).unwrap()).unwrap();
        assert!(!json.contains("sk-live-1234567890"), "{json}");
        assert!(json.contains("***"));
    }

    #[test]
    fn events_serialize_with_a_type_tag_for_the_daemon() {
        let ev = JobEvent::Prompt {
            job: 3,
            prompt: PromptKind::DeviceCode {
                url: "https://x.test".into(),
                code: "ABCD-EFGH".into(),
            },
            message: "Enter the code".into(),
        };
        let json = serde_json::to_value(&ev).unwrap();
        assert_eq!(json["type"], "prompt");
        assert_eq!(json["prompt"]["kind"], "device_code");
        assert_eq!(serde_json::from_value::<JobEvent>(json).unwrap(), ev);
    }

    #[test]
    fn the_log_is_capped_but_keeps_the_final_event_and_valid_cursors() {
        let jobs = Jobs::new();
        let id = jobs.start("noisy", vec![], |h| {
            for i in 0..(EVENT_CAP * 2) {
                h.output(format!("line {i}"));
            }
            Ok(Success::text("done"))
        });
        wait_done(&jobs, id);
        let poll = jobs.poll(id, 0).unwrap();
        assert!(poll.events.len() <= EVENT_CAP + 1);
        assert!(matches!(poll.events.last(), Some(JobEvent::Done { .. })));
        assert_eq!(
            poll.next,
            EVENT_CAP * 2 + 2,
            "cursor counts every event ever pushed"
        );
        assert!(jobs.poll(id, poll.next).unwrap().events.is_empty());
    }

    #[test]
    fn an_unknown_job_polls_as_none() {
        assert!(Jobs::new().poll(99, 0).is_none());
    }

    #[test]
    fn a_refusal_is_a_finished_job() {
        let jobs = Jobs::new();
        let id = jobs.finished("needs a terminal", Err("run it yourself".into()));
        assert_eq!(jobs.state(id), Some(JobState::Failed));
    }
}
