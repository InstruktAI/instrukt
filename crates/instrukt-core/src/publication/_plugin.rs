//! A company's Claude plugin
//! (`project/instrukt/spec/feature/publication/company-plugin`): the folder
//! `plugins/<key>/` of a company's repository, holding one skill whose main
//! file carries the company passage and each `expanded` document in full,
//! and one file for every other document the index selects or a required
//! read reaches, each reference between documents naming where the plugin
//! carries the document it names.
//!
//! The documents are read once, by the reading the company's `AGENTS.md` is
//! rendered from ([`super::_company::read`]); every finding of that reading
//! is a finding here. The plugin adds its own: a repository that holds no
//! company index or is a project, a reference to a document the plugin does
//! not carry or to a file that is no document, frontmatter that is no YAML
//! mapping, and a `visibility` that is unknown or a `public` document that
//! depends on an `internal` one.

#![allow(
    dead_code,
    reason = "the plugin's plan and its command, the next commits, are its callers"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde_yaml_ng::Value;

use super::_company::{self, COMPANY_PASSAGE, DOCS, Selection, listed_line};
use super::_render::{RenderFinding, company_home, finish};
use super::_source::{Committed, EntryIndex};
use super::PublicationError;
use crate::config::{CONFIG_FILE_NAME, RootDocument, classify_root};
use crate::docs::{CompanyFrontmatter, CompanyPath, CompanyReference, CompanyText, company_paths};
use crate::ids::ProjectSlug;

/// The folder below a repository's root that holds its plugins.
const PLUGINS: &str = "plugins";

/// The skill's main file, inside the skill's folder.
const SKILL_FILE: &str = "SKILL.md";

/// The folder of the skill that holds each document carried as a file of
/// its own, at its path relative to `docs/`.
const REFERENCES: &str = "references";

/// The name people write a company by: one line of text that is not blank.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompanyName(String);

impl CompanyName {
    /// `text` as a company's name, or why it is none.
    pub fn parse(text: &str) -> Result<Self, CompanyNameError> {
        if text.contains(['\n', '\r']) {
            Err(CompanyNameError::MoreThanOneLine)
        } else if text.trim().is_empty() {
            Err(CompanyNameError::Blank)
        } else {
            Ok(Self(text.to_owned()))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::str::FromStr for CompanyName {
    type Err = CompanyNameError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Self::parse(text)
    }
}

impl std::fmt::Display for CompanyName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Why a text is no company's name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CompanyNameError {
    #[error("a company's name is the one line people write the company by; this one is blank")]
    Blank,
    #[error(
        "a company's name is the one line people write the company by; this one holds a line break"
    )]
    MoreThanOneLine,
}

/// A company document's visibility: the closed set of two.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Visibility {
    Internal,
    Public,
}

impl Visibility {
    /// The visibility a document's frontmatter declares, `internal` without
    /// one, or the value as written when it is neither.
    fn of(declared: Option<&Value>) -> Result<Self, String> {
        match declared {
            None => Ok(Self::Internal),
            Some(Value::String(text)) if text == "internal" => Ok(Self::Internal),
            Some(Value::String(text)) if text == "public" => Ok(Self::Public),
            Some(Value::String(text)) => Err(text.clone()),
            Some(other) => Err(serde_yaml_ng::to_string(other)
                .map_or_else(|_| format!("{other:?}"), |text| text.trim().to_owned())),
        }
    }
}

/// Something the plugin's rendering found that leaves every file as it is.
/// Each names the file it is about first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginFinding {
    /// A finding of the reading the company's `AGENTS.md` is rendered from.
    Reading(RenderFinding),
    /// The root `instrukt.yaml` cannot be read, so whether the repository is
    /// a company's is unknown.
    UnreadableRoot { reason: String },
    /// The root `instrukt.yaml` makes the repository a project.
    ProjectRepository,
    /// The repository holds no `docs/index.yaml`.
    NoIndex,
    /// A carried document's frontmatter is no YAML mapping: why.
    FrontmatterInvalid { document: String, reason: String },
    /// A carried document's `visibility` is neither `internal` nor `public`.
    UnknownVisibility { document: String, value: String },
    /// A `public` document requires or references an `internal` one.
    PublicOnInternal { document: String, internal: String },
    /// A reference names a company document the plugin does not carry.
    Uncarried { document: String, reference: String },
    /// A reference names a file under `docs/` that is no company document.
    NotADocument { document: String, reference: String },
}

impl std::fmt::Display for PluginFinding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Reading(finding) => write!(f, "{finding}"),
            Self::UnreadableRoot { reason } => write!(
                f,
                "{CONFIG_FILE_NAME}: cannot be read ({reason}); correct it, and the company's plugin is rendered once it can be"
            ),
            Self::ProjectRepository => write!(
                f,
                "{CONFIG_FILE_NAME}: makes this repository a project, and a company's plugin is rendered from a company's repository; run `instrukt docs plugin` there"
            ),
            Self::NoIndex => write!(
                f,
                "{}: missing, so the repository holds no company knowledge to render; create it to select the documents the plugin carries",
                _company::COMPANY_INDEX
            ),
            Self::FrontmatterInvalid { document, reason } => write!(
                f,
                "{document}: its frontmatter cannot be read ({reason}); correct it to a YAML mapping, and nothing is rendered until it is"
            ),
            Self::UnknownVisibility { document, value } => write!(
                f,
                "{document}: its `visibility` is `{value}`; set it to `internal` or `public`, or remove it to make the document `internal`"
            ),
            Self::PublicOnInternal { document, internal } => write!(
                f,
                "{document}: is `public` but requires or references {internal}, which is `internal`; make {internal} `public`, make {document} `internal`, or remove the dependency"
            ),
            Self::Uncarried {
                document,
                reference,
            } => write!(
                f,
                "{document}: references `{reference}`, a company document the plugin does not carry because the index does not select it and no required read reaches it; select it in {} or remove the reference",
                _company::COMPANY_INDEX
            ),
            Self::NotADocument {
                document,
                reference,
            } => write!(
                f,
                "{document}: references `{reference}`, a file under `docs/` that is no company document, which the plugin cannot carry; name a Markdown document instead or remove the reference"
            ),
        }
    }
}

/// The plugin of `key`, named `name`, rendered from `tree`: every file of
/// it but the manifest, by repository-relative path, or the findings that
/// leave every file as it is.
pub(super) enum Rendered {
    Files(BTreeMap<String, String>),
    Findings(Vec<PluginFinding>),
}

/// The folder of the plugin of `key`, from the repository's root.
pub(super) fn folder(key: &ProjectSlug) -> String {
    format!("{PLUGINS}/{key}")
}

/// What the plugin's description says: whose knowledge it holds.
pub(super) fn plugin_description(name: &CompanyName) -> String {
    format!("The knowledge of {name}, as the company keeps it in its own repository.")
}

/// What the skill's description says: whose knowledge it holds and when an
/// agent uses it.
fn skill_description(name: &CompanyName) -> String {
    format!(
        "The knowledge of {name}, as the company keeps it. Use it whenever the work is for {name} or about {name}."
    )
}

/// Renders the plugin of `key`, named `name`, from `tree`.
pub(super) fn render(
    tree: &Committed,
    key: &ProjectSlug,
    name: &CompanyName,
) -> Result<Rendered, PublicationError> {
    let root = classify_root(tree.file(CONFIG_FILE_NAME));
    let index_absent = tree.entry_index(_company::COMPANY_INDEX) == EntryIndex::Absent;
    match &root {
        RootDocument::Unreadable(reason) => {
            return Ok(Rendered::Findings(vec![PluginFinding::UnreadableRoot {
                reason: reason.clone(),
            }]));
        }
        RootDocument::ProjectLayer(_) => {
            return Ok(Rendered::Findings(vec![PluginFinding::ProjectRepository]));
        }
        root if !company_home(root, index_absent) => {
            return Ok(Rendered::Findings(vec![PluginFinding::NoIndex]));
        }
        _ => {}
    }
    let folder = folder(key);
    let selection = match _company::read(tree, Path::new(&folder)) {
        None => return Ok(Rendered::Findings(vec![PluginFinding::NoIndex])),
        Some(Err(findings)) => {
            return Ok(Rendered::Findings(
                findings.into_iter().map(PluginFinding::Reading).collect(),
            ));
        }
        Some(Ok(selection)) => selection,
    };
    let carried = Carried::of(&selection);
    let findings = carried.findings(tree, &selection);
    if !findings.is_empty() {
        return Ok(Rendered::Findings(findings));
    }
    let skill = format!("{folder}/skills/{key}");
    let mut files = BTreeMap::new();
    for (path, selected) in &selection.documents {
        let Some(place) = carried.file(path) else {
            continue;
        };
        let text = company_paths(
            &selected.text,
            &selected.document,
            CompanyText::Whole,
            |at| carried.replacement(tree, path, place, at),
        );
        files.insert(format!("{skill}/{place}"), finish(&text));
    }
    files.insert(
        format!("{skill}/{SKILL_FILE}"),
        main_file(tree, key, name, &selection, &carried)?,
    );
    Ok(Rendered::Files(files))
}

/// The skill's main file: its frontmatter, the company's name as its
/// title, the company passage, each held document and one line for each
/// listed one.
fn main_file(
    tree: &Committed,
    key: &ProjectSlug,
    name: &CompanyName,
    selection: &Selection,
    carried: &Carried,
) -> Result<String, PublicationError> {
    let mut frontmatter = serde_yaml_ng::Mapping::new();
    frontmatter.insert("name".into(), key.as_str().into());
    frontmatter.insert("description".into(), skill_description(name).into());
    let frontmatter =
        serde_yaml_ng::to_string(&frontmatter).map_err(|err| PublicationError::Rendering {
            file: format!("{}/skills/{key}/{SKILL_FILE}", folder(key)),
            reason: err.to_string(),
        })?;
    let mut blocks = vec![
        format!("---\n{frontmatter}---"),
        format!("# {name}"),
        COMPANY_PASSAGE.to_owned(),
    ];
    for path in &selection.held {
        let Some(selected) = selection.documents.get(path) else {
            continue;
        };
        let body = company_paths(
            &selected.text,
            &selected.document,
            CompanyText::Body,
            |at| carried.replacement(tree, path, SKILL_FILE, at),
        );
        blocks.push(body.trim().to_owned());
    }
    let lines: Vec<String> = selection
        .listed
        .iter()
        .filter_map(|(document, description)| {
            Some(listed_line(carried.file(document)?, description))
        })
        .collect();
    if !lines.is_empty() {
        blocks.push(lines.join("\n"));
    }
    Ok(finish(&blocks.join("\n\n")))
}

/// Where the plugin carries each document the selection reaches: by its
/// path inside the skill's folder, `SKILL.md` for a document held in full.
struct Carried {
    places: BTreeMap<String, String>,
}

impl Carried {
    fn of(selection: &Selection) -> Self {
        let held: BTreeSet<&String> = selection.held.iter().collect();
        let places = selection
            .documents
            .keys()
            .map(|path| {
                let place = if held.contains(path) {
                    SKILL_FILE.to_owned()
                } else {
                    let relative = path.strip_prefix(DOCS).unwrap_or(path);
                    format!("{REFERENCES}/{relative}")
                };
                (path.clone(), place)
            })
            .collect();
        Self { places }
    }

    /// The path inside the skill's folder of the file of its own that
    /// carries the document at `path`; `None` when the main file holds it,
    /// or the plugin does not carry it.
    fn file(&self, path: &str) -> Option<&str> {
        self.places
            .get(path)
            .map(String::as_str)
            .filter(|place| *place != SKILL_FILE)
    }

    /// What replaces the path `at` written in the document at `reader`,
    /// carried at `from` inside the skill's folder: where the plugin carries
    /// what it names, relative to `from`'s folder; `None` for a name, or a
    /// path that names nothing the plugin carries.
    fn replacement(
        &self,
        tree: &Committed,
        reader: &str,
        from: &str,
        at: CompanyPath<'_>,
    ) -> Option<String> {
        let written = match at {
            CompanyPath::Reference(CompanyReference::Path(file)) => file.as_str(),
            CompanyPath::Reference(CompanyReference::Name(_)) => return None,
            CompanyPath::Read(read) => read,
        };
        let target = _company::resolve(tree, reader, written).ok()?;
        Some(relative(from, self.places.get(&target)?))
    }

    /// The plugin's own findings on the documents it carries, each
    /// document's in path order, then each `public` document's dependency
    /// on an `internal` one.
    fn findings(&self, tree: &Committed, selection: &Selection) -> Vec<PluginFinding> {
        let mut findings = Vec::new();
        let mut visibility = BTreeMap::new();
        for (path, selected) in &selection.documents {
            match &selected.document.frontmatter {
                CompanyFrontmatter::Invalid(reason) => {
                    findings.push(PluginFinding::FrontmatterInvalid {
                        document: path.clone(),
                        reason: reason.clone(),
                    });
                }
                CompanyFrontmatter::Read {
                    visibility: declared,
                    ..
                } => match Visibility::of(declared.as_ref()) {
                    Ok(declared) => {
                        visibility.insert(path.as_str(), declared);
                    }
                    Err(value) => findings.push(PluginFinding::UnknownVisibility {
                        document: path.clone(),
                        value,
                    }),
                },
            }
            for reference in &selected.document.references {
                let CompanyReference::Path(written) = reference else {
                    continue;
                };
                let Ok(target) = _company::resolve(tree, path, written) else {
                    continue;
                };
                if self.places.contains_key(&target) {
                    continue;
                }
                let finding = if target.ends_with(".md") {
                    PluginFinding::Uncarried {
                        document: path.clone(),
                        reference: written.clone(),
                    }
                } else {
                    PluginFinding::NotADocument {
                        document: path.clone(),
                        reference: written.clone(),
                    }
                };
                findings.push(finding);
            }
        }
        for (path, selected) in &selection.documents {
            if visibility.get(path.as_str()) != Some(&Visibility::Public) {
                continue;
            }
            let references =
                selected
                    .document
                    .references
                    .iter()
                    .filter_map(|reference| match reference {
                        CompanyReference::Path(written) => Some(written.as_str()),
                        CompanyReference::Name(_) => None,
                    });
            let mut named = BTreeSet::new();
            for written in selected
                .document
                .reads
                .iter()
                .map(String::as_str)
                .chain(references)
            {
                let Ok(target) = _company::resolve(tree, path, written) else {
                    continue;
                };
                if visibility.get(target.as_str()) == Some(&Visibility::Internal)
                    && named.insert(target.clone())
                {
                    findings.push(PluginFinding::PublicOnInternal {
                        document: path.clone(),
                        internal: target,
                    });
                }
            }
        }
        findings
    }
}

/// The path of the file `to` relative to the folder of the file `from`,
/// both relative to one folder.
fn relative(from: &str, to: &str) -> String {
    let from: Vec<&str> = from.split('/').collect();
    let from = &from[..from.len().saturating_sub(1)];
    let to: Vec<&str> = to.split('/').collect();
    let common = from
        .iter()
        .zip(&to)
        .take_while(|(a, b)| a == b)
        .count()
        .min(to.len().saturating_sub(1));
    let mut parts: Vec<&str> = vec![".."; from.len() - common];
    parts.extend(&to[common..]);
    parts.join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    const INDEX: &str = super::_company::COMPANY_INDEX;

    fn key() -> ProjectSlug {
        ProjectSlug::parse("acme").unwrap()
    }

    fn name() -> CompanyName {
        CompanyName::parse("Acme").unwrap()
    }

    fn findings(tree: &Committed) -> Vec<PluginFinding> {
        match render(tree, &key(), &name()).unwrap() {
            Rendered::Findings(findings) => findings,
            Rendered::Files(files) => panic!("rendered {:?}", files.keys()),
        }
    }

    #[test]
    fn a_path_names_its_file_from_the_folder_of_the_file_that_holds_it() {
        assert_eq!(
            relative("SKILL.md", "references/teams/a.md"),
            "references/teams/a.md"
        );
        assert_eq!(relative("SKILL.md", "SKILL.md"), "SKILL.md");
        assert_eq!(
            relative("references/teams/a.md", "references/roles.md"),
            "../roles.md"
        );
        assert_eq!(
            relative("references/teams/a.md", "SKILL.md"),
            "../../SKILL.md"
        );
        assert_eq!(
            relative("references/teams/a.md", "references/teams/b.md"),
            "b.md"
        );
    }

    #[test]
    fn a_company_name_is_one_line_that_is_not_blank() {
        assert_eq!(name().as_str(), "Acme");
        assert_eq!(CompanyName::parse("  "), Err(CompanyNameError::Blank));
        assert_eq!(
            CompanyName::parse("Acme\nInc"),
            Err(CompanyNameError::MoreThanOneLine)
        );
    }

    #[test]
    fn a_projects_repository_renders_no_plugin() {
        let tree = Committed::of(&[
            (CONFIG_FILE_NAME, "schema_version: 6\nproject: demo\n"),
            (INDEX, "expanded: [company.md]\n"),
            ("docs/company.md", "# Company\n"),
        ]);
        assert_eq!(findings(&tree), [PluginFinding::ProjectRepository]);
    }

    #[test]
    fn a_repository_without_a_company_index_renders_no_plugin() {
        let tree = Committed::of(&[("docs/company.md", "# Company\n")]);
        assert_eq!(findings(&tree), [PluginFinding::NoIndex]);
    }

    #[test]
    fn frontmatter_that_is_no_yaml_mapping_on_a_carried_document_is_a_finding() {
        let tree = Committed::of(&[
            (INDEX, "expanded: [company.md]\n"),
            ("docs/company.md", "---\n- a list\n---\n\n# Company\n"),
        ]);
        assert!(matches!(
            findings(&tree).as_slice(),
            [PluginFinding::FrontmatterInvalid { document, .. }] if document == "docs/company.md"
        ));
    }

    #[test]
    fn a_visibility_other_than_internal_or_public_is_a_finding_naming_it() {
        let tree = Committed::of(&[
            (INDEX, "expanded: [company.md]\n"),
            (
                "docs/company.md",
                "---\nvisibility: secret\n---\n\n# Company\n",
            ),
        ]);
        assert_eq!(
            findings(&tree),
            [PluginFinding::UnknownVisibility {
                document: "docs/company.md".to_owned(),
                value: "secret".to_owned(),
            }]
        );
    }

    #[test]
    fn a_reference_to_a_document_the_plugin_does_not_carry_is_a_finding() {
        let tree = Committed::of(&[
            (INDEX, "expanded: [company.md]\n"),
            ("docs/company.md", "# Company\n\nSee [drafts](drafts.md).\n"),
            ("docs/drafts.md", "# Drafts\n"),
        ]);
        assert_eq!(
            findings(&tree),
            [PluginFinding::Uncarried {
                document: "docs/company.md".to_owned(),
                reference: "drafts.md".to_owned(),
            }]
        );
    }

    #[test]
    fn a_reference_to_a_file_that_is_no_document_is_a_finding() {
        let tree = Committed::of(&[
            (INDEX, "expanded: [company.md]\n"),
            ("docs/company.md", "# Company\n\nSee ![the map](map.png).\n"),
            ("docs/map.png", "PNG"),
        ]);
        assert_eq!(
            findings(&tree),
            [PluginFinding::NotADocument {
                document: "docs/company.md".to_owned(),
                reference: "map.png".to_owned(),
            }]
        );
    }
}
