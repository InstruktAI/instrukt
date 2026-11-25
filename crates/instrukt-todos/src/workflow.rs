//! The workflow context: a facts-only ledger over recorded evidence and
//! repository state, paired with the three hard integrity invariants and
//! runner assignment (`docs/project/spec/domain/workflow.md`).
//!
//! Code owns facts, the three integrity invariants (no interference,
//! independent exact-SHA review gate at integrate, human-owned approval
//! gates), and runner dispatch. All activity routing lives in prose
//! procedures for the unified `todo-orchestrator`.

mod _coverage;
mod _finding;
mod _flavor;
mod _observe;
mod _provider;
mod _schema;
mod _service;
mod _standing;
#[cfg(test)]
pub(crate) mod _testing;

pub use _coverage::{Approved, Coverage, CoverageEvidence, CoveringItem, CoveringKind, coverage};
pub use _finding::{Finding, Problem};
pub use _flavor::{Flavor, FlavorError, WORKFLOW_FILE};
pub use _observe::ObservationError;
pub(crate) use _observe::{
    VERIFIER_ROLE, handoff_admitted, readiness_approved, verification_admission,
};
pub use _provider::{
    ApprovalStatus, ArtifactFact, AttemptFact, BuildReviewBinding, BuildReviewFact, DriftStatus,
    GatesFact, IntegrateGateStatus, PrerequisiteFact, PrerequisiteStanding, ReviewFact, TodoLedger,
    WorkspaceFact, is_independent_reviewer, prd_approval_satisfied, task_approval_satisfied,
    waiting_for_human_approval,
};
pub use _schema::WorkflowSchema;
pub use _service::{
    Pickup, SelectedFlavorRoles, WorkEntry, Workflow, WorkflowError, related_evidence,
    todos_enabled,
};
