//! Interpreter discovery: file and PATH lookup only, never a spawn.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

const CONDA_PYTHON: &str = "/opt/conda/bin/python";

/// Candidates in order: the workspace venv, PATH `python3`, PATH `python`, conda. Duplicates
/// (by canonical path) keep their first position.
pub fn discover_python(workspace: &Path) -> Vec<PathBuf> {
    let path = std::env::var_os("PATH");
    discover_in(workspace, path.as_deref(), Path::new(CONDA_PYTHON))
}

pub(super) fn discover_in(workspace: &Path, path: Option<&OsStr>, conda: &Path) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = vec![workspace.join(".venv/bin/python")];
    for name in ["python3", "python"] {
        let hit = path
            .map(|p| std::env::split_paths(p).collect::<Vec<_>>())
            .unwrap_or_default()
            .into_iter()
            .map(|dir| dir.join(name))
            .find(|candidate| candidate.is_file());
        found.extend(hit);
    }
    found.push(conda.to_path_buf());
    let mut seen = Vec::new();
    found
        .into_iter()
        .filter(|p| p.is_file())
        .filter(|p| {
            let canonical = p.canonicalize().unwrap_or_else(|_| p.clone());
            if seen.contains(&canonical) {
                false
            } else {
                seen.push(canonical);
                true
            }
        })
        .collect()
}
