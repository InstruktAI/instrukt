//! What a repository operation can fail with or refuse.

use std::path::PathBuf;

use crate::config::ConfigError;
use crate::fsx::FsError;
use crate::git::GitError;
use crate::ids::{CommitSha, IdError};
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
    #[error(
        "nothing was committed: in an adopted project a commit carries its author's review-routing choice; pass `--review` when it goes to the independent review of a head that contains it, or `--exempt <basis>` when it owes none, stating why (`general/policy/commits`)"
    )]
    RoutingMissing,
    #[error(
        "nothing was committed: `--review` and `--exempt <basis>` are the two sides of one review-routing choice; pass exactly one (`general/policy/commits`)"
    )]
    RoutingBoth,
    #[error(
        "nothing was committed: `--exempt <basis>` needs a basis stating why the commit owes no independent review; state it, or pass `--review` instead (`general/policy/commits`)"
    )]
    RoutingBasisEmpty,
    #[error(
        "nothing was committed: review evidence belongs to an adopted project, and this repository is not one, so `--exempt <basis>` has nothing to record; commit with `--review` or without a routing choice (`general/policy/commits`)"
    )]
    ExemptOutsideProject,
    #[error(
        "nothing was committed: this invocation cannot record an exemption, because work's admission of review evidence is not available; commit with `--review`, or commit and then record the decision with `instrukt review exempt` instead of `--exempt <basis>` (`general/policy/commits`)"
    )]
    ExemptionUnavailable,
    #[error(
        "committed {commit}, but its exemption was not recorded ({cause}); the commit stands without a routing record — record the decision with `instrukt review exempt --commit {commit} --basis {basis:?}`"
    )]
    ExemptionNotRecorded {
        commit: CommitSha,
        basis: String,
        cause: InterfaceError,
    },
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
    PublicationState(#[from] crate::publication::PublicationStateError),
    #[error(transparent)]
    Generation(#[from] crate::config::GenerationRefusal),
    #[error(transparent)]
    CompanyRepository(#[from] crate::interfaces::CompanyRepositoryError),
    #[error(
        "{}: cannot be read as the company's creation intent ({reason}); remove it to start the creation anew",
        path.display()
    )]
    CompanyIntent { path: PathBuf, reason: String },
    #[error("the company skeleton cannot be composed: {findings}")]
    Skeleton { findings: String },
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
