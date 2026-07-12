//! Shared `POST /api/voice/transcribe` implementation for `serve.rs` (multi-session daemon) and
//! `remote.rs` (in-chat single-session server) — decode the uploaded audio, ensure the configured
//! whisper model is present, and transcribe. See docs: voice.md (V1: no TUI/mobile UI yet).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use forge_voice::{ModelKind, Transcriber};
use tokio::sync::Mutex;

/// Hard cap on one uploaded audio clip. A few minutes of 16-bit mono WAV comfortably fits; this
/// mirrors the intent of `remote::UPLOAD_BODY_LIMIT` without sharing its (image/text-upload
/// specific) constant.
pub(crate) const VOICE_UPLOAD_BODY_LIMIT: usize = 32 * 1024 * 1024;

/// Caches the last-loaded whisper model so repeated transcribe calls don't reload the GGML file
/// (hundreds of ms to seconds) on every request. `Transcriber` wraps whisper-rs's
/// `WhisperContext`, which is `Send + Sync`, so sharing it behind an `Arc` across requests is
/// sound; the `Mutex` only guards the "is this the right model" check + swap, not inference itself
/// (each request creates its own whisper inference state).
///
/// The currently loaded model: its path (the cache key) + the loaded `Transcriber`.
type LoadedModel = (PathBuf, Arc<Transcriber>);

/// `Clone` is shallow (an `Arc` bump) so the cache stays shared when `ServerState` (remote.rs)
/// is cloned — `DaemonState` (serve.rs) is never cloned, only shared via `Arc<DaemonState>`.
#[derive(Clone, Default)]
pub(crate) struct VoiceState {
    loaded: Arc<Mutex<Option<LoadedModel>>>,
}

impl VoiceState {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    async fn transcriber_for(&self, model_path: PathBuf) -> Result<Arc<Transcriber>> {
        let mut guard = self.loaded.lock().await;
        if let Some((path, transcriber)) = guard.as_ref() {
            if *path == model_path {
                return Ok(transcriber.clone());
            }
        }
        let load_path = model_path.clone();
        let transcriber = tokio::task::spawn_blocking(move || Transcriber::load(&load_path))
            .await
            .context("model-load task panicked")?
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        let transcriber = Arc::new(transcriber);
        *guard = Some((model_path, transcriber.clone()));
        Ok(transcriber)
    }
}

/// Decode + transcribe one uploaded audio clip: parse `config.model`, download it if needed
/// (into `{data_dir}/models/whisper/`), then decode and run inference off the async runtime
/// (`spawn_blocking` — both are CPU-bound).
///
/// `language` overrides `config.language` when given (a query param beats the configured
/// default); `"auto"` (the config default) means auto-detect.
pub(crate) async fn transcribe_upload(
    state: &VoiceState,
    config: &forge_config::VoiceConfig,
    models_dir: &Path,
    bytes: Vec<u8>,
    hint: Option<String>,
    language: Option<String>,
) -> Result<String> {
    let kind: ModelKind = config
        .model
        .parse()
        .map_err(|e: forge_voice::VoiceError| anyhow::anyhow!("{e}"))?;
    let model_path = forge_voice::ensure_model(kind, models_dir, |_, _| {})
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let transcriber = state.transcriber_for(model_path).await?;

    let language =
        language.or_else(|| (config.language != "auto").then(|| config.language.clone()));

    let samples =
        tokio::task::spawn_blocking(move || forge_voice::decode_audio(&bytes, hint.as_deref()))
            .await
            .context("decode task panicked")?
            .map_err(|e| anyhow::anyhow!("{e}"))?;
    let text =
        tokio::task::spawn_blocking(move || transcriber.transcribe(&samples, language.as_deref()))
            .await
            .context("transcribe task panicked")?
            .map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(text)
}

/// `{data_dir}/models/whisper` — where downloaded GGML models live (voice.md).
pub(crate) fn models_dir() -> Result<PathBuf> {
    let data_dir = forge_config::data_dir().context("no data directory on this platform")?;
    Ok(data_dir.join("models").join("whisper"))
}
