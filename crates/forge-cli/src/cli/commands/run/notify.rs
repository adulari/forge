//! `[notifications]`: a desktop notification and terminal bell when a long turn finishes or Forge
//! is waiting on the user while they are looking elsewhere.

use std::time::{Duration, Instant};

use forge_config::notifications::NotificationsConfig;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Alert {
    TurnDone { secs: u64 },
    NeedsInput { what: String },
}

impl Alert {
    pub(crate) fn body(&self) -> String {
        match self {
            Alert::TurnDone { secs } => format!("Turn finished after {}s", secs),
            Alert::NeedsInput { what } => {
                let what: String = what.split_whitespace().collect::<Vec<_>>().join(" ");
                let what: String = what.chars().take(120).collect();
                if what.is_empty() {
                    "Waiting for your input".to_string()
                } else {
                    format!("Waiting for you: {what}")
                }
            }
        }
    }
}

/// Watches the loop's busy flag and pending questions and says when an [`Alert`] is due.
pub(crate) struct Tracker {
    cfg: NotificationsConfig,
    turn_started: Option<Instant>,
    alerted_for: Option<String>,
}

impl Tracker {
    pub(crate) fn new(cfg: NotificationsConfig) -> Self {
        Self {
            cfg,
            turn_started: None,
            alerted_for: None,
        }
    }

    pub(crate) fn enabled(&self) -> bool {
        self.cfg.enabled && (self.cfg.desktop || self.cfg.bell)
    }

    /// `awaiting` is the text of a pending permission or ask_user question. Call once per loop
    /// iteration; a prompt alerts once, a turn alerts when it ends.
    pub(crate) fn observe(
        &mut self,
        busy: bool,
        awaiting: Option<&str>,
        unfocused: bool,
        now: Instant,
    ) -> Option<Alert> {
        if !self.enabled() {
            return None;
        }
        let watching = unfocused || self.cfg.when_focused;
        let mut alert = None;

        match (busy, self.turn_started) {
            (true, None) => self.turn_started = Some(now),
            (false, Some(start)) => {
                self.turn_started = None;
                let ran = now.duration_since(start);
                if watching && ran >= Duration::from_secs(self.cfg.min_turn_secs) {
                    alert = Some(Alert::TurnDone {
                        secs: ran.as_secs(),
                    });
                }
            }
            _ => {}
        }

        match awaiting {
            Some(text) if self.alerted_for.as_deref() != Some(text) => {
                self.alerted_for = Some(text.to_string());
                if watching && alert.is_none() {
                    alert = Some(Alert::NeedsInput {
                        what: text.to_string(),
                    });
                }
            }
            None => self.alerted_for = None,
            _ => {}
        }
        alert
    }

    pub(crate) fn deliver(&self, alert: &Alert) {
        if self.cfg.bell {
            use std::io::Write;
            let mut out = std::io::stdout();
            let _ = out.write_all(b"\x07");
            let _ = out.flush();
        }
        if self.cfg.desktop {
            desktop("Forge", &alert.body());
        }
    }
}

/// `osascript` source for a notification; quotes and backslashes are neutralised so alert text
/// (which can echo a shell command) cannot break out of the AppleScript string.
pub(crate) fn osascript_source(title: &str, body: &str) -> String {
    let clean = |s: &str| s.replace(['\\', '"'], "'");
    format!(
        "display notification \"{}\" with title \"{}\"",
        clean(body),
        clean(title)
    )
}

fn desktop(title: &str, body: &str) {
    let mut cmd = if cfg!(target_os = "macos") {
        let mut c = std::process::Command::new("osascript");
        c.args(["-e", &osascript_source(title, body)]);
        c
    } else if cfg!(target_os = "linux") {
        let mut c = std::process::Command::new("notify-send");
        c.args(["--app-name=Forge", "--", title, body]);
        c
    } else {
        return;
    };
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    // Reaped on a throwaway thread so a finished notifier never lingers as a zombie.
    if let Ok(mut child) = cmd.spawn() {
        std::thread::spawn(move || {
            let _ = child.wait();
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tracker(f: impl FnOnce(&mut NotificationsConfig)) -> Tracker {
        let mut cfg = NotificationsConfig {
            enabled: true,
            min_turn_secs: 10,
            ..Default::default()
        };
        f(&mut cfg);
        Tracker::new(cfg)
    }

    #[test]
    fn disabled_by_default_never_alerts() {
        let mut t = Tracker::new(NotificationsConfig::default());
        let t0 = Instant::now();
        assert_eq!(t.observe(true, None, true, t0), None);
        assert_eq!(
            t.observe(false, None, true, t0 + Duration::from_secs(99)),
            None
        );
        assert_eq!(t.observe(true, Some("allow rm?"), true, t0), None);
    }

    #[test]
    fn long_unfocused_turn_alerts_once_when_it_ends() {
        let mut t = tracker(|_| {});
        let t0 = Instant::now();
        assert_eq!(t.observe(true, None, true, t0), None);
        assert_eq!(
            t.observe(true, None, true, t0 + Duration::from_secs(20)),
            None
        );
        assert_eq!(
            t.observe(false, None, true, t0 + Duration::from_secs(25)),
            Some(Alert::TurnDone { secs: 25 })
        );
        assert_eq!(
            t.observe(false, None, true, t0 + Duration::from_secs(26)),
            None
        );
    }

    #[test]
    fn short_turn_or_focused_window_stays_quiet() {
        let mut t = tracker(|_| {});
        let t0 = Instant::now();
        t.observe(true, None, true, t0);
        assert_eq!(
            t.observe(false, None, true, t0 + Duration::from_secs(3)),
            None
        );
        t.observe(true, None, false, t0);
        assert_eq!(
            t.observe(false, None, false, t0 + Duration::from_secs(60)),
            None
        );
    }

    #[test]
    fn when_focused_opts_in_for_terminals_without_focus_events() {
        let mut t = tracker(|c| c.when_focused = true);
        let t0 = Instant::now();
        t.observe(true, None, false, t0);
        assert!(t
            .observe(false, None, false, t0 + Duration::from_secs(60))
            .is_some());
    }

    #[test]
    fn a_pending_question_alerts_once_and_rearms_after_it_clears() {
        let mut t = tracker(|_| {});
        let t0 = Instant::now();
        t.observe(true, None, true, t0);
        let first = t.observe(true, Some("Allow shell: rm -rf x?"), true, t0);
        assert!(matches!(first, Some(Alert::NeedsInput { .. })));
        assert_eq!(
            t.observe(true, Some("Allow shell: rm -rf x?"), true, t0),
            None
        );
        t.observe(true, None, true, t0);
        assert!(t
            .observe(true, Some("Allow shell: rm -rf x?"), true, t0)
            .is_some());
    }

    #[test]
    fn question_while_focused_is_silent_but_not_replayed_on_blur() {
        let mut t = tracker(|_| {});
        let t0 = Instant::now();
        assert_eq!(t.observe(true, Some("q"), false, t0), None);
        assert_eq!(t.observe(true, Some("q"), true, t0), None);
    }

    #[test]
    fn bodies_are_single_line_and_bounded() {
        let long = format!("Allow\nshell:   {}", "x".repeat(500));
        let body = Alert::NeedsInput { what: long }.body();
        assert!(!body.contains('\n') && body.chars().count() < 160);
        assert_eq!(Alert::TurnDone { secs: 7 }.body(), "Turn finished after 7s");
    }

    #[test]
    fn osascript_text_cannot_escape_its_string() {
        let s = osascript_source("T", "say \"hi\" \\ done");
        assert_eq!(
            s,
            "display notification \"say 'hi' ' done\" with title \"T\""
        );
    }
}
