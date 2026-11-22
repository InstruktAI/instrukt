//! Validation findings: what is wrong with a workflow flavor's `workflow.yaml`, and where.

use std::fmt;
use std::path::PathBuf;

/// One problem in a flavor's workflow declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    /// The file, relative to the flavor's `workflow/` directory.
    pub file: PathBuf,
    /// Where in the file: a key path, a `line:column` for unparsable YAML,
    /// or empty for the whole file.
    pub location: String,
    pub problem: Problem,
}

impl fmt::Display for Finding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.location.is_empty() {
            write!(f, "{}: {}", self.file.display(), self.problem)
        } else {
            write!(
                f,
                "{}: {}: {}",
                self.file.display(),
                self.location,
                self.problem
            )
        }
    }
}

/// What a finding reports.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Problem {
    #[error("the file is missing")]
    MissingFile,
    #[error("cannot read the file: {0}")]
    Unreadable(String),
    #[error("not valid YAML: {0}")]
    Yaml(String),
    #[error("expected {0}")]
    Shape(&'static str),
    #[error("required key `{0}` is missing")]
    MissingKey(&'static str),
    #[error("unknown key `{0}`")]
    UnknownKey(String),
    #[error("`roles` must list at least one role")]
    NoRoles,
    #[error("role `{0}` is declared more than once")]
    DuplicateRole(String),
    #[error("{0}")]
    InvalidRole(String),
    #[error("orchestrator role `{0}` is not declared under `roles`")]
    UndeclaredOrchestrator(String),
    #[error("invalid capability: {0}")]
    InvalidCapability(String),
    #[error("unexpected workflow file `{0}`; prose-routed workflows declare only `workflow.yaml`")]
    UnexpectedFile(String),
}
