//! `instrukt repo stamp`: a commit-review verdict with the complete
//! report's digest, or the author's exemption with its reason, recorded as
//! a git note on each named commit.

use std::path::Path;
use std::time::Instant;

use sha2::{Digest, Sha256};

use crate::git;
use crate::ids::CommitSha;

use super::_error::RepositoryError;
use super::_git;

/// The notes ref commit-review stamps are written to.
pub const REVIEW_NOTES_REF: &str = "refs/notes/instrukt-review";

/// A commit-review outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stamp {
    /// A review approved the commit; carries the report's SHA-256 digest.
    Approve { report_sha256: String },
    /// A review found work owed; carries the report's SHA-256 digest.
    NeedsWork { report_sha256: String },
    /// The author's own exemption: the commit needs no review.
    Exempt { reason: String },
}

impl Stamp {
    /// A review verdict over the complete report `report`.
    pub fn review(approve: bool, report: &[u8]) -> Self {
        let report_sha256 = hex::encode(Sha256::digest(report));
        if approve {
            Stamp::Approve { report_sha256 }
        } else {
            Stamp::NeedsWork { report_sha256 }
        }
    }

    /// The author's exemption; refused without a reason.
    pub fn exempt(reason: &str) -> Result<Self, RepositoryError> {
        let reason = reason.trim();
        if reason.is_empty() {
            return Err(RepositoryError::EmptyReason);
        }
        Ok(Stamp::Exempt {
            reason: reason.to_owned(),
        })
    }

    /// The verdict's name, as the note records it.
    pub fn verdict(&self) -> &'static str {
        match self {
            Stamp::Approve { .. } => "approve",
            Stamp::NeedsWork { .. } => "needs_work",
            Stamp::Exempt { .. } => "exempt",
        }
    }

    fn note(&self) -> String {
        let detail = match self {
            Stamp::Approve { report_sha256 } | Stamp::NeedsWork { report_sha256 } => {
                format!("report_sha256: {report_sha256}")
            }
            Stamp::Exempt { reason } => format!("reason: {}", one_line(reason)),
        };
        format!("verdict: {}\n{detail}\n", self.verdict())
    }
}

/// Records `stamp` as the note on each commit `revisions` names, in the
/// repository containing `cwd`; a commit's earlier stamp is replaced. Every
/// revision is resolved before any note is written. Returns the commits
/// noted, in order.
pub fn stamp(
    cwd: &Path,
    revisions: &[String],
    stamp: &Stamp,
) -> Result<Vec<CommitSha>, RepositoryError> {
    let started = Instant::now();
    if revisions.is_empty() {
        return Err(RepositoryError::NoCommits);
    }
    let root = _git::root(cwd)?;
    let mut commits = Vec::new();
    for revision in revisions {
        commits.push(CommitSha::parse(&_git::resolve_commit(&root, revision)?)?);
    }
    let note = stamp.note();
    for commit in &commits {
        git::run(
            &root,
            [
                "notes",
                "--ref",
                REVIEW_NOTES_REF,
                "add",
                "--force",
                "--message",
                note.as_str(),
                commit.as_str(),
            ],
        )?;
    }
    tracing::debug!(
        event = "commits_stamped",
        verdict = stamp.verdict(),
        commits = ?commits.iter().map(CommitSha::as_str).collect::<Vec<_>>(),
        duration_ms = started.elapsed().as_millis() as u64,
    );
    Ok(commits)
}

/// `text` on one line: line breaks become spaces.
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_review_carries_the_digest_of_the_whole_report() {
        let one = Stamp::review(true, b"report one");
        let same = Stamp::review(true, b"report one");
        let other = Stamp::review(true, b"report two");
        assert_eq!(one, same);
        assert_ne!(one, other);
        assert!(matches!(
            Stamp::review(false, b"report one"),
            Stamp::NeedsWork { .. }
        ));
    }

    #[test]
    fn an_exemption_needs_a_reason() {
        assert!(matches!(
            Stamp::exempt("  "),
            Err(RepositoryError::EmptyReason)
        ));
        assert!(Stamp::exempt("docs typo").is_ok());
    }
}
