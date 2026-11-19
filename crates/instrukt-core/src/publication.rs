//! The publication context: committed-state publication into the home,
//! index patching by owner, agent-file rendering and the eval fingerprint
//! (`docs/project/spec/domain/publication.md`).
//!
//! Installed publication reads committed canonical main; explicit development
//! execution reads the development worktree's committed HEAD. Both read:
//! `docs/general/`, each `docs/domain/<x>/` and `docs/project/`. Each is claimed
//! in the ownership record in the selected home's publication state.
//!
//! Two operations. [`render_repository`] renders a repository's committed
//! entrypoints from one view of it and nothing from the computer, as a
//! [`RenderPlan`] its caller applies ([`write_rendering`] for the working
//! tree). [`sync`]: publish the project the caller stands in from
//! the selected committed source, or do nothing when that commit is already
//! published. What it reads (the committed tree), how it patches the
//! index, stages and swaps the published tree, renders agent files and
//! undoes a failed run stays behind that call. Readers of its state use
//! [`PublishedRecord`] and [`EvalFingerprint`].

mod _adoption;
mod _agents;
mod _company;
mod _editor;
mod _fingerprint;
mod _patch;
pub(crate) use _patch::target_entries;
mod _render;
mod _source;
mod _sync;
mod _tree;
mod _tx;

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub use _adoption::{
    ClaimStatus, ClaimsPreview, GlobalFiles, PublishedSchema, preview_claims, published_schema,
};
pub use _company::CompanyReadProblem;
pub use _editor::{
    EditorChange, EditorChangeKind, SCHEMAS_DIR, committed_modeline, editor_schema_describes,
    refresh_editor_schemas,
};
pub use _fingerprint::EvalFingerprint;
pub use _patch::Claim;
pub use _render::{
    GENERATED_LINE, PlannedFile, RenderFinding, RenderFindingKind, RenderPlan, is_rendering,
};
pub use _sync::{
    GlobalDestination, Refusal, SyncError, SyncFinding, SyncOutcome, SyncReport, SyncRequest, sync,
};

use crate::config::ConfigError;
use crate::docs::DocsError;
use crate::fsx::FsError;
use crate::git::GitError;
use crate::home::Home;
use crate::ids::{CommitSha, ProjectSlug};
use crate::index::IndexError;

/// The view of a repository its entrypoints are rendered from.
#[derive(Debug, Clone, Copy)]
pub enum RepositoryView<'a> {
    /// The working tree whose root is this directory, tracked and
    /// untracked files alike.
    WorkingTree(&'a Path),
    /// The index a commit is being made from in the repository whose root
    /// is `root`: `index` (git's `GIT_INDEX_FILE` for the hook run), or the
    /// repository's own index when `None`.
    Index {
        root: &'a Path,
        index: Option<&'a Path>,
    },
}

/// Renders every entrypoint of the repository in `view` from that view
/// alone (`project/instrukt/spec/feature/publication/agent-entrypoints`,
/// "Committed entrypoints"): what to write, what to remove, what the
/// rendering found and noticed. Reads nothing from the computer and writes
/// nothing.
pub fn render_repository(view: RepositoryView<'_>) -> Result<RenderPlan, PublicationError> {
    // An `AGENTS.override.md` is never committed by instrukt: the view
    // holds it, or, for a commit, the working tree beside the index does.
    let (tree, override_beside_root) = match view {
        RepositoryView::WorkingTree(root) => {
            let tree = _source::Committed::working_tree(root)?;
            let present = tree.file(_render::OVERRIDE_FILE).is_some();
            (tree, present)
        }
        RepositoryView::Index { root, index } => {
            let path = root.join(_render::OVERRIDE_FILE);
            let present = match std::fs::symlink_metadata(&path) {
                Ok(_) => true,
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => false,
                Err(source) => return Err(PublicationError::Io { path, source }),
            };
            (_source::Committed::read_index(root, index)?, present)
        }
    };
    _render::render_tree(&tree, override_beside_root)
}

/// The findings of rendering the company home whose root is `home_dir`,
/// from its working tree: what `instrukt config validate --company` reports
/// beside configuration's own findings.
pub fn company_findings(home_dir: &Path) -> Result<Vec<RenderFinding>, PublicationError> {
    Ok(render_repository(RepositoryView::WorkingTree(home_dir))?.findings)
}

/// Applies `plan` to the working tree whose root is `root`: each rendering
/// that differs from its file is written, and each removal made, one file
/// at a time and each atomically. Returns the paths it changed, relative to
/// `root`.
pub fn write_rendering(root: &Path, plan: &RenderPlan) -> Result<Vec<PathBuf>, PublicationError> {
    let mut changed = Vec::new();
    for file in plan.writes.iter().filter(|file| file.differs()) {
        let target = _tx::own_destination(root, &file.path)?;
        crate::fsx::atomic_write(&target, file.text.as_bytes())?;
        changed.push(file.path.clone());
    }
    for path in &plan.removals {
        let target = _tx::own_destination(root, path)?;
        match crate::fsx::remove_file(&target) {
            Ok(()) => changed.push(path.clone()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(PublicationError::Io {
                    path: target,
                    source,
                });
            }
        }
    }
    Ok(changed)
}

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
    #[error("cannot render {file}: {reason}")]
    Rendering { file: String, reason: String },
    /// A document a committed rendering would hold reads a configured
    /// value, which differs per computer.
    #[error("{document} reads a configured value, which a committed rendering cannot hold")]
    CommittedConfig { document: String },
    /// A document a committed rendering would hold carries a directive the
    /// format does not support.
    #[error("{}", findings.iter().map(ToString::to_string).collect::<Vec<_>>().join("; "))]
    CommittedDirectives { findings: Vec<crate::docs::Finding> },
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
    #[error(
        "project `{project}` has no published baseline; run `instrukt sync` in the primary checkout, then retry checkout rendering"
    )]
    CheckoutUnpublished { project: ProjectSlug },
    #[error(
        "checkout bootstrap was refused: {0}; resolve the finding and run `instrukt sync` in the primary checkout"
    )]
    CheckoutRefused(String),
    #[error("checkout bootstrap failed: {0}")]
    CheckoutPublication(#[source] Box<SyncError>),
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
/// selected source commit last published (canonical main in installed use),
/// the project's checkout path and
/// its repository's remote. Docs compares it with main for the behind-main
/// warning and names it as the source of what the project publishes; the
/// runner finds projects through it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishedRecord {
    pub project: ProjectSlug,
    pub commit: CommitSha,
    /// The source checkout: the primary worktree for installed publication,
    /// or the selected development worktree for explicit development execution.
    pub checkout: PathBuf,
    /// The URL of the repository's `origin` remote; `None` without one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote: Option<String>,
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

/// Every claimed published position's owner, by the domain name the
/// position is addressed by (`general` for `docs/general/`).
pub fn position_owners(
    home: &Home,
) -> Result<std::collections::BTreeMap<crate::ids::DomainName, ProjectSlug>, PublicationStateError>
{
    let dir = home.claims_dir();
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok(std::collections::BTreeMap::new());
        }
        Err(source) => return Err(PublicationStateError::Read { path: dir, source }),
    };
    let mut owners = std::collections::BTreeMap::new();
    for entry in entries {
        let entry = entry.map_err(|source| PublicationStateError::Read {
            path: dir.clone(),
            source,
        })?;
        let path = entry.path();
        if path.extension().is_none_or(|ext| ext != "yaml") {
            continue;
        }
        let text =
            std::fs::read_to_string(&path).map_err(|source| PublicationStateError::Read {
                path: path.clone(),
                source,
            })?;
        let record: _tree::ClaimRecord = serde_yaml_ng::from_str(&text)
            .map_err(|source| PublicationStateError::Parse { path, source })?;
        owners.insert(record.position, record.owner);
    }
    Ok(owners)
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

pub mod status;
