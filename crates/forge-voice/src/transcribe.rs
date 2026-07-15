//! Running whisper.cpp (via whisper-rs) against 16kHz mono f32 samples.

use std::path::Path;

use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

use crate::{Result, VoiceError};

/// A loaded whisper.cpp model, ready to transcribe. Cheap to clone-share (wraps a
/// `WhisperContext`, which is `Send + Sync`) — load once, reuse across many calls instead of
/// reloading the GGML file per request.
pub struct Transcriber {
    ctx: WhisperContext,
}

impl Transcriber {
    /// Load a GGML model from disk. This reads and parses the whole model file — expect this to
    /// take real wall-clock time (tens to hundreds of ms) and CPU; callers on an async runtime
    /// should run it via `spawn_blocking`.
    pub fn load(model_path: impl AsRef<Path>) -> Result<Self> {
        // whisper.cpp writes model-load and inference diagnostics directly to stderr unless its
        // logging hooks are installed. Forge owns the visible terminal, so those native logs must
        // never bypass the voice overlay and corrupt the chat surface.
        whisper_rs::install_logging_hooks();

        let path = model_path.as_ref();
        let path_str = path
            .to_str()
            .ok_or_else(|| VoiceError::Model(format!("non-UTF8 model path: {}", path.display())))?;
        let ctx = WhisperContext::new_with_params(path_str, WhisperContextParameters::default())
            .map_err(|e| VoiceError::Model(format!("loading {}: {e}", path.display())))?;
        Ok(Self { ctx })
    }

    /// Transcribe 16kHz mono f32 `samples`. `language` is a whisper.cpp language code (`"en"`,
    /// `"nl"`, ...) or `None` to auto-detect.
    ///
    /// **CPU-blocking**: this runs the whisper.cpp inference loop synchronously on the calling
    /// thread (typically hundreds of ms to several seconds, depending on model size and audio
    /// length). Callers on an async runtime MUST run it via `spawn_blocking` — calling it
    /// directly from an async task stalls the executor.
    pub fn transcribe(&self, samples: &[f32], language: Option<&str>) -> Result<String> {
        let mut state = self
            .ctx
            .create_state()
            .map_err(|e| VoiceError::Transcribe(format!("creating inference state: {e}")))?;

        // Greedy decoding: whisper.cpp's beam search is more accurate but multiple times slower,
        // which matters here since this is a live "record -> transcribe" UX, not a batch job.
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_language(language);
        params.set_translate(false);
        params.set_suppress_blank(true);
        // Drop non-speech tokens (coughs, background noise markers, etc.) whisper.cpp otherwise
        // emits inline with the transcript.
        params.set_suppress_nst(true);
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);

        state
            .full(params, samples)
            .map_err(|e| VoiceError::Transcribe(format!("running inference: {e}")))?;

        let n_segments = state.full_n_segments();
        let mut text = String::new();
        for i in 0..n_segments {
            let Some(segment) = state.get_segment(i) else {
                continue;
            };
            let Ok(segment_text) = segment.to_str_lossy() else {
                continue;
            };
            let trimmed = segment_text.trim();
            if trimmed.is_empty() {
                continue;
            }
            if !text.is_empty() {
                text.push(' ');
            }
            text.push_str(trimmed);
        }
        Ok(text)
    }
}
