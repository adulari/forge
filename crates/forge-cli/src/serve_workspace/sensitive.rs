//! Files the outside-workspace read must never serve, whatever the transcript says: credentials,
//! key material, Forge's own config and store, and repository internals. A model (or a page it
//! fetched) can name any path, so naming a path is not permission to read it.

use std::path::{Path, PathBuf};

const SECRET_DIRS_UNDER_HOME: &[&str] = &[
    ".ssh",
    ".gnupg",
    ".aws",
    ".azure",
    ".kube",
    ".docker",
    ".password-store",
    ".config/forge",
    ".config/gcloud",
    ".config/gh",
    ".local/share/forge",
    ".local/share/keyrings",
];

const SECRET_ABSOLUTE: &[&str] = &[
    "/etc/shadow",
    "/etc/gshadow",
    "/etc/sudoers",
    "/etc/ssh",
    "/proc",
    "/sys",
];

const SECRET_EXTENSIONS: &[&str] = &[
    "pem", "key", "p12", "pfx", "kdbx", "db", "sqlite", "sqlite3", "db-wal", "db-shm",
];

const SECRET_FILE_NAMES: &[&str] = &[
    ".netrc",
    ".npmrc",
    ".pgpass",
    ".git-credentials",
    "credentials",
    "credentials.json",
];

fn secret_roots(home: Option<&Path>) -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = SECRET_ABSOLUTE.iter().map(PathBuf::from).collect();
    if let Some(home) = home {
        roots.extend(SECRET_DIRS_UNDER_HOME.iter().map(|dir| home.join(dir)));
    }
    roots.extend(forge_config::config_dir());
    roots.extend(forge_config::data_dir());
    if let Some(db) = std::env::var_os("FORGE_DB") {
        roots.push(PathBuf::from(db));
    }
    roots
}

fn looks_sensitive(path: &Path, roots: &[PathBuf]) -> bool {
    if roots.iter().any(|root| path.starts_with(root)) {
        return true;
    }
    if path.components().any(|part| part.as_os_str() == ".git") {
        return true;
    }
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    if name.starts_with(".env") || name.starts_with("id_") {
        return true;
    }
    if SECRET_FILE_NAMES.contains(&name.as_str()) {
        return true;
    }
    path.extension()
        .map(|ext| ext.to_string_lossy().to_ascii_lowercase())
        .is_some_and(|ext| SECRET_EXTENSIONS.contains(&ext.as_str()))
}

/// Whether `path` (as named, and after resolving symlinks) is off limits.
pub(super) fn is_sensitive(path: &Path, home: Option<&Path>) -> bool {
    let mut roots = secret_roots(home);
    let resolved = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    // A symlinked home or data dir must match both spellings.
    let canonical_roots: Vec<PathBuf> = roots.iter().map(|root| resolved(root)).collect();
    roots.extend(canonical_roots);
    looks_sensitive(path, &roots) || looks_sensitive(&resolved(path), &roots)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credentials_keys_forge_state_and_git_internals_are_sensitive() {
        let home = Path::new("/home/u");
        for path in [
            "/home/u/.ssh/id_ed25519",
            "/home/u/.config/forge/config.toml",
            "/home/u/.local/share/forge/forge.db",
            "/home/u/.local/share/keyrings/login.keyring",
            "/home/u/.aws/credentials",
            "/srv/app/.env",
            "/srv/app/.env.production",
            "/srv/app/tls/server.pem",
            "/other/repo/.git/config",
            "/etc/shadow",
            "/proc/self/environ",
        ] {
            assert!(is_sensitive(Path::new(path), Some(home)), "{path}");
        }
        for path in ["/home/u/notes/SOLVER.md", "/srv/app/README.md"] {
            assert!(!is_sensitive(Path::new(path), Some(home)), "{path}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_to_a_secret_is_still_sensitive() {
        let dir = tempfile::tempdir().unwrap();
        let secret = dir.path().join(".env");
        std::fs::write(&secret, "K=v").unwrap();
        let link = dir.path().join("notes.md");
        std::os::unix::fs::symlink(&secret, &link).unwrap();
        assert!(is_sensitive(&link, None));
    }
}
