//! A PDF from the session's workspace, so the file viewer can show it in a frame.
//!
//! - `GET /api/workspace/pdf?session=<id>&path=<p>`  one `.pdf` inside the session's workspace
//!
//! The root comes from the session, the path is canonicalized and must stay inside it, and the
//! bytes must start with the `%PDF-` magic: the extension alone never decides what is served.

use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;

use crate::serve::{err_response, DaemonState};
use crate::serve_git::session_dir;

const MAX_PDF_BYTES: u64 = 25 * 1024 * 1024;

pub(crate) fn routes(base: &str) -> Router<Arc<DaemonState>> {
    Router::new().route(&format!("{base}/api/workspace/pdf"), get(workspace_pdf))
}

#[derive(serde::Deserialize)]
struct PdfQuery {
    #[serde(default)]
    session: String,
    #[serde(default)]
    path: String,
}

fn resolve_pdf(workspace: &Path, raw: &str) -> Result<PathBuf, (StatusCode, &'static str)> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "path is required"));
    }
    let requested = Path::new(raw);
    if requested.components().any(|part| {
        matches!(part, Component::ParentDir)
            || matches!(part, Component::Normal(name) if name == ".git")
    }) {
        return Err((StatusCode::FORBIDDEN, "path must stay inside the workspace"));
    }
    let candidate = if requested.is_absolute() {
        requested.to_path_buf()
    } else {
        workspace.join(requested)
    };
    let resolved =
        fs::canonicalize(&candidate).map_err(|_| (StatusCode::NOT_FOUND, "pdf not found"))?;
    let root = fs::canonicalize(workspace).map_err(|_| {
        (
            StatusCode::NOT_FOUND,
            "session has no working directory on this host",
        )
    })?;
    if !resolved.starts_with(root) {
        return Err((StatusCode::FORBIDDEN, "path is outside this workspace"));
    }
    Ok(resolved)
}

fn read_pdf(path: &Path) -> Result<Vec<u8>, (StatusCode, &'static str)> {
    let file = fs::File::open(path).map_err(|_| (StatusCode::NOT_FOUND, "pdf not found"))?;
    let size = file.metadata().map(|meta| meta.len()).unwrap_or(0);
    if size > MAX_PDF_BYTES {
        return Err((StatusCode::PAYLOAD_TOO_LARGE, "pdf is too large to preview"));
    }
    let mut bytes = Vec::with_capacity(size as usize);
    file.take(MAX_PDF_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "could not read the pdf"))?;
    if !bytes.starts_with(b"%PDF-") {
        return Err((StatusCode::UNSUPPORTED_MEDIA_TYPE, "file is not a pdf"));
    }
    Ok(bytes)
}

async fn workspace_pdf(
    State(state): State<Arc<DaemonState>>,
    Query(query): Query<PdfQuery>,
) -> Response {
    if query.session.trim().is_empty() {
        return err_response(StatusCode::BAD_REQUEST, "session is required");
    }
    let Some(dir) = session_dir(&state, &query.session).await else {
        return err_response(StatusCode::NOT_FOUND, "no such session");
    };
    let outcome = tokio::task::spawn_blocking(move || {
        resolve_pdf(Path::new(&dir), &query.path).and_then(|path| read_pdf(&path))
    })
    .await;
    match outcome {
        Ok(Ok(bytes)) => (
            [
                (header::CONTENT_TYPE, "application/pdf"),
                (header::CACHE_CONTROL, "private, max-age=30"),
                (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            ],
            bytes,
        )
            .into_response(),
        Ok(Err((status, message))) => err_response(status, message),
        Err(_) => err_response(StatusCode::INTERNAL_SERVER_ERROR, "pdf task failed"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_real_pdfs_inside_the_workspace_are_served() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("doc.pdf"), b"%PDF-1.4\n%%EOF").unwrap();
        fs::write(dir.path().join("fake.pdf"), b"<html>").unwrap();
        fs::write(outside.path().join("secret.pdf"), b"%PDF-1.4").unwrap();

        let ok = resolve_pdf(dir.path(), "doc.pdf").unwrap();
        assert!(read_pdf(&ok).unwrap().starts_with(b"%PDF-"));

        let fake = resolve_pdf(dir.path(), "fake.pdf").unwrap();
        assert_eq!(
            read_pdf(&fake).unwrap_err().0,
            StatusCode::UNSUPPORTED_MEDIA_TYPE
        );

        assert_eq!(
            resolve_pdf(dir.path(), "../secret.pdf").unwrap_err().0,
            StatusCode::FORBIDDEN
        );
        let absolute = outside.path().join("secret.pdf");
        assert_eq!(
            resolve_pdf(dir.path(), absolute.to_str().unwrap())
                .unwrap_err()
                .0,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            resolve_pdf(dir.path(), "missing.pdf").unwrap_err().0,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            resolve_pdf(dir.path(), " ").unwrap_err().0,
            StatusCode::BAD_REQUEST
        );
    }
}
