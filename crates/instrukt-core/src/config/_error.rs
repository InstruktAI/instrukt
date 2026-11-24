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
    /// The dotted key the finding concerns (for example, `runner.runtime`).
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
    /// `runner.runtime` names no runtime-mapping entry.
    UnknownRuntime,
    /// A module switch turns on features whose verbs the chosen runtime
    /// leaves empty.
    RuntimeCapabilityMissing,
    /// A reference to a key that does not exist.
    UnresolvedReference,
    /// A reference that leads back to itself.
    ReferenceCycle,
    /// `modules.todos` is on for a project whose code is not on GitHub.
    TodosNeedGitHub,
    /// `modules.events` is on while the event transport has no
    /// `redis.cluster.url`.
    EventsNeedTransport,
    /// A company home's local-only material that its repository tracks or
    /// does not ignore.
    LeavesTheComputer,
    /// `review.automatic` is on and something automatic review needs is
    /// missing.
    AutomaticReviewNotReady,
}

impl FindingKind {
    /// The kind's plain name.
    pub fn name(self) -> &'static str {
        match self {
            Self::NotPermittedInLayer => "not_permitted_in_layer",
            Self::UnknownKey => "unknown_key",
            Self::Missing => "missing",
            Self::InvalidValue => "invalid_value",
            Self::SchemaVersionNotCurrent => "schema_version_not_current",
            Self::ModuleSwitchedOff => "module_switched_off",
            Self::UnknownRuntime => "unknown_runtime",
            Self::RuntimeCapabilityMissing => "runtime_capability_missing",
            Self::UnresolvedReference => "unresolved_reference",
            Self::ReferenceCycle => "reference_cycle",
            Self::TodosNeedGitHub => "todos_need_github",
            Self::EventsNeedTransport => "events_need_transport",
            Self::LeavesTheComputer => "leaves_the_computer",
            Self::AutomaticReviewNotReady => "automatic_review_not_ready",
        }
    }
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
        "{path} has schema_version {found}, newer than this instrukt supports ({current}); upgrade with `{}`",
        crate::INSTALLER_COMMAND
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
        door: String,
    },
    #[error("{path}: schema_version is missing or not a positive integer")]
    UnknownSchema { path: PathBuf },
    #[error("{path} does not exist; create it with `{door}`")]
    NoLayer { path: PathBuf, door: String },
    #[error("cannot create the company home {path}: {source}")]
    CreateHome {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{dir} is not a company: {finding}")]
    NotACompany { dir: PathBuf, finding: String },
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
    #[error(
        "no selected runtime caller context is available; choose runner.runtime and run inside its caller session"
    )]
    NoActiveRuntime,
    #[error("`runtime.{field}`: the active runtime `{runtime}` has no `{field}`")]
    MissingRuntimeField { runtime: String, field: String },
    #[error("`{key}` {message}")]
    InvalidValue { key: String, message: String },
    #[error(
        "agents.yaml holds both `roles.{retired}` and `roles.{current}`; remove one, then run instrukt again"
    )]
    AgentsRoleConflict {
        retired: &'static str,
        current: &'static str,
    },
    #[error(
        "the configuration changed since revision {expected} (current revision {current}); the patch was not applied — read it again"
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

impl ConfigError {
    /// Why a document cannot be loaded, without its path: for an unreadable
    /// document, one that is not YAML or not a mapping, or one whose
    /// `schema_version` is missing or newer than this instrukt knows.
    pub fn unloadable_reason(&self) -> Option<(&std::path::Path, String)> {
        match self {
            Self::Read { path, source } => Some((path, source.to_string())),
            Self::Parse { path, source } => Some((path, format!("it is not valid YAML: {source}"))),
            Self::NotAMapping { path } => Some((path, "it is not a YAML mapping".to_owned())),
            Self::NewerSchema {
                path,
                found,
                current,
            } => Some((
                path,
                format!(
                    "its schema_version {found} is newer than this instrukt supports ({current})"
                ),
            )),
            Self::UnknownSchema { path } => Some((
                path,
                "its schema_version is missing or not a positive integer".to_owned(),
            )),
            _ => None,
        }
    }
}
