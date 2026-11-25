//! Structured runtime primitives; the selected runtime owns conversations and workspaces.
mod _diagnostics;
mod _t3;
mod _t3_completed;
mod _t3_conversations;
mod _t3_diagnostics;
mod _t3_fingerprint;
pub(crate) mod _t3_home;
mod _t3_notifications;
mod _t3_readiness;
pub mod conversations;

use std::path::{Path, PathBuf};

use instrukt_core::config::{Env, Runtime};
use instrukt_core::ids::{ProviderInstanceId, SessionRef};
use serde::{Deserialize, Serialize};

use crate::sessions::{SessionState, SessionsError};
pub use _diagnostics::{NativeCause, NativeDiagnostic};
pub use _t3::T3;
pub(crate) use _t3_conversations::{reports_background_command, without_captured_context};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorktreeRequest {
    pub from: String,
    pub base: String,
    pub branch: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LaunchRuntimeMode {
    ApprovalRequired,
    AutoAcceptEdits,
    Auto,
    FullAccess,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LaunchInteractionMode {
    Plan,
    Default,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedLaunchModes {
    pub runtime: LaunchRuntimeMode,
    pub interaction: LaunchInteractionMode,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativeLaunch {
    pub command_id: String,
    pub thread_id: String,
    pub project_id: String,
    pub existing_worktree: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeStart {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch_modes: Option<ResolvedLaunchModes>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_launch: Option<NativeLaunch>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub preferences: Vec<crate::agents::AgentChoice>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection: Option<instrukt_core::config::AgentSelection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection_evidence: Option<String>,
    pub checkout: PathBuf,
    pub key: String,
    pub title: String,
    pub brief: String,
    pub worktree: Option<WorktreeRequest>,
    /// The agent profiles of the admitted generation the start was prepared
    /// in; a request without them reads `agents.yaml` when it is started.
    #[serde(skip)]
    pub profiles: Profiles,
}

/// The agent profiles a start carries in memory. They are never stored, so
/// they take no part in comparing two requests.
#[derive(Debug, Clone, Default)]
pub struct Profiles(pub Option<std::sync::Arc<instrukt_core::config::Agents>>);
impl PartialEq for Profiles {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}
impl Eq for Profiles {}

/// Readable labels are presentation, independent of durable object identity.
pub fn thread_title(label: &str, _key: &str) -> String {
    label.to_owned()
}

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

/// What the runtime actually established about this thread's identity.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Presence {
    #[default]
    Present,
    Archived,
    Absent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Thread {
    pub session: SessionRef,
    pub project_id: Option<String>,
    pub title: Option<String>,
    pub semantic_key: Option<String>,
    pub state: SessionState,
    #[serde(default)]
    pub presence: Presence,
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

/// A thread the runtime lists for a project and reports finished, as thread
/// retention reads it (`project/instrukt/spec/feature/runner/session-continuity`,
/// `finished_threads`). A time the runtime does not report is `None`, never
/// substituted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinishedThread {
    pub session: SessionRef,
    /// When the thread finished.
    pub finished_at: Option<jiff::Timestamp>,
    /// Its latest activity, a person reading or writing to it included.
    pub latest_activity: Option<jiff::Timestamp>,
    /// Not running, queued, snoozed or waiting on a person's input or
    /// approval.
    pub quiet: bool,
    /// A person pinned it.
    pub pinned: bool,
}

/// What an archive request found and did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveOutcome {
    /// The thread is archived.
    Archived,
    /// It was archived already, which is done.
    AlreadyArchived,
    /// The observation it was chosen from no longer holds; nothing was
    /// archived.
    Changed,
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
    ConversationSource,
    CreateWorktree,
    AttachWorktree,
    Link,
    Notify,
    AgentSelection,
    InputConsumption,
    AssignmentEvidence,
    ConfiguredRuntimes,
    /// Listing finished threads and archiving them (thread retention).
    Archive,
}
impl std::fmt::Display for Capability {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::ConversationSource => "durable conversation changes",
            Self::CreateWorktree => "worktree creation",
            Self::AttachWorktree => "worktree attachment",
            Self::Link => "link",
            Self::Notify => "notification",
            Self::AgentSelection => "agent/model selection",
            Self::InputConsumption => "input consumption observation",
            Self::AssignmentEvidence => "assignment evidence recovery",
            Self::ConfiguredRuntimes => "configured agent runtimes",
            Self::Archive => "thread archiving",
        })
    }
}

pub struct FollowupAssignment<'a> {
    pub bindings: Option<&'a dyn crate::interfaces::SessionBindings>,
    pub original_request: Option<&'a RuntimeStart>,
    pub invocation: &'a crate::keyed::RuntimeInvocation,
    pub request: &'a RuntimeStart,
    pub session: &'a SessionRef,
    pub worktree_path: Option<&'a Path>,
    pub project: Option<&'a str>,
    pub now: jiff::Timestamp,
}

/// Operations use structured facts; no user input is executable command text.
pub struct RuntimeAssignment<'a> {
    pub bindings: Option<&'a dyn crate::interfaces::SessionBindings>,
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

#[derive(Debug, thiserror::Error)]
pub enum DispatchError {
    #[error(transparent)]
    BeforeFrame(SessionsError),
    #[error(transparent)]
    AfterFrame(SessionsError),
}

impl DispatchError {
    pub(crate) fn into_inner(self) -> SessionsError {
        match self {
            Self::BeforeFrame(error) | Self::AfterFrame(error) => error,
        }
    }
}

/// The runtime's reconciliation of one exact send attempt.
pub enum SendEvidence {
    Accepted,
    Rejected,
    IdleWithoutInput,
    Unresolved,
    Presence(Presence),
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

/// A provider snapshot with the time it was observed.
pub struct ObservedSnapshot {
    pub snapshot: ProviderSnapshot,
    pub at: jiff::Timestamp,
}

/// A runtime's readiness kept by the runner: refreshed on the runner's own
/// wake, never on a start.
pub trait Readiness: Send {
    /// Reads the provider snapshot and publishes it for starts to read,
    /// observed at the time on `clock` when the read returned.
    fn refresh(
        &mut self,
        home: &instrukt_core::home::Home,
        clock: &dyn Fn() -> jiff::Timestamp,
    ) -> Result<std::sync::Arc<ObservedSnapshot>, SessionsError>;
    /// What revokes the token this readiness keeps when the process stops:
    /// one attempt, bounded, its failure logged.
    fn stopper(&self) -> Box<dyn Fn() + Send + Sync>;
}

/// Adapter-translated provider facts; expiry and selection belong to runner policy.
pub struct ProviderSnapshot {
    pub providers: Vec<ProviderFacts>,
    pub native_picks: Vec<String>,
}
pub struct ProviderFacts {
    pub instance: ProviderInstanceId,
    pub kind: Option<instrukt_core::config::HarnessKind>,
    pub unavailable_reason: Option<String>,
    pub models: Vec<ReportedModel>,
    pub usage: Vec<UsageWindow>,
}
pub struct ReportedModel {
    pub name: String,
    pub aliases: Vec<String>,
}
pub struct UsageWindow {
    pub id: String,
    pub used_percent: f64,
    pub resets_at: Option<jiff::Timestamp>,
    /// The model family the window applies to, lowercase, as the runtime
    /// reports it (`fable` for a Claude window scoped to Fable); `None` for
    /// a window that applies to all of the profile's models.
    pub model_family: Option<String>,
}
#[derive(Debug, Clone, Copy)]
pub enum SelectionOperation {
    ThreadStart,
    NativeDelegation,
}

pub trait RuntimeAdapter: instrukt_core::interfaces::CallerIdentityLookup {
    fn conversation_source(
        &self,
        _home: &instrukt_core::home::Home,
        _env: &Env,
    ) -> Result<Box<dyn conversations::ConversationSource>, SessionsError> {
        Err(SessionsError::MissingCapability {
            runtime: self.runtime().name().into(),
            capability: Capability::ConversationSource,
        })
    }
    fn diagnostic(&self, operation: &str, error: &SessionsError) -> NativeDiagnostic {
        _diagnostics::diagnose(self.runtime().name(), operation, error, None)
    }
    /// Whether the adapter can prove a send did not reach acceptance.
    fn send_certainly_rejected(&self, _session: &SessionRef, error: &SessionsError) -> bool {
        matches!(
            error,
            SessionsError::Process(instrukt_core::process::ProcessError::Spawn { .. })
        )
    }
    fn runtime(&self) -> Runtime;
    /// One provider snapshot read now, with a token of its own: for a
    /// command that reads readiness once, never for a start.
    fn provider_snapshot(
        &self,
        home: &instrukt_core::home::Home,
    ) -> Result<ProviderSnapshot, SessionsError>;
    /// The readiness a runner keeps for this runtime, if it has any.
    fn readiness(&self) -> Option<Box<dyn Readiness>> {
        None
    }
    /// The last provider snapshot published in this process, if any.
    fn last_snapshot(&self) -> Option<std::sync::Arc<ObservedSnapshot>> {
        None
    }
    /// The enabled agent instances this runtime has configured, each with
    /// the home the adapter resolved from the instance's settings and
    /// environment: read now, with a token of its own. Interpreting a
    /// runtime's home is the adapter's alone.
    fn configured_agent_runtimes(
        &self,
        _home: &instrukt_core::home::Home,
        _env: &Env,
    ) -> Result<Vec<instrukt_core::interfaces::ConfiguredInstance>, SessionsError> {
        Err(SessionsError::MissingCapability {
            runtime: self.runtime().name().into(),
            capability: Capability::ConfiguredRuntimes,
        })
    }
    fn selection_realization(&self, operation: SelectionOperation) -> String;
    fn native_selection(
        &self,
        selection: &instrukt_core::config::AgentSelection,
    ) -> instrukt_core::config::AgentSelection;
    fn launch_scope(&self, home: &instrukt_core::home::Home) -> String;
    fn resolve_launch(
        &self,
        request: &RuntimeStart,
        existing: Option<&SessionRef>,
    ) -> Result<RuntimeStart, SessionsError>;
    fn accepted_start(&self, request: &RuntimeStart) -> Result<bool, SessionsError>;
    fn create_thread(&self, request: &RuntimeStart) -> Result<SessionRef, SessionsError>;
    fn verify_start(&self, request: &RuntimeStart, thread: &Thread) -> Result<(), SessionsError>;
    fn verify_placement(
        &self,
        request: &RuntimeStart,
        thread: &Thread,
    ) -> Result<(), SessionsError>;
    fn delivery_realization(&self) -> DeliveryRealization;
    fn supports(&self, capability: Capability) -> bool;
    fn observe_send(
        &self,
        session: &SessionRef,
        text: &str,
        command_id: Option<&str>,
    ) -> Result<SendEvidence, SessionsError>;
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
    /// The input that opened the turn the session has in progress at the
    /// moment of asking, as the runtime records it; `None` when it has none.
    /// It names the session's current turn, not the turn that started the
    /// process asking: a command left running past its turn observes the turn
    /// then in progress. An error means the runtime cannot say, which callers
    /// treat as no answer.
    fn active_turn_input(&self, _session: &SessionRef) -> Result<Option<String>, SessionsError> {
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
    fn dispatch_send(
        &self,
        session: &SessionRef,
        command_id: &str,
        text: &str,
        before_write: &mut dyn FnMut() -> Result<(), SessionsError>,
    ) -> Result<(), DispatchError>;
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
    /// The threads the runtime lists for the project at `checkout` and
    /// reports finished, with their finishing time, latest activity,
    /// quietness and pin.
    fn finished_threads(&self, _checkout: &Path) -> Result<Vec<FinishedThread>, SessionsError> {
        Err(SessionsError::MissingCapability {
            runtime: self.runtime().name().into(),
            capability: Capability::Archive,
        })
    }
    /// Archives the thread `observed` names only while that observation still
    /// holds at `now`, the caller's clock: present, finished, quiet, not
    /// pinned and without activity after its observed latest activity.
    fn archive(
        &self,
        _observed: &FinishedThread,
        _now: jiff::Timestamp,
    ) -> Result<ArchiveOutcome, SessionsError> {
        Err(SessionsError::MissingCapability {
            runtime: self.runtime().name().into(),
            capability: Capability::Archive,
        })
    }
}

/// Explicit runtime selection for every unattended path.
pub fn adapter(runtime: Runtime) -> Box<dyn RuntimeAdapter> {
    match runtime {
        Runtime::T3 => Box::new(T3::default()),
    }
}
