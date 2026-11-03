//! The interfaces the runner declares for work and workflow
//! (`docs/local/spec/domain/runner.md`, "Context map position").
//!
//! Work's command contract covers every runner mutation; work owns their
//! validation and persistence. The todos crate implements these traits and
//! the CLI wires them into the runner, preserving todos → runner → core.

use std::path::PathBuf;

use instrukt_core::ids::{ProjectSlug, RoleName, TodoSlug};
use serde::{Deserialize, Serialize};

/// A work or workflow interface call failed.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct WorkInterfaceError(pub String);

/// A project the runner considers: its slug and checkout path, as recorded
/// in publication state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectRef {
    pub slug: ProjectSlug,
    pub root: PathBuf,
}

/// A started session's runtime and runtime-native id, per todo and role;
/// the worktree path the runtime returned is retained after the session
/// ends.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionBinding {
    pub runtime: String,
    pub session_id: String,
    pub worktree_path: Option<PathBuf>,
}

/// The outcome of recording an execution claim and reflecting it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaimOutcome {
    /// This computer holds the claim.
    Claimed,
    /// Another computer claimed it first (a revision conflict); leave it.
    ClaimedElsewhere,
}

/// Reads of the work record the runner needs to dispatch.
pub trait WorkReads: Send + Sync {
    /// Active, unheld todos of the project whose prerequisites allow pickup.
    fn pickup_candidates(&self, project: &ProjectRef) -> Result<Vec<TodoSlug>, WorkInterfaceError>;

    fn session_binding(
        &self,
        project: &ProjectRef,
        todo: &TodoSlug,
        role: &RoleName,
    ) -> Result<Option<SessionBinding>, WorkInterfaceError>;
}

/// The runner's writes through work's command contract.
pub trait WorkCommands: Send + Sync {
    fn record_claim(
        &self,
        project: &ProjectRef,
        todo: &TodoSlug,
    ) -> Result<ClaimOutcome, WorkInterfaceError>;

    fn bind_session(
        &self,
        project: &ProjectRef,
        todo: &TodoSlug,
        role: &RoleName,
        binding: &SessionBinding,
    ) -> Result<(), WorkInterfaceError>;

    /// Pulls GitHub changes for local items and flushes pending reflection,
    /// under the per-computer writer lock.
    fn sync_reflection(&self, project: &ProjectRef) -> Result<(), WorkInterfaceError>;

    fn set_automatic_hold(
        &self,
        project: &ProjectRef,
        todo: &TodoSlug,
        cause: &str,
    ) -> Result<(), WorkInterfaceError>;

    fn purge_expired(&self, project: &ProjectRef) -> Result<(), WorkInterfaceError>;
}

/// The next obligation workflow derives for a todo, as the runner
/// dispatches it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assignment {
    /// The role responsible for the obligation.
    pub role: RoleName,
    /// The action id selected by derivation.
    pub action: String,
    /// The rendered instructions for the session.
    pub brief: String,
}

/// Reading workflow's derived state for dispatch.
pub trait WorkflowState: Send + Sync {
    /// The todo's current obligation, or `None` when nothing is owed now.
    fn next_assignment(
        &self,
        project: &ProjectRef,
        todo: &TodoSlug,
    ) -> Result<Option<Assignment>, WorkInterfaceError>;
}
