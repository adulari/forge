//! Safe workspace capsule export/import for Forge Anywhere handoff.

use std::fs::{File, OpenOptions};
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Default maximum size of one workspace file: 25 MiB.
pub const MAX_FILE_BYTES: u64 = 25 * 1024 * 1024;
/// Default maximum compressed capsule size: 100 MiB.
pub const MAX_CAPSULE_BYTES: u64 = 100 * 1024 * 1024;
const MAX_EXPANDED_BYTES: u64 = 500 * 1024 * 1024;
const MAX_METADATA_BYTES: u64 = 1024 * 1024;
const MAX_SESSION_BYTES: u64 = 50 * 1024 * 1024;
const MAX_PATCH_BYTES: u64 = 300 * 1024 * 1024;

/// Capsule resource limits. The V1 product defaults are hard upper bounds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapsuleLimits {
    pub max_file_bytes: u64,
    pub max_compressed_bytes: u64,
}

impl Default for CapsuleLimits {
    fn default() -> Self {
        Self {
            max_file_bytes: MAX_FILE_BYTES,
            max_compressed_bytes: MAX_CAPSULE_BYTES,
        }
    }
}

/// One portable untracked file recorded in a capsule manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapsuleFile {
    pub path: String,
    pub size: u64,
    pub sha256: String,
}

/// Encrypted repository identity used to reject importing into an unrelated checkout.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapsuleRepository {
    /// Hash of the configured origin URL after removing URL user-info. Empty when no origin exists.
    pub origin_fingerprint: String,
    /// Source branch name when `HEAD` is attached. Informational; imports remain detached.
    pub head_ref: Option<String>,
}

/// Authenticated capsule metadata. The whole archive is encrypted by the Anywhere protocol before
/// it leaves the host; hashes additionally detect local archive corruption before import.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapsuleManifest {
    pub version: u8,
    pub session_id: String,
    pub base_commit: String,
    #[serde(default)]
    pub repository: CapsuleRepository,
    pub created_at_ms: u64,
    pub session_sha256: String,
    pub patch_sha256: String,
    pub untracked: Vec<CapsuleFile>,
}

/// Successful export details.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportedCapsule {
    pub path: PathBuf,
    pub manifest: CapsuleManifest,
    pub compressed_bytes: u64,
}

/// Successful import details. The caller imports/remaps `session_json` before acknowledging the
/// service lease; the isolated worktree remains detached until that acknowledgement succeeds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedCapsule {
    pub worktree_path: PathBuf,
    pub manifest: CapsuleManifest,
    pub session_json: Vec<u8>,
}

/// A workspace path rejected during capsule preflight.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnsafePath {
    pub path: String,
    pub reason: String,
}

/// Capsule preflight, archive, Git, and rollback failures.
#[derive(Debug, thiserror::Error)]
pub enum CapsuleError {
    #[error("workspace capsule preflight rejected files: {0}")]
    UnsafeFiles(UnsafePaths),
    #[error("capsule output already exists: {0}")]
    OutputExists(PathBuf),
    #[error("capsule exceeds the {limit} byte compressed limit ({actual} bytes)")]
    CapsuleTooLarge { actual: u64, limit: u64 },
    #[error("capsule archive is invalid: {0}")]
    InvalidArchive(String),
    #[error("capsule base commit is invalid or unavailable: {0}")]
    InvalidBase(String),
    #[error("capsule belongs to a different repository")]
    RepositoryMismatch,
    #[error("destination worktree already exists: {0}")]
    DestinationExists(PathBuf),
    #[error("git {operation} failed: {details}")]
    Git {
        operation: &'static str,
        details: String,
    },
    #[error("capsule I/O failed while {operation}: {source}")]
    Io {
        operation: &'static str,
        #[source]
        source: std::io::Error,
    },
    #[error("capsule JSON is invalid: {0}")]
    Json(#[from] serde_json::Error),
}

/// Display wrapper that preserves every rejected path and reason in one actionable error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnsafePaths(pub Vec<UnsafePath>);

impl std::fmt::Display for UnsafePaths {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (index, rejected) in self.0.iter().enumerate() {
            if index > 0 {
                formatter.write_str("; ")?;
            }
            write!(formatter, "{} ({})", rejected.path, rejected.reason)?;
        }
        Ok(())
    }
}

/// Export the current `HEAD` workspace state and a caller-provided portable session export.
///
/// The command captures staged and unstaged tracked changes with
/// `git diff --binary --full-index HEAD` and only non-ignored untracked regular files. Any unsafe
/// path aborts the entire export; no user file is silently omitted.
pub fn export_capsule(
    repo_root: &Path,
    output_path: &Path,
    session_id: &str,
    session_json: &[u8],
    limits: CapsuleLimits,
) -> Result<ExportedCapsule, CapsuleError> {
    if output_path.exists() {
        return Err(CapsuleError::OutputExists(output_path.to_path_buf()));
    }
    let repo_root = repo_root
        .canonicalize()
        .map_err(|source| CapsuleError::Io {
            operation: "canonicalizing the repository",
            source,
        })?;
    let base_commit = git_text(&repo_root, &["rev-parse", "HEAD"], "resolve HEAD")?;
    validate_commit(&base_commit)?;
    let repository = repository_metadata(&repo_root);
    let patch = git_bytes(
        &repo_root,
        &["diff", "--binary", "--full-index", "HEAD", "--"],
        "create binary workspace patch",
    )?;
    let changed = git_zero_paths(
        &repo_root,
        &["diff", "--name-only", "-z", "HEAD", "--"],
        "list changed paths",
    )?;
    let untracked = git_zero_paths(
        &repo_root,
        &["ls-files", "--others", "--exclude-standard", "-z"],
        "list untracked files",
    )?;

    let mut rejected = Vec::new();
    for path in &changed {
        validate_workspace_file(&repo_root, path, limits.max_file_bytes, true, &mut rejected);
    }
    let mut files = Vec::with_capacity(untracked.len());
    for path in &untracked {
        if validate_workspace_file(
            &repo_root,
            path,
            limits.max_file_bytes,
            false,
            &mut rejected,
        ) {
            let absolute = repo_root.join(path);
            let size = absolute
                .metadata()
                .map(|metadata| metadata.len())
                .unwrap_or(0);
            let hash = hash_file_nofollow(&absolute).unwrap_or_else(|error| {
                rejected.push(UnsafePath {
                    path: display_path(path),
                    reason: error.to_string(),
                });
                String::new()
            });
            if !hash.is_empty() {
                files.push(CapsuleFile {
                    path: portable_path(path),
                    size,
                    sha256: hash,
                });
            }
        }
    }
    if !rejected.is_empty() {
        return Err(CapsuleError::UnsafeFiles(UnsafePaths(rejected)));
    }

    let manifest = CapsuleManifest {
        version: 1,
        session_id: session_id.to_owned(),
        base_commit,
        repository,
        created_at_ms: now_ms(),
        session_sha256: hash_bytes(session_json),
        patch_sha256: hash_bytes(&patch),
        untracked: files,
    };
    let manifest_json = serde_json::to_vec_pretty(&manifest)?;
    let parent = output_path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent).map_err(|source| CapsuleError::Io {
        operation: "creating the capsule output directory",
        source,
    })?;
    let temp_path = parent.join(format!(
        ".forge-capsule-{}-{:016x}.tmp",
        std::process::id(),
        rand_suffix()
    ));
    let result = write_archive(
        &temp_path,
        &repo_root,
        &manifest_json,
        session_json,
        &patch,
        &manifest.untracked,
    );
    if let Err(error) = result {
        let _ = std::fs::remove_file(&temp_path);
        return Err(error);
    }
    let compressed_bytes = std::fs::metadata(&temp_path)
        .map_err(|source| CapsuleError::Io {
            operation: "measuring the capsule",
            source,
        })?
        .len();
    if compressed_bytes > limits.max_compressed_bytes {
        let _ = std::fs::remove_file(&temp_path);
        return Err(CapsuleError::CapsuleTooLarge {
            actual: compressed_bytes,
            limit: limits.max_compressed_bytes,
        });
    }
    std::fs::rename(&temp_path, output_path).map_err(|source| CapsuleError::Io {
        operation: "installing the capsule",
        source,
    })?;
    Ok(ExportedCapsule {
        path: output_path.to_path_buf(),
        manifest,
        compressed_bytes,
    })
}

/// Return the privacy-preserving repository identity used by capsule preflight and import.
#[must_use]
pub fn repository_metadata(repo_root: &Path) -> CapsuleRepository {
    let origin_fingerprint = Command::new("git")
        .current_dir(repo_root)
        .args(["config", "--get", "remote.origin.url"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|origin| origin.trim().to_owned())
        .filter(|origin| !origin.is_empty())
        .map(|origin| hash_bytes(canonical_remote_identity(&origin).as_bytes()))
        .unwrap_or_default();
    let head_ref = Command::new("git")
        .current_dir(repo_root)
        .args(["symbolic-ref", "--quiet", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty());
    CapsuleRepository {
        origin_fingerprint,
        head_ref,
    }
}

fn strip_url_userinfo(origin: &str) -> String {
    let Some((scheme, rest)) = origin.split_once("://") else {
        return origin.to_owned();
    };
    let authority_end = rest.find('/').unwrap_or(rest.len());
    let (authority, suffix) = rest.split_at(authority_end);
    let host = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    format!("{scheme}://{host}{suffix}")
}

fn canonical_remote_identity(origin: &str) -> String {
    let stripped = strip_url_userinfo(origin.trim());
    let (authority, path) = if let Some((_, rest)) = stripped.split_once("://") {
        let slash = rest.find('/').unwrap_or(rest.len());
        let (authority, path) = rest.split_at(slash);
        (authority, path.trim_start_matches('/'))
    } else if let Some((authority, path)) = stripped.split_once(':') {
        // Git's SCP syntax (`git@host:owner/repo`) has no URI scheme. Do not reinterpret local
        // Windows drive paths as remotes.
        if authority.contains('@') || authority.contains('.') || authority == "localhost" {
            (
                authority
                    .rsplit_once('@')
                    .map_or(authority, |(_, host)| host),
                path,
            )
        } else {
            return stripped;
        }
    } else {
        return stripped;
    };
    let host = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host)
        .trim_end_matches(['/', ':'])
        .to_ascii_lowercase();
    let mut path = path.trim_matches('/').to_ascii_lowercase();
    if path.ends_with(".git") {
        path.truncate(path.len() - 4);
    }
    format!("{host}/{path}")
}

/// Import a capsule into a new isolated detached worktree.
///
/// Every archive entry is validated and staged before Git is touched. Any apply/extract failure
/// removes the new worktree with `git worktree remove --force`; the source session lease remains a
/// service concern and must not move until the returned import is acknowledged.
pub fn import_capsule(
    repo_root: &Path,
    capsule_path: &Path,
    destination: &Path,
    limits: CapsuleLimits,
) -> Result<ImportedCapsule, CapsuleError> {
    if destination.exists() {
        return Err(CapsuleError::DestinationExists(destination.to_path_buf()));
    }
    let compressed = std::fs::metadata(capsule_path)
        .map_err(|source| CapsuleError::Io {
            operation: "reading capsule metadata",
            source,
        })?
        .len();
    if compressed > limits.max_compressed_bytes {
        return Err(CapsuleError::CapsuleTooLarge {
            actual: compressed,
            limit: limits.max_compressed_bytes,
        });
    }

    let staging = tempfile::tempdir().map_err(|source| CapsuleError::Io {
        operation: "creating capsule staging directory",
        source,
    })?;
    extract_to_staging(capsule_path, staging.path(), limits)?;
    let manifest_bytes = read_limited(&staging.path().join("manifest.json"), MAX_METADATA_BYTES)?;
    let manifest: CapsuleManifest = serde_json::from_slice(&manifest_bytes)?;
    if manifest.version != 1 {
        return Err(CapsuleError::InvalidArchive(format!(
            "unsupported capsule version {}",
            manifest.version
        )));
    }
    validate_commit(&manifest.base_commit)?;
    let session_json = read_limited(&staging.path().join("session.json"), MAX_SESSION_BYTES)?;
    let patch = read_limited(&staging.path().join("workspace.patch"), MAX_PATCH_BYTES)?;
    verify_hash("session.json", &session_json, &manifest.session_sha256)?;
    verify_hash("workspace.patch", &patch, &manifest.patch_sha256)?;
    verify_manifest_files(staging.path(), &manifest, limits)?;

    let repo_root = repo_root
        .canonicalize()
        .map_err(|source| CapsuleError::Io {
            operation: "canonicalizing the destination repository",
            source,
        })?;
    let local_repository = repository_metadata(&repo_root);
    if !manifest.repository.origin_fingerprint.is_empty()
        && !local_repository.origin_fingerprint.is_empty()
        && manifest.repository.origin_fingerprint != local_repository.origin_fingerprint
    {
        return Err(CapsuleError::RepositoryMismatch);
    }
    git_output(
        &repo_root,
        &[
            "cat-file",
            "-e",
            &format!("{}^{{commit}}", manifest.base_commit),
        ],
        "verify base commit",
    )?;
    let destination_string = destination.to_string_lossy().into_owned();
    git_output(
        &repo_root,
        &[
            "worktree",
            "add",
            "--detach",
            &destination_string,
            &manifest.base_commit,
        ],
        "create detached import worktree",
    )?;

    let import_result = (|| {
        if !patch.is_empty() {
            let patch_path = staging.path().join("workspace.patch");
            let patch_string = patch_path.to_string_lossy().into_owned();
            git_output(
                destination,
                &["apply", "--3way", "--binary", &patch_string],
                "apply capsule patch",
            )?;
        }
        install_untracked(staging.path(), destination, &manifest.untracked)?;
        Ok(ImportedCapsule {
            worktree_path: destination.to_path_buf(),
            manifest,
            session_json,
        })
    })();

    if import_result.is_err() {
        let _ = Command::new("git")
            .current_dir(&repo_root)
            .args(["worktree", "remove", "--force", &destination_string])
            .output();
    }
    import_result
}

fn validate_workspace_file(
    repo_root: &Path,
    relative: &Path,
    max_file_bytes: u64,
    allow_deleted: bool,
    rejected: &mut Vec<UnsafePath>,
) -> bool {
    let shown = display_path(relative);
    if let Err(reason) = validate_relative_path(relative) {
        rejected.push(UnsafePath {
            path: shown,
            reason,
        });
        return false;
    }
    if let Some(reason) = sensitive_path_reason(relative) {
        rejected.push(UnsafePath {
            path: shown,
            reason: reason.into(),
        });
        return false;
    }
    let absolute = repo_root.join(relative);
    let metadata = match std::fs::symlink_metadata(&absolute) {
        Ok(metadata) => metadata,
        Err(error) if allow_deleted && error.kind() == std::io::ErrorKind::NotFound => return true,
        Err(error) => {
            rejected.push(UnsafePath {
                path: shown,
                reason: format!("cannot inspect file: {error}"),
            });
            return false;
        }
    };
    let kind = metadata.file_type();
    if kind.is_symlink() {
        rejected.push(UnsafePath {
            path: shown,
            reason: "symbolic links are not portable".into(),
        });
        return false;
    }
    if !kind.is_file() {
        rejected.push(UnsafePath {
            path: shown,
            reason: "only regular files are portable".into(),
        });
        return false;
    }
    if metadata.len() > max_file_bytes {
        rejected.push(UnsafePath {
            path: shown,
            reason: format!(
                "file is {} bytes; per-file limit is {max_file_bytes}",
                metadata.len()
            ),
        });
        return false;
    }
    true
}

fn validate_relative_path(path: &Path) -> Result<(), String> {
    if path.as_os_str().is_empty() || path.is_absolute() {
        return Err("path must be non-empty and relative".into());
    }
    for component in path.components() {
        match component {
            Component::Normal(name) if name != ".git" => {}
            Component::Normal(_) => return Err(".git paths are forbidden".into()),
            _ => return Err("absolute and traversal paths are forbidden".into()),
        }
    }
    Ok(())
}

fn sensitive_path_reason(path: &Path) -> Option<&'static str> {
    const BUILD_COMPONENTS: &[&str] = &[
        "target",
        "node_modules",
        ".cache",
        ".next",
        "dist",
        "build",
        "coverage",
    ];
    const SECRET_COMPONENTS: &[&str] = &[".ssh", ".gnupg", ".aws"];
    for component in path.components() {
        let Component::Normal(value) = component else {
            continue;
        };
        let lower = value.to_string_lossy().to_ascii_lowercase();
        if BUILD_COMPONENTS.contains(&lower.as_str()) {
            return Some("build output or cache paths are not portable");
        }
        if SECRET_COMPONENTS.contains(&lower.as_str()) {
            return Some("credential directory is forbidden");
        }
    }
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_ascii_lowercase())?;
    let safe_env =
        name.ends_with(".example") || name.ends_with(".sample") || name.ends_with(".template");
    if (name == ".env" || name.starts_with(".env.")) && !safe_env {
        return Some("environment secret file is forbidden");
    }
    if matches!(
        name.as_str(),
        "id_rsa"
            | "id_ed25519"
            | ".npmrc"
            | ".pypirc"
            | "credentials.json"
            | "secrets.json"
            | "secrets.toml"
            | "secrets.yaml"
            | "secrets.yml"
    ) || [".pem", ".key", ".p12", ".pfx"]
        .iter()
        .any(|extension| name.ends_with(extension))
    {
        return Some("credential or private-key file is forbidden");
    }
    None
}

fn write_archive(
    path: &Path,
    repo_root: &Path,
    manifest: &[u8],
    session: &[u8],
    patch: &[u8],
    files: &[CapsuleFile],
) -> Result<(), CapsuleError> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let output = options.open(path).map_err(|source| CapsuleError::Io {
        operation: "creating the capsule archive",
        source,
    })?;
    let encoder = GzEncoder::new(output, Compression::default());
    let mut archive = tar::Builder::new(encoder);
    append_bytes(&mut archive, "manifest.json", manifest)?;
    append_bytes(&mut archive, "session.json", session)?;
    append_bytes(&mut archive, "workspace.patch", patch)?;
    for file in files {
        let relative = Path::new(&file.path);
        let source_path = repo_root.join(relative);
        let mut source = open_nofollow(&source_path).map_err(|source| CapsuleError::Io {
            operation: "opening an untracked capsule file",
            source,
        })?;
        let mut header = tar::Header::new_gnu();
        header.set_size(file.size);
        header.set_mode(0o600);
        header.set_mtime(0);
        header.set_entry_type(tar::EntryType::Regular);
        header.set_cksum();
        archive
            .append_data(
                &mut header,
                Path::new("untracked").join(relative),
                &mut source,
            )
            .map_err(|source| CapsuleError::Io {
                operation: "archiving an untracked file",
                source,
            })?;
    }
    let encoder = archive.into_inner().map_err(|source| CapsuleError::Io {
        operation: "finalizing the capsule tar stream",
        source,
    })?;
    let file = encoder.finish().map_err(|source| CapsuleError::Io {
        operation: "finalizing capsule compression",
        source,
    })?;
    file.sync_all().map_err(|source| CapsuleError::Io {
        operation: "syncing the capsule archive",
        source,
    })
}

fn append_bytes(
    archive: &mut tar::Builder<GzEncoder<File>>,
    path: &str,
    bytes: &[u8],
) -> Result<(), CapsuleError> {
    let mut header = tar::Header::new_gnu();
    header.set_size(bytes.len() as u64);
    header.set_mode(0o600);
    header.set_mtime(0);
    header.set_entry_type(tar::EntryType::Regular);
    header.set_cksum();
    archive
        .append_data(&mut header, path, bytes)
        .map_err(|source| CapsuleError::Io {
            operation: "writing capsule metadata",
            source,
        })
}

fn extract_to_staging(
    capsule_path: &Path,
    staging: &Path,
    limits: CapsuleLimits,
) -> Result<(), CapsuleError> {
    let file = File::open(capsule_path).map_err(|source| CapsuleError::Io {
        operation: "opening the capsule archive",
        source,
    })?;
    let decoder = GzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);
    let mut expanded = 0_u64;
    let mut seen = std::collections::HashSet::new();
    for entry in archive.entries().map_err(|source| CapsuleError::Io {
        operation: "reading capsule entries",
        source,
    })? {
        let mut entry = entry.map_err(|source| CapsuleError::Io {
            operation: "reading a capsule entry",
            source,
        })?;
        if !entry.header().entry_type().is_file() {
            return Err(CapsuleError::InvalidArchive(
                "links, directories, and special entries are forbidden".into(),
            ));
        }
        let entry_path = entry
            .path()
            .map_err(|source| CapsuleError::Io {
                operation: "reading a capsule entry path",
                source,
            })?
            .into_owned();
        validate_relative_path(&entry_path).map_err(CapsuleError::InvalidArchive)?;
        if !seen.insert(entry_path.clone()) {
            return Err(CapsuleError::InvalidArchive(format!(
                "duplicate entry {}",
                entry_path.display()
            )));
        }
        let size = entry.size();
        let limit = archive_entry_limit(&entry_path, limits.max_file_bytes)?;
        if size > limit {
            return Err(CapsuleError::InvalidArchive(format!(
                "{} is {size} bytes; limit is {limit}",
                entry_path.display()
            )));
        }
        expanded = expanded
            .checked_add(size)
            .filter(|total| *total <= MAX_EXPANDED_BYTES)
            .ok_or_else(|| {
                CapsuleError::InvalidArchive("expanded capsule exceeds 500 MiB".into())
            })?;
        let target = staging.join(&entry_path);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|source| CapsuleError::Io {
                operation: "creating capsule staging paths",
                source,
            })?;
        }
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)
            .map_err(|source| CapsuleError::Io {
                operation: "creating a staged capsule file",
                source,
            })?;
        std::io::copy(&mut entry, &mut output).map_err(|source| CapsuleError::Io {
            operation: "extracting a staged capsule file",
            source,
        })?;
    }
    for required in ["manifest.json", "session.json", "workspace.patch"] {
        if !seen.contains(Path::new(required)) {
            return Err(CapsuleError::InvalidArchive(format!(
                "missing required entry {required}"
            )));
        }
    }
    Ok(())
}

fn archive_entry_limit(path: &Path, max_file_bytes: u64) -> Result<u64, CapsuleError> {
    match path.to_str() {
        Some("manifest.json") => Ok(MAX_METADATA_BYTES),
        Some("session.json") => Ok(MAX_SESSION_BYTES),
        Some("workspace.patch") => Ok(MAX_PATCH_BYTES),
        _ if path.starts_with("untracked") => {
            let relative = path.strip_prefix("untracked").map_err(|_| {
                CapsuleError::InvalidArchive("invalid untracked entry prefix".into())
            })?;
            validate_relative_path(relative).map_err(CapsuleError::InvalidArchive)?;
            if let Some(reason) = sensitive_path_reason(relative) {
                return Err(CapsuleError::InvalidArchive(format!(
                    "unsafe untracked path {}: {reason}",
                    relative.display()
                )));
            }
            Ok(max_file_bytes)
        }
        _ => Err(CapsuleError::InvalidArchive(format!(
            "unexpected archive entry {}",
            path.display()
        ))),
    }
}

fn verify_manifest_files(
    staging: &Path,
    manifest: &CapsuleManifest,
    limits: CapsuleLimits,
) -> Result<(), CapsuleError> {
    let mut expected = std::collections::HashSet::new();
    for file in &manifest.untracked {
        let relative = Path::new(&file.path);
        validate_relative_path(relative).map_err(CapsuleError::InvalidArchive)?;
        if sensitive_path_reason(relative).is_some() || file.size > limits.max_file_bytes {
            return Err(CapsuleError::InvalidArchive(format!(
                "unsafe manifest path {}",
                relative.display()
            )));
        }
        if !expected.insert(file.path.clone()) {
            return Err(CapsuleError::InvalidArchive(format!(
                "duplicate manifest path {}",
                file.path
            )));
        }
        let path = staging.join("untracked").join(relative);
        let metadata = std::fs::metadata(&path).map_err(|source| CapsuleError::Io {
            operation: "reading a staged untracked file",
            source,
        })?;
        if metadata.len() != file.size {
            return Err(CapsuleError::InvalidArchive(format!(
                "size mismatch for {}",
                file.path
            )));
        }
        verify_hash(
            &file.path,
            &read_limited(&path, limits.max_file_bytes)?,
            &file.sha256,
        )?;
    }
    let untracked_root = staging.join("untracked");
    if untracked_root.exists() {
        for path in walk_regular_files(&untracked_root)? {
            let relative = path
                .strip_prefix(&untracked_root)
                .map_err(|_| CapsuleError::InvalidArchive("invalid staged path".into()))?;
            if !expected.contains(&portable_path(relative)) {
                return Err(CapsuleError::InvalidArchive(format!(
                    "unmanifested file {}",
                    relative.display()
                )));
            }
        }
    }
    Ok(())
}

fn install_untracked(
    staging: &Path,
    destination: &Path,
    files: &[CapsuleFile],
) -> Result<(), CapsuleError> {
    for file in files {
        let relative = Path::new(&file.path);
        let source = staging.join("untracked").join(relative);
        #[cfg(unix)]
        install_untracked_file_unix(destination, relative, &source)?;
        #[cfg(not(unix))]
        install_untracked_file_portable(destination, relative, &source)?;
    }
    Ok(())
}

#[cfg(unix)]
fn install_untracked_file_unix(
    destination: &Path,
    relative: &Path,
    source: &Path,
) -> Result<(), CapsuleError> {
    use std::ffi::CString;
    use std::os::fd::{AsRawFd as _, FromRawFd as _, OwnedFd};
    use std::os::unix::ffi::OsStrExt as _;

    fn component(value: &std::ffi::OsStr) -> std::io::Result<CString> {
        CString::new(value.as_bytes())
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "path contains NUL"))
    }

    fn open_directory(path: &Path) -> std::io::Result<OwnedFd> {
        let path = CString::new(path.as_os_str().as_bytes()).map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "path contains NUL")
        })?;
        // SAFETY: `path` is a live C string and the returned descriptor is immediately owned.
        let fd = unsafe {
            libc::open(
                path.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            )
        };
        if fd < 0 {
            Err(std::io::Error::last_os_error())
        } else {
            // SAFETY: `fd` was freshly returned by `open` and has one owner.
            Ok(unsafe { OwnedFd::from_raw_fd(fd) })
        }
    }

    fn open_child_directory(parent: &OwnedFd, name: &std::ffi::OsStr) -> std::io::Result<OwnedFd> {
        let name = component(name)?;
        // SAFETY: `name` and `parent` remain live for the call; the returned fd is owned below.
        let mut fd = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            )
        };
        if fd < 0 && std::io::Error::last_os_error().kind() == std::io::ErrorKind::NotFound {
            // SAFETY: arguments are valid and mkdirat does not retain either pointer/fd.
            if unsafe { libc::mkdirat(parent.as_raw_fd(), name.as_ptr(), 0o700) } < 0 {
                let error = std::io::Error::last_os_error();
                if error.kind() != std::io::ErrorKind::AlreadyExists {
                    return Err(error);
                }
            }
            // SAFETY: same invariants as the first `openat` call.
            fd = unsafe {
                libc::openat(
                    parent.as_raw_fd(),
                    name.as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
                )
            };
        }
        if fd < 0 {
            Err(std::io::Error::last_os_error())
        } else {
            // SAFETY: `fd` was freshly returned by `openat` and has one owner.
            Ok(unsafe { OwnedFd::from_raw_fd(fd) })
        }
    }

    let mut directory = open_directory(destination).map_err(|source| CapsuleError::Io {
        operation: "opening the detached worktree without following links",
        source,
    })?;
    if let Some(parent) = relative.parent() {
        for part in parent.components() {
            let std::path::Component::Normal(name) = part else {
                return Err(CapsuleError::InvalidArchive(
                    "untracked path contains a non-normal component".into(),
                ));
            };
            directory =
                open_child_directory(&directory, name).map_err(|source| CapsuleError::Io {
                    operation: "opening imported untracked directories without following links",
                    source,
                })?;
        }
    }
    let name = relative
        .file_name()
        .ok_or_else(|| CapsuleError::InvalidArchive("untracked path has no file name".into()))?;
    let name = component(name).map_err(|source| CapsuleError::Io {
        operation: "validating an imported untracked file name",
        source,
    })?;
    // SAFETY: the directory and C string remain live; O_NOFOLLOW/O_EXCL prevent link traversal
    // and replacement of an existing patch-created path.
    let fd = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            name.as_ptr(),
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            0o600,
        )
    };
    if fd < 0 {
        return Err(CapsuleError::Io {
            operation: "creating an imported untracked file without following links",
            source: std::io::Error::last_os_error(),
        });
    }
    // SAFETY: `fd` was freshly returned by `openat` and has one owner.
    let mut output = unsafe { File::from_raw_fd(fd) };
    let mut input = open_nofollow(source).map_err(|source| CapsuleError::Io {
        operation: "opening a staged untracked file",
        source,
    })?;
    std::io::copy(&mut input, &mut output).map_err(|source| CapsuleError::Io {
        operation: "copying an imported untracked file",
        source,
    })?;
    Ok(())
}

#[cfg(not(unix))]
fn install_untracked_file_portable(
    destination: &Path,
    relative: &Path,
    source: &Path,
) -> Result<(), CapsuleError> {
    let target = destination.join(relative);
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|source| CapsuleError::Io {
            operation: "creating imported untracked directories",
            source,
        })?;
        let mut cursor = destination.to_path_buf();
        for component in relative.parent().into_iter().flat_map(Path::components) {
            cursor.push(component);
            if std::fs::symlink_metadata(&cursor)
                .map_err(|source| CapsuleError::Io {
                    operation: "validating imported untracked directories",
                    source,
                })?
                .file_type()
                .is_symlink()
            {
                return Err(CapsuleError::InvalidArchive(
                    "untracked parent is a symbolic link".into(),
                ));
            }
        }
    }
    let mut input = open_nofollow(source).map_err(|source| CapsuleError::Io {
        operation: "opening a staged untracked file",
        source,
    })?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&target)
        .map_err(|source| CapsuleError::Io {
            operation: "creating an imported untracked file",
            source,
        })?;
    std::io::copy(&mut input, &mut output).map_err(|source| CapsuleError::Io {
        operation: "copying an imported untracked file",
        source,
    })?;
    Ok(())
}

fn walk_regular_files(root: &Path) -> Result<Vec<PathBuf>, CapsuleError> {
    let mut pending = vec![root.to_path_buf()];
    let mut files = Vec::new();
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(&directory).map_err(|source| CapsuleError::Io {
            operation: "walking staged capsule files",
            source,
        })? {
            let entry = entry.map_err(|source| CapsuleError::Io {
                operation: "walking staged capsule files",
                source,
            })?;
            let kind = entry.file_type().map_err(|source| CapsuleError::Io {
                operation: "inspecting a staged capsule file",
                source,
            })?;
            if kind.is_dir() {
                pending.push(entry.path());
            } else if kind.is_file() {
                files.push(entry.path());
            } else {
                return Err(CapsuleError::InvalidArchive(
                    "staging contains a non-regular entry".into(),
                ));
            }
        }
    }
    Ok(files)
}

fn read_limited(path: &Path, limit: u64) -> Result<Vec<u8>, CapsuleError> {
    let metadata = std::fs::metadata(path).map_err(|source| CapsuleError::Io {
        operation: "reading a staged capsule entry",
        source,
    })?;
    if metadata.len() > limit {
        return Err(CapsuleError::InvalidArchive(format!(
            "{} exceeds its {limit} byte limit",
            path.display()
        )));
    }
    std::fs::read(path).map_err(|source| CapsuleError::Io {
        operation: "reading a staged capsule entry",
        source,
    })
}

fn verify_hash(label: &str, bytes: &[u8], expected: &str) -> Result<(), CapsuleError> {
    if hash_bytes(bytes) == expected {
        Ok(())
    } else {
        Err(CapsuleError::InvalidArchive(format!(
            "SHA-256 mismatch for {label}"
        )))
    }
}

fn hash_file_nofollow(path: &Path) -> Result<String, CapsuleError> {
    let mut file = open_nofollow(path).map_err(|source| CapsuleError::Io {
        operation: "opening a capsule file for hashing",
        source,
    })?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|source| CapsuleError::Io {
            operation: "hashing a capsule file",
            source,
        })?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(hex::encode(digest.finalize()))
}

#[cfg(unix)]
fn open_nofollow(path: &Path) -> std::io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt as _;
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
}

#[cfg(not(unix))]
fn open_nofollow(path: &Path) -> std::io::Result<File> {
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "not a regular file",
        ));
    }
    OpenOptions::new().read(true).open(path)
}

fn hash_bytes(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn git_zero_paths(
    repo_root: &Path,
    args: &[&str],
    operation: &'static str,
) -> Result<Vec<PathBuf>, CapsuleError> {
    let bytes = git_bytes(repo_root, args, operation)?;
    Ok(bytes
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(|path| PathBuf::from(String::from_utf8_lossy(path).into_owned()))
        .collect())
}

fn git_text(
    repo_root: &Path,
    args: &[&str],
    operation: &'static str,
) -> Result<String, CapsuleError> {
    let output = git_output(repo_root, args, operation)?;
    String::from_utf8(output.stdout)
        .map(|text| text.trim().to_owned())
        .map_err(|_| CapsuleError::Git {
            operation,
            details: "git output was not UTF-8".into(),
        })
}

fn git_bytes(
    repo_root: &Path,
    args: &[&str],
    operation: &'static str,
) -> Result<Vec<u8>, CapsuleError> {
    Ok(git_output(repo_root, args, operation)?.stdout)
}

fn git_output(
    repo_root: &Path,
    args: &[&str],
    operation: &'static str,
) -> Result<Output, CapsuleError> {
    let output = Command::new("git")
        .current_dir(repo_root)
        .args(args)
        .output()
        .map_err(|source| CapsuleError::Io {
            operation: "starting git",
            source,
        })?;
    if output.status.success() {
        Ok(output)
    } else {
        Err(CapsuleError::Git {
            operation,
            details: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        })
    }
}

fn validate_commit(commit: &str) -> Result<(), CapsuleError> {
    if (commit.len() == 40 || commit.len() == 64)
        && commit.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        Ok(())
    } else {
        Err(CapsuleError::InvalidBase(commit.to_owned()))
    }
}

fn portable_path(path: &Path) -> String {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(value) => Some(value.to_string_lossy()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn display_path(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn rand_suffix() -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    now_ms().hash(&mut hasher);
    std::process::id().hash(&mut hasher);
    std::thread::current().id().hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(repo: &Path, args: &[&str]) {
        let output = Command::new("git")
            .current_dir(repo)
            .args(args)
            .output()
            .expect("start git");
        assert!(
            output.status.success(),
            "git {:?}: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn repository() -> tempfile::TempDir {
        let temp = tempfile::tempdir().expect("temp repository");
        git(temp.path(), &["init", "-q"]);
        git(temp.path(), &["config", "user.email", "forge@example.test"]);
        git(temp.path(), &["config", "user.name", "Forge Test"]);
        std::fs::write(temp.path().join("tracked.bin"), [0_u8, 1, 2, 3]).expect("write tracked");
        git(temp.path(), &["add", "tracked.bin"]);
        git(temp.path(), &["commit", "-qm", "base"]);
        temp
    }

    fn archive(path: &Path, entries: &[(&str, &[u8])]) {
        let file = File::create(path).expect("create test capsule");
        let encoder = GzEncoder::new(file, Compression::default());
        let mut builder = tar::Builder::new(encoder);
        for (name, bytes) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_mode(0o600);
            header.set_size(bytes.len() as u64);
            header.set_path(name).expect("set archive path");
            header.set_cksum();
            builder
                .append(&header, *bytes)
                .expect("append archive entry");
        }
        let encoder = builder.into_inner().expect("finish tar");
        encoder.finish().expect("finish gzip");
    }

    #[test]
    fn binary_patch_and_untracked_file_round_trip() {
        let repo = repository();
        std::fs::write(repo.path().join("tracked.bin"), [0_u8, 9, 2, 3, 0xff])
            .expect("modify binary");
        std::fs::create_dir(repo.path().join("notes")).expect("notes dir");
        std::fs::write(repo.path().join("notes/todo.txt"), "portable\n").expect("untracked file");
        let capsule = repo
            .path()
            .parent()
            .expect("parent")
            .join(format!("capsule-{:016x}.fany", rand_suffix()));
        let exported = export_capsule(
            repo.path(),
            &capsule,
            "session-1",
            br#"{"session":"portable"}"#,
            CapsuleLimits::default(),
        )
        .expect("export capsule");
        assert!(exported.compressed_bytes > 0);

        let destination = repo
            .path()
            .parent()
            .expect("parent")
            .join(format!("import-{:016x}", rand_suffix()));
        let imported = import_capsule(
            repo.path(),
            &capsule,
            &destination,
            CapsuleLimits::default(),
        )
        .expect("import capsule");
        assert_eq!(
            std::fs::read(destination.join("tracked.bin")).expect("imported tracked"),
            [0_u8, 9, 2, 3, 0xff]
        );
        assert_eq!(
            std::fs::read_to_string(destination.join("notes/todo.txt"))
                .expect("imported untracked"),
            "portable\n"
        );
        assert_eq!(imported.session_json, br#"{"session":"portable"}"#);
        git(
            repo.path(),
            &[
                "worktree",
                "remove",
                "--force",
                &destination.to_string_lossy(),
            ],
        );
        std::fs::remove_file(capsule).expect("remove test capsule");
    }

    #[test]
    fn secret_file_aborts_without_creating_a_capsule() {
        let repo = repository();
        std::fs::write(repo.path().join(".env"), "TOKEN=secret\n").expect("secret file");
        let capsule = repo.path().join("unsafe.fany");
        let error = export_capsule(
            repo.path(),
            &capsule,
            "session-1",
            b"{}",
            CapsuleLimits::default(),
        )
        .expect_err("secret must reject export");
        assert!(matches!(error, CapsuleError::UnsafeFiles(_)));
        assert!(!capsule.exists());
    }

    #[cfg(unix)]
    #[test]
    fn symbolic_link_aborts_without_following_it() {
        use std::os::unix::fs::symlink;

        let repo = repository();
        symlink("tracked.bin", repo.path().join("linked.bin")).expect("create symlink");
        let capsule = repo.path().join("unsafe-link.fany");
        let error = export_capsule(
            repo.path(),
            &capsule,
            "session-1",
            b"{}",
            CapsuleLimits::default(),
        )
        .expect_err("symlink must reject export");
        assert!(matches!(error, CapsuleError::UnsafeFiles(_)));
        assert!(!capsule.exists());
    }

    #[test]
    fn invalid_patch_removes_the_detached_worktree() {
        let repo = repository();
        let base_commit = git_text(repo.path(), &["rev-parse", "HEAD"], "test head").unwrap();
        let session = br#"{"session":"portable"}"#;
        let patch = b"this is not a git patch\n";
        let manifest = CapsuleManifest {
            version: 1,
            session_id: "session-conflict".into(),
            base_commit,
            repository: repository_metadata(repo.path()),
            created_at_ms: now_ms(),
            session_sha256: hash_bytes(session),
            patch_sha256: hash_bytes(patch),
            untracked: Vec::new(),
        };
        let manifest = serde_json::to_vec(&manifest).unwrap();
        let capsule = repo.path().join("conflict.fany");
        archive(
            &capsule,
            &[
                ("manifest.json", &manifest),
                ("session.json", session),
                ("workspace.patch", patch),
            ],
        );
        let destination = repo.path().join("failed-import");
        let error = import_capsule(
            repo.path(),
            &capsule,
            &destination,
            CapsuleLimits::default(),
        )
        .expect_err("invalid patch must fail");
        assert!(matches!(
            error,
            CapsuleError::Git {
                operation: "apply capsule patch",
                ..
            }
        ));
        assert!(!destination.exists(), "failed worktree must be rolled back");
    }

    #[test]
    fn repository_url_credentials_are_never_preserved() {
        assert_eq!(
            strip_url_userinfo("https://token:secret@example.test/owner/repo.git"),
            "https://example.test/owner/repo.git"
        );
        assert_eq!(
            strip_url_userinfo("git@example.test:owner/repo.git"),
            "git@example.test:owner/repo.git"
        );
        let canonical = canonical_remote_identity("https://Token@Example.Test/Owner/Repo.git/");
        assert_eq!(canonical, "example.test/owner/repo");
        assert_eq!(
            canonical_remote_identity("ssh://git@example.test/owner/repo.git"),
            canonical
        );
        assert_eq!(
            canonical_remote_identity("git@EXAMPLE.TEST:OWNER/REPO.git"),
            canonical
        );
    }

    #[cfg(unix)]
    #[test]
    fn patch_created_symlink_parent_cannot_escape_worktree() {
        use std::os::unix::fs::symlink;

        let repo = repository();
        let outside = tempfile::tempdir().unwrap();
        symlink(outside.path(), repo.path().join("link")).unwrap();
        git(repo.path(), &["add", "link"]);
        let patch = git_bytes(
            repo.path(),
            &["diff", "--cached", "--binary", "--full-index", "HEAD"],
            "build symlink patch",
        )
        .unwrap();
        git(repo.path(), &["reset", "--hard", "-q", "HEAD"]);
        let _ = std::fs::remove_file(repo.path().join("link"));

        let session = br#"{"session":"portable"}"#;
        let payload = b"must stay contained";
        let manifest = CapsuleManifest {
            version: 1,
            session_id: "session-link-escape".into(),
            base_commit: git_text(repo.path(), &["rev-parse", "HEAD"], "test head").unwrap(),
            repository: repository_metadata(repo.path()),
            created_at_ms: now_ms(),
            session_sha256: hash_bytes(session),
            patch_sha256: hash_bytes(&patch),
            untracked: vec![CapsuleFile {
                path: "link/owned.txt".into(),
                size: payload.len() as u64,
                sha256: hash_bytes(payload),
            }],
        };
        let manifest = serde_json::to_vec(&manifest).unwrap();
        let capsule = repo.path().join("symlink-parent.fany");
        archive(
            &capsule,
            &[
                ("manifest.json", &manifest),
                ("session.json", session),
                ("workspace.patch", &patch),
                ("untracked/link/owned.txt", payload),
            ],
        );
        let destination = repo.path().join("symlink-parent-import");
        import_capsule(
            repo.path(),
            &capsule,
            &destination,
            CapsuleLimits::default(),
        )
        .expect_err("patch-created symlink parent must be rejected");
        assert!(!outside.path().join("owned.txt").exists());
        assert!(!destination.exists());
    }

    #[test]
    fn traversal_archive_is_rejected_before_git_is_touched() {
        let repo = repository();
        let capsule = repo.path().join("traversal.fany");
        let file = File::create(&capsule).unwrap();
        let encoder = GzEncoder::new(file, Compression::default());
        let mut builder = tar::Builder::new(encoder);
        let mut header = tar::Header::new_gnu();
        header.set_mode(0o600);
        header.set_size(1);
        header.set_path("placeholder").unwrap();
        let raw = header.as_mut_bytes();
        raw[..100].fill(0);
        raw[..9].copy_from_slice(b"../escape");
        header.set_cksum();
        builder.append(&header, &b"x"[..]).unwrap();
        let encoder = builder.into_inner().unwrap();
        encoder.finish().unwrap();

        let destination = repo.path().join("traversal-import");
        let error = import_capsule(
            repo.path(),
            &capsule,
            &destination,
            CapsuleLimits::default(),
        )
        .expect_err("traversal must fail");
        assert!(matches!(error, CapsuleError::InvalidArchive(_)));
        assert!(!destination.exists());
        assert!(!repo.path().join("escape").exists());
    }
}
