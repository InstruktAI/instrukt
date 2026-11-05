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

use instrukt_core::ids::{CommitSha, ProjectSlug, TodoSlug};
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
macro_rules! acquisition_events {
    ($($variant:ident, $name:literal, $description:literal, $subject:ident {
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
                match self { $(Self::$variant { .. } => acquisition_events!(@declaration $name, $description, $subject, $($field: $ty),*)),* }
            }

            pub fn declarations() -> Vec<EventDeclaration> {
                vec![$(acquisition_events!(@declaration $name, $description, $subject, $($field: $ty),*)),*]
            }
        }
    };
    (@declaration $name:literal, $description:literal, $subject:ident, $($field:ident: $ty:ty),*) => {
        EventDeclaration {
            name: $name.into(), version: 1, description: $description.into(),
            subject_kind: Some(ReferenceKind::$subject), home_kinds: vec![HomeKind::Project],
            data_fields: vec![$(DataField::of::<$ty>(stringify!($field).trim_start_matches("r#"))),*],
            related_kinds: vec![ReferenceKind::Session], state: None,
        }
    };
}

acquisition_events! {
    UpstreamChanged, "runner.upstream_changed", "A watched upstream ref changed.", Commit {
        project: ProjectSlug, remote: String, r#ref: String, revision: CommitSha
    };
    MainAhead, "runner.main_ahead", "Local main is ahead of remote main with no candidate pending.", Commit {
        project: ProjectSlug, local_main: CommitSha, remote_main: CommitSha
    };
    CiCompleted, "runner.ci_completed", "The integrate candidate CI completed, including a no-CI observation.", Commit {
        project: ProjectSlug, candidate: CommitSha, result: CiResult
    };
    MainFailing, "runner.main_failing", "Remote canonical main head completed CI red.", Commit {
        project: ProjectSlug, head: CommitSha
    };
    WorkflowFinalized, "workflow.finalized", "A finalize-ready todo was handed off to integration.", Todo {
        project: ProjectSlug, todo: TodoSlug, revision: CommitSha
    };
}

impl Event {
    pub fn project(&self) -> &ProjectSlug {
        match self {
            Self::UpstreamChanged { project, .. }
            | Self::MainAhead { project, .. }
            | Self::CiCompleted { project, .. }
            | Self::MainFailing { project, .. }
            | Self::WorkflowFinalized { project, .. } => project,
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
            Self::MainAhead {
                local_main,
                remote_main,
                ..
            } => key.extend([local_main.as_str(), remote_main.as_str()]),
            Self::CiCompleted {
                candidate, result, ..
            } => key.extend([candidate.as_str(), result.as_str()]),
            Self::MainFailing { head, .. } => key.push(head.as_str()),
            Self::WorkflowFinalized { todo, revision, .. } => {
                key.extend([todo.as_str(), revision.as_str()])
            }
        }
        occurrence_id(&key)
    }
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
