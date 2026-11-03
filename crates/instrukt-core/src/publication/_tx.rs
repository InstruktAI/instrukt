//! The writes of one sync as a unit: each write remembers what it
//! replaced, so a sync that fails part-way puts the previous publication
//! back instead of leaving a mix of two.
//!
//! Every target is confined to a root (the home, the project's primary
//! worktree or the user's home directory): no existing component between
//! the root and the target may be a symbolic link, so nothing is written
//! through a link to another file.

use std::path::{Path, PathBuf};

use tempfile::TempDir;

use super::PublicationError;
use crate::fsx::atomic_write;

/// The name prefix of a sync's trash directory below the home root.
const TRASH_PREFIX: &str = ".sync-trash-";

/// The name prefix of a sync's staging directory below the home root.
pub(super) const STAGING_PREFIX: &str = ".sync-staging-";

/// One undoable step.
#[derive(Debug)]
enum Step {
    /// A file was written; `previous` held its earlier content.
    File {
        target: PathBuf,
        previous: Option<Vec<u8>>,
    },
    /// A directory took the place of `target`; its earlier version, if
    /// any, waits in the trash at `saved`.
    Dir {
        target: PathBuf,
        saved: Option<PathBuf>,
    },
    /// A file was removed; it held `previous`.
    RemovedFile { target: PathBuf, previous: Vec<u8> },
}

/// The pending writes of one sync.
#[derive(Debug)]
pub(super) struct Transaction {
    trash: TempDir,
    steps: Vec<Step>,
    moved: usize,
}

impl Transaction {
    /// Starts a transaction whose replaced directories wait in a trash
    /// directory below `home_root` (the same filesystem as the published
    /// tree, so moving them is a rename).
    pub(super) fn begin(home_root: &Path) -> Result<Self, PublicationError> {
        std::fs::create_dir_all(home_root).map_err(|source| PublicationError::Io {
            path: home_root.to_path_buf(),
            source,
        })?;
        let trash = tempfile::Builder::new()
            .prefix(TRASH_PREFIX)
            .tempdir_in(home_root)
            .map_err(|source| PublicationError::Io {
                path: home_root.to_path_buf(),
                source,
            })?;
        Ok(Self {
            trash,
            steps: Vec::new(),
            moved: 0,
        })
    }

    /// Writes `contents` to `target` atomically, unless it already holds
    /// exactly that.
    pub(super) fn write(&mut self, target: &Path, contents: &[u8]) -> Result<(), PublicationError> {
        let previous = read_existing(target)?;
        if previous.as_deref() == Some(contents) {
            return Ok(());
        }
        atomic_write(target, contents)?;
        self.steps.push(Step::File {
            target: target.to_path_buf(),
            previous,
        });
        Ok(())
    }

    /// Removes the file `target` when it exists.
    pub(super) fn remove_file(&mut self, target: &Path) -> Result<(), PublicationError> {
        let Some(previous) = read_existing(target)? else {
            return Ok(());
        };
        std::fs::remove_file(target).map_err(|source| PublicationError::Io {
            path: target.to_path_buf(),
            source,
        })?;
        self.steps.push(Step::RemovedFile {
            target: target.to_path_buf(),
            previous,
        });
        Ok(())
    }

    /// Puts the directory `staged` in the place of `target`, keeping the
    /// earlier `target` in the trash. Both lie on the home's filesystem.
    pub(super) fn replace_dir(
        &mut self,
        staged: &Path,
        target: &Path,
    ) -> Result<(), PublicationError> {
        let saved = self.stash(target)?;
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|source| PublicationError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        if let Err(source) = std::fs::rename(staged, target) {
            if let Some(saved) = &saved {
                restore_dir(saved, target);
            }
            return Err(PublicationError::Io {
                path: target.to_path_buf(),
                source,
            });
        }
        self.steps.push(Step::Dir {
            target: target.to_path_buf(),
            saved,
        });
        Ok(())
    }

    /// Removes the directory `target`, keeping it in the trash.
    pub(super) fn remove_dir(&mut self, target: &Path) -> Result<(), PublicationError> {
        if let Some(saved) = self.stash(target)? {
            // Undone like a replacement whose new directory is absent.
            self.steps.push(Step::Dir {
                target: target.to_path_buf(),
                saved: Some(saved),
            });
        }
        Ok(())
    }

    fn stash(&mut self, target: &Path) -> Result<Option<PathBuf>, PublicationError> {
        match std::fs::symlink_metadata(target) {
            Ok(_) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(source) => {
                return Err(PublicationError::Io {
                    path: target.to_path_buf(),
                    source,
                });
            }
        }
        self.moved += 1;
        let saved = self.trash.path().join(self.moved.to_string());
        std::fs::rename(target, &saved).map_err(|source| PublicationError::Io {
            path: target.to_path_buf(),
            source,
        })?;
        Ok(Some(saved))
    }

    /// Keeps every write. Past this point the sync has published, so
    /// deleting the trash cannot fail it: a trash that cannot be deleted
    /// is reported and left for [`sweep`] on a later sync.
    pub(super) fn commit(self) {
        let path = self.trash.keep();
        if let Err(error) = std::fs::remove_dir_all(&path) {
            tracing::warn!(
                event = "sync_trash_left",
                path = %path.display(),
                %error,
            );
        }
    }

    /// Undoes every write, newest first. Undo is best effort: a step that
    /// cannot be undone is logged and the rest are still undone.
    pub(super) fn rollback(self) {
        for step in self.steps.into_iter().rev() {
            match step {
                Step::File { target, previous } => {
                    let undone = match previous {
                        Some(previous) => {
                            atomic_write(&target, &previous).map_err(|e| e.to_string())
                        }
                        None => std::fs::remove_file(&target).map_err(|e| e.to_string()),
                    };
                    if let Err(error) = undone {
                        tracing::error!(event = "sync_rollback_failed", path = %target.display(), %error);
                    }
                }
                Step::RemovedFile { target, previous } => {
                    if let Err(error) = atomic_write(&target, &previous) {
                        tracing::error!(event = "sync_rollback_failed", path = %target.display(), %error);
                    }
                }
                Step::Dir { target, saved } => {
                    if let Err(error) = remove_any(&target) {
                        tracing::error!(event = "sync_rollback_failed", path = %target.display(), %error);
                    }
                    if let Some(saved) = saved {
                        restore_dir(&saved, &target);
                    }
                }
            }
        }
    }
}

fn restore_dir(saved: &Path, target: &Path) {
    if let Err(error) = std::fs::rename(saved, target) {
        tracing::error!(event = "sync_rollback_failed", path = %target.display(), %error);
    }
}

fn remove_any(path: &Path) -> std::io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() => std::fs::remove_dir_all(path),
        Ok(_) => std::fs::remove_file(path),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    }
}

/// The content of the regular file `target`; `None` when absent. A
/// symbolic link or a directory in its place is refused.
fn read_existing(target: &Path) -> Result<Option<Vec<u8>>, PublicationError> {
    match std::fs::symlink_metadata(target) {
        Ok(meta) if meta.is_file() => {
            std::fs::read(target)
                .map(Some)
                .map_err(|source| PublicationError::Io {
                    path: target.to_path_buf(),
                    source,
                })
        }
        Ok(_) => Err(PublicationError::Unconfined {
            path: target.to_path_buf(),
        }),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(PublicationError::Io {
            path: target.to_path_buf(),
            source,
        }),
    }
}

/// The declared output `root/rel`, with `root` resolved. Refused when
/// `rel` climbs out, or when any existing component below the root — the
/// output itself included — is a symbolic link: an output is written,
/// replaced or removed only as the object it was declared to be, never
/// through an alias of another file.
pub(super) fn confined(root: &Path, rel: &Path) -> Result<PathBuf, PublicationError> {
    let unconfined = |path: PathBuf| PublicationError::Unconfined { path };
    let base = match root.canonicalize() {
        Ok(base) => base,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => root.to_path_buf(),
        Err(source) => {
            return Err(PublicationError::Io {
                path: root.to_path_buf(),
                source,
            });
        }
    };
    let mut path = base;
    let mut exists = true;
    for component in rel.components() {
        let std::path::Component::Normal(part) = component else {
            return Err(unconfined(root.join(rel)));
        };
        path.push(part);
        if !exists {
            continue;
        }
        match std::fs::symlink_metadata(&path) {
            Ok(meta) if meta.file_type().is_symlink() => return Err(unconfined(path)),
            Ok(_) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => exists = false,
            Err(source) => return Err(PublicationError::Io { path, source }),
        }
    }
    Ok(path)
}

/// Deletes what earlier syncs left below `home_root` — trash a finished
/// sync could not delete, staging an interrupted one never moved. Callers
/// hold the docs lock. What cannot be deleted is reported and left.
pub(super) fn sweep(home_root: &Path) -> Result<(), PublicationError> {
    let entries = match std::fs::read_dir(home_root) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(source) => {
            return Err(PublicationError::Io {
                path: home_root.to_path_buf(),
                source,
            });
        }
    };
    for entry in entries {
        let entry = entry.map_err(|source| PublicationError::Io {
            path: home_root.to_path_buf(),
            source,
        })?;
        let name = entry.file_name();
        let leftover = name
            .to_str()
            .is_some_and(|n| n.starts_with(TRASH_PREFIX) || n.starts_with(STAGING_PREFIX));
        if !leftover {
            continue;
        }
        let path = entry.path();
        if let Err(error) = remove_any(&path) {
            tracing::warn!(event = "sync_leftover_kept", path = %path.display(), %error);
        }
    }
    Ok(())
}
