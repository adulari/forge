//! Keeping the session store alive across bad upgrades and bad disks.
//!
//! A migration runs in place, so a build that carries the wrong schema (or a crash halfway) used
//! to leave the only copy of every transcript changed or unreadable. Before a pending migration
//! the file is copied to `<db>.pre-v<N>.bak` (the two newest are kept). At daemon start
//! [`quick_check`] looks for corruption, and [`quarantine_and_restore`] moves a damaged file
//! aside and puts the newest backup that still reads in its place. What happened is written to
//! `<db>.recovery.json` so the diagnostics screen can say so.

use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};

use crate::{Result, Store, StoreError, SCHEMA_VERSION};

/// How many pre-migration copies are kept.
const BACKUPS_KEPT: usize = 2;

/// How long a copy counts as belonging to the migration attempt that is still under way.
const SAME_ATTEMPT_WINDOW: std::time::Duration = std::time::Duration::from_secs(600);

/// Problems `quick_check` names in a report; SQLite can list one per page of a large store.
const SUMMARISED_PROBLEMS: usize = 3;

/// Pages copied per step of the online backup, and the pause between steps so a live writer is
/// not starved while a large store is copied.
const BACKUP_PAGES_PER_STEP: std::ffi::c_int = 512;
const BACKUP_STEP_PAUSE: std::time::Duration = std::time::Duration::from_millis(5);

/// The schema version this build migrates a store to.
pub fn current_schema_version() -> i64 {
    SCHEMA_VERSION
}

/// One pre-migration copy on disk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BackupInfo {
    pub path: PathBuf,
    /// The schema version this copy was taken before migrating to.
    pub before_version: i64,
    pub bytes: u64,
    /// Unix seconds.
    pub modified: i64,
}

/// What the integrity check found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Integrity {
    Ok,
    /// SQLite reports damage (or the file is not a database at all).
    Corrupt(String),
    /// The check could not run (locked, unreadable); not evidence of damage.
    Unknown(String),
}

/// What [`quarantine_and_restore`] / [`restore_backup`] did, persisted for the diagnostics screen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryNotice {
    /// Unix seconds.
    pub at: i64,
    /// `restored` (a backup is back in place) or `fresh_start` (nothing usable to restore).
    pub outcome: String,
    pub reason: String,
    /// The damaged file, kept for manual salvage.
    pub quarantined: String,
    pub restored_from: Option<String>,
}

fn sibling(db: &Path, suffix: &str) -> PathBuf {
    let mut name = db.file_name().unwrap_or_default().to_os_string();
    name.push(suffix);
    db.with_file_name(name)
}

fn backup_path(db: &Path, before_version: i64) -> PathBuf {
    sibling(db, &format!(".pre-v{before_version}.bak"))
}

fn notice_path(db: &Path) -> PathBuf {
    sibling(db, ".recovery.json")
}

fn open_read_only(path: &Path) -> rusqlite::Result<Connection> {
    Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
}

/// Open a file nothing else is writing (a backup) so SQLite leaves no `-wal`/`-shm` beside it.
fn open_immutable(path: &Path) -> rusqlite::Result<Connection> {
    let mut uri = String::from("file:");
    for byte in path.to_string_lossy().bytes() {
        match byte {
            b'%' | b'?' | b'#' | b' ' => uri.push_str(&format!("%{byte:02X}")),
            _ => uri.push(byte as char),
        }
    }
    uri.push_str("?immutable=1");
    Connection::open_with_flags(
        uri,
        OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_URI
            | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
}

fn user_version_and_tables(path: &Path) -> rusqlite::Result<(i64, i64)> {
    let conn = open_read_only(path)?;
    let version = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    let tables = conn.query_row("SELECT count(*) FROM sqlite_master", [], |row| row.get(0))?;
    Ok((version, tables))
}

fn copy_database(src: &Connection, dest: &Path) -> rusqlite::Result<()> {
    let mut out = Connection::open(dest)?;
    let backup = rusqlite::backup::Backup::new(src, &mut out)?;
    backup.run_to_completion(BACKUP_PAGES_PER_STEP, BACKUP_STEP_PAUSE, None)
}

fn restrict(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    let _ = path;
}

/// Snapshot `src` to `dest` with SQLite's online backup, so the copy is consistent while the
/// store is in use. Written to a temporary name first, so `dest` is never a half-written file.
fn snapshot_connection(src: &Connection, dest: &Path) -> Result<()> {
    let tmp = sibling(dest, ".partial");
    let _ = std::fs::remove_file(&tmp);
    let outcome = copy_database(src, &tmp);
    if let Err(error) = outcome {
        let _ = std::fs::remove_file(&tmp);
        return Err(error.into());
    }
    restrict(&tmp);
    std::fs::rename(&tmp, dest).map_err(|e| StoreError::InvalidValue(format!("{e}")))?;
    Ok(())
}

/// Copy a pending-migration store to `<db>.pre-v<target>.bak` and drop all but the newest two.
/// `None` when nothing is pending: no file, an empty file, or a store already at (or past) this
/// build's schema. Never touches the original.
pub fn backup_before_migration(db: &Path) -> Result<Option<PathBuf>> {
    if std::fs::metadata(db).map(|m| m.len()).unwrap_or(0) == 0 {
        return Ok(None);
    }
    let (version, tables) = match user_version_and_tables(db) {
        Ok(found) => found,
        Err(_) => return Ok(None),
    };
    // Version 0 with tables is a store whose first open is still creating its schema, not one
    // with history worth keeping.
    if tables == 0 || version == 0 || version >= SCHEMA_VERSION {
        return Ok(None);
    }
    let dest = backup_path(db, SCHEMA_VERSION);
    if taken_recently(&dest) {
        return Ok(None);
    }
    let src = open_read_only(db)?;
    snapshot_connection(&src, &dest)?;
    prune_backups(db);
    Ok(Some(dest))
}

/// A copy for this same migration was taken moments ago. A second process opening the store while
/// the first is still migrating it would otherwise replace the true pre-migration copy with a
/// half-migrated one.
fn taken_recently(dest: &Path) -> bool {
    std::fs::metadata(dest)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .is_some_and(|age| age < SAME_ATTEMPT_WINDOW)
}

fn prune_backups(db: &Path) {
    let mut found = list_backups(db);
    found.sort_by_key(|b| std::cmp::Reverse((b.modified, b.before_version)));
    for stale in found.into_iter().skip(BACKUPS_KEPT) {
        let _ = std::fs::remove_file(stale.path);
    }
}

/// Every `<db>.pre-v<N>.bak` beside `db`, newest first.
pub fn list_backups(db: &Path) -> Vec<BackupInfo> {
    let Some(dir) = db.parent().map(|p| {
        if p.as_os_str().is_empty() {
            Path::new(".")
        } else {
            p
        }
    }) else {
        return Vec::new();
    };
    let prefix = format!(
        "{}.pre-v",
        db.file_name().unwrap_or_default().to_string_lossy()
    );
    let mut found: Vec<BackupInfo> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let version = name
                .strip_prefix(&prefix)?
                .strip_suffix(".bak")?
                .parse::<i64>()
                .ok()?;
            let meta = entry.metadata().ok()?;
            let modified = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_secs() as i64);
            Some(BackupInfo {
                path: entry.path(),
                before_version: version,
                bytes: meta.len(),
                modified,
            })
        })
        .collect();
    found.sort_by_key(|b| std::cmp::Reverse((b.modified, b.before_version)));
    found
}

/// Run SQLite's `quick_check` on `db`. Lock contention and unreadable files report
/// [`Integrity::Unknown`], never damage, so a busy store is not mistaken for a corrupt one.
pub fn quick_check(db: &Path) -> Integrity {
    check_connection(open_read_only(db))
}

/// [`quick_check`] for a file that is not in use (a backup), which it leaves untouched.
fn quick_check_idle(file: &Path) -> Integrity {
    check_connection(open_immutable(file))
}

fn check_connection(opened: rusqlite::Result<Connection>) -> Integrity {
    let conn = match opened {
        Ok(conn) => conn,
        Err(error) => return classify(error),
    };
    let mut stmt = match conn.prepare("PRAGMA quick_check") {
        Ok(stmt) => stmt,
        Err(error) => return classify(error),
    };
    let rows = match stmt.query_map([], |row| row.get::<_, String>(0)) {
        Ok(rows) => rows,
        Err(error) => return classify(error),
    };
    let mut problems: Vec<String> = Vec::new();
    let mut total = 0;
    for row in rows {
        match row {
            Ok(text) if text == "ok" => {}
            Ok(text) => {
                total += 1;
                if problems.len() < SUMMARISED_PROBLEMS {
                    let body = text.replace("*** in database main ***\n", "");
                    problems.push(body.replace('\n', " "));
                }
            }
            Err(error) => return classify(error),
        }
    }
    if total == 0 {
        return Integrity::Ok;
    }
    let more = total - problems.len();
    let tail = if more > 0 {
        format!(" (and {more} more)")
    } else {
        String::new()
    };
    Integrity::Corrupt(format!("{}{tail}", problems.join("; ")))
}

fn classify(error: rusqlite::Error) -> Integrity {
    use rusqlite::ErrorCode::{DatabaseCorrupt, NotADatabase};
    match error.sqlite_error_code() {
        Some(DatabaseCorrupt | NotADatabase) => Integrity::Corrupt(error.to_string()),
        _ => Integrity::Unknown(error.to_string()),
    }
}

fn unix_now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Move the store and its WAL/SHM files aside as `<db>.<label>-<ts>`; returns the main file's new name.
fn move_aside(db: &Path, label: &str) -> Result<PathBuf> {
    let aside = sibling(db, &format!(".{label}-{}", unix_now()));
    std::fs::rename(db, &aside).map_err(|e| StoreError::InvalidValue(format!("{e}")))?;
    for ext in ["-wal", "-shm"] {
        let from = sibling(db, ext);
        if from.exists() {
            let _ = std::fs::rename(&from, sibling(&aside, ext));
        }
    }
    Ok(aside)
}

fn write_notice(db: &Path, notice: &RecoveryNotice) {
    if let Ok(text) = serde_json::to_string_pretty(notice) {
        let _ = std::fs::write(notice_path(db), text);
    }
}

/// The last recovery recorded beside `db`, if any.
pub fn read_notice(db: &Path) -> Option<RecoveryNotice> {
    serde_json::from_str(&std::fs::read_to_string(notice_path(db)).ok()?).ok()
}

fn put_backup_in_place(db: &Path, backup: &Path) -> Result<()> {
    std::fs::copy(backup, db).map_err(|e| StoreError::InvalidValue(format!("{e}")))?;
    restrict(db);
    Ok(())
}

/// Put the newest backup that still reads in place of `db`, or `None` when none does.
fn restore_newest_readable(db: &Path) -> Result<Option<PathBuf>> {
    for backup in list_backups(db) {
        if quick_check_idle(&backup.path) != Integrity::Ok {
            continue;
        }
        put_backup_in_place(db, &backup.path)?;
        return Ok(Some(backup.path));
    }
    Ok(None)
}

/// A damaged store: keep it as `<db>.corrupt-<ts>` and restore the newest readable backup. With
/// no usable backup the store is left absent, so the next open starts empty; the damaged file is
/// still there to salvage.
pub fn quarantine_and_restore(db: &Path, reason: &str) -> Result<RecoveryNotice> {
    let quarantined = move_aside(db, "corrupt")?;
    let restored_from = restore_newest_readable(db)?;
    let notice = RecoveryNotice {
        at: unix_now(),
        outcome: if restored_from.is_some() {
            "restored"
        } else {
            "fresh_start"
        }
        .to_string(),
        reason: reason.to_string(),
        quarantined: quarantined.display().to_string(),
        restored_from: restored_from.map(|p| p.display().to_string()),
    };
    write_notice(db, &notice);
    Ok(notice)
}

/// Replace the store with a chosen backup, keeping what was there as `<db>.replaced-<ts>`. Only
/// one of the copies [`list_backups`] finds is accepted.
pub fn restore_backup(db: &Path, backup: &Path) -> Result<RecoveryNotice> {
    if !list_backups(db).iter().any(|b| b.path == backup) {
        return Err(StoreError::InvalidValue(
            "that file is not a backup of this store".to_string(),
        ));
    }
    restore_file(db, backup)
}

/// Replace the store with any readable database file (a backup `forge backup` made, say),
/// keeping what was there as `<db>.replaced-<ts>`.
pub fn restore_file(db: &Path, source: &Path) -> Result<RecoveryNotice> {
    if let Integrity::Corrupt(why) | Integrity::Unknown(why) = quick_check_idle(source) {
        return Err(StoreError::InvalidValue(format!(
            "that file cannot be read as a store: {why}"
        )));
    }
    let quarantined = if db.exists() {
        Some(move_aside(db, "replaced")?)
    } else {
        None
    };
    put_backup_in_place(db, source)?;
    let notice = RecoveryNotice {
        at: unix_now(),
        outcome: "restored".to_string(),
        reason: "The session store was restored from a backup".to_string(),
        quarantined: quarantined
            .map(|p| p.display().to_string())
            .unwrap_or_default(),
        restored_from: Some(source.display().to_string()),
    };
    write_notice(db, &notice);
    Ok(notice)
}

/// A consistent copy of the store file at `db`, made without opening it as a Forge store, so it
/// works on a store this build refuses (too new) as well as a healthy one.
pub fn backup_file(db: &Path, dest: &Path) -> Result<()> {
    if let Some(parent) = dest.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|e| StoreError::InvalidValue(format!("{e}")))?;
    }
    snapshot_connection(&open_read_only(db)?, dest)
}

impl Store {
    /// A consistent copy of the live store at `dest` (SQLite online backup), safe while other
    /// connections write. Replaces `dest` atomically.
    pub fn backup_to(&self, dest: &Path) -> Result<()> {
        let conn = self.lock()?;
        snapshot_connection(&conn, dest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seeded(path: &Path, version: i64) {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch("CREATE TABLE t (v TEXT); INSERT INTO t VALUES ('keep me');")
            .unwrap();
        conn.pragma_update(None, "user_version", version).unwrap();
    }

    fn read_marker(path: &Path) -> String {
        Connection::open(path)
            .unwrap()
            .query_row("SELECT v FROM t", [], |r| r.get(0))
            .unwrap()
    }

    #[test]
    fn a_pending_migration_leaves_a_readable_copy_and_the_original_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("forge.db");
        seeded(&db, SCHEMA_VERSION - 1);
        let backup = backup_before_migration(&db)
            .unwrap()
            .expect("a copy is taken");
        assert_eq!(
            backup.file_name().unwrap().to_string_lossy(),
            format!("forge.db.pre-v{SCHEMA_VERSION}.bak")
        );
        assert_eq!(read_marker(&backup), "keep me");
        assert_eq!(read_marker(&db), "keep me");
    }

    #[test]
    fn nothing_is_copied_when_no_migration_is_pending() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("forge.db");
        assert_eq!(backup_before_migration(&db).unwrap(), None, "no file");
        std::fs::write(&db, b"").unwrap();
        assert_eq!(backup_before_migration(&db).unwrap(), None, "empty file");
        std::fs::remove_file(&db).unwrap();
        seeded(&db, SCHEMA_VERSION);
        assert_eq!(
            backup_before_migration(&db).unwrap(),
            None,
            "already current"
        );
        seeded(&dir.path().join("newer.db"), SCHEMA_VERSION + 3);
        assert_eq!(
            backup_before_migration(&dir.path().join("newer.db")).unwrap(),
            None,
            "a newer store is left for the schema-too-new refusal"
        );
        assert!(list_backups(&db).is_empty());
    }

    #[test]
    fn a_second_open_during_the_same_migration_keeps_the_first_copy() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("forge.db");
        seeded(&db, SCHEMA_VERSION - 2);
        let first = backup_before_migration(&db).unwrap().expect("first copy");
        Connection::open(&db)
            .unwrap()
            .execute("UPDATE t SET v = 'half-migrated'", [])
            .unwrap();
        Connection::open(&db)
            .unwrap()
            .pragma_update(None, "user_version", SCHEMA_VERSION - 1)
            .unwrap();
        assert_eq!(backup_before_migration(&db).unwrap(), None);
        assert_eq!(
            read_marker(&first),
            "keep me",
            "the pre-migration state survives"
        );
    }

    #[test]
    fn a_store_still_creating_its_schema_is_not_backed_up() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("forge.db");
        seeded(&db, 0);
        assert_eq!(backup_before_migration(&db).unwrap(), None);
    }

    #[test]
    fn only_the_two_newest_copies_survive() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("forge.db");
        seeded(&db, 1);
        for version in [10, 20, 30] {
            let path = backup_path(&db, version);
            std::fs::copy(&db, &path).unwrap();
            let age = std::time::SystemTime::now()
                - std::time::Duration::from_secs((40 - version) as u64 * 100);
            std::fs::File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_modified(age)
                .unwrap();
        }
        backup_before_migration(&db).unwrap();
        let kept: Vec<i64> = list_backups(&db).iter().map(|b| b.before_version).collect();
        assert_eq!(
            kept,
            vec![SCHEMA_VERSION, 30],
            "the newest two, newest first"
        );
    }

    #[test]
    fn checking_a_backup_leaves_no_wal_files_and_a_long_report_is_summarised() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("my forge?.db");
        seeded(&db, 1);
        {
            let conn = Connection::open(&db).unwrap();
            let mode: String = conn
                .pragma_update_and_check(None, "journal_mode", "WAL", |r| r.get(0))
                .unwrap();
            assert_eq!(mode, "wal");
        }
        let before: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .map(|e| e.file_name())
            .collect();
        assert_eq!(quick_check_idle(&db), Integrity::Ok);
        let after: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .map(|e| e.file_name())
            .collect();
        assert_eq!(
            before.len(),
            after.len(),
            "no -wal/-shm appeared: {after:?}"
        );

        let big = dir.path().join("big.db");
        let conn = Connection::open(&big).unwrap();
        conn.execute_batch("CREATE TABLE t (v TEXT)").unwrap();
        for _ in 0..60 {
            conn.execute("INSERT INTO t VALUES (hex(randomblob(3000)))", [])
                .unwrap();
        }
        drop(conn);
        use std::io::{Seek, SeekFrom, Write};
        let mut file = std::fs::OpenOptions::new().write(true).open(&big).unwrap();
        file.seek(SeekFrom::Start(4096 * 4)).unwrap();
        file.write_all(&vec![0xFFu8; 4096 * 20]).unwrap();
        drop(file);
        let Integrity::Corrupt(report) = quick_check(&big) else {
            panic!("damage must be found")
        };
        assert!(
            report.lines().count() == 1 && report.len() < 400,
            "{report}"
        );
    }

    #[test]
    fn a_healthy_store_passes_and_garbage_is_called_corrupt() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("forge.db");
        seeded(&db, 1);
        assert_eq!(quick_check(&db), Integrity::Ok);
        let junk = dir.path().join("junk.db");
        std::fs::write(&junk, vec![0x42u8; 8192]).unwrap();
        assert!(matches!(quick_check(&junk), Integrity::Corrupt(_)));
        assert!(matches!(
            quick_check(&dir.path().join("missing.db")),
            Integrity::Unknown(_)
        ));
    }

    #[test]
    fn a_damaged_store_is_set_aside_and_the_newest_good_backup_returns() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("forge.db");
        seeded(&db, 1);
        std::fs::copy(&db, backup_path(&db, 5)).unwrap();
        std::fs::write(&db, vec![0x13u8; 8192]).unwrap();
        let notice = quarantine_and_restore(&db, "test").unwrap();
        assert_eq!(notice.outcome, "restored");
        assert_eq!(read_marker(&db), "keep me");
        assert!(Path::new(&notice.quarantined).exists());
        assert!(notice.quarantined.contains("forge.db.corrupt-"));
        assert_eq!(read_notice(&db), Some(notice));
    }

    #[test]
    fn a_damaged_backup_is_skipped_for_an_older_good_one() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("forge.db");
        seeded(&db, 1);
        let good = backup_path(&db, 5);
        std::fs::copy(&db, &good).unwrap();
        let bad = backup_path(&db, 6);
        std::fs::write(&bad, vec![0x99u8; 8192]).unwrap();
        std::fs::File::options()
            .write(true)
            .open(&good)
            .unwrap()
            .set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(500))
            .unwrap();
        std::fs::write(&db, vec![0x13u8; 8192]).unwrap();
        let notice = quarantine_and_restore(&db, "test").unwrap();
        assert_eq!(notice.restored_from, Some(good.display().to_string()));
        assert_eq!(read_marker(&db), "keep me");
    }

    #[test]
    fn without_a_usable_backup_the_store_starts_fresh_and_the_wreck_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("forge.db");
        std::fs::write(&db, vec![0x13u8; 8192]).unwrap();
        let notice = quarantine_and_restore(&db, "test").unwrap();
        assert_eq!(notice.outcome, "fresh_start");
        assert!(!db.exists());
        assert!(Path::new(&notice.quarantined).exists());
    }

    #[test]
    fn restoring_by_request_keeps_what_was_replaced_and_refuses_strangers() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("forge.db");
        seeded(&db, 1);
        let backup = backup_path(&db, 7);
        std::fs::copy(&db, &backup).unwrap();
        Connection::open(&db)
            .unwrap()
            .execute("UPDATE t SET v = 'newer'", [])
            .unwrap();
        let notice = restore_backup(&db, &backup).unwrap();
        assert_eq!(read_marker(&db), "keep me");
        assert_eq!(read_marker(Path::new(&notice.quarantined)), "newer");
        assert!(notice.quarantined.contains("forge.db.replaced-"));
        let stranger = dir.path().join("elsewhere.bak");
        std::fs::copy(&backup, &stranger).unwrap();
        assert!(restore_backup(&db, &stranger).is_err());
    }

    #[test]
    fn a_manual_backup_copies_even_a_store_from_a_newer_build_and_restores_from_any_file() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("forge.db");
        seeded(&db, SCHEMA_VERSION + 4);
        let manual = dir.path().join("backups").join("mine.db");
        backup_file(&db, &manual).unwrap();
        assert_eq!(read_marker(&manual), "keep me");
        Connection::open(&db)
            .unwrap()
            .execute("UPDATE t SET v = 'changed'", [])
            .unwrap();
        restore_file(&db, &manual).unwrap();
        assert_eq!(read_marker(&db), "keep me");
        assert!(restore_file(&db, &dir.path().join("absent.db")).is_err());
    }

    #[test]
    fn opening_a_store_below_this_schema_takes_the_copy_first() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("forge.db");
        drop(Store::open(&db).unwrap());
        Connection::open(&db)
            .unwrap()
            .pragma_update(None, "user_version", SCHEMA_VERSION - 1)
            .unwrap();
        assert!(list_backups(&db).is_empty());
        drop(Store::open(&db).unwrap());
        let copies = list_backups(&db);
        assert_eq!(copies.len(), 1);
        let held: i64 = open_read_only(&copies[0].path)
            .unwrap()
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            held,
            SCHEMA_VERSION - 1,
            "the copy is the pre-migration state"
        );
    }

    #[test]
    fn a_live_store_can_be_backed_up_and_the_copy_opens() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("forge.db");
        let store = Store::open(&db).unwrap();
        let dest = dir.path().join("snap.db");
        store.backup_to(&dest).unwrap();
        assert_eq!(quick_check(&dest), Integrity::Ok);
        drop(Store::open(&dest).unwrap());
    }
}
