//! Moving the review notes ref between machines. instrukt pushes and fetches
//! `refs/notes/instrukt-review` explicitly and never changes a remote's
//! refspecs. Notes from another machine are merged into the local ref with
//! `cat_sort_uniq`, so every stamp of every machine survives.

use std::path::Path;

use crate::git;

use super::_error::RepositoryError;
use super::_git;

/// The notes ref commit-review stamps are written to.
pub const REVIEW_NOTES_REF: &str = "refs/notes/instrukt-review";

/// How many times a rejected push is merged and tried again.
const PUSH_ATTEMPTS: usize = 3;

/// What became of pushing the notes ref.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotesPush {
    /// The remote holds every local note.
    Pushed { remote: String },
    /// The push did not land; the notes are kept locally and the next push
    /// that lands carries them, since pushing the ref carries every note.
    Pending {
        remote: String,
        reason: PendingReason,
    },
    /// The default branch tracks no remote; the notes stay local.
    NoRemote { branch: String },
}

/// Why a notes push is pending.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingReason {
    /// The remote could not be reached.
    Unreachable,
    /// The remote kept rejecting the push after every merge attempt.
    StillRejected,
}

/// What became of fetching the notes ref before reading verdicts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotesFetch {
    /// The remote's notes were merged into the local ones (or it has none).
    Fetched { remote: String },
    /// The fetch failed; the local notes are read as they are.
    Failed { remote: String },
    /// The default branch tracks no remote; the local notes are read.
    NoRemote { branch: String },
}

/// The remote the default branch tracks, or the branch that tracks none.
fn notes_remote(root: &Path) -> Result<Result<String, String>, RepositoryError> {
    let remotes = git::run(root, ["remote"])?;
    let remotes: Vec<&str> = remotes.lines().filter(|line| !line.is_empty()).collect();
    let candidate = if remotes.contains(&"origin") {
        "origin"
    } else {
        remotes.first().copied().unwrap_or("origin")
    };
    let branch = _git::default_branch(root, candidate)?;
    Ok(_git::config(root, &format!("branch.{branch}.remote"))?.ok_or(branch))
}

/// Pushes the local notes ref to the default branch's remote; on rejection
/// merges the remote's notes and tries again, a bounded number of times.
pub(crate) fn push(root: &Path) -> Result<NotesPush, RepositoryError> {
    let remote = match notes_remote(root)? {
        Ok(remote) => remote,
        Err(branch) => return Ok(NotesPush::NoRemote { branch }),
    };
    let refspec = format!("{REVIEW_NOTES_REF}:{REVIEW_NOTES_REF}");
    for attempt in 1..=PUSH_ATTEMPTS {
        let output = git::run_output(
            root,
            ["push", "--porcelain", remote.as_str(), refspec.as_str()],
        )?;
        if output.status.success() {
            return Ok(NotesPush::Pushed { remote });
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        let rejected = stdout.lines().any(|line| line.starts_with('!'));
        tracing::warn!(
            event = "review_notes_push_failed",
            remote = %remote,
            attempt,
            rejected,
            stderr = %String::from_utf8_lossy(&output.stderr).trim(),
        );
        if !rejected {
            return Ok(NotesPush::Pending {
                remote,
                reason: PendingReason::Unreachable,
            });
        }
        if !fetch_and_merge(root, &remote)? {
            return Ok(NotesPush::Pending {
                remote,
                reason: PendingReason::Unreachable,
            });
        }
    }
    Ok(NotesPush::Pending {
        remote,
        reason: PendingReason::StillRejected,
    })
}

/// Fetches the remote notes ref and merges it into the local one, for a
/// reader about to read verdicts.
pub(crate) fn fetch(root: &Path) -> Result<NotesFetch, RepositoryError> {
    let remote = match notes_remote(root)? {
        Ok(remote) => remote,
        Err(branch) => return Ok(NotesFetch::NoRemote { branch }),
    };
    Ok(if fetch_and_merge(root, &remote)? {
        NotesFetch::Fetched { remote }
    } else {
        NotesFetch::Failed { remote }
    })
}

/// Brings the remote's notes into the local ref; `false` when the remote
/// could not be read. A remote without the ref leaves the local one as is.
fn fetch_and_merge(root: &Path, remote: &str) -> Result<bool, RepositoryError> {
    let listed = git::run_output(root, ["ls-remote", remote, REVIEW_NOTES_REF])?;
    if !listed.status.success() {
        tracing::warn!(
            event = "review_notes_fetch_failed",
            remote = %remote,
            stderr = %String::from_utf8_lossy(&listed.stderr).trim(),
        );
        return Ok(false);
    }
    if listed.stdout.iter().all(u8::is_ascii_whitespace) {
        return Ok(true);
    }
    let fetched = format!("refs/instrukt/fetched-notes/{remote}");
    let refspec = format!("+{REVIEW_NOTES_REF}:{fetched}");
    let output = git::run_output(root, ["fetch", "--quiet", remote, refspec.as_str()])?;
    if !output.status.success() {
        tracing::warn!(
            event = "review_notes_fetch_failed",
            remote = %remote,
            stderr = %String::from_utf8_lossy(&output.stderr).trim(),
        );
        return Ok(false);
    }
    git::run(
        root,
        [
            "notes",
            "--ref",
            REVIEW_NOTES_REF,
            "merge",
            "--quiet",
            "--strategy=cat_sort_uniq",
            fetched.as_str(),
        ],
    )?;
    Ok(true)
}
