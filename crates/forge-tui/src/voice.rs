//! Voice overlay state and transcript insertion policy.
//!
//! This private module owns pure `/voice` presentation state; terminal devices
//! and transcription tasks remain in the driver.

/// Ring-buffer width for the `/voice` overlay's live waveform (`▁▂▃▄▅▆▇█` bars) — see voice.md.
pub(crate) const VOICE_WAVEFORM_BARS: usize = 40;

/// Which phase the `/voice` overlay (`/voice` or Ctrl+V) is in. Pure rendering data — the real
/// system resources (recorder thread, whisper model, download task) live in the render loop
/// (`run.rs`), which drives this via `App::voice`.
#[derive(Debug, Clone, PartialEq)]
pub enum VoicePhase {
    /// First use: the whisper model isn't on disk yet, fetching it from Hugging Face.
    Downloading {
        model: String,
        done_mb: f64,
        total_mb: Option<f64>,
    },
    /// Capturing from the microphone. `ptt_active` is true when the terminal supports the kitty
    /// keyboard protocol's release reporting, so holding the chord (instead of tapping it) will
    /// auto-stop and transcribe on release — the footer hint differs accordingly.
    Recording { ptt_active: bool },
    /// Enter (or a push-to-talk release) was pressed: waveform freezes, whisper runs off-thread.
    Transcribing,
    /// A download/mic/whisper failure, shown in the card; the render loop auto-closes it after a
    /// couple of seconds or on the next keypress, whichever comes first.
    Error(String),
}

/// State for the `/voice` overlay: live waveform, elapsed timer, model download progress.
#[derive(Debug, Clone, PartialEq)]
pub struct VoiceOverlay {
    pub phase: VoicePhase,
    /// Wall-clock seconds since recording started; updated by the render loop each tick, the same
    /// way `App::turn_elapsed_secs` is.
    pub elapsed_secs: u64,
    /// Ring buffer of recent RMS levels (0..1), oldest first — scrolls left as new samples arrive.
    /// Bounded to [`VOICE_WAVEFORM_BARS`].
    pub waveform: std::collections::VecDeque<f32>,
    /// Animation tick, advanced every render while open (REC blink / spinner cadence).
    pub anim_tick: u32,
}

impl VoiceOverlay {
    /// First use: the whisper model needs fetching before recording can start.
    pub fn downloading(model: impl Into<String>) -> Self {
        Self {
            phase: VoicePhase::Downloading {
                model: model.into(),
                done_mb: 0.0,
                total_mb: None,
            },
            elapsed_secs: 0,
            waveform: std::collections::VecDeque::with_capacity(VOICE_WAVEFORM_BARS),
            anim_tick: 0,
        }
    }

    /// The model is on disk and the microphone is open.
    pub fn recording(ptt_active: bool) -> Self {
        Self {
            phase: VoicePhase::Recording { ptt_active },
            elapsed_secs: 0,
            waveform: std::collections::VecDeque::with_capacity(VOICE_WAVEFORM_BARS),
            anim_tick: 0,
        }
    }

    /// A download/mic/whisper failure, shown in the card until dismissed/timed out.
    pub fn error(msg: impl Into<String>) -> Self {
        Self {
            phase: VoicePhase::Error(msg.into()),
            elapsed_secs: 0,
            waveform: std::collections::VecDeque::new(),
            anim_tick: 0,
        }
    }

    /// Push a new RMS level sample, scrolling the waveform ring buffer left once it's full.
    pub fn push_level(&mut self, level: f32) {
        if self.waveform.len() >= VOICE_WAVEFORM_BARS {
            self.waveform.pop_front();
        }
        self.waveform.push_back(level.clamp(0.0, 1.0));
    }

    /// `mm:ss` elapsed-time label.
    pub fn mmss(&self) -> String {
        format!(
            "{:02}:{:02}",
            self.elapsed_secs / 60,
            self.elapsed_secs % 60
        )
    }
}

/// Insert a `/voice` transcript into `input` at `cursor`, padding with a single joining space on
/// whichever side would otherwise run the transcript into existing non-whitespace text (so
/// `foo|bar` + "hello" → `foo hello |bar`, but empty input + "hello" → `hello` with no stray
/// space). Mirrors `handle_key`'s char-insertion semantics — cursor lands right after the
/// inserted text. Returns the number of transcript characters inserted (excluding padding), for
/// the "inserted N chars" status note. A blank/whitespace-only transcript inserts nothing.
pub fn insert_voice_transcript(input: &mut String, cursor: &mut usize, text: &str) -> usize {
    let text = text.trim();
    if text.is_empty() {
        return 0;
    }
    let at = (*cursor).min(input.len());
    let needs_lead_space = at > 0 && !input[..at].ends_with(char::is_whitespace);
    let needs_trail_space = at < input.len() && !input[at..].starts_with(char::is_whitespace);
    let mut insert = String::new();
    if needs_lead_space {
        insert.push(' ');
    }
    insert.push_str(text);
    if needs_trail_space {
        insert.push(' ');
    }
    input.insert_str(at, &insert);
    *cursor = at + insert.len();
    text.chars().count()
}
