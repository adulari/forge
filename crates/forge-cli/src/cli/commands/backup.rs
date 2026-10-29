//! `forge backup`: copy the session store somewhere safe, list the copies Forge keeps, and put
//! one back.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use forge_store::db_safety;

use crate::cli::args::host::BackupCmd;
use crate::BackupArgs;

pub(crate) fn backup_cmd(args: BackupArgs) -> Result<()> {
    let db = crate::store_path();
    match args.cmd {
        None => make(&db, args.out.map(PathBuf::from)),
        Some(BackupCmd::List) => list(&db),
        Some(BackupCmd::Restore { path }) => restore(&db, Path::new(&path)),
    }
}

fn default_destination(db: &Path) -> PathBuf {
    let dir = db.parent().unwrap_or(Path::new(".")).join("backups");
    let name = db.file_stem().map_or_else(
        || "forge".to_string(),
        |stem| stem.to_string_lossy().into_owned(),
    );
    dir.join(format!(
        "{name}-{}.db",
        chrono::Local::now().format("%Y%m%d-%H%M%S")
    ))
}

fn make(db: &Path, out: Option<PathBuf>) -> Result<()> {
    if !db.exists() {
        bail!("there is no session store at {} yet", db.display());
    }
    let dest = out.unwrap_or_else(|| default_destination(db));
    db_safety::backup_file(db, &dest).context("copying the session store")?;
    let bytes = std::fs::metadata(&dest).map_or(0, |m| m.len());
    println!("✔ backed up {} ({})", dest.display(), human_bytes(bytes));
    println!("  restore it with: forge backup restore {}", dest.display());
    Ok(())
}

fn list(db: &Path) -> Result<()> {
    let kept = db_safety::list_backups(db);
    let manual_dir = db.parent().unwrap_or(Path::new(".")).join("backups");
    let manual: Vec<_> = std::fs::read_dir(&manual_dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "db"))
        .collect();
    if kept.is_empty() && manual.is_empty() {
        println!("no backups yet. `forge backup` makes one.");
        return Ok(());
    }
    for copy in &kept {
        println!(
            "  {}  before schema v{}  {}",
            copy.path.display(),
            copy.before_version,
            human_bytes(copy.bytes)
        );
    }
    for entry in manual {
        let bytes = entry.metadata().map_or(0, |m| m.len());
        println!("  {}  {}", entry.path().display(), human_bytes(bytes));
    }
    Ok(())
}

fn restore(db: &Path, source: &Path) -> Result<()> {
    if let Some(dir) = forge_config::config_dir() {
        if let Err(error) = crate::host_lock::acquire(&dir) {
            bail!("{error:#}\n  stop the daemon before restoring a backup");
        }
    }
    let notice = db_safety::restore_file(db, source).context("restoring the backup")?;
    println!("✔ restored {}", source.display());
    if !notice.quarantined.is_empty() {
        println!("  the store it replaced is kept at {}", notice.quarantined);
    }
    Ok(())
}

fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_read_in_the_largest_unit_that_fits() {
        assert_eq!(human_bytes(900), "900 B");
        assert_eq!(human_bytes(1536), "1.5 KB");
        assert_eq!(human_bytes(5 * 1024 * 1024), "5.0 MB");
    }

    #[test]
    fn the_default_copy_lands_in_a_backups_folder_beside_the_store() {
        let dest = default_destination(Path::new("/data/forge/forge.db"));
        assert_eq!(dest.parent().unwrap(), Path::new("/data/forge/backups"));
        assert!(dest
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("forge-"));
    }

    #[test]
    fn backing_up_and_restoring_round_trips_a_store() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("forge.db");
        drop(forge_store::Store::open(&db).unwrap());
        let out = dir.path().join("snap.db");
        make(&db, Some(out.clone())).unwrap();
        assert!(db_safety::quick_check(&out) == db_safety::Integrity::Ok);
        assert!(make(&dir.path().join("none.db"), None).is_err());
    }
}
