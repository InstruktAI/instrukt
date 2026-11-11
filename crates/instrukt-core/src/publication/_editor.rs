//! Editor schemas: the JSON Schemas instrukt generates from its own
//! document types into a repository's git-ignored `.instrukt/schemas/`,
//! and the modeline at the top of each file they describe
//! (`docs/project/spec/feature/publication/editor-schemas.md`).
//!
//! `instrukt init` runs [`refresh_editor_schemas`], which writes the schemas
//! and the modelines; every sync that publishes runs [`write_editor_schemas`],
//! which writes the git-ignored schemas and no committed file. The pre-commit
//! section brings the modeline of each file being committed to the top with
//! [`committed_modeline`]. The schemas are a pure function of the installed
//! binary; the modelines are relative paths inside the repository, never a
//! home path.

use std::path::{Path, PathBuf};

use super::_tx::confined;
use super::PublicationError;
use crate::config::{self, CONFIG_FILE_NAME};
use crate::docs::{
    DomainIndex, GeneralIndex, INDEX_FILE, Position, PositionSchema, ProjectIndex, SCHEMA_FILE,
    is_reserved_domain_folder,
};
use crate::fsx::atomic_write;
use crate::ids::DomainName;
use crate::modeline::{leading, with_modeline};

/// Where the schemas live, relative to the repository root.
pub const SCHEMAS_DIR: &str = ".instrukt/schemas";

/// One generated schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EditorSchema {
    GeneralIndex,
    DomainIndex,
    ProjectIndex,
    Snippets,
    Configuration,
}

impl EditorSchema {
    const ALL: [EditorSchema; 5] = [
        EditorSchema::GeneralIndex,
        EditorSchema::DomainIndex,
        EditorSchema::ProjectIndex,
        EditorSchema::Snippets,
        EditorSchema::Configuration,
    ];

    fn file_name(self) -> &'static str {
        match self {
            EditorSchema::GeneralIndex => "general-index.schema.json",
            EditorSchema::DomainIndex => "domain-index.schema.json",
            EditorSchema::ProjectIndex => "project-index.schema.json",
            EditorSchema::Snippets => "snippet-schema.schema.json",
            EditorSchema::Configuration => "instrukt.schema.json",
        }
    }

    /// The schema's bytes: pretty JSON with a final newline.
    fn bytes(self) -> Vec<u8> {
        let value = match self {
            EditorSchema::GeneralIndex => GeneralIndex::editor_schema(),
            EditorSchema::DomainIndex => DomainIndex::editor_schema(),
            EditorSchema::ProjectIndex => ProjectIndex::editor_schema(),
            EditorSchema::Snippets => PositionSchema::editor_schema(),
            EditorSchema::Configuration => config::editor_schema(),
        };
        let mut text = serde_json::to_string_pretty(&value).unwrap_or_default();
        text.push('\n');
        text.into_bytes()
    }

    /// The index schema of `position`.
    fn index_of(position: &Position) -> Self {
        match position {
            Position::General => EditorSchema::GeneralIndex,
            Position::Domain(_) => EditorSchema::DomainIndex,
            Position::Project => EditorSchema::ProjectIndex,
        }
    }
}

/// What the refresh did to one file, by its path relative to the
/// repository root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorChange {
    pub path: PathBuf,
    pub kind: EditorChangeKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorChangeKind {
    /// A schema file was written (absent, or holding other content).
    SchemaWritten,
    /// The modeline was inserted as the file's first line.
    ModelineInserted,
    /// A modeline naming another path was replaced.
    ModelineReplaced,
    /// Not written: the path is, or lies behind, a symbolic link.
    Linked,
}

/// What to do about a path the refresh would write through a symbolic
/// link.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnLink {
    /// Leave it unwritten and report it (`init`).
    Report,
    /// Fail the operation (`sync`).
    Refuse,
}

/// Writes every editor schema below `root`'s `.instrukt/schemas/` and
/// brings the modeline of every file they describe that exists to the top.
/// A file already holding the generated content or its modeline is left
/// untouched. Any other failure to write fails the refresh, naming the
/// file.
pub fn refresh_editor_schemas(
    root: &Path,
    on_link: OnLink,
) -> Result<Vec<EditorChange>, PublicationError> {
    let mut changes = write_editor_schemas(root, on_link)?;
    for (rel, schema) in described(root)? {
        if !exists(&root.join(&rel))? {
            continue;
        }
        let Some(path) = writable(root, &rel, on_link, &mut changes)? else {
            continue;
        };
        let Some(bytes) = read(&path)? else {
            continue;
        };
        let Some(next) = with_modeline(&bytes, &schema_path_from(&rel, schema)) else {
            continue;
        };
        let kind = if leading(&bytes).is_some() {
            EditorChangeKind::ModelineReplaced
        } else {
            EditorChangeKind::ModelineInserted
        };
        atomic_write(&path, &next)?;
        changes.push(EditorChange { path: rel, kind });
    }
    Ok(changes)
}

/// Writes every editor schema below `root`'s git-ignored
/// `.instrukt/schemas/`, leaving every committed file as it is. A schema
/// already holding the generated content is left untouched; any other
/// failure to write fails the refresh, naming the file.
pub fn write_editor_schemas(
    root: &Path,
    on_link: OnLink,
) -> Result<Vec<EditorChange>, PublicationError> {
    let mut changes = Vec::new();
    for schema in EditorSchema::ALL {
        let rel = Path::new(SCHEMAS_DIR).join(schema.file_name());
        let Some(path) = writable(root, &rel, on_link, &mut changes)? else {
            continue;
        };
        let bytes = schema.bytes();
        if read(&path)?.as_deref() != Some(bytes.as_slice()) {
            atomic_write(&path, &bytes)?;
            changes.push(EditorChange {
                path: rel,
                kind: EditorChangeKind::SchemaWritten,
            });
        }
    }
    Ok(changes)
}

/// `bytes`, the content of `rel` (relative to the repository root) as it is
/// being committed, starting with the modeline naming its editor schema;
/// `None` when `rel` is no file an editor schema describes or it already
/// starts with that modeline.
pub fn committed_modeline(rel: &Path, bytes: &[u8]) -> Option<Vec<u8>> {
    let schema = described_by(rel)?;
    with_modeline(bytes, &schema_path_from(rel, schema))
}

/// Whether an editor schema describes the file at `rel`, relative to the
/// repository root.
pub fn editor_schema_describes(rel: &Path) -> bool {
    described_by(rel).is_some()
}

/// The editor schema describing the file at `rel`, relative to the
/// repository root: the project layer, and each position's `index.yaml` and
/// `schema.yaml`.
fn described_by(rel: &Path) -> Option<EditorSchema> {
    if rel == Path::new(CONFIG_FILE_NAME) {
        return Some(EditorSchema::Configuration);
    }
    let file = rel.file_name()?.to_str()?;
    let dir = rel.parent()?;
    let position = if dir == Path::new(&Position::General.dir()) {
        Position::General
    } else if dir == Path::new(&Position::Project.dir()) {
        Position::Project
    } else {
        let name = dir.strip_prefix("docs/domain").ok()?.to_str()?;
        if is_reserved_domain_folder(name) {
            return None;
        }
        Position::Domain(DomainName::parse(name).ok()?)
    };
    if file == INDEX_FILE {
        Some(EditorSchema::index_of(&position))
    } else if file == SCHEMA_FILE {
        Some(EditorSchema::Snippets)
    } else {
        None
    }
}

/// `root/rel` resolved for writing; `None`, with the path reported, when
/// it is or lies behind a symbolic link and `on_link` says to report it.
fn writable(
    root: &Path,
    rel: &Path,
    on_link: OnLink,
    changes: &mut Vec<EditorChange>,
) -> Result<Option<PathBuf>, PublicationError> {
    match confined(root, rel) {
        Ok(path) => Ok(Some(path)),
        Err(PublicationError::Unconfined { .. }) if on_link == OnLink::Report => {
            changes.push(EditorChange {
                path: rel.to_path_buf(),
                kind: EditorChangeKind::Linked,
            });
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

fn exists(path: &Path) -> Result<bool, PublicationError> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(PublicationError::Io {
            path: path.to_path_buf(),
            source,
        }),
    }
}

/// Every file below `root` an editor schema describes, with its schema:
/// the project layer, and each position's `index.yaml` and `schema.yaml`.
fn described(root: &Path) -> Result<Vec<(PathBuf, EditorSchema)>, PublicationError> {
    let mut positions = vec![Position::General];
    let domains = root.join("docs/domain");
    match std::fs::read_dir(&domains) {
        Ok(entries) => {
            let mut names = Vec::new();
            for entry in entries {
                let entry = entry.map_err(|source| PublicationError::Io {
                    path: domains.clone(),
                    source,
                })?;
                let name = entry.file_name().to_string_lossy().into_owned();
                if is_reserved_domain_folder(&name) {
                    continue;
                }
                if let Ok(domain) = DomainName::parse(&name) {
                    names.push(domain);
                }
            }
            names.sort();
            positions.extend(names.into_iter().map(Position::Domain));
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(source) => {
            return Err(PublicationError::Io {
                path: domains,
                source,
            });
        }
    }
    positions.push(Position::Project);
    let mut files = vec![(PathBuf::from(CONFIG_FILE_NAME), EditorSchema::Configuration)];
    for position in &positions {
        let dir = PathBuf::from(position.dir());
        files.push((dir.join(INDEX_FILE), EditorSchema::index_of(position)));
        files.push((dir.join(SCHEMA_FILE), EditorSchema::Snippets));
    }
    Ok(files)
}

/// The path from the directory of `file` (relative to the repository
/// root) to `schema` inside the repository's `.instrukt/schemas/`.
fn schema_path_from(file: &Path, schema: EditorSchema) -> String {
    let depth = file.parent().map_or(0, |dir| dir.components().count());
    let mut path = "../".repeat(depth);
    path.push_str(SCHEMAS_DIR);
    path.push('/');
    path.push_str(schema.file_name());
    path
}

/// The bytes at `path`; `None` when absent.
fn read(path: &Path) -> Result<Option<Vec<u8>>, PublicationError> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(PublicationError::Io {
            path: path.to_path_buf(),
            source,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_modeline_path_climbs_from_the_file_to_the_repository_root() {
        assert_eq!(
            schema_path_from(Path::new("instrukt.yaml"), EditorSchema::Configuration),
            ".instrukt/schemas/instrukt.schema.json"
        );
        assert_eq!(
            schema_path_from(
                Path::new("docs/domain/sdlc/index.yaml"),
                EditorSchema::DomainIndex
            ),
            "../../../.instrukt/schemas/domain-index.schema.json"
        );
        assert_eq!(
            schema_path_from(
                Path::new("docs/project/schema.yaml"),
                EditorSchema::Snippets
            ),
            "../../.instrukt/schemas/snippet-schema.schema.json"
        );
    }

    #[test]
    fn the_project_layer_is_described_by_the_configuration_schema() {
        assert_eq!(
            described_by(Path::new("instrukt.yaml")),
            Some(EditorSchema::Configuration)
        );
    }

    #[test]
    fn a_domain_index_is_described_by_the_domain_index_schema() {
        assert_eq!(
            described_by(Path::new("docs/domain/sdlc/index.yaml")),
            Some(EditorSchema::DomainIndex)
        );
    }

    #[test]
    fn a_position_schema_file_is_described_by_the_snippet_schema() {
        assert_eq!(
            described_by(Path::new("docs/project/schema.yaml")),
            Some(EditorSchema::Snippets)
        );
    }

    #[test]
    fn an_index_below_a_position_takes_no_modeline() {
        let rel = Path::new("docs/project/spec/index.yaml");
        assert_eq!(described_by(rel), None);
        assert_eq!(committed_modeline(rel, b"a: 1\n"), None);
    }

    #[test]
    fn a_file_already_starting_with_its_modeline_is_left_as_it_is() {
        let rel = Path::new("instrukt.yaml");
        let marked = committed_modeline(rel, b"a: 1\n").unwrap();
        assert_eq!(committed_modeline(rel, &marked), None);
    }

    #[test]
    fn the_schemas_are_the_same_bytes_every_time() {
        for schema in EditorSchema::ALL {
            assert_eq!(schema.bytes(), schema.bytes(), "{}", schema.file_name());
            assert!(
                serde_json::from_slice::<serde_json::Value>(&schema.bytes())
                    .is_ok_and(|value| value.is_object()),
                "{} is a JSON Schema object",
                schema.file_name()
            );
        }
    }
}
