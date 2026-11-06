//! Spend envelopes for a `/goal`: "use at most this slice of a subscription window (or this many
//! dollars) on this goal". Pure accounting and parsing; readings come from the caller, so the
//! tests inject them and nothing here reads a clock or a store.
//!
//! A quota envelope records the window utilisation when the goal starts and is spent once
//! `current - start >= slice`. A window that rolls over mid-goal carries what was used before the
//! rollover and counts from zero in the new one. A USD envelope is the session cost since the goal
//! started.

use serde::{Deserialize, Serialize};

/// A quota reading older than this is not trusted to say how much of the slice is left.
pub const ENVELOPE_MAX_READING_AGE_SECS: i64 = 10 * 60;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EnvelopeKind {
    /// `provider` is the family (`claude`, `codex`); `window` a store window kind
    /// (`weekly`, `five_hour`).
    Quota {
        provider: String,
        window: String,
    },
    Usd,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EnvelopeSpec {
    pub kind: EnvelopeKind,
    /// Percentage points of the window, or dollars for [`EnvelopeKind::Usd`].
    pub slice: f64,
}

const USAGE: &str = "use claude:20%w, codex:15%w, claude:30%5h or $5";

fn parse_window(text: &str) -> Option<&'static str> {
    match text {
        "w" | "weekly" => Some("weekly"),
        "5h" => Some("five_hour"),
        _ => None,
    }
}

impl EnvelopeSpec {
    /// Parse `claude:20%w`, `codex:15%w`, `claude:30%5h` or `$5`. With `default_provider`, the
    /// provider prefix may be left off (`30%w`), which is how `/goal budget` raises a slice.
    pub fn parse(text: &str, default_provider: Option<&str>) -> Result<Self, String> {
        let text = text.trim().to_ascii_lowercase();
        if let Some(amount) = text.strip_prefix('$') {
            let slice: f64 = amount
                .parse()
                .map_err(|_| format!("'{text}' is not a dollar amount — {USAGE}"))?;
            if !slice.is_finite() || slice <= 0.0 {
                return Err(format!("the envelope must be above $0 — {USAGE}"));
            }
            return Ok(Self {
                kind: EnvelopeKind::Usd,
                slice,
            });
        }
        let (provider, rest) = match text.split_once(':') {
            Some((provider, rest)) => (provider.to_string(), rest),
            None => match default_provider {
                Some(provider) => (provider.to_string(), text.as_str()),
                None => return Err(format!("'{text}' needs a provider — {USAGE}")),
            },
        };
        if provider != "claude" && provider != "codex" {
            return Err(format!("unknown provider '{provider}' — {USAGE}"));
        }
        let (amount, window) = rest
            .split_once('%')
            .ok_or_else(|| format!("'{text}' needs a percentage and window — {USAGE}"))?;
        let window =
            parse_window(window).ok_or_else(|| format!("unknown window '{window}' — {USAGE}"))?;
        let slice: f64 = amount
            .parse()
            .map_err(|_| format!("'{amount}' is not a percentage — {USAGE}"))?;
        if !slice.is_finite() || slice <= 0.0 || slice > 100.0 {
            return Err(format!("the slice must be between 0 and 100% — {USAGE}"));
        }
        Ok(Self {
            kind: EnvelopeKind::Quota {
                provider,
                window: window.to_string(),
            },
            slice,
        })
    }

    /// `claude weekly`, `codex 5h`, or `spend`.
    pub fn label(&self) -> String {
        match &self.kind {
            EnvelopeKind::Quota { provider, window } => {
                format!("{provider} {}", window_label(window))
            }
            EnvelopeKind::Usd => "spend".to_string(),
        }
    }
}

fn window_label(window: &str) -> &str {
    match window {
        "five_hour" => "5h",
        other => other,
    }
}

/// One utilisation reading of a subscription window.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QuotaReading {
    /// Percent of the window used, 0-100.
    pub pct: f64,
    pub observed_at: i64,
    pub resets_at: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvelopeCheck {
    Within,
    Spent,
    /// No fresh reading, so the slice cannot be verified.
    Unverified,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EnvelopeState {
    pub spec: EnvelopeSpec,
    /// Window percentage (or session dollars) when counting began. `None` after a resume into a
    /// fresh window, until the next reading.
    pub start: Option<f64>,
    /// Consumed before the latest window rollover.
    pub carried: f64,
    /// Consumed so far, in the spec's unit.
    pub used: f64,
    pub resets_at: Option<i64>,
    pub spent: bool,
    /// Set when the last check could not be verified.
    pub unverified: bool,
}

impl EnvelopeState {
    /// An envelope whose baseline is `start` (the window percentage or session cost right now).
    pub fn new(spec: EnvelopeSpec, start: f64, resets_at: Option<i64>) -> Self {
        Self {
            spec,
            start: Some(start),
            carried: 0.0,
            used: 0.0,
            resets_at,
            spent: false,
            unverified: false,
        }
    }

    pub fn is_usd(&self) -> bool {
        self.spec.kind == EnvelopeKind::Usd
    }

    fn settle(&mut self) -> EnvelopeCheck {
        self.unverified = false;
        self.spent = self.used >= self.spec.slice;
        if self.spent {
            EnvelopeCheck::Spent
        } else {
            EnvelopeCheck::Within
        }
    }

    /// Fold a quota reading in. `None` or an old one is [`EnvelopeCheck::Unverified`].
    pub fn observe_quota(&mut self, reading: Option<QuotaReading>, now: i64) -> EnvelopeCheck {
        let Some(reading) =
            reading.filter(|r| now - r.observed_at <= ENVELOPE_MAX_READING_AGE_SECS)
        else {
            self.unverified = true;
            return EnvelopeCheck::Unverified;
        };
        let reset_moved = matches!(
            (self.resets_at, reading.resets_at),
            (Some(before), Some(after)) if after > before + 600
        );
        match self.start {
            None => self.start = Some(reading.pct),
            Some(start) if reading.pct + 0.5 < start || reset_moved => {
                self.carried = self.used;
                self.start = Some(0.0);
            }
            Some(_) => {}
        }
        self.resets_at = reading.resets_at.or(self.resets_at);
        self.used = self.carried + (reading.pct - self.start.unwrap_or(reading.pct)).max(0.0);
        self.settle()
    }

    /// Fold the session's total cost in.
    pub fn observe_usd(&mut self, session_usd: f64) -> EnvelopeCheck {
        let start = *self.start.get_or_insert(session_usd);
        self.used = self.carried + (session_usd - start).max(0.0);
        self.settle()
    }

    /// Set a new envelope on the same goal. The same kind keeps its baseline and only moves the
    /// slice; a different kind starts over from the next reading.
    pub fn retarget(&mut self, spec: EnvelopeSpec) {
        if spec.kind == self.spec.kind {
            self.spec.slice = spec.slice;
            self.spent = self.used >= self.spec.slice;
        } else {
            *self = Self {
                spec,
                start: None,
                carried: 0.0,
                used: 0.0,
                resets_at: None,
                spent: false,
                unverified: false,
            };
        }
    }

    /// Once the window has reset, the next resume counts from a fresh baseline.
    pub fn rebaseline(&mut self) {
        self.start = None;
        self.carried = 0.0;
        self.used = 0.0;
        self.spent = false;
        self.unverified = false;
    }

    /// `envelope 12/20% claude weekly` or `envelope $1.20/$5.00`.
    pub fn meter(&self) -> String {
        match self.spec.kind {
            EnvelopeKind::Usd => {
                format!("envelope ${:.2}/${:.2}", self.used, self.spec.slice)
            }
            EnvelopeKind::Quota { .. } => format!(
                "envelope {}/{}% {}",
                pct(self.used),
                pct(self.spec.slice),
                self.spec.label()
            ),
        }
    }

    /// Fill fraction for a bar, 0.0-1.0.
    pub fn fill(&self) -> f64 {
        if self.spec.slice <= 0.0 {
            return 1.0;
        }
        (self.used / self.spec.slice).clamp(0.0, 1.0)
    }

    /// `20% of Claude weekly` or `$5.00`.
    pub fn slice_phrase(&self) -> String {
        match &self.spec.kind {
            EnvelopeKind::Usd => format!("${:.2}", self.spec.slice),
            EnvelopeKind::Quota { provider, window } => {
                let mut name = provider.clone();
                if let Some(first) = name.get_mut(0..1) {
                    first.make_ascii_uppercase();
                }
                let window = match window.as_str() {
                    "weekly" => "weekly".to_string(),
                    other => window_label(other).to_string(),
                };
                format!("{}% of {name} {window}", pct(self.spec.slice))
            }
        }
    }

    /// The raise hint matching the spec, e.g. `30%w` or `$10`.
    pub fn raise_hint(&self) -> String {
        match &self.spec.kind {
            EnvelopeKind::Usd => format!("${:.0}", (self.spec.slice * 1.5).ceil().max(1.0)),
            EnvelopeKind::Quota { window, .. } => {
                let suffix = if window == "five_hour" { "5h" } else { "w" };
                format!("{}%{suffix}", (self.spec.slice * 1.5).ceil().min(100.0))
            }
        }
    }
}

/// `12`, or `12.5` when the value has a visible fraction.
fn pct(value: f64) -> String {
    if (value - value.round()).abs() < 0.05 {
        format!("{:.0}", value.round())
    } else {
        format!("{value:.1}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn weekly(slice: f64) -> EnvelopeSpec {
        EnvelopeSpec::parse(&format!("claude:{slice}%w"), None).unwrap()
    }

    fn reading(pct: f64, observed_at: i64) -> Option<QuotaReading> {
        Some(QuotaReading {
            pct,
            observed_at,
            resets_at: Some(10_000_000),
        })
    }

    #[test]
    fn parses_every_documented_form() {
        assert_eq!(
            EnvelopeSpec::parse("claude:20%w", None).unwrap(),
            EnvelopeSpec {
                kind: EnvelopeKind::Quota {
                    provider: "claude".into(),
                    window: "weekly".into()
                },
                slice: 20.0
            }
        );
        let codex = EnvelopeSpec::parse("codex:15%w", None).unwrap();
        assert_eq!(codex.label(), "codex weekly");
        let short = EnvelopeSpec::parse("Claude:30%5h", None).unwrap();
        assert_eq!(short.label(), "claude 5h");
        assert_eq!(
            EnvelopeSpec::parse("$5", None).unwrap().kind,
            EnvelopeKind::Usd
        );
        assert_eq!(EnvelopeSpec::parse("$5.50", None).unwrap().slice, 5.5);
        assert_eq!(
            EnvelopeSpec::parse("30%w", Some("claude")).unwrap().label(),
            "claude weekly"
        );
    }

    #[test]
    fn rejects_nonsense_with_a_usage_hint() {
        for bad in [
            "",
            "20%w",
            "claude:0%w",
            "claude:150%w",
            "gemini:5%w",
            "claude:5%x",
            "$0",
            "$x",
        ] {
            let err = EnvelopeSpec::parse(bad, None).unwrap_err();
            assert!(err.contains("claude:20%w"), "{bad}: {err}");
        }
    }

    #[test]
    fn spends_when_usage_since_start_reaches_the_slice() {
        let mut env = EnvelopeState::new(weekly(20.0), 40.0, None);
        assert_eq!(
            env.observe_quota(reading(45.0, 100), 100),
            EnvelopeCheck::Within
        );
        assert_eq!(env.used, 5.0);
        assert_eq!(env.meter(), "envelope 5/20% claude weekly");
        assert_eq!(
            env.observe_quota(reading(59.0, 200), 200),
            EnvelopeCheck::Within
        );
        assert_eq!(
            env.observe_quota(reading(60.0, 300), 300),
            EnvelopeCheck::Spent
        );
        assert!(env.spent);
        assert_eq!(env.fill(), 1.0);
    }

    #[test]
    fn a_stale_or_missing_reading_cannot_verify() {
        let mut env = EnvelopeState::new(weekly(20.0), 40.0, None);
        assert_eq!(env.observe_quota(None, 100), EnvelopeCheck::Unverified);
        assert!(env.unverified);
        let old = reading(41.0, 0);
        assert_eq!(
            env.observe_quota(old, ENVELOPE_MAX_READING_AGE_SECS + 1),
            EnvelopeCheck::Unverified
        );
        assert_eq!(
            env.observe_quota(reading(41.0, 5000), 5000),
            EnvelopeCheck::Within
        );
        assert!(!env.unverified);
    }

    #[test]
    fn a_window_rollover_carries_what_was_used_before_it() {
        let mut env = EnvelopeState::new(weekly(20.0), 80.0, Some(10_000_000));
        assert_eq!(
            env.observe_quota(reading(90.0, 10), 10),
            EnvelopeCheck::Within
        );
        assert_eq!(env.used, 10.0);
        assert_eq!(
            env.observe_quota(reading(3.0, 20), 20),
            EnvelopeCheck::Within
        );
        assert_eq!(env.used, 13.0);
        assert_eq!(
            env.observe_quota(reading(10.0, 30), 30),
            EnvelopeCheck::Spent
        );
    }

    #[test]
    fn usd_counts_session_cost_since_start() {
        let spec = EnvelopeSpec::parse("$5", None).unwrap();
        let mut env = EnvelopeState::new(spec, 12.0, None);
        assert_eq!(env.observe_usd(15.0), EnvelopeCheck::Within);
        assert_eq!(env.meter(), "envelope $3.00/$5.00");
        assert_eq!(env.observe_usd(17.0), EnvelopeCheck::Spent);
    }

    #[test]
    fn raising_the_slice_keeps_the_baseline_and_a_new_kind_starts_over() {
        let mut env = EnvelopeState::new(weekly(20.0), 40.0, None);
        env.observe_quota(reading(60.0, 10), 10);
        assert!(env.spent);
        env.retarget(EnvelopeSpec::parse("30%w", Some("claude")).unwrap());
        assert!(!env.spent);
        assert_eq!(env.start, Some(40.0));
        assert_eq!(env.spec.slice, 30.0);
        env.retarget(EnvelopeSpec::parse("$5", None).unwrap());
        assert_eq!(env.start, None);
    }

    #[test]
    fn rebaseline_counts_from_the_next_reading() {
        let mut env = EnvelopeState::new(weekly(20.0), 40.0, None);
        env.observe_quota(reading(60.0, 10), 10);
        env.rebaseline();
        assert_eq!(
            env.observe_quota(reading(5.0, 20), 20),
            EnvelopeCheck::Within
        );
        assert_eq!(env.used, 0.0);
        assert_eq!(env.start, Some(5.0));
    }
}
