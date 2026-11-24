//! Moving review evidence to another computer (`github-apps`, Evidence when
//! the serving computer changes): one export file, digest-declared, and the
//! rule an import applies to each commit.

use serde::{Deserialize, Serialize};

use super::{Moment, Verdict, Version};

/// The first line's mark; the version of the file's shape follows it.
const MARK: &str = "instrukt-review-export 1";

/// One export: what stood on the computer that wrote it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Export {
    pub project: String,
    pub computer: String,
    pub written_at: String,
    pub verdicts: Vec<ExportedVerdict>,
    pub exemptions: Vec<ExportedExemption>,
    pub derived: Vec<ExportedDerived>,
}

/// The current version of a head verdict, as it was recorded, with its
/// moment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ExportedVerdict {
    pub head: String,
    pub verdict: String,
    pub name: String,
    pub report: String,
    pub reviewer: String,
    pub entered_by: Option<String>,
    pub base: String,
    pub base_on_canonical: bool,
    pub request: Option<u64>,
    pub unattributed: Vec<String>,
    pub moment: String,
    pub recorded_at: String,
    pub recorded_on: String,
}

/// A current exemption.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ExportedExemption {
    pub commit: String,
    pub basis: String,
    pub declared_by: Option<String>,
    pub names_session: bool,
    pub recorded_at: String,
    pub recorded_on: String,
}

/// A derived approval that was not withdrawn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ExportedDerived {
    pub candidate: String,
    pub canonical: String,
    pub coverage: String,
    pub name: String,
    pub moment: String,
}

/// The file's text: its mark and the digest of the JSON that follows the
/// first newline, then that JSON.
pub(super) fn write(export: &Export) -> Result<(String, String), serde_json::Error> {
    let json = serde_json::to_string_pretty(export)?;
    let digest = crate::evidence::ContentDigest::of(&json).to_string();
    Ok((format!("{MARK} {digest}\n{json}"), digest))
}

/// Why a file is not an export this instrukt takes in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Unreadable {
    NotAnExport,
    DigestMismatch,
    Shape(String),
}

/// The export `text` holds, with its declared digest, after the digest
/// matched its content.
pub(super) fn read(text: &str) -> Result<(Export, String), Unreadable> {
    let (first, json) = text.split_once('\n').ok_or(Unreadable::NotAnExport)?;
    let declared = first
        .strip_prefix(MARK)
        .and_then(|rest| rest.strip_prefix(' '))
        .ok_or(Unreadable::NotAnExport)?;
    if crate::evidence::ContentDigest::of(json).as_str() != declared {
        return Err(Unreadable::DigestMismatch);
    }
    let export =
        serde_json::from_str(json).map_err(|error| Unreadable::Shape(error.to_string()))?;
    Ok((export, declared.to_owned()))
}

/// Which verdict stands where the file and this computer hold two different
/// ones, as the importing operator states it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Statement {
    Imported,
    Held,
}

/// What an import does with the file's verdict of a commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Rule {
    /// No verdict of the commit stands here: the file's stands.
    Stands,
    /// The same version stands here; it takes the file's moment where that
    /// is later.
    Same { later: Option<Moment> },
    /// Two verdicts of one commit; the statement chooses.
    Two,
}

/// The rule for the file's verdict `file` of a commit whose verdict
/// standing here is `held`.
pub(super) fn rule(
    held: Option<&Version>,
    file_verdict: Verdict,
    file_name: &str,
    file_moment: Moment,
) -> Rule {
    let Some(held) = held else {
        return Rule::Stands;
    };
    if held.verdict == file_verdict && held.name.as_str() == file_name {
        return Rule::Same {
            later: (file_moment > held.moment).then_some(file_moment),
        };
    }
    Rule::Two
}

/// The moment of the verdict that stands after an import set `set_aside`
/// aside for it: unchanged where already later, otherwise the import's, or
/// the first moment after the one set aside.
pub(super) fn chosen_moment(standing: Moment, set_aside: Moment, now: jiff::Timestamp) -> Moment {
    if standing > set_aside {
        standing
    } else {
        Moment::decided(now, Some(set_aside))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evidence::ContentDigest;
    use instrukt_core::ids::{CommitSha, SessionRef};

    fn moment(at: &str) -> Moment {
        Moment::at(at.parse().unwrap())
    }

    fn held(report: &str, at: &str) -> Version {
        Version {
            number: 1,
            verdict: Verdict::Approve,
            name: ContentDigest::of(report),
            report: report.into(),
            reviewer: SessionRef::parse("t3:r").unwrap(),
            entered_by: None,
            base: CommitSha::parse(&"b".repeat(40)).unwrap(),
            base_on_canonical: true,
            request: None,
            unattributed: Vec::new(),
            moment: moment(at),
            recorded_at: jiff::Timestamp::UNIX_EPOCH,
            recorded_on: "here".into(),
        }
    }

    #[test]
    fn a_commit_without_a_verdict_here_takes_the_files() {
        assert_eq!(
            rule(
                None,
                Verdict::Approve,
                "sha256:x",
                moment("2026-11-24T10:00:00Z")
            ),
            Rule::Stands
        );
    }

    #[test]
    fn the_same_version_takes_the_files_later_moment() {
        let held = held("report", "2026-11-24T10:00:00Z");
        let later = moment("2026-11-24T11:00:00Z");
        assert_eq!(
            rule(Some(&held), Verdict::Approve, held.name.as_str(), later),
            Rule::Same { later: Some(later) }
        );
    }

    #[test]
    fn the_same_version_keeps_its_own_later_moment() {
        let held = held("report", "2026-11-24T10:00:00Z");
        assert_eq!(
            rule(
                Some(&held),
                Verdict::Approve,
                held.name.as_str(),
                moment("2026-11-24T09:00:00Z")
            ),
            Rule::Same { later: None }
        );
    }

    #[test]
    fn another_report_is_two_verdicts_of_one_commit() {
        let held = held("report", "2026-11-24T10:00:00Z");
        assert_eq!(
            rule(
                Some(&held),
                Verdict::Approve,
                ContentDigest::of("other").as_str(),
                moment("2026-11-24T10:00:00Z")
            ),
            Rule::Two
        );
    }

    #[test]
    fn a_standing_verdict_already_later_keeps_its_moment() {
        let standing = moment("2026-11-24T12:00:00Z");
        assert_eq!(
            chosen_moment(
                standing,
                moment("2026-11-24T11:00:00Z"),
                "2026-11-24T13:00:00Z".parse().unwrap()
            ),
            standing
        );
    }

    #[test]
    fn a_standing_verdict_not_later_takes_the_imports_moment() {
        let now: jiff::Timestamp = "2026-11-24T13:00:00Z".parse().unwrap();
        assert_eq!(
            chosen_moment(
                moment("2026-11-24T11:00:00Z"),
                moment("2026-11-24T12:00:00Z"),
                now
            ),
            Moment::at(now)
        );
    }

    #[test]
    fn an_import_whose_clock_is_not_later_takes_the_first_moment_after_the_one_set_aside() {
        let set_aside = moment("2026-11-24T14:00:00Z");
        assert_eq!(
            chosen_moment(
                moment("2026-11-24T11:00:00Z"),
                set_aside,
                "2026-11-24T13:00:00Z".parse().unwrap()
            ),
            set_aside.next()
        );
    }

    #[test]
    fn a_file_whose_content_does_not_match_its_digest_is_refused() {
        let export = Export {
            project: "demo".into(),
            computer: "c".into(),
            written_at: "t".into(),
            verdicts: Vec::new(),
            exemptions: Vec::new(),
            derived: Vec::new(),
        };
        let (text, _) = write(&export).unwrap();
        assert_eq!(read(&text).map(|(e, _)| e), Ok(export));
        assert_eq!(
            read(&text.replace("\"demo\"", "\"other\"")).map(|(e, _)| e),
            Err(Unreadable::DigestMismatch)
        );
    }
}
