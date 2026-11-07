//! The repository facts the context asks git for.

use std::path::{Path, PathBuf};

use crate::git;

use super::_error::RepositoryError;

/// The root of the working tree containing `cwd`; refused outside one.
pub(crate) fn root(cwd: &Path) -> Result<PathBuf, RepositoryError> {
    git::worktree_root(cwd)?.ok_or_else(|| RepositoryError::NotInWorktree {
        dir: cwd.to_path_buf(),
    })
}

/// The repository's primary worktree (the first `git worktree list`
/// names).
pub fn primary_worktree(root: &Path) -> Result<PathBuf, RepositoryError> {
    let listing = git::run(root, ["worktree", "list", "--porcelain"])?;
    Ok(listing
        .lines()
        .find_map(|line| line.strip_prefix("worktree "))
        .map_or_else(|| root.to_path_buf(), PathBuf::from))
}

/// The repository's git directory shared by all its worktrees.
pub(crate) fn common_dir(root: &Path) -> Result<PathBuf, RepositoryError> {
    let dir = PathBuf::from(git::run(root, ["rev-parse", "--git-common-dir"])?);
    Ok(if dir.is_absolute() {
        dir
    } else {
        root.join(dir)
    })
}

/// The value of git configuration `key` as git resolves it for `root`;
/// `None` when unset.
pub(crate) fn config(root: &Path, key: &str) -> Result<Option<String>, RepositoryError> {
    let value = git::run(root, ["config", "--get", "--default", "", key])?;
    Ok(Some(value).filter(|value| !value.is_empty()))
}

/// The directory git runs hooks from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HooksDir {
    pub path: PathBuf,
    /// Whether `core.hooksPath` chose it.
    pub from_hooks_path: bool,
}

/// The hooks directory of the repository rooted at `root`: what
/// `core.hooksPath` names when it is set, otherwise the repository's own.
pub(crate) fn hooks_dir(root: &Path) -> Result<HooksDir, RepositoryError> {
    let from_hooks_path = config(root, "core.hooksPath")?.is_some();
    let path = PathBuf::from(git::run(root, ["rev-parse", "--git-path", "hooks"])?);
    let path = if path.is_absolute() {
        path
    } else {
        root.join(path)
    };
    Ok(HooksDir {
        path,
        from_hooks_path,
    })
}

/// The default branch of `remote` (`git::default_branch`).
pub(crate) fn default_branch(root: &Path, remote: &str) -> Result<String, RepositoryError> {
    Ok(git::default_branch(root, remote)?)
}

/// The branch checked out in `root`; `None` when `HEAD` is detached.
pub(crate) fn current_branch(root: &Path) -> Result<Option<String>, RepositoryError> {
    if !git::succeeds(root, ["symbolic-ref", "--quiet", "HEAD"])? {
        return Ok(None);
    }
    Ok(Some(git::run(root, ["symbolic-ref", "--short", "HEAD"])?))
}

/// The upstream of `branch`: its remote's name and the full ref of the
/// remote-tracking branch; `None` when it has none.
pub(crate) fn upstream(
    root: &Path,
    branch: &str,
) -> Result<Option<(String, String)>, RepositoryError> {
    let local = format!("refs/heads/{branch}");
    let text = git::run(
        root,
        [
            "for-each-ref",
            "--format=%(upstream:remotename)%00%(upstream)",
            local.as_str(),
        ],
    )?;
    Ok(text.split_once('\0').and_then(|(remote, tracking)| {
        (!remote.is_empty() && !tracking.is_empty())
            .then(|| (remote.to_owned(), tracking.to_owned()))
    }))
}

/// The full commit id `revision` names.
pub(crate) fn resolve_commit(root: &Path, revision: &str) -> Result<String, RepositoryError> {
    let spec = format!("{revision}^{{commit}}");
    git::run(root, ["rev-parse", "--verify", "--quiet", spec.as_str()]).map_err(|source| {
        RepositoryError::UnknownCommit {
            revision: revision.to_owned(),
            source: Box::new(source),
        }
    })
}
