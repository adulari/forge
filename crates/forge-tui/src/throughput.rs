//! Live output-token throughput for the statusline.
//!
//! Providers report token usage only when a call COMPLETES, so a meter driven by usage events
//! alone sits frozen for the entire time the model is actually generating — the one moment a
//! throughput readout is worth having. This tracks the streamed character deltas the presenter
//! already emits (`AssistantDelta`, `Reasoning`) and converts them with a chars-per-token ratio
//! that is RE-CALIBRATED from every real usage report, so the live figure is anchored to
//! measured data rather than a hardcoded guess about the tokenizer.
//!
//! The rate is computed over a short sliding window rather than the whole turn: a turn spends
//! most of its wall time in tool calls, and averaging generation across those reads as a much
//! slower model than the one the user is watching.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Rate window. Long enough to survive the gap between two streamed chunks, short enough that
/// the number tracks what is happening now instead of the turn's history.
const WINDOW: Duration = Duration::from_millis(3_000);

/// Minimum spacing between retained samples, so a chatty stream cannot grow the deque unbounded.
const SAMPLE_MIN_GAP: Duration = Duration::from_millis(120);

/// Starting chars-per-token, used only until the first usage report calibrates it. ~4 chars per
/// token is the usual English-plus-code ballpark for current BPE tokenizers.
const DEFAULT_CHARS_PER_TOKEN: f64 = 4.0;

/// Weight given to a fresh calibration sample. Low enough that one short call with an odd ratio
/// cannot swing the meter, high enough to adapt within a few calls when the model changes.
const CALIBRATION_WEIGHT: f64 = 0.3;

/// Clamp for the calibrated ratio. A degenerate call (one token, hundreds of buffered characters,
/// or reasoning the provider bills differently) must not poison the estimate.
const CHARS_PER_TOKEN_MIN: f64 = 1.5;
const CHARS_PER_TOKEN_MAX: f64 = 12.0;

/// Sparkline history length — one bar per sampled rate, oldest first.
const HISTORY: usize = 8;

const SPARK: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

/// Rate bands for colouring. Chosen from what current hosted models actually deliver: below
/// ~15 tok/s a turn feels slow, above ~60 tok/s it feels fast.
pub const SLOW_TOK_PER_SEC: f64 = 15.0;
pub const FAST_TOK_PER_SEC: f64 = 60.0;

#[derive(Debug, Clone)]
pub struct ThroughputMeter {
    /// `(when, cumulative estimated output tokens)`, oldest first, pruned to [`WINDOW`].
    samples: VecDeque<(Instant, f64)>,
    /// Recent rates for the sparkline, oldest first.
    history: VecDeque<f64>,
    /// Streamed characters not yet accounted for by a usage report.
    pending_chars: f64,
    /// Exact cumulative output tokens for the turn, from the last usage report.
    confirmed_tokens: f64,
    chars_per_token: f64,
    last_sample: Option<Instant>,
    /// The turn's overall rate, kept once it ends so the statusline can show what just happened
    /// instead of blanking the moment generation stops.
    frozen: Option<f64>,
    turn_started: Option<Instant>,
}

impl Default for ThroughputMeter {
    fn default() -> Self {
        Self {
            samples: VecDeque::new(),
            history: VecDeque::new(),
            pending_chars: 0.0,
            confirmed_tokens: 0.0,
            chars_per_token: DEFAULT_CHARS_PER_TOKEN,
            last_sample: None,
            frozen: None,
            turn_started: None,
        }
    }
}

impl ThroughputMeter {
    /// Reset for a new turn. `chars_per_token` deliberately SURVIVES: it describes the model's
    /// tokenizer, not this turn, and carrying it over means the second turn onward starts with a
    /// calibrated estimate instead of the default guess.
    pub fn on_turn_start(&mut self, now: Instant) {
        self.samples.clear();
        self.history.clear();
        self.pending_chars = 0.0;
        self.confirmed_tokens = 0.0;
        self.last_sample = None;
        self.frozen = None;
        self.turn_started = Some(now);
    }

    /// Streamed output characters. Counted in `char`s, not bytes: a byte count inflates the rate
    /// on any non-ASCII output.
    pub fn on_delta(&mut self, chars: usize, now: Instant) {
        self.pending_chars += chars as f64;
        self.sample(now);
    }

    /// A completed call's cumulative turn output tokens, from the provider. Anchors the estimate
    /// to measured data and re-calibrates the chars-per-token ratio from the delta.
    pub fn on_usage(&mut self, turn_output_tokens: u64, now: Instant) {
        let reported = turn_output_tokens as f64;
        let generated = reported - self.confirmed_tokens;
        if generated > 0.0 && self.pending_chars > 0.0 {
            let observed =
                (self.pending_chars / generated).clamp(CHARS_PER_TOKEN_MIN, CHARS_PER_TOKEN_MAX);
            self.chars_per_token =
                self.chars_per_token * (1.0 - CALIBRATION_WEIGHT) + observed * CALIBRATION_WEIGHT;
        }
        // Only advance; a provider that reports cumulative usage per call must not move the
        // anchor backwards and mint negative throughput.
        self.confirmed_tokens = self.confirmed_tokens.max(reported);
        self.pending_chars = 0.0;
        self.sample(now);
    }

    /// Cumulative output tokens for the turn: measured, plus an estimate for what has streamed
    /// since the last report.
    fn tokens(&self) -> f64 {
        self.confirmed_tokens + self.pending_chars / self.chars_per_token
    }

    fn sample(&mut self, now: Instant) {
        if self
            .last_sample
            .is_some_and(|last| now.duration_since(last) < SAMPLE_MIN_GAP)
        {
            // Still fold the new characters into the newest sample, or a burst of small deltas
            // inside one gap would be invisible until the next one.
            if let Some(last) = self.samples.back_mut() {
                last.1 = self.confirmed_tokens + self.pending_chars / self.chars_per_token;
            }
            return;
        }
        self.last_sample = Some(now);
        self.samples.push_back((now, self.tokens()));
        self.prune(now);
        if let Some(rate) = self.rate(now) {
            if self.history.len() == HISTORY {
                self.history.pop_front();
            }
            self.history.push_back(rate);
        }
    }

    fn prune(&mut self, now: Instant) {
        // Keep one sample older than the window: it is the baseline the rate is measured from.
        while self.samples.len() > 2
            && self
                .samples
                .get(1)
                .is_some_and(|(t, _)| now.duration_since(*t) > WINDOW)
        {
            self.samples.pop_front();
        }
    }

    /// Tokens per second over the sliding window, or `None` until there is enough signal to say
    /// anything — never a guess from a single sample.
    pub fn rate(&self, now: Instant) -> Option<f64> {
        let (first_at, first_tokens) = *self.samples.front()?;
        let (last_at, last_tokens) = *self.samples.back()?;
        // Nothing has streamed for a whole window — the model is running a tool, not generating.
        // Holding the last rate on screen would claim a speed that is no longer happening.
        if now.duration_since(last_at) > WINDOW {
            return None;
        }
        let elapsed = last_at.duration_since(first_at).as_secs_f64();
        if elapsed < 0.25 {
            return None;
        }
        let produced = last_tokens - first_tokens;
        if produced <= 0.0 {
            return None;
        }
        Some(produced / elapsed)
    }

    /// Freeze the turn's average so the readout survives the end of generation. Returns nothing;
    /// [`Self::display`] picks the frozen value up once the session is no longer busy.
    pub fn on_turn_end(&mut self, now: Instant) {
        let Some(started) = self.turn_started else {
            return;
        };
        let elapsed = now.duration_since(started).as_secs_f64();
        let tokens = self.tokens();
        if elapsed >= 1.0 && tokens > 0.0 {
            self.frozen = Some(tokens / elapsed);
        }
    }

    /// The label and sparkline to render, or `None` when there is nothing honest to show.
    ///
    /// While `busy`, this is the live windowed rate. Afterwards it is the turn's average, which
    /// is a different (and lower) number by design — a turn is mostly tool calls — so it is
    /// rendered dimmed by the caller rather than presented as the live figure.
    pub fn display(&self, busy: bool, now: Instant) -> Option<Throughput> {
        if busy {
            let rate = self.rate(now)?;
            return Some(Throughput {
                tok_per_sec: rate,
                spark: self.spark(),
                live: true,
            });
        }
        Some(Throughput {
            tok_per_sec: self.frozen?,
            spark: self.spark(),
            live: false,
        })
    }

    /// Recent rates as block characters, scaled to the largest rate in view so the shape shows
    /// the trend regardless of the model's absolute speed. Empty until two samples exist.
    fn spark(&self) -> String {
        if self.history.len() < 2 {
            return String::new();
        }
        let peak = self.history.iter().copied().fold(0.0_f64, f64::max);
        if peak <= 0.0 {
            return String::new();
        }
        self.history
            .iter()
            .map(|rate| {
                let idx = ((rate / peak) * (SPARK.len() - 1) as f64).round() as usize;
                SPARK[idx.min(SPARK.len() - 1)]
            })
            .collect()
    }
}

/// What the statusline should draw.
#[derive(Debug, Clone, PartialEq)]
pub struct Throughput {
    pub tok_per_sec: f64,
    /// Sparkline of recent rates, or empty when there is not enough history to shape one.
    pub spark: String,
    /// `true` while the model is generating (live window), `false` for a finished turn's average.
    pub live: bool,
}

impl Throughput {
    /// `⚡ 64 tok/s ▂▄▇█`. Sub-10 rates keep one decimal, because the difference between 2 and 9
    /// tok/s is the difference between unusable and tolerable and rounding hides it.
    pub fn label(&self) -> String {
        let rate = if self.tok_per_sec < 10.0 {
            format!("{:.1}", self.tok_per_sec)
        } else {
            format!("{}", self.tok_per_sec.round() as u64)
        };
        if self.spark.is_empty() {
            format!("⚡ {rate} tok/s")
        } else {
            format!("⚡ {rate} tok/s {}", self.spark)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(base: Instant, ms: u64) -> Instant {
        base + Duration::from_millis(ms)
    }

    #[test]
    fn a_streaming_turn_reports_a_rate_before_any_usage_arrives() {
        // The whole point: usage lands only when a call completes, so a meter that waits for it
        // shows nothing during generation.
        let base = Instant::now();
        let mut m = ThroughputMeter::default();
        m.on_turn_start(base);
        // 400 chars over 1s at the default 4 chars/token ≈ 100 tokens ≈ 100 tok/s.
        for step in 1..=8 {
            m.on_delta(50, at(base, step * 125));
        }
        let rate = m
            .rate(at(base, 1_000))
            .expect("a streaming turn has a measurable rate");
        assert!(
            (80.0..=120.0).contains(&rate),
            "expected ~100 tok/s from 400 chars in 1s, got {rate}"
        );
    }

    #[test]
    fn usage_recalibrates_the_ratio_away_from_the_default_guess() {
        let base = Instant::now();
        let mut m = ThroughputMeter::default();
        m.on_turn_start(base);
        // 800 characters that the provider says were 100 tokens: 8 chars/token, double the
        // default. The estimate must move toward the measurement, not stay at 4.0.
        m.on_delta(800, at(base, 100));
        m.on_usage(100, at(base, 200));
        assert!(
            m.chars_per_token > DEFAULT_CHARS_PER_TOKEN,
            "ratio must move toward the observed 8 chars/token, got {}",
            m.chars_per_token
        );
        assert!(
            m.chars_per_token <= CHARS_PER_TOKEN_MAX,
            "ratio stays clamped, got {}",
            m.chars_per_token
        );
    }

    #[test]
    fn a_pathological_ratio_cannot_poison_the_estimate() {
        let base = Instant::now();
        let mut m = ThroughputMeter::default();
        m.on_turn_start(base);
        // One token reported against 10k buffered characters — a real shape when a provider
        // batches function-call arguments into a single-token completion.
        m.on_delta(10_000, at(base, 100));
        m.on_usage(1, at(base, 200));
        assert!(
            m.chars_per_token <= CHARS_PER_TOKEN_MAX,
            "clamped, got {}",
            m.chars_per_token
        );
    }

    #[test]
    fn cumulative_usage_never_moves_the_anchor_backwards() {
        let base = Instant::now();
        let mut m = ThroughputMeter::default();
        m.on_turn_start(base);
        m.on_usage(500, at(base, 100));
        m.on_usage(300, at(base, 200));
        assert_eq!(
            m.confirmed_tokens, 500.0,
            "a lower report must not mint negative throughput"
        );
    }

    #[test]
    fn a_stale_stream_stops_claiming_a_rate() {
        // Generation stopped and a long tool call is running. Leaving the last rate on screen
        // would assert a speed that is not happening.
        let base = Instant::now();
        let mut m = ThroughputMeter::default();
        m.on_turn_start(base);
        for step in 1..=6 {
            m.on_delta(60, at(base, step * 150));
        }
        assert!(m.rate(at(base, 900)).is_some(), "live while streaming");
        assert_eq!(
            m.rate(at(base, 9_000)),
            None,
            "a window with no new characters is not a rate"
        );
    }

    #[test]
    fn an_idle_meter_reports_nothing_rather_than_zero() {
        let base = Instant::now();
        let m = ThroughputMeter::default();
        assert_eq!(m.rate(at(base, 5_000)), None);
        assert_eq!(m.display(true, at(base, 5_000)), None);
        assert_eq!(m.display(false, at(base, 5_000)), None);
    }

    #[test]
    fn a_finished_turn_shows_its_average_marked_not_live() {
        let base = Instant::now();
        let mut m = ThroughputMeter::default();
        m.on_turn_start(base);
        m.on_delta(400, at(base, 500));
        m.on_usage(100, at(base, 1_000));
        m.on_turn_end(at(base, 2_000));
        let shown = m
            .display(false, at(base, 2_000))
            .expect("a finished turn keeps its average");
        assert!(!shown.live);
        assert!(
            (40.0..=60.0).contains(&shown.tok_per_sec),
            "100 tokens over 2s is ~50 tok/s, got {}",
            shown.tok_per_sec
        );
    }

    #[test]
    fn the_label_keeps_a_decimal_only_where_it_changes_the_meaning() {
        let fast = Throughput {
            tok_per_sec: 63.7,
            spark: "▂▄█".into(),
            live: true,
        };
        assert_eq!(fast.label(), "⚡ 64 tok/s ▂▄█");
        let slow = Throughput {
            tok_per_sec: 2.4,
            spark: String::new(),
            live: true,
        };
        assert_eq!(slow.label(), "⚡ 2.4 tok/s");
    }

    #[test]
    fn the_sparkline_scales_to_the_rates_in_view() {
        let base = Instant::now();
        let mut m = ThroughputMeter::default();
        m.on_turn_start(base);
        // Accelerating stream: each sample delivers more than the last, so the bars must rise.
        for step in 1..=6u64 {
            m.on_delta((step * 40) as usize, at(base, step * 200));
        }
        let spark = m.spark();
        assert!(!spark.is_empty(), "history should have shaped a sparkline");
        let chars: Vec<char> = spark.chars().collect();
        let first = SPARK.iter().position(|c| *c == chars[0]).expect("bar char");
        let last = SPARK
            .iter()
            .position(|c| *c == chars[chars.len() - 1])
            .expect("bar char");
        assert!(
            last >= first,
            "an accelerating stream must not render as a falling sparkline: {spark}"
        );
    }
}
