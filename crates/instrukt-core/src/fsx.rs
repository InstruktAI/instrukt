//! Filesystem effects every context shares: whole-file atomic writes and
//! advisory locks.
//!
//! A write either lands completely or not at all — readers never see a
//! partly written file. A lock is held for the lifetime of its guard and
//! released by the operating system if the process dies.

use std::fs::{File, OpenOptions, TryLockError};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// A filesystem effect failed; names the path it concerned.
#[derive(Debug, thiserror::Error)]
pub enum FsError {
    #[error("cannot write {path}: {source}")]
    Write { path: PathBuf, source: io::Error },
    #[error("cannot lock {path}: {source}")]
    Lock { path: PathBuf, source: io::Error },
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

/// An exclusive advisory lock on a lock file, held until dropped.
#[derive(Debug)]
pub struct LockGuard {
    // Held only for its lock; the OS releases it when the file closes.
    _file: File,
    path: PathBuf,
}

impl LockGuard {
    /// Blocks until the exclusive lock on `path` is held. Creates the lock
    /// file and its parent directories when missing.
    pub fn acquire(path: &Path) -> Result<Self, FsError> {
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
        let held = LockGuard::acquire(&lock).unwrap();
        assert!(LockGuard::try_acquire(&lock).unwrap().is_none());
        drop(held);
        assert!(LockGuard::try_acquire(&lock).unwrap().is_some());
    }
}
