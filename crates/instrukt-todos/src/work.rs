//! The work context: the local-leading work record and its GitHub
//! reflection (`docs/local/spec/domain/work.md`).
//!
//! [`Work`] is the one capability: it keeps each project's todos in
//! `~/.instrukt/state/work/<project>/` under a per-computer writer lock,
//! validates every change against the whole record before it commits,
//! reflects committed changes to the project's GitHub repository (issues,
//! native artifact versions, a record envelope comment carrying the
//! committed revision and stamp ledger, the linked pull request), and
//! pulls newer remote revisions in with conditional requests. A write the
//! local record accepted is never undone by a failed reflection.
//!
//! It serves `instrukt todo` (its methods), the runner (`Pickup`,
//! `Claims`, `SessionBindings`, `Reflection`, `TaskFirings`,
//! `AutomaticHolds`, `RetentionPurge`), workflow
//! ([`crate::evidence::WorkEvidence`]) and core ([`WorkExistence`] for
//! `TodoExistence`).

mod _diff;
mod _envelope;
mod _error;
mod _github;
mod _model;
mod _ops;
mod _ports;
mod _reflect;
mod _session;
mod _stamps;
mod _store;

use std::sync::Arc;

use instrukt_core::config::{Configuration, Env};
use instrukt_core::home::Home;
use instrukt_runner::interfaces::ProjectRef;
use jiff::Timestamp;

pub use _diff::DiffLine;
pub use _error::WorkError;
pub use _github::GitHubError;
pub use _model::{
    Artifact, Binding, Claim, Delivery, ReleasedHold, Removal, TodoId, TodoRecord, TodoRef, Version,
};
pub use _ops::{
    CreateRequest, ListEntry, Listing, Outcome, PrerequisiteView, Selection, TodoView,
    UpdateReport, UpdateRequest,
};
pub use _ports::WorkExistence;
pub use _session::{GitHubRead, PendingReason, ReflectionStatus, describe};
pub use _stamps::StampRequest;

/// The clock work reads, injected by the composition root.
pub type Clock = Arc<dyn Fn() -> Timestamp + Send + Sync>;

/// The work record of this computer.
pub struct Work {
    home: Home,
    env: Env,
    clock: Clock,
}

impl Work {
    /// Work over `home`, reading credentials and the caller's session from
    /// `env` and time from `clock`.
    pub fn new(home: Home, env: Env, clock: Clock) -> Self {
        Self { home, env, clock }
    }

    /// The project rooted at `root`, as its project layer names it.
    pub fn project_at(&self, root: &std::path::Path) -> Result<ProjectRef, WorkError> {
        let config = Configuration::load(&self.home, Some(root))?;
        let slug = config.project()?.ok_or(WorkError::NoProject)?;
        Ok(ProjectRef {
            slug,
            root: root.to_path_buf(),
        })
    }

    pub(crate) fn home(&self) -> &Home {
        &self.home
    }

    pub(crate) fn env(&self) -> &Env {
        &self.env
    }

    pub(crate) fn now(&self) -> Timestamp {
        (self.clock)()
    }
}
