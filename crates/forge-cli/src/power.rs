//! Keeping the host awake while Forge works (`[power] keep_awake`).
//!
//! A sleeping laptop freezes `forge serve` mid-turn and the phone driving it loses the session.
//! While the configured mode wants it, the daemon holds an OS idle/sleep inhibitor: a
//! `systemd-inhibit` child on Linux, `caffeinate` on macOS, an execution-state request on
//! Windows. Holding stops the machine sleeping, not the screen locking. The inhibitor is released
//! a short while after the work stops, so the gap between two turns does not flap it.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use forge_config::power::KeepAwake;

use crate::serve::SessionRegistry;

const TICK: Duration = Duration::from_secs(1);
const INPUT_REFRESH: Duration = Duration::from_secs(2);
const CONFIG_REFRESH: Duration = Duration::from_secs(15);
const RELEASE_GRACE: Duration = Duration::from_secs(30);
const RETRY_AFTER_FAILURE: Duration = Duration::from_secs(60);
const REASON: &str = "Forge is running a session";

/// What the diagnostics screen reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PowerStatus {
    pub(crate) mode: KeepAwake,
    pub(crate) held: bool,
    pub(crate) backend: &'static str,
    /// Why holding failed, when it did.
    pub(crate) problem: Option<String>,
}

fn status_cell() -> &'static Mutex<PowerStatus> {
    static CELL: OnceLock<Mutex<PowerStatus>> = OnceLock::new();
    CELL.get_or_init(|| {
        Mutex::new(PowerStatus {
            mode: KeepAwake::Off,
            held: false,
            backend: "none",
            problem: None,
        })
    })
}

pub(crate) fn status() -> PowerStatus {
    status_cell()
        .lock()
        .map(|s| s.clone())
        .unwrap_or(PowerStatus {
            mode: KeepAwake::Off,
            held: false,
            backend: "none",
            problem: None,
        })
}

/// A held inhibitor. Dropping it releases the machine to sleep again.
pub(crate) trait Hold: Send {
    /// False once the underlying inhibitor has gone away on its own.
    fn alive(&mut self) -> bool;
}

pub(crate) trait Backend: Send {
    fn name(&self) -> &'static str;
    fn acquire(&mut self, why: &str) -> Result<Box<dyn Hold>, String>;
}

/// The decision loop, separated from the OS so it can be tested with a fake backend.
pub(crate) struct Governor {
    backend: Box<dyn Backend>,
    held: Option<Box<dyn Hold>>,
    release_at: Option<Instant>,
    retry_at: Option<Instant>,
    problem: Option<String>,
}

impl Governor {
    pub(crate) fn new(backend: Box<dyn Backend>) -> Self {
        Self {
            backend,
            held: None,
            release_at: None,
            retry_at: None,
            problem: None,
        }
    }

    /// Move towards `wanted`; returns whether an inhibitor is held afterwards.
    pub(crate) fn tick(&mut self, wanted: bool, now: Instant) -> bool {
        if self.held.as_mut().is_some_and(|hold| !hold.alive()) {
            self.problem = Some("the inhibitor process exited unexpectedly".to_string());
            self.held = None;
            self.retry_at = Some(now + RETRY_AFTER_FAILURE);
        }
        if wanted {
            self.release_at = None;
            if self.held.is_none() && self.retry_at.is_none_or(|at| now >= at) {
                match self.backend.acquire(REASON) {
                    Ok(hold) => {
                        self.held = Some(hold);
                        self.problem = None;
                        self.retry_at = None;
                    }
                    Err(reason) => {
                        self.problem = Some(reason);
                        self.retry_at = Some(now + RETRY_AFTER_FAILURE);
                    }
                }
            }
        } else if self.held.is_some() {
            let due = *self.release_at.get_or_insert(now + RELEASE_GRACE);
            if now >= due {
                self.held = None;
                self.release_at = None;
            }
        } else {
            self.problem = None;
            self.retry_at = None;
        }
        self.held.is_some()
    }

    pub(crate) fn problem(&self) -> Option<&str> {
        self.problem.as_deref()
    }

    pub(crate) fn backend_name(&self) -> &'static str {
        self.backend.name()
    }
}

#[derive(Default)]
struct Inputs {
    working: AtomicBool,
    clients: AtomicUsize,
    mode: Mutex<KeepAwake>,
}

/// Start the keep-awake controller for this daemon. The OS calls happen on a dedicated thread
/// that lives as long as the daemon, since an execution-state request belongs to its thread.
pub(crate) fn start(registry: Arc<SessionRegistry>) {
    let inputs = Arc::new(Inputs::default());
    if let Ok(mut mode) = inputs.mode.lock() {
        *mode = forge_config::load().unwrap_or_default().power.keep_awake;
    }
    let refresher = inputs.clone();
    tokio::spawn(async move {
        let mut last_config = Instant::now();
        loop {
            let sessions = registry.all().await;
            let working = sessions
                .iter()
                .any(|session| session.snapshot_rx.borrow().snapshot.busy);
            let clients = registry.signals.fleet_client_count()
                + sessions
                    .iter()
                    .map(|session| session.ws_clients.load(Ordering::Relaxed))
                    .sum::<usize>();
            refresher.working.store(working, Ordering::Relaxed);
            refresher.clients.store(clients, Ordering::Relaxed);
            if last_config.elapsed() >= CONFIG_REFRESH {
                last_config = Instant::now();
                if let Ok(Ok(config)) = tokio::task::spawn_blocking(forge_config::load).await {
                    if let Ok(mut mode) = refresher.mode.lock() {
                        *mode = config.power.keep_awake;
                    }
                }
            }
            tokio::time::sleep(INPUT_REFRESH).await;
        }
    });
    let spawned = std::thread::Builder::new()
        .name("forge-power".into())
        .spawn(move || run(inputs));
    if let Err(error) = spawned {
        tracing::warn!("keep-awake controller did not start: {error}");
    }
}

fn run(inputs: Arc<Inputs>) {
    let mut governor = Governor::new(platform_backend());
    loop {
        let mode = inputs.mode.lock().map(|m| *m).unwrap_or_default();
        let wanted = mode.wants_inhibitor(
            inputs.working.load(Ordering::Relaxed),
            inputs.clients.load(Ordering::Relaxed) > 0,
        );
        let held = governor.tick(wanted, Instant::now());
        if let Ok(mut cell) = status_cell().lock() {
            *cell = PowerStatus {
                mode,
                held,
                backend: governor.backend_name(),
                problem: governor.problem().map(str::to_string),
            };
        }
        std::thread::sleep(TICK);
    }
}

fn platform_backend() -> Box<dyn Backend> {
    #[cfg(target_os = "linux")]
    return Box::new(os::SystemdInhibit);
    #[cfg(target_os = "macos")]
    return Box::new(os::Caffeinate);
    #[cfg(windows)]
    return Box::new(os::ExecutionState);
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    return Box::new(os::Unsupported);
}

#[cfg(unix)]
mod os {
    use std::process::{Child, Command, Stdio};

    use super::{Backend, Hold};

    struct Process(Child);

    impl Hold for Process {
        fn alive(&mut self) -> bool {
            matches!(self.0.try_wait(), Ok(None))
        }
    }

    impl Drop for Process {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    fn spawn_held(mut command: Command) -> Result<Box<dyn Hold>, String> {
        let mut child = command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| format!("{}: {error}", command.get_program().to_string_lossy()))?;
        std::thread::sleep(std::time::Duration::from_millis(150));
        match child.try_wait() {
            Ok(None) => Ok(Box::new(Process(child))),
            _ => Err(format!(
                "{} exited straight away (is there a session bus / logind?)",
                command.get_program().to_string_lossy()
            )),
        }
    }

    #[cfg(target_os = "linux")]
    pub(super) struct SystemdInhibit;

    #[cfg(target_os = "linux")]
    impl Backend for SystemdInhibit {
        fn name(&self) -> &'static str {
            "systemd-inhibit"
        }

        fn acquire(&mut self, why: &str) -> Result<Box<dyn Hold>, String> {
            let mut command = Command::new("systemd-inhibit");
            // The guard command ends when the daemon does, even if the daemon is killed, so a
            // crash cannot leave the machine unable to sleep.
            command.args([
                "--what=idle:sleep",
                "--who=Forge",
                "--mode=block",
                &format!("--why={why}"),
                "sh",
                "-c",
                &format!(
                    "while kill -0 {} 2>/dev/null; do sleep 5; done",
                    std::process::id()
                ),
            ]);
            spawn_held(command)
        }
    }

    #[cfg(target_os = "macos")]
    pub(super) struct Caffeinate;

    #[cfg(target_os = "macos")]
    impl Backend for Caffeinate {
        fn name(&self) -> &'static str {
            "caffeinate"
        }

        fn acquire(&mut self, _why: &str) -> Result<Box<dyn Hold>, String> {
            let mut command = Command::new("caffeinate");
            command.args(["-i", "-w", &std::process::id().to_string()]);
            spawn_held(command)
        }
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    pub(super) struct Unsupported;

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    impl Backend for Unsupported {
        fn name(&self) -> &'static str {
            "unsupported"
        }

        fn acquire(&mut self, _why: &str) -> Result<Box<dyn Hold>, String> {
            Err("keep-awake is not supported on this platform".to_string())
        }
    }
}

#[cfg(windows)]
mod os {
    use super::{Backend, Hold};

    const ES_CONTINUOUS: u32 = 0x8000_0000;
    const ES_SYSTEM_REQUIRED: u32 = 0x0000_0001;

    #[link(name = "kernel32")]
    extern "system" {
        fn SetThreadExecutionState(flags: u32) -> u32;
    }

    pub(super) struct ExecutionState;

    struct Requested;

    impl Hold for Requested {
        fn alive(&mut self) -> bool {
            true
        }
    }

    impl Drop for Requested {
        fn drop(&mut self) {
            // SAFETY: plain value argument; clears this thread's continuous request.
            unsafe { SetThreadExecutionState(ES_CONTINUOUS) };
        }
    }

    impl Backend for ExecutionState {
        fn name(&self) -> &'static str {
            "SetThreadExecutionState"
        }

        fn acquire(&mut self, _why: &str) -> Result<Box<dyn Hold>, String> {
            // SAFETY: plain value argument; the request lasts until cleared or the thread ends,
            // and this runs on the dedicated controller thread that also releases it.
            let previous = unsafe { SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED) };
            if previous == 0 {
                Err("SetThreadExecutionState failed".to_string())
            } else {
                Ok(Box::new(Requested))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use super::*;

    #[derive(Default)]
    struct Shared {
        held: AtomicUsize,
        acquired: AtomicUsize,
        fail: AtomicBool,
        die: AtomicBool,
    }

    struct FakeHold(Arc<Shared>);
    impl Hold for FakeHold {
        fn alive(&mut self) -> bool {
            !self.0.die.load(Ordering::SeqCst)
        }
    }
    impl Drop for FakeHold {
        fn drop(&mut self) {
            self.0.held.fetch_sub(1, Ordering::SeqCst);
        }
    }
    struct FakeBackend(Arc<Shared>);
    impl Backend for FakeBackend {
        fn name(&self) -> &'static str {
            "fake"
        }
        fn acquire(&mut self, _why: &str) -> Result<Box<dyn Hold>, String> {
            if self.0.fail.load(Ordering::SeqCst) {
                return Err("no bus".into());
            }
            self.0.held.fetch_add(1, Ordering::SeqCst);
            self.0.acquired.fetch_add(1, Ordering::SeqCst);
            Ok(Box::new(FakeHold(self.0.clone())))
        }
    }

    fn rig() -> (Governor, Arc<Shared>, Instant) {
        let shared = Arc::new(Shared::default());
        (
            Governor::new(Box::new(FakeBackend(shared.clone()))),
            shared,
            Instant::now(),
        )
    }

    #[test]
    fn work_takes_the_inhibitor_at_once_and_idleness_gives_it_back_after_the_grace() {
        let (mut governor, shared, t0) = rig();
        assert!(!governor.tick(false, t0));
        assert!(governor.tick(true, t0));
        assert_eq!(shared.held.load(Ordering::SeqCst), 1);
        assert!(
            governor.tick(false, t0 + Duration::from_secs(1)),
            "still in the grace"
        );
        assert!(governor.tick(false, t0 + Duration::from_secs(20)));
        assert!(!governor.tick(false, t0 + Duration::from_secs(32)));
        assert_eq!(shared.held.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn a_gap_between_turns_does_not_release_and_retake() {
        let (mut governor, shared, t0) = rig();
        governor.tick(true, t0);
        governor.tick(false, t0 + Duration::from_secs(5));
        governor.tick(true, t0 + Duration::from_secs(8));
        governor.tick(false, t0 + Duration::from_secs(9));
        assert!(
            governor.tick(false, t0 + Duration::from_secs(30)),
            "the grace restarted at 9s"
        );
        assert_eq!(shared.acquired.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_failed_acquire_is_reported_and_retried_only_after_a_pause() {
        let (mut governor, shared, t0) = rig();
        shared.fail.store(true, Ordering::SeqCst);
        assert!(!governor.tick(true, t0));
        assert_eq!(governor.problem(), Some("no bus"));
        shared.fail.store(false, Ordering::SeqCst);
        assert!(
            !governor.tick(true, t0 + Duration::from_secs(10)),
            "not retried yet"
        );
        assert!(governor.tick(true, t0 + Duration::from_secs(61)));
        assert_eq!(governor.problem(), None);
    }

    #[test]
    fn an_inhibitor_that_dies_is_noticed_and_retaken_later() {
        let (mut governor, shared, t0) = rig();
        assert!(governor.tick(true, t0));
        shared.die.store(true, Ordering::SeqCst);
        assert!(!governor.tick(true, t0 + Duration::from_secs(1)));
        assert!(governor.problem().is_some());
        shared.die.store(false, Ordering::SeqCst);
        assert!(governor.tick(true, t0 + Duration::from_secs(62)));
        assert_eq!(shared.acquired.load(Ordering::SeqCst), 2);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn the_real_linux_backend_either_holds_or_says_why() {
        let mut backend = os::SystemdInhibit;
        match backend.acquire("test") {
            Ok(mut hold) => assert!(hold.alive()),
            Err(reason) => assert!(!reason.is_empty()),
        }
    }
}
