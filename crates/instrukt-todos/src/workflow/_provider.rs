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

use crate::evidence::{
    ApprovalGate, ApprovedBy, ArtifactName, BuildVerdict, ContentDigest, DriftClass,
    PreparationReview, Prerequisite, ReviewVerdict, StampFact, TodoEvidence, artifact_names,
    upstream_of,
};

use super::_facts::{FactError, FactValue, Facts, member};

/// The artifact a build review's complete report is written to.
#[cfg(test)]
pub(crate) const BUILD_REVIEW_FINDINGS: &str = artifact_names::REVIEW_FINDINGS;

/// Each artifact whose current digest is a fact, with that fact's name.
/// Fact names are the fact table's; artifact names are work's.
const DIGEST_FACTS: [(&str, &str); 9] = [
    (artifact_names::PRD, "prd.digest"),
    (artifact_names::ARCHITECTURE, "architecture.digest"),
    (artifact_names::DIAGNOSIS, "diagnosis.digest"),
    (
        artifact_names::IMPLEMENTATION_PLAN,
        "implementation_plan.digest",
    ),
    (
        artifact_names::PRD_REVIEW_FINDINGS,
        "prd_review_findings.digest",
    ),
    (
        artifact_names::ARCHITECTURE_REVIEW_FINDINGS,
        "architecture_review_findings.digest",
    ),
    (
        artifact_names::PLAN_REVIEW_FINDINGS,
        "plan_review_findings.digest",
    ),
    (artifact_names::TASK_PLAN, "task_plan.digest"),
    (
        artifact_names::QUALITY_CHECKLIST,
        "quality_checklist.digest",
    ),
];

/// Each artifact with a drift classification, with its fact's name.
pub(crate) const DRIFT_FACTS: [(&str, &str); 3] = [
    (artifact_names::PRD, "drift.prd"),
    (artifact_names::ARCHITECTURE, "drift.architecture"),
    (
        artifact_names::IMPLEMENTATION_PLAN,
        "drift.implementation_plan",
    ),
];

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
    /// For each tracked artifact (by artifact name), what its current
    /// version's recorded grounding shows against current evidence; an
    /// artifact without an entry shows no grounding change.
    pub grounding: std::collections::BTreeMap<String, GroundingChange>,
    /// For each tracked artifact whose latest drift judgment recorded the
    /// grounding it assessed, what that grounding shows now; a cosmetic
    /// judgment holds only while it shows no change.
    pub judged_grounding: std::collections::BTreeMap<String, GroundingChange>,
}

/// What a version's recorded grounding shows now (`artifact-currency`,
/// "Drift evidence").
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct GroundingChange {
    pub code: CodeChange,
    /// A referenced or owned snippet's current digest differs.
    pub snippets: bool,
    /// A prerequisite artifact's current digest differs.
    pub dependencies: bool,
    /// A referenced snippet or path does not resolve now: incomplete
    /// provenance (`artifact-currency`, "Cosmetic settlement with
    /// incomplete provenance"), never fresh until a judgment assesses it.
    pub unresolved: bool,
}

impl GroundingChange {
    /// Whether this is drift evidence: a change, code that could not be
    /// compared, or provenance that does not resolve, each pending until
    /// analyzed.
    fn drifted(self) -> bool {
        self.changed() || self.unresolved
    }

    /// Whether the recorded evidence changed or could not be compared. A
    /// judgment that assessed an unresolved reference still holds while it
    /// stays unresolved: only a change to it needs a new assessment.
    fn changed(self) -> bool {
        self.code != CodeChange::Unchanged || self.snippets || self.dependencies
    }
}

/// Whether a non-merge commit between the recorded and the current
/// canonical head changes a referenced path.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum CodeChange {
    /// None does, or the version references no paths.
    #[default]
    Unchanged,
    Changed,
    /// Git could not compare the two heads, or there is no current
    /// canonical head.
    Uncomparable,
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
    for (name, fact) in DIGEST_FACTS {
        if let Some(digest) = ledger.current(name) {
            facts.insert(fact, text(digest.as_str()))?;
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
    drift(&ledger, input.observed, &mut facts)?;
    build(&ledger, input.observed, &mut facts)?;
    lane_resources(input.evidence, input.observed, &mut facts)?;
    // A seeded plan is input to planning: once preparation writes a new
    // version, the current plan is derived and no longer seeded.
    let plan_versions = ledger
        .history(artifact_names::IMPLEMENTATION_PLAN)
        .map_or(0, <[_]>::len);
    facts.insert(
        "implementation_plan.seeded",
        FactValue::Bool(input.evidence.implementation_plan_seeded && plan_versions == 1),
    )?;
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

/// `build_review.baseline_commit`: the build-start base commit current
/// when the current build-review verdict was stamped (the latest build
/// start before it in the ledger).
fn baseline_commit<'e>(ledger: &Ledger<'e>) -> Option<&'e CommitSha> {
    let verdict = ledger
        .latest_at(|fact| matches!(fact, StampFact::BuildVerdict { .. }).then_some(()))?
        .0;
    ledger.evidence.stamps[..verdict]
        .iter()
        .rev()
        .find_map(|stamp| match &stamp.fact {
            StampFact::BuildStarted { base, .. } => Some(base),
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

    /// Whether artifact `name` has a version written after the mutation
    /// numbered `sequence`.
    fn rewritten_after(&self, name: &str, sequence: u64) -> bool {
        self.history(name)
            .is_some_and(|versions| versions.iter().any(|version| version.sequence > sequence))
    }

    /// The latest stamp `pick` selects, with its ledger position.
    fn latest_at<T>(&self, pick: impl Fn(&'e StampFact) -> Option<T>) -> Option<(usize, T)> {
        self.evidence
            .stamps
            .iter()
            .enumerate()
            .rev()
            .find_map(|(index, stamp)| pick(&stamp.fact).map(|value| (index, value)))
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

    /// The record sequence of the stamp at `position`.
    fn at(&self, position: usize) -> Option<u64> {
        self.evidence
            .stamps
            .get(position)
            .map(|stamp| stamp.sequence)
    }

    /// The latest kickback: build and review facts before it are cleared.
    fn build_reset(&self) -> Option<usize> {
        self.position(|fact| matches!(fact, StampFact::Kickback { .. }).then_some(()))
    }

    /// Ledger positions of the stamps that invalidate review `review`: a
    /// substantive drift judgment of an artifact whose settlement
    /// invalidates it (`artifact-currency`, "Substantive settlement"), or a
    /// kickback recording that it invalidates `review`.
    fn invalidations(&self, review: PreparationReview) -> impl Iterator<Item = usize> + '_ {
        self.evidence
            .stamps
            .iter()
            .enumerate()
            .filter(move |(_, stamp)| match &stamp.fact {
                StampFact::Kickback {
                    invalidated_reviews,
                    ..
                } => invalidated_reviews.contains(&review),
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
                ReviewVerdict::Approve => member::APPROVE,
                ReviewVerdict::NeedsWork => member::NEEDS_WORK,
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
/// from an eligible approver, while it still binds the evidence it approved
/// (`artifact-currency`, "Substantive settlement"; `approvals`, carry):
///
/// - an invalidation of the gate's review after the approval revokes it, so
///   re-deriving the same bytes does not revive it;
/// - it binds the version current when it was stamped: a later version of
///   the artifact, even one with an equal digest, is not approved by it.
///
/// An earlier approval reaches a later version only through a carry, which
/// work records as a new approval stamp (`--carry-from`) under a current
/// `approve` verdict on that version.
fn approvals(
    ledger: &Ledger<'_>,
    approvers: Approvers,
    facts: &mut Facts,
) -> Result<(), FactError> {
    let current =
        |gate: ApprovalGate, approver: Approver, artifact: &str, revoked: Option<usize>| {
            let (position, (by, target)) = ledger.latest_at(|fact| match fact {
                StampFact::Approval {
                    gate: stamped,
                    by,
                    target,
                    ..
                } if *stamped == gate && eligible(approver, *by) => Some((*by, target)),
                _ => None,
            })?;
            if revoked.is_some_and(|revoked| revoked > position) {
                return None;
            }
            let approved_current = ledger
                .history(artifact)
                .and_then(|versions| versions.last())
                .zip(ledger.at(position))
                .is_some_and(|(version, at)| version.sequence < at);
            approved_current.then_some((by, target))
        };
    let prd_revoked = ledger.invalidations(PreparationReview::PrdReview).last();
    if let Some((by, target)) = current(
        ApprovalGate::Prd,
        approvers.prd,
        artifact_names::PRD,
        prd_revoked,
    ) {
        facts.insert("prd_approval.digest", text(target.as_str()))?;
        facts.insert(
            "prd_approval.by_human",
            FactValue::Bool(by == ApprovedBy::Human),
        )?;
    }
    if let Some((by, target)) = current(
        ApprovalGate::Task,
        approvers.task,
        artifact_names::TASK_PLAN,
        None,
    ) {
        facts.insert("task_approved.target_digest", text(target.as_str()))?;
        let by = match by {
            ApprovedBy::Human => member::HUMAN,
            ApprovedBy::Peer => member::PEER,
        };
        facts.insert("task_approved.by", text(by))?;
    }
    Ok(())
}

/// `drift.<artifact>`: one effective classification per tracked artifact
/// (`artifact-currency`, "Drift evidence"), substantive before pending
/// before cosmetic before fresh.
///
/// - A recorded judgment counts while the artifact has not been rewritten
///   after it; a rewrite captures new provenance and clears it.
/// - The current version's recorded provenance is compared with each
///   upstream's current digest; a difference is drift evidence.
/// - Drift evidence is pending analysis, unless a cosmetic judgment made
///   after every upstream version it could have seen settles it: the same
///   evidence stays settled; a newer upstream version needs a new judgment.
/// - Without drift evidence, recorded provenance is fresh. A version written
///   before provenance was recorded supplies no drift fact.
fn drift(ledger: &Ledger<'_>, observed: &Observations, facts: &mut Facts) -> Result<(), FactError> {
    for (name, fact) in DRIFT_FACTS {
        let Some(version) = ledger.history(name).and_then(|versions| versions.last()) else {
            continue;
        };
        let judged = ledger
            .evidence
            .stamps
            .iter()
            .rev()
            .find_map(|stamp| match &stamp.fact {
                StampFact::DriftReviewed {
                    artifact, class, ..
                } if artifact.as_str() == name => Some((*class, stamp.sequence)),
                _ => None,
            })
            .filter(|(_, at)| !ledger.rewritten_after(name, *at));
        let grounding = observed.grounding.get(name).copied().unwrap_or_default();
        let drifted = grounding.drifted()
            || version.derived_from.iter().any(|upstream| {
                ledger.current(upstream.artifact.as_str()) != upstream.digest.as_ref()
            });
        let recorded = !version.derived_from.is_empty()
            || version.grounding != crate::evidence::Grounding::default();
        let class = match judged {
            Some((DriftClass::Substantive, _)) => Some(member::SUBSTANTIVE),
            Some((DriftClass::Cosmetic, at)) if drifted => {
                let upstream_changed = upstream_of(name)
                    .iter()
                    .any(|upstream| ledger.rewritten_after(upstream, at));
                let assessed_changed = observed
                    .judged_grounding
                    .get(name)
                    .is_some_and(|change| change.changed());
                Some(if upstream_changed || assessed_changed {
                    member::PENDING
                } else {
                    member::COSMETIC
                })
            }
            Some((DriftClass::Cosmetic, _)) => Some(member::COSMETIC),
            None if drifted => Some(member::PENDING),
            None if !recorded => None,
            None => Some(member::FRESH),
        };
        if let Some(class) = class {
            facts.insert(fact, text(class))?;
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
            BuildVerdict::Approve => member::APPROVE,
            BuildVerdict::ChangesRequested => member::CHANGES_REQUESTED,
        };
        facts.insert("build_review.verdict", text(verdict))?;
        facts.insert("build_review.target_digest", FactValue::Sha(target.clone()))?;
        facts.insert(
            "build_review.binding",
            text(binding(ledger, report, observed.reviewed_change)),
        )?;
        if let Some(base) = baseline_commit(ledger) {
            facts.insert("build_review.baseline_commit", FactValue::Sha(base.clone()))?;
        }
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
            facts.insert("finalize.status", text(member::READY))?;
            facts.insert("finalize.sha", FactValue::Sha(commit.clone()))?;
        }
        None => facts.insert("finalize.status", text(member::PENDING))?,
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
        .any(|name| ledger.rewritten_after(name, stamp.sequence));
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
    match ledger.current(artifact_names::REVIEW_FINDINGS) {
        None => member::UNBOUND,
        Some(current) if current != report => member::REPORT_CHANGED,
        Some(_) => match change {
            Some(ChangeSinceReview::Unchanged) => member::EXACT_TARGET,
            Some(ChangeSinceReview::CleanUpstreamMerges) => member::CLEAN_UPSTREAM_MERGES,
            Some(ChangeSinceReview::TodoChanged) => member::TODO_CHANGED,
            Some(ChangeSinceReview::Undetermined) | None => member::UNDETERMINED,
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

/// What recorded dependency grounding shows against `others`, the
/// related todos' current evidence: whether a prerequisite artifact's
/// digest changed, and whether a dependency is unresolved. Dependencies
/// are matched by durable identity, never by slug, which a later todo may
/// reuse; a record without an identity, or an identity no todo has, is
/// unresolved: incomplete provenance.
pub(crate) fn dependency_change(
    recorded: &[crate::evidence::DependencyDigest],
    others: &[TodoEvidence],
) -> (bool, bool) {
    let mut changed = false;
    let mut unresolved = false;
    for dependency in recorded {
        let Some(todo) = dependency
            .id
            .as_ref()
            .and_then(|id| others.iter().find(|other| &other.id == id))
        else {
            unresolved = true;
            continue;
        };
        let current = todo
            .artifacts
            .iter()
            .find(|artifact| artifact.name == dependency.artifact)
            .and_then(|artifact| artifact.versions.last())
            .map(|version| &version.digest);
        changed |= current != dependency.digest.as_ref();
    }
    (changed, unresolved)
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

    use super::super::_facts::{FactName, member};
    use super::super::_testing::{EvidenceBuilder, digest, session, sha, todo};
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

    /// An enum-typed fact's member, as the provider supplies it.
    fn value(member: &str) -> FactValue {
        FactValue::Text(member.to_owned())
    }

    /// The fact value of the evidence digest built from `fixture`.
    fn digest_value(fixture: &str) -> FactValue {
        FactValue::Text(digest(fixture).as_str().to_owned())
    }

    /// The fact value of a retained worktree path.
    fn path_value(path: &str) -> FactValue {
        FactValue::Text(std::path::Path::new(path).display().to_string())
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
            grounding: Default::default(),
        }
    }

    fn kickback() -> StampFact {
        StampFact::Kickback {
            reason: "the plan misses the API".into(),
            input: digest("input-2"),
            invalidated_plan: Some(digest("plan-1")),
            invalidated_reviews: crate::evidence::plan_review_only(),
        }
    }

    fn approval(gate: ApprovalGate, by: ApprovedBy, target: &str) -> StampFact {
        StampFact::Approval {
            gate,
            by,
            target: digest(target),
            carried_from: None,
            reasoning: None,
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
        assert_eq!(get(&facts, "prd.digest"), Some(&digest_value("prd-2")));
        assert_eq!(get(&facts, "kind"), Some(&value(member::FEATURE)));
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
        assert_eq!(
            get(&facts, "finalize.status"),
            Some(&value(member::PENDING))
        );
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
        assert_eq!(
            get(&facts, "prd_review.verdict"),
            Some(&value(member::APPROVE))
        );
        assert_eq!(
            get(&facts, "prd_review.target_digest"),
            Some(&digest_value("prd-2"))
        );
        assert_eq!(
            get(&facts, "prd_review.report_digest"),
            Some(&digest_value("findings-1"))
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
        assert_eq!(get(&facts, "drift.prd"), Some(&value(member::SUBSTANTIVE)));
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
                grounding: Default::default(),
            })
            .build();
        let facts = facts_of(&evidence, &Observations::default());
        assert_eq!(
            get(&facts, "drift.implementation_plan"),
            Some(&value(member::COSMETIC))
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
        assert_eq!(
            get(&facts, "finalize.status"),
            Some(&value(member::PENDING))
        );
    }

    #[test]
    fn a_peer_approval_does_not_satisfy_a_human_gate() {
        let evidence = todo(TodoKind::Feature)
            .artifact(artifact_names::PRD, "prd-1")
            .stamp(approval(ApprovalGate::Prd, ApprovedBy::Peer, "prd-1"))
            .build();
        let facts = facts_of(&evidence, &Observations::default());
        assert_eq!(get(&facts, "prd_approval.digest"), None);
    }

    #[test]
    fn a_peer_gate_admits_the_peer_approval() {
        let evidence = todo(TodoKind::Task)
            .artifact(artifact_names::TASK_PLAN, "task-1")
            .stamp(approval(ApprovalGate::Task, ApprovedBy::Peer, "task-1"))
            .build();
        let gates = Approvers {
            prd: Approver::Human,
            task: Approver::Peer,
        };
        let facts = facts_with(&evidence, &[], gates, &Observations::default());
        assert_eq!(
            get(&facts, "task_approved.target_digest"),
            Some(&digest_value("task-1"))
        );
        assert_eq!(get(&facts, "task_approved.by"), Some(&value(member::PEER)));
    }

    #[test]
    fn a_human_approval_is_attributed_to_a_human() {
        let evidence = todo(TodoKind::Feature)
            .artifact(artifact_names::PRD, "prd-1")
            .stamp(approval(ApprovalGate::Prd, ApprovedBy::Human, "prd-1"))
            .build();
        let facts = facts_of(&evidence, &Observations::default());
        assert_eq!(
            get(&facts, "prd_approval.digest"),
            Some(&digest_value("prd-1"))
        );
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
        assert_eq!(
            get(&facts, "build_review.verdict"),
            Some(&value(member::APPROVE))
        );
        assert_eq!(
            get(&facts, "build_review.target_digest"),
            Some(&FactValue::Sha(sha('a')))
        );
        assert_eq!(
            get(&facts, "build_review.binding"),
            Some(&value(member::EXACT_TARGET))
        );
    }

    #[test]
    fn only_clean_upstream_merges_keep_a_verdict_current() {
        let observed = observed_change('c', ChangeSinceReview::CleanUpstreamMerges);
        let facts = facts_of(&reviewed().build(), &observed);
        assert_eq!(
            get(&facts, "build_review.binding"),
            Some(&value(member::CLEAN_UPSTREAM_MERGES))
        );
    }

    #[test]
    fn changed_todo_work_makes_the_verdict_stale() {
        let observed = observed_change('c', ChangeSinceReview::TodoChanged);
        let facts = facts_of(&reviewed().build(), &observed);
        assert_eq!(
            get(&facts, "build_review.binding"),
            Some(&value(member::TODO_CHANGED))
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
            Some(&value(member::REPORT_CHANGED))
        );
    }

    #[test]
    fn a_verdict_without_its_findings_is_unbound() {
        let evidence = built()
            .stamp(build_verdict(BuildVerdict::Approve, 'a', "report-1"))
            .build();
        let facts = facts_of(&evidence, &at_head('a'));
        assert_eq!(
            get(&facts, "build_review.binding"),
            Some(&value(member::UNBOUND))
        );
    }

    #[test]
    fn an_unanswerable_repository_leaves_the_binding_undetermined() {
        let observed = observed_change('c', ChangeSinceReview::Undetermined);
        let facts = facts_of(&reviewed().build(), &observed);
        assert_eq!(
            get(&facts, "build_review.binding"),
            Some(&value(member::UNDETERMINED))
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
        assert_eq!(get(&facts, "finalize.status"), Some(&value(member::READY)));
        assert_eq!(get(&facts, "finalize.sha"), Some(&FactValue::Sha(sha('a'))));
    }

    #[test]
    fn a_later_completion_resets_finalize_readiness() {
        let evidence = built()
            .stamp(StampFact::FinalizeReady { commit: sha('a') })
            .stamp(completed('b'))
            .build();
        let facts = facts_of(&evidence, &at_head('b'));
        assert_eq!(
            get(&facts, "finalize.status"),
            Some(&value(member::PENDING))
        );
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
        assert_eq!(
            get(&facts, "worktree.path"),
            Some(&path_value("/work/add-login"))
        );
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
            grounding: Default::default(),
            judged_grounding: Default::default(),
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

    // ---- kickback, drift provenance and seeded plans ----

    #[test]
    fn a_kickback_clears_exactly_the_reviews_it_records() {
        let evidence = todo(TodoKind::Feature)
            .artifact(artifact_names::PRD, "prd-1")
            .stamp(verdict(
                PreparationReview::PrdReview,
                ReviewVerdict::Approve,
                "prd-1",
            ))
            .stamp(StampFact::Kickback {
                reason: "the requirements miss offline use".into(),
                input: digest("input-2"),
                invalidated_plan: None,
                invalidated_reviews: vec![PreparationReview::PrdReview],
            })
            .build();
        let facts = facts_of(&evidence, &Observations::default());
        assert_eq!(get(&facts, "prd_review.verdict"), None);
        assert_eq!(
            get(&facts, "prd_review.cleared_by_substantive_drift").cloned(),
            yes()
        );
        assert_eq!(
            get(&facts, "plan_review.cleared_by_substantive_drift").cloned(),
            no()
        );
    }

    /// A PRD derived from input `input-1`.
    fn derived_prd() -> EvidenceBuilder {
        todo(TodoKind::Feature)
            .artifact(artifact_names::INPUT, "input-1")
            .derived(
                artifact_names::PRD,
                "prd-1",
                &[(artifact_names::INPUT, Some("input-1"))],
            )
    }

    fn cosmetic(artifact: &str) -> StampFact {
        StampFact::DriftReviewed {
            artifact: ArtifactName::new(artifact),
            class: DriftClass::Cosmetic,
            reason: "wording".into(),
            examined: digest("evidence"),
            grounding: Default::default(),
        }
    }

    #[test]
    fn provenance_matching_its_upstream_is_fresh() {
        let facts = facts_of(&derived_prd().build(), &Observations::default());
        assert_eq!(get(&facts, "drift.prd"), Some(&value(member::FRESH)));
    }

    #[test]
    fn a_changed_upstream_makes_drift_pending() {
        let evidence = derived_prd()
            .artifact(artifact_names::INPUT, "input-2")
            .build();
        let facts = facts_of(&evidence, &Observations::default());
        assert_eq!(get(&facts, "drift.prd"), Some(&value(member::PENDING)));
    }

    #[test]
    fn an_upstream_that_appeared_after_the_version_is_drift() {
        let evidence = todo(TodoKind::Feature)
            .derived(
                artifact_names::ARCHITECTURE,
                "arch-1",
                &[(artifact_names::INPUT, None), (artifact_names::PRD, None)],
            )
            .artifact(artifact_names::PRD, "prd-1")
            .build();
        let facts = facts_of(&evidence, &Observations::default());
        assert_eq!(
            get(&facts, "drift.architecture"),
            Some(&value(member::PENDING))
        );
    }

    #[test]
    fn a_cosmetic_judgment_settles_the_evidence_it_saw() {
        let evidence = derived_prd()
            .artifact(artifact_names::INPUT, "input-2")
            .stamp(cosmetic(artifact_names::PRD))
            .build();
        let facts = facts_of(&evidence, &Observations::default());
        assert_eq!(get(&facts, "drift.prd"), Some(&value(member::COSMETIC)));
    }

    #[test]
    fn a_newer_upstream_after_a_cosmetic_judgment_needs_a_new_one() {
        let evidence = derived_prd()
            .artifact(artifact_names::INPUT, "input-2")
            .stamp(cosmetic(artifact_names::PRD))
            .artifact(artifact_names::INPUT, "input-3")
            .build();
        let facts = facts_of(&evidence, &Observations::default());
        assert_eq!(get(&facts, "drift.prd"), Some(&value(member::PENDING)));
    }

    #[test]
    fn a_substantive_judgment_outranks_pending_evidence() {
        let evidence = derived_prd()
            .artifact(artifact_names::INPUT, "input-2")
            .stamp(substantive(artifact_names::PRD))
            .build();
        let facts = facts_of(&evidence, &Observations::default());
        assert_eq!(get(&facts, "drift.prd"), Some(&value(member::SUBSTANTIVE)));
    }

    #[test]
    fn a_version_without_recorded_provenance_supplies_no_drift() {
        let evidence = todo(TodoKind::Feature)
            .artifact(artifact_names::PRD, "prd-1")
            .build();
        let facts = facts_of(&evidence, &Observations::default());
        assert_eq!(get(&facts, "drift.prd"), None);
    }

    #[test]
    fn a_plan_filed_with_the_todo_is_seeded() {
        let evidence = todo(TodoKind::Change)
            .seeded()
            .artifact(artifact_names::IMPLEMENTATION_PLAN, "plan-0")
            .build();
        let facts = facts_of(&evidence, &Observations::default());
        assert_eq!(get(&facts, "implementation_plan.seeded").cloned(), yes());
    }

    #[test]
    fn a_seeded_plan_redrafted_by_planning_is_no_longer_seeded() {
        let evidence = todo(TodoKind::Change)
            .seeded()
            .artifact(artifact_names::IMPLEMENTATION_PLAN, "plan-0")
            .artifact(artifact_names::IMPLEMENTATION_PLAN, "plan-1")
            .build();
        let facts = facts_of(&evidence, &Observations::default());
        assert_eq!(get(&facts, "implementation_plan.seeded").cloned(), no());
    }

    // ---- approval currency ----

    /// A feature whose PRD `prd-1` was reviewed and approved by a human.
    fn approved_prd() -> EvidenceBuilder {
        todo(TodoKind::Feature)
            .artifact(artifact_names::PRD, "prd-1")
            .stamp(verdict(
                PreparationReview::PrdReview,
                ReviewVerdict::Approve,
                "prd-1",
            ))
            .stamp(approval(ApprovalGate::Prd, ApprovedBy::Human, "prd-1"))
    }

    #[test]
    fn re_deriving_the_same_bytes_after_substantive_drift_needs_approval_again() {
        let evidence = approved_prd()
            .stamp(substantive(artifact_names::PRD))
            .artifact(artifact_names::PRD, "prd-1")
            .stamp(verdict(
                PreparationReview::PrdReview,
                ReviewVerdict::Approve,
                "prd-1",
            ))
            .build();
        let facts = facts_of(&evidence, &Observations::default());
        assert_eq!(
            get(&facts, "prd_review.verdict"),
            Some(&value(member::APPROVE))
        );
        assert_eq!(get(&facts, "prd_approval.digest"), None);
    }

    #[test]
    fn an_approval_does_not_reach_a_later_version_with_an_equal_digest() {
        let evidence = approved_prd()
            .artifact(artifact_names::PRD, "prd-2")
            .artifact(artifact_names::PRD, "prd-1")
            .build();
        let facts = facts_of(&evidence, &Observations::default());
        assert_eq!(get(&facts, "prd_approval.digest"), None);
    }

    #[test]
    fn a_carry_after_re_review_approves_the_re_derived_version() {
        let evidence = approved_prd()
            .stamp(substantive(artifact_names::PRD))
            .artifact(artifact_names::PRD, "prd-1")
            .stamp(verdict(
                PreparationReview::PrdReview,
                ReviewVerdict::Approve,
                "prd-1",
            ))
            .stamp(StampFact::Approval {
                gate: ApprovalGate::Prd,
                by: ApprovedBy::Human,
                target: digest("prd-1"),
                carried_from: Some(digest("prd-1")),
                reasoning: None,
            })
            .build();
        let facts = facts_of(&evidence, &Observations::default());
        assert_eq!(
            get(&facts, "prd_approval.digest"),
            Some(&digest_value("prd-1"))
        );
        assert_eq!(get(&facts, "prd_approval.by_human").cloned(), yes());
    }

    #[test]
    fn a_kickback_of_the_prd_review_revokes_its_approval() {
        let evidence = approved_prd()
            .stamp(StampFact::Kickback {
                reason: "offline use is missing".into(),
                input: digest("input-2"),
                invalidated_plan: None,
                invalidated_reviews: vec![PreparationReview::PrdReview],
            })
            .build();
        let facts = facts_of(&evidence, &Observations::default());
        assert_eq!(get(&facts, "prd_approval.digest"), None);
    }

    #[test]
    fn the_baseline_is_the_build_start_current_when_the_verdict_was_stamped() {
        let evidence = todo(TodoKind::Change)
            .stamp(StampFact::BuildStarted {
                base: sha('1'),
                plan: digest("plan-1"),
            })
            .stamp(completed('a'))
            .artifact(BUILD_REVIEW_FINDINGS, "report-1")
            .stamp(build_verdict(BuildVerdict::Approve, 'a', "report-1"))
            .stamp(StampFact::BuildStarted {
                base: sha('2'),
                plan: digest("plan-2"),
            })
            .build();
        let facts = facts_of(&evidence, &at_head('a'));
        assert_eq!(
            get(&facts, "build_review.baseline_commit"),
            Some(&FactValue::Sha(sha('1')))
        );
    }

    // ---- record order and grounding ----

    #[test]
    fn on_a_frozen_clock_a_rewrite_still_settles_substantive_drift() {
        let evidence = todo(TodoKind::Feature)
            .frozen()
            .artifact(artifact_names::PRD, "prd-1")
            .stamp(substantive(artifact_names::PRD))
            .artifact(artifact_names::PRD, "prd-2")
            .build();
        let facts = facts_of(&evidence, &Observations::default());
        assert_eq!(get(&facts, "drift.prd"), None);
        assert_eq!(
            get(&facts, "prd_review.cleared_by_substantive_drift").cloned(),
            no()
        );
    }

    #[test]
    fn on_a_frozen_clock_a_newer_upstream_unsettles_a_cosmetic_judgment() {
        let evidence = derived_prd()
            .frozen()
            .artifact(artifact_names::INPUT, "input-2")
            .stamp(cosmetic(artifact_names::PRD))
            .artifact(artifact_names::INPUT, "input-3")
            .build();
        let facts = facts_of(&evidence, &Observations::default());
        assert_eq!(get(&facts, "drift.prd"), Some(&value(member::PENDING)));
    }

    #[test]
    fn on_a_frozen_clock_an_approval_binds_the_version_before_it() {
        let evidence = approved_prd().frozen().build();
        let facts = facts_of(&evidence, &Observations::default());
        assert_eq!(
            get(&facts, "prd_approval.digest"),
            Some(&digest_value("prd-1"))
        );
        let rewritten = approved_prd()
            .frozen()
            .artifact(artifact_names::PRD, "prd-1")
            .build();
        let facts = facts_of(&rewritten, &Observations::default());
        assert_eq!(get(&facts, "prd_approval.digest"), None);
    }

    /// A plan grounded on the canonical head `a`, referencing `src/`.
    fn grounded_plan() -> EvidenceBuilder {
        todo(TodoKind::Change).grounded(
            artifact_names::IMPLEMENTATION_PLAN,
            "plan-1",
            crate::evidence::Grounding {
                head: Some(sha('a')),
                referenced_paths: vec!["src/".to_owned()],
                ..Default::default()
            },
        )
    }

    fn plan_grounding(change: GroundingChange) -> Observations {
        Observations {
            grounding: [(artifact_names::IMPLEMENTATION_PLAN.to_owned(), change)]
                .into_iter()
                .collect(),
            ..Observations::default()
        }
    }

    #[test]
    fn grounding_with_unchanged_evidence_is_fresh() {
        let facts = facts_of(&grounded_plan().build(), &Observations::default());
        assert_eq!(
            get(&facts, "drift.implementation_plan"),
            Some(&value(member::FRESH))
        );
    }

    #[test]
    fn a_commit_changing_a_referenced_path_is_pending_drift() {
        let observed = plan_grounding(GroundingChange {
            code: CodeChange::Changed,
            ..GroundingChange::default()
        });
        let facts = facts_of(&grounded_plan().build(), &observed);
        assert_eq!(
            get(&facts, "drift.implementation_plan"),
            Some(&value(member::PENDING))
        );
    }

    #[test]
    fn code_that_cannot_be_compared_is_pending() {
        let observed = plan_grounding(GroundingChange {
            code: CodeChange::Uncomparable,
            ..GroundingChange::default()
        });
        let facts = facts_of(&grounded_plan().build(), &observed);
        assert_eq!(
            get(&facts, "drift.implementation_plan"),
            Some(&value(member::PENDING))
        );
    }

    #[test]
    fn a_changed_snippet_is_pending_drift() {
        let observed = plan_grounding(GroundingChange {
            snippets: true,
            ..GroundingChange::default()
        });
        let facts = facts_of(&grounded_plan().build(), &observed);
        assert_eq!(
            get(&facts, "drift.implementation_plan"),
            Some(&value(member::PENDING))
        );
    }

    #[test]
    fn a_changed_prerequisite_artifact_is_pending_drift() {
        let observed = plan_grounding(GroundingChange {
            dependencies: true,
            ..GroundingChange::default()
        });
        let facts = facts_of(&grounded_plan().build(), &observed);
        assert_eq!(
            get(&facts, "drift.implementation_plan"),
            Some(&value(member::PENDING))
        );
    }

    #[test]
    fn a_cosmetic_judgment_settles_changed_code() {
        let observed = plan_grounding(GroundingChange {
            code: CodeChange::Changed,
            ..GroundingChange::default()
        });
        let evidence = grounded_plan()
            .stamp(cosmetic(artifact_names::IMPLEMENTATION_PLAN))
            .build();
        let facts = facts_of(&evidence, &observed);
        assert_eq!(
            get(&facts, "drift.implementation_plan"),
            Some(&value(member::COSMETIC))
        );
    }

    // ---- the evidence a cosmetic judgment assessed ----

    /// A grounded plan whose code changed since it was written, judged
    /// cosmetic; `since_judgment` is what the judgment's recorded grounding
    /// shows now.
    fn judged_plan(since_judgment: GroundingChange) -> (TodoEvidence, Observations) {
        let evidence = grounded_plan()
            .stamp(cosmetic(artifact_names::IMPLEMENTATION_PLAN))
            .build();
        let mut observed = plan_grounding(GroundingChange {
            code: CodeChange::Changed,
            ..GroundingChange::default()
        });
        observed.judged_grounding = [(
            artifact_names::IMPLEMENTATION_PLAN.to_owned(),
            since_judgment,
        )]
        .into_iter()
        .collect();
        (evidence, observed)
    }

    #[test]
    fn a_cosmetic_judgment_holds_while_its_assessed_evidence_matches() {
        let (evidence, observed) = judged_plan(GroundingChange::default());
        let facts = facts_of(&evidence, &observed);
        assert_eq!(
            get(&facts, "drift.implementation_plan"),
            Some(&value(member::COSMETIC))
        );
    }

    #[test]
    fn a_code_change_after_a_cosmetic_judgment_is_pending_again() {
        let (evidence, observed) = judged_plan(GroundingChange {
            code: CodeChange::Changed,
            ..GroundingChange::default()
        });
        let facts = facts_of(&evidence, &observed);
        assert_eq!(
            get(&facts, "drift.implementation_plan"),
            Some(&value(member::PENDING))
        );
    }

    #[test]
    fn a_snippet_change_after_a_cosmetic_judgment_is_pending_again() {
        let (evidence, observed) = judged_plan(GroundingChange {
            snippets: true,
            ..GroundingChange::default()
        });
        let facts = facts_of(&evidence, &observed);
        assert_eq!(
            get(&facts, "drift.implementation_plan"),
            Some(&value(member::PENDING))
        );
    }

    // ---- incomplete provenance ----

    #[test]
    fn an_unresolved_reference_is_never_fresh() {
        let observed = plan_grounding(GroundingChange {
            unresolved: true,
            ..GroundingChange::default()
        });
        let facts = facts_of(&grounded_plan().build(), &observed);
        assert_eq!(
            get(&facts, "drift.implementation_plan"),
            Some(&value(member::PENDING))
        );
    }

    #[test]
    fn a_cosmetic_judgment_settles_a_reference_that_stays_unresolved() {
        let (evidence, mut observed) = judged_plan(GroundingChange::default());
        observed.grounding.insert(
            artifact_names::IMPLEMENTATION_PLAN.to_owned(),
            GroundingChange {
                unresolved: true,
                ..GroundingChange::default()
            },
        );
        let facts = facts_of(&evidence, &observed);
        assert_eq!(
            get(&facts, "drift.implementation_plan"),
            Some(&value(member::COSMETIC))
        );
    }

    // ---- dependency grounding by durable identity ----

    fn identity(fill: char) -> crate::work::TodoId {
        crate::work::TodoId::parse(&fill.to_string().repeat(32)).unwrap()
    }

    /// Prerequisite `parse-lists` with identity `id` and PRD `prd`.
    fn prerequisite(id: char, prd: &str) -> TodoEvidence {
        let mut other = todo(TodoKind::Feature)
            .artifact(artifact_names::PRD, prd)
            .build();
        other.id = identity(id);
        other.slug = instrukt_core::ids::TodoSlug::parse("parse-lists").unwrap();
        other
    }

    /// Grounding on `parse-lists`' PRD `prd-1`, recorded with `id`.
    fn on_prerequisite(id: Option<char>) -> Vec<crate::evidence::DependencyDigest> {
        vec![crate::evidence::DependencyDigest {
            id: id.map(identity),
            todo: instrukt_core::ids::TodoSlug::parse("parse-lists").unwrap(),
            artifact: ArtifactName::new(artifact_names::PRD),
            digest: Some(digest("prd-1")),
        }]
    }

    #[test]
    fn an_unchanged_prerequisite_artifact_is_neither_changed_nor_unresolved() {
        let others = [prerequisite('1', "prd-1")];
        assert_eq!(
            dependency_change(&on_prerequisite(Some('1')), &others),
            (false, false)
        );
    }

    #[test]
    fn a_rewritten_prerequisite_artifact_is_a_change() {
        let others = [prerequisite('1', "prd-2")];
        assert_eq!(
            dependency_change(&on_prerequisite(Some('1')), &others),
            (true, false)
        );
    }

    #[test]
    fn a_reused_slug_is_not_the_recorded_prerequisite() {
        // `parse-lists` now names another todo, whose PRD matches by chance.
        let others = [prerequisite('2', "prd-1")];
        assert_eq!(
            dependency_change(&on_prerequisite(Some('1')), &others),
            (false, true)
        );
    }

    #[test]
    fn a_dependency_recorded_without_an_identity_is_unresolved() {
        let others = [prerequisite('1', "prd-1")];
        assert_eq!(
            dependency_change(&on_prerequisite(None), &others),
            (false, true)
        );
    }

    /// Dependency grounding as records stored it before dependencies
    /// carried the prerequisite's identity: an artifact version's
    /// `grounding`, and a drift judgment's stamp fact.
    const PRE_IDENTITY_VERSION_GROUNDING: &str = r#"{
        "head": "0123456789abcdef0123456789abcdef01234567",
        "dependencies": [
            {"todo": "parse-lists", "artifact": "prd", "digest": "prd-1"}
        ]
    }"#;
    const PRE_IDENTITY_DRIFT_STAMP: &str = r#"{
        "fact": "drift_reviewed",
        "artifact": "implementation_plan",
        "class": "cosmetic",
        "reason": "wording only",
        "examined": "plan-1",
        "grounding": {
            "dependencies": [
                {"todo": "parse-lists", "artifact": "prd", "digest": "prd-1"}
            ]
        }
    }"#;

    #[test]
    fn a_dependency_stored_before_identities_reads_as_unresolved() {
        let others = [prerequisite('1', "prd-1")];
        let version: crate::evidence::Grounding =
            serde_json::from_str(PRE_IDENTITY_VERSION_GROUNDING).unwrap();
        assert_eq!(
            dependency_change(&version.dependencies, &others),
            (false, true)
        );
        let stamp: crate::evidence::StampFact =
            serde_json::from_str(PRE_IDENTITY_DRIFT_STAMP).unwrap();
        let crate::evidence::StampFact::DriftReviewed { grounding, .. } = stamp else {
            panic!("the fixture is a drift judgment");
        };
        assert_eq!(
            dependency_change(&grounding.dependencies, &others),
            (false, true)
        );
    }
}
