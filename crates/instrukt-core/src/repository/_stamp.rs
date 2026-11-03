//! `instrukt repo stamp`: a commit-review verdict with the complete
//! report's digest, or the author's exemption with its reason, recorded as
//! a git note line on each named commit with the time it was made, pushed
//! to the default branch's remote; and the reading of those notes, settled
//! under the latest-stamp rule, for the integrator.

use std::path::Path;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};

use crate::git;
use crate::ids::CommitSha;

use super::_error::RepositoryError;
use super::_git;
use super::_notes::{self, NotesFetch, NotesPush, REVIEW_NOTES_REF};

/// A commit-review verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Approve,
    NeedsWork,
    Exempt,
}

impl Verdict {
    /// The verdict's name, as a note records it.
    pub fn name(self) -> &'static str {
        match self {
            Verdict::Approve => "approve",
            Verdict::NeedsWork => "needs_work",
            Verdict::Exempt => "exempt",
        }
    }

    fn parse(name: &str) -> Option<Self> {
        [Verdict::Approve, Verdict::NeedsWork, Verdict::Exempt]
            .into_iter()
            .find(|verdict| verdict.name() == name)
    }

    /// Whether the integrator takes a commit settled with this verdict.
    pub fn releases(self) -> bool {
        matches!(self, Verdict::Approve | Verdict::Exempt)
    }
}

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

    pub fn verdict(&self) -> Verdict {
        match self {
            Stamp::Approve { .. } => Verdict::Approve,
            Stamp::NeedsWork { .. } => Verdict::NeedsWork,
            Stamp::Exempt { .. } => Verdict::Exempt,
        }
    }
}

/// When a stamp was made: written as RFC 3339 UTC with nanoseconds, so
/// note lines sort by time and stamps a moment apart do not tie.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StampTime {
    unix_nanos: u128,
}

impl StampTime {
    /// The time `time` names; refused before the Unix epoch.
    pub fn from_system(time: SystemTime) -> Result<Self, RepositoryError> {
        let since = time
            .duration_since(UNIX_EPOCH)
            .map_err(|_| RepositoryError::ClockBeforeEpoch)?;
        Ok(Self {
            unix_nanos: since.as_nanos(),
        })
    }

    /// `YYYY-MM-DDTHH:MM:SS.nnnnnnnnnZ`.
    pub fn rfc3339(self) -> String {
        let nanos = self.unix_nanos % 1_000_000_000;
        let secs = self.unix_nanos / 1_000_000_000;
        let (days, of_day) = (secs / 86_400, secs % 86_400);
        let (year, month, day) = civil_from_days(days);
        format!(
            "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{nanos:09}Z",
            of_day / 3600,
            of_day / 60 % 60,
            of_day % 60
        )
    }

    /// The time an [`rfc3339`](Self::rfc3339) text names.
    fn parse(text: &str) -> Option<Self> {
        let bytes = text.as_bytes();
        let shape_ok = bytes.len() == 30
            && bytes[4] == b'-'
            && bytes[7] == b'-'
            && bytes[10] == b'T'
            && bytes[13] == b':'
            && bytes[16] == b':'
            && bytes[19] == b'.'
            && bytes[29] == b'Z';
        if !shape_ok {
            return None;
        }
        let number = |range: std::ops::Range<usize>| -> Option<u128> {
            let digits = text.get(range)?;
            digits
                .bytes()
                .all(|b| b.is_ascii_digit())
                .then(|| digits.parse().ok())
                .flatten()
        };
        let (year, month, day) = (number(0..4)?, number(5..7)?, number(8..10)?);
        let (hour, minute, second) = (number(11..13)?, number(14..16)?, number(17..19)?);
        let nanos = number(20..29)?;
        if !(1..=12).contains(&month)
            || day == 0
            || day > 31
            || hour > 23
            || minute > 59
            || second > 59
        {
            return None;
        }
        let days = days_from_civil(year, month, day)?;
        let secs = days * 86_400 + hour * 3600 + minute * 60 + second;
        let parsed = Self {
            unix_nanos: secs * 1_000_000_000 + nanos,
        };
        // A day past its month's end would not read back as written.
        (parsed.rfc3339() == text).then_some(parsed)
    }
}

/// The proleptic Gregorian date `days` after 1970-01-01.
fn civil_from_days(days: u128) -> (u128, u128, u128) {
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z % 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + u128::from(month <= 2);
    (year, month, day)
}

/// The days from 1970-01-01 to the date; `None` before it.
fn days_from_civil(year: u128, month: u128, day: u128) -> Option<u128> {
    let year = if month <= 2 {
        year.checked_sub(1)?
    } else {
        year
    };
    let era = year / 400;
    let yoe = year - era * 400;
    let mp = if month > 2 { month - 3 } else { month + 9 };
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    (era * 146_097 + doe).checked_sub(719_468)
}

/// One stamp as a note records it: what and when.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimedStamp {
    pub at: StampTime,
    pub stamp: Stamp,
}

impl TimedStamp {
    /// The note line: `at: <time>; verdict: <verdict>; report_sha256:
    /// <hex>` or `…; reason: <text>`. The time leads, so lines sort by time.
    fn line(&self) -> String {
        let detail = match &self.stamp {
            Stamp::Approve { report_sha256 } | Stamp::NeedsWork { report_sha256 } => {
                format!("report_sha256: {report_sha256}")
            }
            Stamp::Exempt { reason } => format!("reason: {}", one_line(reason)),
        };
        format!(
            "at: {}; verdict: {}; {detail}",
            self.at.rfc3339(),
            self.stamp.verdict().name()
        )
    }

    /// The stamp a note line records; `None` for a line that records none.
    fn parse_line(line: &str) -> Option<Self> {
        let rest = line.trim().strip_prefix("at: ")?;
        let (at, rest) = rest.split_once("; ")?;
        let rest = rest.strip_prefix("verdict: ")?;
        let (verdict, detail) = rest.split_once("; ")?;
        let stamp = match Verdict::parse(verdict)? {
            Verdict::Approve => Stamp::Approve {
                report_sha256: detail.strip_prefix("report_sha256: ")?.to_owned(),
            },
            Verdict::NeedsWork => Stamp::NeedsWork {
                report_sha256: detail.strip_prefix("report_sha256: ")?.to_owned(),
            },
            Verdict::Exempt => Stamp::Exempt {
                reason: detail.strip_prefix("reason: ")?.to_owned(),
            },
        };
        Some(Self {
            at: StampTime::parse(at)?,
            stamp,
        })
    }
}

/// A commit's verdict under the latest-stamp rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Settlement {
    /// The latest stamp decides (stamps tied at the latest time decide only
    /// when they all agree on `approve` or on `exempt`).
    Settled(Verdict),
    /// No stamp, or stamps tied at the latest time that do not settle it.
    Unsettled,
}

impl Settlement {
    /// The verdict of a commit carrying `stamps`.
    pub fn of(stamps: &[TimedStamp]) -> Self {
        let Some(latest) = stamps.iter().map(|stamp| stamp.at).max() else {
            return Settlement::Unsettled;
        };
        let tied: Vec<Verdict> = stamps
            .iter()
            .filter(|stamp| stamp.at == latest)
            .map(|stamp| stamp.stamp.verdict())
            .collect();
        match tied.as_slice() {
            [only] => Settlement::Settled(*only),
            [first, rest @ ..] if first.releases() && rest.iter().all(|v| v == first) => {
                Settlement::Settled(*first)
            }
            _ => Settlement::Unsettled,
        }
    }

    /// Whether the integrator takes the commit: its verdict is `approve` or
    /// `exempt`.
    pub fn releases(self) -> bool {
        matches!(self, Settlement::Settled(verdict) if verdict.releases())
    }
}

/// What `repo stamp` did: the commits noted, in order, and the push of the
/// notes ref.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StampOutcome {
    pub commits: Vec<CommitSha>,
    pub push: NotesPush,
}

/// Records `stamp`, made at `at`, on each commit `revisions` names in the
/// repository containing `cwd`, beside the stamps the commit already
/// carries, then pushes the notes ref to the default branch's remote. The
/// caller supplies the time (the system clock in production). Every
/// revision is resolved before any note is written; a push that does not
/// land leaves the stamp recorded locally and pending.
pub fn stamp(
    cwd: &Path,
    revisions: &[String],
    stamp: &Stamp,
    at: StampTime,
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
    let line = TimedStamp {
        at,
        stamp: stamp.clone(),
    }
    .line();
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
        verdict = stamp.verdict().name(),
        at = %at.rfc3339(),
        commits = ?commits.iter().map(CommitSha::as_str).collect::<Vec<_>>(),
        push = ?push,
        duration_ms = started.elapsed().as_millis() as u64,
    );
    Ok(StampOutcome { commits, push })
}

/// One commit as a verdict reader sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitReview {
    pub commit: CommitSha,
    /// Every stamp the commit carries, oldest first.
    pub stamps: Vec<TimedStamp>,
    /// Its verdict under the latest-stamp rule.
    pub settlement: Settlement,
}

/// The review state of the commits a verdict reader asked about, read after
/// fetching the notes ref.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewNotes {
    /// What the fetch before reading did; a failed fetch means the local
    /// notes were read as they are.
    pub fetch: NotesFetch,
    /// Each commit asked about, in order.
    pub commits: Vec<CommitReview>,
}

/// Reads the review state of each commit `revisions` names, in the
/// repository containing `cwd`: fetches the notes ref from the default
/// branch's remote first (reporting a failed fetch rather than hiding it),
/// then settles each commit under the latest-stamp rule. The integrator
/// takes commits while [`Settlement::releases`] holds.
pub fn read_review_notes(cwd: &Path, revisions: &[String]) -> Result<ReviewNotes, RepositoryError> {
    let root = _git::root(cwd)?;
    let mut shas = Vec::new();
    for revision in revisions {
        shas.push(CommitSha::parse(&_git::resolve_commit(&root, revision)?)?);
    }
    let fetch = _notes::fetch(&root)?;
    let mut commits = Vec::new();
    for commit in shas {
        let mut stamps = Vec::new();
        for line in note_lines(&root, &commit)? {
            let stamp =
                TimedStamp::parse_line(&line).ok_or_else(|| RepositoryError::MalformedNote {
                    commit: commit.to_string(),
                    line: line.clone(),
                })?;
            stamps.push(stamp);
        }
        stamps.sort_by_key(|stamp| stamp.at);
        let settlement = Settlement::of(&stamps);
        commits.push(CommitReview {
            commit,
            stamps,
            settlement,
        });
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
    use std::time::Duration;

    use super::*;

    fn at(secs: u64) -> StampTime {
        StampTime::from_system(UNIX_EPOCH + Duration::from_secs(secs)).unwrap()
    }

    fn timed(secs: u64, stamp: Stamp) -> TimedStamp {
        TimedStamp {
            at: at(secs),
            stamp,
        }
    }

    fn approve() -> Stamp {
        Stamp::review(true, b"report")
    }

    fn needs_work() -> Stamp {
        Stamp::review(false, b"report")
    }

    fn exempt() -> Stamp {
        Stamp::exempt("generated").unwrap()
    }

    #[test]
    fn the_same_report_gives_the_same_digest() {
        assert_eq!(
            Stamp::review(true, b"report one"),
            Stamp::review(true, b"report one")
        );
    }

    #[test]
    fn a_different_report_gives_a_different_digest() {
        assert_ne!(
            Stamp::review(true, b"report one"),
            Stamp::review(true, b"report two")
        );
    }

    #[test]
    fn a_review_that_does_not_approve_needs_work() {
        assert_eq!(
            Stamp::review(false, b"report").verdict(),
            Verdict::NeedsWork
        );
    }

    fn reads_back(stamp: &TimedStamp) {
        let read = TimedStamp::parse_line(&stamp.line()).unwrap();
        assert_eq!(&read, stamp);
    }

    #[test]
    fn an_approval_reads_back_from_its_note_line() {
        reads_back(&timed(1_793_724_900, approve()));
    }

    #[test]
    fn a_needs_work_verdict_reads_back_from_its_note_line() {
        reads_back(&timed(0, needs_work()));
    }

    #[test]
    fn an_exemption_reason_spanning_lines_reads_back_on_one_line() {
        let stamp = timed(
            951_782_400,
            Stamp::exempt("typo; nothing\nto review").unwrap(),
        );
        let read = TimedStamp::parse_line(&stamp.line()).unwrap();
        assert_eq!(read.at, stamp.at);
        assert_eq!(read.line(), stamp.line());
    }

    #[test]
    fn a_line_without_its_time_records_no_stamp() {
        let line = timed(1, approve()).line();
        let (_, untimed) = line.split_once("; ").unwrap();
        assert_eq!(TimedStamp::parse_line(untimed), None);
    }

    #[test]
    fn note_lines_sort_in_time_order() {
        assert!(timed(1_000, exempt()).line() < timed(1_000_000_000, needs_work()).line());
    }

    #[test]
    fn a_day_past_its_months_end_is_refused() {
        let line = timed(1_000_000_000, needs_work()).line();
        let impossible = line.replacen("2001-09-09", "2001-02-30", 1);
        assert_ne!(impossible, line, "the fixture's date was replaced");
        assert_eq!(TimedStamp::parse_line(&impossible), None);
    }

    #[test]
    fn a_commit_without_stamps_is_unsettled() {
        assert_eq!(Settlement::of(&[]), Settlement::Unsettled);
    }

    #[test]
    fn a_later_approval_supersedes_an_earlier_needs_work() {
        let settled = Settlement::of(&[timed(1, needs_work()), timed(2, approve())]);
        assert_eq!(settled, Settlement::Settled(Verdict::Approve));
        assert!(settled.releases());
    }

    #[test]
    fn a_later_needs_work_reopens_an_approved_commit() {
        let settled = Settlement::of(&[timed(1, approve()), timed(2, needs_work())]);
        assert_eq!(settled, Settlement::Settled(Verdict::NeedsWork));
        assert!(!settled.releases());
    }

    #[test]
    fn approvals_tied_at_the_latest_time_settle_as_approve() {
        let stamps = [
            timed(1, needs_work()),
            timed(2, approve()),
            timed(2, approve()),
        ];
        assert_eq!(
            Settlement::of(&stamps),
            Settlement::Settled(Verdict::Approve)
        );
    }

    #[test]
    fn exemptions_tied_at_the_latest_time_settle_as_exempt() {
        let stamps = [timed(2, exempt()), timed(2, exempt())];
        assert_eq!(
            Settlement::of(&stamps),
            Settlement::Settled(Verdict::Exempt)
        );
    }

    #[test]
    fn an_approval_tied_with_needs_work_is_unsettled() {
        let stamps = [timed(2, approve()), timed(2, needs_work())];
        assert_eq!(Settlement::of(&stamps), Settlement::Unsettled);
    }

    #[test]
    fn an_approval_tied_with_an_exemption_is_unsettled() {
        let stamps = [timed(2, approve()), timed(2, exempt())];
        assert_eq!(Settlement::of(&stamps), Settlement::Unsettled);
    }

    #[test]
    fn needs_work_tied_with_needs_work_is_unsettled() {
        let stamps = [timed(2, needs_work()), timed(2, needs_work())];
        assert_eq!(Settlement::of(&stamps), Settlement::Unsettled);
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
