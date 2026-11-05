//! The work context's errors. Refusals name their door.

use std::path::PathBuf;

use instrukt_core::config::ConfigError;
use instrukt_core::fsx::FsError;
use instrukt_core::git::GitError;
use instrukt_core::ids::{ProjectSlug, TodoSlug};
use instrukt_core::machine::MachineError;
use instrukt_core::publication::PublicationError;
use instrukt_runner::events::EventLogError;
use instrukt_runner::interfaces::WorkInterfaceError;

use super::_github::GitHubError;

#[derive(Debug, thiserror::Error)]
pub enum WorkError {
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error(transparent)]
    Git(#[from] GitError),
    #[error(transparent)]
    Fs(#[from] FsError),
    #[error(transparent)]
    Machine(#[from] MachineError),
    /// The published eval fingerprint could not be read.
    #[error(transparent)]
    Publication(#[from] PublicationError),
    /// The runner's event log refused the finalize-ready append.
    #[error(transparent)]
    EventLog(#[from] EventLogError),
    #[error("cannot access {}: {source}", path.display())]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    /// The state database could not be opened, read or written.
    #[error(transparent)]
    StateDb(#[from] instrukt_core::state_db::StateDbError),
    /// The project's documentation could not be read for grounding.
    #[error(transparent)]
    Docs(#[from] instrukt_core::docs::DocsError),
    #[error("{} is a symbolic link; instrukt does not follow links in its home", path.display())]
    Symlink { path: PathBuf },
    /// The project layer declares no `project` slug.
    #[error(
        "this repository declares no project; set `project` in its instrukt.yaml (`instrukt config init`)"
    )]
    NoProject,
    /// The repository to reflect to cannot be derived.
    #[error(transparent)]
    Repository(#[from] instrukt_core::github::ResolveError),
    /// Todos need the project's code on GitHub (`github-record`, "Code on
    /// GitHub").
    #[error("{}", instrukt_core::github::todos_need_github(reason))]
    CodeNotOnGitHub { reason: String },
    #[error("{project} has no todo `{todo}`; list the project's work with `instrukt todo list`")]
    UnknownTodo {
        project: ProjectSlug,
        todo: TodoSlug,
    },
    #[error(
        "todo `{todo}` changed on GitHub since this computer last read it; refresh it with `instrukt todo refresh {todo}`, then re-apply your change"
    )]
    Conflicted { todo: TodoSlug },
    /// The record's contract refuses the change.
    #[error("{0}")]
    Refused(String),
    /// GitHub could not answer a read that has no local answer.
    #[error("GitHub: {0}")]
    GitHub(#[from] GitHubError),
}

impl WorkError {
    pub(crate) fn refused(reason: impl Into<String>) -> Self {
        Self::Refused(reason.into())
    }
}

impl From<WorkError> for WorkInterfaceError {
    fn from(err: WorkError) -> Self {
        match err {
            WorkError::UnknownTodo { project, todo } => {
                WorkInterfaceError::UnknownTodo { project, todo }
            }
            WorkError::Conflicted { todo } => WorkInterfaceError::Conflicted { todo },
            WorkError::Refused(reason) => WorkInterfaceError::Refused { reason },
            other => WorkInterfaceError::Store {
                message: other.to_string(),
            },
        }
    }
}
