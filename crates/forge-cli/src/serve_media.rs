//! Images the app shows inline, served from the session that owns them.
//!
//! - `GET /api/workspace/image?session=<id>&path=<p>[&w=<px>]`  one image in the session's
//!   workspace, or in its evidence folder (screenshots live outside the project tree)
//! - `GET /api/evidence?session=<id>`                            the session's captured screenshots
//! - `GET /api/evidence/image?session=<id>&file=<name>[&w=<px>]` one of them
//!
//! A client never names a directory: roots come from the session, every path is canonicalized and
//! must stay inside the workspace or that session's evidence folder, and what is served is decided
//! by the file's magic bytes, never by its name. `w` asks for a downsampled copy for thumbnails.

use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use forge_tools::evidence::{EvidenceKind, Ledger};
use tokio::sync::Semaphore;

use crate::image_input::{sniff, Sniffed};
use crate::serve::{err_response, json_response, DaemonState};
use crate::serve_git::session_dir;

const MAX_IMAGE_BYTES: u64 = 20 * 1024 * 1024;
const MAX_DECODE_PIXELS: u64 = 64 * 1024 * 1024;
const MIN_THUMB: u32 = 32;
const MAX_THUMB: u32 = 1024;
const MAX_EVIDENCE_ITEMS: usize = 500;

/// A phone scrolling a gallery asks for dozens of thumbnails at once; decoding them all together
/// is what would hurt the daemon, so only a couple run at a time and the rest wait their turn.
static THUMBNAILS: Semaphore = Semaphore::const_new(2);

pub(crate) fn routes(base: &str) -> Router<Arc<DaemonState>> {
    Router::new()
        .route(&format!("{base}/api/workspace/image"), get(workspace_image))
        .route(&format!("{base}/api/evidence"), get(evidence_list))
        .route(&format!("{base}/api/evidence/image"), get(evidence_image))
}

#[derive(serde::Deserialize)]
pub(crate) struct ImageQuery {
    #[serde(default)]
    session: String,
    #[serde(default)]
    path: String,
    #[serde(default)]
    file: String,
    w: Option<u32>,
}

#[derive(serde::Serialize)]
struct EvidenceImage {
    file: String,
    ts: String,
    source: String,
    caption: Option<String>,
    url: Option<String>,
    size: u64,
}

#[derive(serde::Serialize)]
struct EvidenceResponse {
    items: Vec<EvidenceImage>,
}

struct Fail(StatusCode, &'static str);

impl Fail {
    fn into_response(self) -> Response {
        err_response(self.0, self.1)
    }
}

fn not_found(message: &'static str) -> Fail {
    Fail(StatusCode::NOT_FOUND, message)
}

fn forbidden(message: &'static str) -> Fail {
    Fail(StatusCode::FORBIDDEN, message)
}

async fn session_workspace(state: &DaemonState, session: &str) -> Result<PathBuf, Fail> {
    if session.trim().is_empty() {
        return Err(Fail(StatusCode::BAD_REQUEST, "session is required"));
    }
    let dir = session_dir(state, session)
        .await
        .ok_or_else(|| not_found("no such session"))?;
    fs::canonicalize(dir).map_err(|_| not_found("session has no working directory on this host"))
}

fn evidence_dir(workspace: &Path, session: &str) -> Option<PathBuf> {
    Ledger::new(workspace, session).map(|ledger| ledger.dir().to_path_buf())
}

/// The file a request names, once it is known to sit inside one of `roots`. A relative path is
/// the workspace's; an absolute one is accepted so the path an agent printed works as written.
fn resolve_inside(roots: &[PathBuf], raw: &str) -> Result<PathBuf, Fail> {
    let raw = raw.trim();
    let raw = raw.strip_prefix("file://").unwrap_or(raw);
    if raw.is_empty() {
        return Err(Fail(StatusCode::BAD_REQUEST, "path is required"));
    }
    let requested = Path::new(raw);
    if requested.components().any(|part| {
        matches!(part, Component::ParentDir)
            || matches!(part, Component::Normal(name) if name == ".git")
    }) {
        return Err(forbidden("path must stay inside the workspace"));
    }
    let candidate = if requested.is_absolute() {
        requested.to_path_buf()
    } else {
        roots[0].join(requested)
    };
    let resolved = fs::canonicalize(&candidate).map_err(|_| not_found("image not found"))?;
    let inside = roots
        .iter()
        .filter_map(|root| fs::canonicalize(root).ok())
        .any(|root| resolved.starts_with(root));
    if !inside {
        return Err(forbidden("path is outside this session"));
    }
    Ok(resolved)
}

struct Served {
    bytes: Vec<u8>,
    content_type: &'static str,
}

fn content_type(kind: Sniffed) -> &'static str {
    match kind {
        Sniffed::Png => "image/png",
        Sniffed::Jpeg => "image/jpeg",
        Sniffed::Gif => "image/gif",
        Sniffed::Webp => "image/webp",
        Sniffed::Heic => "image/heic",
    }
}

fn looks_like_svg(bytes: &[u8]) -> bool {
    let head = &bytes[..bytes.len().min(1024)];
    let text = String::from_utf8_lossy(head);
    text.contains("<svg")
}

fn load_image(path: &Path, thumb: Option<u32>) -> Result<Served, Fail> {
    let metadata = fs::metadata(path).map_err(|_| not_found("image not found"))?;
    if !metadata.is_file() {
        return Err(Fail(StatusCode::BAD_REQUEST, "path is not a file"));
    }
    if metadata.len() > MAX_IMAGE_BYTES {
        return Err(Fail(
            StatusCode::PAYLOAD_TOO_LARGE,
            "image is larger than 20 MiB",
        ));
    }
    let bytes = fs::read(path).map_err(|_| not_found("image not readable"))?;
    let Some(kind) = sniff(&bytes) else {
        if looks_like_svg(&bytes) {
            return Ok(Served {
                bytes,
                content_type: "image/svg+xml",
            });
        }
        return Err(Fail(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "file is not an image",
        ));
    };
    let Some(width) = thumb else {
        return Ok(Served {
            bytes,
            content_type: content_type(kind),
        });
    };
    downsample(bytes, kind, width)
}

fn downsample(bytes: Vec<u8>, kind: Sniffed, width: u32) -> Result<Served, Fail> {
    if kind == Sniffed::Heic {
        return Ok(Served {
            bytes,
            content_type: content_type(kind),
        });
    }
    let reader = image::ImageReader::new(std::io::Cursor::new(&bytes))
        .with_guessed_format()
        .map_err(|_| Fail(StatusCode::UNSUPPORTED_MEDIA_TYPE, "unreadable image"))?;
    let (w, h) = reader
        .into_dimensions()
        .map_err(|_| Fail(StatusCode::UNSUPPORTED_MEDIA_TYPE, "unreadable image"))?;
    if u64::from(w) * u64::from(h) > MAX_DECODE_PIXELS {
        return Err(Fail(
            StatusCode::PAYLOAD_TOO_LARGE,
            "image is too large to preview",
        ));
    }
    if w <= width {
        return Ok(Served {
            bytes,
            content_type: content_type(kind),
        });
    }
    let decoded = image::load_from_memory(&bytes)
        .map_err(|_| Fail(StatusCode::UNSUPPORTED_MEDIA_TYPE, "unreadable image"))?;
    let height = ((u64::from(h) * u64::from(width)) / u64::from(w)).max(1) as u32;
    let small = decoded.thumbnail(width, height);
    let mut out = Vec::new();
    let (format, mime) = if kind == Sniffed::Jpeg {
        (image::ImageFormat::Jpeg, "image/jpeg")
    } else {
        (image::ImageFormat::Png, "image/png")
    };
    let encodable = if format == image::ImageFormat::Jpeg {
        image::DynamicImage::ImageRgb8(small.to_rgb8())
    } else {
        small
    };
    encodable
        .write_to(&mut std::io::Cursor::new(&mut out), format)
        .map_err(|_| Fail(StatusCode::INTERNAL_SERVER_ERROR, "could not scale image"))?;
    Ok(Served {
        bytes: out,
        content_type: mime,
    })
}

fn clamp_thumb(width: Option<u32>) -> Option<u32> {
    width.map(|w| w.clamp(MIN_THUMB, MAX_THUMB))
}

async fn serve(path: PathBuf, width: Option<u32>, cache: &'static str) -> Response {
    let thumb = clamp_thumb(width);
    let _permit = if thumb.is_some() {
        THUMBNAILS.acquire().await.ok()
    } else {
        None
    };
    match tokio::task::spawn_blocking(move || load_image(&path, thumb)).await {
        Ok(Ok(served)) => (
            [
                (header::CONTENT_TYPE, served.content_type),
                (header::CACHE_CONTROL, cache),
                (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
                (
                    header::CONTENT_SECURITY_POLICY,
                    "sandbox; default-src 'none'",
                ),
            ],
            served.bytes,
        )
            .into_response(),
        Ok(Err(fail)) => fail.into_response(),
        Err(_) => err_response(StatusCode::INTERNAL_SERVER_ERROR, "image task failed"),
    }
}

async fn workspace_image(
    State(state): State<Arc<DaemonState>>,
    Query(query): Query<ImageQuery>,
) -> Response {
    let workspace = match session_workspace(&state, &query.session).await {
        Ok(workspace) => workspace,
        Err(fail) => return fail.into_response(),
    };
    let mut roots = vec![workspace.clone()];
    roots.extend(evidence_dir(&workspace, &query.session));
    match resolve_inside(&roots, &query.path) {
        Ok(path) => serve(path, query.w, "private, max-age=30").await,
        Err(fail) => fail.into_response(),
    }
}

fn is_image_file(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    ["png", "jpg", "jpeg", "gif", "webp"]
        .iter()
        .any(|ext| lower.ends_with(&format!(".{ext}")))
}

async fn evidence_list(
    State(state): State<Arc<DaemonState>>,
    Query(query): Query<ImageQuery>,
) -> Response {
    let workspace = match session_workspace(&state, &query.session).await {
        Ok(workspace) => workspace,
        Err(fail) => return fail.into_response(),
    };
    let session = query.session.clone();
    let items = tokio::task::spawn_blocking(move || list_evidence(&workspace, &session))
        .await
        .unwrap_or_default();
    json_response(&EvidenceResponse { items })
}

fn list_evidence(workspace: &Path, session: &str) -> Vec<EvidenceImage> {
    let Some(ledger) = Ledger::new(workspace, session) else {
        return Vec::new();
    };
    let mut items: Vec<EvidenceImage> = ledger
        .items()
        .into_iter()
        .filter(|item| item.kind == EvidenceKind::Screenshot && is_image_file(&item.file))
        .filter_map(|item| {
            let size = fs::metadata(ledger.dir().join(&item.file)).ok()?.len();
            Some(EvidenceImage {
                file: item.file,
                ts: item.ts,
                source: item.source,
                caption: item.caption,
                url: item.url,
                size,
            })
        })
        .collect();
    if items.len() > MAX_EVIDENCE_ITEMS {
        items.drain(..items.len() - MAX_EVIDENCE_ITEMS);
    }
    items
}

async fn evidence_image(
    State(state): State<Arc<DaemonState>>,
    Query(query): Query<ImageQuery>,
) -> Response {
    let workspace = match session_workspace(&state, &query.session).await {
        Ok(workspace) => workspace,
        Err(fail) => return fail.into_response(),
    };
    let Some(dir) = evidence_dir(&workspace, &query.session) else {
        return not_found("no such session").into_response();
    };
    if query.file.is_empty()
        || query.file.contains(['/', '\\'])
        || query.file.starts_with('.')
        || !is_image_file(&query.file)
    {
        return Fail(StatusCode::BAD_REQUEST, "not an evidence image").into_response();
    }
    match resolve_inside(
        std::slice::from_ref(&dir),
        &dir.join(&query.file).to_string_lossy(),
    ) {
        Ok(path) => serve(path, query.w, "private, max-age=31536000, immutable").await,
        Err(fail) => fail.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(width: u32, height: u32) -> Vec<u8> {
        let image = image::RgbaImage::from_pixel(width, height, image::Rgba([200, 40, 40, 255]));
        let mut out = Vec::new();
        image::DynamicImage::ImageRgba8(image)
            .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        out
    }

    #[test]
    fn relative_and_absolute_paths_resolve_inside_the_workspace() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("docs")).unwrap();
        fs::write(dir.path().join("docs/a.png"), png(4, 4)).unwrap();
        let roots = vec![fs::canonicalize(dir.path()).unwrap()];
        let relative = resolve_inside(&roots, "docs/a.png").ok().unwrap();
        let absolute = resolve_inside(&roots, &dir.path().join("docs/a.png").to_string_lossy())
            .ok()
            .unwrap();
        assert_eq!(relative, absolute);
    }

    #[test]
    fn paths_that_leave_the_session_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("secret.png"), png(2, 2)).unwrap();
        fs::create_dir(dir.path().join(".git")).unwrap();
        fs::write(dir.path().join(".git/x.png"), png(2, 2)).unwrap();
        let roots = vec![fs::canonicalize(dir.path()).unwrap()];
        for raw in [
            "../secret.png".to_string(),
            outside
                .path()
                .join("secret.png")
                .to_string_lossy()
                .into_owned(),
            ".git/x.png".to_string(),
        ] {
            let Err(Fail(status, _)) = resolve_inside(&roots, &raw) else {
                panic!("{raw} resolved");
            };
            assert_eq!(status, StatusCode::FORBIDDEN, "{raw}");
        }
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(
                outside.path().join("secret.png"),
                dir.path().join("link.png"),
            )
            .unwrap();
            let Err(Fail(status, _)) = resolve_inside(&roots, "link.png") else {
                panic!("symlink escaped");
            };
            assert_eq!(status, StatusCode::FORBIDDEN);
        }
    }

    #[test]
    fn what_is_served_follows_the_bytes_not_the_name() {
        let dir = tempfile::tempdir().unwrap();
        let fake = dir.path().join("fake.png");
        fs::write(&fake, "just text").unwrap();
        let Err(Fail(status, _)) = load_image(&fake, None) else {
            panic!("text served as an image");
        };
        assert_eq!(status, StatusCode::UNSUPPORTED_MEDIA_TYPE);

        let real = dir.path().join("no-extension");
        fs::write(&real, png(3, 3)).unwrap();
        let served = load_image(&real, None).ok().unwrap();
        assert_eq!(served.content_type, "image/png");

        let svg = dir.path().join("logo.svg");
        fs::write(&svg, "<svg xmlns='http://www.w3.org/2000/svg'/>").unwrap();
        assert_eq!(
            load_image(&svg, None).ok().unwrap().content_type,
            "image/svg+xml"
        );
    }

    #[test]
    fn thumbnails_shrink_big_images_and_leave_small_ones() {
        let dir = tempfile::tempdir().unwrap();
        let big = dir.path().join("big.png");
        fs::write(&big, png(800, 400)).unwrap();
        let served = load_image(&big, Some(200)).ok().unwrap();
        let decoded = image::load_from_memory(&served.bytes).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (200, 100));

        let small = dir.path().join("small.png");
        let original = png(40, 40);
        fs::write(&small, &original).unwrap();
        assert_eq!(load_image(&small, Some(200)).ok().unwrap().bytes, original);
    }

    #[test]
    fn only_captured_screenshots_are_listed_in_capture_order() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = Ledger::new(dir.path(), "sess-g").unwrap();
        let capture = |caption: &'static str| forge_tools::evidence::Capture {
            caption: Some(caption),
            ..Default::default()
        };
        ledger
            .record(
                EvidenceKind::Screenshot,
                "browser",
                "png",
                &png(2, 2),
                capture("first"),
            )
            .unwrap();
        ledger.record_test_output("cargo test", 0, "ok").unwrap();
        ledger
            .record(
                EvidenceKind::Screenshot,
                "device",
                "png",
                &png(2, 2),
                capture("second"),
            )
            .unwrap();
        let listed = list_evidence(dir.path(), "sess-g");
        let captions: Vec<_> = listed.iter().filter_map(|i| i.caption.as_deref()).collect();
        assert_eq!(captions, ["first", "second"]);
        assert_eq!(listed[0].file, "0001-browser.png");
        assert!(listed[0].size > 0);
        assert!(list_evidence(dir.path(), "../escape").is_empty());
    }

    #[tokio::test]
    async fn the_routes_serve_workspace_images_and_the_evidence_list() {
        use tower::ServiceExt;

        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("shot.png"), png(8, 8)).unwrap();
        fs::write(dir.path().join("note.txt"), "hello\n").unwrap();
        let store = Arc::new(forge_store::Store::open_in_memory().unwrap());
        let session = store
            .create_session(&dir.path().display().to_string(), "safe")
            .unwrap();
        Ledger::new(dir.path(), &session)
            .unwrap()
            .record(
                EvidenceKind::Screenshot,
                "browser",
                "png",
                &png(64, 64),
                forge_tools::evidence::Capture {
                    caption: Some("cart"),
                    ..Default::default()
                },
            )
            .unwrap();
        let state = Arc::new(DaemonState {
            registry: Arc::new(crate::serve::SessionRegistry::new()),
            terminals: Arc::new(crate::serve_terminal::TerminalRegistry::new()),
            store,
            base: "/tok".into(),
            mock: true,
            default_cwd: dir.path().display().to_string(),
            project_roots: Vec::new(),
            push: None,
            apns: None,
            voice: crate::voice::VoiceState::new(),
            anywhere_enable: tokio::sync::watch::channel(false).0,
        });
        let router = crate::serve::daemon_router(state);
        let get = |uri: String| {
            let router = router.clone();
            async move {
                router
                    .oneshot(
                        axum::http::Request::get(uri)
                            .body(axum::body::Body::empty())
                            .unwrap(),
                    )
                    .await
                    .unwrap()
            }
        };

        let ok = get(format!(
            "/tok/api/workspace/image?session={session}&path=shot.png"
        ))
        .await;
        assert_eq!(ok.status(), StatusCode::OK);
        assert_eq!(ok.headers()[header::CONTENT_TYPE], "image/png");

        let escaped = get(format!(
            "/tok/api/workspace/image?session={session}&path=../x.png"
        ))
        .await;
        assert_eq!(escaped.status(), StatusCode::FORBIDDEN);

        let stored_file = get(format!(
            "/tok/api/workspace/file?session={session}&path=note.txt"
        ))
        .await;
        assert_eq!(
            stored_file.status(),
            StatusCode::OK,
            "files open for a session that is not live"
        );

        let unknown = get("/tok/api/workspace/image?session=nope&path=shot.png".into()).await;
        assert_eq!(unknown.status(), StatusCode::NOT_FOUND);

        let listing = get(format!("/tok/api/evidence?session={session}")).await;
        assert_eq!(listing.status(), StatusCode::OK);
        let body = axum::body::to_bytes(listing.into_body(), 1 << 20)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["items"][0]["caption"], "cart");

        let thumb = get(format!(
            "/tok/api/evidence/image?session={session}&file=0001-browser.png&w=32"
        ))
        .await;
        assert_eq!(thumb.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(thumb.into_body(), 1 << 20)
            .await
            .unwrap();
        assert_eq!(image::load_from_memory(&bytes).unwrap().width(), 32);

        let traversal = get(format!(
            "/tok/api/evidence/image?session={session}&file=../x.png"
        ))
        .await;
        assert_eq!(traversal.status(), StatusCode::BAD_REQUEST);
    }
}
