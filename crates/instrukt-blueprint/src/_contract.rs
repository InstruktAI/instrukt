//! The graph contract (`docs/local/spec/feature/blueprint/graph-engine.md`):
//! one JSON object with `nodes` (`id`, `source_file`) and `links`
//! (`source`, `target`, `relation`), validated before anything is stored or
//! answered.
//!
//! Validation is the anti-corruption layer between an engine and the store:
//! a document that is not an object, lacks `nodes` or `links` as lists, or
//! carries a `source_file` that is not a repository-relative path is
//! refused. Nodes and links that are not objects or lack a required member
//! are skipped, not refused. Members beyond the contract are kept verbatim
//! in the stored document.

use std::fmt;

use serde_json::{Map, Value};

/// Why a document does not satisfy the graph contract.
#[derive(Debug, thiserror::Error)]
pub enum ContractError {
    #[error("the graph is not valid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("the graph is not a JSON object")]
    NotObject,
    #[error("the graph has no `{0}` list")]
    MissingList(&'static str),
    #[error("the graph's `{0}` is not a list")]
    NotList(&'static str),
    #[error("node {index}: `source_file` {problem}")]
    SourceFile {
        index: usize,
        problem: SourceFileProblem,
    },
}

/// How a node's `source_file` breaks the contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceFileProblem {
    NotString,
    Absolute(String),
    ParentSegment(String),
    Backslash(String),
}

impl fmt::Display for SourceFileProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotString => f.write_str("is not a string"),
            Self::Absolute(path) => write!(f, "`{path}` is absolute"),
            Self::ParentSegment(path) => write!(f, "`{path}` contains a `..` segment"),
            Self::Backslash(path) => write!(f, "`{path}` contains a backslash"),
        }
    }
}

/// A link's relation. The named variants are the dependency relations
/// (`docs/local/spec/domain/blueprint.md`); every other relation — including
/// containment — is kept as [`Relation::Other`] and never traversed as a
/// dependency.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Relation {
    Calls,
    IndirectCall,
    References,
    Imports,
    ImportsFrom,
    ReExports,
    Inherits,
    Extends,
    Implements,
    Uses,
    MixesIn,
    Embeds,
    Other(String),
}

impl Relation {
    pub fn parse(value: &str) -> Self {
        match value {
            "calls" => Self::Calls,
            "indirect_call" => Self::IndirectCall,
            "references" => Self::References,
            "imports" => Self::Imports,
            "imports_from" => Self::ImportsFrom,
            "re_exports" => Self::ReExports,
            "inherits" => Self::Inherits,
            "extends" => Self::Extends,
            "implements" => Self::Implements,
            "uses" => Self::Uses,
            "mixes_in" => Self::MixesIn,
            "embeds" => Self::Embeds,
            other => Self::Other(other.to_owned()),
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::Calls => "calls",
            Self::IndirectCall => "indirect_call",
            Self::References => "references",
            Self::Imports => "imports",
            Self::ImportsFrom => "imports_from",
            Self::ReExports => "re_exports",
            Self::Inherits => "inherits",
            Self::Extends => "extends",
            Self::Implements => "implements",
            Self::Uses => "uses",
            Self::MixesIn => "mixes_in",
            Self::Embeds => "embeds",
            Self::Other(other) => other,
        }
    }

    /// Whether a link of this relation makes its source depend on its target.
    pub fn is_dependency(&self) -> bool {
        !matches!(self, Self::Other(_))
    }
}

impl fmt::Display for Relation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A node's optional `source_location`: `L<line>` or `L<line>-<line>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Location {
    pub start: u32,
    pub end: Option<u32>,
}

impl Location {
    /// Parses `L<line>` or `L<line>-<line>`; anything else is no location.
    pub fn parse(value: &str) -> Option<Self> {
        let rest = value.strip_prefix('L')?;
        let (start, end) = match rest.split_once('-') {
            Some((start, end)) => (start, Some(end.strip_prefix('L').unwrap_or(end))),
            None => (rest, None),
        };
        let start = start.parse().ok()?;
        let end = match end {
            Some(end) => Some(end.parse().ok()?),
            None => None,
        };
        Some(Self { start, end })
    }
}

/// A node that carries the contract's required members.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    pub id: String,
    /// Repository-relative, validated.
    pub source_file: String,
    pub label: Option<String>,
    pub location: Option<Location>,
}

impl Node {
    /// Whether the node is defined in `file`, comparing equivalent
    /// spellings of a repository-relative path as equal
    /// ([`normalize_path`]).
    pub fn is_in_file(&self, file: &str) -> bool {
        normalize_path(&self.source_file) == normalize_path(file)
    }

    /// How answers name the node: its label, else its id.
    pub fn name(&self) -> &str {
        self.label.as_deref().unwrap_or(&self.id)
    }
}

/// A link that carries the contract's required members.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    pub source: String,
    pub target: String,
    pub relation: Relation,
}

/// A graph that satisfies the contract: the engine's document as given,
/// with the nodes and links answers use.
#[derive(Debug, Clone)]
pub struct Graph {
    document: Map<String, Value>,
    nodes: Vec<Node>,
    links: Vec<Link>,
}

impl Graph {
    /// Validates JSON bytes against the contract.
    pub fn from_json(bytes: &[u8]) -> Result<Self, ContractError> {
        Self::from_value(serde_json::from_slice(bytes)?)
    }

    /// Validates a JSON value against the contract.
    pub fn from_value(value: Value) -> Result<Self, ContractError> {
        let Value::Object(document) = value else {
            return Err(ContractError::NotObject);
        };
        let raw_nodes = list(&document, "nodes")?;
        let raw_links = list(&document, "links")?;
        let mut nodes = Vec::new();
        for (index, raw) in raw_nodes.iter().enumerate() {
            let Value::Object(raw) = raw else { continue };
            let source_file = match raw.get("source_file") {
                None => None,
                Some(Value::String(path)) => {
                    check_relative(path)
                        .map_err(|problem| ContractError::SourceFile { index, problem })?;
                    Some(path.clone())
                }
                Some(_) => {
                    return Err(ContractError::SourceFile {
                        index,
                        problem: SourceFileProblem::NotString,
                    });
                }
            };
            let (Some(id), Some(source_file)) = (string(raw, "id"), source_file) else {
                continue;
            };
            nodes.push(Node {
                id,
                source_file,
                label: string(raw, "label"),
                location: raw
                    .get("source_location")
                    .and_then(Value::as_str)
                    .and_then(Location::parse),
            });
        }
        let links = raw_links
            .iter()
            .filter_map(Value::as_object)
            .filter_map(|raw| {
                Some(Link {
                    source: string(raw, "source")?,
                    target: string(raw, "target")?,
                    relation: Relation::parse(raw.get("relation")?.as_str()?),
                })
            })
            .collect();
        Ok(Self {
            document,
            nodes,
            links,
        })
    }

    /// The document as stored: every member the engine emitted.
    pub fn to_json(&self) -> Result<Vec<u8>, ContractError> {
        Ok(serde_json::to_vec(&self.document)?)
    }

    pub fn nodes(&self) -> &[Node] {
        &self.nodes
    }

    pub fn links(&self) -> &[Link] {
        &self.links
    }

    /// The nodes whose `source_file` is `file`, in any equivalent spelling
    /// ([`normalize_path`]).
    pub fn nodes_in_file<'g>(&'g self, file: &'g str) -> impl Iterator<Item = &'g Node> + 'g {
        let file = normalize_path(file);
        self.nodes
            .iter()
            .filter(move |node| normalize_path(&node.source_file) == file)
    }

    /// The nodes with id `id` (an engine may repeat an id).
    pub fn nodes_with_id<'g>(&'g self, id: &'g str) -> impl Iterator<Item = &'g Node> + 'g {
        self.nodes.iter().filter(move |node| node.id == id)
    }
}

fn list<'d>(
    document: &'d Map<String, Value>,
    field: &'static str,
) -> Result<&'d [Value], ContractError> {
    match document.get(field) {
        None => Err(ContractError::MissingList(field)),
        Some(Value::Array(items)) => Ok(items),
        Some(_) => Err(ContractError::NotList(field)),
    }
}

fn string(raw: &Map<String, Value>, key: &str) -> Option<String> {
    raw.get(key).and_then(Value::as_str).map(str::to_owned)
}

/// One spelling of a repository-relative path: `.` segments and empty
/// segments (repeated, leading `./` or trailing slashes) are dropped, so
/// `./src//a.rs` and `src/a.rs` compare equal. The contract already refuses
/// `..` segments, so no other spelling names the same file.
pub fn normalize_path(path: &str) -> String {
    path.split('/')
        .filter(|segment| !segment.is_empty() && *segment != ".")
        .collect::<Vec<_>>()
        .join("/")
}

fn check_relative(path: &str) -> Result<(), SourceFileProblem> {
    if path.contains('\\') {
        return Err(SourceFileProblem::Backslash(path.to_owned()));
    }
    if path.starts_with('/') || std::path::Path::new(path).is_absolute() {
        return Err(SourceFileProblem::Absolute(path.to_owned()));
    }
    if path.split('/').any(|segment| segment == "..") {
        return Err(SourceFileProblem::ParentSegment(path.to_owned()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_document_that_is_not_an_object_or_lacks_lists_is_refused() {
        assert!(matches!(
            Graph::from_value(json!([])),
            Err(ContractError::NotObject)
        ));
        assert!(matches!(
            Graph::from_value(json!({"links": []})),
            Err(ContractError::MissingList("nodes"))
        ));
        assert!(matches!(
            Graph::from_value(json!({"nodes": [], "links": {}})),
            Err(ContractError::NotList("links"))
        ));
        assert!(matches!(
            Graph::from_json(b"not json"),
            Err(ContractError::Json(_))
        ));
    }

    #[test]
    fn a_source_file_must_be_a_repository_relative_string() {
        for (bad, expected) in [
            (json!(3), SourceFileProblem::NotString),
            (
                json!("/abs/a.rs"),
                SourceFileProblem::Absolute("/abs/a.rs".into()),
            ),
            (
                json!("src/../a.rs"),
                SourceFileProblem::ParentSegment("src/../a.rs".into()),
            ),
            (
                json!("src\\a.rs"),
                SourceFileProblem::Backslash("src\\a.rs".into()),
            ),
        ] {
            let doc = json!({"nodes": [{"id": "ok", "source_file": "a.rs"}, {"id": "x", "source_file": bad}], "links": []});
            match Graph::from_value(doc) {
                Err(ContractError::SourceFile { index: 1, problem }) => {
                    assert_eq!(problem, expected)
                }
                other => panic!("expected refusal, got {other:?}"),
            }
        }
        // `..` only as a whole segment.
        assert!(
            Graph::from_value(
                json!({"nodes": [{"id": "a", "source_file": "a..b/c.rs"}], "links": []})
            )
            .is_ok()
        );
    }

    #[test]
    fn incomplete_nodes_and_links_are_skipped_and_extra_members_kept() {
        let doc = json!({
            "directed": true,
            "nodes": [
                {"id": "a", "source_file": "a.rs", "label": "A", "source_location": "L3-9", "kind": "fn"},
                {"id": "b"},
                "not an object",
                {"source_file": "c.rs"}
            ],
            "links": [
                {"source": "a", "target": "b", "relation": "calls"},
                {"source": "a", "target": "b"},
                7
            ]
        });
        let graph = Graph::from_value(doc.clone()).unwrap();
        assert_eq!(graph.nodes().len(), 1);
        let node = &graph.nodes()[0];
        assert_eq!(node.name(), "A");
        assert_eq!(
            node.location,
            Some(Location {
                start: 3,
                end: Some(9)
            })
        );
        assert_eq!(graph.links().len(), 1);
        let stored: Value = serde_json::from_slice(&graph.to_json().unwrap()).unwrap();
        assert_eq!(stored, doc);
    }

    #[test]
    fn locations_parse_line_or_range() {
        assert_eq!(
            Location::parse("L12"),
            Some(Location {
                start: 12,
                end: None
            })
        );
        assert_eq!(
            Location::parse("L1-4"),
            Some(Location {
                start: 1,
                end: Some(4)
            })
        );
        assert_eq!(Location::parse("12"), None);
        assert_eq!(Location::parse("Lx"), None);
    }

    #[test]
    fn only_the_dependency_relations_are_dependencies() {
        for name in [
            "calls",
            "indirect_call",
            "references",
            "imports",
            "imports_from",
            "re_exports",
            "inherits",
            "extends",
            "implements",
            "uses",
            "mixes_in",
            "embeds",
        ] {
            let relation = Relation::parse(name);
            assert!(relation.is_dependency(), "{name}");
            assert_eq!(relation.as_str(), name);
        }
        assert!(!Relation::parse("contains").is_dependency());
        assert!(!Relation::parse("method").is_dependency());
    }
}
