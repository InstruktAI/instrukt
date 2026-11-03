//! The fact provider: a todo's recorded evidence and the repository
//! observations made for it, translated into the engine's typed [`Facts`]
//! (`docs/local/spec/domain/workflow.md`, the fact table and its honesty
//! rules; `artifact-currency`).
//!
//! Pure: it reads only its input. Everything the record cannot say about
//! the repository (heads, cleanliness, review notes, whether a reviewed
//! commit is still the todo's work) arrives as [`Observations`], made by
//! `_observe`. A fact with no evidence is not supplied; `false` and `0` are
//! supplied as values.

use instrukt_core::config::Approver;
use instrukt_core::ids::{CommitSha, SessionRef};
use jiff::Timestamp;

use crate::evidence::{
    ApprovalGate, ApprovedBy, ArtifactName, BuildVerdict, ContentDigest, DriftClass,
    PreparationReview, Prerequisite, ReviewVerdict, StampFact, TodoEvidence, artifact_names,
};

use super::_facts::{FactError, FactValue, Facts};

/// The artifact a build review's complete report is written to
/// (`review-build`: `instrukt todo artifacts write {slug} review-findings`).
/// Work's `artifact_names` has no constant for it yet; reported to the work
/// builder.
pub(crate) const BUILD_REVIEW_FINDINGS: &str = "review_findings";

/// Artifacts whose new version makes an earlier build completion stale
/// (`workflow` domain: "a new build-relevant artifact version clears
/// `build.complete`"): what the build implements and the checklist it is
/// admitted against.
const BUILD_RELEVANT: [&str; 2] = [
    artifact_names::IMPLEMENTATION_PLAN,
    artifact_names::QUALITY_CHECKLIST,
];

/// Who may approve each gate (`approvals.prd`, `approvals.task`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Approvers {
    pub prd: Approver,
    pub task: Approver,
}

/// Where one prerequisite stands, read from its own evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PrerequisiteStanding {
    Delivered,
    Undelivered,
    /// A structural holder with no children: never satisfied by having none.
    EmptyHolder,
    /// The record no longer has the todo.
    NotFound,
}

/// Whether the todo's work changed since a reviewed commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChangeSinceReview {
    /// The reviewed commit is the current head.
    Unchanged,
    /// Only clean merges of the canonical branch separate them.
    CleanUpstreamMerges,
    TodoChanged,
    /// The repository could not tell.
    Undetermined,
}

/// The assignment worktree, while it exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorktreeObservation {
    pub clean: bool,
    pub head: Option<CommitSha>,
}

/// What the repository says, observed for this call.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Observations {
    /// `None` when the worktree does not exist.
    pub worktree: Option<WorktreeObservation>,
    /// The local todo branch.
    pub branch_head: Option<CommitSha>,
    /// The todo branch on the remote, as last fetched.
    pub remote_head: Option<CommitSha>,
    /// Canonical-branch commits of the todo whose commit-review settlement
    /// is neither `approve` nor `exempt`.
    pub unreviewed_claim_commits: Vec<CommitSha>,
    /// For the current build verdict's reviewed commit; `None` when there
    /// is no current verdict or no head to compare with.
    pub reviewed_change: Option<ChangeSinceReview>,
}

/// Everything one derivation of facts reads.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ProviderInput<'a> {
    pub evidence: &'a TodoEvidence,
    /// One standing per entry of `evidence.prerequisites`.
    pub prerequisites: &'a [PrerequisiteStanding],
    pub approvers: Approvers,
    pub observed: &'a Observations,
}

/// The typed facts for `input`.
pub(crate) fn derive_facts(input: &ProviderInput<'_>) -> Result<Facts, FactError> {
    let ledger = Ledger::new(input.evidence);
    let mut facts = Facts::new();
    facts.insert("kind", text(input.evidence.kind.as_str()))?;
    facts.insert(
        "deps.delivered",
        FactValue::Bool(
            input
                .prerequisites
                .iter()
                .all(|standing| *standing == PrerequisiteStanding::Delivered),
        ),
    )?;
    for name in [
        artifact_names::PRD,
        artifact_names::ARCHITECTURE,
        artifact_names::DIAGNOSIS,
        artifact_names::IMPLEMENTATION_PLAN,
        artifact_names::PRD_REVIEW_FINDINGS,
        artifact_names::ARCHITECTURE_REVIEW_FINDINGS,
        artifact_names::PLAN_REVIEW_FINDINGS,
        artifact_names::TASK_PLAN,
        artifact_names::QUALITY_CHECKLIST,
    ] {
        if let Some(digest) = ledger.current(name) {
            facts.insert(&format!("{name}.digest"), text(digest.as_str()))?;
        }
    }
    preparation_reviews(&ledger, &mut facts)?;
    approvals(&ledger, input.approvers, &mut facts)?;
    if let Some(reason) = ledger.latest(|fact| match fact {
        StampFact::ArchitectureSkipped { reason } => Some(reason),
        _ => None,
    }) {
        facts.insert("architecture_skipped.reason", text(reason))?;
    }
    if let Some(Some(diagnosis)) = ledger.latest(|fact| match fact {
        StampFact::DiagnosisSettled { diagnosis } => Some(Some(diagnosis)),
        StampFact::DiagnosisUnresolved { .. } => Some(None),
        _ => None,
    }) {
        facts.insert("diagnosis_settled.digest", text(diagnosis.as_str()))?;
    }
    facts.insert(
        "claim_commits.unreviewed",
        FactValue::ShaList(input.observed.unreviewed_claim_commits.clone()),
    )?;
    drift(&ledger, &mut facts)?;
    build(&ledger, input.observed, &mut facts)?;
    lane_resources(input.evidence, input.observed, &mut facts)?;
    if let Some(commit) = &input.evidence.delivery {
        facts.insert("delivery.commit", FactValue::Sha(commit.clone()))?;
    }
    Ok(facts)
}

/// The current build verdict: the latest build-review verdict stamp after
/// the latest build completion and the latest kickback, each of which
/// clears the verdicts before it. Its reviewed commit is what
/// `_observe` compares the current head with.
pub(crate) fn current_build_verdict(
    evidence: &TodoEvidence,
) -> Option<(BuildVerdict, &CommitSha, &ContentDigest)> {
    let ledger = Ledger::new(evidence);
    let since = ledger
        .build_reset()
        .max(ledger.position(|fact| matches!(fact, StampFact::BuildComplete { .. }).then_some(())));
    ledger.latest_after(since, |fact| match fact {
        StampFact::BuildVerdict {
            verdict,
            target,
            report,
        } => Some((*verdict, target, report)),
        _ => None,
    })
}

fn text(value: &str) -> FactValue {
    FactValue::Text(value.to_owned())
}

/// The stamp ledger and artifact histories, read in record order.
struct Ledger<'e> {
    evidence: &'e TodoEvidence,
}

impl<'e> Ledger<'e> {
    fn new(evidence: &'e TodoEvidence) -> Self {
        Self { evidence }
    }

    /// The current version's digest of artifact `name`.
    fn current(&self, name: &str) -> Option<&'e ContentDigest> {
        self.history(name)
            .and_then(|versions| versions.last())
            .map(|version| &version.digest)
    }

    fn history(&self, name: &str) -> Option<&'e [crate::evidence::ArtifactVersion]> {
        self.evidence
            .artifacts
            .iter()
            .find(|artifact| artifact.name.as_str() == name)
            .map(|artifact| artifact.versions.as_slice())
    }

    /// Whether artifact `name` has a version created after `at`.
    fn rewritten_after(&self, name: &str, at: Timestamp) -> bool {
        self.history(name)
            .is_some_and(|versions| versions.iter().any(|version| version.created > at))
    }

    /// The latest stamp `pick` selects from, with its value.
    fn latest<T>(&self, pick: impl Fn(&'e StampFact) -> Option<T>) -> Option<T> {
        self.latest_after(None, pick)
    }

    /// The latest stamp `pick` selects at a ledger position after `since`.
    fn latest_after<T>(
        &self,
        since: Option<usize>,
        pick: impl Fn(&'e StampFact) -> Option<T>,
    ) -> Option<T> {
        self.evidence
            .stamps
            .iter()
            .enumerate()
            .rev()
            .take_while(|(index, _)| since.is_none_or(|since| *index > since))
            .find_map(|(_, stamp)| pick(&stamp.fact))
    }

    /// The ledger position of the latest stamp `pick` selects.
    fn position(&self, pick: impl Fn(&'e StampFact) -> Option<()>) -> Option<usize> {
        self.evidence
            .stamps
            .iter()
            .enumerate()
            .rev()
            .find_map(|(index, stamp)| pick(&stamp.fact).map(|()| index))
    }

    /// The time of the stamp at `position`.
    fn at(&self, position: usize) -> Option<Timestamp> {
        self.evidence.stamps.get(position).map(|stamp| stamp.at)
    }

    /// The latest kickback: build and review facts before it are cleared.
    fn build_reset(&self) -> Option<usize> {
        self.position(|fact| matches!(fact, StampFact::Kickback { .. }).then_some(()))
    }

    /// Ledger positions of the stamps that invalidate review `review`: a
    /// substantive drift judgment of an artifact whose settlement
    /// invalidates it (`artifact-currency`, "Substantive settlement"), or a
    /// kickback, which invalidates the plan and its review.
    fn invalidations(&self, review: PreparationReview) -> impl Iterator<Item = usize> + '_ {
        self.evidence
            .stamps
            .iter()
            .enumerate()
            .filter(move |(_, stamp)| match &stamp.fact {
                StampFact::Kickback { .. } => review == PreparationReview::PlanReview,
                StampFact::DriftReviewed {
                    artifact,
                    class: DriftClass::Substantive,
                    ..
                } => invalidated_by(artifact).contains(&review),
                _ => false,
            })
            .map(|(index, _)| index)
    }
}

/// The reviews a substantive judgment of `artifact` invalidates.
fn invalidated_by(artifact: &ArtifactName) -> &'static [PreparationReview] {
    use PreparationReview::{ArchitectureReview, PlanReview, PrdReview};
    match artifact.as_str() {
        artifact_names::PRD => &[PrdReview, ArchitectureReview, PlanReview],
        artifact_names::ARCHITECTURE => &[ArchitectureReview, PlanReview],
        artifact_names::DIAGNOSIS | artifact_names::IMPLEMENTATION_PLAN => &[PlanReview],
        _ => &[],
    }
}

/// The fact-name prefix of a preparation review, and the artifact it
/// reviews.
fn review_names(review: PreparationReview) -> (&'static str, &'static str) {
    match review {
        PreparationReview::PrdReview => ("prd_review", artifact_names::PRD),
        PreparationReview::ArchitectureReview => {
            ("architecture_review", artifact_names::ARCHITECTURE)
        }
        PreparationReview::PlanReview => ("plan_review", artifact_names::IMPLEMENTATION_PLAN),
    }
}

/// `<r>.verdict`, `<r>.target_digest`, `<r>.report_digest`,
/// `<r>.cleared_by_substantive_drift` and `<r>.rounds`. Only verdicts after
/// the latest invalidation of `r` count. An invalidation keeps `r` cleared
/// until the artifact `r` reviews is rewritten after it.
fn preparation_reviews(ledger: &Ledger<'_>, facts: &mut Facts) -> Result<(), FactError> {
    for review in [
        PreparationReview::PrdReview,
        PreparationReview::ArchitectureReview,
        PreparationReview::PlanReview,
    ] {
        let (prefix, target) = review_names(review);
        let invalidations: Vec<usize> = ledger.invalidations(review).collect();
        let since = invalidations.last().copied();
        let verdict_of = |fact: &StampFact| match fact {
            StampFact::PreparationVerdict { phase, .. } => (*phase == review).then_some(()),
            _ => None,
        };
        if let Some((verdict, target_digest, report)) =
            ledger.latest_after(since, |fact| match fact {
                StampFact::PreparationVerdict {
                    phase,
                    verdict,
                    target,
                    report,
                } if *phase == review => Some((*verdict, target, report)),
                _ => None,
            })
        {
            let verdict = match verdict {
                ReviewVerdict::Approve => "approve",
                ReviewVerdict::NeedsWork => "needs_work",
            };
            facts.insert(&format!("{prefix}.verdict"), text(verdict))?;
            facts.insert(
                &format!("{prefix}.target_digest"),
                text(target_digest.as_str()),
            )?;
            facts.insert(&format!("{prefix}.report_digest"), text(report.as_str()))?;
        }
        let cleared = invalidations.iter().any(|&position| {
            ledger
                .at(position)
                .is_some_and(|at| !ledger.rewritten_after(target, at))
        });
        facts.insert(
            &format!("{prefix}.cleared_by_substantive_drift"),
            FactValue::Bool(cleared),
        )?;
        let rounds = ledger
            .evidence
            .stamps
            .iter()
            .enumerate()
            .filter(|(index, stamp)| {
                since.is_none_or(|since| *index > since) && verdict_of(&stamp.fact).is_some()
            })
            .count();
        facts.insert(
            &format!("{prefix}.rounds"),
            FactValue::Int(i64::try_from(rounds).unwrap_or(i64::MAX)),
        )?;
    }
    Ok(())
}

/// Whether an approval by `by` counts for a gate `approver` decides: a
/// human gate admits only a human's approval; a peer gate admits a peer's
/// approval on the person's behalf, and the person's own.
fn eligible(approver: Approver, by: ApprovedBy) -> bool {
    match approver {
        Approver::Human => by == ApprovedBy::Human,
        Approver::Peer => true,
    }
}

/// `prd_approval.*` and `task_approved.*`: the latest approval of each gate
/// from an eligible approver.
fn approvals(
    ledger: &Ledger<'_>,
    approvers: Approvers,
    facts: &mut Facts,
) -> Result<(), FactError> {
    let latest = |gate: ApprovalGate, approver: Approver| {
        ledger.latest(|fact| match fact {
            StampFact::Approval {
                gate: stamped,
                by,
                target,
                ..
            } if *stamped == gate && eligible(approver, *by) => Some((*by, target)),
            _ => None,
        })
    };
    if let Some((by, target)) = latest(ApprovalGate::Prd, approvers.prd) {
        facts.insert("prd_approval.digest", text(target.as_str()))?;
        facts.insert(
            "prd_approval.by_human",
            FactValue::Bool(by == ApprovedBy::Human),
        )?;
    }
    if let Some((by, target)) = latest(ApprovalGate::Task, approvers.task) {
        facts.insert("task_approved.target_digest", text(target.as_str()))?;
        let by = match by {
            ApprovedBy::Human => "human",
            ApprovedBy::Peer => "peer",
        };
        facts.insert("task_approved.by", text(by))?;
    }
    Ok(())
}

/// `drift.<artifact>`: the latest recorded classification, while the
/// artifact has not been rewritten after it. A rewrite clears it
/// (`artifact-currency`, "Substantive settlement"). Pending and fresh need
/// recorded provenance, which the evidence does not carry yet; they are
/// not supplied.
fn drift(ledger: &Ledger<'_>, facts: &mut Facts) -> Result<(), FactError> {
    for name in [
        artifact_names::PRD,
        artifact_names::ARCHITECTURE,
        artifact_names::IMPLEMENTATION_PLAN,
    ] {
        let judged = ledger
            .evidence
            .stamps
            .iter()
            .rev()
            .find_map(|stamp| match &stamp.fact {
                StampFact::DriftReviewed {
                    artifact, class, ..
                } if artifact.as_str() == name => Some((*class, stamp.at)),
                _ => None,
            });
        if let Some((class, at)) = judged
            && !ledger.rewritten_after(name, at)
        {
            let class = match class {
                DriftClass::Cosmetic => "cosmetic",
                DriftClass::Substantive => "substantive",
            };
            facts.insert(&format!("drift.{name}"), text(class))?;
        }
    }
    Ok(())
}

/// Build start, completion, review and finalization facts.
fn build(ledger: &Ledger<'_>, observed: &Observations, facts: &mut Facts) -> Result<(), FactError> {
    if let Some((base, plan)) = ledger.latest(|fact| match fact {
        StampFact::BuildStarted { base, plan } => Some((base, plan)),
        _ => None,
    }) {
        facts.insert("build.start_base_sha", text(base.as_str()))?;
        facts.insert("build.plan_digest", text(plan.as_str()))?;
    }
    let reset = ledger.build_reset();
    let completion =
        ledger.position(|fact| matches!(fact, StampFact::BuildComplete { .. }).then_some(()));
    let current_completion =
        completion.filter(|position| reset.is_none_or(|reset| *position > reset));
    facts.insert(
        "build.complete",
        FactValue::Bool(
            current_completion.is_some_and(|position| completion_holds(ledger, position, observed)),
        ),
    )?;
    if let Some((verdict, target, report)) = current_build_verdict(ledger.evidence) {
        let verdict = match verdict {
            BuildVerdict::Approve => "approve",
            BuildVerdict::ChangesRequested => "changes_requested",
        };
        facts.insert("build_review.verdict", text(verdict))?;
        facts.insert("build_review.target_digest", FactValue::Sha(target.clone()))?;
        facts.insert(
            "build_review.binding",
            text(binding(ledger, report, observed.reviewed_change)),
        )?;
    }
    let rounds = ledger
        .evidence
        .stamps
        .iter()
        .enumerate()
        .filter(|(index, stamp)| {
            reset.is_none_or(|reset| *index > reset)
                && matches!(stamp.fact, StampFact::BuildVerdict { .. })
        })
        .count();
    facts.insert(
        "build_review.round",
        FactValue::Int(i64::try_from(rounds).unwrap_or(i64::MAX)),
    )?;
    let deferrals_pending = ledger
        .current(artifact_names::DEFERRALS)
        .is_some_and(|current| {
            ledger.latest(|fact| match fact {
                StampFact::DeferralsProcessed { deferrals } => Some(deferrals),
                _ => None,
            }) != Some(current)
        });
    facts.insert("deferrals.pending", FactValue::Bool(deferrals_pending))?;
    let finalized = ledger.latest_after(reset.max(completion), |fact| match fact {
        StampFact::FinalizeReady { commit } => Some(commit),
        _ => None,
    });
    match finalized {
        Some(commit) => {
            facts.insert("finalize.status", text("ready"))?;
            facts.insert("finalize.sha", FactValue::Sha(commit.clone()))?;
        }
        None => facts.insert("finalize.status", text("pending"))?,
    }
    Ok(())
}

/// Whether the completion stamped at `position` still holds: no
/// build-relevant artifact was rewritten after it, and the work branch has
/// not moved from the stamped head. A head that cannot be observed is no
/// evidence of a later commit.
fn completion_holds(ledger: &Ledger<'_>, position: usize, observed: &Observations) -> bool {
    let Some(stamp) = ledger.evidence.stamps.get(position) else {
        return false;
    };
    let StampFact::BuildComplete { head } = &stamp.fact else {
        return false;
    };
    let rewritten = BUILD_RELEVANT
        .iter()
        .any(|name| ledger.rewritten_after(name, stamp.at));
    let current_head = observed
        .worktree
        .as_ref()
        .and_then(|worktree| worktree.head.as_ref())
        .or(observed.branch_head.as_ref());
    !rewritten && current_head.is_none_or(|current| current == head)
}

/// `build_review.binding` for a verdict whose report digest is `report`
/// (`artifact-currency`, "Review bindings").
fn binding(
    ledger: &Ledger<'_>,
    report: &ContentDigest,
    change: Option<ChangeSinceReview>,
) -> &'static str {
    match ledger.current(BUILD_REVIEW_FINDINGS) {
        None => "unbound",
        Some(current) if current != report => "report_changed",
        Some(_) => match change {
            Some(ChangeSinceReview::Unchanged) => "exact_target",
            Some(ChangeSinceReview::CleanUpstreamMerges) => "clean_upstream_merges",
            Some(ChangeSinceReview::TodoChanged) => "todo_changed",
            Some(ChangeSinceReview::Undetermined) | None => "undetermined",
        },
    }
}

/// The claim, the session bindings and the assignment worktree.
fn lane_resources(
    evidence: &TodoEvidence,
    observed: &Observations,
    facts: &mut Facts,
) -> Result<(), FactError> {
    facts.insert("claim.held", FactValue::Bool(evidence.claim.is_some()))?;
    let mut sessions: Vec<SessionRef> = Vec::new();
    for binding in &evidence.bindings {
        if !sessions.contains(&binding.binding.session) {
            sessions.push(binding.binding.session.clone());
        }
    }
    facts.insert("sessions.bound", FactValue::Sessions(sessions))?;
    if let Some(path) = evidence.bindings.iter().rev().find_map(|b| b.worktree()) {
        facts.insert("worktree.path", text(&path.display().to_string()))?;
    }
    if let Some(worktree) = &observed.worktree {
        facts.insert("worktree.exists", FactValue::Bool(true))?;
        facts.insert("worktree.clean", FactValue::Bool(worktree.clean))?;
        if let Some(head) = &worktree.head {
            facts.insert("worktree.head", FactValue::Sha(head.clone()))?;
        }
    }
    if let Some(head) = &observed.branch_head {
        facts.insert("branch.head", FactValue::Sha(head.clone()))?;
    }
    if let Some(head) = &observed.remote_head {
        facts.insert("remote.head", FactValue::Sha(head.clone()))?;
    }
    Ok(())
}

/// The standing of each of `evidence`'s prerequisites, given how to read a
/// prerequisite todo's own evidence.
pub(crate) fn standings<E>(
    evidence: &TodoEvidence,
    mut read: impl FnMut(&instrukt_core::ids::TodoSlug) -> Result<Option<TodoEvidence>, E>,
) -> Result<Vec<PrerequisiteStanding>, E> {
    let mut out = Vec::with_capacity(evidence.prerequisites.len());
    for prerequisite in &evidence.prerequisites {
        out.push(match prerequisite {
            Prerequisite::EmptyHolder(_) => PrerequisiteStanding::EmptyHolder,
            Prerequisite::Todo(slug) => match read(slug)? {
                Some(todo) if todo.status == crate::evidence::TodoStatus::Delivered => {
                    PrerequisiteStanding::Delivered
                }
                Some(_) => PrerequisiteStanding::Undelivered,
                None => PrerequisiteStanding::NotFound,
            },
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use instrukt_runner::interfaces::TodoKind;

    use super::super::_facts::FactName;
    use super::super::_testing::{EvidenceBuilder, digest, session, sha, text, todo};
    use super::*;

    const HUMAN_GATES: Approvers = Approvers {
        prd: Approver::Human,
        task: Approver::Human,
    };

    fn facts_of(evidence: &TodoEvidence, observed: &Observations) -> Facts {
        facts_with(evidence, &[], HUMAN_GATES, observed)
    }

    fn facts_with(
        evidence: &TodoEvidence,
        prerequisites: &[PrerequisiteStanding],
        approvers: Approvers,
        observed: &Observations,
    ) -> Facts {
        derive_facts(&ProviderInput {
            evidence,
            prerequisites,
            approvers,
            observed,
        })
        .unwrap()
    }

    fn get<'f>(facts: &'f Facts, name: &str) -> Option<&'f FactValue> {
        facts.get(FactName::parse(name).unwrap())
    }

    fn yes() -> Option<FactValue> {
        Some(FactValue::Bool(true))
    }

    fn no() -> Option<FactValue> {
        Some(FactValue::Bool(false))
    }

    fn verdict(phase: PreparationReview, verdict: ReviewVerdict, target: &str) -> StampFact {
        StampFact::PreparationVerdict {
            phase,
            verdict,
            target: digest(target),
            report: digest("findings-1"),
        }
    }

    fn substantive(artifact: &str) -> StampFact {
        StampFact::DriftReviewed {
            artifact: ArtifactName::new(artifact),
            class: DriftClass::Substantive,
            reason: "scope moved".into(),
            examined: digest("evidence"),
        }
    }

    fn kickback() -> StampFact {
        StampFact::Kickback {
            reason: "the plan misses the API".into(),
            input: digest("input-2"),
            invalidated_plan: Some(digest("plan-1")),
        }
    }

    fn approval(gate: ApprovalGate, by: ApprovedBy, target: &str) -> StampFact {
        StampFact::Approval {
            gate,
            by,
            target: digest(target),
            carried_from: None,
        }
    }

    fn completed(head: char) -> StampFact {
        StampFact::BuildComplete { head: sha(head) }
    }

    fn build_verdict(verdict: BuildVerdict, target: char, report: &str) -> StampFact {
        StampFact::BuildVerdict {
            verdict,
            target: sha(target),
            report: digest(report),
        }
    }

    /// A worktree whose head is `head`.
    fn at_head(head: char) -> Observations {
        Observations {
            worktree: Some(WorktreeObservation {
                clean: true,
                head: Some(sha(head)),
            }),
            ..Observations::default()
        }
    }

    /// A built change: the plan and checklist written, completion stamped
    /// at `a`.
    fn built() -> EvidenceBuilder {
        todo(TodoKind::Change)
            .artifact(artifact_names::IMPLEMENTATION_PLAN, "plan-1")
            .artifact(artifact_names::QUALITY_CHECKLIST, "checklist-1")
            .stamp(completed('a'))
    }

    /// A built change with an approving build verdict on `a` whose report
    /// is written.
    fn reviewed() -> EvidenceBuilder {
        built()
            .artifact(BUILD_REVIEW_FINDINGS, "report-1")
            .stamp(build_verdict(BuildVerdict::Approve, 'a', "report-1"))
    }

    fn observed_change(head: char, change: ChangeSinceReview) -> Observations {
        Observations {
            reviewed_change: Some(change),
            ..at_head(head)
        }
    }

    // ---- record facts ----

    #[test]
    fn an_artifact_digest_is_its_current_version() {
        let evidence = todo(TodoKind::Feature)
            .artifact(artifact_names::PRD, "prd-1")
            .artifact(artifact_names::PRD, "prd-2")
            .build();
        let facts = facts_of(&evidence, &Observations::default());
        assert_eq!(get(&facts, "prd.digest"), Some(&text("prd-2")));
        assert_eq!(get(&facts, "kind"), Some(&text("feature")));
    }

    #[test]
    fn a_fresh_todo_supplies_typed_false_and_leaves_missing_evidence_absent() {
        let facts = facts_of(&todo(TodoKind::Bug).build(), &Observations::default());
        assert_eq!(get(&facts, "prd.digest"), None);
        assert_eq!(get(&facts, "prd_review.verdict"), None);
        assert_eq!(get(&facts, "claim_commits.unreviewed"), None);
        assert_eq!(get(&facts, "worktree.exists"), None);
        assert_eq!(get(&facts, "claim.held").cloned(), no());
        assert_eq!(get(&facts, "build.complete").cloned(), no());
        assert_eq!(get(&facts, "deferrals.pending").cloned(), no());
        assert_eq!(get(&facts, "plan_review.rounds"), Some(&FactValue::Int(0)));
        assert_eq!(get(&facts, "finalize.status"), Some(&text("pending")));
        assert_eq!(get(&facts, "deps.delivered").cloned(), yes());
    }

    #[test]
    fn dependencies_are_delivered_only_when_every_prerequisite_is() {
        let evidence = todo(TodoKind::Change).build();
        let one_open = [
            PrerequisiteStanding::Delivered,
            PrerequisiteStanding::Undelivered,
        ];
        let facts = facts_with(&evidence, &one_open, HUMAN_GATES, &Observations::default());
        assert_eq!(get(&facts, "deps.delivered").cloned(), no());
    }

    #[test]
    fn an_empty_holder_never_satisfies_a_dependency() {
        let evidence = todo(TodoKind::Change).build();
        let holder = [PrerequisiteStanding::EmptyHolder];
        let facts = facts_with(&evidence, &holder, HUMAN_GATES, &Observations::default());
        assert_eq!(get(&facts, "deps.delivered").cloned(), no());
    }

    #[test]
    fn the_latest_review_verdict_binds_its_target_and_report_and_counts_rounds() {
        let evidence = todo(TodoKind::Feature)
            .artifact(artifact_names::PRD, "prd-1")
            .stamp(verdict(
                PreparationReview::PrdReview,
                ReviewVerdict::NeedsWork,
                "prd-1",
            ))
            .artifact(artifact_names::PRD, "prd-2")
            .stamp(verdict(
                PreparationReview::PrdReview,
                ReviewVerdict::Approve,
                "prd-2",
            ))
            .build();
        let facts = facts_of(&evidence, &Observations::default());
        assert_eq!(get(&facts, "prd_review.verdict"), Some(&text("approve")));
        assert_eq!(
            get(&facts, "prd_review.target_digest"),
            Some(&text("prd-2"))
        );
        assert_eq!(
            get(&facts, "prd_review.report_digest"),
            Some(&text("findings-1"))
        );
        assert_eq!(get(&facts, "prd_review.rounds"), Some(&FactValue::Int(2)));
        assert_eq!(get(&facts, "plan_review.verdict"), None);
    }

    #[test]
    fn substantive_prd_drift_clears_every_review_it_invalidates() {
        let evidence = todo(TodoKind::Feature)
            .artifact(artifact_names::PRD, "prd-1")
            .artifact(artifact_names::ARCHITECTURE, "arch-1")
            .stamp(verdict(
                PreparationReview::PrdReview,
                ReviewVerdict::Approve,
                "prd-1",
            ))
            .stamp(verdict(
                PreparationReview::ArchitectureReview,
                ReviewVerdict::Approve,
                "arch-1",
            ))
            .stamp(substantive(artifact_names::PRD))
            .build();
        let facts = facts_of(&evidence, &Observations::default());
        assert_eq!(get(&facts, "drift.prd"), Some(&text("substantive")));
        assert_eq!(get(&facts, "prd_review.verdict"), None);
        assert_eq!(get(&facts, "architecture_review.verdict"), None);
        for review in ["prd_review", "architecture_review", "plan_review"] {
            assert_eq!(
                get(&facts, &format!("{review}.cleared_by_substantive_drift")).cloned(),
                yes(),
                "{review}"
            );
        }
    }

    #[test]
    fn rewriting_the_drifted_prd_settles_it_but_not_the_downstream_reviews() {
        let evidence = todo(TodoKind::Feature)
            .artifact(artifact_names::PRD, "prd-1")
            .artifact(artifact_names::ARCHITECTURE, "arch-1")
            .stamp(substantive(artifact_names::PRD))
            .artifact(artifact_names::PRD, "prd-2")
            .build();
        let facts = facts_of(&evidence, &Observations::default());
        assert_eq!(get(&facts, "drift.prd"), None);
        assert_eq!(
            get(&facts, "prd_review.cleared_by_substantive_drift").cloned(),
            no()
        );
        assert_eq!(
            get(&facts, "architecture_review.cleared_by_substantive_drift").cloned(),
            yes()
        );
    }

    #[test]
    fn a_cosmetic_judgment_holds_while_the_artifact_is_unchanged() {
        let evidence = todo(TodoKind::Change)
            .artifact(artifact_names::IMPLEMENTATION_PLAN, "plan-1")
            .stamp(StampFact::DriftReviewed {
                artifact: ArtifactName::new(artifact_names::IMPLEMENTATION_PLAN),
                class: DriftClass::Cosmetic,
                reason: "wording".into(),
                examined: digest("evidence"),
            })
            .build();
        let facts = facts_of(&evidence, &Observations::default());
        assert_eq!(
            get(&facts, "drift.implementation_plan"),
            Some(&text("cosmetic"))
        );
        assert_eq!(
            get(&facts, "plan_review.cleared_by_substantive_drift").cloned(),
            no()
        );
    }

    #[test]
    fn a_kickback_clears_the_plan_review_and_the_build() {
        let evidence = reviewed()
            .stamp(verdict(
                PreparationReview::PlanReview,
                ReviewVerdict::Approve,
                "plan-1",
            ))
            .stamp(StampFact::FinalizeReady { commit: sha('a') })
            .stamp(kickback())
            .build();
        let facts = facts_of(&evidence, &at_head('a'));
        assert_eq!(get(&facts, "plan_review.verdict"), None);
        assert_eq!(
            get(&facts, "plan_review.cleared_by_substantive_drift").cloned(),
            yes()
        );
        assert_eq!(
            get(&facts, "prd_review.cleared_by_substantive_drift").cloned(),
            no()
        );
        assert_eq!(get(&facts, "build.complete").cloned(), no());
        assert_eq!(get(&facts, "build_review.verdict"), None);
        assert_eq!(get(&facts, "finalize.status"), Some(&text("pending")));
    }

    #[test]
    fn a_peer_approval_does_not_satisfy_a_human_gate() {
        let evidence = todo(TodoKind::Feature)
            .stamp(approval(ApprovalGate::Prd, ApprovedBy::Peer, "prd-1"))
            .build();
        let facts = facts_of(&evidence, &Observations::default());
        assert_eq!(get(&facts, "prd_approval.digest"), None);
    }

    #[test]
    fn a_peer_gate_admits_the_peer_approval() {
        let evidence = todo(TodoKind::Task)
            .stamp(approval(ApprovalGate::Task, ApprovedBy::Peer, "task-1"))
            .build();
        let gates = Approvers {
            prd: Approver::Human,
            task: Approver::Peer,
        };
        let facts = facts_with(&evidence, &[], gates, &Observations::default());
        assert_eq!(
            get(&facts, "task_approved.target_digest"),
            Some(&text("task-1"))
        );
        assert_eq!(get(&facts, "task_approved.by"), Some(&text("peer")));
    }

    #[test]
    fn a_human_approval_is_attributed_to_a_human() {
        let evidence = todo(TodoKind::Feature)
            .stamp(approval(ApprovalGate::Prd, ApprovedBy::Human, "prd-1"))
            .build();
        let facts = facts_of(&evidence, &Observations::default());
        assert_eq!(get(&facts, "prd_approval.digest"), Some(&text("prd-1")));
        assert_eq!(get(&facts, "prd_approval.by_human").cloned(), yes());
    }

    #[test]
    fn a_diagnosis_left_unresolved_after_settling_is_not_settled() {
        let evidence = todo(TodoKind::Bug)
            .artifact(artifact_names::DIAGNOSIS, "diag-1")
            .stamp(StampFact::DiagnosisSettled {
                diagnosis: digest("diag-1"),
            })
            .stamp(StampFact::DiagnosisUnresolved {
                diagnosis: digest("diag-1"),
                reason: "cannot reproduce".into(),
            })
            .build();
        let facts = facts_of(&evidence, &Observations::default());
        assert_eq!(get(&facts, "diagnosis_settled.digest"), None);
    }

    // ---- build ----

    #[test]
    fn a_completion_at_the_current_head_holds() {
        let facts = facts_of(&built().build(), &at_head('a'));
        assert_eq!(get(&facts, "build.complete").cloned(), yes());
    }

    #[test]
    fn a_commit_after_completion_clears_it() {
        let facts = facts_of(&built().build(), &at_head('b'));
        assert_eq!(get(&facts, "build.complete").cloned(), no());
    }

    #[test]
    fn a_new_plan_version_after_completion_clears_it() {
        let evidence = built()
            .artifact(artifact_names::IMPLEMENTATION_PLAN, "plan-2")
            .build();
        let facts = facts_of(&evidence, &at_head('a'));
        assert_eq!(get(&facts, "build.complete").cloned(), no());
    }

    #[test]
    fn a_completion_stamp_clears_the_earlier_build_verdict() {
        let evidence = built()
            .stamp(build_verdict(
                BuildVerdict::ChangesRequested,
                'a',
                "report-1",
            ))
            .stamp(completed('b'))
            .build();
        let facts = facts_of(&evidence, &at_head('b'));
        assert_eq!(get(&facts, "build_review.verdict"), None);
        assert_eq!(get(&facts, "build_review.round"), Some(&FactValue::Int(1)));
    }

    #[test]
    fn a_verdict_on_the_current_head_with_its_report_is_bound_exactly() {
        let observed = observed_change('a', ChangeSinceReview::Unchanged);
        let facts = facts_of(&reviewed().build(), &observed);
        assert_eq!(get(&facts, "build_review.verdict"), Some(&text("approve")));
        assert_eq!(
            get(&facts, "build_review.target_digest"),
            Some(&FactValue::Sha(sha('a')))
        );
        assert_eq!(
            get(&facts, "build_review.binding"),
            Some(&text("exact_target"))
        );
    }

    #[test]
    fn only_clean_upstream_merges_keep_a_verdict_current() {
        let observed = observed_change('c', ChangeSinceReview::CleanUpstreamMerges);
        let facts = facts_of(&reviewed().build(), &observed);
        assert_eq!(
            get(&facts, "build_review.binding"),
            Some(&text("clean_upstream_merges"))
        );
    }

    #[test]
    fn changed_todo_work_makes_the_verdict_stale() {
        let observed = observed_change('c', ChangeSinceReview::TodoChanged);
        let facts = facts_of(&reviewed().build(), &observed);
        assert_eq!(
            get(&facts, "build_review.binding"),
            Some(&text("todo_changed"))
        );
    }

    #[test]
    fn rewritten_findings_make_the_verdict_stale() {
        let evidence = reviewed()
            .artifact(BUILD_REVIEW_FINDINGS, "report-2")
            .build();
        let observed = observed_change('a', ChangeSinceReview::Unchanged);
        let facts = facts_of(&evidence, &observed);
        assert_eq!(
            get(&facts, "build_review.binding"),
            Some(&text("report_changed"))
        );
    }

    #[test]
    fn a_verdict_without_its_findings_is_unbound() {
        let evidence = built()
            .stamp(build_verdict(BuildVerdict::Approve, 'a', "report-1"))
            .build();
        let facts = facts_of(&evidence, &at_head('a'));
        assert_eq!(get(&facts, "build_review.binding"), Some(&text("unbound")));
    }

    #[test]
    fn an_unanswerable_repository_leaves_the_binding_undetermined() {
        let observed = observed_change('c', ChangeSinceReview::Undetermined);
        let facts = facts_of(&reviewed().build(), &observed);
        assert_eq!(
            get(&facts, "build_review.binding"),
            Some(&text("undetermined"))
        );
    }

    #[test]
    fn processed_deferrals_are_not_pending() {
        let evidence = built()
            .artifact(artifact_names::DEFERRALS, "deferrals-1")
            .stamp(StampFact::DeferralsProcessed {
                deferrals: digest("deferrals-1"),
            })
            .build();
        let facts = facts_of(&evidence, &at_head('a'));
        assert_eq!(get(&facts, "deferrals.pending").cloned(), no());
    }

    #[test]
    fn a_new_deferrals_version_is_pending_again() {
        let evidence = built()
            .artifact(artifact_names::DEFERRALS, "deferrals-1")
            .stamp(StampFact::DeferralsProcessed {
                deferrals: digest("deferrals-1"),
            })
            .artifact(artifact_names::DEFERRALS, "deferrals-2")
            .build();
        let facts = facts_of(&evidence, &at_head('a'));
        assert_eq!(get(&facts, "deferrals.pending").cloned(), yes());
    }

    #[test]
    fn finalize_readiness_after_completion_names_its_commit() {
        let evidence = built()
            .stamp(StampFact::FinalizeReady { commit: sha('a') })
            .build();
        let facts = facts_of(&evidence, &at_head('a'));
        assert_eq!(get(&facts, "finalize.status"), Some(&text("ready")));
        assert_eq!(get(&facts, "finalize.sha"), Some(&FactValue::Sha(sha('a'))));
    }

    #[test]
    fn a_later_completion_resets_finalize_readiness() {
        let evidence = built()
            .stamp(StampFact::FinalizeReady { commit: sha('a') })
            .stamp(completed('b'))
            .build();
        let facts = facts_of(&evidence, &at_head('b'));
        assert_eq!(get(&facts, "finalize.status"), Some(&text("pending")));
        assert_eq!(get(&facts, "finalize.sha"), None);
    }

    // ---- lane resources ----

    #[test]
    fn the_worktree_path_outlives_the_worktree() {
        let evidence = todo(TodoKind::Change)
            .claimed()
            .bound("work-orchestrator", "lead", None)
            .bound("builder", "build-1", Some("/work/add-login"))
            .build();
        let facts = facts_of(&evidence, &Observations::default());
        assert_eq!(get(&facts, "worktree.path"), Some(&text("/work/add-login")));
        assert_eq!(get(&facts, "worktree.exists"), None);
        assert_eq!(get(&facts, "claim.held").cloned(), yes());
        assert_eq!(
            get(&facts, "sessions.bound"),
            Some(&FactValue::Sessions(vec![
                session("lead"),
                session("build-1")
            ]))
        );
    }

    #[test]
    fn an_existing_worktree_reports_its_head_and_cleanliness() {
        let observed = Observations {
            worktree: Some(WorktreeObservation {
                clean: false,
                head: Some(sha('a')),
            }),
            branch_head: Some(sha('a')),
            remote_head: Some(sha('b')),
            unreviewed_claim_commits: vec![sha('c')],
            reviewed_change: None,
        };
        let facts = facts_of(&todo(TodoKind::Change).build(), &observed);
        assert_eq!(get(&facts, "worktree.exists").cloned(), yes());
        assert_eq!(get(&facts, "worktree.clean").cloned(), no());
        assert_eq!(
            get(&facts, "worktree.head"),
            Some(&FactValue::Sha(sha('a')))
        );
        assert_eq!(get(&facts, "remote.head"), Some(&FactValue::Sha(sha('b'))));
        assert_eq!(
            get(&facts, "claim_commits.unreviewed"),
            Some(&FactValue::ShaList(vec![sha('c')]))
        );
    }

    #[test]
    fn a_delivered_todo_names_its_delivered_commit() {
        let mut evidence = todo(TodoKind::Change)
            .status(crate::evidence::TodoStatus::Delivered)
            .build();
        evidence.delivery = Some(sha('d'));
        let facts = facts_of(&evidence, &Observations::default());
        assert_eq!(
            get(&facts, "delivery.commit"),
            Some(&FactValue::Sha(sha('d')))
        );
    }
}
