//! Filesystem effects every context shares: whole-file atomic writes,
//! advisory locks, and the write fence every write passes.
//!
//! A write either lands completely or not at all — readers never see a
//! partly written file. A lock is held for the lifetime of its guard and
//! released by the operating system if the process dies.
//!
//! Every file operation instrukt makes itself goes through this module, which
//! admits it at the write fence first (`_fence`); the workspace's clippy
//! configuration disallows the standard library's writing operations, and
//! SQLite's opens, everywhere else. The logger is admitted where it is
//! initialised ([`admit`]).

mod _fence;

use std::collections::BTreeSet;
use std::fs::{File, OpenOptions, TryLockError};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{PoisonError, RwLock};
use std::time::{Duration, Instant};

pub use _fence::{
    EXIT as FENCE_EXIT, RECORD as FENCE_RECORD, RECORD_ENTRY as FENCE_RECORD_ENTRY,
    RecordUnavailable as FenceRecordUnavailable, Refusal as FenceRefusal, Setting as FenceSetting,
    VARIABLE as FENCE_VARIABLE,
};

/// Fences the locations the startup environment names in
/// [`FENCE_VARIABLE`] for the rest of this process. Called once, first, by
/// the binary; a record it names that cannot be opened stops the binary
/// before it does anything.
pub fn fence(env: &crate::config::Env) -> Result<(), FenceRecordUnavailable> {
    _fence::install(&env.write_fence())
}

/// Whether this process refused a write at its fence: the binary then ends
/// with [`FENCE_EXIT`], whatever the refused operation's caller did.
pub fn fence_refused() -> bool {
    _fence::refused()
}

/// Whether `error`, or an error it carries, is a refusal at the fence.
pub fn is_fence_refusal(error: &(dyn std::error::Error + 'static)) -> bool {
    _fence::is_refusal(error)
}

/// Admits a write to `path` that a library makes itself, before calling it.
pub fn admit(path: &Path) -> io::Result<()> {
    _fence::admit(path)
}

/// Whether a fence is installed: without one, admission does nothing, so
/// a caller need not even work out what it would admit.
pub fn fence_active() -> bool {
    _fence::active()
}

/// [`std::fs::write`], admitted at the fence.
#[allow(clippy::disallowed_methods, reason = "admitted at the fence")]
pub fn write(path: impl AsRef<Path>, contents: impl AsRef<[u8]>) -> io::Result<()> {
    _fence::admit(path.as_ref())?;
    std::fs::write(path, contents)
}

/// [`std::fs::create_dir_all`], admitted at the fence.
#[allow(clippy::disallowed_methods, reason = "admitted at the fence")]
pub fn create_dir_all(path: impl AsRef<Path>) -> io::Result<()> {
    _fence::admit(path.as_ref())?;
    std::fs::create_dir_all(path)
}

/// [`std::fs::remove_file`], admitted at the fence.
#[allow(clippy::disallowed_methods, reason = "admitted at the fence")]
pub fn remove_file(path: impl AsRef<Path>) -> io::Result<()> {
    _fence::admit(path.as_ref())?;
    std::fs::remove_file(path)
}

/// [`std::fs::remove_dir`], admitted at the fence.
#[allow(clippy::disallowed_methods, reason = "admitted at the fence")]
pub fn remove_dir(path: impl AsRef<Path>) -> io::Result<()> {
    _fence::admit(path.as_ref())?;
    std::fs::remove_dir(path)
}

/// [`std::fs::remove_dir_all`], admitted at the fence.
#[allow(clippy::disallowed_methods, reason = "admitted at the fence")]
pub fn remove_dir_all(path: impl AsRef<Path>) -> io::Result<()> {
    _fence::admit(path.as_ref())?;
    std::fs::remove_dir_all(path)
}

/// [`std::os::unix::fs::symlink`], admitted at the fence where the link is
/// made.
#[allow(clippy::disallowed_methods, reason = "admitted at the fence")]
pub fn symlink(points_to: impl AsRef<Path>, link: impl AsRef<Path>) -> io::Result<()> {
    _fence::admit(link.as_ref())?;
    std::os::unix::fs::symlink(points_to, link)
}

/// [`std::fs::rename`], admitted at the fence on both ends.
#[allow(clippy::disallowed_methods, reason = "admitted at the fence")]
pub fn rename(from: impl AsRef<Path>, to: impl AsRef<Path>) -> io::Result<()> {
    _fence::admit(from.as_ref())?;
    _fence::admit(to.as_ref())?;
    std::fs::rename(from, to)
}

/// [`std::fs::set_permissions`], admitted at the fence.
#[allow(clippy::disallowed_methods, reason = "admitted at the fence")]
pub fn set_permissions(
    path: impl AsRef<Path>,
    permissions: std::fs::Permissions,
) -> io::Result<()> {
    _fence::admit(path.as_ref())?;
    std::fs::set_permissions(path, permissions)
}

/// [`File::create`], admitted at the fence.
#[allow(clippy::disallowed_methods, reason = "admitted at the fence")]
pub fn create(path: impl AsRef<Path>) -> io::Result<File> {
    _fence::admit(path.as_ref())?;
    File::create(path)
}

/// Opens `path` with `options`, admitted at the fence. Reading alone needs
/// no admission: [`File::open`] reads.
#[allow(clippy::disallowed_methods, reason = "admitted at the fence")]
pub fn open(options: &OpenOptions, path: impl AsRef<Path>) -> io::Result<File> {
    _fence::admit(path.as_ref())?;
    options.open(path)
}

/// A named temporary file in `dir`, admitted at the fence.
#[allow(clippy::disallowed_methods, reason = "admitted at the fence")]
pub fn temp_file_in(dir: impl AsRef<Path>) -> io::Result<tempfile::NamedTempFile> {
    _fence::admit(dir.as_ref())?;
    tempfile::NamedTempFile::new_in(dir)
}

/// Puts `temp` in `path`'s place, admitted at the fence.
#[allow(clippy::disallowed_methods, reason = "admitted at the fence")]
pub fn persist(
    temp: tempfile::NamedTempFile,
    path: impl AsRef<Path>,
) -> Result<File, tempfile::PersistError> {
    if let Err(error) = _fence::admit(path.as_ref()) {
        return Err(tempfile::PersistError { error, file: temp });
    }
    temp.persist(path)
}

/// Puts `temp` at `path` unless something is there, admitted at the fence.
#[allow(clippy::disallowed_methods, reason = "admitted at the fence")]
pub fn persist_noclobber(
    temp: tempfile::NamedTempFile,
    path: impl AsRef<Path>,
) -> Result<File, tempfile::PersistError> {
    if let Err(error) = _fence::admit(path.as_ref()) {
        return Err(tempfile::PersistError { error, file: temp });
    }
    temp.persist_noclobber(path)
}

/// A temporary directory `builder` makes in `dir`, admitted at the fence.
#[allow(clippy::disallowed_methods, reason = "admitted at the fence")]
pub fn temp_dir_in(
    builder: &tempfile::Builder<'_, '_>,
    dir: impl AsRef<Path>,
) -> io::Result<tempfile::TempDir> {
    _fence::admit(dir.as_ref())?;
    builder.tempdir_in(dir)
}

/// A temporary directory `builder` makes in the system's temporary
/// directory, admitted at the fence.
#[allow(clippy::disallowed_methods, reason = "admitted at the fence")]
pub fn temp_dir(builder: &tempfile::Builder<'_, '_>) -> io::Result<tempfile::TempDir> {
    if _fence::active() {
        _fence::admit(&std::env::temp_dir())?;
    }
    builder.tempdir()
}

/// Opens the SQLite database at `path` with `flags`, admitted at the fence
/// by what the open can do to files: one that may write or create the
/// database is admitted with its journal and write-ahead sidecars; a
/// read-only one, when the database keeps a write-ahead log (whose shared
/// memory a reader creates and changes), with those sidecars. Reading a
/// database that keeps no write-ahead log touches no file and is not
/// admitted.
///
/// The open is recorded first: from then on this process never copies the
/// file from outside SQLite ([`outside_sqlite`]).
#[allow(clippy::disallowed_methods, reason = "admitted at the fence")]
pub fn open_database(
    path: impl AsRef<Path>,
    flags: rusqlite::OpenFlags,
) -> rusqlite::Result<rusqlite::Connection> {
    admit_database(path.as_ref(), flags)?;
    record_sqlite_open(path.as_ref());
    rusqlite::Connection::open_with_flags(path, flags)
}

/// Opens the SQLite database at `path` through a `file:` URI carrying
/// `query`, admitted as [`open_database`] admits it.
#[allow(clippy::disallowed_methods, reason = "admitted at the fence")]
pub fn open_database_uri(
    path: &Path,
    query: &str,
    flags: rusqlite::OpenFlags,
) -> rusqlite::Result<rusqlite::Connection> {
    let mut uri = url::Url::from_file_path(path)
        .map_err(|()| rusqlite::Error::InvalidPath(path.to_path_buf()))?;
    uri.set_query(Some(query));
    admit_database(path, flags)?;
    record_sqlite_open(path);
    rusqlite::Connection::open_with_flags(
        uri.as_str(),
        flags | rusqlite::OpenFlags::SQLITE_OPEN_URI,
    )
}

/// The files this process has opened through SQLite, each by its resolved
/// path. Transient: rebuilt as the process opens files, never stored.
static SQLITE_OPENED: RwLock<BTreeSet<PathBuf>> = RwLock::new(BTreeSet::new());

/// Records that this process opens the file at `path` through SQLite. The
/// write lock is taken only for a file not yet recorded, so it waits for a
/// copy of that file in flight ([`outside_sqlite`]) and an open of a
/// recorded file waits for nothing.
fn record_sqlite_open(path: &Path) {
    let key = _fence::resolved(path).path;
    let recorded = SQLITE_OPENED
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .contains(&key);
    if !recorded {
        SQLITE_OPENED
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key);
    }
}

/// Runs `copy`, which opens the file at `path` outside SQLite, unless this
/// process has opened that file through SQLite: then `None`, and the file is
/// read through SQLite instead. A process's `fcntl` locks on a file belong
/// to the process, and closing any descriptor of the file releases all of
/// them; SQLite defers that close only for descriptors it opened itself, so
/// a copy would release the locks this process's connections still count
/// on. A first SQLite open of `path` waits for the copy to end.
pub fn outside_sqlite<T>(path: &Path, copy: impl FnOnce() -> T) -> Option<T> {
    let key = _fence::resolved(path).path;
    let opened = SQLITE_OPENED.read().unwrap_or_else(PoisonError::into_inner);
    if opened.contains(&key) {
        return None;
    }
    let copied = copy();
    drop(opened);
    Some(copied)
}

/// Admits what opening the database at `path` with `flags` can write.
/// SQLite follows a final symbolic link and keeps its side files beside the
/// file it reaches, so their names are derived from the resolved path.
/// Without a fence installed, nothing is resolved or read.
fn admit_database(path: &Path, flags: rusqlite::OpenFlags) -> rusqlite::Result<()> {
    if !_fence::active() {
        return Ok(());
    }
    let real = _fence::resolved(path).path;
    let sidecar = |suffix: &str| {
        let mut name = real.as_os_str().to_owned();
        name.push(suffix);
        PathBuf::from(name)
    };
    let writes = flags.intersects(
        rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_CREATE,
    );
    let mut touched = Vec::new();
    if writes {
        touched.extend([path.to_path_buf(), sidecar("-journal")]);
    }
    if writes || keeps_write_ahead_log(&real, &sidecar("-wal")) {
        touched.extend([sidecar("-wal"), sidecar("-shm")]);
    }
    for file in touched {
        _fence::admit(&file).map_err(|error| {
            rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_PERM),
                Some(error.to_string()),
            )
        })?;
    }
    Ok(())
}

/// Whether the database at `path` keeps a write-ahead log: its header says
/// so, or a log is already beside it.
fn keeps_write_ahead_log(path: &Path, wal: &Path) -> bool {
    use std::io::Read as _;
    let mut header = [0_u8; 20];
    let declared = File::open(path)
        .and_then(|mut file| file.read_exact(&mut header))
        .is_ok_and(|()| header[18] == 2 || header[19] == 2);
    declared || wal.exists()
}

/// A filesystem effect failed; names the path it concerned.
#[derive(Debug, thiserror::Error)]
pub enum FsError {
    #[error("cannot write {path}: {source}")]
    Write { path: PathBuf, source: io::Error },
    #[error("cannot lock {path}: {source}")]
    Lock { path: PathBuf, source: io::Error },
    #[error("{path} is held by another instrukt process; gave up after {} s", crate::process::whole_seconds(*waited))]
    LockTimeout { path: PathBuf, waited: Duration },
    #[error("{path} has no parent directory")]
    NoParent { path: PathBuf },
}

/// Writes `contents` to `path` atomically: a temporary file in the same
/// directory, flushed and synced, renamed over the target. Creates missing
/// parent directories.
pub fn atomic_write(path: &Path, contents: &[u8]) -> Result<(), FsError> {
    let parent = path.parent().ok_or_else(|| FsError::NoParent {
        path: path.to_path_buf(),
    })?;
    let wrap = |source: io::Error| FsError::Write {
        path: path.to_path_buf(),
        source,
    };
    create_dir_all(parent).map_err(wrap)?;
    let mut temp = temp_file_in(parent).map_err(wrap)?;
    temp.write_all(contents).map_err(wrap)?;
    temp.as_file().sync_all().map_err(wrap)?;
    persist(temp, path).map_err(|err| wrap(err.error))?;
    Ok(())
}

/// Writes `contents` to `path` atomically, as [`atomic_write`] does, with
/// `permissions` set on the file before it takes the target's place — so a
/// reader never sees the new content under other permissions (an executable
/// hook file stays executable throughout).
pub fn atomic_write_with_permissions(
    path: &Path,
    contents: &[u8],
    permissions: std::fs::Permissions,
) -> Result<(), FsError> {
    let parent = path.parent().ok_or_else(|| FsError::NoParent {
        path: path.to_path_buf(),
    })?;
    let wrap = |source: io::Error| FsError::Write {
        path: path.to_path_buf(),
        source,
    };
    create_dir_all(parent).map_err(wrap)?;
    let mut temp = temp_file_in(parent).map_err(wrap)?;
    temp.write_all(contents).map_err(wrap)?;
    temp.as_file().set_permissions(permissions).map_err(wrap)?;
    temp.as_file().sync_all().map_err(wrap)?;
    persist(temp, path).map_err(|err| wrap(err.error))?;
    Ok(())
}

/// An exclusive advisory lock on a lock file, held until dropped.
#[derive(Debug)]
pub struct LockGuard {
    // Held only for its lock; the OS releases it when the file closes.
    _file: File,
    path: PathBuf,
}

impl LockGuard {
    /// Takes the exclusive lock on `path`, waiting at most `timeout` for a
    /// holder to release it; a lock still held then is a
    /// [`FsError::LockTimeout`]. Creates the lock file and its parent
    /// directories when missing.
    pub fn acquire(path: &Path, timeout: Duration) -> Result<Self, FsError> {
        wait_for(path, timeout, || Self::try_acquire(path))
    }

    /// Takes the exclusive lock on `path` if nobody holds it; `Ok(None)`
    /// when another process (or guard) holds it.
    pub fn try_acquire(path: &Path) -> Result<Option<Self>, FsError> {
        let file = open_lock_file(path)?;
        match file.try_lock() {
            Ok(()) => Ok(Some(Self {
                _file: file,
                path: path.to_path_buf(),
            })),
            Err(TryLockError::WouldBlock) => Ok(None),
            Err(TryLockError::Error(source)) => Err(FsError::Lock {
                path: path.to_path_buf(),
                source,
            }),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Takes the exclusive lock on `path`, waiting for as long as another
    /// holder keeps it; the operating system grants it as that holder lets go.
    pub fn wait(path: &Path) -> Result<Self, FsError> {
        let file = open_lock_file(path)?;
        file.lock().map_err(|source| FsError::Lock {
            path: path.to_path_buf(),
            source,
        })?;
        Ok(Self {
            _file: file,
            path: path.to_path_buf(),
        })
    }

    /// The open lock file, for a holder that records itself in it.
    pub fn file(&self) -> &File {
        &self._file
    }
}

/// A shared advisory lock on a lock file, held until dropped. Any number
/// of shared guards coexist; none coexists with a [`LockGuard`] on the same
/// file.
#[derive(Debug)]
pub struct SharedLockGuard {
    // Held only for its lock; the OS releases it when the file closes.
    _file: File,
    path: PathBuf,
}

impl SharedLockGuard {
    /// Takes a shared lock on `path`, waiting at most `timeout` for an
    /// exclusive holder to release it; still held then is a
    /// [`FsError::LockTimeout`]. Creates the lock file and its parent
    /// directories when missing.
    pub fn acquire(path: &Path, timeout: Duration) -> Result<Self, FsError> {
        wait_for(path, timeout, || Self::try_acquire(path))
    }

    /// Takes a shared lock on `path` unless an exclusive guard holds it;
    /// `Ok(None)` when one does.
    pub fn try_acquire(path: &Path) -> Result<Option<Self>, FsError> {
        let file = open_lock_file(path)?;
        match file.try_lock_shared() {
            Ok(()) => Ok(Some(Self {
                _file: file,
                path: path.to_path_buf(),
            })),
            Err(TryLockError::WouldBlock) => Ok(None),
            Err(TryLockError::Error(source)) => Err(FsError::Lock {
                path: path.to_path_buf(),
                source,
            }),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Retries `attempt` with backoff until it yields a guard or `timeout` has
/// passed. The wait uses monotonic real time, as process limits do.
fn wait_for<G>(
    path: &Path,
    timeout: Duration,
    attempt: impl Fn() -> Result<Option<G>, FsError>,
) -> Result<G, FsError> {
    let started = Instant::now();
    let limit = crate::process::Limit::start(timeout);
    let mut pause = Duration::from_millis(5);
    loop {
        match limit.judge(attempt()?) {
            crate::process::Look::Ended(guard) => return Ok(guard),
            crate::process::Look::Passed => {
                return Err(FsError::LockTimeout {
                    path: path.to_path_buf(),
                    waited: started.elapsed(),
                });
            }
            crate::process::Look::Running => {}
        }
        std::thread::sleep(pause);
        pause = (pause * 2).min(Duration::from_millis(200));
    }
}

fn open_lock_file(path: &Path) -> Result<File, FsError> {
    let wrap = |source: io::Error| FsError::Lock {
        path: path.to_path_buf(),
        source,
    };
    if let Some(parent) = path.parent() {
        create_dir_all(parent).map_err(wrap)?;
    }
    open(
        OpenOptions::new().create(true).truncate(false).write(true),
        path,
    )
    .map_err(wrap)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_write_creates_parents_and_replaces_whole_content() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("a/b/file.yaml");
        atomic_write(&target, b"first version, long").unwrap();
        atomic_write(&target, b"second").unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "second");
        let leftovers: Vec<_> = std::fs::read_dir(target.parent().unwrap())
            .unwrap()
            .collect();
        assert_eq!(leftovers.len(), 1, "no temporary file stays behind");
    }

    #[test]
    fn a_held_lock_is_not_taken_twice_and_frees_on_drop() {
        let dir = tempfile::tempdir().unwrap();
        let lock = dir.path().join("locks/docs.lock");
        let held = LockGuard::acquire(&lock, Duration::from_secs(1)).unwrap();
        assert!(LockGuard::try_acquire(&lock).unwrap().is_none());
        let refused = LockGuard::acquire(&lock, Duration::from_millis(50));
        assert!(
            matches!(refused, Err(FsError::LockTimeout { .. })),
            "{refused:?}"
        );
        drop(held);
        // A child forked by another thread may hold a copy of the lock's
        // descriptor until it execs, so freeing is observed with a bound.
        assert!(LockGuard::acquire(&lock, Duration::from_secs(5)).is_ok());
    }

    #[test]
    fn shared_locks_coexist_and_exclude_the_exclusive_one() {
        let dir = tempfile::tempdir().unwrap();
        let lock = dir.path().join("store/.prune.lock");
        let first = SharedLockGuard::acquire(&lock, Duration::from_secs(1)).unwrap();
        let second = SharedLockGuard::try_acquire(&lock).unwrap();
        assert!(second.is_some());
        assert!(LockGuard::try_acquire(&lock).unwrap().is_none());
        drop(first);
        drop(second);
        let held = LockGuard::acquire(&lock, Duration::from_secs(5)).unwrap();
        let refused = SharedLockGuard::acquire(&lock, Duration::from_millis(50));
        assert!(
            matches!(refused, Err(FsError::LockTimeout { .. })),
            "{refused:?}"
        );
        drop(held);
        assert!(SharedLockGuard::acquire(&lock, Duration::from_secs(5)).is_ok());
    }
}
