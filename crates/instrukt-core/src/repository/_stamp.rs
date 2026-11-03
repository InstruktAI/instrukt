//! `instrukt repo stamp`: a commit-review verdict with the complete
//! report's digest, or the author's exemption with its reason, recorded as
//! a git note on each named commit, pushed to the default branch's remote;
//! and the reading of those notes for the integrator.

use std::path::Path;
use std::time::Instant;

use sha2::{Digest, Sha256};

use crate::git;
use crate::ids::CommitSha;

use super::_error::RepositoryError;
use super::_git;
use super::_notes::{self, NotesFetch, NotesPush, REVIEW_NOTES_REF};

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

    /// The note line recording this stamp: one line per stamp, so merged
    /// notes (`cat_sort_uniq`) keep each stamp whole.
    fn line(&self) -> String {
        match self {
            Stamp::Approve { report_sha256 } | Stamp::NeedsWork { report_sha256 } => {
                format!(
                    "verdict: {}; report_sha256: {report_sha256}",
                    self.verdict()
                )
            }
            Stamp::Exempt { reason } => {
                format!("verdict: {}; reason: {}", self.verdict(), one_line(reason))
            }
        }
    }

    /// The stamp a note line records; `None` for a line that records none.
    fn parse_line(line: &str) -> Option<Self> {
        let rest = line.trim().strip_prefix("verdict: ")?;
        let (verdict, detail) = rest.split_once("; ")?;
        match verdict {
            "approve" => Some(Stamp::Approve {
                report_sha256: detail.strip_prefix("report_sha256: ")?.to_owned(),
            }),
            "needs_work" => Some(Stamp::NeedsWork {
                report_sha256: detail.strip_prefix("report_sha256: ")?.to_owned(),
            }),
            "exempt" => Some(Stamp::Exempt {
                reason: detail.strip_prefix("reason: ")?.to_owned(),
            }),
            _ => None,
        }
    }
}

/// What `repo stamp` did: the commits noted, in order, and the push of the
/// notes ref.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StampOutcome {
    pub commits: Vec<CommitSha>,
    pub push: NotesPush,
}

/// Records `stamp` on each commit `revisions` names, in the repository
/// containing `cwd`, beside the stamps the commit already carries, then
/// pushes the notes ref to the default branch's remote. Every revision is
/// resolved before any note is written; a push that does not land leaves
/// the stamp recorded locally and pending.
pub fn stamp(
    cwd: &Path,
    revisions: &[String],
    stamp: &Stamp,
) -> Result<StampOutcome, RepositoryError> {
    let started = Instant::now();
    if revisions.is_empty() {
        return Err(RepositoryError::NoCommits);
    }
    let root = _git::root(cwd)?;
    let mut commits = Vec::new();
    for revision in revisions {
        commits.push(CommitSha::parse(&_git::resolve_commit(&root, revision)?)?);
    }
    let line = stamp.line();
    for commit in &commits {
        let mut lines = note_lines(&root, commit)?;
        lines.push(line.clone());
        lines.sort();
        lines.dedup();
        let note = lines.join("\n");
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
    let push = _notes::push(&root)?;
    tracing::debug!(
        event = "commits_stamped",
        verdict = stamp.verdict(),
        commits = ?commits.iter().map(CommitSha::as_str).collect::<Vec<_>>(),
        push = ?push,
        duration_ms = started.elapsed().as_millis() as u64,
    );
    Ok(StampOutcome { commits, push })
}

/// The review stamps of the commits a verdict reader asked about, read
/// after fetching the notes ref.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewNotes {
    /// What the fetch before reading did; a failed fetch means the local
    /// notes were read as they are.
    pub fetch: NotesFetch,
    /// Each commit asked about, in order, with every stamp it carries.
    pub commits: Vec<(CommitSha, Vec<Stamp>)>,
}

/// Reads the review stamps of each commit `revisions` names, in the
/// repository containing `cwd`: fetches the notes ref from the default
/// branch's remote first, and reports a failed fetch rather than hiding it.
/// The integrator reads verdicts through this.
pub fn read_review_notes(cwd: &Path, revisions: &[String]) -> Result<ReviewNotes, RepositoryError> {
    let root = _git::root(cwd)?;
    let mut shas = Vec::new();
    for revision in revisions {
        shas.push(CommitSha::parse(&_git::resolve_commit(&root, revision)?)?);
    }
    let fetch = _notes::fetch(&root)?;
    let mut commits = Vec::new();
    for sha in shas {
        let mut stamps = Vec::new();
        for line in note_lines(&root, &sha)? {
            let stamp = Stamp::parse_line(&line).ok_or_else(|| RepositoryError::MalformedNote {
                commit: sha.to_string(),
                line: line.clone(),
            })?;
            stamps.push(stamp);
        }
        commits.push((sha, stamps));
    }
    Ok(ReviewNotes { fetch, commits })
}

/// The non-empty lines of `commit`'s review note; none without a note.
fn note_lines(root: &Path, commit: &CommitSha) -> Result<Vec<String>, RepositoryError> {
    if !git::succeeds(
        root,
        ["notes", "--ref", REVIEW_NOTES_REF, "list", commit.as_str()],
    )? {
        return Ok(Vec::new());
    }
    let note = git::run(
        root,
        ["notes", "--ref", REVIEW_NOTES_REF, "show", commit.as_str()],
    )?;
    Ok(note
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect())
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
    fn every_stamp_reads_back_from_its_note_line() {
        for stamp in [
            Stamp::review(true, b"report"),
            Stamp::review(false, b"report"),
            Stamp::exempt("typo; nothing\nto review").unwrap(),
        ] {
            let read = Stamp::parse_line(&stamp.line()).unwrap();
            assert_eq!(read.line(), stamp.line());
        }
        assert_eq!(Stamp::parse_line("verdict: maybe; reason: x"), None);
    }

    /// A clone of `remote` at `dir` with an identity of its own and one
    /// commit on main, pushed.
    fn machine(remote: &Path, dir: &Path) {
        let base = dir.parent().unwrap();
        git::run(
            base,
            [
                std::ffi::OsStr::new("clone"),
                std::ffi::OsStr::new("--quiet"),
                remote.as_os_str(),
                dir.as_os_str(),
            ],
        )
        .unwrap();
        git::run(dir, ["config", "user.name", "Machine"]).unwrap();
        git::run(dir, ["config", "user.email", "machine@example.test"]).unwrap();
        git::run(dir, ["config", "commit.gpgsign", "false"]).unwrap();
        git::run(dir, ["config", "init.defaultBranch", "main"]).unwrap();
        git::run(dir, ["config", "branch.main.remote", "origin"]).unwrap();
        git::run(dir, ["config", "branch.main.merge", "refs/heads/main"]).unwrap();
        git::run(dir, ["symbolic-ref", "HEAD", "refs/heads/main"]).unwrap();
    }

    #[test]
    fn a_reader_sees_another_machines_verdicts_after_its_fetch_and_reports_a_failed_fetch() {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path();
        let remote = base.join("origin.git");
        std::fs::create_dir_all(&remote).unwrap();
        git::run(
            &remote,
            ["init", "--quiet", "--bare", "--initial-branch=main"],
        )
        .unwrap();
        let first = base.join("first");
        machine(&remote, &first);
        git::run(&first, ["commit", "--quiet", "--allow-empty", "-m", "one"]).unwrap();
        git::run(&first, ["push", "--quiet", "origin", "main"]).unwrap();
        let second = base.join("second");
        machine(&remote, &second);
        let head = git::run(&first, ["rev-parse", "HEAD"]).unwrap();

        let approve = Stamp::review(true, b"report");
        let stamped = stamp(&first, std::slice::from_ref(&head), &approve).unwrap();
        assert!(
            matches!(stamped.push, NotesPush::Pushed { .. }),
            "{:?}",
            stamped.push
        );

        let read = read_review_notes(&second, std::slice::from_ref(&head)).unwrap();
        assert!(
            matches!(read.fetch, NotesFetch::Fetched { .. }),
            "{:?}",
            read.fetch
        );
        assert_eq!(read.commits.len(), 1);
        assert_eq!(read.commits[0].0.as_str(), head);
        assert_eq!(read.commits[0].1, vec![approve.clone()]);

        std::fs::rename(&remote, base.join("moved.git")).unwrap();
        let offline = read_review_notes(&second, std::slice::from_ref(&head)).unwrap();
        assert!(
            matches!(offline.fetch, NotesFetch::Failed { .. }),
            "{:?}",
            offline.fetch
        );
        assert_eq!(offline.commits, read.commits, "the local notes are read");
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
