//! Structured runtime primitives; the selected runtime owns conversations and workspaces.
mod _t3;
mod _t3_readiness;

use std::path::{Path, PathBuf};

use instrukt_core::config::{Env, Runtime};
use instrukt_core::ids::SessionRef;
use serde::{Deserialize, Serialize};

use crate::sessions::{SessionState, SessionsError};
pub use _t3::T3;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorktreeRequest {
    pub from: String,
    pub base: String,
    pub branch: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeStart {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub preferences: Vec<crate::agents::AgentChoice>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection: Option<instrukt_core::config::AgentSelection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection_evidence: Option<String>,
    pub checkout: PathBuf,
    pub key: String,
    pub brief: String,
    pub worktree: Option<WorktreeRequest>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Thread {
    pub session: SessionRef,
    pub project_id: Option<String>,
    pub title: Option<String>,
    pub semantic_key: Option<String>,
    pub state: SessionState,
    pub branch: Option<String>,
    pub worktree: Option<PathBuf>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub interrupted: bool,
    pub settled: bool,
    pub settled_at: Option<String>,
    pub native_status: String,
    pub run_id: Option<String>,
    pub activity: Option<NativeActivity>,
    pub native_details: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativeActivity {
    pub at: jiff::Timestamp,
    pub source: String,
    pub meaning: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub text: String,
    pub run_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Conversation {
    pub session: SessionRef,
    pub messages: Vec<Message>,
    pub limit: usize,
    pub bounded: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capability {
    CreateWorktree,
    AttachWorktree,
    Link,
    Notify,
    AgentSelection,
    InputConsumption,
    AssignmentEvidence,
}
impl std::fmt::Display for Capability {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::CreateWorktree => "worktree creation",
            Self::AttachWorktree => "worktree attachment",
            Self::Link => "link",
            Self::Notify => "notification",
            Self::AgentSelection => "agent/model selection",
            Self::InputConsumption => "input consumption observation",
            Self::AssignmentEvidence => "assignment evidence recovery",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreationPermit {
    Initial,
    Reconcile,
}

pub struct FollowupAssignment<'a> {
    pub invocation: &'a crate::keyed::RuntimeInvocation,
    pub request: &'a RuntimeStart,
    pub session: &'a SessionRef,
    pub worktree_path: Option<&'a Path>,
    pub project: Option<&'a str>,
    pub now: jiff::Timestamp,
}

/// Operations use structured facts; no user input is executable command text.
pub struct RuntimeAssignment<'a> {
    pub creation: CreationPermit,
    pub invocation: &'a crate::keyed::RuntimeInvocation,
    pub project: Option<&'a str>,
    pub request: &'a RuntimeStart,
    pub now: jiff::Timestamp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryRealization {
    Steering,
    Queued,
    EntryOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputState {
    Unknown,
    Accepted,
    Consumed,
}

/// Runtime-owned input evidence, independent of transcript/run-id representation.
#[derive(Debug, Clone)]
pub struct InputObservation {
    pub state: InputState,
    pub reference: Option<String>,
    pub source: String,
    pub observed_at: jiff::Timestamp,
}
#[derive(Debug, Clone)]
pub struct AssignmentEvidence {
    pub opening: String,
    pub waiting: Option<String>,
    pub source: String,
    pub observed_at: jiff::Timestamp,
}

pub trait RuntimeAdapter: instrukt_core::interfaces::CallerIdentityLookup {
    fn runtime(&self) -> Runtime;
    fn prepare_start(
        &self,
        home: &instrukt_core::home::Home,
        request: &RuntimeStart,
        now: jiff::Timestamp,
    ) -> Result<RuntimeStart, SessionsError>;
    fn create_thread(&self, request: &RuntimeStart) -> Result<Thread, SessionsError>;
    fn verify_start(&self, request: &RuntimeStart, thread: &Thread) -> Result<(), SessionsError>;
    fn delivery_realization(&self) -> DeliveryRealization;
    fn supports(&self, capability: Capability) -> bool;
    fn observe_input(
        &self,
        _session: &SessionRef,
        _text: &str,
        _now: jiff::Timestamp,
    ) -> Result<InputObservation, SessionsError> {
        Err(SessionsError::MissingCapability {
            runtime: self.runtime().name().into(),
            capability: Capability::InputConsumption,
        })
    }
    fn assignment_evidence(
        &self,
        _session: &SessionRef,
        _now: jiff::Timestamp,
    ) -> Result<AssignmentEvidence, SessionsError> {
        Err(SessionsError::MissingCapability {
            runtime: self.runtime().name().into(),
            capability: Capability::AssignmentEvidence,
        })
    }
    fn list(&self, checkout: &Path) -> Result<Vec<Thread>, SessionsError>;
    fn start_or_wake(&self, assignment: &RuntimeAssignment<'_>) -> Result<Thread, SessionsError>;
    fn start(&self, request: &RuntimeAssignment<'_>) -> Result<Thread, SessionsError>;
    fn send(&self, session: &SessionRef, text: &str) -> Result<(), SessionsError>;
    fn read_state(&self, session: &SessionRef) -> Result<Thread, SessionsError>;
    fn read_conversation(&self, session: &SessionRef) -> Result<Conversation, SessionsError>;
    fn caller_identity(&self, env: &Env) -> Option<instrukt_core::interfaces::CallerIdentity> {
        self.lookup_caller(env)
    }
    fn link(&self, _a: &SessionRef, _b: &SessionRef, _remove: bool) -> Result<(), SessionsError> {
        Err(SessionsError::MissingCapability {
            runtime: self.runtime().name().into(),
            capability: Capability::Link,
        })
    }
    fn notify(&self, _reference: &str, _link: Option<&str>) -> Result<(), SessionsError> {
        Err(SessionsError::MissingCapability {
            runtime: self.runtime().name().into(),
            capability: Capability::Notify,
        })
    }
}

/// Missing optional native metadata is unknown; two supplied identities may conflict.
pub(crate) fn project_conflict(a: &Option<String>, b: &Option<String>) -> bool {
    a.as_ref().zip(b.as_ref()).is_some_and(|(a, b)| a != b)
}

/// Explicit runtime selection for every unattended path.
pub fn adapter(runtime: Runtime) -> Box<dyn RuntimeAdapter> {
    match runtime {
        Runtime::T3 => Box::new(T3::default()),
    }
}
