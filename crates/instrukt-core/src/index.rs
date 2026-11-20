//! The global index: `~/.instrukt/state/index.yaml`, every published entry of
//! every project, each carrying its owner.
//!
//! This is the persisted format shared by its writer and its readers:
//! publication builds entries from a project's committed docs and patches
//! the index by owner (`docs/project/spec/domain/publication.md`, "Global
//! index"); docs lists and resolves from it (`docs index`, `docs get`).
//! Every entry carries a path relative to the published docs store. The
//! publication record retains source checkout provenance.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::fsx::FsError;
use crate::home::Home;
use crate::ids::{ProjectSlug, RoleName, SnippetId, ThirdPartyId, TodoSlug};
use crate::subscription::{SubscriptionAttendance, SubscriptionHome, SubscriptionIdentity};

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
    #[error("unsupported index schema_version {found} in {path}; upgrade instrukt")]
    Version { path: PathBuf, found: u64 },
    #[error(transparent)]
    Write(#[from] FsError),
}

/// The whole global index.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GlobalIndex {
    pub schema_version: u64,
    /// Taxonomy snippets, global and local.
    pub snippets: Vec<IndexEntry>,
    /// Third-party docs, indexed apart from taxonomy snippets.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub third_party: Vec<ThirdPartyEntry>,
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
    /// Relative to `~/.instrukt/docs/`, including `project/<slug>/…`.
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subscription_home: Option<SubscriptionHome>,
    #[serde(default)]
    pub subscription_identity: SubscriptionIdentity,
    #[serde(default)]
    pub subscription_attendance: SubscriptionAttendance,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_reads: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parameters: Vec<IndexParameter>,
    #[serde(default = "default_interruptible")]
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

/// One third-party doc.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThirdPartyEntry {
    pub id: ThirdPartyId,
    pub description: String,
    /// Relative to `~/.instrukt/docs/`.
    pub path: PathBuf,
    pub source_project: ProjectSlug,
}

/// A procedure declaring a role, as the index holds it: delivered, or still
/// pending its delivering todo. Each consumer decides what a pending one
/// means for it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RoleProcedure<'i> {
    /// Delivered: its contract is current.
    Delivered(&'i IndexEntry),
    /// Pending: published with its pending ownership, not yet delivered.
    Pending(&'i IndexEntry),
}

impl GlobalIndex {
    /// The procedures of the published positions (`general` and the
    /// domains, never a project's) that declare `role`, in index order.
    /// Validation lets one procedure declare a role, so there is at most one.
    pub fn procedures_declaring(&self, role: &RoleName) -> Vec<RoleProcedure<'_>> {
        self.snippets
            .iter()
            .filter(|entry| {
                entry.taxonomy == "procedure"
                    && entry.scope == IndexScope::Global
                    && entry.role.as_deref() == Some(role.as_str())
            })
            .map(|entry| {
                if entry.delivered_by.is_empty() {
                    RoleProcedure::Delivered(entry)
                } else {
                    RoleProcedure::Pending(entry)
                }
            })
            .collect()
    }

    /// The home's index; `None` when nothing has been published yet.
    pub fn load(home: &Home) -> Result<Option<Self>, IndexError> {
        Self::load_from(&home.index_file())
    }

    /// The home's snippet entries, each decoded on its own: an entry this
    /// version cannot decode is dropped and named, and every other entry is
    /// kept. `None` when nothing has been published yet; an index that cannot
    /// be read as a whole is an error, never taken as empty.
    pub fn load_entries(home: &Home) -> Result<Option<IndexEntries>, IndexError> {
        let path = home.index_file();
        let Some(text) = read_text(&path)? else {
            return Ok(None);
        };
        Self::entries(&path, &text).map(Some)
    }

    /// Read available entries and log the ones this version cannot decode.
    /// Publication uses the strict loader before rewriting the index.
    pub fn load_readable(home: &Home) -> Result<Option<Self>, IndexError> {
        Ok(Self::load_entries(home)?.map(|entries| {
            for dropped in &entries.dropped {
                tracing::warn!(event = "published_index_entry_dropped", entry = dropped.label(), reason = %dropped.reason);
            }
            entries.into_index()
        }))
    }

    fn entries(path: &Path, text: &str) -> Result<IndexEntries, IndexError> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Document {
            schema_version: u64,
            snippets: Vec<serde_yaml_ng::Value>,
            #[serde(default)]
            third_party: Vec<serde_yaml_ng::Value>,
        }

        let document: Document =
            serde_yaml_ng::from_value(Self::prepared(path, text)?).map_err(|source| {
                IndexError::Parse {
                    path: path.into(),
                    source,
                }
            })?;
        let mut out = IndexEntries::default();
        out.entries = decode_entries(document.snippets, &mut out.dropped);
        out.third_party = decode_entries(document.third_party, &mut out.dropped);
        if document.schema_version == 1 {
            upcast_paths(&mut out.entries);
        }
        Ok(out)
    }

    fn load_from(path: &Path) -> Result<Option<Self>, IndexError> {
        let Some(text) = read_text(path)? else {
            return Ok(None);
        };
        Self::parse(path, &text).map(Some)
    }

    /// The index document as this version reads it: an index without a
    /// schema version read as version 1, a newer one refused.
    fn prepared(path: &Path, text: &str) -> Result<serde_yaml_ng::Value, IndexError> {
        let map: serde_yaml_ng::Mapping =
            serde_yaml_ng::from_str(text).map_err(|source| IndexError::Parse {
                path: path.into(),
                source,
            })?;
        let mut value = serde_yaml_ng::Value::Mapping(map);
        if value
            .get("snippets")
            .is_some_and(|entries| !entries.is_sequence())
        {
            return Err(IndexError::Parse {
                path: path.into(),
                source: <serde_yaml_ng::Error as serde::de::Error>::custom(
                    "snippets must be a YAML sequence",
                ),
            });
        }
        let old = value.get("schema_version").is_none();
        if old {
            value["schema_version"] = 1.into();
            if let Some(entries) = value
                .get_mut("snippets")
                .and_then(serde_yaml_ng::Value::as_sequence_mut)
            {
                for entry in entries {
                    if let Some(map) = entry.as_mapping_mut() {
                        map.remove("dispatch");
                    }
                }
            }
        }
        if let Some(found) = value
            .get("schema_version")
            .and_then(serde_yaml_ng::Value::as_u64)
            && !matches!(found, 1 | 2)
        {
            return Err(IndexError::Version {
                path: path.into(),
                found,
            });
        }
        Ok(value)
    }

    fn parse(path: &Path, text: &str) -> Result<Self, IndexError> {
        let value = Self::prepared(path, text)?;
        let mut index: Self =
            serde_yaml_ng::from_value(value).map_err(|source| IndexError::Parse {
                path: path.into(),
                source,
            })?;
        // Upcast old checkout locations without reading them. Publication
        // persists the current shape under its existing owner lock.
        if index.schema_version == 1 {
            upcast_paths(&mut index.snippets);
            index.schema_version = 2;
        }
        Ok(index)
    }
}

fn upcast_paths(entries: &mut [IndexEntry]) {
    for entry in entries
        .iter_mut()
        .filter(|entry| entry.scope == IndexScope::Local)
    {
        entry.path = PathBuf::from(format!("{}.md", entry.id.as_str()));
    }
}

fn decode_entries<T: serde::de::DeserializeOwned>(
    values: Vec<serde_yaml_ng::Value>,
    dropped: &mut Vec<DroppedEntry>,
) -> Vec<T> {
    values
        .into_iter()
        .filter_map(|value| {
            let id = value
                .get("id")
                .and_then(serde_yaml_ng::Value::as_str)
                .map(str::to_owned);
            match serde_yaml_ng::from_value(value) {
                Ok(entry) => Some(entry),
                Err(error) => {
                    dropped.push(DroppedEntry {
                        id,
                        reason: error.to_string(),
                    });
                    None
                }
            }
        })
        .collect()
}

/// The index's text; `None` when it does not exist.
fn read_text(path: &Path) -> Result<Option<String>, IndexError> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(IndexError::Read {
            path: path.to_path_buf(),
            source,
        }),
    }
}

/// The snippet entries of the index, each decoded on its own.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct IndexEntries {
    pub entries: Vec<IndexEntry>,
    third_party: Vec<ThirdPartyEntry>,
    /// The entries this version could not decode.
    pub dropped: Vec<DroppedEntry>,
}

impl IndexEntries {
    pub fn into_index(self) -> GlobalIndex {
        GlobalIndex {
            schema_version: 2,
            snippets: self.entries,
            third_party: self.third_party,
        }
    }
}

/// An index entry that was dropped: its id when it has one, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DroppedEntry {
    pub id: Option<String>,
    pub reason: String,
}

impl DroppedEntry {
    pub fn label(&self) -> &str {
        self.id.as_deref().unwrap_or("<missing id>")
    }
}

fn default_interruptible() -> bool {
    true
}

impl Default for GlobalIndex {
    fn default() -> Self {
        Self {
            schema_version: 2,
            snippets: Vec::new(),
            third_party: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn an_old_index_upcasts_project_locations_without_changing_other_entries() {
        let old = "schema_version: 1\nsnippets:\n- id: project/demo/spec/thing\n  description: A thing.\n  type: spec\n  scope: local\n  domain: project\n  path: /source/demo/docs/project/spec/thing.md\n  source_project: demo\n- id: acme/spec/thing\n  description: Published thing.\n  type: spec\n  scope: global\n  domain: acme\n  path: acme/spec/thing.md\n  source_project: demo\n";
        let expected = old
            .replace("schema_version: 1", "schema_version: 2")
            .replace(
                "/source/demo/docs/project/spec/thing.md",
                "project/demo/spec/thing.md",
            );
        assert_eq!(
            GlobalIndex::parse(Path::new("index.yaml"), old).unwrap(),
            serde_yaml_ng::from_str::<GlobalIndex>(&expected).unwrap()
        );
    }

    /// An index entry for a procedure `id` in `domain`, with `scope`,
    /// declaring `role`, pending `pending`'s delivering todo when given.
    fn procedure(id: &str, scope: &str, domain: &str, role: &str, pending: Option<&str>) -> String {
        let delivered_by =
            pending.map_or(String::new(), |slug| format!("  delivered_by: [{slug}]\n"));
        format!(
            "- id: {id}\n  description: A procedure.\n  type: procedure\n  scope: {scope}\n  domain: {domain}\n  path: {id}.md\n  source_project: demo\n  role: {role}\n{delivered_by}"
        )
    }

    fn index(entries: &[String]) -> GlobalIndex {
        GlobalIndex::parse(
            Path::new("index.yaml"),
            &format!("schema_version: 2\nsnippets:\n{}", entries.concat()),
        )
        .unwrap()
    }

    #[test]
    fn a_role_is_answered_by_the_published_procedure_that_declares_it_and_its_pending_state() {
        let index = index(&[
            procedure(
                "project/demo/procedure/local",
                "local",
                "project",
                "helper",
                None,
            ),
            procedure("acme/procedure/other", "global", "acme", "reviewer", None),
            procedure(
                "general/procedure/helping",
                "global",
                "general",
                "helper",
                None,
            ),
            procedure(
                "acme/procedure/drafting",
                "global",
                "acme",
                "drafter",
                Some("draft-it"),
            ),
        ]);
        assert_eq!(
            index.procedures_declaring(&RoleName::parse("helper").unwrap()),
            [RoleProcedure::Delivered(&index.snippets[2])]
        );
        assert_eq!(
            index.procedures_declaring(&RoleName::parse("drafter").unwrap()),
            [RoleProcedure::Pending(&index.snippets[3])]
        );
        assert_eq!(
            index.procedures_declaring(&RoleName::parse("nobody").unwrap()),
            []
        );
    }

    #[test]
    fn an_entry_this_version_cannot_decode_is_dropped_and_named_beside_the_others() {
        let text = format!(
            "schema_version: 2\nsnippets:\n{}- id: acme/procedure/broken\n  type: procedure\n- 7\n",
            procedure("acme/procedure/kept", "global", "acme", "helper", None)
        );
        let entries = GlobalIndex::entries(Path::new("index.yaml"), &text).unwrap();
        assert_eq!(
            entries
                .entries
                .iter()
                .map(|entry| entry.id.as_str())
                .collect::<Vec<_>>(),
            ["acme/procedure/kept"]
        );
        insta::assert_debug_snapshot!(entries.dropped, @r#"
        [
            DroppedEntry {
                id: Some(
                    "acme/procedure/broken",
                ),
                reason: "missing field `description`",
            },
            DroppedEntry {
                id: None,
                reason: "invalid type: integer `7`, expected struct IndexEntry",
            },
        ]
        "#);
        assert!(matches!(
            GlobalIndex::entries(Path::new("index.yaml"), "schema_version: 2\nsnippets: 7\n"),
            Err(IndexError::Parse { .. })
        ));
    }

    #[test]
    fn a_newer_index_does_not_require_this_versions_body_shape() {
        let newer = "schema_version: 3\nfuture_field: future_value\n";
        assert!(matches!(
            GlobalIndex::parse(Path::new("index.yaml"), newer),
            Err(IndexError::Version { found: 3, .. })
        ));
    }
}
