//! Where a file instrukt writes in a repository really is. A path inside
//! the working tree can be a symlink, or sit below one, that leads out of
//! it; instrukt resolves the leaf and every ancestor before it reads or
//! writes, refuses a target outside the places it may write, and writes to
//! the resolved file so a link inside them stays a link.

use std::path::{Component, Path, PathBuf};

use super::_error::RepositoryError;

/// How many symlinks one resolution follows before it gives up.
const LINK_LIMIT: usize = 40;

/// The canonical directories a write may land in.
#[derive(Debug, Clone)]
pub(crate) struct Bounds(Vec<PathBuf>);

impl Bounds {
    /// The bounds `dirs`, each canonicalized.
    pub(crate) fn of(dirs: &[&Path]) -> Result<Self, RepositoryError> {
        dirs.iter()
            .map(|dir| {
                std::fs::canonicalize(dir).map_err(|source| RepositoryError::Read {
                    path: dir.to_path_buf(),
                    source,
                })
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Self)
    }

    /// The file `path` names once every symlink of its leaf and ancestors is
    /// followed (a missing leaf resolves to where it would be created);
    /// refused when that lies outside every bound.
    pub(crate) fn resolve(&self, path: &Path) -> Result<PathBuf, RepositoryError> {
        let target = resolve(path, LINK_LIMIT)?;
        if self.0.iter().any(|bound| target.starts_with(bound)) {
            Ok(target)
        } else {
            Err(RepositoryError::Escapes {
                path: path.to_path_buf(),
                target,
            })
        }
    }
}

fn resolve(path: &Path, links_left: usize) -> Result<PathBuf, RepositoryError> {
    let read_error = |source| RepositoryError::Read {
        path: path.to_path_buf(),
        source,
    };
    match std::fs::canonicalize(path) {
        Ok(resolved) => return Ok(resolved),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(source) => return Err(read_error(source)),
    }
    // The leaf (or an ancestor) does not exist: resolve the parent, then
    // follow the leaf when it is a dangling link.
    let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
        return Ok(path.to_path_buf());
    };
    let parent = resolve(parent, links_left)?;
    let candidate = parent.join(name);
    match std::fs::symlink_metadata(&candidate) {
        Ok(meta) if meta.file_type().is_symlink() => {
            if links_left == 0 {
                return Err(RepositoryError::Escapes {
                    path: path.to_path_buf(),
                    target: candidate,
                });
            }
            let link = std::fs::read_link(&candidate).map_err(read_error)?;
            resolve(&normalize(&parent.join(link)), links_left - 1)
        }
        Ok(_) => Ok(candidate),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(candidate),
        Err(source) => Err(read_error(source)),
    }
}

/// `path` with `.` and `..` folded lexically, for a link target that
/// names a place that does not exist yet.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}
