//! Typed producer facts and the open canonical event boundary.
//!
//! The acquisition enum is a producer model, not the received-event vocabulary.
//! Received envelopes preserve external types and unknown facts unchanged.

mod _catalog;
mod _envelope;
mod _producer;
mod _reference;
#[cfg(test)]
mod _tests;

use instrukt_core::ids::{CommitSha, ProjectSlug, RoleName, SessionRef, TodoSlug};
use jiff::Timestamp;
use serde::{Deserialize, Serialize};

pub use _catalog::{
    CatalogError, DataField, DataFieldType, DataValueKind, EventCatalog, EventDeclaration,
    LifecycleState, StateDeclaration, StateStanding,
};
pub use _envelope::{EmissionContext, EnvelopeError, EventEnvelope, EventTime, occurrence_id};
pub use _producer::{DeadLettered, ProducerEvent};
pub use _reference::{EventHome, EventHomeOwner, EventRef, HomeKind, ReferenceKind};
pub use instrukt_core::event_type::{TypePattern, TypePatternError};

// One declaration defines the producer's Rust fields, wire name and catalog.
// A subject named `Any` declares any reference kind. Related kinds default to
// `Session`; an entry lists its own in brackets after the subject.
macro_rules! acquisition_events {
    ($($variant:ident, $name:literal, $description:literal, $delivery:ident, $subject:ident $([$($related:ident),+])? {
        $($field:ident: $ty:ty),* $(,)?
    });* $(;)?) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(tag = "event")]
        pub enum Event {
            $(#[doc = $description]
              #[serde(rename = $name)]
              $variant { $($field: $ty),* }),*
        }

        impl Event {
            pub const NAMES: &'static [&'static str] = &[$($name),*];

            pub fn name(&self) -> &'static str {
                match self { $(Self::$variant { .. } => $name),* }
            }

            pub fn declaration(&self) -> EventDeclaration {
                match self { $(Self::$variant { .. } => acquisition_events!(@declaration $name, $description, $delivery, $subject, [$($($related),+)?], $($field: $ty),*)),* }
            }

            pub fn declarations() -> Vec<EventDeclaration> {
                vec![$(acquisition_events!(@declaration $name, $description, $delivery, $subject, [$($($related),+)?], $($field: $ty),*)),*]
            }
        }
    };
    (@declaration $name:literal, $description:literal, $delivery:ident, $subject:ident, [$($related:ident),*], $($field:ident: $ty:ty),*) => {
        EventDeclaration {
            name: $name.into(), delivery_kind: instrukt_core::event_type::DeliveryKind::$delivery, version: 1, description: $description.into(),
            subject_kind: acquisition_events!(@subject $subject), home_kinds: vec![HomeKind::Project],
            data_fields: vec![$(DataField::of::<$ty>(stringify!($field).trim_start_matches("r#"))),*],
            related_kinds: acquisition_events!(@related $($related),*), state: None,
        }
    };
    (@related) => { vec![ReferenceKind::Session] };
    (@related $($related:ident),+) => { vec![$(ReferenceKind::$related),+] };
    (@subject Any) => { None };
    (@subject $subject:ident) => { Some(ReferenceKind::$subject) };
}

acquisition_events! {
    UpstreamChanged, "runner.upstream_changed", "A watched upstream ref changed.", State, RepositoryRef {
        project: ProjectSlug, remote: String, r#ref: String, revision: CommitSha
    };
    CiCompleted, "runner.ci_completed", "The integrate candidate CI completed, including a no-CI observation.", State, Commit {
        project: ProjectSlug, candidate: CommitSha, result: CiResult
    };
    DependencyFailing, "runner.dependency_failing", "A check run on a current dependency-update head failed.", Occurrence, Commit {
        project: ProjectSlug, branch: String, head: CommitSha, check_run_id: u64, check_name: String, conclusion: Option<String>
    };
    MainFailing, "runner.main_failing", "Remote canonical main head completed CI red.", State, RepositoryRef {
        project: ProjectSlug, head: CommitSha
    };
    WorkflowFinalized, "workflow.finalized", "A finalize-ready todo was handed off to integration.", Occurrence, Todo {
        project: ProjectSlug, todo: TodoSlug, revision: CommitSha
    };
    ReviewNeeded, "runner.review_needed", "A commit is owed an independent review: the current head of an eligible pull request, or an integration candidate.", State, Any {
        project: ProjectSlug, pull_request: Option<u64>, head_branch: Option<String>, base_branch: Option<String>, head: CommitSha, base: CommitSha, request: u64
    };
    AttentionExecutionStalled, "runner.attention.execution_stalled", "The session holding a todo's execution claim stopped, failed or is gone and its recovery did not move the work, or no session could be started from the model order available.", Occurrence, Todo {
        project: ProjectSlug, todo: TodoSlug, session: Option<SessionRef>, workflow_state: String, cause: StalledCause, model_order: Vec<String>
    };
    AttentionInputWaiting, "runner.attention.input_waiting", "The session holding a todo's execution claim began a wait for input while the todo does not wait on a person.", Occurrence, Todo {
        project: ProjectSlug, todo: TodoSlug, session: SessionRef, workflow_state: String, wait: u64
    };
    AttentionHoldPlaced, "runner.attention.hold_placed", "A change made on this computer placed a dispatch hold on a todo.", Occurrence, Todo {
        project: ProjectSlug, todo: TodoSlug, source: AttentionHoldSource, reason: String, placed_at: Timestamp
    };
    AttentionHoldReleased, "runner.attention.hold_released", "A change made on this computer removed a todo's dispatch hold.", Occurrence, Todo {
        project: ProjectSlug, todo: TodoSlug, source: AttentionHoldSource, placed_at: Timestamp
    };
    AttentionAttemptsFailing, "runner.attention.attempts_failing", "A role's attempt on a todo failed where an earlier attempt of that role on that todo already failed.", Occurrence, Todo {
        project: ProjectSlug, todo: TodoSlug, role: RoleName, attempt: u64, reason: String
    };
    AttentionFleetQuiet, "runner.attention.fleet_quiet", "The project's attention episode ended: its last active todo that is unheld and not a structural holder left active work or was held.", Occurrence, Todo {
        project: ProjectSlug, todo: TodoSlug, opened_by: String
    };
    AttentionRaised, "runner.attention.raised", "An agent raised a condition about active todos of the project it works in.", Occurrence, Todo [Todo, Session] {
        project: ProjectSlug, todo: TodoSlug, related: Vec<TodoSlug>, obligation: String, raise: String
    };
}

impl Event {
    pub fn project(&self) -> &ProjectSlug {
        match self {
            Self::UpstreamChanged { project, .. }
            | Self::CiCompleted { project, .. }
            | Self::MainFailing { project, .. }
            | Self::DependencyFailing { project, .. }
            | Self::WorkflowFinalized { project, .. }
            | Self::ReviewNeeded { project, .. }
            | Self::AttentionExecutionStalled { project, .. }
            | Self::AttentionInputWaiting { project, .. }
            | Self::AttentionHoldPlaced { project, .. }
            | Self::AttentionHoldReleased { project, .. }
            | Self::AttentionAttemptsFailing { project, .. }
            | Self::AttentionFleetQuiet { project, .. }
            | Self::AttentionRaised { project, .. } => project,
        }
    }

    /// The declared source occurrence key, independent of JSON field order,
    /// enrichment, source computer, timestamps or transport positions.
    pub fn occurrence_id(&self) -> String {
        let mut key = vec![self.name(), self.project().as_str()];
        match self {
            Self::UpstreamChanged {
                remote,
                r#ref,
                revision,
                ..
            } => key.extend([remote.as_str(), r#ref.as_str(), revision.as_str()]),
            Self::CiCompleted {
                candidate, result, ..
            } => key.extend([candidate.as_str(), result.as_str()]),
            Self::MainFailing { head, .. } => key.push(head.as_str()),
            Self::DependencyFailing {
                branch,
                head,
                check_run_id,
                conclusion,
                ..
            } => {
                return occurrence_id(&[
                    self.name(),
                    self.project().as_str(),
                    branch,
                    head.as_str(),
                    &check_run_id.to_string(),
                    conclusion.as_deref().unwrap_or(""),
                ]);
            }
            Self::WorkflowFinalized { todo, revision, .. } => {
                key.extend([todo.as_str(), revision.as_str()])
            }
            Self::ReviewNeeded { head, request, .. } => {
                return review_request_id(self.project(), head, *request);
            }
            Self::AttentionExecutionStalled {
                todo,
                session,
                workflow_state,
                model_order,
                ..
            } => {
                let started = match session {
                    Some(session) => vec![session.to_string()],
                    None => model_order.clone(),
                };
                key.push(todo.as_str());
                key.extend(started.iter().map(String::as_str));
                key.push(workflow_state.as_str());
                return occurrence_id(&key);
            }
            Self::AttentionInputWaiting {
                todo,
                session,
                wait,
                ..
            } => {
                return occurrence_id(&[
                    self.name(),
                    self.project().as_str(),
                    todo.as_str(),
                    &session.to_string(),
                    &wait.to_string(),
                ]);
            }
            Self::AttentionHoldPlaced {
                todo,
                source,
                placed_at,
                ..
            }
            | Self::AttentionHoldReleased {
                todo,
                source,
                placed_at,
                ..
            } => {
                return occurrence_id(&[
                    self.name(),
                    self.project().as_str(),
                    todo.as_str(),
                    &placed_at.to_string(),
                    source.as_str(),
                ]);
            }
            Self::AttentionAttemptsFailing {
                todo,
                role,
                attempt,
                ..
            } => {
                return occurrence_id(&[
                    self.name(),
                    self.project().as_str(),
                    todo.as_str(),
                    role.as_str(),
                    &attempt.to_string(),
                ]);
            }
            Self::AttentionFleetQuiet { opened_by, .. } => key.push(opened_by.as_str()),
            Self::AttentionRaised { raise, .. } => key.push(raise.as_str()),
        }
        occurrence_id(&key)
    }
}

/// The occurrence identity of review request `request` for `head` of
/// `project`: the project, the commit and the request number
/// (`pull-request-review`, "Review requests").
pub fn review_request_id(project: &ProjectSlug, head: &CommitSha, request: u64) -> String {
    occurrence_id(&[
        "runner.review_needed",
        project.as_str(),
        head.as_str(),
        &request.to_string(),
    ])
}

/// A completed CI result for one candidate commit. How GitHub's combined
/// status and check-run conclusions map onto it is the runner's
/// event-source translation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CiResult {
    Success,
    Failure,
    /// The project has no CI: the canonical branch's head carries no
    /// status and no check run, and the candidate has nothing reported
    /// either (`integration-gate`, "Project without CI"). The integrator
    /// verifies the candidate with the project's local test action.
    NoCi,
}

/// How the execution behind a stalled todo ended up stalled
/// (`fleet-attention`, Attention conditions).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StalledCause {
    /// The session stopped and the recovery its contract provides did not
    /// move the work.
    Stopped,
    /// The session's turn ended failed.
    Failed,
    /// The runtime no longer knows the session.
    Gone,
    /// No session could be started from the model order available.
    NeverStarted,
}

/// The source of a dispatch hold, as fleet attention carries it. Work's own
/// hold source maps onto it (`dependencies-and-holds`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttentionHoldSource {
    Manual,
    Automatic,
    #[serde(rename = "github")]
    GitHub,
}

impl AttentionHoldSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Automatic => "automatic",
            Self::GitHub => "github",
        }
    }
}

/// Whether an append added a new event or met its idempotency key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppendOutcome {
    Appended,
    AlreadyLogged,
}

/// The event log could not be read or written.
#[derive(Debug, thiserror::Error)]
pub enum EventLogError {
    #[error(transparent)]
    Envelope(#[from] EnvelopeError),
    #[error(transparent)]
    Machine(#[from] instrukt_core::machine::MachineError),
    #[error(transparent)]
    State(#[from] crate::state::StateError),
}

/// Appending to the runner's event log, idempotently by the event's key,
/// inside the caller's write transaction: the event commits with the change
/// it records, or not at all.
pub trait EventLog: Send + Sync {
    fn append(
        &self,
        tx: &instrukt_core::state_db::rusqlite::Transaction<'_>,
        event: &Event,
    ) -> Result<AppendOutcome, EventLogError>;
}
