//! What adoption reads of publication before anything is published
//! (`docs/project/spec/feature/repository/project-initialization.md`, "The
//! claims report", "The project schema", "Kept instructions"): the claims a
//! repository's first sync would make, a published domain's `schema.yaml`,
//! and whether a project file is one publication wrote. Nothing here writes.

use std::path::{Path, PathBuf};

use super::_agents::TARGETS;
use super::_tree::{self, Outputs};
use super::_tx::resolved;
use super::PublicationError;
use crate::docs::{GENERAL, Position, SCHEMA_FILE, is_reserved_domain_folder};
use crate::home::Home;
use crate::ids::{DomainName, ProjectSlug};

/// Who holds a position a repository's first sync would claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaimStatus {
    /// No project owns it: the sync claims it.
    Free,
    /// This project owns it already.
    Ours,
    /// Another project owns it: the sync would be refused.
    OwnedBy(ProjectSlug),
}

/// Whether the first sync would render the global agent files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GlobalFiles {
    /// The repository holds no `docs/general/`.
    NotHeld,
    /// `docs/general/` is owned by another project; the sync would be
    /// refused.
    OwnedElsewhere(ProjectSlug),
    /// The sync renders the global file of each supported agent CLI
    /// installed here.
    Rendered {
        /// Each installed agent CLI with its global file.
        agents: Vec<(&'static str, PathBuf)>,
        /// Existing global files instrukt did not write: each would stop
        /// the sync.
        foreign: Vec<PathBuf>,
    },
}

/// The claims a repository's first sync would make, read from its working
/// tree and the computer's ownership record. It claims nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimsPreview {
    /// Each position the working tree holds, `general` first, then the
    /// domains by name.
    pub positions: Vec<(Position, ClaimStatus)>,
    /// Each `docs/domain/<x>/` the working tree holds under a reserved
    /// name: the sync would refuse it.
    pub reserved: Vec<String>,
    pub global_files: GlobalFiles,
}

impl ClaimsPreview {
    /// Whether the repository holds anything it would publish.
    pub fn is_empty(&self) -> bool {
        self.positions.is_empty() && self.reserved.is_empty()
    }
}

/// The claims the repository whose working tree is `root`, declaring
/// `project`, would make on this computer.
pub fn preview_claims(
    home: &Home,
    root: &Path,
    project: &ProjectSlug,
) -> Result<ClaimsPreview, PublicationError> {
    let owners = _tree::claims(home)?;
    let status = |domain: &DomainName| match owners.get(domain) {
        None => ClaimStatus::Free,
        Some(owner) if owner == project => ClaimStatus::Ours,
        Some(owner) => ClaimStatus::OwnedBy(owner.clone()),
    };
    let mut positions = Vec::new();
    let mut reserved = Vec::new();
    if holds_files(&root.join(Position::General.dir()))?
        && let Ok(general) = DomainName::parse(GENERAL)
    {
        positions.push((Position::General, status(&general)));
    }
    let domains = root.join("docs/domain");
    let mut names = Vec::new();
    match std::fs::read_dir(&domains) {
        Ok(entries) => {
            for entry in entries {
                let entry = entry.map_err(|source| PublicationError::Io {
                    path: domains.clone(),
                    source,
                })?;
                if holds_files(&entry.path())? {
                    names.push(entry.file_name().to_string_lossy().into_owned());
                }
            }
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(source) => {
            return Err(PublicationError::Io {
                path: domains,
                source,
            });
        }
    }
    names.sort();
    for name in names {
        if is_reserved_domain_folder(&name) {
            reserved.push(format!("docs/domain/{name}/"));
        } else if let Ok(domain) = DomainName::parse(&name) {
            let claim = status(&domain);
            positions.push((Position::Domain(domain), claim));
        }
    }
    let global_files = match positions.iter().find(|(p, _)| *p == Position::General) {
        None => GlobalFiles::NotHeld,
        Some((_, ClaimStatus::OwnedBy(owner))) => GlobalFiles::OwnedElsewhere(owner.clone()),
        Some(_) => rendered_global_files(home, &owners)?,
    };
    Ok(ClaimsPreview {
        positions,
        reserved,
        global_files,
    })
}

/// The installed agent CLIs' global files, and those of them holding
/// content the current owner of `docs/general/` did not leave there — the
/// predicate the sync's overwrite guard refuses by.
fn rendered_global_files(
    home: &Home,
    owners: &std::collections::BTreeMap<DomainName, ProjectSlug>,
) -> Result<GlobalFiles, PublicationError> {
    let written = match DomainName::parse(GENERAL)
        .ok()
        .and_then(|general| owners.get(&general))
    {
        Some(owner) => Outputs::load(home, owner)?,
        None => Outputs::default(),
    };
    let user_home = home.user_home();
    let mut agents = Vec::new();
    let mut foreign = Vec::new();
    for target in TARGETS {
        if !resolved(user_home, Path::new(target.dir))?.is_dir() {
            continue;
        }
        let rel = Path::new(target.dir).join(target.entrypoint);
        let path = user_home.join(&rel);
        if written.foreign(user_home, &rel)? {
            foreign.push(path.clone());
        }
        agents.push((target.name, path));
    }
    Ok(GlobalFiles::Rendered { agents, foreign })
}

/// Whether `dir` holds at least one file, at any depth.
fn holds_files(dir: &Path) -> Result<bool, PublicationError> {
    if !dir.is_dir() {
        return Ok(false);
    }
    for entry in walkdir::WalkDir::new(dir) {
        let entry = entry.map_err(|err| PublicationError::Io {
            path: dir.to_path_buf(),
            source: err.into(),
        })?;
        if !entry.file_type().is_dir() {
            return Ok(true);
        }
    }
    Ok(false)
}

/// What the computer publishes of a domain's `schema.yaml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PublishedSchema {
    /// The domain is not published on this computer.
    NotPublished,
    /// The domain is published without a `schema.yaml`.
    NoSchema,
    /// The published `schema.yaml`'s bytes.
    Schema(Vec<u8>),
}

/// The `schema.yaml` the computer publishes for `domain`.
pub fn published_schema(
    home: &Home,
    domain: &DomainName,
) -> Result<PublishedSchema, PublicationError> {
    if !_tree::claims(home)?.contains_key(domain) {
        return Ok(PublishedSchema::NotPublished);
    }
    let path = home.domain_dir(domain).join(SCHEMA_FILE);
    match std::fs::read(&path) {
        Ok(bytes) => Ok(PublishedSchema::Schema(bytes)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(PublishedSchema::NoSchema),
        Err(source) => Err(PublicationError::Io { path, source }),
    }
}

/// Whether `project`'s last publication wrote the file `rel` (relative to
/// its primary worktree).
pub fn wrote_project_file(
    home: &Home,
    project: &ProjectSlug,
    rel: &Path,
) -> Result<bool, PublicationError> {
    Ok(Outputs::load(home, project)?
        .agents
        .iter()
        .any(|written| written == rel))
}
