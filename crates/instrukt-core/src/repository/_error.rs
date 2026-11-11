//! What a repository operation can fail with or refuse.

use std::path::PathBuf;

use crate::config::ConfigError;
use crate::fsx::FsError;
use crate::git::GitError;
use crate::ids::IdError;
use crate::interfaces::InterfaceError;
use crate::publication::PublicationError;

use super::_section::SectionError;

/// A repository operation failed or was refused; every variant names what
/// it concerned.
#[derive(Debug, thiserror::Error)]
pub enum RepositoryError {
    #[error(transparent)]
    App(#[from] crate::github::AppError),
    #[error(transparent)]
    GitHubRepository(#[from] crate::github::ResolveError),
    #[error(
        "main is integrator-only; commit in the todo worktree or a short-lived ad hoc worktree, then review its head before integration"
    )]
    CanonicalAuthoring,
    #[error("{dir} is not inside a git repository's working tree")]
    NotInWorktree { dir: PathBuf },
    #[error("cannot read {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("cannot create {path}: {source}")]
    Create {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("cannot remove {path}: {source}")]
    Remove {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error(
        "{path} resolves to {target}, outside the repository; instrukt does not write there and changed nothing"
    )]
    Escapes { path: PathBuf, target: PathBuf },
    #[error("{path} is not UTF-8 text")]
    NotText { path: PathBuf },
    #[error("{path}: {source}; fix or remove its instrukt markers, then run again")]
    Section { path: PathBuf, source: SectionError },
    #[error(
        "line {line} of the message (`{text}`) carries the `{key}` trailer, which instrukt writes itself; remove it and supply the fact through {door}"
    )]
    OwnedTrailer {
        line: usize,
        text: String,
        key: &'static str,
        door: &'static str,
    },
    #[error(
        "the active agent `{agent}` has no git author; set `agents.{agent}.git_author` in the global layer (`Name <email>`)"
    )]
    AgentAuthorMissing { agent: String },
    #[error("`agents.{agent}.git_author` is `{value}`, which is not of the form `Name <email>`")]
    AgentAuthorMalformed { agent: String, value: String },
    #[error("`{revision}` names no commit: {source}")]
    UnknownCommit {
        revision: String,
        source: Box<GitError>,
    },
    #[error("malformed hook worker job `{job}`")]
    WorkerJob { job: String },
    #[error("{path} is not a valid hook run record: {source}")]
    Record {
        path: PathBuf,
        source: serde_yaml_ng::Error,
    },
    #[error("{path} has hook-record version {version}; upgrade instrukt before reading it")]
    HookRecordVersion { path: PathBuf, version: u32 },
    #[error("cannot render the hook run record: {0}")]
    RenderRecord(serde_yaml_ng::Error),
    #[error("cannot start {program}: {source}")]
    Spawn {
        program: PathBuf,
        source: std::io::Error,
    },
    #[error("cannot wait for {program}: {source}")]
    Wait {
        program: PathBuf,
        source: std::io::Error,
    },
    #[error(
        "{} declares no `project` slug; add `project: <slug>` to it and run again",
        path.display()
    )]
    NoProjectSlug { path: PathBuf },
    #[error(transparent)]
    Publication(#[from] PublicationError),
    #[error(transparent)]
    Id(#[from] IdError),
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error(transparent)]
    Git(#[from] GitError),
    #[error(transparent)]
    Fs(#[from] FsError),
    #[error(transparent)]
    Interface(#[from] InterfaceError),
}
