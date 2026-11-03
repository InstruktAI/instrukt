//! The publication context: committed-state publication into the home,
//! index patching by owner, entrypoint compilation and the eval fingerprint
//! (`docs/local/spec/domain/publication.md`).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::fsx::{FsError, atomic_write};
use crate::home::Home;
use crate::ids::{CommitSha, ProjectSlug};

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

    /// Writes the record atomically. Callers hold the docs lock.
    pub fn save(&self, home: &Home) -> Result<(), PublicationStateError> {
        let text = serde_yaml_ng::to_string(self).map_err(PublicationStateError::Render)?;
        atomic_write(&record_path(home, &self.project), text.as_bytes())?;
        Ok(())
    }
}

fn record_path(home: &Home, project: &ProjectSlug) -> PathBuf {
    home.publication_state_dir()
        .join(format!("{}.yaml", project.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_read_back_per_project_and_all_together() {
        let dir = tempfile::tempdir().unwrap();
        let home = Home::at_user_home(dir.path().to_path_buf());
        let project = ProjectSlug::parse("demo").unwrap();
        assert_eq!(PublishedRecord::load(&home, &project).unwrap(), None);
        let record = PublishedRecord {
            project: project.clone(),
            commit: CommitSha::parse(&"a".repeat(40)).unwrap(),
            checkout: PathBuf::from("/work/demo"),
        };
        record.save(&home).unwrap();
        assert_eq!(
            PublishedRecord::load(&home, &project).unwrap(),
            Some(record.clone())
        );
        assert_eq!(PublishedRecord::load_all(&home).unwrap(), vec![record]);
    }
}
