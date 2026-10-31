//! The leader key and key sequences: a chord such as Ctrl+X followed by a second key, held for a
//! short window with a which-key strip listing what each next key does.
//!
//! [`Leader`] is pure state: it is fed key events and answers "not mine", "consumed" or "run this
//! action", so it is testable without a terminal.

use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent};
use forge_config::{KeySequence, KeybindsConfig};
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;

use crate::keybinds::{combo_display, matches};
use crate::theme::styles;

/// What a key press did to the sequence in progress.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Step {
    /// Not part of any sequence: handle it as an ordinary key.
    Pass,
    /// Taken by a sequence that is waiting for more, or that was abandoned.
    Consumed,
    /// A sequence finished: run this action.
    Fire(String),
}

pub(super) struct Leader {
    sequences: Vec<KeySequence>,
    timeout: Duration,
    /// Indexes into `sequences` still alive, and how many steps have been typed.
    live: Vec<usize>,
    depth: usize,
    deadline: Option<Instant>,
    /// The keybinds `sequences` were built from, so a config reload rebuilds them.
    source: KeybindsConfig,
}

impl Leader {
    pub(super) fn new(keybinds: &KeybindsConfig) -> Self {
        Self {
            sequences: keybinds.effective_sequences(),
            timeout: Duration::from_millis(keybinds.leader_timeout_ms.max(100)),
            live: Vec::new(),
            depth: 0,
            deadline: None,
            source: keybinds.clone(),
        }
    }

    /// Rebuild after the keymap changed (hot reload, the configurator). Drops a half-typed one.
    pub(super) fn sync(&mut self, keybinds: &KeybindsConfig) {
        if self.source != *keybinds {
            *self = Self::new(keybinds);
        }
    }

    #[cfg(test)]
    pub(super) fn pending(&self) -> bool {
        self.depth > 0
    }

    fn reset(&mut self) {
        self.live.clear();
        self.depth = 0;
        self.deadline = None;
    }

    /// Drop a half-typed sequence whose window has passed. True when something was dropped, so
    /// the caller repaints and the strip goes away.
    pub(super) fn expire(&mut self, now: Instant) -> bool {
        if self.deadline.is_some_and(|d| now >= d) {
            self.reset();
            return true;
        }
        false
    }

    pub(super) fn on_key(&mut self, ev: &KeyEvent, now: Instant) -> Step {
        if self.sequences.is_empty() {
            return Step::Pass;
        }
        if self.depth > 0 && ev.code == KeyCode::Esc {
            self.reset();
            return Step::Consumed;
        }
        let pool: Vec<usize> = if self.depth == 0 {
            (0..self.sequences.len()).collect()
        } else {
            self.live.clone()
        };
        let hit: Vec<usize> = pool
            .into_iter()
            .filter(|&i| {
                self.sequences[i]
                    .steps
                    .get(self.depth)
                    .is_some_and(|step| matches(step, ev))
            })
            .collect();
        if hit.is_empty() {
            let started = self.depth > 0;
            self.reset();
            return if started { Step::Consumed } else { Step::Pass };
        }
        if let Some(&done) = hit
            .iter()
            .find(|&&i| self.sequences[i].steps.len() == self.depth + 1)
        {
            let action = self.sequences[done].action.clone();
            self.reset();
            return Step::Fire(action);
        }
        self.live = hit;
        self.depth += 1;
        self.deadline = Some(now + self.timeout);
        Step::Consumed
    }

    /// What the strip shows: the keys typed so far and, per live sequence, its next key and action.
    fn strip(&self) -> Option<(String, Vec<(String, String)>)> {
        if self.depth == 0 {
            return None;
        }
        let first = self.live.first().map(|&i| &self.sequences[i])?;
        let typed = first.steps[..self.depth]
            .iter()
            .map(combo_display)
            .collect::<Vec<_>>()
            .join(" ");
        let mut entries: Vec<(String, String)> = self
            .live
            .iter()
            .map(|&i| {
                let seq = &self.sequences[i];
                let rest = seq.steps[self.depth..]
                    .iter()
                    .map(combo_display)
                    .collect::<Vec<_>>()
                    .join(" ");
                (rest, seq.action.replace('_', " "))
            })
            .collect();
        entries.sort();
        Some((typed, entries))
    }

    /// Draw the which-key strip over the bottom rows of `frame`, if a sequence is in progress.
    pub(super) fn render(&self, frame: &mut Frame) {
        let Some((typed, entries)) = self.strip() else {
            return;
        };
        let area = frame.area();
        let width = area.width as usize;
        if width < 12 || area.height < 4 {
            return;
        }
        let mut rows: Vec<Vec<Span<'static>>> = vec![vec![
            Span::styled(format!(" {typed} "), styles::brand().bold()),
            Span::styled("then", styles::faint()),
        ]];
        let mut used: usize = rows[0].iter().map(|s| s.content.width()).sum();
        for (key, label) in entries {
            let cell = key.width() + 1 + label.width() + 3;
            if used + cell > width && used > 1 {
                rows.push(vec![Span::raw(" ")]);
                used = 1;
            }
            let row = rows.last_mut().expect("at least one row");
            row.push(Span::raw("  "));
            row.push(Span::styled(key, styles::key_chip()));
            row.push(Span::styled(format!(" {label}"), styles::muted()));
            used += cell;
        }
        let height = (rows.len() as u16 + 1).min(area.height / 2).max(2);
        let strip = Rect {
            x: area.x,
            y: area.bottom().saturating_sub(height),
            width: area.width,
            height,
        };
        frame.render_widget(Clear, strip);
        let mut lines: Vec<Line<'static>> = rows.into_iter().map(Line::from).collect();
        lines.push(Line::from(Span::styled(" Esc cancels", styles::faint())));
        lines.truncate(height as usize);
        frame.render_widget(Paragraph::new(lines), strip);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    fn leader(toml_leader: bool) -> Leader {
        let kb = KeybindsConfig {
            leader: toml_leader.then(forge_config::default_leader),
            ..Default::default()
        };
        Leader::new(&kb)
    }

    fn ctrl_x() -> KeyEvent {
        KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL)
    }

    fn plain(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    #[test]
    fn nothing_happens_while_the_leader_is_off() {
        let mut l = leader(false);
        assert_eq!(l.on_key(&ctrl_x(), Instant::now()), Step::Pass);
        assert!(!l.pending());
    }

    #[test]
    fn leader_then_n_fires_new_session() {
        let mut l = leader(true);
        let now = Instant::now();
        assert_eq!(l.on_key(&ctrl_x(), now), Step::Consumed);
        assert!(l.pending());
        assert_eq!(l.on_key(&plain('n'), now), Step::Fire("new_session".into()));
        assert!(!l.pending());
    }

    #[test]
    fn an_unknown_follow_up_is_swallowed_and_ends_the_sequence() {
        let mut l = leader(true);
        let now = Instant::now();
        l.on_key(&ctrl_x(), now);
        assert_eq!(l.on_key(&plain('9'), now), Step::Consumed);
        assert!(!l.pending());
        assert_eq!(l.on_key(&plain('9'), now), Step::Pass, "typing resumes");
    }

    #[test]
    fn escape_cancels_and_the_window_expires() {
        let mut l = leader(true);
        let now = Instant::now();
        l.on_key(&ctrl_x(), now);
        assert_eq!(
            l.on_key(&KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), now),
            Step::Consumed
        );
        assert!(!l.pending());
        l.on_key(&ctrl_x(), now);
        assert!(!l.expire(now + Duration::from_millis(1999)));
        assert!(l.expire(now + Duration::from_millis(2000)));
        assert!(!l.pending());
        assert!(!l.expire(now + Duration::from_secs(60)));
    }

    #[test]
    fn the_strip_lists_the_follow_ups() {
        let mut l = leader(true);
        l.on_key(&ctrl_x(), Instant::now());
        let (typed, entries) = l.strip().expect("strip while pending");
        assert_eq!(typed, "Ctrl+X");
        assert!(entries.contains(&("N".to_string(), "new session".to_string())));
        assert!(entries.len() >= 10);
    }

    #[test]
    fn multi_step_sequences_walk_to_the_end() {
        let mut kb = KeybindsConfig::default();
        kb.sequences.insert("help".into(), "ctrl+g a b".into());
        let mut l = Leader::new(&kb);
        let now = Instant::now();
        let ctrl_g = KeyEvent::new(KeyCode::Char('g'), KeyModifiers::CONTROL);
        assert_eq!(l.on_key(&ctrl_g, now), Step::Consumed);
        assert_eq!(l.on_key(&plain('a'), now), Step::Consumed);
        assert_eq!(l.on_key(&plain('b'), now), Step::Fire("help".into()));
    }

    #[test]
    fn a_reload_rebuilds_the_table() {
        let mut l = leader(false);
        let kb = KeybindsConfig {
            leader: Some(forge_config::default_leader()),
            ..Default::default()
        };
        l.sync(&kb);
        assert_eq!(l.on_key(&ctrl_x(), Instant::now()), Step::Consumed);
    }
}
