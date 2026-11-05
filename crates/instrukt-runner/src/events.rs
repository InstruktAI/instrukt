//! instrukt's event vocabulary and the event-log append the runner offers
//! (`docs/project/spec/domain/runner.md`, "Domain events emitted" and
//! "Domain events consumed"; `event-activation`, "Delivery and restart
//! boundary").
//!
//! The runner implements [`EventLog`] over the state database's `events`
//! table (per-consumer cursors, handled marks and idempotency keys are its
//! internals). Work's `instrukt todo stamp <slug> --finalize-ready` appends
//! [`Event::WorkflowFinalized`] through it, in the stamp's own transaction;
//! the CLI hands work the implementation (`crates/instrukt-cli/src/wiring.rs`,
//! `event_log`).

use instrukt_core::ids::{CommitSha, ProjectSlug, TodoSlug};
use serde::{Deserialize, Serialize};

/// One instrukt event. Its idempotency key is its whole payload, which is
/// exactly the key the runner spec lists for each event: appending an
/// event equal to one already logged is the same event.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "event")]
pub enum Event {
    /// A watched upstream ref changed (`git ls-remote` on a tick).
    #[serde(rename = "runner.upstream_changed")]
    UpstreamChanged {
        project: ProjectSlug,
        remote: String,
        r#ref: String,
        revision: CommitSha,
    },
    /// Local main is ahead of remote main and no candidate is pending.
    #[serde(rename = "runner.main_ahead")]
    MainAhead {
        project: ProjectSlug,
        local_main: CommitSha,
        remote_main: CommitSha,
    },
    /// The integrate candidate's CI result completed.
    #[serde(rename = "runner.ci_completed")]
    CiCompleted {
        project: ProjectSlug,
        candidate: CommitSha,
        result: CiResult,
    },
    /// The CI of remote canonical main's head commit completed red. It
    /// wakes the integrator to restore the canonical branch and authorizes
    /// nothing by itself.
    #[serde(rename = "runner.main_failing")]
    MainFailing {
        project: ProjectSlug,
        /// The failing head commit.
        head: CommitSha,
    },
    /// A finalized todo was handed off to integration: work's
    /// finalize-ready stamp, expressing workflow's handoff.
    #[serde(rename = "workflow.finalized")]
    WorkflowFinalized {
        project: ProjectSlug,
        todo: TodoSlug,
        /// The finalized commit the stamp names.
        revision: CommitSha,
    },
}

impl Event {
    /// Every name of the vocabulary, as subscriptions use them.
    pub const NAMES: [&'static str; 5] = [
        "runner.upstream_changed",
        "runner.main_ahead",
        "runner.ci_completed",
        "runner.main_failing",
        "workflow.finalized",
    ];

    /// The vocabulary name subscriptions use (`runner.ci_completed`, …).
    pub fn name(&self) -> &'static str {
        match self {
            Event::UpstreamChanged { .. } => "runner.upstream_changed",
            Event::MainAhead { .. } => "runner.main_ahead",
            Event::CiCompleted { .. } => "runner.ci_completed",
            Event::MainFailing { .. } => "runner.main_failing",
            Event::WorkflowFinalized { .. } => "workflow.finalized",
        }
    }

    /// The project the event concerns.
    pub fn project(&self) -> &ProjectSlug {
        match self {
            Event::UpstreamChanged { project, .. }
            | Event::MainAhead { project, .. }
            | Event::CiCompleted { project, .. }
            | Event::MainFailing { project, .. }
            | Event::WorkflowFinalized { project, .. } => project,
        }
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
