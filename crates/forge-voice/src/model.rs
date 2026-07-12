//! Which whisper.cpp GGML model to run, and fetching it from Hugging Face on first use.

use std::path::{Path, PathBuf};
use std::str::FromStr;

use futures::StreamExt;
use serde::{Deserialize, Serialize};

use crate::{Result, VoiceError};

/// A whisper.cpp GGML model size. Bigger = more accurate, slower, larger download.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelKind {
    Tiny,
    Base,
    Small,
    Medium,
}

impl ModelKind {
    /// Lowercase name, as used in the ggml filename and config (`voice.model = "base"`).
    pub fn as_str(&self) -> &'static str {
        match self {
            ModelKind::Tiny => "tiny",
            ModelKind::Base => "base",
            ModelKind::Small => "small",
            ModelKind::Medium => "medium",
        }
    }

    /// The GGML file name this model is stored/downloaded as, e.g. `ggml-base.bin`.
    pub fn file_name(&self) -> String {
        format!("ggml-{}.bin", self.as_str())
    }

    /// The upstream download URL (ggerganov's whisper.cpp model mirror on Hugging Face).
    pub fn url(&self) -> String {
        format!(
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/{}",
            self.file_name()
        )
    }
}

impl FromStr for ModelKind {
    type Err = VoiceError;

    fn from_str(s: &str) -> Result<Self> {
        match s.to_ascii_lowercase().as_str() {
            "tiny" => Ok(ModelKind::Tiny),
            "base" => Ok(ModelKind::Base),
            "small" => Ok(ModelKind::Small),
            "medium" => Ok(ModelKind::Medium),
            other => Err(VoiceError::UnknownModelKind(other.to_string())),
        }
    }
}

impl std::fmt::Display for ModelKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A reqwest `ClientBuilder` pre-seeded with Mozilla's bundled root CAs, so the model download
/// works on a host with no OS trust store. Mirrors `forge_tools::web`'s builder (this crate can't
/// depend on forge-tools).
fn bundled_client_builder() -> reqwest::ClientBuilder {
    let certs = webpki_root_certs::TLS_SERVER_ROOT_CERTS
        .iter()
        .filter_map(|der| reqwest::Certificate::from_der(der.as_ref()).ok());
    reqwest::Client::builder().tls_certs_only(certs)
}

/// Ensure `kind`'s GGML model file exists under `dir`, downloading it if missing. Returns the
/// final model path. `progress(bytes_done, total_bytes)` is called as chunks arrive during a
/// download (`total_bytes` is `None` if the server didn't send `Content-Length`); it is NOT
/// called when the file already exists (nothing to report).
///
/// The download streams to `<file>.part` and only renames to the final name once complete, so a
/// crash or Ctrl-C mid-download never leaves a corrupt file that looks "present" on the next run.
pub async fn ensure_model(
    kind: ModelKind,
    dir: &Path,
    mut progress: impl FnMut(u64, Option<u64>),
) -> Result<PathBuf> {
    let final_path = dir.join(kind.file_name());
    if tokio::fs::try_exists(&final_path).await.unwrap_or(false) {
        return Ok(final_path);
    }
    tokio::fs::create_dir_all(dir).await?;

    let client = bundled_client_builder()
        .build()
        .map_err(|e| VoiceError::Download(format!("building HTTP client: {e}")))?;
    let url = kind.url();
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| VoiceError::Download(format!("requesting {url}: {e}")))?;
    if !resp.status().is_success() {
        return Err(VoiceError::Download(format!(
            "{url} returned HTTP {}",
            resp.status()
        )));
    }
    let total = resp.content_length();

    let part_path = dir.join(format!("{}.part", kind.file_name()));
    let mut file = tokio::fs::File::create(&part_path).await?;
    let mut done: u64 = 0;
    let mut stream = resp.bytes_stream();
    use tokio::io::AsyncWriteExt;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| VoiceError::Download(format!("reading {url}: {e}")))?;
        file.write_all(&chunk).await?;
        done += chunk.len() as u64;
        progress(done, total);
    }
    file.flush().await?;
    drop(file);

    tokio::fs::rename(&part_path, &final_path).await?;
    Ok(final_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_kind_from_str_is_case_insensitive() {
        assert_eq!(ModelKind::from_str("tiny").unwrap(), ModelKind::Tiny);
        assert_eq!(ModelKind::from_str("Base").unwrap(), ModelKind::Base);
        assert_eq!(ModelKind::from_str("SMALL").unwrap(), ModelKind::Small);
        assert_eq!(ModelKind::from_str("medium").unwrap(), ModelKind::Medium);
        assert!(ModelKind::from_str("large").is_err());
    }

    #[test]
    fn model_kind_file_name_and_url() {
        assert_eq!(ModelKind::Tiny.file_name(), "ggml-tiny.bin");
        assert_eq!(
            ModelKind::Base.url(),
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin"
        );
        assert_eq!(ModelKind::Medium.file_name(), "ggml-medium.bin");
    }

    #[test]
    fn model_kind_serde_round_trips_lowercase() {
        let json = serde_json::to_string(&ModelKind::Small).unwrap();
        assert_eq!(json, "\"small\"");
        let back: ModelKind = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ModelKind::Small);
    }

    #[tokio::test]
    async fn ensure_model_skips_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let existing = dir.path().join(ModelKind::Tiny.file_name());
        tokio::fs::write(&existing, b"not a real model, just a stand-in")
            .await
            .unwrap();

        let mut calls = 0;
        let path = ensure_model(ModelKind::Tiny, dir.path(), |_, _| calls += 1)
            .await
            .unwrap();
        assert_eq!(path, existing);
        assert_eq!(calls, 0, "no download attempted for an existing file");
    }
}
