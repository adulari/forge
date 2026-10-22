//! How a connection waits for SQLite's single WAL write lock.
//!
//! SQLite's stock busy handler (`busy_timeout`) backs off to one poll every 100 ms. A writer that
//! commits short transactions back to back — the Lattice indexer purging a deleted workspace one
//! file at a time, a batch of PageRank updates, the catalog sweep's row-per-model upserts — leaves
//! a gap of microseconds between them, and a 100 ms poll almost never lands in it. Measured on this
//! machine: a writer looping 0.8 ms transactions starved a second writer for its entire 5 s
//! timeout, while 1 ms polls got in within 35 ms. With several sessions or bench tasks sharing one
//! store that starvation surfaced as `building session: starting session: database is locked` and
//! `running the agent turn: database is locked`.
//!
//! [`wait_for_lock`] polls in milliseconds instead, for up to [`BUSY_WAIT_BUDGET`].

use std::cell::Cell;
use std::time::{Duration, Instant};

/// How long one statement keeps waiting for the write lock before SQLite returns `SQLITE_BUSY`.
/// Long enough that only a genuinely stuck writer fails a turn; critical writes still retry the
/// whole transaction on top of this (`with_busy_retry`) for `SQLITE_BUSY_SNAPSHOT`, which no busy
/// handler sees.
pub(crate) const BUSY_WAIT_BUDGET: Duration = Duration::from_secs(30);

thread_local! {
    /// When the current lock wait on this thread began. SQLite calls the handler on the thread
    /// running the statement, with `attempt == 0` at the start of every new wait.
    static WAIT_STARTED: Cell<Option<Instant>> = const { Cell::new(None) };
}

/// The pause before poll number `attempt`: 1 ms at first, easing to 5 ms for a long wait so a
/// stalled writer does not cost a busy loop.
fn pause_before(attempt: i32) -> Duration {
    Duration::from_millis(match attempt {
        ..=49 => 1,
        50..=499 => 2,
        _ => 5,
    })
}

/// The busy handler installed on every pooled connection: `true` asks SQLite to try again.
pub(crate) fn wait_for_lock(attempt: i32) -> bool {
    let now = Instant::now();
    let started = WAIT_STARTED.with(|cell| match cell.get() {
        Some(started) if attempt > 0 => started,
        _ => {
            cell.set(Some(now));
            now
        }
    });
    if now.duration_since(started) >= BUSY_WAIT_BUDGET {
        return false;
    }
    std::thread::sleep(pause_before(attempt));
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::{Connection, TransactionBehavior};

    fn temp_db() -> std::path::PathBuf {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        std::env::temp_dir().join(format!(
            "forge-busy-{}-{}.db",
            std::process::id(),
            N.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        ))
    }

    fn connect(path: &std::path::Path) -> Connection {
        let conn = Connection::open(path).unwrap();
        conn.pragma_update(None, "journal_mode", "WAL").unwrap();
        conn
    }

    /// Run a writer committing small transactions back to back for `for_how_long`, and meanwhile
    /// one critical write on a connection configured by `configure`. Returns whether it landed.
    fn critical_write_survives(configure: impl FnOnce(&Connection)) -> bool {
        let path = temp_db();
        connect(&path)
            .execute_batch("CREATE TABLE t (x INTEGER)")
            .unwrap();
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let writer = {
            let (path, stop) = (path.clone(), std::sync::Arc::clone(&stop));
            std::thread::spawn(move || {
                let mut conn = connect(&path);
                conn.busy_timeout(Duration::from_secs(30)).unwrap();
                let mut first = true;
                while !stop.load(std::sync::atomic::Ordering::SeqCst) {
                    let tx = conn
                        .transaction_with_behavior(TransactionBehavior::Immediate)
                        .unwrap();
                    for i in 0..200 {
                        tx.execute("INSERT INTO t VALUES (?1)", [i]).unwrap();
                    }
                    tx.commit().unwrap();
                    if first {
                        first = false;
                        started_tx.send(()).unwrap();
                    }
                }
            })
        };
        started_rx.recv().unwrap();
        let mut conn = connect(&path);
        configure(&conn);
        let landed = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .and_then(|tx| {
                tx.execute("INSERT INTO t VALUES (-1)", [])?;
                tx.commit()
            })
            .is_ok();
        stop.store(true, std::sync::atomic::Ordering::SeqCst);
        writer.join().unwrap();
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
        }
        landed
    }

    #[test]
    fn short_polls_get_the_lock_from_a_writer_committing_in_a_loop() {
        assert!(critical_write_survives(|conn| {
            conn.busy_handler(Some(wait_for_lock)).unwrap();
        }));
    }

    #[test]
    fn the_wait_gives_up_once_the_budget_is_spent() {
        WAIT_STARTED.with(|cell| cell.set(Some(Instant::now() - BUSY_WAIT_BUDGET)));
        assert!(!wait_for_lock(7));
        assert!(wait_for_lock(0), "a new wait starts a new budget");
    }
}
