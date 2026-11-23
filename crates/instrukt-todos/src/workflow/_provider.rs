//! Pure factual ledger over a todo's recorded evidence and observed
//! repository state (`docs/project/spec/domain/workflow.md`).
//!
//! No transition table, no state-machine routing: this module derives only
//! current facts (workspace/git state, active attempt lock, artifact currency
//! and drift, review findings currency, human approval gate status, and
//! integration review readiness).

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use instrukt_core::config::Approver;
use instrukt_core::ids::{CommitSha, ProjectSlug, RoleName, SessionRef, TodoSlug};
use instrukt_runner::interfaces::{AssignmentKind, Lane, TodoKind};
use serde::Serialize;

use crate::evidence::{
    Actor, ApprovalGate, ApprovedBy, ArtifactVersion, BuildVerdict, ContentDigest,
    DependencyDigest, DriftClass, PreparationReview, Prerequisite, ReviewVerdict, RoleBinding,
    StampFact, TodoEvidence, TodoStatus, artifact_names, upstream_of,
};

/// Each artifact with a drift classification.
pub(crate) const DRIFT_ARTIFACTS: [&str; 3] = [
    artifact_names::PRD,
    artifact_names::ARCHITECTURE,
    artifact_names::IMPLEMENTATION_PLAN,
];

/// Writer and authoring/analysis roles whose sessions cannot independently
/// approve a build (`setup` is retained for legacy and test-flavor records).
const WRITER_ROLES: [&str; 11] = [
    "builder",
    "fixer",
    "setup",
    "finalizer",
    "intaker",
    "architect",
    "planner",
    "diagnoser",
    "deferrer",
    "task-preparer",
    "drift-analyzer",
];

/// Who may approve each gate (`approvals.prd`, `approvals.task`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Approvers {
    pub prd: Approver,
    pub task: Approver,
}

/// Where one prerequisite stands, read from its own evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PrerequisiteStanding {
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
    /// The reviewed commit is the exact current head.
    Unchanged,
    /// Only clean merges of the canonical branch separate them.
    CleanUpstreamMerges,
    TodoChanged,
    /// The repository could not tell.
    Undetermined,
}

/// Whether a referenced path changed on the canonical branch since grounding.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum CodeChange {
    #[default]
    Unchanged,
    Changed,
    Uncomparable,
}

/// What a version's recorded grounding shows now (`artifact-currency`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct GroundingChange {
    pub code: CodeChange,
    pub snippets: bool,
    pub dependencies: bool,
    /// Incomplete provenance (such as a missing referenced snippet) requires
    /// a drift judgment. A referenced path missing at the canonical head is
    /// unresolved if it existed at the recorded head or that head is absent
    /// or unreadable. Absence at both heads establishes a planned creation.
    pub unresolved: bool,
}

impl GroundingChange {
    fn changed(self) -> bool {
        self.code != CodeChange::Unchanged || self.snippets || self.dependencies
    }

    fn drifted(self) -> bool {
        self.changed() || self.unresolved
    }
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
    pub worktree: Option<WorktreeObservation>,
    pub worktree_path: Option<PathBuf>,
    pub branch_head: Option<CommitSha>,
    pub remote_head: Option<CommitSha>,
    pub reviewed_change: Option<ChangeSinceReview>,
    pub grounding: BTreeMap<String, GroundingChange>,
    pub judged_grounding: BTreeMap<String, GroundingChange>,
}

/// Drift classification for a tracked artifact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DriftStatus {
    Fresh,
    Pending,
    Cosmetic,
    Substantive,
}

impl DriftStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fresh => "fresh",
            Self::Pending => "pending",
            Self::Cosmetic => "cosmetic",
            Self::Substantive => "substantive",
        }
    }
}

/// Exact-SHA binding status of the latest build review verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BuildReviewBinding {
    ExactTarget,
    TodoChanged,
    ReportChanged,
    Unbound,
    Undetermined,
}

impl BuildReviewBinding {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ExactTarget => "exact_target",
            Self::TodoChanged => "todo_changed",
            Self::ReportChanged => "report_changed",
            Self::Unbound => "unbound",
            Self::Undetermined => "undetermined",
        }
    }
}

/// Status of a configured approval gate (`approvals.prd`, `approvals.task`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ApprovalStatus {
    NotRequired,
    Pending {
        approver: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        target_digest: Option<ContentDigest>,
    },
    Approved {
        by: ApprovedBy,
        target_digest: ContentDigest,
    },
}

/// Status of the independent exact-SHA review gate for integration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum IntegrateGateStatus {
    Ready {
        commit: CommitSha,
        finalize_stamped: bool,
    },
    Blocked {
        reasons: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorkspaceFact {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worktree_path: Option<PathBuf>,
    pub worktree_exists: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worktree_clean: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worktree_head: Option<CommitSha>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch_head: Option<CommitSha>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote_head: Option<CommitSha>,
    pub pushed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AttemptFact {
    pub role: RoleName,
    pub attempt: u64,
    pub assignment: AssignmentKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session: Option<SessionRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_head: Option<CommitSha>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_head: Option<CommitSha>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_reason: Option<String>,
    pub active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ArtifactFact {
    pub name: String,
    pub versions: usize,
    pub current_digest: ContentDigest,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub drift: Option<DriftStatus>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReviewFact {
    pub phase: String,
    pub verdict: ReviewVerdict,
    pub target_digest: ContentDigest,
    pub report_digest: ContentDigest,
    pub current: bool,
    pub open_findings: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BuildReviewFact {
    pub verdict: BuildVerdict,
    pub target_commit: CommitSha,
    pub report_digest: ContentDigest,
    pub reviewer: String,
    pub independent: bool,
    pub binding: BuildReviewBinding,
    pub open_findings: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PrerequisiteFact {
    pub slug: TodoSlug,
    pub standing: PrerequisiteStanding,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GatesFact {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_workspace_attempt: Option<String>,
    pub prd_approval: ApprovalStatus,
    pub task_approval: ApprovalStatus,
    pub integrate_review_gate: IntegrateGateStatus,
}

/// Complete factual ledger for one todo: zero routing decisions, pure
/// observable facts over the record, worktree, and three integrity gates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TodoLedger {
    pub project: ProjectSlug,
    pub slug: TodoSlug,
    pub kind: TodoKind,
    pub status: TodoStatus,
    pub claimed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claim_session: Option<SessionRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hold_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delivery_commit: Option<CommitSha>,
    pub workspace: WorkspaceFact,
    pub active_attempts: Vec<AttemptFact>,
    pub artifacts: Vec<ArtifactFact>,
    pub preparation_reviews: Vec<ReviewFact>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub build_review: Option<BuildReviewFact>,
    pub diagnosis_settled: bool,
    pub deferrals_open: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finalize_ready_commit: Option<CommitSha>,
    pub prerequisites: Vec<PrerequisiteFact>,
    pub prerequisites_satisfied: bool,
    pub gates: GatesFact,
}

impl TodoLedger {
    /// Compact, scannable factual ledger text for orchestrators and humans.
    pub fn render_text(&self) -> String {
        let mut lines = Vec::new();
        lines.push(format!(
            "todo: {} ({}) [{} · {}]",
            self.slug,
            self.project,
            self.kind,
            status_word(self.status)
        ));
        lines.push(match (&self.claimed, &self.claim_session) {
            (true, Some(session)) => format!("claim: held by {session}"),
            (true, None) => "claim: held (runner)".to_owned(),
            (false, _) => "claim: unclaimed".to_owned(),
        });
        if let Some(reason) = &self.hold_reason {
            lines.push(format!("hold: {reason}"));
        }
        if let Some(commit) = &self.delivery_commit {
            lines.push(format!("delivered: {commit}"));
        }

        // Workspace / git state
        lines.push("workspace:".to_owned());
        if let Some(branch) = &self.workspace.branch {
            lines.push(format!("  branch: {branch}"));
        }
        if let Some(path) = &self.workspace.worktree_path {
            let clean = match self.workspace.worktree_clean {
                Some(true) => "clean",
                Some(false) => "dirty",
                None => "absent",
            };
            lines.push(format!("  worktree: {} ({clean})", path.display()));
        } else {
            lines.push("  worktree: none".to_owned());
        }
        if let Some(head) = &self.workspace.worktree_head {
            lines.push(format!("  worktree head: {head}"));
        }
        if let Some(head) = &self.workspace.branch_head {
            lines.push(format!("  branch head: {head}"));
        }
        if let Some(head) = &self.workspace.remote_head {
            lines.push(format!(
                "  remote head: {head} ({})",
                if self.workspace.pushed {
                    "in sync"
                } else {
                    "behind local"
                }
            ));
        }

        // Active attempts (Invariant 1)
        if !self.active_attempts.is_empty() {
            lines.push("active attempts:".to_owned());
            for a in &self.active_attempts {
                let who = a
                    .session
                    .as_ref()
                    .map(ToString::to_string)
                    .unwrap_or_else(|| "pending bind".to_owned());
                lines.push(format!(
                    "  {}/{} ({:?}): {who}",
                    a.role, a.attempt, a.assignment
                ));
            }
        }

        // Artifacts & drift
        lines.push("artifacts:".to_owned());
        if self.artifacts.is_empty() {
            lines.push("  none".to_owned());
        } else {
            for a in &self.artifacts {
                let drift = a
                    .drift
                    .map(|d| format!(" [drift: {}]", d.as_str()))
                    .unwrap_or_default();
                lines.push(format!(
                    "  {} (v{}): {}{drift}",
                    a.name, a.versions, a.current_digest
                ));
            }
        }

        // Reviews & open findings
        lines.push("reviews:".to_owned());
        if self.preparation_reviews.is_empty() && self.build_review.is_none() {
            lines.push("  none".to_owned());
        } else {
            for r in &self.preparation_reviews {
                let state = if r.current { "current" } else { "stale" };
                let findings = if r.open_findings {
                    " [open findings]"
                } else {
                    ""
                };
                lines.push(format!(
                    "  {}: {} ({state}; target {}, report {}){findings}",
                    r.phase,
                    review_verdict_word(r.verdict),
                    r.target_digest,
                    r.report_digest
                ));
            }
            if let Some(b) = &self.build_review {
                let ind = if b.independent {
                    "independent"
                } else {
                    "NOT independent"
                };
                let findings = if b.open_findings {
                    " [open findings]"
                } else {
                    ""
                };
                lines.push(format!(
                    "  build_review: {} ({}, {ind}, commit {}, report {}, by {}){findings}",
                    build_verdict_word(b.verdict),
                    b.binding.as_str(),
                    b.target_commit,
                    b.report_digest,
                    b.reviewer
                ));
            }
        }
        if self.diagnosis_settled {
            lines.push("diagnosis: settled".to_owned());
        }
        if self.deferrals_open {
            lines.push("deferrals: unprocessed".to_owned());
        }
        if let Some(commit) = &self.finalize_ready_commit {
            lines.push(format!("finalize_ready: {commit}"));
        }

        // Prerequisites
        if !self.prerequisites.is_empty() {
            let items: Vec<String> = self
                .prerequisites
                .iter()
                .map(|p| format!("{} ({})", p.slug, standing_word(p.standing)))
                .collect();
            lines.push(format!(
                "prerequisites ({}): {}",
                if self.prerequisites_satisfied {
                    "satisfied"
                } else {
                    "blocked"
                },
                items.join(", ")
            ));
        }

        // Three hard integrity gates
        lines.push("gates:".to_owned());
        lines.push(match &self.gates.active_workspace_attempt {
            Some(holder) => format!("  interference lock: locked by {holder}"),
            None => "  interference lock: free".to_owned(),
        });
        lines.push(format!(
            "  prd approval: {}",
            render_approval(&self.gates.prd_approval)
        ));
        if self.kind == TodoKind::Task {
            lines.push(format!(
                "  task approval: {}",
                render_approval(&self.gates.task_approval)
            ));
        }
        match &self.gates.integrate_review_gate {
            IntegrateGateStatus::Ready {
                commit,
                finalize_stamped,
            } => {
                let handoff = if *finalize_stamped {
                    "finalized for integrate"
                } else {
                    "ready to stamp --finalize-ready"
                };
                lines.push(format!("  review gate: ready ({commit}; {handoff})"));
            }
            IntegrateGateStatus::Blocked { reasons } => {
                lines.push(format!("  review gate: blocked ({})", reasons.join("; ")));
            }
        }

        lines.join("\n") + "\n"
    }
}

fn render_approval(status: &ApprovalStatus) -> String {
    match status {
        ApprovalStatus::NotRequired => "not required".to_owned(),
        ApprovalStatus::Pending {
            approver,
            target_digest: Some(digest),
        } => format!("pending {approver} approval (target {digest})"),
        ApprovalStatus::Pending {
            approver,
            target_digest: None,
        } => format!("pending {approver} approval (no artifact yet)"),
        ApprovalStatus::Approved { by, target_digest } => {
            let who = match by {
                ApprovedBy::Human => "human",
                ApprovedBy::Peer => "peer",
            };
            format!("approved by {who} ({target_digest})")
        }
    }
}

fn status_word(status: TodoStatus) -> &'static str {
    match status {
        TodoStatus::Active => "active",
        TodoStatus::Iceboxed => "iceboxed",
        TodoStatus::StructuralHolder => "structural_holder",
        TodoStatus::Delivered => "delivered",
        TodoStatus::Removed => "removed",
    }
}

fn standing_word(standing: PrerequisiteStanding) -> &'static str {
    match standing {
        PrerequisiteStanding::Delivered => "delivered",
        PrerequisiteStanding::Undelivered => "undelivered",
        PrerequisiteStanding::EmptyHolder => "empty_holder",
        PrerequisiteStanding::NotFound => "not_found",
    }
}

fn review_verdict_word(v: ReviewVerdict) -> &'static str {
    match v {
        ReviewVerdict::Approve => "approve",
        ReviewVerdict::NeedsWork => "needs_work",
    }
}

fn build_verdict_word(v: BuildVerdict) -> &'static str {
    match v {
        BuildVerdict::Approve => "approve",
        BuildVerdict::ChangesRequested => "changes_requested",
    }
}

/// Select the todo's active or recorded workspace binding.
pub(crate) fn selected_workspace(evidence: &TodoEvidence) -> Option<&RoleBinding> {
    if evidence.status == TodoStatus::Delivered {
        return None;
    }
    if let Some(active) = evidence.bindings.iter().rev().find(|b| {
        matches!(b.attempt.lane, Some(Lane::Work | Lane::Prepare))
            && b.attempt.branch.is_some()
            && b.attempt.released_at.is_none()
    }) {
        return Some(active);
    }
    if let Some(finalizer) = evidence
        .bindings
        .iter()
        .rev()
        .find(|b| b.role.as_str() == "finalizer" && b.attempt.completion_sequence.is_some())
    {
        return Some(finalizer);
    }
    evidence.bindings.iter().rev().find(|b| {
        b.attempt.assignment == AssignmentKind::Driver
            && matches!(b.attempt.lane, Some(Lane::Work | Lane::Prepare))
            && b.worktree().is_some()
    })
}

/// Returns whether `actor` is an independent reviewer for a build on this
/// todo: a session distinct from `caller`, distinct from any driver/claimant
/// orchestrator session, and distinct from any writer role session on the todo.
pub fn is_independent_reviewer(
    claim_session: Option<&SessionRef>,
    bindings: &[(RoleName, AssignmentKind, Option<&SessionRef>)],
    reviewer: &Actor,
    caller: Option<&SessionRef>,
) -> bool {
    let Actor::Session(reviewer_session) = reviewer else {
        return false;
    };
    if caller == Some(reviewer_session) {
        return false;
    }
    if claim_session == Some(reviewer_session) {
        return false;
    }
    !bindings.iter().any(|(role, assignment, session)| {
        *session == Some(reviewer_session)
            && (*assignment == AssignmentKind::Driver || WRITER_ROLES.contains(&role.as_str()))
    })
}

/// Pure ledger view over a todo's evidence.
struct Ledger<'e> {
    evidence: &'e TodoEvidence,
}

impl<'e> Ledger<'e> {
    fn new(evidence: &'e TodoEvidence) -> Self {
        Self { evidence }
    }

    fn history(&self, name: &str) -> Option<&'e [ArtifactVersion]> {
        self.evidence
            .artifacts
            .iter()
            .find(|a| a.name.as_str() == name)
            .map(|a| a.versions.as_slice())
    }

    fn current(&self, name: &str) -> Option<&'e ContentDigest> {
        self.history(name)?.last().map(|v| &v.digest)
    }

    fn current_version(&self, name: &str) -> Option<&'e ArtifactVersion> {
        self.history(name)?.last()
    }

    fn at(&self, stamp_index: usize) -> Option<u64> {
        self.evidence.stamps.get(stamp_index).map(|s| s.sequence)
    }

    fn latest_at<T>(&self, mut pick: impl FnMut(&'e StampFact) -> Option<T>) -> Option<(usize, T)> {
        self.evidence
            .stamps
            .iter()
            .enumerate()
            .rev()
            .find_map(|(i, s)| pick(&s.fact).map(|v| (i, v)))
    }

    fn latest_after<T>(
        &self,
        after: Option<usize>,
        mut pick: impl FnMut(&'e StampFact) -> Option<T>,
    ) -> Option<T> {
        let start = after.map_or(0, |i| i + 1);
        self.evidence.stamps[start..]
            .iter()
            .rev()
            .find_map(|s| pick(&s.fact))
    }

    fn latest_invalidation(&self, phase: PreparationReview) -> Option<usize> {
        self.evidence
            .stamps
            .iter()
            .enumerate()
            .rev()
            .find_map(|(i, s)| match &s.fact {
                StampFact::Kickback {
                    invalidated_reviews,
                    ..
                } if invalidated_reviews.contains(&phase) => Some(i),
                StampFact::DriftReviewed {
                    artifact,
                    class: DriftClass::Substantive,
                    ..
                } if invalidates_phase(artifact.as_str(), phase) => Some(i),
                _ => None,
            })
    }
}

fn invalidates_phase(artifact: &str, phase: PreparationReview) -> bool {
    match artifact {
        artifact_names::PRD => true,
        artifact_names::ARCHITECTURE => matches!(
            phase,
            PreparationReview::ArchitectureReview | PreparationReview::PlanReview
        ),
        artifact_names::IMPLEMENTATION_PLAN => phase == PreparationReview::PlanReview,
        _ => false,
    }
}

fn eligible(approver: Approver, by: ApprovedBy) -> bool {
    match approver {
        Approver::Human => by == ApprovedBy::Human,
        Approver::Peer => true,
    }
}

fn current_approval<'e>(
    ledger: &Ledger<'e>,
    gate: ApprovalGate,
    approver: Approver,
    artifact: &str,
    revoked: Option<usize>,
) -> Option<(ApprovedBy, &'e ContentDigest)> {
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
}

/// Whether the todo's PRD approval invariant is satisfied under `approver`.
/// Features and any todo carrying a `prd` artifact require an eligible
/// approval bound to the current `prd` version.
pub fn prd_approval_satisfied(evidence: &TodoEvidence, approver: Approver) -> bool {
    let ledger = Ledger::new(evidence);
    let requires_prd =
        evidence.kind == TodoKind::Feature || ledger.current(artifact_names::PRD).is_some();
    if !requires_prd {
        return true;
    }
    let Some(digest) = ledger.current(artifact_names::PRD) else {
        return false;
    };
    let revoked = ledger.latest_invalidation(PreparationReview::PrdReview);
    current_approval(
        &ledger,
        ApprovalGate::Prd,
        approver,
        artifact_names::PRD,
        revoked,
    )
    .map(|(_, target)| target)
        == Some(digest)
}

/// Whether a task todo's task-plan approval invariant is satisfied under `approver`.
pub fn task_approval_satisfied(evidence: &TodoEvidence, approver: Approver) -> bool {
    if evidence.kind != TodoKind::Task {
        return true;
    }
    let ledger = Ledger::new(evidence);
    let Some(digest) = ledger.current(artifact_names::TASK_PLAN) else {
        return false;
    };
    current_approval(
        &ledger,
        ApprovalGate::Task,
        approver,
        artifact_names::TASK_PLAN,
        None,
    )
    .map(|(_, target)| target)
        == Some(digest)
}

/// Whether current evidence still needs a human's approval (for board
/// `Needs you` reflection and suppressing premature runner re-dispatch).
pub fn waiting_for_human_approval(evidence: &TodoEvidence, prd: Approver, task: Approver) -> bool {
    if evidence.status != TodoStatus::Active {
        return false;
    }
    let ledger = Ledger::new(evidence);
    let needs = |gate, approver, artifact, revoked| {
        approver == Approver::Human
            && ledger.current(artifact).is_some_and(|digest| {
                current_approval(&ledger, gate, approver, artifact, revoked)
                    .map(|(_, target)| target)
                    != Some(digest)
            })
    };
    let prd_review = ledger.latest_after(
        ledger.latest_invalidation(PreparationReview::PrdReview),
        |fact| match fact {
            StampFact::PreparationVerdict {
                phase: PreparationReview::PrdReview,
                verdict,
                target,
                report,
            } => Some((verdict, target, report)),
            _ => None,
        },
    );
    let eligible_prd = prd_review.is_some_and(|(verdict, target, report)| {
        *verdict == ReviewVerdict::Approve
            && ledger.current(artifact_names::PRD) == Some(target)
            && ledger.current(artifact_names::PRD_REVIEW_FINDINGS) == Some(report)
    });
    eligible_prd
        && needs(
            ApprovalGate::Prd,
            prd,
            artifact_names::PRD,
            ledger.latest_invalidation(PreparationReview::PrdReview),
        )
        || (evidence.kind == TodoKind::Task
            && needs(ApprovalGate::Task, task, artifact_names::TASK_PLAN, None))
}

/// The current build verdict if any (ignoring superseded verdicts).
pub(crate) fn current_build_verdict(
    evidence: &TodoEvidence,
) -> Option<(BuildVerdict, &CommitSha, &ContentDigest)> {
    evidence.stamps.iter().rev().find_map(|s| match &s.fact {
        StampFact::BuildVerdict {
            verdict,
            target,
            report,
        } => Some((*verdict, target, report)),
        _ => None,
    })
}

/// The latest finalize-ready commit if it still stands (no later build
/// completion or kickback).
pub(crate) fn current_finalize_ready(evidence: &TodoEvidence) -> Option<&CommitSha> {
    evidence
        .stamps
        .iter()
        .rev()
        .find_map(|s| match &s.fact {
            StampFact::FinalizeReady { commit } => Some(Some(commit)),
            StampFact::BuildComplete { .. } | StampFact::Kickback { .. } => Some(None),
            _ => None,
        })
        .flatten()
}

/// Whether a finalize handoff is already pending for the current published
/// head (so the runner should wait for `integrate` rather than re-dispatching
/// `todo-orchestrator`).
pub(crate) fn finalize_handoff_pending(evidence: &TodoEvidence, observed: &Observations) -> bool {
    let Some(finalized) = current_finalize_ready(evidence) else {
        return false;
    };
    let head = observed
        .branch_head
        .as_ref()
        .or_else(|| observed.worktree.as_ref().and_then(|w| w.head.as_ref()))
        .or(observed.remote_head.as_ref());
    head == Some(finalized)
}

/// Factual progress fingerprint for runner stall detection (`executions.rs`).
/// Changes whenever a stamp, artifact version, attempt state, or git head
/// advances.
pub(crate) fn progress_fingerprint(evidence: &TodoEvidence, observed: &Observations) -> String {
    let max_seq = evidence
        .stamps
        .iter()
        .map(|s| s.sequence)
        .chain(
            evidence
                .artifacts
                .iter()
                .flat_map(|a| a.versions.iter().map(|v| v.sequence)),
        )
        .max()
        .unwrap_or(0);
    let head = observed
        .worktree
        .as_ref()
        .and_then(|w| w.head.as_ref())
        .or(observed.branch_head.as_ref())
        .map(|h| &h.as_str()[..8.min(h.as_str().len())])
        .unwrap_or("none");
    let active_attempts = evidence
        .bindings
        .iter()
        .filter(|b| {
            matches!(
                b.attempt.assignment,
                AssignmentKind::Workspace | AssignmentKind::Child
            ) && b.attempt.released_at.is_none()
        })
        .map(|b| {
            format!(
                "{}:{}:{}",
                b.role,
                b.attempt.number,
                if b.attempt.no_start_reason.is_some() {
                    "nostart"
                } else {
                    "live"
                }
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    let pushed = observed.branch_head.is_some() && observed.branch_head == observed.remote_head;
    format!(
        "s{max_seq}:b{}:h{head}:p{}:a[{active_attempts}]",
        evidence.bindings.len(),
        u8::from(pushed)
    )
}

/// Drift classification of one tracked artifact from current grounding and
/// drift-review stamps.
fn drift_status(
    ledger: &Ledger<'_>,
    artifact: &str,
    observed: &Observations,
) -> Option<DriftStatus> {
    let version = ledger.current_version(artifact)?;
    let grounding_drifted = observed
        .grounding
        .get(artifact)
        .is_some_and(|g| g.drifted());
    let upstream_drifted = upstream_of(artifact).iter().any(|u| {
        let recorded = version
            .derived_from
            .iter()
            .find(|up| up.artifact.as_str() == *u)
            .and_then(|up| up.digest.as_ref());
        let current = ledger.current(u);
        recorded != current && current.is_some()
    });
    if !grounding_drifted && !upstream_drifted {
        return Some(DriftStatus::Fresh);
    }
    let latest_review = ledger
        .evidence
        .stamps
        .iter()
        .rev()
        .find_map(|s| match &s.fact {
            StampFact::DriftReviewed {
                artifact: reviewed,
                class,
                examined,
                ..
            } if reviewed.as_str() == artifact => Some((s.sequence, *class, examined)),
            _ => None,
        });
    let Some((seq, class, examined)) = latest_review else {
        return Some(DriftStatus::Pending);
    };
    if examined != &version.digest || seq < version.sequence {
        return Some(DriftStatus::Pending);
    }
    if class == DriftClass::Cosmetic
        && observed
            .judged_grounding
            .get(artifact)
            .is_some_and(|g| g.changed())
    {
        return Some(DriftStatus::Pending);
    }
    Some(match class {
        DriftClass::Cosmetic => DriftStatus::Cosmetic,
        DriftClass::Substantive => DriftStatus::Substantive,
    })
}

/// Pure factual derivation of [`TodoLedger`].
pub(crate) fn derive_ledger(
    evidence: &TodoEvidence,
    observed: &Observations,
    approvers: Approvers,
    prerequisites: Vec<(TodoSlug, PrerequisiteStanding)>,
) -> TodoLedger {
    let ledger = Ledger::new(evidence);
    let binding_tuples: Vec<(RoleName, AssignmentKind, Option<&SessionRef>)> = evidence
        .bindings
        .iter()
        .map(|b| {
            (
                b.role.clone(),
                b.attempt.assignment,
                Some(&b.binding.session),
            )
        })
        .collect();

    let branch = evidence
        .branch
        .clone()
        .or_else(|| selected_workspace(evidence).and_then(|b| b.attempt.branch.clone()));
    let pushed = observed.branch_head.is_some()
        && observed.remote_head.is_some()
        && observed.branch_head == observed.remote_head;

    let workspace = WorkspaceFact {
        branch,
        worktree_path: observed.worktree_path.clone(),
        worktree_exists: observed.worktree.is_some(),
        worktree_clean: observed.worktree.as_ref().map(|w| w.clean),
        worktree_head: observed.worktree.as_ref().and_then(|w| w.head.clone()),
        branch_head: observed.branch_head.clone(),
        remote_head: observed.remote_head.clone(),
        pushed,
    };

    let active_attempts: Vec<AttemptFact> = evidence
        .bindings
        .iter()
        .filter(|b| b.attempt.released_at.is_none())
        .map(|b| AttemptFact {
            role: b.role.clone(),
            attempt: b.attempt.number,
            assignment: b.attempt.assignment,
            session: Some(b.binding.session.clone()),
            base_head: b.attempt.base_head.clone(),
            completed_head: b.attempt.completed_head.clone(),
            failure_reason: b.attempt.failure_reason.clone(),
            active: true,
        })
        .collect();

    let artifacts: Vec<ArtifactFact> = evidence
        .artifacts
        .iter()
        .filter_map(|a| {
            let last = a.versions.last()?;
            let drift = DRIFT_ARTIFACTS
                .contains(&a.name.as_str())
                .then(|| drift_status(&ledger, a.name.as_str(), observed))
                .flatten();
            Some(ArtifactFact {
                name: a.name.as_str().to_owned(),
                versions: a.versions.len(),
                current_digest: last.digest.clone(),
                drift,
            })
        })
        .collect();

    let mut preparation_reviews = Vec::new();
    for (phase, phase_name, target_art, report_art) in [
        (
            PreparationReview::PrdReview,
            "prd_review",
            artifact_names::PRD,
            artifact_names::PRD_REVIEW_FINDINGS,
        ),
        (
            PreparationReview::ArchitectureReview,
            "architecture_review",
            artifact_names::ARCHITECTURE,
            artifact_names::ARCHITECTURE_REVIEW_FINDINGS,
        ),
        (
            PreparationReview::PlanReview,
            "plan_review",
            artifact_names::IMPLEMENTATION_PLAN,
            artifact_names::PLAN_REVIEW_FINDINGS,
        ),
    ] {
        let invalidated = ledger.latest_invalidation(phase);
        if let Some((verdict, target, report)) = ledger.latest_after(invalidated, |fact| match fact
        {
            StampFact::PreparationVerdict {
                phase: p,
                verdict,
                target,
                report,
            } if *p == phase => Some((*verdict, target.clone(), report.clone())),
            _ => None,
        }) {
            let current = ledger.current(target_art) == Some(&target)
                && ledger.current(report_art) == Some(&report);
            preparation_reviews.push(ReviewFact {
                phase: phase_name.to_owned(),
                verdict,
                target_digest: target,
                report_digest: report,
                current,
                open_findings: verdict == ReviewVerdict::NeedsWork && current,
            });
        }
    }

    let build_review = evidence.stamps.iter().rev().find_map(|s| match &s.fact {
        StampFact::BuildVerdict {
            verdict,
            target,
            report,
        } => {
            let report_matches = ledger.current(artifact_names::REVIEW_FINDINGS) == Some(report);
            let head = observed
                .worktree
                .as_ref()
                .and_then(|w| w.head.as_ref())
                .or(observed.branch_head.as_ref());
            let binding = if !report_matches {
                BuildReviewBinding::ReportChanged
            } else if head == Some(target) {
                BuildReviewBinding::ExactTarget
            } else {
                match observed.reviewed_change {
                    Some(ChangeSinceReview::Unchanged) => BuildReviewBinding::ExactTarget,
                    Some(
                        ChangeSinceReview::CleanUpstreamMerges | ChangeSinceReview::TodoChanged,
                    ) => BuildReviewBinding::TodoChanged,
                    Some(ChangeSinceReview::Undetermined) => BuildReviewBinding::Undetermined,
                    None => BuildReviewBinding::Unbound,
                }
            };
            let independent = is_independent_reviewer(
                evidence.claim.as_ref().and_then(|c| c.session.as_ref()),
                &binding_tuples,
                &s.actor,
                None,
            );
            Some(BuildReviewFact {
                verdict: *verdict,
                target_commit: target.clone(),
                report_digest: report.clone(),
                reviewer: match &s.actor {
                    Actor::Session(id) => id.to_string(),
                    Actor::GitHub { login } => format!("@{login}"),
                    Actor::Person { name } => name.clone(),
                },
                independent,
                binding,
                open_findings: *verdict == BuildVerdict::ChangesRequested
                    && binding == BuildReviewBinding::ExactTarget,
            })
        }
        _ => None,
    });

    let diagnosis_settled =
        ledger.current(artifact_names::DIAGNOSIS).is_some_and(|d| {
            evidence.stamps.iter().rev().any(
                |s| matches!(&s.fact, StampFact::DiagnosisSettled { diagnosis } if diagnosis == d),
            )
        });

    let deferrals_open = ledger.current(artifact_names::DEFERRALS).is_some_and(|d| {
        !evidence.stamps.iter().rev().any(
            |s| matches!(&s.fact, StampFact::DeferralsProcessed { deferrals } if deferrals == d),
        )
    });

    let finalize_ready_commit = current_finalize_ready(evidence).cloned();

    // Gate 1: Interference lock
    let active_workspace_attempt = evidence
        .bindings
        .iter()
        .rev()
        .find(|b| {
            matches!(
                b.attempt.assignment,
                AssignmentKind::Workspace | AssignmentKind::Child
            ) && b.attempt.released_at.is_none()
        })
        .map(|b| format!("{}/{}", b.role, b.attempt.number));

    // Gate 3: Human-owned approvals
    let prd_approval = {
        let requires_prd =
            evidence.kind == TodoKind::Feature || ledger.current(artifact_names::PRD).is_some();
        if !requires_prd {
            ApprovalStatus::NotRequired
        } else {
            let revoked = ledger.latest_invalidation(PreparationReview::PrdReview);
            match (
                ledger.current(artifact_names::PRD),
                current_approval(
                    &ledger,
                    ApprovalGate::Prd,
                    approvers.prd,
                    artifact_names::PRD,
                    revoked,
                ),
            ) {
                (Some(digest), Some((by, target))) if target == digest => {
                    ApprovalStatus::Approved {
                        by,
                        target_digest: target.clone(),
                    }
                }
                (digest, _) => ApprovalStatus::Pending {
                    approver: approver_word(approvers.prd).to_owned(),
                    target_digest: digest.cloned(),
                },
            }
        }
    };

    let task_approval = if evidence.kind != TodoKind::Task {
        ApprovalStatus::NotRequired
    } else {
        match (
            ledger.current(artifact_names::TASK_PLAN),
            current_approval(
                &ledger,
                ApprovalGate::Task,
                approvers.task,
                artifact_names::TASK_PLAN,
                None,
            ),
        ) {
            (Some(digest), Some((by, target))) if target == digest => ApprovalStatus::Approved {
                by,
                target_digest: target.clone(),
            },
            (digest, _) => ApprovalStatus::Pending {
                approver: approver_word(approvers.task).to_owned(),
                target_digest: digest.cloned(),
            },
        }
    };

    // Gate 2: Independent exact-SHA review gate for integrate
    let integrate_review_gate = {
        let mut reasons = Vec::new();
        if active_workspace_attempt.is_some() {
            reasons.push("active workspace attempt must be released".to_owned());
        }
        if matches!(prd_approval, ApprovalStatus::Pending { .. }) {
            reasons.push("PRD approval required".to_owned());
        }
        match &observed.worktree {
            None => reasons.push("worktree not present".to_owned()),
            Some(wt) if !wt.clean => reasons.push("worktree has uncommitted changes".to_owned()),
            _ => {}
        }
        let head = observed
            .worktree
            .as_ref()
            .and_then(|w| w.head.clone())
            .or_else(|| observed.branch_head.clone());
        if !pushed {
            reasons.push("branch head not pushed to origin".to_owned());
        }
        match &build_review {
            Some(br)
                if br.verdict == BuildVerdict::Approve
                    && br.independent
                    && br.binding == BuildReviewBinding::ExactTarget => {}
            Some(br) if !br.independent => {
                reasons.push("build review approve is not independent".to_owned());
            }
            Some(br) if br.verdict != BuildVerdict::Approve => {
                reasons.push("build review has changes_requested".to_owned());
            }
            Some(_) => {
                reasons.push(
                    "build review approve does not match exact current HEAD and findings"
                        .to_owned(),
                );
            }
            None => {
                reasons.push("no independent build_review approve stamp".to_owned());
            }
        }
        if reasons.is_empty()
            && let Some(commit) = head
        {
            IntegrateGateStatus::Ready {
                finalize_stamped: finalize_ready_commit.as_ref() == Some(&commit),
                commit,
            }
        } else {
            IntegrateGateStatus::Blocked { reasons }
        }
    };

    let prerequisites_satisfied = prerequisites
        .iter()
        .all(|(_, s)| *s == PrerequisiteStanding::Delivered);
    let prerequisites = prerequisites
        .into_iter()
        .map(|(slug, standing)| PrerequisiteFact { slug, standing })
        .collect();

    TodoLedger {
        project: evidence.project.clone(),
        slug: evidence.slug.clone(),
        kind: evidence.kind,
        status: evidence.status,
        claimed: evidence.claim.is_some(),
        claim_session: evidence.claim.as_ref().and_then(|c| c.session.clone()),
        hold_reason: evidence.hold.as_ref().map(|h| h.reason.clone()),
        delivery_commit: evidence.delivery.clone(),
        workspace,
        active_attempts,
        artifacts,
        preparation_reviews,
        build_review,
        diagnosis_settled,
        deferrals_open,
        finalize_ready_commit,
        prerequisites,
        prerequisites_satisfied,
        gates: GatesFact {
            active_workspace_attempt,
            prd_approval,
            task_approval,
            integrate_review_gate,
        },
    }
}

fn approver_word(approver: Approver) -> &'static str {
    match approver {
        Approver::Human => "human",
        Approver::Peer => "peer",
    }
}

/// Whether any prerequisite artifact snapshot in `dependencies` changed or
/// no longer resolves against `prerequisites`.
pub(crate) fn dependency_change(
    dependencies: &[DependencyDigest],
    prerequisites: &[TodoEvidence],
) -> (bool, bool) {
    let mut changed = false;
    let mut unresolved = false;
    for dep in dependencies {
        let Some(id) = &dep.id else {
            unresolved = true;
            continue;
        };
        let Some(other) = prerequisites.iter().find(|e| &e.id == id) else {
            unresolved = true;
            continue;
        };
        let current = other
            .artifacts
            .iter()
            .find(|a| a.name == dep.artifact)
            .and_then(|a| a.versions.last())
            .map(|v| &v.digest);
        if current != dep.digest.as_ref() {
            changed = true;
        }
    }
    (changed, unresolved)
}

/// Standing of every prerequisite in `evidence.prerequisites`.
pub(crate) fn standings<E>(
    prerequisites: &[Prerequisite],
    mut load: impl FnMut(&TodoSlug) -> Result<Option<TodoEvidence>, E>,
) -> Result<Vec<(TodoSlug, PrerequisiteStanding)>, E> {
    let mut out = Vec::with_capacity(prerequisites.len());
    let mut seen = BTreeSet::new();
    for prereq in prerequisites {
        match prereq {
            Prerequisite::EmptyHolder(slug) => {
                if seen.insert(slug.clone()) {
                    out.push((slug.clone(), PrerequisiteStanding::EmptyHolder));
                }
            }
            Prerequisite::Todo(slug) => {
                if !seen.insert(slug.clone()) {
                    continue;
                }
                let standing = match load(slug)? {
                    None => PrerequisiteStanding::NotFound,
                    Some(other) if other.status == TodoStatus::Delivered => {
                        PrerequisiteStanding::Delivered
                    }
                    Some(_) => PrerequisiteStanding::Undelivered,
                };
                out.push((slug.clone(), standing));
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::super::_testing::{digest, session, sha, todo};
    use super::*;

    #[test]
    fn fast_track_bug_needs_no_prd_or_plan_to_pass_integrate_gate() {
        let head = sha('a');
        let mut evidence = todo(TodoKind::Bug)
            .artifact(artifact_names::INPUT, "fix crash")
            .artifact(artifact_names::REVIEW_FINDINGS, "looks great")
            .claimed()
            .build();
        // Independent reviewer stamps approve on exact HEAD
        evidence.stamps.push(crate::evidence::StampRecord {
            source_comment: None,
            fact: StampFact::BuildVerdict {
                verdict: BuildVerdict::Approve,
                target: head.clone(),
                report: digest("looks great"),
            },
            at: jiff::Timestamp::from_second(1_800_000_010).unwrap(),
            actor: Actor::Session(session("independent-reviewer")),
            model: None,
            eval_fingerprint: None,
            role: Some(RoleName::parse("build-reviewer").unwrap()),
            human_involved: None,
            sequence: 10,
            model_provenance: Default::default(),
        });
        let observed = Observations {
            worktree: Some(WorktreeObservation {
                clean: true,
                head: Some(head.clone()),
            }),
            worktree_path: Some(PathBuf::from("/tmp/wt")),
            branch_head: Some(head.clone()),
            remote_head: Some(head.clone()),
            reviewed_change: Some(ChangeSinceReview::Unchanged),
            ..Default::default()
        };
        let ledger = derive_ledger(
            &evidence,
            &observed,
            Approvers {
                prd: Approver::Human,
                task: Approver::Human,
            },
            Vec::new(),
        );
        assert_eq!(ledger.gates.prd_approval, ApprovalStatus::NotRequired);
        assert_eq!(
            ledger.gates.integrate_review_gate,
            IntegrateGateStatus::Ready {
                commit: head,
                finalize_stamped: false,
            }
        );
    }

    #[test]
    fn self_approval_by_orchestrator_or_builder_fails_integrate_gate() {
        let head = sha('b');
        let mut evidence = todo(TodoKind::Bug)
            .artifact(artifact_names::INPUT, "fix crash")
            .artifact(artifact_names::REVIEW_FINDINGS, "self approved")
            .claimed()
            .build();
        evidence.stamps.push(crate::evidence::StampRecord {
            source_comment: None,
            fact: StampFact::BuildVerdict {
                verdict: BuildVerdict::Approve,
                target: head.clone(),
                report: digest("self approved"),
            },
            at: jiff::Timestamp::from_second(1_800_000_010).unwrap(),
            // Same session as claim holder ("orchestrator")!
            actor: Actor::Session(session("orchestrator")),
            model: None,
            eval_fingerprint: None,
            role: Some(RoleName::parse("build-reviewer").unwrap()),
            human_involved: None,
            sequence: 10,
            model_provenance: Default::default(),
        });
        let observed = Observations {
            worktree: Some(WorktreeObservation {
                clean: true,
                head: Some(head.clone()),
            }),
            worktree_path: Some(PathBuf::from("/tmp/wt")),
            branch_head: Some(head.clone()),
            remote_head: Some(head),
            reviewed_change: Some(ChangeSinceReview::Unchanged),
            ..Default::default()
        };
        let ledger = derive_ledger(
            &evidence,
            &observed,
            Approvers {
                prd: Approver::Human,
                task: Approver::Human,
            },
            Vec::new(),
        );
        assert!(matches!(
            ledger.gates.integrate_review_gate,
            IntegrateGateStatus::Blocked { .. }
        ));
    }

    #[test]
    fn feature_requires_prd_approval_before_integrate_gate() {
        let head = sha('c');
        let mut evidence = todo(TodoKind::Feature)
            .artifact(artifact_names::INPUT, "new feature")
            .artifact(artifact_names::PRD, "prd-v1")
            .artifact(artifact_names::REVIEW_FINDINGS, "looks good")
            .claimed()
            .build();
        evidence.stamps.push(crate::evidence::StampRecord {
            source_comment: None,
            fact: StampFact::BuildVerdict {
                verdict: BuildVerdict::Approve,
                target: head.clone(),
                report: digest("looks good"),
            },
            at: jiff::Timestamp::from_second(1_800_000_010).unwrap(),
            actor: Actor::Session(session("independent-reviewer")),
            model: None,
            eval_fingerprint: None,
            role: Some(RoleName::parse("build-reviewer").unwrap()),
            human_involved: None,
            sequence: 10,
            model_provenance: Default::default(),
        });
        let observed = Observations {
            worktree: Some(WorktreeObservation {
                clean: true,
                head: Some(head.clone()),
            }),
            worktree_path: Some(PathBuf::from("/tmp/wt")),
            branch_head: Some(head.clone()),
            remote_head: Some(head),
            reviewed_change: Some(ChangeSinceReview::Unchanged),
            ..Default::default()
        };
        let ledger = derive_ledger(
            &evidence,
            &observed,
            Approvers {
                prd: Approver::Human,
                task: Approver::Human,
            },
            Vec::new(),
        );
        assert!(matches!(
            ledger.gates.prd_approval,
            ApprovalStatus::Pending { .. }
        ));
        assert!(matches!(
            ledger.gates.integrate_review_gate,
            IntegrateGateStatus::Blocked { ref reasons }
                if reasons.iter().any(|r| r.contains("PRD approval"))
        ));
    }

    #[test]
    fn kickback_or_build_complete_resets_finalize_ready_at_same_head() {
        let head = sha('d');
        let observed = Observations {
            worktree: Some(WorktreeObservation {
                clean: true,
                head: Some(head.clone()),
            }),
            worktree_path: Some(PathBuf::from("/tmp/wt")),
            branch_head: Some(head.clone()),
            remote_head: Some(head.clone()),
            reviewed_change: Some(ChangeSinceReview::Unchanged),
            ..Default::default()
        };
        let stamp = |fact: StampFact, seq: u64| crate::evidence::StampRecord {
            source_comment: None,
            fact,
            at: jiff::Timestamp::from_second(1_800_000_000 + seq as i64).unwrap(),
            actor: Actor::Session(session("lead")),
            model: None,
            eval_fingerprint: None,
            role: Some(RoleName::parse("todo-orchestrator").unwrap()),
            human_involved: None,
            sequence: seq,
            model_provenance: Default::default(),
        };

        let mut kicked = todo(TodoKind::Change).build();
        kicked.stamps.push(stamp(
            StampFact::FinalizeReady {
                commit: head.clone(),
            },
            1,
        ));
        assert_eq!(current_finalize_ready(&kicked), Some(&head));
        assert!(finalize_handoff_pending(&kicked, &observed));

        kicked.stamps.push(stamp(
            StampFact::Kickback {
                reason: "CI failed on main".into(),
                input: digest("input"),
                invalidated_plan: None,
                invalidated_reviews: Vec::new(),
            },
            2,
        ));
        assert_eq!(current_finalize_ready(&kicked), None);
        assert!(!finalize_handoff_pending(&kicked, &observed));

        let mut rebuilt = todo(TodoKind::Change).build();
        rebuilt.stamps.push(stamp(
            StampFact::FinalizeReady {
                commit: head.clone(),
            },
            1,
        ));
        rebuilt
            .stamps
            .push(stamp(StampFact::BuildComplete { head: head.clone() }, 2));
        assert_eq!(current_finalize_ready(&rebuilt), None);
        assert!(!finalize_handoff_pending(&rebuilt, &observed));
    }
}
