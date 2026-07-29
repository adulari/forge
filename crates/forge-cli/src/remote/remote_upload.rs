//! Remote file upload and voice-transcription handlers.

use super::*;

// ---------------------------------------------------------------------------
// File/image upload (v7)
// ---------------------------------------------------------------------------

/// Hard cap on ONE uploaded file. Phone photos compress well under this; anything larger has no
/// business riding a chat prompt.
pub(crate) const UPLOAD_MAX_BYTES: usize = 10 * 1024 * 1024;

/// The request-body limit for the upload route: the file cap plus headroom for multipart
/// boundaries/headers and a couple of small siblings (e.g. a screenshot + a note file).
pub(crate) const UPLOAD_BODY_LIMIT: usize = UPLOAD_MAX_BYTES + 2 * 1024 * 1024;

/// Flatten an untrusted upload filename to a single safe path component: the final component
/// only (no traversal), characters outside `[A-Za-z0-9._-]` replaced with `_`, leading dots
/// stripped (no hidden files, no `..` remnants), length-capped, never empty.
pub(crate) fn sanitize_upload_name(name: &str) -> String {
    let last = name.rsplit(['/', '\\']).next().unwrap_or_default().trim();
    let mut clean: String = last
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .skip_while(|&c| c == '.')
        .take(80)
        .collect();
    if clean.is_empty() || clean.chars().all(|c| c == '_' || c == '.') {
        clean = "upload".to_string();
    }
    clean
}

/// Is this upload an image (→ vision input) by declared content type or file extension?
pub(crate) fn upload_is_image(content_type: Option<&str>, name: &str) -> bool {
    if content_type.is_some_and(|t| t.starts_with("image/")) {
        return true;
    }
    let ext = name.rsplit('.').next().unwrap_or_default().to_lowercase();
    matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "gif" | "webp")
}

/// Store one uploaded file under `dir` (created as needed): size-capped, name sanitized and
/// timestamp-prefixed (collision-free, ordered), and non-images required to be UTF-8 text —
/// only images and text files have an injection path into a prompt, so anything else is
/// refused at the door instead of parked on disk. Returns the stored path + whether it's an
/// image; errors are human-readable and map onto 4xx responses.
pub(crate) fn store_upload(
    dir: &std::path::Path,
    name: &str,
    content_type: Option<&str>,
    bytes: &[u8],
) -> Result<(std::path::PathBuf, bool), String> {
    if bytes.is_empty() {
        return Err("empty file".to_string());
    }
    if bytes.len() > UPLOAD_MAX_BYTES {
        return Err(format!(
            "file too large ({} bytes > {} max)",
            bytes.len(),
            UPLOAD_MAX_BYTES
        ));
    }
    let image = upload_is_image(content_type, name);
    if !image && std::str::from_utf8(bytes).is_err() {
        return Err("only images and UTF-8 text files can ride a prompt".to_string());
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("upload dir: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| format!("securing upload dir: {e}"))?;
    }
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let safe_name = sanitize_upload_name(name);
    for _ in 0..16 {
        let nonce = rand::random::<u64>();
        let path = dir.join(format!("{ts}-{nonce:016x}-{safe_name}"));
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        match options.open(&path) {
            Ok(mut file) => {
                use std::io::Write as _;
                file.write_all(bytes)
                    .map_err(|e| format!("writing upload: {e}"))?;
                return Ok((path, image));
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(format!("writing upload: {error}")),
        }
    }
    Err("could not allocate a unique upload path".to_string())
}

/// `POST /<token>/api/upload` — multipart file/image upload for the in-chat single-session
/// server (the daemon has its own session-addressed twin in `serve.rs`). Each stored file is
/// delivered to the render loop as [`RemoteInput::Attach`] and rides the next prompt.
pub(crate) fn upload_root(
    workspace: Option<&Arc<std::sync::RwLock<std::path::PathBuf>>>,
) -> Option<std::path::PathBuf> {
    workspace.and_then(|workspace| {
        workspace
            .read()
            .ok()
            .map(|workspace| workspace.join(".forge").join("uploads"))
    })
}

pub(super) async fn upload_handler(
    State(state): State<Arc<ServerState>>,
    mut multipart: axum::extract::Multipart,
) -> Response {
    let Some(root) = upload_root(state.upload_root.as_ref()) else {
        return upload_error(
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            "uploads are unavailable (no working directory)",
        );
    };
    let sid = state.snapshot_rx.borrow().snapshot.session_id.clone();
    if sid.is_empty() {
        return upload_error(
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            "session not ready yet — retry in a moment",
        );
    }
    let dir = root.join(sanitize_upload_name(&sid));
    let mut stored: Vec<serde_json::Value> = Vec::new();
    loop {
        let field = match multipart.next_field().await {
            Ok(Some(f)) => f,
            Ok(None) => break,
            Err(e) => {
                return upload_error(
                    axum::http::StatusCode::BAD_REQUEST,
                    &format!("malformed multipart body: {e}"),
                );
            }
        };
        let name = field.file_name().unwrap_or("upload").to_string();
        let content_type = field.content_type().map(str::to_string);
        let bytes = match field.bytes().await {
            Ok(b) => b,
            Err(e) => {
                // axum surfaces the body-limit overflow here.
                return upload_error(
                    axum::http::StatusCode::PAYLOAD_TOO_LARGE,
                    &format!("upload failed: {e}"),
                );
            }
        };
        match store_upload(&dir, &name, content_type.as_deref(), &bytes) {
            Ok((path, image)) => {
                let path_str = path.display().to_string();
                if state
                    .input_tx
                    .send(RemoteInput::Attach {
                        path: path_str.clone(),
                        image,
                    })
                    .await
                    .is_err()
                {
                    return upload_error(
                        axum::http::StatusCode::CONFLICT,
                        "remote control is shutting down",
                    );
                }
                stored.push(serde_json::json!({
                    "name": sanitize_upload_name(&name),
                    "path": path_str,
                    "image": image,
                }));
            }
            Err(msg) => {
                return upload_error(axum::http::StatusCode::UNPROCESSABLE_ENTITY, &msg);
            }
        }
    }
    if stored.is_empty() {
        return upload_error(axum::http::StatusCode::BAD_REQUEST, "no files in the body");
    }
    (
        [
            (axum::http::header::CONTENT_TYPE, "application/json"),
            (axum::http::header::CACHE_CONTROL, "no-store"),
        ],
        serde_json::json!({ "files": stored }).to_string(),
    )
        .into_response()
}

/// A JSON error body for the upload route (shape shared with the daemon's handlers).
pub(crate) fn upload_error(status: axum::http::StatusCode, msg: &str) -> Response {
    (
        status,
        [
            (axum::http::header::CONTENT_TYPE, "application/json"),
            (axum::http::header::CACHE_CONTROL, "no-store"),
        ],
        serde_json::json!({ "error": msg }).to_string(),
    )
        .into_response()
}

/// Query for `POST /<token>/api/voice/transcribe` — an optional language override for this clip.
#[derive(serde::Deserialize)]
pub(super) struct VoiceTranscribeParams {
    language: Option<String>,
}

/// `POST /<token>/api/voice/transcribe?language=<code>` — local whisper.cpp speech-to-text
/// (voice.md, V1): multipart audio in (first field with bytes, any name), `{"text": "..."}` out.
/// Mirrors the daemon's twin in `serve.rs`; not session-scoped (the model cache lives on
/// `ServerState`, this server only ever drives one session anyway).
pub(super) async fn voice_transcribe_handler(
    State(state): State<Arc<ServerState>>,
    axum::extract::Query(params): axum::extract::Query<VoiceTranscribeParams>,
    mut multipart: axum::extract::Multipart,
) -> Response {
    let field = match multipart.next_field().await {
        Ok(Some(f)) => f,
        Ok(None) => {
            return upload_error(axum::http::StatusCode::BAD_REQUEST, "no audio in the body")
        }
        Err(e) => {
            return upload_error(
                axum::http::StatusCode::BAD_REQUEST,
                &format!("malformed multipart body: {e}"),
            );
        }
    };
    let hint = field
        .file_name()
        .map(str::to_string)
        .or_else(|| field.content_type().map(str::to_string));
    let bytes = match field.bytes().await {
        Ok(b) => b.to_vec(),
        Err(e) => {
            return upload_error(
                axum::http::StatusCode::PAYLOAD_TOO_LARGE,
                &format!("upload failed: {e}"),
            );
        }
    };

    let models_dir = match crate::voice::models_dir() {
        Ok(d) => d,
        Err(e) => {
            return upload_error(
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                &format!("{e}"),
            )
        }
    };
    let config = forge_config::load().unwrap_or_default();
    match crate::voice::transcribe_upload(
        &state.voice,
        &config.voice,
        &models_dir,
        bytes,
        hint,
        params.language,
    )
    .await
    {
        Ok(text) => (
            [
                (axum::http::header::CONTENT_TYPE, "application/json"),
                (axum::http::header::CACHE_CONTROL, "no-store"),
            ],
            serde_json::json!({ "text": text }).to_string(),
        )
            .into_response(),
        Err(e) => upload_error(
            axum::http::StatusCode::UNPROCESSABLE_ENTITY,
            &format!("{e}"),
        ),
    }
}
