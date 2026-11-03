//! The configuration context: `instrukt.yaml` in a global and a project
//! layer (`docs/local/spec/domain/configuration.md`).

use std::path::Path;

use crate::ids::{IdError, ProjectSlug};

/// The file name of both configuration layers.
pub const CONFIG_FILE_NAME: &str = "instrukt.yaml";

/// A configuration document could not be read.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("cannot read {path}: {source}")]
    Read {
        path: std::path::PathBuf,
        source: std::io::Error,
    },
    #[error("{path} is not valid YAML: {source}")]
    Parse {
        path: std::path::PathBuf,
        source: serde_yaml_ng::Error,
    },
    #[error("{path}: `project` {source}")]
    Slug {
        path: std::path::PathBuf,
        source: IdError,
    },
}

/// The project slug the repository rooted at `repo_root` declares in its
/// project layer; `None` when it has no project layer or declares none.
///
/// Docs, publication, repository and specs read the slug through this
/// signature.
pub fn project_slug(repo_root: &Path) -> Result<Option<ProjectSlug>, ConfigError> {
    let path = repo_root.join(CONFIG_FILE_NAME);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => return Err(ConfigError::Read { path, source }),
    };
    let doc: serde_yaml_ng::Value =
        serde_yaml_ng::from_str(&text).map_err(|source| ConfigError::Parse {
            path: path.clone(),
            source,
        })?;
    match doc.get("project").and_then(serde_yaml_ng::Value::as_str) {
        Some(raw) => ProjectSlug::parse(raw)
            .map(Some)
            .map_err(|source| ConfigError::Slug { path, source }),
        None => Ok(None),
    }
}
