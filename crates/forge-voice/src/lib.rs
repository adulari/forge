//! Local whisper.cpp speech-to-text core (docs: `/tmp/.../voice/DESIGN.md` — V1).
//!
//! No cloud: audio never leaves the machine. Four seams, kept independent so callers only pull
//! in what they need:
//! - [`model`]: which whisper.cpp GGML model to use, and fetching it from Hugging Face.
//! - [`transcribe`]: running whisper-rs against 16kHz mono f32 samples (CPU-blocking).
//! - [`record`]: optional cpal microphone capture -> 16kHz mono f32 (`microphone` feature).
//! - [`decode`]: turning an uploaded file's bytes (wav/m4a/aac/mp4) into 16kHz mono f32.

pub mod decode;
pub mod model;
pub mod record;
pub mod transcribe;

pub use decode::decode_audio;
pub use model::{ensure_model, ModelKind};
pub use record::{Recorder, RecordingHandle};
pub use transcribe::Transcriber;

/// Errors from any part of the voice pipeline.
#[derive(Debug, thiserror::Error)]
pub enum VoiceError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("model download failed: {0}")]
    Download(String),
    #[error("failed to load whisper model: {0}")]
    Model(String),
    #[error("transcription failed: {0}")]
    Transcribe(String),
    #[error("audio decode failed: {0}")]
    Decode(String),
    #[error("microphone recording failed: {0}")]
    Record(String),
    #[error(
        "microphone capture is unavailable in this build; on Linux, install the ALSA development libraries and rebuild Forge with `--features microphone`"
    )]
    MicrophoneUnavailable,
    #[error("no input (microphone) device available")]
    NoInputDevice,
    #[error("unrecognized model kind '{0}' (expected tiny, base, small, or medium)")]
    UnknownModelKind(String),
}

/// This crate's `Result` alias.
pub type Result<T> = std::result::Result<T, VoiceError>;
