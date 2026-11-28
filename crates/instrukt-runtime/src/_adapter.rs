//! Runtime adapter, readiness, and optional archive capability traits.
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use instrukt_core::config::{AgentSelection, Env};
use instrukt_core::home::Home;
use instrukt_core::ids::SessionRef;
use instrukt_core::interfaces::{CallerIdentity, CallerIdentityLookup, ConfiguredInstance};
use instrukt_core::process::ProcessError;
use jiff::Timestamp;

use crate::cold_text::ColdTextSource;
use crate::{
    ArchiveOutcome, AssignmentEvidence, Capability, Conversation, DeliveryRealization,
    DispatchError, ExecutionRootMode, ExecutionRootStatus, FinishedThread, InputObservation,
    NativeDiagnostic, ObservedSnapshot, ProviderSnapshot, RuntimeError, RuntimeStart,
    SelectionOperation, SendEvidence, Thread, diagnose,
};

/// A runtime's readiness kept by the runner: refreshed on the runner's own wake,
/// never on a start.
pub trait Readiness: Send {
    fn refresh(
        &mut self,
        home: &Home,
        clock: &dyn Fn() -> Timestamp,
    ) -> Result<Arc<ObservedSnapshot>, RuntimeError>;

    fn stopper(&self) -> Box<dyn Fn() + Send + Sync>;
}

/// Optional non-live thread-archiving capability used solely by thread retention.
pub trait Archive: Send + Sync {
    fn finished_threads(&self, checkout: &Path) -> Result<Vec<FinishedThread>, RuntimeError>;
    fn archive(
        &self,
        observed: &FinishedThread,
        now: Timestamp,
    ) -> Result<ArchiveOutcome, RuntimeError>;
}

/// Runtime-neutral live spine and optional capability accessors.
pub trait RuntimeAdapter: CallerIdentityLookup + Send + Sync {
    fn runtime_name(&self) -> &'static str;

    fn cold_text_source(
        &self,
        _home: &Home,
        _env: &Env,
    ) -> Result<Box<dyn ColdTextSource>, RuntimeError> {
        Err(RuntimeError::MissingCapability {
            runtime: self.runtime_name().into(),
            capability: Capability::ColdTextSource,
        })
    }

    fn diagnostic(&self, operation: &str, error: &RuntimeError) -> NativeDiagnostic {
        diagnose(self.runtime_name(), operation, error, None)
    }

    /// Whether the adapter can prove a send did not reach acceptance.
    fn send_certainly_rejected(&self, _session: &SessionRef, error: &RuntimeError) -> bool {
        matches!(error, RuntimeError::Process(ProcessError::Spawn { .. }))
    }

    /// One provider snapshot read now, with a token of its own: for a command
    /// that reads readiness once, never for a start.
    fn provider_snapshot(&self, home: &Home) -> Result<ProviderSnapshot, RuntimeError>;

    /// The readiness a runner keeps for this runtime, if it has any.
    fn readiness(&self) -> Option<Box<dyn Readiness>> {
        None
    }

    /// The last provider snapshot published in this process, if any.
    fn last_snapshot(&self) -> Option<Arc<ObservedSnapshot>> {
        None
    }

    fn configured_agent_runtimes(
        &self,
        _home: &Home,
        _env: &Env,
    ) -> Result<Vec<ConfiguredInstance>, RuntimeError> {
        Err(RuntimeError::MissingCapability {
            runtime: self.runtime_name().into(),
            capability: Capability::ConfiguredRuntimes,
        })
    }

    fn selection_realization(&self, operation: SelectionOperation) -> String;

    fn native_selection(&self, selection: &AgentSelection) -> AgentSelection;

    fn launch_scope(&self, home: &Home) -> String;

    fn resolve_launch(
        &self,
        request: &RuntimeStart,
        existing: Option<&SessionRef>,
    ) -> Result<RuntimeStart, RuntimeError>;

    fn accepted_start(&self, request: &RuntimeStart) -> Result<bool, RuntimeError>;

    fn create_thread(&self, request: &RuntimeStart) -> Result<SessionRef, RuntimeError>;

    fn verify_start(&self, request: &RuntimeStart, thread: &Thread) -> Result<(), RuntimeError>;

    fn verify_placement(&self, request: &RuntimeStart, thread: &Thread)
    -> Result<(), RuntimeError>;

    fn verify_thread_project(
        &self,
        checkout: &Path,
        thread: &Thread,
        key: Option<&str>,
    ) -> Result<(), RuntimeError>;

    fn execution_root(
        &self,
        root: &Path,
        mode: ExecutionRootMode,
    ) -> Result<ExecutionRootStatus, RuntimeError>;

    fn delivery_realization(&self) -> DeliveryRealization;

    fn supports(&self, capability: Capability) -> bool;

    fn observe_send(
        &self,
        session: &SessionRef,
        text: &str,
        command_id: Option<&str>,
    ) -> Result<SendEvidence, RuntimeError>;

    fn observe_input(
        &self,
        _session: &SessionRef,
        _text: &str,
        _now: Timestamp,
    ) -> Result<InputObservation, RuntimeError> {
        Err(RuntimeError::MissingCapability {
            runtime: self.runtime_name().into(),
            capability: Capability::InputConsumption,
        })
    }

    fn assignment_evidence(
        &self,
        _session: &SessionRef,
        _now: Timestamp,
    ) -> Result<AssignmentEvidence, RuntimeError> {
        Err(RuntimeError::MissingCapability {
            runtime: self.runtime_name().into(),
            capability: Capability::AssignmentEvidence,
        })
    }

    fn list(&self, checkout: &Path) -> Result<Vec<Thread>, RuntimeError>;

    fn dispatch_send(
        &self,
        session: &SessionRef,
        command_id: &str,
        text: &str,
        before_write: &mut (dyn FnMut() -> Result<(), RuntimeError> + Send),
    ) -> Result<(), DispatchError>;

    fn send(&self, session: &SessionRef, text: &str) -> Result<(), RuntimeError>;

    fn read_state(&self, session: &SessionRef) -> Result<Thread, RuntimeError>;

    /// Like [`Self::read_state`], bounded to `timeout` for caller wait budgets.
    fn read_state_within(
        &self,
        session: &SessionRef,
        _timeout: Duration,
    ) -> Result<Thread, RuntimeError> {
        self.read_state(session)
    }

    fn read_conversation(&self, session: &SessionRef) -> Result<Conversation, RuntimeError>;

    fn caller_identity(&self, env: &Env) -> Option<CallerIdentity> {
        self.lookup_caller(env)
    }

    fn link(&self, _a: &SessionRef, _b: &SessionRef, _remove: bool) -> Result<(), RuntimeError> {
        Err(RuntimeError::MissingCapability {
            runtime: self.runtime_name().into(),
            capability: Capability::Link,
        })
    }

    fn notify(&self, _reference: &str, _link: Option<&str>) -> Result<(), RuntimeError> {
        Err(RuntimeError::MissingCapability {
            runtime: self.runtime_name().into(),
            capability: Capability::Notify,
        })
    }

    fn archive_capability(&self) -> Result<&dyn Archive, RuntimeError> {
        Err(RuntimeError::MissingCapability {
            runtime: self.runtime_name().into(),
            capability: Capability::Archive,
        })
    }

    fn finished_threads(&self, checkout: &Path) -> Result<Vec<FinishedThread>, RuntimeError> {
        self.archive_capability()?.finished_threads(checkout)
    }

    fn archive(
        &self,
        observed: &FinishedThread,
        now: Timestamp,
    ) -> Result<ArchiveOutcome, RuntimeError> {
        self.archive_capability()?.archive(observed, now)
    }

    /// Revokes any long-lived session lease held by this adapter on shutdown.
    fn shutdown(&self) {}
}
