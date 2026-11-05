//! The three positions documentation lives in, and the two documents a
//! position may carry at its root: its `index.yaml` and its `schema.yaml`
//! (`docs/local/spec/domain/docs.md`, "Position", "Position index",
//! "Position schema").
//!
//! This module holds the shapes only: where a position sits in a
//! repository and what each root document may say, parsed strictly with
//! the keys its position allows. What an index selects for rendering, and
//! which sections a schema adds to a folder, is decided by their readers.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::ids::{DomainName, SnippetId};

/// The name `docs/general/` is addressed by wherever a domain name is
/// accepted.
pub const GENERAL: &str = "general";

/// A position's index, at its root.
pub const INDEX_FILE: &str = "index.yaml";

/// A position's schema, at its root.
pub const SCHEMA_FILE: &str = "schema.yaml";

/// Names no folder below `docs/domain/` may carry: `general` and `project`
/// are positions of their own, and `third-party` keeps its existing
/// special case.
pub const RESERVED_DOMAIN_FOLDERS: [&str; 3] = [GENERAL, "project", "third-party"];

/// Whether `name` is reserved, so `docs/domain/<name>/` is refused.
pub fn is_reserved_domain_folder(name: &str) -> bool {
    RESERVED_DOMAIN_FOLDERS.contains(&name)
}

/// One of the three places a repository's documentation lives.
///
/// `Domain` never holds `general`: [`Position::of_domain`] maps that name
/// to [`Position::General`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Position {
    /// `docs/general/`: always on.
    General,
    /// `docs/domain/<x>/`: on demand.
    Domain(DomainName),
    /// `docs/project/`: the project's own.
    Project,
}

impl Position {
    /// The published position a domain name addresses.
    pub fn of_domain(domain: &DomainName) -> Self {
        if domain.as_str() == GENERAL {
            Position::General
        } else {
            Position::Domain(domain.clone())
        }
    }

    /// The position's root, relative to the repository root, without a
    /// trailing `/`.
    pub fn dir(&self) -> String {
        match self {
            Position::General => format!("docs/{GENERAL}"),
            Position::Domain(domain) => format!("docs/domain/{domain}"),
            Position::Project => "docs/project".to_owned(),
        }
    }

    /// The domain name a published position is addressed by; `None` for
    /// the project.
    pub fn domain(&self) -> Option<DomainName> {
        match self {
            Position::General => DomainName::parse(GENERAL).ok(),
            Position::Domain(domain) => Some(domain.clone()),
            Position::Project => None,
        }
    }
}

impl fmt::Display for Position {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/", self.dir())
    }
}

/// A supported agent CLI: a deployment target of the global agent files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentCli {
    Claude,
    Codex,
    Gemini,
    Deepseek,
}

/// `docs/general/index.yaml`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GeneralIndex {
    /// Rendered in full, each with its closure; may name any domain's ids.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub expanded: Vec<SnippetId>,
    /// Rendered as one listing line each; may name any domain's ids.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub listed: Vec<SnippetId>,
    /// Snippets appended to one agent CLI's global file only.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub agents: BTreeMap<AgentCli, Vec<SnippetId>>,
}

/// `docs/domain/<x>/index.yaml`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DomainIndex {
    /// The one line saying what the domain is about.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub expanded: Vec<SnippetId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub listed: Vec<SnippetId>,
}

/// `docs/project/index.yaml`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectIndex {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub expanded: Vec<SnippetId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub listed: Vec<SnippetId>,
}

/// A position's `index.yaml`, read with the keys its position allows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PositionIndex {
    General(GeneralIndex),
    Domain(DomainIndex),
    Project(ProjectIndex),
}

impl PositionIndex {
    /// Parses the `index.yaml` of `position`. A key the position does not
    /// allow, a value of the wrong shape, a malformed id or an agent CLI
    /// instrukt does not deploy to is an error. A file holding no document
    /// (empty, or comments only) is an index that selects nothing.
    pub fn parse(position: &Position, text: &str) -> Result<Self, serde_yaml_ng::Error> {
        Ok(match position {
            Position::General => PositionIndex::General(document(text)?),
            Position::Domain(_) => PositionIndex::Domain(document(text)?),
            Position::Project => PositionIndex::Project(document(text)?),
        })
    }

    /// The ids rendered in full.
    pub fn expanded(&self) -> &[SnippetId] {
        match self {
            PositionIndex::General(index) => &index.expanded,
            PositionIndex::Domain(index) => &index.expanded,
            PositionIndex::Project(index) => &index.expanded,
        }
    }

    /// The ids rendered as one listing line each.
    pub fn listed(&self) -> &[SnippetId] {
        match self {
            PositionIndex::General(index) => &index.listed,
            PositionIndex::Domain(index) => &index.listed,
            PositionIndex::Project(index) => &index.listed,
        }
    }
}

/// The H2 sections one folder path of a position schema adds.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FolderSections {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub optional: Vec<String>,
}

/// A position's `schema.yaml`: per folder path relative to the position
/// root (`spec/feature`), the sections it adds for the snippets under it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PositionSchema {
    pub folders: BTreeMap<String, FolderSections>,
}

impl PositionSchema {
    /// Parses a `schema.yaml`. A folder carrying a key other than
    /// `required` and `optional`, or a value of the wrong shape, is an
    /// error. A file holding no document is a schema that adds nothing.
    pub fn parse(text: &str) -> Result<Self, serde_yaml_ng::Error> {
        document(text)
    }
}

/// Parses one YAML document; a text holding none is the default value.
fn document<T: Default + for<'de> Deserialize<'de>>(text: &str) -> Result<T, serde_yaml_ng::Error> {
    Ok(serde_yaml_ng::from_str::<Option<T>>(text)?.unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(text: &str) -> SnippetId {
        SnippetId::parse(text).unwrap()
    }

    fn domain(name: &str) -> DomainName {
        DomainName::parse(name).unwrap()
    }

    #[test]
    fn a_domain_name_addresses_its_position() {
        assert_eq!(Position::of_domain(&domain("general")), Position::General);
        assert_eq!(
            Position::of_domain(&domain("sdlc")),
            Position::Domain(domain("sdlc"))
        );
    }

    #[test]
    fn each_position_has_its_root() {
        assert_eq!(Position::General.dir(), "docs/general");
        assert_eq!(Position::Domain(domain("sdlc")).dir(), "docs/domain/sdlc");
        assert_eq!(Position::Project.dir(), "docs/project");
    }

    #[test]
    fn general_project_and_third_party_are_reserved_domain_folders() {
        for name in ["general", "project", "third-party"] {
            assert!(is_reserved_domain_folder(name), "{name}");
        }
        assert!(!is_reserved_domain_folder("sdlc"));
    }

    #[test]
    fn the_general_index_reads_its_three_keys() {
        let text = "expanded: [general/policy/a]\nlisted: [sdlc/policy/b]\n\
                    agents:\n  claude: [general/policy/agents/claude]\n";
        let PositionIndex::General(index) = PositionIndex::parse(&Position::General, text).unwrap()
        else {
            panic!("a general index");
        };
        assert_eq!(index.expanded, [id("general/policy/a")]);
        assert_eq!(index.listed, [id("sdlc/policy/b")]);
        assert_eq!(
            index.agents,
            BTreeMap::from([(AgentCli::Claude, vec![id("general/policy/agents/claude")])])
        );
    }

    #[test]
    fn a_domain_index_reads_its_description() {
        let index =
            PositionIndex::parse(&Position::Domain(domain("sdlc")), "description: Code\n").unwrap();
        assert_eq!(
            index,
            PositionIndex::Domain(DomainIndex {
                description: Some("Code".to_owned()),
                ..DomainIndex::default()
            })
        );
    }

    #[test]
    fn each_position_refuses_the_keys_it_does_not_allow() {
        let sdlc = Position::Domain(domain("sdlc"));
        for (position, text) in [
            (&sdlc, "agents: {}\n"),
            (&Position::Project, "agents: {}\n"),
            (&Position::Project, "description: x\n"),
            (&Position::General, "description: x\n"),
            (&Position::General, "bundles: []\n"),
        ] {
            assert!(
                PositionIndex::parse(position, text).is_err(),
                "{position} {text}"
            );
        }
    }

    #[test]
    fn an_agent_instrukt_does_not_deploy_to_is_refused() {
        assert!(PositionIndex::parse(&Position::General, "agents:\n  cursor: []\n").is_err());
    }

    #[test]
    fn an_index_with_no_document_selects_nothing() {
        let index = PositionIndex::parse(&Position::Project, "# a modeline only\n").unwrap();
        assert!(index.expanded().is_empty() && index.listed().is_empty());
    }

    #[test]
    fn a_schema_adds_sections_per_folder() {
        let schema = PositionSchema::parse(
            "spec/feature:\n  required: [Use cases]\nspec/domain:\n  optional: [Notes]\n",
        )
        .unwrap();
        assert_eq!(
            schema.folders["spec/feature"].required,
            ["Use cases".to_owned()]
        );
        assert_eq!(schema.folders["spec/domain"].optional, ["Notes".to_owned()]);
    }

    #[test]
    fn a_schema_folder_refuses_keys_other_than_required_and_optional() {
        assert!(PositionSchema::parse("spec/feature:\n  dropped: [What it is]\n").is_err());
    }
}
