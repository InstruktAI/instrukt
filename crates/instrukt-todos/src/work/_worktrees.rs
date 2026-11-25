//! What git shows of a project's worktrees, for the work context's releases
//! of a todo's worktree: git's worktree list, whether git keeps a worktree
//! by its lock or it holds uncommitted work, the staging location beside a
//! worktree that one rename moves it to, and whether anything is at a path.

use std::path::{Path, PathBuf};

use instrukt_core::git;
use instrukt_core::ids::CommitSha;

use super::_error::WorkError;
use super::_model::TodoId;
use super::_store::Checkout;

/// A worktree of the project as git lists it.
#[derive(Debug, Clone)]
pub(super) struct Worktree {
    pub(super) path: PathBuf,
    /// The commit it has checked out.
    pub(super) head: Option<CommitSha>,
    /// The branch it has checked out.
    pub(super) branch: Option<String>,
    /// Why it is locked, when it is (`""` for no reason given).
    pub(super) locked: Option<String>,
    /// Whether git reports it prunable: the directory git records for it is
    /// gone.
    pub(super) prunable: bool,
}

impl Worktree {
    /// What it has checked out: its branch, else the commit it is
    /// detached at.
    pub(super) fn checkout(&self) -> Option<Checkout> {
        match (&self.branch, &self.head) {
            (Some(branch), _) => Some(Checkout::Branch(branch.clone())),
            (None, Some(head)) => Some(Checkout::Detached(head.clone())),
            (None, None) => None,
        }
    }
}

/// The project's worktrees, the primary one first.
pub(super) fn worktrees(root: &Path) -> Result<Vec<Worktree>, git::GitError> {
    Ok(parse_worktrees(&git::run(
        root,
        ["worktree", "list", "--porcelain"],
    )?))
}

/// The worktrees `git worktree list --porcelain` printed as `listed`.
pub(super) fn parse_worktrees(listed: &str) -> Vec<Worktree> {
    let mut found: Vec<Worktree> = Vec::new();
    for line in listed.lines() {
        if let Some(path) = line.strip_prefix("worktree ") {
            found.push(Worktree {
                path: PathBuf::from(path),
                head: None,
                branch: None,
                locked: None,
                prunable: false,
            });
        } else if let Some(last) = found.last_mut() {
            if let Some(head) = line.strip_prefix("HEAD ") {
                last.head = CommitSha::parse(head).ok();
            } else if let Some(branch) = line.strip_prefix("branch refs/heads/") {
                last.branch = Some(branch.to_owned());
            } else if line == "locked" {
                last.locked = Some(String::new());
            } else if let Some(reason) = line.strip_prefix("locked ") {
                last.locked = Some(reason.to_owned());
            } else if line == "prunable" || line.starts_with("prunable ") {
                last.prunable = true;
            }
        }
    }
    found
}

/// Why git keeps a worktree from being released.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Kept {
    /// It is locked, with the reason git records (`""` for none given).
    Locked(String),
    /// It holds uncommitted changes.
    Uncommitted,
}

/// Whether git keeps `worktree` by its lock or it holds uncommitted work. A
/// status git could not read is no answer.
pub(super) fn kept(worktree: &Worktree) -> Result<Option<Kept>, git::GitError> {
    if let Some(reason) = &worktree.locked {
        return Ok(Some(Kept::Locked(reason.clone())));
    }
    let changes = git::run(&worktree.path, ["status", "--porcelain"])?;
    if changes.trim().is_empty() {
        Ok(None)
    } else {
        Ok(Some(Kept::Uncommitted))
    }
}

/// Where a release of `todo` moves the worktree at `path`: beside it, in the
/// same directory, so git moves it with one rename, under a name that starts
/// with `prefix`. None when `path` has no directory beside it.
pub(super) fn staging_path(path: &Path, prefix: &str, todo: &TodoId) -> Option<PathBuf> {
    match (path.parent(), path.file_name()) {
        (Some(parent), Some(_)) => Some(parent.join(staging_name(path, prefix, todo))),
        _ => None,
    }
}

/// The name of the staging location of the worktree at `path`: `prefix`, the
/// todo's identity and a digest of the worktree's whole path, so it has the
/// same short length whatever the worktree is called — well within the 255
/// bytes a file name may have — and differs for each worktree of the todo
/// in one directory.
fn staging_name(path: &Path, prefix: &str, todo: &TodoId) -> String {
    use sha2::Digest as _;
    let digest = sha2::Sha256::digest(path.as_os_str().as_encoded_bytes());
    format!("{prefix}{}-{}", todo.as_str(), hex::encode(&digest[..8]))
}

/// Whether anything is at `path`; a look that fails is no answer.
pub(super) fn exists(path: &Path) -> std::io::Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(err),
    }
}

/// The commit ref `reference` points at; `None` when git reports no such
/// ref. A failure to read it is an error.
pub(super) fn ref_tip(root: &Path, reference: &str) -> Result<Option<CommitSha>, WorkError> {
    let listed = git::run(
        root,
        [
            "for-each-ref",
            "--format=%(objectname) %(refname)",
            reference,
        ],
    )?;
    for line in listed.lines() {
        if let Some((sha, name)) = line.split_once(' ')
            && name == reference
        {
            let sha = CommitSha::parse(sha).map_err(|err| {
                WorkError::refused(format!(
                    "git reports {reference} at {sha}, not a commit: {err}"
                ))
            })?;
            return Ok(Some(sha));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn todo() -> TodoId {
        TodoId::parse(&"0".repeat(32)).unwrap()
    }

    #[test]
    fn a_worktree_git_reports_prunable_is_parsed_prunable() {
        let listed = parse_worktrees(
            "worktree /repo\nHEAD aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\nbranch refs/heads/main\n\nworktree /work/tree\nHEAD aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\ndetached\nprunable gitdir file points to non-existent location\n",
        );
        assert_eq!(
            listed.iter().map(|w| w.prunable).collect::<Vec<_>>(),
            [false, true]
        );
    }

    #[test]
    fn the_staging_name_of_a_worktree_named_at_the_file_name_limit_fits_in_a_file_name() {
        let path = PathBuf::from("/work").join("w".repeat(240));
        assert!(staging_name(&path, ".instrukt-wd-", &todo()).len() <= 255);
    }

    #[test]
    fn worktrees_of_one_todo_in_one_directory_have_distinct_staging_names() {
        let (one, two) = (PathBuf::from("/work/one"), PathBuf::from("/work/two"));
        assert_ne!(
            staging_name(&one, ".instrukt-wd-", &todo()),
            staging_name(&two, ".instrukt-wd-", &todo())
        );
    }
}
