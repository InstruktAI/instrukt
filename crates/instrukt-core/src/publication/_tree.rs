//! The published tree in the home: the ownership record of the published
//! positions, staging a project's positions from its committed files, and
//! the per-project record of what a publication wrote outside the home.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::_source::{Committed, safe_relative};
use super::PublicationError;
use crate::docs::{Position, is_reserved_domain_folder};
use crate::fsx::atomic_write;
use crate::home::Home;
use crate::ids::{DomainName, ProjectSlug};

/// The generated bundle's file name inside a published domain.
pub(super) const BUNDLE_FILE: &str = "bundle.md";

/// One claim of the ownership record: the project that owns a published
/// position on this computer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct ClaimRecord {
    /// The position, by the domain name it is addressed by (`general` for
    /// `docs/general/`).
    pub position: DomainName,
    pub owner: ProjectSlug,
}

/// Where the claim on the position `domain` addresses is recorded.
pub(super) fn claim_path(home: &Home, domain: &DomainName) -> PathBuf {
    home.claims_dir().join(format!("{}.yaml", domain.as_str()))
}

/// Every claimed position's owner, by the domain name it is addressed by.
pub(super) fn claims(home: &Home) -> Result<BTreeMap<DomainName, ProjectSlug>, PublicationError> {
    let dir = home.claims_dir();
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(source) => return Err(PublicationError::Io { path: dir, source }),
    };
    let mut owners = BTreeMap::new();
    for entry in entries {
        let entry = entry.map_err(|source| PublicationError::Io {
            path: dir.clone(),
            source,
        })?;
        let path = entry.path();
        if path.extension().is_none_or(|ext| ext != "yaml") {
            continue;
        }
        let text = std::fs::read_to_string(&path).map_err(|source| PublicationError::Io {
            path: path.clone(),
            source,
        })?;
        let record: ClaimRecord = serde_yaml_ng::from_str(&text)
            .map_err(|source| PublicationError::Parse { path, source })?;
        owners.insert(record.position, record.owner);
    }
    Ok(owners)
}

/// The published positions the committed tree holds, by the domain name
/// each is addressed by: `general` for `docs/general/`, `<x>` for each
/// `docs/domain/<x>/`. Folders with a reserved name come back apart, for
/// the sync to refuse; a folder whose name is no domain name is left to
/// validation, which reports its files.
pub(super) fn held_positions(committed: &Committed) -> (Vec<DomainName>, Vec<String>) {
    let mut held = std::collections::BTreeSet::new();
    let mut reserved = std::collections::BTreeSet::new();
    if committed.below("docs/general/").next().is_some()
        && let Ok(general) = DomainName::parse(crate::docs::GENERAL)
    {
        held.insert(general);
    }
    for (rel, _) in committed.below("docs/domain/") {
        let Some((folder, _)) = rel.split_once('/') else {
            continue;
        };
        if is_reserved_domain_folder(folder) {
            reserved.insert(format!("docs/domain/{folder}/"));
        } else if let Ok(domain) = DomainName::parse(folder) {
            held.insert(domain);
        }
    }
    (held.into_iter().collect(), reserved.into_iter().collect())
}

/// Writes the committed files of the position `domain` addresses
/// (`docs/general/…` or `docs/domain/<x>/…`, raw) into `staged`.
pub(super) fn stage_domain(
    committed: &Committed,
    domain: &DomainName,
    staged: &Path,
) -> Result<(), PublicationError> {
    let prefix = format!("{}/", Position::of_domain(domain).dir());
    std::fs::create_dir_all(staged).map_err(|source| PublicationError::Io {
        path: staged.to_path_buf(),
        source,
    })?;
    for (rel, bytes) in committed.below(&prefix) {
        if rel == BUNDLE_FILE {
            // A generated name: an authored file of that name never
            // replaces what publication writes there.
            continue;
        }
        atomic_write(&staged.join(safe_relative(rel)?), bytes)?;
    }
    Ok(())
}

/// What one project's last publication wrote outside the home, so the next
/// sync can retract what it no longer yields.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Outputs {
    /// Compiled `AGENTS.md` files, relative to the primary worktree.
    #[serde(default)]
    pub agents: Vec<PathBuf>,
    /// Global agent entrypoints, relative to the user's home directory.
    #[serde(default)]
    pub entrypoints: Vec<PathBuf>,
}

impl Outputs {
    pub(super) fn record_path(home: &Home, project: &ProjectSlug) -> PathBuf {
        home.publication_outputs_dir()
            .join(format!("{}.yaml", project.as_str()))
    }

    pub(super) fn load(home: &Home, project: &ProjectSlug) -> Result<Self, PublicationError> {
        let path = Self::record_path(home, project);
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(source) => return Err(PublicationError::Io { path, source }),
        };
        serde_yaml_ng::from_str(&text).map_err(|source| PublicationError::Parse { path, source })
    }

    /// The record's path and contents.
    pub(super) fn rendered(
        &self,
        home: &Home,
        project: &ProjectSlug,
    ) -> Result<(PathBuf, Vec<u8>), PublicationError> {
        let text = serde_yaml_ng::to_string(self).map_err(PublicationError::Render)?;
        Ok((Self::record_path(home, project), text.into_bytes()))
    }
}
