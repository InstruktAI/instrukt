//! Reusable SQLite online-backup and consistency verification primitives
//! (`project/instrukt/spec/feature/runner/home-backup`).
//!
//! Shared by home backup, guarded restore, and single-company container export.

use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;
use std::time::{Duration, Instant};

use rusqlite::backup::{Backup, StepResult};
use rusqlite::{ErrorCode, OpenFlags};

use super::{BUSY_TIMEOUT, StoreError, refuse_database_links};

const BACKUP_STORE: &str = "SQLite database";

/// Snapshots the live SQLite database at `src` into a self-contained database
/// file at `dst` using SQLite's online backup API.
///
/// Opens both connections through [`crate::fsx::open_database`], refuses
/// symbolic links on either database or its sidecars, and copies in a single
/// WAL-safe snapshot (`step(-1)`) so concurrent WAL writers neither block nor
/// livelock the backup.
pub fn online_backup(src: &Path, dst: &Path) -> Result<(), StoreError> {
    refuse_database_links(BACKUP_STORE, src)?;
    refuse_database_links(BACKUP_STORE, dst)?;
    let src_flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let src_conn =
        crate::fsx::open_database(src, src_flags).map_err(StoreError::sqlite(BACKUP_STORE, src))?;
    src_conn
        .busy_timeout(BUSY_TIMEOUT)
        .map_err(StoreError::sqlite(BACKUP_STORE, src))?;

    let dst_flags = OpenFlags::SQLITE_OPEN_READ_WRITE
        | OpenFlags::SQLITE_OPEN_CREATE
        | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let mut dst_conn =
        crate::fsx::open_database(dst, dst_flags).map_err(StoreError::sqlite(BACKUP_STORE, dst))?;
    dst_conn
        .busy_timeout(BUSY_TIMEOUT)
        .map_err(StoreError::sqlite(BACKUP_STORE, dst))?;

    {
        let backup =
            Backup::new(&src_conn, &mut dst_conn).map_err(StoreError::sqlite(BACKUP_STORE, src))?;
        let deadline = Instant::now() + BUSY_TIMEOUT;
        let mut backoff = Duration::from_millis(10);
        loop {
            match backup.step(-1) {
                Ok(StepResult::Done) => break,
                Ok(StepResult::More | StepResult::Busy | StepResult::Locked) => {
                    if Instant::now() >= deadline {
                        return Err(StoreError::invalid(
                            BACKUP_STORE,
                            src,
                            "online backup timed out waiting on SQLite lock",
                        ));
                    }
                    std::thread::sleep(backoff);
                    backoff = (backoff * 2).min(Duration::from_millis(200));
                }
                Ok(_) => {}
                Err(rusqlite::Error::SqliteFailure(err, _))
                    if matches!(
                        err.code,
                        ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked
                    ) && Instant::now() < deadline =>
                {
                    std::thread::sleep(backoff);
                    backoff = (backoff * 2).min(Duration::from_millis(200));
                }
                Err(source) => return Err(StoreError::sqlite(BACKUP_STORE, src)(source)),
            }
        }
    }
    // `sqlite3_backup_step` copies the source header (including WAL mode bytes);
    // switch the standalone snapshot back to DELETE mode so read-only checks and
    // archived copies carry no WAL sidecars.
    dst_conn
        .pragma_update(None, "journal_mode", "DELETE")
        .map_err(StoreError::sqlite(BACKUP_STORE, dst))?;
    drop(dst_conn);
    drop(src_conn);
    crate::fsx::set_permissions(dst, std::fs::Permissions::from_mode(0o600)).map_err(|source| {
        StoreError::Lock(crate::fsx::FsError::Write {
            path: dst.to_path_buf(),
            source,
        })
    })?;
    Ok(())
}

/// Runs `PRAGMA quick_check` on the SQLite database at `path` and fails unless
/// SQLite answers with a single `ok` row.
pub fn quick_check(path: &Path) -> Result<(), StoreError> {
    run_check(path, "quick_check")
}

/// Runs `PRAGMA integrity_check` on the SQLite database at `path` and fails
/// unless SQLite answers with a single `ok` row.
pub fn integrity_check(path: &Path) -> Result<(), StoreError> {
    run_check(path, "integrity_check")
}

fn run_check(path: &Path, pragma: &str) -> Result<(), StoreError> {
    refuse_database_links(BACKUP_STORE, path)?;
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let conn =
        crate::fsx::open_database(path, flags).map_err(StoreError::sqlite(BACKUP_STORE, path))?;
    conn.busy_timeout(BUSY_TIMEOUT)
        .map_err(StoreError::sqlite(BACKUP_STORE, path))?;
    let mut stmt = conn
        .prepare(&format!("PRAGMA {pragma}"))
        .map_err(StoreError::sqlite(BACKUP_STORE, path))?;
    let rows: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(StoreError::sqlite(BACKUP_STORE, path))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(StoreError::sqlite(BACKUP_STORE, path))?;
    if rows.len() == 1 && rows[0].eq_ignore_ascii_case("ok") {
        return Ok(());
    }
    let detail = if rows.is_empty() {
        format!("PRAGMA {pragma} returned no rows")
    } else {
        format!("PRAGMA {pragma} failed: {}", rows.join("; "))
    };
    Err(StoreError::invalid(BACKUP_STORE, path, detail))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    #[test]
    fn online_backup_captures_consistent_snapshot_under_concurrent_wal_writer() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("live.db");
        let dst = dir.path().join("snapshot.db");

        let conn = Connection::open(&src).unwrap();
        conn.pragma_update(None, "journal_mode", "WAL").unwrap();
        conn.execute("CREATE TABLE items (id INTEGER PRIMARY KEY, val TEXT)", [])
            .unwrap();
        for i in 0..50_i64 {
            conn.execute("INSERT INTO items VALUES (?1, 'initial')", [i])
                .unwrap();
        }
        drop(conn);

        // Concurrent WAL writer committing transactions while online_backup runs.
        let stop = Arc::new(AtomicBool::new(false));
        let stop_writer = Arc::clone(&stop);
        let src_writer = src.clone();
        let writer = std::thread::spawn(move || {
            let conn = Connection::open(&src_writer).unwrap();
            conn.busy_timeout(Duration::from_secs(5)).unwrap();
            let mut next = 100_i64;
            while !stop_writer.load(Ordering::Relaxed) && next < 300 {
                let _ = conn.execute("INSERT INTO items VALUES (?1, 'concurrent')", [next]);
                next += 1;
            }
        });

        online_backup(&src, &dst).unwrap();
        stop.store(true, Ordering::Relaxed);
        writer.join().unwrap();

        quick_check(&dst).unwrap();
        integrity_check(&dst).unwrap();
        // Destination snapshot has no WAL sidecars left behind.
        assert!(!dir.path().join("snapshot.db-wal").exists());
        assert!(!dir.path().join("snapshot.db-shm").exists());

        let snap = Connection::open(&dst).unwrap();
        let count: i64 = snap
            .query_row("SELECT COUNT(*) FROM items WHERE id < 50", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 50);
    }

    #[test]
    fn quick_check_and_integrity_check_reject_corrupted_databases() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("corrupt.db");
        let conn = Connection::open(&db_path).unwrap();
        conn.execute(
            "CREATE TABLE entries (id INTEGER PRIMARY KEY, body TEXT)",
            [],
        )
        .unwrap();
        conn.execute("CREATE INDEX idx_body ON entries(body)", [])
            .unwrap();
        for i in 0..200_i64 {
            conn.execute(
                "INSERT INTO entries VALUES (?1, ?2)",
                rusqlite::params![i, format!("payload-{i:04}-xxxxxxxxxxxxxxxxxxxxxxxx")],
            )
            .unwrap();
        }
        drop(conn);

        quick_check(&db_path).unwrap();
        integrity_check(&db_path).unwrap();

        // Corrupt the interior pages of the SQLite file.
        let mut bytes = std::fs::read(&db_path).unwrap();
        assert!(bytes.len() > 8192);
        for byte in &mut bytes[4096..8192] {
            *byte = 0xFF;
        }
        std::fs::write(&db_path, &bytes).unwrap();

        assert!(quick_check(&db_path).is_err());
        assert!(integrity_check(&db_path).is_err());
    }

    #[test]
    fn online_backup_refuses_symlinks_on_source_or_destination() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real.db");
        let conn = Connection::open(&real).unwrap();
        conn.execute("CREATE TABLE t (x INT)", []).unwrap();
        drop(conn);

        let link_src = dir.path().join("link-src.db");
        std::os::unix::fs::symlink(&real, &link_src).unwrap();
        let dst = dir.path().join("out.db");
        assert!(matches!(
            online_backup(&link_src, &dst),
            Err(StoreError::Symlink { .. })
        ));

        let link_dst = dir.path().join("link-dst.db");
        std::os::unix::fs::symlink(dir.path().join("elsewhere.db"), &link_dst).unwrap();
        assert!(matches!(
            online_backup(&real, &link_dst),
            Err(StoreError::Symlink { .. })
        ));
    }
}
