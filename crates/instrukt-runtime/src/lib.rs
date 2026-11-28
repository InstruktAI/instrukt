//! Runtime-neutral facade, typed turn-observation contracts, and adapter registry.
mod _adapter;
mod _diagnostics;
mod _error;
mod _registry;
mod _types;
pub mod cold_text;

pub use _adapter::{Archive, Readiness, RuntimeAdapter};
pub use _diagnostics::{NativeCause, NativeDiagnostic, diagnose, reference};
pub use _error::{DispatchError, ExecutionRootCause, RuntimeError};
pub use _registry::{RuntimeFactory, RuntimeRegistry};
pub use _types::{
    ActiveTurn, ArchiveOutcome, AssignmentEvidence, Capability, Conversation, DeliveryRealization,
    ExecutionRootMode, ExecutionRootStatus, FinishedThread, InputObservation, InputState,
    LaunchInteractionMode, LaunchRuntimeMode, Message, NativeActivity, NativeLaunch,
    ObservedSnapshot, ObservedTurn, OpeningInput, Presence, Profiles, ProviderFacts,
    ProviderSnapshot, RUNTIME_COMMAND_TIMEOUT, ReportedModel, ResolvedLaunchModes, RuntimeStart,
    SelectionOperation, SendEvidence, SessionState, Thread, TurnState, UsageWindow,
    WorktreeRequest, thread_title,
};
