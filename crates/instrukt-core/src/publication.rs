//! The publication context: committed-state publication into the home,
//! index patching by owner, entrypoint compilation and the eval fingerprint
//! (`docs/project/spec/domain/publication.md`).
//!
//! What a project publishes follows from what its committed main holds:
//! `docs/general/` and each `docs/domain/<x>/`. Each is claimed in the
//! ownership record under `~/.instrukt/state/publication/claims/`.
//!
//! One operation, [`sync`]: publish the project the caller stands in from
//! its canonical main, or do nothing when that commit is already
//! published. What it reads (the committed tree), how it patches the
//! index, stages and swaps the published tree, compiles entrypoints and
//! undoes a failed run stays behind that call. Readers of its state use
//! [`PublishedRecord`] and [`EvalFingerprint`].

mod _agents;
mod _compile;
mod _fingerprint;
mod _patch;
mod _source;
mod _sync;
mod _tree;
mod _tx;

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub use _fingerprint::EvalFingerprint;
pub use _patch::Claim;
pub use _sync::{Refusal, SyncError, SyncOutcome, SyncReport, SyncRequest, sync};

use crate::config::ConfigError;
use crate::docs::DocsError;
use crate::fsx::FsError;
use crate::git::GitError;
use crate::home::Home;
use crate::ids::{CommitSha, ProjectSlug};
use crate::index::IndexError;

/// A sync could not run to its end. Refusals are not errors (see
/// [`Refusal`]); these are.
#[derive(Debug, thiserror::Error)]
pub enum PublicationError {
    #[error("{} is not inside a git repository", dir.display())]
    NotInRepository { dir: PathBuf },
    #[error("{} does not exist; a project is adopted with `instrukt config init`", path.display())]
    NoProjectLayer { path: PathBuf },
    #[error("{} declares no `project` slug", path.display())]
    NoProjectSlug { path: PathBuf },
    #[error(
        "the committed {CONFIG} declares project `{committed}`, the working tree `{working}`; commit the change to publish it",
        CONFIG = crate::config::CONFIG_FILE_NAME
    )]
    SlugMismatch {
        committed: String,
        working: ProjectSlug,
    },
    #[error("cannot find the primary worktree of the repository at {}", dir.display())]
    NoPrimaryWorktree { dir: PathBuf },
    #[error("cannot tell the default branch of the repository at {}", dir.display())]
    NoDefaultBranch { dir: PathBuf },
    #[error("the default branch `{branch}` has no commit in {}", dir.display())]
    MainHasNoCommit { dir: PathBuf, branch: String },
    #[error("git returned no content for {path} ({object})")]
    UnreadableObject { object: String, path: String },
    #[error("committed path `{path}` leaves the repository")]
    UnsafePath { path: String },
    #[error(
        "{} is a symbolic link or lies behind one; publication writes only inside its root",
        path.display()
    )]
    Unconfined { path: PathBuf },
    #[error(
        "{} is a symbolic link that leads nowhere; publication changes nothing through it",
        path.display()
    )]
    DanglingLink { path: PathBuf },
    #[error("{} cannot take the entrypoint-loading key: {reason}", path.display())]
    Settings { path: PathBuf, reason: String },
    #[error("cannot compile {file}: {reason}")]
    Compile { file: String, reason: String },
    #[error("cannot access {}: {source}", path.display())]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{} is not valid: {source}", path.display())]
    Parse {
        path: PathBuf,
        source: serde_yaml_ng::Error,
    },
    #[error("cannot render a publication record: {0}")]
    Render(serde_yaml_ng::Error),
    #[error(transparent)]
    Git(#[from] GitError),
    #[error(transparent)]
    Fs(#[from] FsError),
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error(transparent)]
    Docs(#[from] DocsError),
    #[error(transparent)]
    Index(#[from] IndexError),
    #[error(transparent)]
    State(#[from] PublicationStateError),
}

/// Publication state could not be read or written.
#[derive(Debug, thiserror::Error)]
pub enum PublicationStateError {
    #[error("cannot read {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{path} is not a valid publication record: {source}")]
    Parse {
        path: PathBuf,
        source: serde_yaml_ng::Error,
    },
    #[error("cannot render the publication record: {0}")]
    Render(serde_yaml_ng::Error),
    #[error(transparent)]
    Write(#[from] FsError),
}

/// What publication records per project under `state/publication/`: the
/// commit of canonical main last published and the project's checkout path.
/// Docs compares it with main for the behind-main warning; the runner finds
/// projects through it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishedRecord {
    pub project: ProjectSlug,
    pub commit: CommitSha,
    /// The project's primary worktree.
    pub checkout: PathBuf,
}

impl PublishedRecord {
    /// The record of `project`; `None` when it never published.
    pub fn load(home: &Home, project: &ProjectSlug) -> Result<Option<Self>, PublicationStateError> {
        let path = record_path(home, project);
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(source) => return Err(PublicationStateError::Read { path, source }),
        };
        serde_yaml_ng::from_str(&text)
            .map(Some)
            .map_err(|source| PublicationStateError::Parse { path, source })
    }

    /// Every project's record, in project order.
    pub fn load_all(home: &Home) -> Result<Vec<Self>, PublicationStateError> {
        let dir = home.publication_state_dir();
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(source) => return Err(PublicationStateError::Read { path: dir, source }),
        };
        let mut records = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|source| PublicationStateError::Read {
                path: dir.clone(),
                source,
            })?;
            let name = entry.file_name();
            let Some(stem) = name.to_str().and_then(|n| n.strip_suffix(".yaml")) else {
                continue;
            };
            let Ok(project) = ProjectSlug::parse(stem) else {
                continue;
            };
            if let Some(record) = Self::load(home, &project)? {
                records.push(record);
            }
        }
        records.sort_by(|a, b| a.project.cmp(&b.project));
        Ok(records)
    }
}

/// The project that owns the published position `domain` addresses
/// (`general` for `docs/general/`), from the ownership record; `None` when
/// no project publishes it.
pub fn domain_owner(
    home: &Home,
    domain: &crate::ids::DomainName,
) -> Result<Option<ProjectSlug>, PublicationStateError> {
    let path = _tree::claim_path(home, domain);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => return Err(PublicationStateError::Read { path, source }),
    };
    serde_yaml_ng::from_str::<_tree::ClaimRecord>(&text)
        .map(|record| Some(record.owner))
        .map_err(|source| PublicationStateError::Parse { path, source })
}

fn record_path(home: &Home, project: &ProjectSlug) -> PathBuf {
    home.publication_state_dir()
        .join(format!("{}.yaml", project.as_str()))
}
