//! The onboarding screen's pure state: pages, cursors, tracked jobs and key handling. It does no
//! I/O. Anything that touches the machine is returned as an [`Effect`] for the host (the
//! `forge` binary) to carry out, and the host feeds results back through [`Screen::apply_poll`],
//! [`Screen::terminal_finished`] and [`Screen::machine_updated`].

use std::collections::{HashSet, VecDeque};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use forge_onboarding::{
    Action, JobEvent, JobId, JobPoll, JobState, Machine, PlanMode, Session, StepId, StepStatus,
};

/// Why a job was started, so its result lands in the right place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Purpose {
    Verify(String),
    StoreKey(String),
    /// The one-action Claude connection (`claude setup-token` in a pseudo-terminal).
    ConnectClaude,
    StoreClaudeToken,
    /// Item `n` of the "set up everything recommended" run.
    Batch(usize),
    OauthSignIn(String),
    LocalInstall(String),
    LocalPull(String),
    LocalSave,
    Setting,
    Searxng,
    StoreSearchKey,
    StoreBenchKey,
    Import,
    Service,
    Anywhere,
    Desktop,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    /// Start a background job.
    Run { action: Action, purpose: Purpose },
    /// Hand the terminal to a child process (the host suspends the screen first).
    Terminal { action: Action, purpose: Purpose },
    /// Read the machine again and call [`Screen::machine_updated`].
    Redetect,
    /// Open a page in the browser.
    OpenUrl(String),
    /// Type text into a running job that asked for it (a code from the browser).
    Input { job: JobId, text: String },
    /// Stop a running job.
    Cancel(JobId),
    /// Write the onboarding state to disk.
    Persist,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exit {
    /// The run reached its end.
    Finished,
    /// The person left early ("skip for now").
    Left,
}

/// What a masked entry field is collecting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretTarget {
    ApiKey {
        provider: String,
        replace: bool,
    },
    ClaudeToken,
    /// A code a running sign-in job is waiting for.
    JobCode(JobId),
    SearchKey,
    BenchmarkKey,
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub target: SecretTarget,
    pub label: String,
    pub hint: Option<String>,
    pub url: Option<String>,
    pub buf: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoticeKind {
    Info,
    Ok,
    Warn,
    Err,
}

#[derive(Debug, Clone)]
pub struct Notice {
    pub kind: NoticeKind,
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct Tracked {
    pub id: JobId,
    pub purpose: Purpose,
    pub cursor: usize,
    pub label: String,
    pub state: JobState,
}

/// Page-local state that survives moving between pages.
#[derive(Debug, Clone, Default)]
pub struct Ui {
    pub behavior_group: usize,
    pub temper: usize,
    pub credit: usize,
    pub exposure: usize,
    pub notifications: bool,
    pub theme: usize,
    pub prefs_row: usize,
    pub reporting: usize,
    pub reporting_details: bool,
    pub local_autostart: bool,
    pub trust_folder: bool,
    pub import_checked: Vec<bool>,
}

/// A browser sign-in in progress: what the person has to open, and whether the QR is showing.
#[derive(Debug, Clone)]
pub struct SignIn {
    pub job: JobId,
    pub url: Option<String>,
    pub qr: bool,
}

pub struct Screen {
    pub session: Session,
    pub page: usize,
    pub cursor: usize,
    pub entry: Option<Entry>,
    pub jobs: Vec<Tracked>,
    /// Recent output lines of the newest job, oldest first.
    pub log: VecDeque<String>,
    pub notice: Option<Notice>,
    pub tick: usize,
    pub exit: Option<Exit>,
    pub ui: Ui,
    /// The host starts a session when the run ends, so the last page says so.
    pub launches_session: bool,
    pub sign_in: Option<SignIn>,
    /// The small "Connect Claude Code" card shown at session start instead of the full flow.
    pub reconnect_card: bool,
    /// The person asked not to be asked about the card again.
    pub card_dismissed: bool,
    pub batch: Option<super::batch::Batch>,
    pub(crate) requested: HashSet<String>,
    pub(crate) focus_id: Option<String>,
    pub(crate) import_queue: VecDeque<Action>,
}

pub(crate) const LOG_CAP: usize = 200;

impl Screen {
    pub fn new(session: Session) -> Self {
        let page = session.start_index();
        let mut screen = Self {
            session,
            page,
            cursor: 0,
            entry: None,
            jobs: Vec::new(),
            log: VecDeque::new(),
            notice: None,
            tick: 0,
            exit: None,
            ui: Ui::default(),
            launches_session: false,
            sign_in: None,
            reconnect_card: false,
            card_dismissed: false,
            batch: None,
            requested: HashSet::new(),
            focus_id: None,
            import_queue: VecDeque::new(),
        };
        screen.init_ui();
        screen
    }

    /// Effects to run when the screen opens (the first page's own setup).
    pub fn opening_effects(&mut self) -> Vec<Effect> {
        self.enter_page()
    }

    pub fn step(&self) -> StepId {
        self.session.plan[self.page.min(self.session.plan.len() - 1)].id
    }

    pub fn is_last(&self) -> bool {
        self.page + 1 >= self.session.plan.len()
    }

    pub fn tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
    }

    pub fn busy(&self) -> bool {
        self.jobs.iter().any(|j| j.state == JobState::Running)
    }

    pub fn running(&self, purpose: &Purpose) -> bool {
        self.jobs
            .iter()
            .any(|j| j.state == JobState::Running && &j.purpose == purpose)
    }

    /// `(job, cursor)` of every job the host should poll.
    pub fn pollable(&self) -> Vec<(JobId, usize)> {
        self.jobs
            .iter()
            .filter(|j| j.state == JobState::Running)
            .map(|j| (j.id, j.cursor))
            .collect()
    }

    pub fn job_started(&mut self, id: JobId, purpose: Purpose, label: String) {
        self.log.clear();
        self.jobs.push(Tracked {
            id,
            purpose,
            cursor: 0,
            label,
            state: JobState::Running,
        });
    }

    pub fn notify(&mut self, kind: NoticeKind, text: impl Into<String>) {
        self.notice = Some(Notice {
            kind,
            text: text.into(),
        });
    }

    pub fn machine_updated(&mut self, machine: Machine) -> Vec<Effect> {
        self.session.machine = machine;
        if self.step() == StepId::Models {
            if let Some(id) = self.focus_id.clone() {
                if let Some(index) = self.cards().iter().position(|c| c.id == id) {
                    self.cursor = index;
                }
            }
        }
        self.session
            .state
            .refresh_required(&self.session.machine, self.session.surface);
        self.session.plan = forge_onboarding::plan(
            &self.session.state,
            &self.session.machine,
            self.session.surface,
            self.session.mode,
        );
        if self.page >= self.session.plan.len() {
            self.page = self.session.plan.len() - 1;
        }
        let mut effects = vec![Effect::Persist];
        effects.extend(self.auto_verify());
        effects
    }

    /// Fold new job events in; returns follow-up effects once a job ends.
    pub fn apply_poll(&mut self, poll: &JobPoll) -> Vec<Effect> {
        let Some(index) = self.jobs.iter().position(|j| j.id == poll.job) else {
            return Vec::new();
        };
        self.jobs[index].cursor = poll.next;
        let mut result = None;
        let mut failure = None;
        for event in &poll.events {
            match event {
                JobEvent::Output { line, .. } => self.push_log(line.clone()),
                JobEvent::Progress { label, .. } => self.push_log(label.clone()),
                JobEvent::Prompt {
                    prompt, message, ..
                } => {
                    self.push_log(message.clone());
                    self.prompt_arrived(poll.job, prompt);
                }
                JobEvent::Done { result: r, .. } => result = Some(r.clone()),
                JobEvent::Failed { error, .. } => failure = Some(error.clone()),
                JobEvent::Started { .. } => {}
            }
        }
        if poll.state == JobState::Running {
            return Vec::new();
        }
        self.jobs[index].state = poll.state;
        if self.sign_in.as_ref().is_some_and(|s| s.job == poll.job) {
            self.sign_in = None;
            if matches!(
                self.entry.as_ref().map(|e| &e.target),
                Some(SecretTarget::JobCode(_))
            ) {
                self.entry = None;
            }
        }
        let purpose = self.jobs[index].purpose.clone();
        match (result, failure) {
            (Some(result), _) => self.job_succeeded(&purpose, result),
            (None, Some(error)) => self.job_failed(&purpose, &error),
            (None, None) => Vec::new(),
        }
    }

    /// A running job needs the person: show the sign-in page, or ask for the code it shows.
    fn prompt_arrived(&mut self, job: JobId, prompt: &forge_onboarding::PromptKind) {
        use forge_onboarding::PromptKind;
        match prompt {
            PromptKind::OpenUrl { url } | PromptKind::DeviceCode { url, .. } => {
                let qr = self.sign_in.as_ref().is_some_and(|s| s.qr);
                self.sign_in = Some(SignIn {
                    job,
                    url: Some(url.clone()),
                    qr,
                });
            }
            PromptKind::Code { label } => {
                if self.sign_in.is_none() {
                    self.sign_in = Some(SignIn {
                        job,
                        url: None,
                        qr: false,
                    });
                }
                self.entry = Some(Entry {
                    target: SecretTarget::JobCode(job),
                    label: label.clone(),
                    hint: Some("Paste it here and press Enter. It is not saved or shown.".into()),
                    url: None,
                    buf: String::new(),
                });
            }
            PromptKind::Terminal => {}
        }
    }

    fn push_log(&mut self, line: String) {
        if self.log.len() >= LOG_CAP {
            self.log.pop_front();
        }
        self.log.push_back(line);
    }

    /// Mark the current step and move to the next page.
    pub(crate) fn advance(&mut self) -> Vec<Effect> {
        let step = self.step();
        if !matches!(step, StepId::Done) {
            self.session.state.mark_done(step);
        }
        self.go_to(self.page + 1)
    }

    pub(crate) fn skip_step(&mut self) -> Vec<Effect> {
        let step = self.step();
        self.session.state.mark_skipped(step);
        self.go_to(self.page + 1)
    }

    pub(crate) fn go_to(&mut self, page: usize) -> Vec<Effect> {
        let mut effects = vec![Effect::Persist];
        if page >= self.session.plan.len() {
            effects.extend(self.finish());
            return effects;
        }
        self.page = page;
        effects.extend(self.enter_page());
        effects
    }

    pub(crate) fn finish(&mut self) -> Vec<Effect> {
        self.session.complete();
        self.exit = Some(Exit::Finished);
        vec![Effect::Persist]
    }

    fn leave(&mut self) -> Vec<Effect> {
        self.session.postpone();
        self.exit = Some(Exit::Left);
        vec![Effect::Persist]
    }

    /// Handle one key press.
    pub fn key(&mut self, key: KeyEvent) -> Vec<Effect> {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return self.leave();
        }
        if self.entry.is_some() {
            return self.entry_key(key);
        }
        self.notice = None;
        if let Some(sign_in) = self.sign_in.clone() {
            match key.code {
                KeyCode::Char('o') => {
                    if let Some(url) = sign_in.url {
                        return vec![Effect::OpenUrl(url)];
                    }
                    return Vec::new();
                }
                KeyCode::Char('v') => {
                    if let Some(s) = self.sign_in.as_mut() {
                        s.qr = !s.qr;
                    }
                    return Vec::new();
                }
                KeyCode::Esc | KeyCode::Char('c') => return vec![Effect::Cancel(sign_in.job)],
                KeyCode::Char('q') => return self.leave(),
                _ => return Vec::new(),
            }
        }
        if self.reconnect_card {
            return self.card_key(key);
        }
        match key.code {
            KeyCode::Char('q') => return self.leave(),
            KeyCode::Esc | KeyCode::Char('b') => {
                if self.page > 0 {
                    self.log.clear();
                    return self.go_to(self.page - 1);
                }
                return Vec::new();
            }
            _ => {}
        }
        match self.step() {
            StepId::InstallCli => self.simple_page_key(key),
            StepId::Welcome => self.welcome_key(key),
            StepId::Models => self.models_key(key),
            StepId::Behavior => self.behavior_key(key),
            StepId::WebSearch => self.search_key(key),
            StepId::Benchmarks => self.benchmarks_key(key),
            StepId::Import => self.import_key(key),
            StepId::Service => self.service_key(key),
            StepId::Anywhere => self.anywhere_key(key),
            StepId::Surfaces => self.surfaces_key(key),
            StepId::Preferences => self.prefs_key(key),
            StepId::Reporting => self.reporting_key(key),
            StepId::Done => self.done_key(key),
        }
    }

    /// Keys of the session-start card: connect now, later, or never ask again.
    fn card_key(&mut self, key: KeyEvent) -> Vec<Effect> {
        match key.code {
            KeyCode::Enter => {
                if self.running(&Purpose::ConnectClaude) {
                    return Vec::new();
                }
                vec![Effect::Run {
                    action: Action::ConnectClaude,
                    purpose: Purpose::ConnectClaude,
                }]
            }
            KeyCode::Char('n') => {
                self.card_dismissed = true;
                self.exit = Some(Exit::Left);
                Vec::new()
            }
            KeyCode::Esc | KeyCode::Char('l') | KeyCode::Char('q') => {
                self.exit = Some(Exit::Left);
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    fn simple_page_key(&mut self, key: KeyEvent) -> Vec<Effect> {
        match key.code {
            KeyCode::Enter => self.advance(),
            _ => Vec::new(),
        }
    }

    fn welcome_key(&mut self, key: KeyEvent) -> Vec<Effect> {
        match key.code {
            KeyCode::Enter => self.start_batch(),
            KeyCode::Char('n') | KeyCode::Right => self.advance(),
            KeyCode::Char('f') if self.session.gate().complete => self.finish(),
            _ => Vec::new(),
        }
    }

    /// Whether the status of `step` reads as done in the rail.
    pub fn step_done(&self, index: usize) -> bool {
        let step = &self.session.plan[index];
        step.satisfied || self.session.state.status(step.id) == StepStatus::Done
    }

    /// A one-line label for what Enter does right now (the footer chip).
    pub fn enter_label(&self) -> String {
        if self.entry.is_some() {
            return "Save".into();
        }
        match self.step() {
            StepId::Welcome => "Set up everything recommended".into(),
            StepId::Models => self.models_enter_label(),
            StepId::WebSearch => self.search_enter_label(),
            StepId::Import => self.import_enter_label(),
            StepId::Service => self.service_enter_label(),
            StepId::Anywhere => {
                if self.session.machine.anywhere.signed_in {
                    "Continue".into()
                } else {
                    "Set it up".into()
                }
            }
            StepId::Surfaces => {
                if self.session.machine.desktop.installed {
                    "Continue".into()
                } else {
                    "Install the desktop app".into()
                }
            }
            StepId::Done if self.launches_session => "Start a session".into(),
            StepId::Done => "Finish".into(),
            _ => "Continue".into(),
        }
    }

    pub fn mode(&self) -> PlanMode {
        self.session.mode
    }
}
