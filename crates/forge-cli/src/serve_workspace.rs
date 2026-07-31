//! Session-scoped workspace browsing, search, reads, and conflict-safe edits.
//!
//! The client never supplies a filesystem root. Every request resolves the live session's
//! worktree (or cwd), then confines relative paths to that canonical root. All routes are under
//! the same unguessable `forge serve` token as the rest of the desktop/mobile API.

use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::UNIX_EPOCH;

use axum::extract::{Json, Query, State};
use axum::http::StatusCode;
use axum::response::Response;
use ignore::{DirEntry, WalkBuilder};
use sha2::{Digest, Sha256};

use crate::serve::{err_response, json_response, DaemonState};

const MAX_DIRECTORY_ENTRIES: usize = 2_000;
const MAX_FILE_BYTES: u64 = 1024 * 1024;
const MAX_SEARCH_FILE_BYTES: u64 = 512 * 1024;
const MAX_SEARCH_BYTES: u64 = 64 * 1024 * 1024;
const MAX_SEARCH_FILES: usize = 20_000;
const DEFAULT_SEARCH_RESULTS: usize = 50;
const MAX_SEARCH_RESULTS: usize = 100;

#[derive(Debug)]
struct WorkspaceFailure {
    status: StatusCode,
    message: String,
}

impl WorkspaceFailure {
    fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }

    fn bad(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, message)
    }

    fn forbidden(message: impl Into<String>) -> Self {
        Self::new(StatusCode::FORBIDDEN, message)
    }

    fn not_found(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, message)
    }

    fn internal(error: impl std::fmt::Display) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
    }
}

impl From<std::io::Error> for WorkspaceFailure {
    fn from(error: std::io::Error) -> Self {
        match error.kind() {
            std::io::ErrorKind::NotFound => Self::not_found("workspace path does not exist"),
            std::io::ErrorKind::PermissionDenied => {
                Self::forbidden("workspace path is not readable")
            }
            _ => Self::internal(error),
        }
    }
}

fn failure_response(error: WorkspaceFailure) -> Response {
    err_response(error.status, &error.message)
}

#[derive(serde::Deserialize)]
pub(crate) struct WorkspacePathQuery {
    #[serde(default)]
    session: String,
    #[serde(default)]
    path: String,
}

#[derive(serde::Deserialize)]
pub(crate) struct WorkspaceSearchQuery {
    #[serde(default)]
    session: String,
    #[serde(default)]
    q: String,
    #[serde(default)]
    mode: String,
    limit: Option<usize>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WorkspaceWriteRequest {
    session: String,
    path: String,
    content: String,
    expected_hash: String,
}

#[derive(serde::Serialize)]
pub(crate) struct WorkspaceEntry {
    name: String,
    path: String,
    /// `directory` | `file` | `symlink`.
    kind: &'static str,
    size: u64,
    modified_ms: Option<u64>,
}

#[derive(serde::Serialize)]
pub(crate) struct WorkspaceEntriesResponse {
    root: String,
    path: String,
    entries: Vec<WorkspaceEntry>,
    truncated: usize,
}

#[derive(Debug, serde::Serialize)]
pub(crate) struct WorkspaceFileResponse {
    root: String,
    path: String,
    name: String,
    content: String,
    size: u64,
    modified_ms: Option<u64>,
    hash: String,
    extension: Option<String>,
}

#[derive(serde::Serialize)]
pub(crate) struct WorkspaceSearchResult {
    path: String,
    /// `file` for path search, `match` for content search.
    kind: &'static str,
    line: Option<usize>,
    column: Option<usize>,
    preview: Option<String>,
}

#[derive(serde::Serialize)]
pub(crate) struct WorkspaceSearchResponse {
    query: String,
    mode: &'static str,
    results: Vec<WorkspaceSearchResult>,
    scanned_files: usize,
    truncated: bool,
}

async fn session_root(state: &DaemonState, session: &str) -> Result<PathBuf, WorkspaceFailure> {
    if session.trim().is_empty() {
        return Err(WorkspaceFailure::bad("session is required"));
    }
    let handle = state
        .registry
        .get(session)
        .await
        .ok_or_else(|| WorkspaceFailure::not_found("no such session"))?;
    let raw = handle
        .worktree
        .clone()
        .unwrap_or_else(|| handle.cwd.clone());
    let root = fs::canonicalize(&raw).map_err(|_| {
        WorkspaceFailure::not_found("session has no working directory on this host")
    })?;
    if !root.is_dir() {
        return Err(WorkspaceFailure::not_found(
            "session has no working directory on this host",
        ));
    }
    Ok(root)
}

fn normalized_relative(raw: &str, allow_empty: bool) -> Result<PathBuf, WorkspaceFailure> {
    let raw = raw.trim();
    if Path::new(raw).is_absolute() {
        return Err(WorkspaceFailure::bad(
            "path must be relative to the workspace root",
        ));
    }
    let trimmed = raw.trim_matches('/');
    if trimmed.is_empty() {
        return if allow_empty {
            Ok(PathBuf::new())
        } else {
            Err(WorkspaceFailure::bad("path is required"))
        };
    }

    let candidate = Path::new(trimmed);

    let mut clean = PathBuf::new();
    for component in candidate.components() {
        match component {
            Component::Normal(part) => {
                if part == ".git" {
                    return Err(WorkspaceFailure::forbidden(
                        "the repository metadata directory is not a workspace file",
                    ));
                }
                clean.push(part);
            }
            Component::CurDir => {}
            _ => {
                return Err(WorkspaceFailure::bad(
                    "path must stay inside the workspace root",
                ))
            }
        }
    }
    if clean.as_os_str().is_empty() && !allow_empty {
        return Err(WorkspaceFailure::bad("path is required"));
    }
    Ok(clean)
}

fn resolve_existing(
    root: &Path,
    raw: &str,
    allow_root: bool,
) -> Result<(PathBuf, String), WorkspaceFailure> {
    let relative = normalized_relative(raw, allow_root)?;
    let candidate = root.join(&relative);
    let resolved = fs::canonicalize(&candidate)?;
    if !resolved.starts_with(root) {
        return Err(WorkspaceFailure::forbidden(
            "workspace symlink points outside the session root",
        ));
    }
    Ok((resolved, wire_path(&relative)))
}

fn wire_path(path: &Path) -> String {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(part) => Some(part.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn modified_ms(metadata: &fs::Metadata) -> Option<u64> {
    metadata
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| u64::try_from(duration.as_millis()).ok())
}

fn content_hash(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn read_file(root: &Path, raw_path: &str) -> Result<WorkspaceFileResponse, WorkspaceFailure> {
    let relative_path = normalized_relative(raw_path, false)?;
    let unresolved = root.join(&relative_path);
    let symlink_metadata = fs::symlink_metadata(&unresolved)?;
    if symlink_metadata.file_type().is_symlink() {
        return Err(WorkspaceFailure::forbidden(
            "editing through workspace symlinks is not supported",
        ));
    }
    let (path, relative) = resolve_existing(root, raw_path, false)?;
    let metadata = fs::metadata(&path)?;
    if !metadata.is_file() {
        return Err(WorkspaceFailure::bad("workspace path is not a file"));
    }
    if metadata.len() > MAX_FILE_BYTES {
        return Err(WorkspaceFailure::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            format!(
                "file is larger than the {} byte editor limit",
                MAX_FILE_BYTES
            ),
        ));
    }
    let bytes = fs::read(&path)?;
    if bytes.contains(&0) {
        return Err(WorkspaceFailure::new(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "binary files cannot be opened in the text editor",
        ));
    }
    let content = String::from_utf8(bytes.clone()).map_err(|_| {
        WorkspaceFailure::new(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "file is not valid UTF-8 text",
        )
    })?;
    Ok(WorkspaceFileResponse {
        root: root.to_string_lossy().into_owned(),
        path: relative,
        name: path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default(),
        content,
        size: metadata.len(),
        modified_ms: modified_ms(&metadata),
        hash: content_hash(&bytes),
        extension: path
            .extension()
            .map(|extension| extension.to_string_lossy().into_owned()),
    })
}

fn directory_entries(
    root: &Path,
    raw_path: &str,
) -> Result<WorkspaceEntriesResponse, WorkspaceFailure> {
    let (directory, relative) = resolve_existing(root, raw_path, true)?;
    if !directory.is_dir() {
        return Err(WorkspaceFailure::bad("workspace path is not a directory"));
    }

    let mut entries = Vec::new();
    let mut truncated = 0usize;
    for result in fs::read_dir(&directory)? {
        let entry = result?;
        if entry.file_name() == ".git" {
            continue;
        }
        if entries.len() >= MAX_DIRECTORY_ENTRIES {
            // Stop walking a pathological flat directory. `truncated` is a sentinel here: the
            // response promises that rows were omitted, not an expensive exact remainder count.
            truncated = 1;
            break;
        }
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        let kind = if metadata.file_type().is_symlink() {
            "symlink"
        } else if metadata.is_dir() {
            "directory"
        } else if metadata.is_file() {
            "file"
        } else {
            continue;
        };
        let relative_path = path
            .strip_prefix(root)
            .map_err(WorkspaceFailure::internal)?;
        entries.push(WorkspaceEntry {
            name: entry.file_name().to_string_lossy().into_owned(),
            path: wire_path(relative_path),
            kind,
            size: metadata.len(),
            modified_ms: modified_ms(&metadata),
        });
    }
    entries.sort_by(|left, right| {
        let left_directory = left.kind == "directory";
        let right_directory = right.kind == "directory";
        right_directory
            .cmp(&left_directory)
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
            .then_with(|| left.name.cmp(&right.name))
    });

    Ok(WorkspaceEntriesResponse {
        root: root.to_string_lossy().into_owned(),
        path: relative,
        entries,
        truncated,
    })
}

fn skipped_search_directory(entry: &DirEntry) -> bool {
    if entry.depth() == 0 {
        return false;
    }
    if !entry.file_type().is_some_and(|kind| kind.is_dir()) {
        return false;
    }
    matches!(
        entry.file_name().to_string_lossy().as_ref(),
        ".git"
            | ".forge"
            | ".expo"
            | ".next"
            | "node_modules"
            | "target"
            | "dist"
            | "build"
            | "coverage"
    )
}

fn search_workspace(
    root: &Path,
    raw_query: &str,
    raw_mode: &str,
    requested_limit: Option<usize>,
) -> Result<WorkspaceSearchResponse, WorkspaceFailure> {
    let query = raw_query.trim();
    if query.is_empty() {
        return Err(WorkspaceFailure::bad("search query is required"));
    }
    let mode = match raw_mode.trim().to_ascii_lowercase().as_str() {
        "" | "files" => "files",
        "content" => "content",
        _ => {
            return Err(WorkspaceFailure::bad(
                "search mode must be files or content",
            ))
        }
    };
    let limit = requested_limit
        .unwrap_or(DEFAULT_SEARCH_RESULTS)
        .clamp(1, MAX_SEARCH_RESULTS);
    let query_lower = query.to_lowercase();
    let mut results = Vec::new();
    let mut ranked_files = Vec::new();
    let mut scanned_files = 0usize;
    let mut scanned_bytes = 0u64;
    let mut truncated = false;

    let mut builder = WalkBuilder::new(root);
    builder
        .hidden(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .follow_links(false)
        .filter_entry(|entry| !skipped_search_directory(entry));

    for result in builder.build() {
        let Ok(entry) = result else {
            continue;
        };
        if entry.depth() == 0 || !entry.file_type().is_some_and(|kind| kind.is_file()) {
            continue;
        }
        scanned_files += 1;
        if scanned_files > MAX_SEARCH_FILES {
            truncated = true;
            break;
        }
        let Ok(relative) = entry.path().strip_prefix(root) else {
            continue;
        };
        let path = wire_path(relative);

        if mode == "files" {
            let path_lower = path.to_lowercase();
            let name_lower = entry.file_name().to_string_lossy().to_lowercase();
            let score = if name_lower.starts_with(&query_lower) {
                Some(0)
            } else if name_lower.contains(&query_lower) {
                Some(1)
            } else if path_lower.starts_with(&query_lower) {
                Some(2)
            } else if path_lower.contains(&query_lower) {
                Some(3)
            } else {
                None
            };
            if let Some(score) = score {
                ranked_files.push((score, path.len(), path));
            }
            continue;
        }

        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if metadata.len() > MAX_SEARCH_FILE_BYTES {
            continue;
        }
        if scanned_bytes.saturating_add(metadata.len()) > MAX_SEARCH_BYTES {
            truncated = true;
            break;
        }
        scanned_bytes += metadata.len();
        let Ok(bytes) = fs::read(entry.path()) else {
            continue;
        };
        if bytes.contains(&0) {
            continue;
        }
        let Ok(content) = String::from_utf8(bytes) else {
            continue;
        };
        for (line_index, line) in content.lines().enumerate() {
            let line_lower = line.to_lowercase();
            let Some(column) = line_lower.find(&query_lower) else {
                continue;
            };
            results.push(WorkspaceSearchResult {
                path: path.clone(),
                kind: "match",
                line: Some(line_index + 1),
                column: Some(line[..column].chars().count() + 1),
                preview: Some(line.trim().chars().take(240).collect()),
            });
            if results.len() >= limit {
                truncated = true;
                break;
            }
        }
        if results.len() >= limit {
            break;
        }
    }

    if mode == "files" {
        ranked_files.sort();
        if ranked_files.len() > limit {
            truncated = true;
        }
        results = ranked_files
            .into_iter()
            .take(limit)
            .map(|(_, _, path)| WorkspaceSearchResult {
                path,
                kind: "file",
                line: None,
                column: None,
                preview: None,
            })
            .collect();
    }

    Ok(WorkspaceSearchResponse {
        query: query.to_string(),
        mode,
        results,
        scanned_files: scanned_files.min(MAX_SEARCH_FILES),
        truncated,
    })
}

fn write_file(
    root: &Path,
    request: WorkspaceWriteRequest,
) -> Result<WorkspaceFileResponse, WorkspaceFailure> {
    if request.content.len() > MAX_FILE_BYTES as usize {
        return Err(WorkspaceFailure::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            format!(
                "content is larger than the {} byte editor limit",
                MAX_FILE_BYTES
            ),
        ));
    }
    let current = read_file(root, &request.path)?;
    if current.hash != request.expected_hash {
        return Err(WorkspaceFailure::new(
            StatusCode::CONFLICT,
            "file changed since it was opened; reload before saving",
        ));
    }
    let (path, _) = resolve_existing(root, &request.path, false)?;
    let metadata = fs::metadata(&path)?;
    let parent = path
        .parent()
        .ok_or_else(|| WorkspaceFailure::bad("file has no workspace parent"))?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(request.content.as_bytes())?;
    temporary.as_file_mut().flush()?;
    temporary
        .as_file()
        .set_permissions(metadata.permissions())?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(&path)
        .map_err(|error| WorkspaceFailure::internal(error.error))?;
    read_file(root, &request.path)
}

pub(crate) async fn workspace_entries(
    State(state): State<Arc<DaemonState>>,
    Query(params): Query<WorkspacePathQuery>,
) -> Response {
    let root = match session_root(&state, &params.session).await {
        Ok(root) => root,
        Err(error) => return failure_response(error),
    };
    match tokio::task::spawn_blocking(move || directory_entries(&root, &params.path)).await {
        Ok(Ok(response)) => json_response(&response),
        Ok(Err(error)) => failure_response(error),
        Err(error) => failure_response(WorkspaceFailure::internal(error)),
    }
}

pub(crate) async fn workspace_file(
    State(state): State<Arc<DaemonState>>,
    Query(params): Query<WorkspacePathQuery>,
) -> Response {
    let root = match session_root(&state, &params.session).await {
        Ok(root) => root,
        Err(error) => return failure_response(error),
    };
    match tokio::task::spawn_blocking(move || read_file(&root, &params.path)).await {
        Ok(Ok(response)) => json_response(&response),
        Ok(Err(error)) => failure_response(error),
        Err(error) => failure_response(WorkspaceFailure::internal(error)),
    }
}

pub(crate) async fn workspace_search(
    State(state): State<Arc<DaemonState>>,
    Query(params): Query<WorkspaceSearchQuery>,
) -> Response {
    let root = match session_root(&state, &params.session).await {
        Ok(root) => root,
        Err(error) => return failure_response(error),
    };
    match tokio::task::spawn_blocking(move || {
        search_workspace(&root, &params.q, &params.mode, params.limit)
    })
    .await
    {
        Ok(Ok(response)) => json_response(&response),
        Ok(Err(error)) => failure_response(error),
        Err(error) => failure_response(WorkspaceFailure::internal(error)),
    }
}

pub(crate) async fn workspace_write(
    State(state): State<Arc<DaemonState>>,
    Json(request): Json<WorkspaceWriteRequest>,
) -> Response {
    let root = match session_root(&state, &request.session).await {
        Ok(root) => root,
        Err(error) => return failure_response(error),
    };
    match tokio::task::spawn_blocking(move || write_file(&root, request)).await {
        Ok(Ok(response)) => json_response(&response),
        Ok(Err(error)) => failure_response(error),
        Err(error) => failure_response(WorkspaceFailure::internal(error)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_paths_reject_escape_and_repository_metadata() {
        assert!(normalized_relative("src/main.rs", false).is_ok());
        assert!(normalized_relative("", true).is_ok());
        assert!(normalized_relative("../secret", false).is_err());
        assert!(normalized_relative("/etc/passwd", false).is_err());
        assert!(normalized_relative(".git/config", false).is_err());
    }

    #[test]
    fn read_and_write_use_optimistic_hashes() {
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("hello.txt");
        fs::write(&file, "one\n").unwrap();
        let opened = read_file(directory.path(), "hello.txt").unwrap();

        let saved = write_file(
            directory.path(),
            WorkspaceWriteRequest {
                session: "ignored-in-unit-test".into(),
                path: "hello.txt".into(),
                content: "two\n".into(),
                expected_hash: opened.hash.clone(),
            },
        )
        .unwrap();
        assert_eq!(saved.content, "two\n");
        assert_ne!(saved.hash, opened.hash);

        let conflict = write_file(
            directory.path(),
            WorkspaceWriteRequest {
                session: "ignored-in-unit-test".into(),
                path: "hello.txt".into(),
                content: "three\n".into(),
                expected_hash: opened.hash,
            },
        )
        .unwrap_err();
        assert_eq!(conflict.status, StatusCode::CONFLICT);
        assert_eq!(fs::read_to_string(file).unwrap(), "two\n");
    }

    #[test]
    fn search_supports_ranked_paths_and_bounded_content_matches() {
        let directory = tempfile::tempdir().unwrap();
        fs::create_dir(directory.path().join("src")).unwrap();
        fs::write(
            directory.path().join("src/workspace_model.rs"),
            "first line\nCritical Needle here\n",
        )
        .unwrap();
        fs::write(directory.path().join("workspace.md"), "needle again\n").unwrap();

        let files = search_workspace(directory.path(), "workspace", "files", Some(10)).unwrap();
        assert_eq!(files.results.len(), 2);
        assert_eq!(files.results[0].path, "workspace.md");

        let content = search_workspace(directory.path(), "needle", "content", Some(10)).unwrap();
        assert_eq!(content.results.len(), 2);
        let source_match = content
            .results
            .iter()
            .find(|result| result.path == "src/workspace_model.rs")
            .unwrap();
        assert_eq!(source_match.line, Some(2));
        assert_eq!(source_match.column, Some(10));
    }

    #[cfg(unix)]
    #[test]
    fn symlink_escape_is_rejected() {
        use std::os::unix::fs::symlink;

        let workspace = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("secret.txt"), "secret").unwrap();
        symlink(
            outside.path().join("secret.txt"),
            workspace.path().join("escape.txt"),
        )
        .unwrap();

        let error = read_file(workspace.path(), "escape.txt").unwrap_err();
        assert_eq!(error.status, StatusCode::FORBIDDEN);
    }
}
