//! The global index: `~/.instrukt/docs/index.yaml`, every published entry of
//! every project, each carrying its owner.
//!
//! This is the persisted format shared by its writer and its readers:
//! publication builds entries from a project's committed docs and patches
//! the index by owner (`docs/project/spec/domain/publication.md`, "Global
//! index"); docs lists and resolves from it (`docs index`, `docs get`). The
//! entry fields follow the predecessor's index, plus local entries carrying
//! the absolute path of their file in their project.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::fsx::FsError;
use crate::home::Home;
use crate::ids::{ProjectSlug, SnippetId, ThirdPartyId, TodoSlug};

/// The global index could not be read or written.
#[derive(Debug, thiserror::Error)]
pub enum IndexError {
    #[error("cannot read {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{path} is not a valid index: {source}")]
    Parse {
        path: PathBuf,
        source: serde_yaml_ng::Error,
    },
    #[error("cannot render the index: {0}")]
    Render(serde_yaml_ng::Error),
    #[error(transparent)]
    Write(#[from] FsError),
}

/// The whole global index.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GlobalIndex {
    /// Taxonomy snippets, global and local.
    #[serde(default)]
    pub snippets: Vec<IndexEntry>,
    /// Third-party docs, indexed apart from taxonomy snippets.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub third_party: Vec<ThirdPartyEntry>,
    /// Published include fragments, each with its owner. Not retrievable
    /// by id; recorded so publication patches and retracts them by owner.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fragments: Vec<FragmentEntry>,
}

/// Whether an entry is of a published domain or a project's local half.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IndexScope {
    Global,
    Local,
}

/// What the home records about one snippet.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IndexEntry {
    pub id: SnippetId,
    pub description: String,
    /// The taxonomy: principle, concept, policy, procedure, design, spec, adr.
    #[serde(rename = "type")]
    pub taxonomy: String,
    pub scope: IndexScope,
    /// The published domain, or the local half's structural domain.
    pub domain: String,
    /// Global entries: relative to `~/.instrukt/docs/`. Local entries: the
    /// absolute path of the file in its project.
    pub path: PathBuf,
    /// The owning project.
    pub source_project: ProjectSlug,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visibility: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub delivered_by: Vec<TodoSlug>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub superseded_by: Vec<TodoSlug>,
    /// Procedure entry metadata; absent for every other snippet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activation_trigger: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subscriptions: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parameters: Vec<IndexParameter>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dispatch: Option<IndexDispatch>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_delivery: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub interruptible: bool,
}

/// One declared procedure parameter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexParameter {
    pub name: String,
    #[serde(default)]
    pub required: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub rest: bool,
}

/// A procedure's dispatch defaults.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexDispatch {
    pub agent: Vec<String>,
    pub thinking_mode: String,
}

/// One third-party doc.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThirdPartyEntry {
    pub id: ThirdPartyId,
    pub description: String,
    /// Relative to `~/.instrukt/docs/`.
    pub path: PathBuf,
    pub source_project: ProjectSlug,
}

/// One published include fragment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FragmentEntry {
    /// Its path below `docs/global/snippets/`, without `.md` (`@include:
    /// docs/snippets/<name>.md`).
    pub name: String,
    /// Relative to `~/.instrukt/docs/`.
    pub path: PathBuf,
    pub source_project: ProjectSlug,
}

impl GlobalIndex {
    /// The home's index; `None` when nothing has been published yet.
    pub fn load(home: &Home) -> Result<Option<Self>, IndexError> {
        Self::load_from(&home.index_file())
    }

    fn load_from(path: &Path) -> Result<Option<Self>, IndexError> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(source) => {
                return Err(IndexError::Read {
                    path: path.to_path_buf(),
                    source,
                });
            }
        };
        serde_yaml_ng::from_str(&text)
            .map(Some)
            .map_err(|source| IndexError::Parse {
                path: path.to_path_buf(),
                source,
            })
    }
}
