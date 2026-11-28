//! Sidecar-free SQLite inspection. A stable main/WAL image is reconstructed in
//! memory through the last checksum-valid commit, then opened by SQLite itself.
//! Only a process that has not opened the store through SQLite copies it: in
//! one that has, closing the copy's descriptors would release the locks its
//! connections hold, so the store is read through SQLite there
//! ([`crate::fsx::outside_sqlite`]).
//! WAL framing/checksums: https://www.sqlite.org/fileformat2.html#walformat
use super::{Connection, StoreError};
use backon::{BlockingRetryable, BlockingSleeper, ExponentialBuilder};
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

const ROLLBACK_PENDING_MESSAGE: &str =
    "rollback recovery is pending; inspect after the writing operation completes";
const STATE_CHANGED_MESSAGE: &str = "state changed during inspection; retry the read";
const INVALID_SNAPSHOT_MESSAGE: &str =
    "invalid SQLite/WAL snapshot; retry after the writer completes recovery";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SnapshotFile {
    Main,
    Wal,
    Journal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LockTarget {
    Main,
    Shm,
}

#[derive(Debug, PartialEq, Eq)]
enum SnapshotImage {
    Empty,
    Bytes(Vec<u8>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LockProbe {
    /// Another process holds `F_WRLCK` in the probed byte range.
    ActiveWriter,
    /// No conflicting `F_WRLCK` is held (`F_UNLCK`).
    Unlocked,
    /// The probed file did not exist.
    Missing,
}

enum AttemptError {
    Retryable(StoreError),
    Terminal(StoreError),
}

impl AttemptError {
    fn is_retryable(&self) -> bool {
        matches!(self, Self::Retryable(_))
    }

    fn into_inner(self) -> StoreError {
        match self {
            Self::Retryable(err) | Self::Terminal(err) => err,
        }
    }
}

fn snapshot_backoff() -> ExponentialBuilder {
    ExponentialBuilder::default()
        .with_min_delay(Duration::from_millis(2))
        .with_max_delay(Duration::from_millis(50))
        .without_max_times()
        .with_total_delay(Some(super::BUSY_TIMEOUT))
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
fn acquire_image(
    store: &'static str,
    path: &Path,
    backoff: ExponentialBuilder,
    sleeper: impl BlockingSleeper,
    mut read_file: impl FnMut(SnapshotFile) -> Result<Vec<u8>, StoreError>,
    mut probe_lock: impl FnMut(LockTarget) -> Result<LockProbe, StoreError>,
) -> Result<SnapshotImage, StoreError> {
    let mut invalid_retries: u8 = 0;
    let attempt = || -> Result<SnapshotImage, AttemptError> {
        let mut ticket_3883_retried = false;
        loop {
            let main = read_file(SnapshotFile::Main).map_err(AttemptError::Terminal)?;
            let wal = read_file(SnapshotFile::Wal).map_err(AttemptError::Terminal)?;
            let rollback = read_file(SnapshotFile::Journal).map_err(AttemptError::Terminal)?;
            if main != read_file(SnapshotFile::Main).map_err(AttemptError::Terminal)?
                || wal != read_file(SnapshotFile::Wal).map_err(AttemptError::Terminal)?
                || rollback != read_file(SnapshotFile::Journal).map_err(AttemptError::Terminal)?
            {
                invalid_retries = 0;
                return Err(AttemptError::Retryable(StoreError::invalid(
                    store,
                    path,
                    STATE_CHANGED_MESSAGE,
                )));
            }
            if has_nonzero(&rollback) {
                invalid_retries = 0;
                match probe_lock(LockTarget::Main).map_err(AttemptError::Terminal)? {
                    LockProbe::ActiveWriter | LockProbe::Missing => {
                        return Err(AttemptError::Retryable(StoreError::invalid(
                            store,
                            path,
                            ROLLBACK_PENDING_MESSAGE,
                        )));
                    }
                    LockProbe::Unlocked => {
                        // SQLite Ticket #3883: a writer may unlink or zero `-journal`
                        // and release its lock between our read and `F_GETLK`, or a
                        // new writer may start a fresh journal. Re-check `-journal`
                        // before concluding an orphaned hot journal.
                        let recheck =
                            read_file(SnapshotFile::Journal).map_err(AttemptError::Terminal)?;
                        if !has_nonzero(&recheck) || recheck != rollback {
                            if !ticket_3883_retried {
                                ticket_3883_retried = true;
                                continue;
                            }
                            return Err(AttemptError::Retryable(StoreError::invalid(
                                store,
                                path,
                                STATE_CHANGED_MESSAGE,
                            )));
                        }
                        return Err(AttemptError::Terminal(StoreError::invalid(
                            store,
                            path,
                            ROLLBACK_PENDING_MESSAGE,
                        )));
                    }
                }
            }
            if main.is_empty() && wal.is_empty() && rollback.is_empty() {
                return Ok(SnapshotImage::Empty);
            }
            match fold(store, path, main, &wal) {
                Ok(image) => return Ok(SnapshotImage::Bytes(image)),
                Err(err) => {
                    let main_active = matches!(
                        probe_lock(LockTarget::Main).map_err(AttemptError::Terminal)?,
                        LockProbe::ActiveWriter | LockProbe::Missing
                    );
                    let wal_active = !main_active
                        && matches!(
                            probe_lock(LockTarget::Shm).map_err(AttemptError::Terminal)?,
                            LockProbe::ActiveWriter
                        );
                    if main_active || wal_active {
                        invalid_retries = 0;
                        return Err(AttemptError::Retryable(err));
                    }
                    if invalid_retries == 0 {
                        invalid_retries = 1;
                        return Err(AttemptError::Retryable(err));
                    }
                    return Err(AttemptError::Terminal(err));
                }
            }
        }
    };

    attempt
        .retry(backoff)
        .when(AttemptError::is_retryable)
        .sleep(sleeper)
        .call()
        .map_err(AttemptError::into_inner)
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
    let image = acquire_image(
        store,
        path,
        snapshot_backoff(),
        std::thread::sleep,
        |file| match file {
            SnapshotFile::Main => bytes(store, path),
            SnapshotFile::Wal => bytes(store, &wal_path),
            SnapshotFile::Journal => bytes(store, &journal),
        },
        |target| match target {
            LockTarget::Main => probe_write_lock(store, path, SQLITE_LOCK_START, SQLITE_LOCK_LEN),
            LockTarget::Shm => probe_write_lock(
                store,
                &shm_path,
                SQLITE_SHM_WRITE_LOCK_START,
                SQLITE_SHM_WRITE_LOCK_LEN,
            ),
        },
    )?;
    let mut conn = open_memory_connection(store, path)?;
    if let SnapshotImage::Bytes(image) = image {
        conn.deserialize_read_exact(
            super::rusqlite::MAIN_DB,
            image.as_slice(),
            image.len(),
            false,
        )
        .map_err(StoreError::sqlite(store, path))?;
    }
    Ok(conn)
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
    use super::{
        INVALID_SNAPSHOT_MESSAGE, LockProbe, LockTarget, ROLLBACK_PENDING_MESSAGE, SnapshotFile,
        SnapshotImage, acquire_image, snapshot_backoff,
    };
    use crate::fsx::FsError;
    use crate::state_db::StoreError;
    use std::collections::VecDeque;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    fn sqlite_header(journal_version: u8, marker: u8) -> Vec<u8> {
        let mut bytes = vec![0u8; 100];
        bytes[..16].copy_from_slice(b"SQLite format 3\0");
        bytes[18] = journal_version;
        bytes[19] = journal_version;
        bytes[60] = marker;
        bytes
    }

    fn wal_header(marker: u8) -> Vec<u8> {
        sqlite_header(2, marker)
    }

    fn folded_header(marker: u8) -> Vec<u8> {
        sqlite_header(1, marker)
    }

    fn nonzero_journal() -> Vec<u8> {
        let mut bytes = vec![0u8; 512];
        bytes[..8].copy_from_slice(&[0xd9, 0xd5, 0x05, 0xf9, 0x20, 0xa1, 0x63, 0xd7]);
        bytes
    }

    fn run_script(
        mut reads: VecDeque<(SnapshotFile, Result<Vec<u8>, StoreError>)>,
        mut probes: VecDeque<(LockTarget, Result<LockProbe, StoreError>)>,
    ) -> (Result<SnapshotImage, StoreError>, Vec<Duration>) {
        let sleeps = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&sleeps);
        let result = acquire_image(
            "test store",
            Path::new("test.db"),
            snapshot_backoff(),
            move |delay| recorded.lock().unwrap().push(delay),
            |file| {
                let (expected, outcome) = reads
                    .pop_front()
                    .unwrap_or_else(|| panic!("unexpected read of {file:?}"));
                assert_eq!(file, expected);
                outcome
            },
            |target| {
                let (expected, outcome) = probes
                    .pop_front()
                    .unwrap_or_else(|| panic!("unexpected lock probe of {target:?}"));
                assert_eq!(target, expected);
                outcome
            },
        );
        assert!(reads.is_empty(), "unconsumed reads: {}", reads.len());
        assert!(probes.is_empty(), "unconsumed probes: {}", probes.len());
        let delays = sleeps.lock().unwrap().clone();
        (result, delays)
    }

    fn push_stable_reads(
        reads: &mut VecDeque<(SnapshotFile, Result<Vec<u8>, StoreError>)>,
        main: &[u8],
        wal: &[u8],
        journal: &[u8],
    ) {
        for _ in 0..2 {
            reads.push_back((SnapshotFile::Main, Ok(main.to_vec())));
            reads.push_back((SnapshotFile::Wal, Ok(wal.to_vec())));
            reads.push_back((SnapshotFile::Journal, Ok(journal.to_vec())));
        }
    }

    #[test]
    fn active_rollback_writer_retries_with_backoff_until_committed_image_appears() {
        let mut reads = VecDeque::new();
        push_stable_reads(&mut reads, &wal_header(1), &[], &nonzero_journal());
        push_stable_reads(&mut reads, &wal_header(2), &[], &[]);
        let probes = VecDeque::from([(LockTarget::Main, Ok(LockProbe::ActiveWriter))]);

        let (result, sleeps) = run_script(reads, probes);
        assert_eq!(result.unwrap(), SnapshotImage::Bytes(folded_header(2)));
        assert_eq!(sleeps, vec![Duration::from_millis(2)]);
    }

    #[test]
    fn ticket_3883_vanished_journal_retries_immediately_without_sleeping() {
        let mut reads = VecDeque::new();
        push_stable_reads(&mut reads, &wal_header(1), &[], &nonzero_journal());
        // Ticket #3883 re-check sees the journal gone after F_GETLK returned F_UNLCK.
        reads.push_back((SnapshotFile::Journal, Ok(vec![])));
        push_stable_reads(&mut reads, &wal_header(2), &[], &[]);
        let probes = VecDeque::from([(LockTarget::Main, Ok(LockProbe::Unlocked))]);

        let (result, sleeps) = run_script(reads, probes);
        assert_eq!(result.unwrap(), SnapshotImage::Bytes(folded_header(2)));
        assert!(sleeps.is_empty());
    }

    #[test]
    fn orphaned_hot_journal_fails_immediately_without_retrying() {
        let journal = nonzero_journal();
        let mut reads = VecDeque::new();
        push_stable_reads(&mut reads, &wal_header(1), &[], &journal);
        reads.push_back((SnapshotFile::Journal, Ok(journal)));
        let probes = VecDeque::from([(LockTarget::Main, Ok(LockProbe::Unlocked))]);

        let (result, sleeps) = run_script(reads, probes);
        match result {
            Err(StoreError::Invalid { message, .. }) => {
                assert_eq!(message, ROLLBACK_PENDING_MESSAGE)
            }
            other => panic!("expected StoreError::Invalid, got {other:?}"),
        }
        assert!(sleeps.is_empty());
    }

    #[test]
    fn lock_probe_error_propagates_without_falling_back_to_unlocked() {
        let mut reads = VecDeque::new();
        push_stable_reads(&mut reads, &wal_header(1), &[], &nonzero_journal());
        let probes = VecDeque::from([(
            LockTarget::Main,
            Err(StoreError::Lock(FsError::NoParent {
                path: PathBuf::from("test.db"),
            })),
        )]);

        let (result, sleeps) = run_script(reads, probes);
        assert!(matches!(
            result,
            Err(StoreError::Lock(FsError::NoParent { .. }))
        ));
        assert!(sleeps.is_empty());
    }

    #[test]
    fn zero_byte_image_with_no_sidecars_returns_empty_snapshot() {
        let mut reads = VecDeque::new();
        push_stable_reads(&mut reads, &[], &[], &[]);

        let (result, sleeps) = run_script(reads, VecDeque::new());
        assert_eq!(result.unwrap(), SnapshotImage::Empty);
        assert!(sleeps.is_empty());
    }

    #[test]
    fn corrupt_non_empty_image_with_no_active_writer_fails_after_one_confirmation_retry() {
        let corrupt = b"not a sqlite database";
        let mut reads = VecDeque::new();
        push_stable_reads(&mut reads, corrupt, &[], &[]);
        push_stable_reads(&mut reads, corrupt, &[], &[]);
        let probes = VecDeque::from([
            (LockTarget::Main, Ok(LockProbe::Unlocked)),
            (LockTarget::Shm, Ok(LockProbe::Missing)),
            (LockTarget::Main, Ok(LockProbe::Unlocked)),
            (LockTarget::Shm, Ok(LockProbe::Missing)),
        ]);

        let (result, sleeps) = run_script(reads, probes);
        match result {
            Err(StoreError::Invalid { message, .. }) => {
                assert_eq!(message, INVALID_SNAPSHOT_MESSAGE)
            }
            other => panic!("expected StoreError::Invalid, got {other:?}"),
        }
        assert_eq!(sleeps, vec![Duration::from_millis(2)]);
    }

    #[test]
    fn concurrent_file_change_retries_through_exponential_backoff_until_stable() {
        let mut reads = VecDeque::new();
        // Attempt 1: main mutates between first and second read.
        reads.push_back((SnapshotFile::Main, Ok(wal_header(1))));
        reads.push_back((SnapshotFile::Wal, Ok(vec![])));
        reads.push_back((SnapshotFile::Journal, Ok(vec![])));
        reads.push_back((SnapshotFile::Main, Ok(wal_header(2))));
        // Attempt 2: main mutates again.
        reads.push_back((SnapshotFile::Main, Ok(wal_header(2))));
        reads.push_back((SnapshotFile::Wal, Ok(vec![])));
        reads.push_back((SnapshotFile::Journal, Ok(vec![])));
        reads.push_back((SnapshotFile::Main, Ok(wal_header(3))));
        // Attempt 3: stable.
        push_stable_reads(&mut reads, &wal_header(3), &[], &[]);

        let (result, sleeps) = run_script(reads, VecDeque::new());
        assert_eq!(result.unwrap(), SnapshotImage::Bytes(folded_header(3)));
        assert_eq!(
            sleeps,
            vec![Duration::from_millis(2), Duration::from_millis(4)]
        );
    }
}
