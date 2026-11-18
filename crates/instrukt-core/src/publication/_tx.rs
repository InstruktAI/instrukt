//! The writes of one sync as a unit: each write remembers what it
//! replaced, so a sync that fails part-way puts the previous publication
//! back instead of leaving a mix of two.
//!
//! Nothing is written through a link. A symbolic link at one of
//! publication's own destinations is removed and the file or directory
//! written in its place, as an undoable step: a failed sync puts the link
//! back. Targets a repository or instrukt controls — outputs in the
//! project's primary worktree and the home's own tree — are
//! [`own_destination`]s: no component on the way to them may be a link.
//! An agent CLI's file in the user's home is a [`destination`]: the
//! directories on the way are the user's and are used as they resolve,
//! while a link at the file itself is replaced.

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
    /// A symbolic link at `target`, pointing at `points_to`, was removed so
    /// a file or directory could take its place.
    Link { target: PathBuf, points_to: PathBuf },
    /// The directory `target` was created where nothing stood.
    CreatedDir { target: PathBuf },
}

/// The pending writes of one sync.
#[derive(Debug)]
pub(super) struct Transaction {
    trash: TempDir,
    steps: Vec<Step>,
    moved: usize,
    /// The links this transaction replaced, by where they stood.
    replaced: Vec<PathBuf>,
}

impl Transaction {
    /// Starts a transaction whose replaced directories wait in a trash
    /// directory below `home_root` (the same filesystem as the published
    /// tree, so moving them is a rename).
    pub(super) fn begin(home_root: &Path) -> Result<Self, PublicationError> {
        crate::fsx::create_dir_all(home_root).map_err(|source| PublicationError::Io {
            path: home_root.to_path_buf(),
            source,
        })?;
        let trash =
            crate::fsx::temp_dir_in(tempfile::Builder::new().prefix(TRASH_PREFIX), home_root)
                .map_err(|source| PublicationError::Io {
                    path: home_root.to_path_buf(),
                    source,
                })?;
        Ok(Self {
            trash,
            steps: Vec::new(),
            moved: 0,
            replaced: Vec::new(),
        })
    }

    /// The links this transaction replaced so far, by where they stood.
    pub(super) fn replaced_links(&self) -> &[PathBuf] {
        &self.replaced
    }

    /// Whether anything was written, replaced or removed so far.
    pub(super) fn changed(&self) -> bool {
        !self.steps.is_empty()
    }

    /// Removes a symbolic link standing at `target`, dangling or not, so a
    /// file or directory can take its place; the file it points to is left
    /// as it is. Undone by making the link again. Whether a link stood there.
    pub(super) fn unlink(&mut self, target: &Path) -> Result<bool, PublicationError> {
        let io = |source| PublicationError::Io {
            path: target.to_path_buf(),
            source,
        };
        match std::fs::symlink_metadata(target) {
            Ok(meta) if meta.file_type().is_symlink() => {
                let points_to = std::fs::read_link(target).map_err(io)?;
                crate::fsx::remove_file(target).map_err(io)?;
                self.steps.push(Step::Link {
                    target: target.to_path_buf(),
                    points_to,
                });
                self.replaced.push(target.to_path_buf());
                Ok(true)
            }
            Ok(_) => Ok(false),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(source) => Err(io(source)),
        }
    }

    /// Writes `contents` to `target` atomically, unless it already holds
    /// exactly that; a link at `target` is replaced by the file. Whether it
    /// wrote.
    pub(super) fn write(
        &mut self,
        target: &Path,
        contents: &[u8],
    ) -> Result<bool, PublicationError> {
        self.unlink(target)?;
        let previous = read_existing(target)?;
        if previous.as_deref() == Some(contents) {
            return Ok(false);
        }
        atomic_write(target, contents)?;
        self.steps.push(Step::File {
            target: target.to_path_buf(),
            previous,
        });
        Ok(true)
    }

    /// Makes `target` a directory: a link there is replaced, and a missing
    /// directory is created, with its parents. Undone by removing what was
    /// created and making the link again.
    pub(super) fn ensure_dir(&mut self, target: &Path) -> Result<(), PublicationError> {
        self.unlink(target)?;
        match std::fs::symlink_metadata(target) {
            Ok(_) => return Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(PublicationError::Io {
                    path: target.to_path_buf(),
                    source,
                });
            }
        }
        crate::fsx::create_dir_all(target).map_err(|source| PublicationError::Io {
            path: target.to_path_buf(),
            source,
        })?;
        self.steps.push(Step::CreatedDir {
            target: target.to_path_buf(),
        });
        Ok(())
    }

    /// Removes the file `target` when it exists; a link there is removed
    /// and the file it points to left as it is.
    pub(super) fn remove_file(&mut self, target: &Path) -> Result<(), PublicationError> {
        if self.unlink(target)? {
            return Ok(());
        }
        let Some(previous) = read_existing(target)? else {
            return Ok(());
        };
        crate::fsx::remove_file(target).map_err(|source| PublicationError::Io {
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
    /// earlier `target` in the trash, unless `target` already holds exactly
    /// the same files with the same bytes; a link at `target` is replaced by
    /// the directory. Both lie on the home's filesystem. Whether it wrote.
    pub(super) fn replace_dir(
        &mut self,
        staged: &Path,
        target: &Path,
    ) -> Result<bool, PublicationError> {
        self.unlink(target)?;
        if same_tree(staged, target)? {
            return Ok(false);
        }
        let saved = self.stash(target)?;
        if let Some(parent) = target.parent() {
            crate::fsx::create_dir_all(parent).map_err(|source| PublicationError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        if let Err(source) = crate::fsx::rename(staged, target) {
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
        Ok(true)
    }

    /// Removes the directory `target`, keeping it in the trash; a link there
    /// is removed and the directory it points to left as it is.
    pub(super) fn remove_dir(&mut self, target: &Path) -> Result<(), PublicationError> {
        if self.unlink(target)? {
            return Ok(());
        }
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
        crate::fsx::rename(target, &saved).map_err(|source| PublicationError::Io {
            path: target.to_path_buf(),
            source,
        })?;
        Ok(Some(saved))
    }

    /// Keeps every write. Past this point the sync has published, so
    /// deleting the trash cannot fail it: a trash that cannot be deleted
    /// is returned for the caller to report, and [`sweep`] deletes it on a
    /// later sync.
    pub(super) fn commit(self) -> Option<PathBuf> {
        let path = self.trash.keep();
        match crate::fsx::remove_dir_all(&path) {
            Ok(()) => None,
            Err(error) => {
                tracing::warn!(event = "sync_trash_left", path = %path.display(), %error);
                Some(path)
            }
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
                        None => crate::fsx::remove_file(&target).map_err(|e| e.to_string()),
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
                Step::CreatedDir { target } => {
                    if let Err(error) = remove_any(&target) {
                        tracing::error!(event = "sync_rollback_failed", path = %target.display(), %error);
                    }
                }
                Step::Link { target, points_to } => {
                    let relinked =
                        remove_any(&target).and_then(|()| crate::fsx::symlink(&points_to, &target));
                    if let Err(error) = relinked {
                        tracing::error!(event = "sync_rollback_failed", path = %target.display(), %error);
                    }
                }
            }
        }
    }
}

fn restore_dir(saved: &Path, target: &Path) {
    if let Err(error) = crate::fsx::rename(saved, target) {
        tracing::error!(event = "sync_rollback_failed", path = %target.display(), %error);
    }
}

fn remove_any(path: &Path) -> std::io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() => crate::fsx::remove_dir_all(path),
        Ok(_) => crate::fsx::remove_file(path),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    }
}

/// Whether the directory `target` holds exactly the files `staged` holds,
/// each with the same bytes, and nothing else: no link, no other file.
fn same_tree(staged: &Path, target: &Path) -> Result<bool, PublicationError> {
    match std::fs::symlink_metadata(target) {
        Ok(meta) if meta.is_dir() => {}
        Ok(_) => return Ok(false),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(source) => {
            return Err(PublicationError::Io {
                path: target.to_path_buf(),
                source,
            });
        }
    }
    Ok(tree(staged)? == tree(target)?)
}

/// Every entry below `root` by its path relative to `root`: a file with its
/// bytes, a directory, or a link with where it points.
fn tree(root: &Path) -> Result<std::collections::BTreeMap<PathBuf, Entry>, PublicationError> {
    let mut entries = std::collections::BTreeMap::new();
    for entry in walkdir::WalkDir::new(root).min_depth(1) {
        let entry = entry.map_err(|err| PublicationError::Io {
            path: root.to_path_buf(),
            source: err.into(),
        })?;
        let io = |source| PublicationError::Io {
            path: entry.path().to_path_buf(),
            source,
        };
        let kind = entry.file_type();
        let value = if kind.is_symlink() {
            Entry::Link(std::fs::read_link(entry.path()).map_err(io)?)
        } else if kind.is_dir() {
            Entry::Dir
        } else {
            Entry::File(std::fs::read(entry.path()).map_err(io)?)
        };
        let rel = entry
            .path()
            .strip_prefix(root)
            .map(Path::to_path_buf)
            .unwrap_or_default();
        entries.insert(rel, value);
    }
    Ok(entries)
}

/// One entry of a compared tree.
#[derive(Debug, PartialEq, Eq)]
enum Entry {
    File(Vec<u8>),
    Dir,
    Link(PathBuf),
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

/// The declared output `root/rel` publication writes, replaces or removes,
/// with `root` resolved: refused when `rel` climbs out, or when a component
/// on the way to it is a symbolic link. A link at the output itself is no
/// refusal: the transaction replaces it.
pub(super) fn own_destination(root: &Path, rel: &Path) -> Result<PathBuf, PublicationError> {
    let Some(name) = rel.file_name() else {
        return Err(PublicationError::Unconfined {
            path: root.join(rel),
        });
    };
    let parent = rel.parent().unwrap_or(Path::new(""));
    Ok(confined(root, parent)?.join(name))
}

/// Where the global agent file or settings file `root/rel` is written: the
/// directories on the way are the user's, resolved through their links as
/// [`resolved`] does; the file itself is never followed, so a link there,
/// dangling or not, is what the transaction replaces.
pub(super) fn destination(root: &Path, rel: &Path) -> Result<PathBuf, PublicationError> {
    let Some(name) = rel.file_name() else {
        return Err(PublicationError::Unconfined {
            path: root.join(rel),
        });
    };
    let parent = rel.parent().unwrap_or(Path::new(""));
    Ok(resolved(root, parent)?.join(name))
}

/// Where the user-owned file `root/rel` really lives: every existing
/// component, the file itself included, resolved through the user's
/// symbolic links (a dotfiles link stays a link; the write lands at its
/// target). The missing rest of the path is appended as written. A link
/// that leads nowhere is refused, naming it. `rel` climbing out of `root`
/// is refused.
pub(super) fn resolved(root: &Path, rel: &Path) -> Result<PathBuf, PublicationError> {
    let mut path = root.to_path_buf();
    let parts: Vec<&std::ffi::OsStr> = rel
        .components()
        .map(|component| match component {
            std::path::Component::Normal(part) => Ok(part),
            _ => Err(PublicationError::Unconfined {
                path: root.join(rel),
            }),
        })
        .collect::<Result<_, _>>()?;
    for (at, part) in parts.iter().enumerate() {
        let candidate = path.join(part);
        match std::fs::symlink_metadata(&candidate) {
            Ok(_) => {
                path = candidate.canonicalize().map_err(|source| {
                    if source.kind() == std::io::ErrorKind::NotFound {
                        PublicationError::DanglingLink {
                            path: candidate.clone(),
                        }
                    } else {
                        PublicationError::Io {
                            path: candidate.clone(),
                            source,
                        }
                    }
                })?;
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return Ok(parts[at..].iter().fold(path, |path, part| path.join(part)));
            }
            Err(source) => {
                return Err(PublicationError::Io {
                    path: candidate,
                    source,
                });
            }
        }
    }
    Ok(path)
}

/// Deletes what earlier syncs left below `home_root` — trash a finished
/// sync could not delete, staging an interrupted one never moved. Callers
/// hold the docs lock. What cannot be deleted is returned, sorted, and left.
pub(super) fn sweep(home_root: &Path) -> Result<Vec<PathBuf>, PublicationError> {
    let entries = match std::fs::read_dir(home_root) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => {
            return Err(PublicationError::Io {
                path: home_root.to_path_buf(),
                source,
            });
        }
    };
    let mut kept = Vec::new();
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
            kept.push(path);
        }
    }
    kept.sort();
    Ok(kept)
}
