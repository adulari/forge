//! Pauses the `run_code` deadline while the user answers a permission prompt.
//!
//! An inner write in a guarded mode asks the user, and the script must not lose its budget while
//! the prompt sits on screen. The session's presenter is wrapped for the length of one script;
//! every blocking interaction (`confirm`, `ask`, `ask_form`, `read_line`) adds its wall time to a
//! shared counter that the pump subtracts from the elapsed time.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use forge_types::{
    Answer, ConfirmOutcome, Presenter, PresenterEvent, QChoice, Question, SideEffect,
};

/// Where the real presenter comes back when the wrapper drops.
pub(crate) type PresenterSlot = Arc<Mutex<Option<Box<dyn Presenter>>>>;

/// Milliseconds spent waiting on the user since the clock was installed.
#[derive(Clone, Default)]
pub(crate) struct PromptClock(Arc<AtomicU64>);

impl PromptClock {
    pub(crate) fn waited(&self) -> std::time::Duration {
        std::time::Duration::from_millis(self.0.load(Ordering::Relaxed))
    }

    fn timed<T>(&self, f: impl FnOnce() -> T) -> T {
        let start = Instant::now();
        let out = f();
        self.0
            .fetch_add(start.elapsed().as_millis() as u64, Ordering::Relaxed);
        out
    }
}

/// The real presenter, handed back through `slot` when the wrapper drops.
pub(crate) struct Timed {
    inner: Option<Box<dyn Presenter>>,
    clock: PromptClock,
    slot: PresenterSlot,
}

impl Timed {
    fn inner(&mut self) -> &mut dyn Presenter {
        self.inner
            .as_deref_mut()
            .expect("the wrapped presenter is present until drop")
    }
}

impl Drop for Timed {
    fn drop(&mut self) {
        if let Ok(mut slot) = self.slot.lock() {
            *slot = self.inner.take();
        }
    }
}

impl Presenter for Timed {
    fn emit(&mut self, event: PresenterEvent) {
        self.inner().emit(event);
    }
    fn confirm(&mut self, tool: &str, side_effect: SideEffect) -> ConfirmOutcome {
        let clock = self.clock.clone();
        clock.timed(|| self.inner().confirm(tool, side_effect))
    }
    fn ask(&mut self, question: &str, options: &[QChoice], allow_other: bool) -> String {
        let clock = self.clock.clone();
        clock.timed(|| self.inner().ask(question, options, allow_other))
    }
    fn ask_form(&mut self, questions: &[Question]) -> Vec<Answer> {
        let clock = self.clock.clone();
        clock.timed(|| self.inner().ask_form(questions))
    }
    fn read_line(&mut self) -> Option<String> {
        let clock = self.clock.clone();
        clock.timed(|| self.inner().read_line())
    }
    fn is_attended(&self) -> bool {
        self.inner.as_deref().is_none_or(|p| p.is_attended())
    }
    fn recap_sink(&self) -> Option<Box<dyn Presenter>> {
        self.inner.as_deref().and_then(|p| p.recap_sink())
    }
    fn consumes_recap(&self) -> bool {
        self.inner.as_deref().is_some_and(|p| p.consumes_recap())
    }
    fn consumes_suggestions(&self) -> bool {
        self.inner
            .as_deref()
            .is_some_and(|p| p.consumes_suggestions())
    }
}

/// Placeholder that holds the presenter field for the instant of a swap. Never shown anything.
struct Parked;

impl Presenter for Parked {
    fn emit(&mut self, _event: PresenterEvent) {}
    fn confirm(&mut self, _tool: &str, _side_effect: SideEffect) -> ConfirmOutcome {
        ConfirmOutcome::Deny
    }
    fn ask(&mut self, _question: &str, _options: &[QChoice], _allow_other: bool) -> String {
        forge_types::NO_ANSWER.to_string()
    }
    fn read_line(&mut self) -> Option<String> {
        None
    }
}

/// Wraps `presenter` in place and returns the clock plus the slot the real presenter comes back
/// through. Pair every call with [`uninstall`].
pub(crate) fn install(presenter: &mut Box<dyn Presenter>) -> (PromptClock, PresenterSlot) {
    let clock = PromptClock::default();
    let slot = Arc::new(Mutex::new(None));
    let real = std::mem::replace(presenter, Box::new(Parked));
    *presenter = Box::new(Timed {
        inner: Some(real),
        clock: clock.clone(),
        slot: Arc::clone(&slot),
    });
    (clock, slot)
}

/// Puts the real presenter back. If the turn future was dropped mid-script this never runs and
/// the wrapper stays in place, which is harmless: it delegates every call.
pub(crate) fn uninstall(presenter: &mut Box<dyn Presenter>, slot: &PresenterSlot) {
    let wrapper = std::mem::replace(presenter, Box::new(Parked));
    drop(wrapper);
    if let Some(real) = slot.lock().ok().and_then(|mut s| s.take()) {
        *presenter = real;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Slow(Arc<AtomicU64>);
    impl Presenter for Slow {
        fn emit(&mut self, _event: PresenterEvent) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
        fn confirm(&mut self, _tool: &str, _side_effect: SideEffect) -> ConfirmOutcome {
            std::thread::sleep(std::time::Duration::from_millis(30));
            ConfirmOutcome::Allow
        }
        fn ask(&mut self, _q: &str, _o: &[QChoice], _a: bool) -> String {
            String::new()
        }
        fn read_line(&mut self) -> Option<String> {
            None
        }
    }

    #[test]
    fn prompt_time_is_counted_and_the_real_presenter_comes_back() {
        let emitted = Arc::new(AtomicU64::new(0));
        let mut presenter: Box<dyn Presenter> = Box::new(Slow(Arc::clone(&emitted)));
        let (clock, slot) = install(&mut presenter);
        presenter.emit(PresenterEvent::Warning("x".into()));
        assert_eq!(
            presenter.confirm("write_file", SideEffect::Write),
            ConfirmOutcome::Allow
        );
        assert!(clock.waited() >= std::time::Duration::from_millis(25));
        uninstall(&mut presenter, &slot);
        presenter.emit(PresenterEvent::Warning("y".into()));
        assert_eq!(
            emitted.load(Ordering::Relaxed),
            2,
            "both emits reached the real presenter"
        );
        let before = clock.waited();
        presenter.confirm("write_file", SideEffect::Write);
        assert_eq!(
            clock.waited(),
            before,
            "the clock is detached after uninstall"
        );
    }
}
