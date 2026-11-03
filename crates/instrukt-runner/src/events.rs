//! instrukt's event vocabulary and the event-log append the runner offers
//! (`docs/local/spec/domain/runner.md`, "Domain events").

use instrukt_core::ids::{CommitSha, ProjectSlug, TodoSlug};
use serde::{Deserialize, Serialize};

/// One instrukt event. Its idempotency key is its whole payload: appending
/// an event equal to one already logged is the same event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event")]
pub enum Event {
    /// A watched upstream ref changed.
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
    /// A finalized todo was handed off to integration.
    #[serde(rename = "workflow.finalized")]
    WorkflowFinalized {
        project: ProjectSlug,
        todo: TodoSlug,
        revision: CommitSha,
    },
}

impl Event {
    /// The vocabulary name subscriptions use (`runner.ci_completed`, …).
    pub fn name(&self) -> &'static str {
        match self {
            Event::UpstreamChanged { .. } => "runner.upstream_changed",
            Event::MainAhead { .. } => "runner.main_ahead",
            Event::CiCompleted { .. } => "runner.ci_completed",
            Event::WorkflowFinalized { .. } => "workflow.finalized",
        }
    }
}

/// A completed CI result for one candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CiResult {
    Success,
    Failure,
}

/// Whether an append added a new event or met its idempotency key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppendOutcome {
    Appended,
    AlreadyLogged,
}

/// The event log could not be written.
#[derive(Debug, thiserror::Error)]
#[error("event log: {0}")]
pub struct EventLogError(pub String);

/// Appending to the runner's event log in `state/runner/`. The runner
/// implements it; work's finalize-ready stamp uses it to record
/// `workflow.finalized`.
pub trait EventLog: Send + Sync {
    fn append(&self, event: &Event) -> Result<AppendOutcome, EventLogError>;
}
