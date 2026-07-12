use anyhow::{Context, Result};

use crate::*;

/// `forge voice <op>` — local whisper.cpp speech-to-text, no mic/server needed to test it.
pub(crate) async fn voice_cmd(op: VoiceOp) -> Result<()> {
    match op {
        VoiceOp::Transcribe {
            file,
            language,
            model,
        } => transcribe_cmd(file, language, model).await,
        VoiceOp::Setup { model } => setup_cmd(model).await,
    }
}

async fn transcribe_cmd(
    file: String,
    language: Option<String>,
    model: Option<String>,
) -> Result<()> {
    let config = forge_config::load().unwrap_or_default();
    let kind = resolve_model_kind(model, &config)?;
    let models_dir = crate::voice::models_dir()?;

    let bytes = tokio::fs::read(&file)
        .await
        .with_context(|| format!("reading {file}"))?;
    let hint = Some(file.clone());
    let samples =
        tokio::task::spawn_blocking(move || forge_voice::decode_audio(&bytes, hint.as_deref()))
            .await
            .context("decode task panicked")?
            .map_err(|e| anyhow::anyhow!("{e}"))?;

    let model_path = forge_voice::ensure_model(kind, &models_dir, download_progress)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    eprintln!();

    let language =
        language.or_else(|| (config.voice.language != "auto").then_some(config.voice.language));
    let text = tokio::task::spawn_blocking(move || -> anyhow::Result<String> {
        let transcriber =
            forge_voice::Transcriber::load(&model_path).map_err(|e| anyhow::anyhow!("{e}"))?;
        transcriber
            .transcribe(&samples, language.as_deref())
            .map_err(|e| anyhow::anyhow!("{e}"))
    })
    .await
    .context("transcribe task panicked")??;

    println!("{text}");
    Ok(())
}

async fn setup_cmd(model: Option<String>) -> Result<()> {
    let config = forge_config::load().unwrap_or_default();
    let kind = resolve_model_kind(model, &config)?;
    let models_dir = crate::voice::models_dir()?;
    let path = forge_voice::ensure_model(kind, &models_dir, download_progress)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    eprintln!();
    println!("⌬ voice model ready — {}", path.display());
    Ok(())
}

fn resolve_model_kind(
    model: Option<String>,
    config: &forge_config::Config,
) -> Result<forge_voice::ModelKind> {
    let name = model.unwrap_or_else(|| config.voice.model.clone());
    name.parse::<forge_voice::ModelKind>()
        .map_err(|e| anyhow::anyhow!("{e}"))
}

/// Print a single self-overwriting progress line to stderr as a model downloads (stdout stays
/// clean for the transcript).
fn download_progress(done: u64, total: Option<u64>) {
    let done_mb = done as f64 / 1_048_576.0;
    match total {
        Some(total) if total > 0 => {
            let pct = (done as f64 / total as f64 * 100.0).min(100.0);
            let total_mb = total as f64 / 1_048_576.0;
            eprint!("\r⌬ fetching whisper model · {done_mb:.1}/{total_mb:.1} MB ({pct:.0}%)");
        }
        _ => eprint!("\r⌬ fetching whisper model · {done_mb:.1} MB"),
    }
}
