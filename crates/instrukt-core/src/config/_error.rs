//! The configuration context's errors and validation findings.

use std::fmt;
use std::path::PathBuf;

use crate::fsx::FsError;
use crate::git::GitError;
use crate::ids::IdError;

/// One validation finding: a key of a document that breaks a rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    /// The document the finding concerns.
    pub document: PathBuf,
    /// The dotted key the finding concerns (`runtimes`, `runtimes.0.start`).
    pub key: String,
    /// Which rule the key breaks.
    pub kind: FindingKind,
    /// What is wrong with it, for a reader.
    pub message: String,
}

impl fmt::Display for Finding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}: {}: {}",
            self.document.display(),
            self.key,
            self.message
        )
    }
}

/// The rule a finding reports broken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FindingKind {
    /// A key of the other layer.
    NotPermittedInLayer,
    /// A key the schema does not know.
    UnknownKey,
    /// A required key or field is absent (`schema_version`, a runtime field).
    Missing,
    /// A value of the wrong type or outside its allowed values.
    InvalidValue,
    /// A `schema_version` other than the one this instrukt writes.
    SchemaVersionNotCurrent,
    /// `requires` names a module switched off in the effective configuration.
    ModuleSwitchedOff,
    /// `agents.order` names an agent no runtime-mapping entry can start.
    AgentNotStartable,
    /// `runner.runtime` names no runtime-mapping entry.
    UnknownRuntime,
    /// A reference to a key that does not exist.
    UnresolvedReference,
    /// A reference that leads back to itself.
    ReferenceCycle,
}

/// The findings that refused a write, one per line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Findings(pub Vec<Finding>);

impl fmt::Display for Findings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, finding) in self.0.iter().enumerate() {
            if i > 0 {
                f.write_str("\n")?;
            }
            write!(f, "{finding}")?;
        }
        Ok(())
    }
}

/// A configuration operation failed or was refused.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("cannot read {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{path} is not valid YAML: {source}")]
    Parse {
        path: PathBuf,
        source: serde_yaml_ng::Error,
    },
    #[error("{path} is not a YAML mapping")]
    NotAMapping { path: PathBuf },
    #[error("{path}: `project` {source}")]
    Slug { path: PathBuf, source: IdError },
    #[error(
        "{path} has schema_version {found}, newer than this instrukt supports ({current}); upgrade instrukt"
    )]
    NewerSchema {
        path: PathBuf,
        found: u64,
        current: u64,
    },
    #[error(
        "{path} has schema_version {found}, older than the current {current}; migrate it with `{door}`"
    )]
    OlderSchema {
        path: PathBuf,
        found: u64,
        current: u64,
        door: &'static str,
    },
    #[error("{path}: schema_version is missing or not a positive integer")]
    UnknownSchema { path: PathBuf },
    #[error("{path} does not exist; create it with `{door}`")]
    NoLayer { path: PathBuf, door: &'static str },
    #[error("{dir} is not inside a git repository; there is no project layer (use --global)")]
    NotInProject { dir: PathBuf },
    #[error("`{path}` does not exist: no `{segment}` in {parent}")]
    MissingKey {
        path: String,
        segment: String,
        parent: String,
    },
    #[error("`{path}` is a mapping or list; a reference needs a scalar value")]
    NotScalar { path: String },
    #[error("environment variable `{name}` is not set")]
    UnsetVariable { name: String },
    #[error("reference cycle: {}", chain.join(" -> "))]
    Cycle { chain: Vec<String> },
    #[error("no configured runtime's session variable is set; `runtime.*` has no value here")]
    NoActiveRuntime,
    #[error("`runtime.{field}`: the active runtime `{runtime}` has no `{field}`")]
    MissingRuntimeField { runtime: String, field: String },
    #[error("`{key}` {message}")]
    InvalidValue { key: String, message: String },
    #[error(
        "the configuration changed since revision {expected} (current revision {current}); nothing was written — read it again"
    )]
    RevisionMismatch { expected: String, current: String },
    #[error("the patch is not a YAML mapping")]
    PatchNotAMapping,
    #[error("the patch is not valid YAML: {0}")]
    PatchParse(serde_yaml_ng::Error),
    #[error("refused; nothing was written:\n{0}")]
    Invalid(Findings),
    #[error(
        "the primary worktree's folder `{folder}` normalizes to an empty project slug; run `instrukt init --slug <slug>`"
    )]
    EmptySlug { folder: String },
    #[error("cannot render {path} as YAML: {source}")]
    Render {
        path: PathBuf,
        source: serde_yaml_ng::Error,
    },
    #[error(transparent)]
    Fs(#[from] FsError),
    #[error(transparent)]
    Git(#[from] GitError),
}
