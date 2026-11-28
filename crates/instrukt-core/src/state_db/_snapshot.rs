//! Sidecar-free SQLite inspection. A stable main/WAL image is reconstructed in
//! memory through the last checksum-valid commit, then opened by SQLite itself.
//! Only a process that has not opened the store through SQLite copies it: in
//! one that has, closing the copy's descriptors would release the locks its
//! connections hold, so the store is read through SQLite there
//! ([`crate::fsx::outside_sqlite`]).
//! WAL framing/checksums: https://www.sqlite.org/fileformat2.html#walformat
use super::{Connection, StoreError};
use rustix::io::Errno;
use rustix::process::{Flock, FlockOffsetType, FlockType, fcntl_getlk};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// SQLite's POSIX byte-range lock window on the database file
/// (`PENDING_BYTE = 0x4000_0000`, `RESERVED_BYTE = 0x4000_0001`,
/// `SHARED_FIRST = 0x4000_0002`, `SHARED_SIZE = 510`).
/// Active rollback-journal writers hold `F_WRLCK` on `RESERVED` (upgrading to
/// `PENDING` + `EXCLUSIVE`), and active hot-journal recovery holds `F_WRLCK`
/// on `PENDING` + `EXCLUSIVE`.
const SQLITE_LOCK_START: u64 = 0x4000_0000;
const SQLITE_LOCK_LEN: u64 = 512;

/// SQLite's WAL-index write/checkpoint/recovery lock bytes in `-shm`
/// (`UNIX_SHM_BASE = 120`: `WAL_WRITE_LOCK`, `WAL_CKPT_LOCK`, `WAL_RECOVER_LOCK`).
const SQLITE_SHM_WRITE_LOCK_START: u64 = 120;
const SQLITE_SHM_WRITE_LOCK_LEN: u64 = 3;

const INITIAL_RETRY_PAUSE: Duration = Duration::from_millis(2);
const MAX_RETRY_PAUSE: Duration = Duration::from_millis(50);

const ROLLBACK_PENDING_MESSAGE: &str =
    "rollback recovery is pending; inspect after the writing operation completes";
const STATE_CHANGED_MESSAGE: &str = "state changed during inspection; retry the read";
const INVALID_SNAPSHOT_MESSAGE: &str =
    "invalid SQLite/WAL snapshot; retry after the writer completes recovery";

enum LockProbe {
    /// Another process holds `F_WRLCK` in the probed byte range.
    ActiveWriter,
    /// No conflicting `F_WRLCK` is held (`F_UNLCK`).
    Unlocked,
    /// The probed file did not exist.
    Missing,
}

pub(super) fn sidecar(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(suffix);
    name.into()
}
fn bytes(store: &'static str, path: &Path) -> Result<Vec<u8>, StoreError> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(bytes),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(vec![]),
        Err(e) => Err(StoreError::invalid(store, path, e.to_string())),
    }
}
fn has_nonzero(bytes: &[u8]) -> bool {
    bytes.iter().any(|b| *b != 0)
}
/// Probes POSIX `fcntl(F_GETLK)` with `F_RDLCK` on an `O_RDONLY` descriptor so
/// only conflicting `F_WRLCK` holds report `ActiveWriter`, shared readers are
/// ignored, no lock is taken, and no file is created. Safe to close because
/// callers run inside [`crate::fsx::outside_sqlite`].
fn probe_write_lock(
    store: &'static str,
    path: &Path,
    start: u64,
    length: u64,
) -> Result<LockProbe, StoreError> {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(LockProbe::Missing),
        Err(e) => return Err(StoreError::invalid(store, path, e.to_string())),
    };
    let probe = Flock {
        start,
        length,
        pid: None,
        typ: FlockType::ReadLock,
        offset_type: FlockOffsetType::Set,
    };
    loop {
        match fcntl_getlk(&file, &probe) {
            Ok(Some(_)) => return Ok(LockProbe::ActiveWriter),
            Ok(None) => return Ok(LockProbe::Unlocked),
            Err(Errno::INTR) => continue,
            Err(err) if err == Errno::OPNOTSUPP || err == Errno::NOTSUP || err == Errno::NOSYS => {
                return Ok(LockProbe::Unlocked);
            }
            Err(err) => {
                return Err(StoreError::invalid(
                    store,
                    path,
                    std::io::Error::from(err).to_string(),
                ));
            }
        }
    }
}
fn open_memory_connection(store: &'static str, path: &Path) -> Result<Connection, StoreError> {
    let sql = StoreError::sqlite(store, path);
    let conn = Connection::open_in_memory().map_err(&sql)?;
    conn.pragma_update(None, "temp_store", "MEMORY")
        .map_err(&sql)?;
    Ok(conn)
}
fn sleep_before_retry(limit: &crate::process::Limit, pause: &mut Duration) -> bool {
    let remaining = limit.remaining();
    if remaining.is_zero() {
        return false;
    }
    std::thread::sleep((*pause).min(remaining));
    *pause = (*pause * 2).min(MAX_RETRY_PAUSE);
    true
}
/// The store at `path` as an in-memory image; `None` when this process has
/// opened it through SQLite and reads it live instead.
pub(super) fn read(store: &'static str, path: &Path) -> Result<Option<Connection>, StoreError> {
    crate::fsx::outside_sqlite(path, || copy(store, path)).transpose()
}
fn copy(store: &'static str, path: &Path) -> Result<Connection, StoreError> {
    let wal_path = sidecar(path, "-wal");
    let shm_path = sidecar(path, "-shm");
    let journal = sidecar(path, "-journal");
    let limit = crate::process::Limit::start(super::BUSY_TIMEOUT);
    let mut pause = INITIAL_RETRY_PAUSE;
    let mut invalid_retries: u8 = 0;

    loop {
        let main = bytes(store, path)?;
        let wal = bytes(store, &wal_path)?;
        let rollback = bytes(store, &journal)?;
        if main != bytes(store, path)?
            || wal != bytes(store, &wal_path)?
            || rollback != bytes(store, &journal)?
        {
            invalid_retries = 0;
            if !sleep_before_retry(&limit, &mut pause) {
                return Err(StoreError::invalid(store, path, STATE_CHANGED_MESSAGE));
            }
            continue;
        }
        if has_nonzero(&rollback) {
            invalid_retries = 0;
            match probe_write_lock(store, path, SQLITE_LOCK_START, SQLITE_LOCK_LEN)? {
                LockProbe::ActiveWriter | LockProbe::Missing => {
                    if !sleep_before_retry(&limit, &mut pause) {
                        return Err(StoreError::invalid(store, path, ROLLBACK_PENDING_MESSAGE));
                    }
                    continue;
                }
                LockProbe::Unlocked => {
                    // SQLite Ticket #3883: a writer may unlink or zero `-journal`
                    // and release its lock between our read and `F_GETLK`, or a
                    // new writer may start a fresh journal. Re-check `-journal`
                    // before concluding an orphaned hot journal.
                    let recheck = bytes(store, &journal)?;
                    if !has_nonzero(&recheck) || recheck != rollback {
                        if limit.remaining().is_zero() {
                            return Err(StoreError::invalid(store, path, STATE_CHANGED_MESSAGE));
                        }
                        continue;
                    }
                    return Err(StoreError::invalid(store, path, ROLLBACK_PENDING_MESSAGE));
                }
            }
        }
        if main.is_empty() && wal.is_empty() && rollback.is_empty() {
            return open_memory_connection(store, path);
        }
        let image = match fold(store, path, main, &wal) {
            Ok(image) => image,
            Err(err) => {
                let main_active = matches!(
                    probe_write_lock(store, path, SQLITE_LOCK_START, SQLITE_LOCK_LEN)?,
                    LockProbe::ActiveWriter | LockProbe::Missing
                );
                let wal_active = !main_active
                    && matches!(
                        probe_write_lock(
                            store,
                            &shm_path,
                            SQLITE_SHM_WRITE_LOCK_START,
                            SQLITE_SHM_WRITE_LOCK_LEN,
                        )?,
                        LockProbe::ActiveWriter
                    );
                if main_active || wal_active {
                    invalid_retries = 0;
                    if !sleep_before_retry(&limit, &mut pause) {
                        return Err(err);
                    }
                    continue;
                }
                if invalid_retries == 0 {
                    invalid_retries = 1;
                    if !sleep_before_retry(&limit, &mut pause) {
                        return Err(err);
                    }
                    continue;
                }
                return Err(err);
            }
        };
        let mut conn = open_memory_connection(store, path)?;
        conn.deserialize_read_exact(
            super::rusqlite::MAIN_DB,
            image.as_slice(),
            image.len(),
            false,
        )
        .map_err(StoreError::sqlite(store, path))?;
        return Ok(conn);
    }
}
fn be(bytes: &[u8]) -> u32 {
    u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}
fn checksum(bytes: &[u8], big: bool, mut sum: (u32, u32)) -> (u32, u32) {
    for words in bytes.as_chunks::<8>().0 {
        let word = |part: &[u8]| {
            if big {
                be(part)
            } else {
                u32::from_le_bytes([part[0], part[1], part[2], part[3]])
            }
        };
        sum.0 = sum.0.wrapping_add(word(&words[..4])).wrapping_add(sum.1);
        sum.1 = sum.1.wrapping_add(word(&words[4..])).wrapping_add(sum.0);
    }
    sum
}
fn fold(
    store: &'static str,
    path: &Path,
    mut main: Vec<u8>,
    wal: &[u8],
) -> Result<Vec<u8>, StoreError> {
    let invalid = || StoreError::invalid(store, path, INVALID_SNAPSHOT_MESSAGE);
    if main.len() < 100 || &main[..16] != b"SQLite format 3\0" {
        return Err(invalid());
    }
    if !wal.is_empty() {
        if wal.len() < 32 {
            return Err(invalid());
        }
        let magic = be(wal);
        if !matches!(magic, 0x377f0682 | 0x377f0683) || be(&wal[4..]) != 3_007_000 {
            return Err(invalid());
        }
        let big = magic & 1 != 0;
        let size = be(&wal[8..]) as usize;
        if !(512..=65536).contains(&size) || !size.is_power_of_two() {
            return Err(invalid());
        }
        let mut sum = checksum(&wal[..24], big, (0, 0));
        if sum != (be(&wal[24..]), be(&wal[28..])) {
            return Err(invalid());
        }
        let mut pending = Vec::new();
        for frame in wal[32..].chunks_exact(size + 24) {
            if frame[8..16] != wal[16..24] {
                break;
            }
            let pg = be(frame) as usize;
            if pg == 0 {
                break;
            }
            let next = checksum(&frame[24..], big, checksum(&frame[..8], big, sum));
            if next != (be(&frame[16..]), be(&frame[20..])) {
                break;
            }
            sum = next;
            pending.push((pg, &frame[24..]));
            let pages = be(&frame[4..]) as usize;
            if pages != 0 {
                let length = pages.checked_mul(size).ok_or_else(invalid)?;
                if length > main.len().saturating_add(wal.len()) {
                    return Err(invalid());
                }
                main.resize(length, 0);
                for (pg, data) in pending.drain(..) {
                    if pg <= pages {
                        let offset = (pg - 1).checked_mul(size).ok_or_else(invalid)?;
                        main[offset..offset + size].copy_from_slice(data);
                    }
                }
            }
        }
    }
    if main.len() < 100 {
        return Err(invalid());
    }
    // The WAL has already been applied; the deserialized image has no disk journal.
    main[18] = 1;
    main[19] = 1;
    Ok(main)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::home::Home;
    use crate::state_db::{Migration, Read, Store, StoreDb, StoreLock, inspect_file};
    use std::io::{BufRead, BufReader, Write};
    use std::process::{Command, Stdio};
    use std::sync::{Arc, Barrier, mpsc};
    use std::time::{Duration, Instant};

    const ROLLBACK_WRITER_ENV: &str = "INSTRUKT_SNAPSHOT_ROLLBACK_WRITER_DB";
    const INIT_WRITER_ENV: &str = "INSTRUKT_SNAPSHOT_INIT_WRITER_HOME";
    const ITEMS_MIGRATIONS: &[Migration] = &[Migration::Sql(
        "CREATE TABLE items(id INTEGER PRIMARY KEY, label TEXT NOT NULL);",
    )];

    fn items_store(dir: &Path) -> Store {
        Store {
            name: "test store",
            file: dir.join("items.db"),
            lock: StoreLock::Exclusive(dir.join("items.lock"), Duration::from_secs(5)),
            migrations: ITEMS_MIGRATIONS,
        }
    }

    fn run_rollback_writer_child(db_path: &Path) {
        let conn = Connection::open(db_path).unwrap();
        conn.execute_batch(
            "PRAGMA journal_mode = DELETE;
             PRAGMA synchronous = FULL;
             CREATE TABLE items(id INTEGER PRIMARY KEY, label TEXT NOT NULL);
             INSERT INTO items(id, label) VALUES (1, 'before');
             PRAGMA user_version = 1;",
        )
        .unwrap();
        conn.execute_batch(
            "BEGIN IMMEDIATE;
             UPDATE items SET label = 'committed' WHERE id = 1;",
        )
        .unwrap();
        let journal = sidecar(db_path, "-journal");
        let journal_bytes = std::fs::read(&journal).unwrap();
        assert!(
            journal_bytes.iter().any(|b| *b != 0),
            "active rollback writer must have a non-zero journal on disk"
        );
        let mut stdout = std::io::stdout().lock();
        writeln!(stdout, "LOCKED").unwrap();
        stdout.flush().unwrap();
        let mut line = String::new();
        let _ = std::io::stdin().read_line(&mut line);
        conn.execute_batch("COMMIT;").unwrap();
    }

    fn spawn_rollback_writer(
        test_name: &str,
        db_path: &Path,
    ) -> (std::process::Child, BufReader<std::process::ChildStdout>) {
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", test_name, "--nocapture"])
            .env(ROLLBACK_WRITER_ENV, db_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut reader = BufReader::new(child.stdout.take().unwrap());
        let mut line = String::new();
        loop {
            line.clear();
            let n = reader.read_line(&mut line).unwrap();
            assert!(n > 0, "rollback writer exited before signaling LOCKED");
            if line.trim() == "LOCKED" {
                break;
            }
        }
        (child, reader)
    }

    #[test]
    fn snapshot_read_waits_for_active_external_rollback_writer_without_creating_sidecars() {
        if let Some(db_path) = std::env::var_os(ROLLBACK_WRITER_ENV) {
            run_rollback_writer_child(Path::new(&db_path));
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let store = items_store(dir.path());
        let (mut child, _stdout) = spawn_rollback_writer(
            "state_db::_snapshot::tests::snapshot_read_waits_for_active_external_rollback_writer_without_creating_sidecars",
            &store.file,
        );
        let mut child_stdin = child.stdin.take().unwrap();

        let barrier = Arc::new(Barrier::new(2));
        let reader_barrier = Arc::clone(&barrier);
        let reader_store = store.clone();
        let (tx, rx) = mpsc::channel();
        let handle = std::thread::spawn(move || {
            reader_barrier.wait();
            let label = reader_store.read(true).map(|read| match read {
                Some(Read::Current(conn)) => conn
                    .query_row("SELECT label FROM items WHERE id = 1", [], |row| {
                        row.get::<_, String>(0)
                    })
                    .unwrap(),
                _ => panic!("expected current snapshot after active writer commits"),
            });
            let _ = tx.send(label);
        });

        barrier.wait();
        // While the active writer holds the rollback lock and non-zero -journal,
        // snapshot inspection must wait instead of failing immediately.
        if let Ok(early) = rx.recv_timeout(Duration::from_millis(120)) {
            let _ = child.kill();
            let _ = child.wait();
            panic!(
                "snapshot read returned early while an active external writer held a rollback journal: {early:?}"
            );
        }

        writeln!(child_stdin, "COMMIT").unwrap();
        drop(child_stdin);
        let status = child.wait().unwrap();
        assert!(status.success(), "rollback writer exited with {status}");

        let label = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("snapshot read should finish once the writer commits")
            .expect("snapshot read should succeed once the writer commits");
        handle.join().unwrap();
        assert_eq!(label, "committed");

        for suffix in ["-wal", "-shm", "-journal"] {
            assert!(
                !sidecar(&store.file, suffix).exists(),
                "snapshot read must leave no {suffix} sidecar on disk"
            );
        }
        assert!(
            !dir.path().join("items.lock").exists(),
            "snapshot read must not create the store lock file"
        );
    }

    #[test]
    fn snapshot_read_fails_promptly_on_orphaned_hot_journal() {
        if let Some(db_path) = std::env::var_os(ROLLBACK_WRITER_ENV) {
            run_rollback_writer_child(Path::new(&db_path));
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let store = items_store(dir.path());
        let (mut child, _stdout) = spawn_rollback_writer(
            "state_db::_snapshot::tests::snapshot_read_fails_promptly_on_orphaned_hot_journal",
            &store.file,
        );
        child.kill().unwrap();
        let _ = child.wait().unwrap();

        let journal_path = sidecar(&store.file, "-journal");
        let journal_before = std::fs::read(&journal_path).unwrap();
        assert!(journal_before.iter().any(|b| *b != 0));

        let started = Instant::now();
        let err = match store.read(true) {
            Err(err) => err,
            Ok(_) => panic!("orphaned hot journal must be rejected"),
        };
        let elapsed = started.elapsed();

        match err {
            StoreError::Invalid { message, .. } => assert_eq!(message, ROLLBACK_PENDING_MESSAGE),
            other => panic!("expected StoreError::Invalid, got {other:?}"),
        }
        assert!(
            elapsed < super::super::BUSY_TIMEOUT / 2,
            "orphaned hot journal should fail promptly without waiting out BUSY_TIMEOUT (took {elapsed:?})"
        );
        assert_eq!(std::fs::read(&journal_path).unwrap(), journal_before);
        for suffix in ["-wal", "-shm"] {
            assert!(!sidecar(&store.file, suffix).exists());
        }
        assert!(!dir.path().join("items.lock").exists());
    }

    #[test]
    fn zero_byte_database_matches_live_sqlite_across_all_snapshot_entry_paths() {
        let dir = tempfile::tempdir().unwrap();
        let store = items_store(dir.path());
        std::fs::write(&store.file, []).unwrap();

        assert!(store.read(true).unwrap().is_none());
        assert_eq!(store.stored_schema().unwrap(), Some(0));
        let inspected = inspect_file(&store.file)
            .unwrap()
            .expect("inspect_file should return an empty in-memory connection for a 0-byte file");
        let version: u32 = inspected
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        let query_only: u32 = inspected
            .pragma_query_value(None, "query_only", |row| row.get(0))
            .unwrap();
        let tables: u32 = inspected
            .query_row("SELECT count(*) FROM sqlite_master", [], |row| row.get(0))
            .unwrap();
        assert_eq!((version, query_only, tables), (0, 1, 0));

        assert!(std::fs::read(&store.file).unwrap().is_empty());
        for suffix in ["-wal", "-shm", "-journal"] {
            assert!(!sidecar(&store.file, suffix).exists());
        }
        assert!(!dir.path().join("items.lock").exists());
    }

    #[test]
    fn corrupt_non_empty_file_fails_promptly_without_mutating_disk() {
        let dir = tempfile::tempdir().unwrap();
        let store = items_store(dir.path());
        let corrupt = b"not a sqlite database";
        std::fs::write(&store.file, corrupt).unwrap();

        let started = Instant::now();
        let err = match store.read(true) {
            Err(err) => err,
            Ok(_) => panic!("corrupt non-empty file must be rejected"),
        };
        let elapsed = started.elapsed();

        match err {
            StoreError::Invalid { message, .. } => assert_eq!(message, INVALID_SNAPSHOT_MESSAGE),
            other => panic!("expected StoreError::Invalid, got {other:?}"),
        }
        assert!(
            elapsed < super::super::BUSY_TIMEOUT / 2,
            "corrupt static file should fail promptly without waiting out BUSY_TIMEOUT (took {elapsed:?})"
        );
        assert_eq!(std::fs::read(&store.file).unwrap(), corrupt);
        for suffix in ["-wal", "-shm", "-journal"] {
            assert!(!sidecar(&store.file, suffix).exists());
        }
        assert!(!dir.path().join("items.lock").exists());
    }

    #[test]
    fn snapshot_read_succeeds_during_concurrent_store_initialization_and_wal_writes() {
        if let Some(home_dir) = std::env::var_os(INIT_WRITER_ENV) {
            let home = Home::at_user_home(PathBuf::from(home_dir));
            let mut db = StoreDb::open(&Store::state(&home)).unwrap();
            for seq in 1..=10 {
                db.write::<_, StoreError>(|tx| {
                    tx.execute(
                        "INSERT INTO events(seq, id, name, payload, envelope, logged_at)
                         VALUES (?1, ?2, 'snapshot.test', '{}', '{}', '2026-11-28T00:00:00Z')",
                        super::super::rusqlite::params![seq, format!("ev-{seq}")],
                    )
                    .map_err(StoreError::sqlite(
                        "the state database",
                        &home.state_db_file(),
                    ))?;
                    Ok(())
                })
                .unwrap();
            }
            return;
        }

        for trial in 0..5 {
            let dir = tempfile::tempdir().unwrap();
            let home = Home::at_user_home(dir.path().to_path_buf());
            let store = Store::state(&home);

            let mut child = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "state_db::_snapshot::tests::snapshot_read_succeeds_during_concurrent_store_initialization_and_wal_writes",
                    "--nocapture",
                ])
                .env(INIT_WRITER_ENV, dir.path())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();

            while child.try_wait().unwrap().is_none() {
                let _ = store.read(true).unwrap_or_else(|err| {
                    panic!("trial {trial} read(true) failed during init: {err}")
                });
                let _ = store.stored_schema().unwrap_or_else(|err| {
                    panic!("trial {trial} stored_schema() failed during init: {err}")
                });
            }
            let status = child.wait().unwrap();
            assert!(
                status.success(),
                "trial {trial} writer exited with {status}"
            );

            let Some(Read::Current(conn)) = store.read(true).unwrap() else {
                panic!("trial {trial} expected initialized state database after writer exit");
            };
            let count: i64 = conn
                .query_row("SELECT count(*) FROM events", [], |row| row.get(0))
                .unwrap();
            assert_eq!(count, 10);
            assert_eq!(
                store.stored_schema().unwrap(),
                Some(crate::state_db::schema_version())
            );
            for suffix in ["-wal", "-shm", "-journal"] {
                assert!(
                    !sidecar(&store.file, suffix).exists(),
                    "trial {trial}: snapshot read must leave no {suffix} sidecar after writer exit"
                );
            }
        }
    }
}
