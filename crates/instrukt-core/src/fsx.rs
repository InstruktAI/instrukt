//! Filesystem effects every context shares: whole-file atomic writes and
//! advisory locks.
//!
//! A write either lands completely or not at all — readers never see a
//! partly written file. A lock is held for the lifetime of its guard and
//! released by the operating system if the process dies.

use std::fs::{File, OpenOptions, TryLockError};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// A filesystem effect failed; names the path it concerned.
#[derive(Debug, thiserror::Error)]
pub enum FsError {
    #[error("cannot write {path}: {source}")]
    Write { path: PathBuf, source: io::Error },
    #[error("cannot lock {path}: {source}")]
    Lock { path: PathBuf, source: io::Error },
    #[error("{path} is held by another instrukt process; gave up after {waited:?}")]
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
    std::fs::create_dir_all(parent).map_err(wrap)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(wrap)?;
    temp.write_all(contents).map_err(wrap)?;
    temp.as_file().sync_all().map_err(wrap)?;
    temp.persist(path).map_err(|err| wrap(err.error))?;
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
    std::fs::create_dir_all(parent).map_err(wrap)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(wrap)?;
    temp.write_all(contents).map_err(wrap)?;
    temp.as_file().set_permissions(permissions).map_err(wrap)?;
    temp.as_file().sync_all().map_err(wrap)?;
    temp.persist(path).map_err(|err| wrap(err.error))?;
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

/// Retries `attempt` with backoff until it yields a guard or `timeout`
/// has passed.
fn wait_for<G>(
    path: &Path,
    timeout: Duration,
    attempt: impl Fn() -> Result<Option<G>, FsError>,
) -> Result<G, FsError> {
    let started = Instant::now();
    let mut pause = Duration::from_millis(5);
    loop {
        if let Some(guard) = attempt()? {
            return Ok(guard);
        }
        let waited = started.elapsed();
        if waited >= timeout {
            return Err(FsError::LockTimeout {
                path: path.to_path_buf(),
                waited,
            });
        }
        std::thread::sleep(pause.min(timeout - waited));
        pause = (pause * 2).min(Duration::from_millis(200));
    }
}

fn open_lock_file(path: &Path) -> Result<File, FsError> {
    let wrap = |source: io::Error| FsError::Lock {
        path: path.to_path_buf(),
        source,
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(wrap)?;
    }
    OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)
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
