//! The published tree in the home: per-domain publisher records, staging a
//! project's domains from its committed files, and the per-project record
//! of what a publication wrote outside the home.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::_source::{Committed, safe_relative};
use super::PublicationError;
use crate::fsx::atomic_write;
use crate::home::Home;
use crate::ids::{DomainName, ProjectSlug};

/// The publisher record's file name inside a published domain.
pub(super) const PUBLISHER_FILE: &str = "publisher.yaml";

/// The generated bundle's file name inside a published domain.
pub(super) const BUNDLE_FILE: &str = "bundle.md";

/// Directory names of the published tree that are not domains.
const NOT_DOMAINS: [&str; 2] = ["snippets", "third-party"];

/// Who published a domain, and the trigger line it declared.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct PublisherRecord {
    pub domain: DomainName,
    pub publisher: ProjectSlug,
    pub trigger: String,
}

/// Every published domain's publisher record, by domain.
pub(super) fn publishers(
    home: &Home,
) -> Result<BTreeMap<DomainName, PublisherRecord>, PublicationError> {
    let docs = home.docs_dir();
    let entries = match std::fs::read_dir(&docs) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(source) => return Err(PublicationError::Io { path: docs, source }),
    };
    let mut records = BTreeMap::new();
    for entry in entries {
        let entry = entry.map_err(|source| PublicationError::Io {
            path: docs.clone(),
            source,
        })?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if name.starts_with('.') || NOT_DOMAINS.contains(&name) {
            continue;
        }
        let path = entry.path().join(PUBLISHER_FILE);
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
            Err(err) if err.kind() == std::io::ErrorKind::NotADirectory => continue,
            Err(source) => return Err(PublicationError::Io { path, source }),
        };
        let record: PublisherRecord = serde_yaml_ng::from_str(&text)
            .map_err(|source| PublicationError::Parse { path, source })?;
        records.insert(record.domain.clone(), record);
    }
    Ok(records)
}

/// Writes `domain`'s committed files (`docs/global/<domain>/…`, raw) and
/// its publisher record into `staged`.
pub(super) fn stage_domain(
    committed: &Committed,
    record: &PublisherRecord,
    staged: &Path,
) -> Result<(), PublicationError> {
    let prefix = format!("docs/global/{}/", record.domain);
    std::fs::create_dir_all(staged).map_err(|source| PublicationError::Io {
        path: staged.to_path_buf(),
        source,
    })?;
    for (rel, bytes) in committed.below(&prefix) {
        if rel == PUBLISHER_FILE || rel == BUNDLE_FILE {
            // Generated names: an authored file of that name never
            // replaces what publication writes there.
            continue;
        }
        atomic_write(&staged.join(safe_relative(rel)?), bytes)?;
    }
    let text = serde_yaml_ng::to_string(record).map_err(PublicationError::Render)?;
    atomic_write(&staged.join(PUBLISHER_FILE), text.as_bytes())?;
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
    fn path(home: &Home, project: &ProjectSlug) -> PathBuf {
        home.publication_outputs_dir()
            .join(format!("{}.yaml", project.as_str()))
    }

    pub(super) fn load(home: &Home, project: &ProjectSlug) -> Result<Self, PublicationError> {
        let path = Self::path(home, project);
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
        Ok((Self::path(home, project), text.into_bytes()))
    }
}
