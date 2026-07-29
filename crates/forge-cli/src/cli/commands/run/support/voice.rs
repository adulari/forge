use super::*;

pub(crate) fn voice_is_hold(held_ms: u128) -> bool {
    held_ms >= VOICE_PTT_HOLD_MS
}

/// Wire a freshly-dispatched `/voice` into the loop-local recorder/download state. `App::voice`
/// (the rendering-facing state) is already set by `dispatch_command` — this only handles the real
/// system resources, which must live loop-local so `App` stays `Clone + Default`. Shared by every
/// place `/voice` can be triggered from: the palette, a typed `/voice` line, remote input, and the
/// Ctrl+V shortcut. Resets every voice-related loop-local first, so no state from a previous voice
/// session can bleed through.
#[allow(clippy::too_many_arguments)]
pub(crate) fn apply_voice_start(
    start: VoiceStart,
    tui: &mut forge_tui::Tui,
    app: &mut forge_tui::App,
    voice_handle: &mut Option<forge_voice::RecordingHandle>,
    voice_model_path: &mut Option<std::path::PathBuf>,
    voice_download_progress_rx: &mut Option<tokio::sync::watch::Receiver<(u64, Option<u64>)>>,
    voice_download_done_rx: &mut Option<
        tokio::sync::oneshot::Receiver<std::result::Result<forge_voice::RecordingHandle, String>>,
    >,
    voice_started_at: &mut Option<std::time::Instant>,
    voice_error_until: &mut Option<std::time::Instant>,
    voice_ptt_active: &mut bool,
) {
    *voice_handle = None;
    *voice_model_path = None;
    *voice_download_progress_rx = None;
    *voice_download_done_rx = None;
    *voice_started_at = None;
    *voice_error_until = None;
    if *voice_ptt_active {
        tui.pop_voice_ptt();
        *voice_ptt_active = false;
    }
    match start {
        VoiceStart::Recording { handle, model_path } => {
            *voice_ptt_active = tui.push_voice_ptt();
            if let Some(v) = app.voice.as_mut() {
                v.phase = forge_tui::VoicePhase::Recording {
                    ptt_active: *voice_ptt_active,
                };
            }
            *voice_handle = Some(handle);
            *voice_model_path = Some(model_path);
            *voice_started_at = Some(std::time::Instant::now());
        }
        VoiceStart::Downloading {
            model_path,
            progress_rx,
            done_rx,
        } => {
            *voice_model_path = Some(model_path);
            *voice_download_progress_rx = Some(progress_rx);
            *voice_download_done_rx = Some(done_rx);
        }
        VoiceStart::Error => {
            // `app.voice` is already the error card (set by `dispatch_command`).
            *voice_error_until =
                Some(std::time::Instant::now() + std::time::Duration::from_secs(2));
        }
    }
}

/// Stop the recording and kick off transcription in the background — shared by the Enter key
/// (toggle mode) and a held-then-released push-to-talk chord. Moves the overlay into
/// `VoicePhase::Transcribing`; the tick loop polls the returned receiver for the result.
pub(crate) fn start_voice_transcribe(
    app: &mut forge_tui::App,
    handle: forge_voice::RecordingHandle,
    model_path: std::path::PathBuf,
) -> tokio::sync::oneshot::Receiver<forge_voice::Result<String>> {
    if let Some(v) = app.voice.as_mut() {
        v.phase = forge_tui::VoicePhase::Transcribing;
    }
    let config = forge_config::load().unwrap_or_default();
    let language = (config.voice.language != "auto").then_some(config.voice.language);
    let (tx, rx) = tokio::sync::oneshot::channel();
    tokio::task::spawn_blocking(move || {
        let result = (|| -> forge_voice::Result<String> {
            let samples = handle.stop()?;
            let transcriber = forge_voice::Transcriber::load(&model_path)?;
            transcriber.transcribe(&samples, language.as_deref())
        })();
        let _ = tx.send(result);
    });
    rx
}
