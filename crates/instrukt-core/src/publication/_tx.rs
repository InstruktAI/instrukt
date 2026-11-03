//! The writes of one sync as a unit: each write remembers what it
//! replaced, so a sync that fails part-way puts the previous publication
//! back instead of leaving a mix of two.
//!
//! Every target is confined to a root (the home, the project's primary
//! worktree or the user's home directory): no existing component between
//! the root and the target may be a symbolic link, so nothing is written
//! through a link out of its root.

use std::path::{Path, PathBuf};

use tempfile::TempDir;

use super::PublicationError;
use crate::fsx::atomic_write;

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
            .prefix(".sync-trash-")
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

    /// Keeps every write; the trash is deleted.
    pub(super) fn commit(self) -> Result<(), PublicationError> {
        let path = self.trash.path().to_path_buf();
        self.trash
            .close()
            .map_err(|source| PublicationError::Io { path, source })
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

/// Where `root/rel` resolves: every existing component is resolved through
/// its symbolic links, and must stay inside `root` (resolved likewise);
/// `rel` climbing out, or a link leading out, is refused. The missing rest
/// of the path is appended as written.
pub(super) fn confined(root: &Path, rel: &Path) -> Result<PathBuf, PublicationError> {
    let unconfined = || PublicationError::Unconfined {
        path: root.join(rel),
    };
    let parts: Vec<&std::ffi::OsStr> = rel
        .components()
        .map(|component| match component {
            std::path::Component::Normal(part) => Ok(part),
            _ => Err(unconfined()),
        })
        .collect::<Result<_, _>>()?;
    let base = match root.canonicalize() {
        Ok(base) => base,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(root.join(rel)),
        Err(source) => {
            return Err(PublicationError::Io {
                path: root.to_path_buf(),
                source,
            });
        }
    };
    let mut resolved = base.clone();
    for (at, part) in parts.iter().enumerate() {
        let candidate = resolved.join(part);
        match std::fs::symlink_metadata(&candidate) {
            Ok(_) => {
                resolved = candidate
                    .canonicalize()
                    .map_err(|source| PublicationError::Io {
                        path: candidate.clone(),
                        source,
                    })?;
                if !resolved.starts_with(&base) {
                    return Err(PublicationError::Unconfined { path: candidate });
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return Ok(parts[at..]
                    .iter()
                    .fold(resolved, |path, part| path.join(part)));
            }
            Err(source) => {
                return Err(PublicationError::Io {
                    path: candidate,
                    source,
                });
            }
        }
    }
    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rollback_restores_files_and_directories() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let file = root.join("docs/index.yaml");
        atomic_write(&file, b"old").unwrap();
        let domain = root.join("docs/alpha");
        atomic_write(&domain.join("a.md"), b"old a").unwrap();
        let staged = root.join("staged");
        atomic_write(&staged.join("b.md"), b"new b").unwrap();
        let gone = root.join("docs/beta");
        atomic_write(&gone.join("c.md"), b"c").unwrap();

        let mut tx = Transaction::begin(root).unwrap();
        tx.write(&file, b"new").unwrap();
        tx.replace_dir(&staged, &domain).unwrap();
        tx.remove_dir(&gone).unwrap();
        tx.write(&root.join("docs/fresh.md"), b"fresh").unwrap();
        assert_eq!(std::fs::read(domain.join("b.md")).unwrap(), b"new b");
        tx.rollback();

        assert_eq!(std::fs::read(&file).unwrap(), b"old");
        assert_eq!(std::fs::read(domain.join("a.md")).unwrap(), b"old a");
        assert!(!domain.join("b.md").exists());
        assert_eq!(std::fs::read(gone.join("c.md")).unwrap(), b"c");
        assert!(!root.join("docs/fresh.md").exists());
    }

    #[cfg(unix)]
    #[test]
    fn a_symbolic_link_out_of_the_root_is_refused_and_one_inside_is_followed() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), root.join("out")).unwrap();
        std::fs::create_dir(root.join("real")).unwrap();
        std::os::unix::fs::symlink(root.join("real"), root.join("in")).unwrap();
        assert!(matches!(
            confined(&root, Path::new("out/AGENTS.md")),
            Err(PublicationError::Unconfined { .. })
        ));
        assert_eq!(
            confined(&root, Path::new("in/AGENTS.md")).unwrap(),
            root.join("real/AGENTS.md")
        );
        assert!(matches!(
            confined(&root, Path::new("../x")),
            Err(PublicationError::Unconfined { .. })
        ));
    }
}
