//! Runtime-neutral re-exports plus runner-owned assignment wrappers.
use std::path::Path;

use instrukt_core::ids::SessionRef;
pub use instrukt_runtime::cold_text;
pub use instrukt_runtime::{
    ActiveTurn, Archive, ArchiveOutcome, AssignmentEvidence, Capability, Conversation,
    DeliveryRealization, DispatchError, ExecutionRootCause, ExecutionRootMode, ExecutionRootStatus,
    FinishedThread, InputObservation, InputState, LaunchInteractionMode, LaunchRuntimeMode,
    Message, NativeActivity, NativeCause, NativeDiagnostic, NativeLaunch, ObservedSnapshot,
    ObservedTurn, OpeningInput, Presence, Profiles, ProviderFacts, ProviderSnapshot,
    RUNTIME_COMMAND_TIMEOUT, Readiness, ReportedModel, ResolvedLaunchModes, RuntimeAdapter,
    RuntimeError, RuntimeFactory, RuntimeRegistry, RuntimeStart, SelectionOperation, SendEvidence,
    SessionState, Thread, TurnState, UsageWindow, WorktreeRequest, diagnose, reference,
    thread_title,
};

/// Reconstruct the preceding driver's label from its retained role and phase placement.
/// This historical format uses the original phase branch, never a current todo slug.
pub fn legacy_driver_title(role: &str, branch: &str, key: &str) -> Option<String> {
    let slug = branch
        .strip_suffix("/prepare")
        .or_else(|| branch.strip_suffix("/work"))?;
    if role.is_empty() || slug.is_empty() || key.is_empty() {
        return None;
    }
    Some(format!("{role} · {slug} · {key}"))
}

/// Operations use structured facts; no user input is executable command text.
pub struct RuntimeAssignment<'a> {
    pub bindings: Option<&'a dyn crate::interfaces::SessionBindings>,
    pub invocation: &'a crate::keyed::RuntimeInvocation,
    pub project: Option<&'a str>,
    pub request: &'a RuntimeStart,
    /// An existing linked worktree of `request.checkout` to attach on initial
    /// launch; persisted on [`NativeLaunch::existing_worktree`].
    pub existing_worktree: Option<&'a Path>,
    pub now: jiff::Timestamp,
}

pub struct FollowupAssignment<'a> {
    pub bindings: Option<&'a dyn crate::interfaces::SessionBindings>,
    pub original_request: Option<&'a RuntimeStart>,
    pub invocation: &'a crate::keyed::RuntimeInvocation,
    pub request: &'a RuntimeStart,
    pub session: &'a SessionRef,
    pub worktree_path: Option<&'a Path>,
    pub project: Option<&'a str>,
    /// When `true`, succession uses `request.brief` verbatim (for example,
    /// replacing a pre-upgrade `turn_output` opening brief with a freshly
    /// rendered explicit-frontstage brief) instead of copying the prior
    /// generation's conversation assignment prefix.
    pub replace_brief: bool,
    pub now: jiff::Timestamp,
}
