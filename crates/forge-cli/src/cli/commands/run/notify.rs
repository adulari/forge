//! `[notifications]`: a desktop notification, terminal bell and optional sound when a long turn
//! finishes, fails, or Forge is waiting on the user while they are looking elsewhere.

use std::time::{Duration, Instant};

use forge_config::notifications::{NotificationsConfig, NotifySound};

/// Install marker for the one-time "Enable notifications?" invitation.
pub(crate) const INVITE_NOTICE: &str = "notifications-invite";
/// A first turn must run at least this long before the invitation appears.
const INVITE_MIN_SECS: u64 = 30;
const TITLE_MAX: usize = 80;
const BODY_MAX: usize = 240;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Alert {
    TurnDone { secs: u64 },
    TurnFailed { secs: u64 },
    NeedsInput { what: String },
    Invite { secs: u64 },
}

impl Alert {
    pub(crate) fn body(&self) -> String {
        match self {
            Alert::TurnDone { secs } => format!("Turn finished after {}s", secs),
            Alert::TurnFailed { secs } => format!("Turn failed after {}s", secs),
            Alert::NeedsInput { what } => {
                let what = sanitize(what, 120);
                if what.is_empty() {
                    "Waiting for your input".to_string()
                } else {
                    format!("Waiting for you: {what}")
                }
            }
            Alert::Invite { secs } => format!("That turn took {secs}s"),
        }
    }

    /// The one-time transcript line offered after a first long turn.
    pub(crate) fn invite_text(secs: u64) -> String {
        format!(
            "That turn took {secs}s. Enable notifications? Forge can alert you when a long turn \
             finishes or needs you. Turn it on in /config under Notifications."
        )
    }
}

/// Alert text is untrusted (it can echo a shell command or model output): escape sequences and
/// control characters are dropped, whitespace collapses to single spaces, and the result is cut
/// to `max` characters with an ellipsis.
pub(crate) fn sanitize(text: &str, max: usize) -> String {
    let mut clean = String::with_capacity(text.len().min(max * 4));
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            match chars.next() {
                Some('[') => {
                    for n in chars.by_ref() {
                        if ('@'..='~').contains(&n) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    while let Some(n) = chars.next() {
                        if n == '\x07' || (n == '\x1b' && chars.next_if_eq(&'\\').is_some()) {
                            break;
                        }
                    }
                }
                _ => {}
            }
        } else if c.is_whitespace() {
            clean.push(' ');
        } else if !c.is_control() {
            clean.push(c);
        }
    }
    let collapsed = clean.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= max {
        return collapsed;
    }
    let mut cut: String = collapsed.chars().take(max.saturating_sub(1)).collect();
    cut.truncate(cut.trim_end().len());
    cut.push('…');
    cut
}

/// Watches the loop's busy flag, error count and pending questions and says when an [`Alert`] is
/// due.
pub(crate) struct Tracker {
    cfg: NotificationsConfig,
    turn_started: Option<Instant>,
    errors_at_start: u64,
    alerted_for: Option<String>,
    invite_open: bool,
}

impl Tracker {
    pub(crate) fn new(cfg: NotificationsConfig) -> Self {
        Self {
            cfg,
            turn_started: None,
            errors_at_start: 0,
            alerted_for: None,
            invite_open: false,
        }
    }

    /// Offer the one-time invitation after the first long turn, while notifications are off.
    pub(crate) fn with_invite(mut self, open: bool) -> Self {
        self.invite_open = open && !self.cfg.enabled;
        self
    }

    pub(crate) fn enabled(&self) -> bool {
        self.cfg.enabled
            && (self.cfg.desktop || self.cfg.bell || self.cfg.sound != NotifySound::Off)
    }

    pub(crate) fn watching(&self) -> bool {
        self.enabled() || self.invite_open
    }

    /// `awaiting` is the text of a pending permission or ask_user question and `errors` the count
    /// of turn errors so far. Call once per loop iteration; a prompt alerts once, a turn alerts
    /// when it ends.
    pub(crate) fn observe(
        &mut self,
        busy: bool,
        awaiting: Option<&str>,
        errors: u64,
        unfocused: bool,
        now: Instant,
    ) -> Option<Alert> {
        let live = self.enabled();
        let watching = unfocused || self.cfg.when_focused;
        let mut alert = None;

        match (busy, self.turn_started) {
            (true, None) => {
                self.turn_started = Some(now);
                self.errors_at_start = errors;
            }
            (false, Some(start)) => {
                self.turn_started = None;
                let ran = now.duration_since(start);
                let secs = ran.as_secs();
                if live {
                    if errors > self.errors_at_start {
                        if watching && self.cfg.on_error {
                            alert = Some(Alert::TurnFailed { secs });
                        }
                    } else if watching
                        && self.cfg.on_done
                        && ran >= Duration::from_secs(self.cfg.min_turn_secs)
                    {
                        alert = Some(Alert::TurnDone { secs });
                    }
                } else if self.invite_open && secs >= INVITE_MIN_SECS {
                    self.invite_open = false;
                    alert = Some(Alert::Invite { secs });
                }
            }
            _ => {}
        }

        if !live {
            return alert;
        }
        match awaiting {
            Some(text) if self.alerted_for.as_deref() != Some(text) => {
                self.alerted_for = Some(text.to_string());
                if watching && self.cfg.on_input && alert.is_none() {
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
        if matches!(alert, Alert::Invite { .. }) {
            return;
        }
        if self.cfg.bell {
            use std::io::Write;
            let mut out = std::io::stdout();
            let _ = out.write_all(b"\x07");
            let _ = out.flush();
        }
        let title = sanitize("Forge", TITLE_MAX);
        let body = sanitize(&alert.body(), BODY_MAX);
        if self.cfg.desktop {
            desktop(&title, &body, self.cfg.sound == NotifySound::Default);
        }
        if let NotifySound::File(path) = &self.cfg.sound {
            play_file(path);
        }
    }
}

/// `osascript` source for a notification; quotes and backslashes are neutralised so alert text
/// (which can echo a shell command) cannot break out of the AppleScript string.
pub(crate) fn osascript_source(title: &str, body: &str, sound: bool) -> String {
    let clean = |s: &str| s.replace(['\\', '"'], "'");
    let sound = if sound { " sound name \"default\"" } else { "" };
    format!(
        "display notification \"{}\" with title \"{}\"{sound}",
        clean(body),
        clean(title)
    )
}

fn desktop(title: &str, body: &str, sound: bool) {
    let cmd = if cfg!(target_os = "macos") {
        let mut c = std::process::Command::new("osascript");
        c.args(["-e", &osascript_source(title, body, sound)]);
        c
    } else if cfg!(target_os = "linux") {
        let mut c = std::process::Command::new("notify-send");
        c.arg("--app-name=Forge");
        if sound {
            c.arg("--hint=string:sound-name:message-new-instant");
        }
        c.args(["--", title, body]);
        c
    } else {
        return;
    };
    spawn_detached(cmd);
}

fn play_file(path: &str) {
    let player: &[&str] = if cfg!(target_os = "macos") {
        &["afplay"]
    } else if cfg!(target_os = "linux") {
        &["paplay", "aplay"]
    } else {
        return;
    };
    for name in player {
        let mut cmd = std::process::Command::new(name);
        cmd.arg(if path.starts_with('-') {
            format!("./{path}")
        } else {
            path.to_string()
        });
        if spawn_detached(cmd) {
            return;
        }
    }
}

fn spawn_detached(mut cmd: std::process::Command) -> bool {
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    // Reaped on a throwaway thread so a finished notifier never lingers as a zombie.
    match cmd.spawn() {
        Ok(mut child) => {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
            true
        }
        Err(_) => false,
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
        assert_eq!(t.observe(true, None, 0, true, t0), None);
        assert_eq!(
            t.observe(false, None, 0, true, t0 + Duration::from_secs(99)),
            None
        );
        assert_eq!(t.observe(true, Some("allow rm?"), 0, true, t0), None);
    }

    #[test]
    fn long_unfocused_turn_alerts_once_when_it_ends() {
        let mut t = tracker(|_| {});
        let t0 = Instant::now();
        assert_eq!(t.observe(true, None, 0, true, t0), None);
        assert_eq!(
            t.observe(true, None, 0, true, t0 + Duration::from_secs(20)),
            None
        );
        assert_eq!(
            t.observe(false, None, 0, true, t0 + Duration::from_secs(25)),
            Some(Alert::TurnDone { secs: 25 })
        );
        assert_eq!(
            t.observe(false, None, 0, true, t0 + Duration::from_secs(26)),
            None
        );
    }

    #[test]
    fn short_turn_or_focused_window_stays_quiet() {
        let mut t = tracker(|_| {});
        let t0 = Instant::now();
        t.observe(true, None, 0, true, t0);
        assert_eq!(
            t.observe(false, None, 0, true, t0 + Duration::from_secs(3)),
            None
        );
        t.observe(true, None, 0, false, t0);
        assert_eq!(
            t.observe(false, None, 0, false, t0 + Duration::from_secs(60)),
            None
        );
    }

    #[test]
    fn when_focused_opts_in_for_terminals_without_focus_events() {
        let mut t = tracker(|c| c.when_focused = true);
        let t0 = Instant::now();
        t.observe(true, None, 0, false, t0);
        assert!(t
            .observe(false, None, 0, false, t0 + Duration::from_secs(60))
            .is_some());
    }

    #[test]
    fn a_pending_question_alerts_once_and_rearms_after_it_clears() {
        let mut t = tracker(|_| {});
        let t0 = Instant::now();
        t.observe(true, None, 0, true, t0);
        let first = t.observe(true, Some("Allow shell: rm -rf x?"), 0, true, t0);
        assert!(matches!(first, Some(Alert::NeedsInput { .. })));
        assert_eq!(
            t.observe(true, Some("Allow shell: rm -rf x?"), 0, true, t0),
            None
        );
        t.observe(true, None, 0, true, t0);
        assert!(t
            .observe(true, Some("Allow shell: rm -rf x?"), 0, true, t0)
            .is_some());
    }

    #[test]
    fn question_while_focused_is_silent_but_not_replayed_on_blur() {
        let mut t = tracker(|_| {});
        let t0 = Instant::now();
        assert_eq!(t.observe(true, Some("q"), 0, false, t0), None);
        assert_eq!(t.observe(true, Some("q"), 0, true, t0), None);
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
        let s = osascript_source("T", "say \"hi\" \\ done", false);
        assert_eq!(
            s,
            "display notification \"say 'hi' ' done\" with title \"T\""
        );
    }

    #[test]
    fn osascript_sound_is_opt_in() {
        let s = osascript_source("T", "b", true);
        assert!(s.ends_with("sound name \"default\""));
    }

    #[test]
    fn a_failed_turn_alerts_even_when_short_and_respects_its_flag() {
        let mut t = tracker(|_| {});
        let t0 = Instant::now();
        t.observe(true, None, 0, true, t0);
        assert_eq!(
            t.observe(false, None, 1, true, t0 + Duration::from_secs(2)),
            Some(Alert::TurnFailed { secs: 2 })
        );
        let mut quiet = tracker(|c| c.on_error = false);
        quiet.observe(true, None, 0, true, t0);
        assert_eq!(
            quiet.observe(false, None, 1, true, t0 + Duration::from_secs(60)),
            None
        );
    }

    #[test]
    fn errors_from_an_earlier_turn_do_not_fail_the_next_one() {
        let mut t = tracker(|_| {});
        let t0 = Instant::now();
        t.observe(true, None, 3, true, t0);
        assert_eq!(
            t.observe(false, None, 3, true, t0 + Duration::from_secs(30)),
            Some(Alert::TurnDone { secs: 30 })
        );
    }

    #[test]
    fn per_event_flags_silence_done_and_input() {
        let mut t = tracker(|c| {
            c.on_done = false;
            c.on_input = false;
        });
        let t0 = Instant::now();
        t.observe(true, None, 0, true, t0);
        assert_eq!(t.observe(true, Some("q"), 0, true, t0), None);
        assert_eq!(
            t.observe(false, None, 0, true, t0 + Duration::from_secs(60)),
            None
        );
    }

    #[test]
    fn a_sound_alone_keeps_the_tracker_live() {
        let t = tracker(|c| {
            c.desktop = false;
            c.bell = false;
            c.sound = NotifySound::Default;
        });
        assert!(t.enabled());
        let silent = tracker(|c| {
            c.desktop = false;
            c.bell = false;
        });
        assert!(!silent.enabled());
    }

    #[test]
    fn the_invitation_comes_once_after_a_first_long_turn() {
        let mut t = Tracker::new(NotificationsConfig::default()).with_invite(true);
        let t0 = Instant::now();
        t.observe(true, None, 0, false, t0);
        assert_eq!(
            t.observe(false, None, 0, false, t0 + Duration::from_secs(29)),
            None
        );
        t.observe(true, None, 0, false, t0);
        assert_eq!(
            t.observe(false, None, 0, false, t0 + Duration::from_secs(31)),
            Some(Alert::Invite { secs: 31 })
        );
        t.observe(true, None, 0, false, t0);
        assert_eq!(
            t.observe(false, None, 0, false, t0 + Duration::from_secs(90)),
            None
        );
    }

    #[test]
    fn no_invitation_when_notifications_are_already_on_or_declined() {
        let on = Tracker::new(NotificationsConfig {
            enabled: true,
            ..Default::default()
        })
        .with_invite(true);
        assert!(!on.invite_open);
        let mut declined = Tracker::new(NotificationsConfig::default()).with_invite(false);
        let t0 = Instant::now();
        declined.observe(true, None, 0, false, t0);
        assert_eq!(
            declined.observe(false, None, 0, false, t0 + Duration::from_secs(99)),
            None
        );
    }

    #[test]
    fn sanitize_strips_escapes_and_controls() {
        assert_eq!(
            sanitize(
                "\x1b[31mred\x1b[0m\x07 \x1b]0;title\x07ok\ttab\nline\r\x00",
                80
            ),
            "red ok tab line"
        );
        assert_eq!(
            sanitize("a\x1b]8;;http://x\x1b\\link\x1b]8;;\x1b\\", 80),
            "alink"
        );
    }

    #[test]
    fn sanitize_truncates_on_a_character_boundary() {
        let long = "é".repeat(500);
        let cut = sanitize(&long, BODY_MAX);
        assert_eq!(cut.chars().count(), BODY_MAX);
        assert!(cut.ends_with('…'));
        assert_eq!(
            sanitize(&"x".repeat(100), TITLE_MAX).chars().count(),
            TITLE_MAX
        );
        assert_eq!(sanitize("short", TITLE_MAX), "short");
    }
}
