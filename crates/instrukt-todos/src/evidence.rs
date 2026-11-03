//! Work's evidence read contract for workflow: the record of one todo as
//! the fact provider reads it (`docs/local/spec/domain/work.md`,
//! "Published contracts": "evidence read capabilities for workflow";
//! `docs/local/spec/domain/workflow.md`, "Flavor-facing contract", the
//! fact table).
//!
//! Work implements [`WorkEvidence`] over its local-leading record; workflow
//! translates a [`TodoEvidence`] (plus repository observations it makes
//! itself: worktree, branch and remote heads, review notes) into the
//! engine's typed facts. The types carry recorded evidence only — no
//! currency, drift or readiness judgment, which are workflow's.
//!
//! Ownership for wave 3: the work builder owns this file and implements the
//! trait; the workflow builder reads it. Changes are additive and
//! announced, so the fact provider never meets a renamed field.

use std::path::PathBuf;

use instrukt_core::ids::{CommitSha, ProjectSlug, RoleName, SessionRef, TodoSlug};
use instrukt_runner::interfaces::{
    AgentPreference, ProjectRef, SessionBinding, TaskTiming, TodoKind, WorkInterfaceError,
};
use jiff::Timestamp;
use serde::{Deserialize, Serialize};

/// Reading one todo's recorded evidence.
pub trait WorkEvidence: Send + Sync {
    /// The record of `todo` in `project` as evidence, pulled through from
    /// GitHub first when it is a local item (`github-record`,
    /// "Local-leading pull-through"); `None` when the project has no todo
    /// with that slug. A conflicted item refuses with
    /// [`WorkInterfaceError::Conflicted`]. Delivered and removed todos
    /// within their retention are returned read-only.
    fn evidence(
        &self,
        project: &ProjectRef,
        todo: &TodoSlug,
    ) -> Result<Option<TodoEvidence>, WorkInterfaceError>;
}

/// A todo's record as evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TodoEvidence {
    pub project: ProjectSlug,
    pub slug: TodoSlug,
    pub kind: TodoKind,
    pub status: TodoStatus,
    /// Every artifact with its retained versions.
    pub artifacts: Vec<ArtifactHistory>,
    /// The stamp ledger, oldest first.
    pub stamps: Vec<StampRecord>,
    /// The item-level execution claim, while held (`claim.held`).
    pub claim: Option<ClaimRecord>,
    /// Per-role session bindings, retained after their sessions end
    /// (`sessions.bound`, `worktree.path`).
    pub bindings: Vec<RoleBinding>,
    /// The dispatch hold, while set.
    pub hold: Option<HoldRecord>,
    /// The prerequisites, with structural holders expanded to the todos
    /// whose milestones satisfy them (`dependencies-and-holds`,
    /// "Structural holders").
    pub prerequisites: Vec<Prerequisite>,
    /// The confirmed delivered commit (`delivery.commit`).
    pub delivery: Option<CommitSha>,
    /// A task's schedule and last firing.
    pub timing: Option<TaskTiming>,
    /// A task's agent/model preference.
    pub agent: Option<AgentPreference>,
}

/// Where a todo stands in the record (`work` domain, "Todo" lifecycle).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TodoStatus {
    Active,
    Iceboxed,
    /// A group parent outside the todo lifecycle.
    StructuralHolder,
    Delivered,
    /// Withdrawn, within its recovery window.
    Removed,
}

/// The name an artifact is written under (`instrukt todo artifacts write
/// <slug> <name> …`). Its identity in the record is its provider address;
/// the name is how commands and facts address it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ArtifactName(String);

impl ArtifactName {
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ArtifactName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Artifact names the fact table reads (`<name>.digest`), plus the intent
/// record `input` and the build lane's `deferrals`.
pub mod artifact_names {
    pub const INPUT: &str = "input";
    pub const PRD: &str = "prd";
    pub const ARCHITECTURE: &str = "architecture";
    pub const DIAGNOSIS: &str = "diagnosis";
    pub const IMPLEMENTATION_PLAN: &str = "implementation_plan";
    pub const TASK_PLAN: &str = "task_plan";
    pub const PRD_REVIEW_FINDINGS: &str = "prd_review_findings";
    pub const ARCHITECTURE_REVIEW_FINDINGS: &str = "architecture_review_findings";
    pub const PLAN_REVIEW_FINDINGS: &str = "plan_review_findings";
    pub const QUALITY_CHECKLIST: &str = "quality_checklist";
    pub const DEFERRALS: &str = "deferrals";
    /// The build review's complete report, which a build-review verdict
    /// binds (`stamps`, "Stamp forms").
    pub const REVIEW_FINDINGS: &str = "review_findings";
}

/// A content digest of one artifact version. Its text form is work's to
/// define; the provider compares digests for equality and renders them as
/// fact values.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ContentDigest(String);

impl ContentDigest {
    pub fn new(digest: impl Into<String>) -> Self {
        Self(digest.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ContentDigest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// An artifact with its retained versions, oldest first; the last is
/// current (`github-record`, "Artifacts and stamps").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactHistory {
    pub name: ArtifactName,
    pub versions: Vec<ArtifactVersion>,
}

/// One saved version: its digest, workflow writer and creation time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactVersion {
    pub digest: ContentDigest,
    pub writer: Actor,
    pub created: Timestamp,
}

/// Who acted: a session, a GitHub account, or a named person outside any
/// session (`stamps`, "Evidence and attribution": no identity is guessed).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Actor {
    Session(SessionRef),
    GitHub { login: String },
    Person { name: String },
}

/// Model identity on a stamp; absent outside an agent session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelIdentity {
    pub model: String,
    pub version: String,
}

/// One stamp: its fact and the envelope every stamp carries (`stamps`,
/// "Evidence and attribution").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StampRecord {
    pub fact: StampFact,
    pub at: Timestamp,
    pub actor: Actor,
    pub model: Option<ModelIdentity>,
    /// The published eval fingerprint copied at write time; `None` before
    /// any publication recorded one.
    pub eval_fingerprint: Option<String>,
    /// The role the acting session played, where known.
    pub role: Option<RoleName>,
    /// The audit-only `--human-involved` declaration, where given.
    pub human_involved: Option<bool>,
}

/// A preparation review phase (`--phase prd_review|architecture_review|plan_review`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreparationReview {
    PrdReview,
    ArchitectureReview,
    PlanReview,
}

/// A preparation review verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewVerdict {
    Approve,
    NeedsWork,
}

/// A build review verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BuildVerdict {
    Approve,
    ChangesRequested,
}

/// What an approval authorizes (`--prd-approved`, `--task-approved`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalGate {
    Prd,
    Task,
}

/// Whether a human or a peer approved (`--by human|peer`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovedBy {
    Human,
    Peer,
}

/// A drift classification an agent recorded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DriftClass {
    Cosmetic,
    Substantive,
}

/// A task execution's recorded result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskResult {
    Success,
    Error,
}

/// The admitted stamp facts and the evidence each binds (`stamps`,
/// "Mutation boundary" and "Stamp forms"; `approvals`, "Command
/// boundary"). Review rounds are not stored; they are counted from
/// verdict stamps.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "fact", rename_all = "snake_case")]
pub enum StampFact {
    /// `--verdict approve|needs_work --phase <preparation review>`.
    PreparationVerdict {
        phase: PreparationReview,
        verdict: ReviewVerdict,
        target: ContentDigest,
        report: ContentDigest,
    },
    /// `--verdict approve|changes_requested --phase build_review`.
    BuildVerdict {
        verdict: BuildVerdict,
        target: CommitSha,
        report: ContentDigest,
    },
    /// `--prd-approved` / `--task-approved --by human|peer`. A carried
    /// approval retains its earlier source in the envelope and names the
    /// target it was carried from.
    Approval {
        gate: ApprovalGate,
        by: ApprovedBy,
        target: ContentDigest,
        carried_from: Option<ContentDigest>,
    },
    /// `--intent-checked`: the `input` version checked and the
    /// conversation it was authored in.
    IntentChecked {
        input: ContentDigest,
        conversation: SessionRef,
    },
    /// `--kickback --reason`.
    Kickback {
        reason: String,
        input: ContentDigest,
        invalidated_plan: Option<ContentDigest>,
    },
    /// `--architecture-skipped <reason>`.
    ArchitectureSkipped { reason: String },
    /// `--diagnosis-settled`.
    DiagnosisSettled { diagnosis: ContentDigest },
    /// `--diagnosis-unresolved --reason` (also establishes the hold).
    DiagnosisUnresolved {
        diagnosis: ContentDigest,
        reason: String,
    },
    /// `--drift-reviewed <artifact> cosmetic|substantive <reason>`.
    DriftReviewed {
        artifact: ArtifactName,
        class: DriftClass,
        reason: String,
        examined: ContentDigest,
    },
    /// `--build-started`: the original base and approved plan digest.
    BuildStarted {
        base: CommitSha,
        plan: ContentDigest,
    },
    /// `--build-complete`: the stamped work-branch head.
    BuildComplete { head: CommitSha },
    /// `--deferrals-processed`: the deferrals artifact it covers.
    DeferralsProcessed { deferrals: ContentDigest },
    /// `--finalize-ready`: the commit handed to integration.
    FinalizeReady { commit: CommitSha },
    /// `--task-firing-started`: the due occurrence.
    TaskFiringStarted { occurrence: Timestamp },
    /// `--task-outcome success|error [--reason]`.
    TaskOutcome {
        occurrence: Timestamp,
        result: TaskResult,
        reason: Option<String>,
    },
}

/// The held execution claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimRecord {
    /// The session holding it, when one does.
    pub session: Option<SessionRef>,
    pub at: Timestamp,
}

/// A session binding with the role it is recorded for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoleBinding {
    pub role: RoleName,
    pub binding: SessionBinding,
}

impl RoleBinding {
    /// The worktree path the binding retained, if the runtime returned one.
    pub fn worktree(&self) -> Option<&PathBuf> {
        self.binding.worktree.as_ref()
    }
}

/// Where a dispatch hold came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HoldSource {
    /// `instrukt todo update <slug> --hold --reason`.
    Manual,
    /// Set by the runner or by a stamp that establishes one.
    Automatic,
}

/// A dispatch hold with its cause (`dependencies-and-holds`, "Session
/// bindings, claims and holds").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HoldRecord {
    pub source: HoldSource,
    pub reason: String,
    pub by: Actor,
    pub at: Timestamp,
}

/// One prerequisite after structural-holder expansion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Prerequisite {
    /// A todo whose milestone gates the dependent; read its own evidence
    /// for its status and preparation.
    Todo(TodoSlug),
    /// A structural holder with no children: never satisfied by having
    /// none.
    EmptyHolder(TodoSlug),
}
