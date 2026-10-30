//! Per-skill invocation counts, kept in `skill-usage.json` under the data directory.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

static WRITE_LOCK: Mutex<()> = Mutex::new(());

fn usage_path() -> Option<PathBuf> {
    crate::data_dir().map(|d| d.join("skill-usage.json"))
}

pub fn read_counts_at(path: &Path) -> BTreeMap<String, u64> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

pub fn record_use_at(path: &Path, name: &str) {
    let _guard = WRITE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut counts = read_counts_at(path);
    *counts.entry(name.to_string()).or_insert(0) += 1;
    let Ok(body) = serde_json::to_string(&counts) else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let tmp = path.with_extension("json.tmp");
    if std::fs::write(&tmp, body).is_ok() {
        let _ = std::fs::rename(&tmp, path);
    }
}

/// Count one invocation of `name`; a failed write never disturbs the turn that triggered it.
pub fn record_skill_use(name: &str) {
    if let Some(path) = usage_path() {
        record_use_at(&path, name);
    }
}

pub fn skill_use_counts() -> BTreeMap<String, u64> {
    usage_path().map(|p| read_counts_at(&p)).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_accumulate_per_skill() {
        let dir = std::env::temp_dir().join(format!("forge-usage-{}", std::process::id()));
        let path = dir.join("skill-usage.json");
        std::fs::remove_dir_all(&dir).ok();
        record_use_at(&path, "rust");
        record_use_at(&path, "rust");
        record_use_at(&path, "debug");
        let counts = read_counts_at(&path);
        assert_eq!(counts["rust"], 2);
        assert_eq!(counts["debug"], 1);
        std::fs::remove_dir_all(&dir).ok();
    }
}
