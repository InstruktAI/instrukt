//! The candidate's coverage finding (`project/instrukt/spec/feature/workflow/integration-gate`,
//! Review coverage of a candidate): the evidence work holds that
//! workflow's coverage derivation reads, and what work does with its
//! answer — the derived approval recorded and published, or withdrawn.

use instrukt_core::git;
use instrukt_core::ids::CommitSha;
use instrukt_runner::state::RunnerState;
use jiff::Timestamp;

use super::{
    _published, _recognition, DeliveryState, Derived, Destination, Moment, REMOTE, ReviewError,
    ReviewWork, Verdict, derived_approval, derived_target, evidence, refused,
};
use crate::evidence::ContentDigest;

/// The evidence candidate coverage reads, as work holds it.
struct CoverageReader<'a> {
    work: &'a ReviewWork,
    project: &'a instrukt_runner::interfaces::ProjectRef,
}

impl CoverageReader<'_> {
    /// The todo a commit's `Todo:` trailer names.
    fn todo_of(&self, commit: &CommitSha) -> Result<Option<crate::evidence::TodoEvidence>, String> {
        let Some(records) = &self.work.records else {
            return Ok(None);
        };
        let named = git::run(
            &self.project.root,
            [
                "log",
                "-1",
                "--format=%(trailers:key=Todo,valueonly)",
                commit.as_str(),
            ],
        )
        .map_err(|e| e.to_string())?;
        let Some(slug) = named
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .and_then(|slug| instrukt_core::ids::TodoSlug::parse(slug).ok())
        else {
            return Ok(None);
        };
        records
            .evidence(self.project, &slug)
            .map_err(|e| e.to_string())
    }
}

/// An independent approval a todo's record binds to an exact head.
#[derive(Debug, Clone, PartialEq, Eq)]
struct RecordApproval {
    head: CommitSha,
    report: String,
    /// For a build-review approval, its stamp's record order: how the
    /// todo's reflection names the verdicts reviewed on its pull request.
    /// None for a reviewed preparation head.
    build: Option<u64>,
}

impl RecordApproval {
    /// The pull request this approval cites: for a build-review approval,
    /// the todo's pull request once `reflected` — the pull request and the
    /// build verdicts reviewed there — holds its own verdict, whatever
    /// became of any other; never for a reviewed preparation head.
    fn cited_pull(&self, reflected: Option<&(u64, Vec<u64>)>) -> Option<u64> {
        match (self.build, reflected) {
            (Some(sequence), Some((pull, reviews))) if reviews.contains(&sequence) => Some(*pull),
            _ => None,
        }
    }

    /// The covering item's reference (`github-apps`, Verdict publication):
    /// the todo, the approval and its report's digest, and the pull request
    /// it cites.
    fn reference(
        &self,
        todo: &instrukt_core::ids::TodoSlug,
        reflected: Option<&(u64, Vec<u64>)>,
    ) -> String {
        let (head, report) = (&self.head, &self.report);
        match (self.build, self.cited_pull(reflected)) {
            (Some(_), Some(pull)) => format!(
                "build-review approval of {head} in todo {todo} ({report}), reflected as a review on pull request #{pull}"
            ),
            (Some(_), None) => format!("build-review approval of {head} in todo {todo} ({report})"),
            (None, _) => format!("preparation-head approval of {head} in todo {todo} ({report})"),
        }
    }
}

/// The head a todo's record approves last, when the last build verdict or
/// preparation-head review of that head among `facts` approves it.
fn record_approval<'a>(
    stamps: impl IntoIterator<Item = (u64, &'a crate::evidence::StampFact)>,
    commit: Option<&CommitSha>,
) -> Option<RecordApproval> {
    use crate::evidence::{BuildVerdict, ReviewVerdict, StampFact};
    let mut last: Option<(RecordApproval, bool)> = None;
    for (sequence, fact) in stamps {
        let (head, report, approves, build) = match fact {
            StampFact::BuildVerdict {
                verdict,
                target,
                report,
            } => (
                target,
                report,
                *verdict == BuildVerdict::Approve,
                Some(sequence),
            ),
            StampFact::PreparationHeadReviewed {
                verdict,
                head,
                report,
            } => (head, report, *verdict == ReviewVerdict::Approve, None),
            _ => continue,
        };
        if commit.is_none_or(|commit| commit == head) {
            last = Some((
                RecordApproval {
                    head: head.clone(),
                    report: report.to_string(),
                    build,
                },
                approves,
            ));
        }
    }
    last.filter(|(_, approves)| *approves)
        .map(|(approval, _)| approval)
}

impl CoverageReader<'_> {
    /// The todo's pull request with the build verdicts reviewed there, by
    /// their stamps' record order, when it has one.
    fn reflection(&self, todo: &crate::work::TodoId) -> Result<Option<(u64, Vec<u64>)>, String> {
        use instrukt_core::state_db::rusqlite::OptionalExtension as _;
        let Some(db) = instrukt_core::state_db::StateRead::open(&self.work.home)
            .map_err(|error| error.to_string())?
        else {
            return Ok(None);
        };
        db.conn()
            .query_row(
                "SELECT pull_number, reviews FROM reflections
                 WHERE todo_id = ?1 AND pull_number IS NOT NULL",
                [todo.as_str()],
                |row| Ok((row.get::<_, u64>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(|error| error.to_string())?
            .map(|(pull, reviews)| {
                serde_json::from_str::<Vec<u64>>(&reviews)
                    .map(|reviews| (pull, reviews))
                    .map_err(|error| error.to_string())
            })
            .transpose()
    }
}

impl crate::workflow::CoverageEvidence for CoverageReader<'_> {
    fn approved(&self, commit: &CommitSha) -> Result<Option<crate::workflow::Approved>, String> {
        let held =
            evidence(&self.work.home, &self.project.slug, commit).map_err(|e| e.to_string())?;
        if let Some(version) = held.current()
            && version.verdict == Verdict::Approve
            && version.base_on_canonical
        {
            return Ok(Some(crate::workflow::Approved {
                reference: format!(
                    "head verdict {} of {commit} by {}",
                    version.name, version.reviewer
                ),
                todo: None,
            }));
        }
        let Some(record) = self.todo_of(commit)? else {
            return Ok(None);
        };
        let Some(approval) = record_approval(
            record.stamps.iter().map(|s| (s.sequence, &s.fact)),
            Some(commit),
        ) else {
            return Ok(None);
        };
        let reflected = match approval.build {
            Some(_) => self.reflection(&record.id)?,
            None => None,
        };
        Ok(Some(crate::workflow::Approved {
            reference: approval.reference(&record.slug, reflected.as_ref()),
            todo: Some(record.slug.clone()),
        }))
    }

    fn exempted(&self, commit: &CommitSha) -> Result<Option<String>, String> {
        let held =
            evidence(&self.work.home, &self.project.slug, commit).map_err(|e| e.to_string())?;
        Ok(held.exemption.map(|exemption| {
            format!(
                "exemption of {commit}: {} (declared by {})",
                exemption.basis,
                exemption
                    .declared_by
                    .map_or_else(|| "no resolved session".to_owned(), |s| s.to_string())
            )
        }))
    }

    fn delivery_head(
        &self,
        todo: &instrukt_core::ids::TodoSlug,
    ) -> Result<Option<CommitSha>, String> {
        let Some(records) = &self.work.records else {
            return Ok(None);
        };
        Ok(records
            .evidence(self.project, todo)
            .map_err(|e| e.to_string())?
            .and_then(|record| {
                record_approval(record.stamps.iter().map(|s| (s.sequence, &s.fact)), None)
            })
            .map(|approval| approval.head))
    }

    fn red_promotion(&self, head: &CommitSha) -> Result<Option<CommitSha>, String> {
        let state = RunnerState::new(&self.work.home);
        if !instrukt_runner::review::observed_red(&state, &self.project.slug, head)
            .map_err(|e| e.to_string())?
        {
            return Ok(None);
        }
        let held =
            evidence(&self.work.home, &self.project.slug, head).map_err(|e| e.to_string())?;
        if let Some(version) = held.current() {
            return Ok(Some(version.base.clone()));
        }
        let derived = derived_approval(&self.work.home, &self.project.slug, head)
            .map_err(|e| e.to_string())?;
        Ok(derived.and_then(|d| CommitSha::parse(&d.canonical).ok()))
    }
}

/// Work's part of a candidate's coverage (`integration-gate`, Review
/// coverage of a candidate): the candidate's own verdict settles it;
/// otherwise workflow derives coverage, and the derived approval is
/// recorded and published, or withdrawn while GitHub does not show it.
pub(super) fn candidate_finding(
    work: &ReviewWork,
    project: &instrukt_runner::interfaces::ProjectRef,
    repository: &str,
    candidate: &CommitSha,
    pulls: &[u64],
    now: Timestamp,
    clock: &dyn Fn() -> Timestamp,
) -> Result<instrukt_runner::interfaces::CandidateFinding, ReviewError> {
    use instrukt_runner::interfaces::CandidateFinding;
    let held = evidence(&work.home, &project.slug, candidate)?;
    if held
        .current()
        .is_some_and(|version| version.base_on_canonical)
    {
        return Ok(CandidateFinding::Settled);
    }
    let recorded = derived_approval(&work.home, &project.slug, candidate)?;
    let shown = |approval: &Derived| {
        held.deliveries.iter().any(|d| {
            d.destination == Destination::Status
                && d.state == DeliveryState::Shown
                && d.name == approval.name
        })
    };
    if let Some(approval) = recorded.as_ref().filter(|a| a.withdrawn.is_none())
        && shown(approval)
    {
        return Ok(CandidateFinding::Settled);
    }
    let canonical_branch = git::default_branch(&project.root, REMOTE)?;
    let canonical = format!("refs/remotes/{REMOTE}/{canonical_branch}");
    let reader = CoverageReader { work, project };
    match crate::workflow::coverage(&project.root, candidate, &canonical, &reader) {
        crate::workflow::Coverage::CannotAnswer(cause) => Ok(CandidateFinding::CannotAnswer(cause)),
        crate::workflow::Coverage::Uncovered(commit) => {
            if let Some(approval) = recorded.as_ref().filter(|a| a.withdrawn.is_none()) {
                let mut db = _published::open(&work.home)?;
                db.write(|tx| _published::withdraw(tx, &project.slug, approval, &commit, now))?;
                tracing::info!(event = "review_derived_approval_withdrawn", project = %project.slug, candidate = %candidate, uncovered = %commit);
            }
            Ok(CandidateFinding::Owed)
        }
        crate::workflow::Coverage::Covered(items) => {
            let based = git::run(
                &project.root,
                ["merge-base", candidate.as_str(), canonical.as_str()],
            )?;
            let coverage = serde_json::to_string_pretty(&serde_json::json!({
                "candidate": candidate.as_str(),
                "canonical": based.trim(),
                "covering": items.iter().map(|item| serde_json::json!({
                    "commit": item.commit.as_str(),
                    "kind": item.kind.as_str(),
                    "evidence": item.reference,
                })).collect::<Vec<_>>(),
            }))
            .map_err(|e| ReviewError::Record(e.to_string()))?;
            let approval = Derived {
                candidate: candidate.clone(),
                canonical: based.trim().to_owned(),
                name: ContentDigest::of(&coverage),
                coverage,
                moment: Moment::decided(now, None),
                recorded_at: now,
                withdrawn: None,
                imported: false,
            };
            let mut db = _published::open(&work.home)?;
            db.write(|tx| _published::record_derived(tx, &project.slug, &approval))?;
            drop(db);
            tracing::info!(event = "review_derived_approval_recorded", project = %project.slug, candidate = %candidate, items = items.len());
            let gh = work
                .access
                .review_client(repository, clock)
                .map_err(|error| refused(error.to_string()))?;
            let reviewer = work
                .access
                .review_identities()
                .map_err(|error| refused(error.to_string()))?
                .reviewer;
            _recognition::Publication {
                home: &work.home,
                gh: &gh,
                project: &project.slug,
                repository,
                reviewer: &reviewer,
                now,
            }
            .deliver(candidate, &derived_target(&approval), pulls)?;
            Ok(CandidateFinding::Covered)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sha(c: char) -> CommitSha {
        CommitSha::parse(&c.to_string().repeat(40)).unwrap()
    }

    fn build(verdict: crate::evidence::BuildVerdict, target: char) -> crate::evidence::StampFact {
        crate::evidence::StampFact::BuildVerdict {
            verdict,
            target: sha(target),
            report: ContentDigest::of("report"),
        }
    }

    #[test]
    fn a_finalized_approval_cites_the_todos_pull_request_once_its_own_verdict_is_reflected_there() {
        use crate::evidence::BuildVerdict::{Approve, ChangesRequested};
        let facts = [build(ChangesRequested, 'a'), build(Approve, 'a')];
        let stamps = [(3, &facts[0]), (7, &facts[1])];
        let approval = record_approval(stamps, Some(&sha('a'))).unwrap();
        assert_eq!(approval.build, Some(7));
        assert_eq!(approval.cited_pull(Some(&(12, vec![3, 7]))), Some(12));
        assert_eq!(
            approval.cited_pull(Some(&(12, vec![3]))),
            None,
            "not yet reflected there"
        );
        assert_eq!(approval.cited_pull(None), None);
    }

    #[test]
    fn an_approval_is_cited_when_an_earlier_verdict_is_not_reflected() {
        use crate::evidence::BuildVerdict::{Approve, ChangesRequested};
        let facts = [build(ChangesRequested, 'a'), build(Approve, 'a')];
        let stamps = [(3, &facts[0]), (7, &facts[1])];
        let approval = record_approval(stamps, Some(&sha('a'))).unwrap();
        assert_eq!(approval.cited_pull(Some(&(12, vec![7]))), Some(12));
    }

    #[test]
    fn a_reviewed_preparation_heads_approval_cites_no_pull_request() {
        let facts = [crate::evidence::StampFact::PreparationHeadReviewed {
            verdict: crate::evidence::ReviewVerdict::Approve,
            head: sha('a'),
            report: ContentDigest::of("report"),
        }];
        let approval = record_approval([(1, &facts[0])], Some(&sha('a'))).unwrap();
        assert_eq!(approval.build, None);
        assert_eq!(approval.cited_pull(Some(&(12, vec![1]))), None);
    }
}
