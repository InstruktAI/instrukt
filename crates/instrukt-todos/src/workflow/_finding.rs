//! Validation findings: what is wrong with a flavor, and where.

use std::fmt;
use std::path::PathBuf;

/// One problem in a flavor's files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    /// The file, relative to the flavor's `workflow/` directory.
    pub file: PathBuf,
    /// Where in the file: a key path such as `transitions[3].when.all[1]`,
    /// a `line:column` for unparsable YAML, or empty for the whole file.
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
    #[error("orchestrator role `{0}` is not declared under `roles` in actions.yaml")]
    UndeclaredOrchestrator(String),
    #[error("declared `flow: {declared}` does not match the file name `{expected}`")]
    FlowMismatch { declared: String, expected: String },
    #[error("state `{0}` is declared more than once")]
    DuplicateState(String),
    #[error("state `{0}` is not declared in `states`")]
    UndeclaredState(String),
    #[error("action `{0}` is not configured in actions.yaml")]
    UnknownAction(String),
    #[error("effect `{0}` is not configured under `effects` in actions.yaml")]
    UnknownEffect(String),
    #[error("`{0}` is not a fact the workflow declares")]
    UnknownFact(String),
    #[error("unknown condition operator `{0}`")]
    UnknownOperator(String),
    #[error("a condition needs at least one operator")]
    EmptyCondition,
    #[error("a literal must be a string, boolean or number")]
    InvalidLiteral,
    #[error("template path `{0}` must stay within workflow/")]
    TemplateOutside(String),
    #[error("template `{0}` does not exist")]
    TemplateMissing(String),
    #[error("`{{brief}}` is the rendered template and is not available here")]
    BriefUnavailable,
}
