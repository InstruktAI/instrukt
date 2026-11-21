//! What adoption reads of publication before anything is published
//! (`docs/project/spec/feature/repository/project-initialization.md`, "The
//! claims report", "The project schema", "Kept instructions"): the claims a
//! repository's first sync would make, a published domain's `schema.yaml`,
//! and whether a project file is one publication wrote. Nothing here writes.

use std::path::{Path, PathBuf};

use super::_agents::targets;
use super::_tree;
use super::PublicationError;
use crate::docs::{GENERAL, Position, SCHEMA_FILE, is_reserved_domain_folder};
use crate::home::Home;
use crate::ids::{DomainName, ProjectSlug};
use crate::interfaces::{Answer, ConfiguredAgentRuntimes};

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
    /// The sync writes the global file of each agent runtime the selected
    /// runtime reports as configured, whatever the file holds.
    Rendered {
        /// Each file, with its runtime and the instances it serves.
        files: Vec<PlannedGlobalFile>,
    },
    /// The selected runtime cannot be asked: the sync would write no global
    /// file until it can.
    Unreachable { runtime: String, cause: String },
}

/// One global agent file the first sync will write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedGlobalFile {
    pub runtime: &'static str,
    pub path: PathBuf,
    pub instances: Vec<String>,
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
/// The global agent files come from what `runtimes`, the selected runtime's
/// adapter, reports, as the sync asks it.
pub fn preview_claims(
    home: &Home,
    root: &Path,
    project: &ProjectSlug,
    runtimes: Option<&dyn ConfiguredAgentRuntimes>,
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
        Some(_) => rendered_global_files(home, runtimes),
    };
    Ok(ClaimsPreview {
        positions,
        reserved,
        global_files,
    })
}

/// The global files the first sync writes: those of the configured
/// runtimes, as the selected runtime reports them.
fn rendered_global_files(
    home: &Home,
    runtimes: Option<&dyn ConfiguredAgentRuntimes>,
) -> GlobalFiles {
    let Some(runtimes) = runtimes else {
        return GlobalFiles::Rendered { files: Vec::new() };
    };
    match runtimes.configured(home, &crate::config::Env::from_process()) {
        Answer::Unreachable { runtime, cause } => GlobalFiles::Unreachable { runtime, cause },
        Answer::Reported(instances) => GlobalFiles::Rendered {
            files: targets(home.user_home(), &instances)
                .files
                .into_iter()
                .map(|file| PlannedGlobalFile {
                    runtime: file.destination.cli,
                    path: file.path(),
                    instances: file.instances,
                })
                .collect(),
        },
    }
}

/// Whether `dir` holds at least one file, at any depth.
pub(super) fn holds_files(dir: &Path) -> Result<bool, PublicationError> {
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
