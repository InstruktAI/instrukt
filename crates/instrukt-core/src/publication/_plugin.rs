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
//!
//! A plan of a rendering ([`plan`]) also keeps or raises the manifest's
//! version against what `plugins/<key>/` holds, sets the plugin's entry in
//! the repository's plugin catalog, and finds every symbolic link on the
//! way to either. It is a function of the view alone; applying it is
//! [`super::write_plugin`].

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde_json::Value as Json;
use serde_yaml_ng::Value;

use super::_company::{self, COMPANY_PASSAGE, DOCS, Selection, listed_line};
use super::_render::{PlannedFile, RenderFinding, RenderFindingKind, company_home, finish};
use super::_source::{Committed, EntryIndex};
use super::_units::{LEGEND, Unit, frame};
use super::PublicationError;
use crate::config::{CONFIG_FILE_NAME, RootDocument, classify_root};
use crate::docs::{
    self, CompanyFrontmatter, CompanyPath, CompanyReference, CompanyText, company_paths,
};
use crate::ids::ProjectSlug;

/// The folder below a repository's root that holds its plugins.
const PLUGINS: &str = "plugins";

/// The repository's plugin catalog, from its root.
pub(super) const CATALOG: &str = ".claude-plugin/marketplace.json";

/// The plugin's manifest, inside the plugin's folder.
const MANIFEST: &str = ".claude-plugin/plugin.json";

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
    /// A symbolic link at or above the plugin's folder, inside it, or at or
    /// above the catalog.
    Link { path: String },
    /// The manifest the repository holds cannot be read as a JSON object:
    /// why.
    ManifestUnreadable { path: String, reason: String },
    /// The manifest's `version` is not three numbers: as written, `None`
    /// when it holds none.
    ManifestVersion {
        path: String,
        version: Option<String>,
    },
    /// The repository holds no plugin catalog.
    NoCatalog,
    /// The catalog cannot be read as a JSON object holding a list of
    /// plugins: why.
    CatalogUnreadable { reason: String },
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
            Self::Link { path } => write!(
                f,
                "{path}: a symbolic link, and nothing of the plugin or its catalog is read, written or removed through one; replace it with what it points to"
            ),
            Self::ManifestUnreadable { path, reason } => write!(
                f,
                "{path}: cannot be read as the plugin's manifest ({reason}); correct it to a JSON object or remove it, and the version is kept or raised from it once it can be read"
            ),
            Self::ManifestVersion {
                path,
                version: Some(version),
            } => write!(
                f,
                "{path}: its `version` is `{version}`, which is not three numbers; set it to three numbers joined by dots, such as `1.0.0`"
            ),
            Self::ManifestVersion {
                path,
                version: None,
            } => write!(
                f,
                "{path}: holds no `version` text; set one of three numbers joined by dots, such as `1.0.0`"
            ),
            Self::NoCatalog => write!(
                f,
                "{CATALOG}: missing, and instrukt creates no catalog because its name and owner are the repository's; create it with its `name`, `owner` and a `plugins` list"
            ),
            Self::CatalogUnreadable { reason } => write!(
                f,
                "{CATALOG}: cannot be read as a plugin catalog ({reason}); correct it to a JSON object holding a `plugins` list, and the plugin's entry is set once it can be read"
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
    let Some(reading) = _company::read(tree, Path::new(&folder)) else {
        return Ok(Rendered::Findings(vec![PluginFinding::NoIndex]));
    };
    let selection = reading.selection;
    let units = match _company::held_units(tree, &selection) {
        Ok(units) => units,
        Err(found) => {
            return Ok(Rendered::Findings(
                found
                    .into_iter()
                    .map(|finding| {
                        PluginFinding::Reading(RenderFinding {
                            path: PathBuf::from(&folder),
                            kind: RenderFindingKind::Unit { finding },
                        })
                    })
                    .collect(),
            ));
        }
    };
    let carried = Carried::of(&selection, &units);
    // The documents the shared reading could read are judged by the
    // plugin's own rules too, so one call reports every finding.
    let mut findings: Vec<PluginFinding> = reading
        .findings
        .into_iter()
        .map(PluginFinding::Reading)
        .collect();
    findings.extend(carried.findings(tree, &selection));
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
        main_file(tree, key, name, &selection, &units, &carried)?,
    );
    Ok(Rendered::Files(files))
}

/// The skill's main file: its frontmatter, the company's name as its
/// title, the company passage, the units' legend when it holds a unit, each
/// held document — as its unit, framed under its path with the line naming
/// the file that carries its prose, or in its prose — and one line for each
/// listed one.
fn main_file(
    tree: &Committed,
    key: &ProjectSlug,
    name: &CompanyName,
    selection: &Selection,
    units: &BTreeMap<String, Unit>,
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
    if !units.is_empty() {
        blocks.push(LEGEND.to_owned());
    }
    for path in &selection.held {
        if let (Some(unit), Some(place)) = (units.get(path), carried.file(path)) {
            let document = docs::company_document(&unit.body);
            let body = company_paths(&unit.body, &document, CompanyText::Body, |at| {
                carried.replacement(tree, path, SKILL_FILE, at)
            });
            let framed = frame(
                path,
                &format!(
                    "Full prose: `{}`\n\n{}",
                    relative(SKILL_FILE, place),
                    body.trim()
                ),
            );
            blocks.push(framed.trim().to_owned());
            continue;
        }
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
/// path inside the skill's folder, `SKILL.md` for a document held in full
/// in its prose. A document the main file holds as its unit is carried in
/// prose as a file of its own under `references/`, as every other one is.
struct Carried {
    places: BTreeMap<String, String>,
}

impl Carried {
    fn of(selection: &Selection, units: &BTreeMap<String, Unit>) -> Self {
        let held: BTreeSet<&String> = selection
            .held
            .iter()
            .filter(|path| !units.contains_key(*path))
            .collect();
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
                CompanyFrontmatter::Null => {
                    findings.push(PluginFinding::FrontmatterInvalid {
                        document: path.clone(),
                        reason: "it is empty or `null`, not a YAML mapping".to_owned(),
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

/// A plugin's version: three numbers, of which instrukt raises only the
/// last. Each number is held as the ASCII digits it is written with, so the
/// first two stay exactly what the manifest holds and the last has no bound.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginVersion {
    /// The first two numbers with the dot between them, as written.
    series: String,
    /// The last number's digits, as written.
    patch: String,
}

impl PluginVersion {
    /// `text` as three numbers joined by dots, each of ASCII digits only;
    /// `None` for anything else.
    fn parse(text: &str) -> Option<Self> {
        let number = |part: &str| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());
        let (series, patch) = text.rsplit_once('.')?;
        let (major, minor) = series.split_once('.')?;
        [major, minor, patch].into_iter().all(number).then(|| Self {
            series: series.to_owned(),
            patch: patch.to_owned(),
        })
    }

    /// The version a plugin starts from where the repository holds no
    /// manifest for it: 0.1.0.
    fn first() -> Self {
        Self {
            series: "0.1".to_owned(),
            patch: "0".to_owned(),
        }
    }

    /// This version with its last number raised by one, the first two as
    /// they are.
    fn raised(&self) -> Self {
        let mut digits = self.patch.clone().into_bytes();
        let mut carry = true;
        for digit in digits.iter_mut().rev() {
            if *digit == b'9' {
                *digit = b'0';
            } else {
                *digit += 1;
                carry = false;
                break;
            }
        }
        if carry {
            digits.insert(0, b'1');
        }
        Self {
            series: self.series.clone(),
            patch: digits.into_iter().map(char::from).collect(),
        }
    }
}

impl std::fmt::Display for PluginVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}", self.series, self.patch)
    }
}

/// What rendering a company's plugin from one view of its repository finds
/// and would change. It writes nothing itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginPlan {
    /// The plugin's folder, relative to the repository's root.
    pub folder: PathBuf,
    /// What leaves every file as it is.
    pub findings: Vec<PluginFinding>,
    /// What the rendering changes against what the view holds; `None` when
    /// a finding leaves the rendering, the held version or the plugin's
    /// folder unknown. It is applied only when the plan holds no finding.
    pub drift: Option<PluginDrift>,
    /// The catalog with the plugin's entry set, when that differs from what
    /// the view holds: known from the key, the name and the catalog alone,
    /// whatever the plugin's files. `None` when the entry already holds its
    /// values, or a finding or a link leaves the catalog unread. It is
    /// written last, with the drift, only when the plan holds no finding.
    pub catalog: Option<PlannedFile>,
}

/// What a rendering changes against what the view holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginDrift {
    /// The plugin's version once the rendering is applied.
    pub version: PluginVersion,
    /// Each file of the plugin whose content differs from what the view
    /// holds, or that it does not hold, in the order they are written: the
    /// plugin's files other than its manifest, then its manifest.
    pub writes: Vec<PlannedFile>,
    /// Each file the view holds under the plugin's folder that the plugin
    /// does not hold, relative to the repository's root.
    pub removals: Vec<PathBuf>,
}

/// The version the plugin's manifest holds, as far as it can be known.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Held {
    /// The repository holds no manifest for the plugin.
    Absent,
    Version(PluginVersion),
    /// A finding, or a link on the way to the manifest, leaves it unknown.
    Unknown,
}

/// What a view says of the plugin before anything is compared.
struct Assessment<'t> {
    findings: Vec<PluginFinding>,
    /// The rendering, unless a finding of the documents prevents it.
    files: Option<BTreeMap<String, String>>,
    held: Held,
    /// The catalog as read, with its bytes, when it can be read.
    catalog: Option<(Json, &'t [u8])>,
    /// Whether a link stands at or above the plugin's folder or inside it.
    folder_linked: bool,
}

/// Reads what `tree` says of the plugin of `key`, named `name`, and every
/// finding, comparing nothing.
fn assess<'t>(
    tree: &'t Committed,
    key: &ProjectSlug,
    name: &CompanyName,
) -> Result<Assessment<'t>, PublicationError> {
    let folder = folder(key);
    let manifest_path = format!("{folder}/{MANIFEST}");
    let mut folder_linked = false;
    let mut findings = Vec::new();
    for link in tree.links() {
        let at_folder = on_the_way(link, &folder) || within(link, &folder);
        if at_folder || on_the_way(link, CATALOG) {
            folder_linked |= at_folder;
            findings.push(PluginFinding::Link {
                path: link.to_owned(),
            });
        }
    }
    let rendered = render(tree, key, name)?;
    // Nothing behind a link is read: the manifest and the catalog are each
    // judged unless a link stands at or above that file.
    let held = if tree.is_link(&manifest_path) {
        Held::Unknown
    } else {
        match held_version(tree, &manifest_path) {
            Ok(None) => Held::Absent,
            Ok(Some(version)) => Held::Version(version),
            Err(finding) => {
                findings.push(finding);
                Held::Unknown
            }
        }
    };
    let catalog = if tree.is_link(CATALOG) {
        None
    } else {
        match read_catalog(tree) {
            Ok(catalog) => Some(catalog),
            Err(finding) => {
                findings.push(finding);
                None
            }
        }
    };
    let files = match rendered {
        Rendered::Files(files) => Some(files),
        Rendered::Findings(found) => {
            findings.extend(found);
            None
        }
    };
    Ok(Assessment {
        findings,
        files,
        held,
        catalog,
        folder_linked,
    })
}

/// Every finding of the plugin of `key`, named `name`, in `tree`, a view
/// holding the plugin's folder and the catalog, without comparing any file.
pub(super) fn findings(
    tree: &Committed,
    key: &ProjectSlug,
    name: &CompanyName,
) -> Result<Vec<PluginFinding>, PublicationError> {
    Ok(assess(tree, key, name)?.findings)
}

/// Plans the rendering of the plugin of `key`, named `name`, from `tree`,
/// a view holding the plugin's folder and the catalog
/// ([`Committed::working_tree_places`]): every finding, and what the
/// rendering changes wherever that can be known, findings or not.
pub(super) fn plan(
    tree: &Committed,
    key: &ProjectSlug,
    name: &CompanyName,
) -> Result<PluginPlan, PublicationError> {
    let assessment = assess(tree, key, name)?;
    let catalog = match assessment.catalog {
        Some((document, bytes)) => catalog_entry(document, bytes, key, name)?,
        None => None,
    };
    let drift = match (assessment.files, assessment.held, assessment.folder_linked) {
        (Some(files), Held::Absent, false) => Some(drift(tree, key, name, files, None)?),
        (Some(files), Held::Version(version), false) => {
            Some(drift(tree, key, name, files, Some(version))?)
        }
        _ => None,
    };
    Ok(PluginPlan {
        folder: PathBuf::from(folder(key)),
        findings: assessment.findings,
        drift,
        catalog,
    })
}

/// What the rendering `files`, every file but the manifest, changes against
/// what `tree` holds: the version kept from `held` when the rendering with
/// it equals the folder file for file, and raised otherwise; the files to
/// write, in their order; and the files to remove.
fn drift(
    tree: &Committed,
    key: &ProjectSlug,
    name: &CompanyName,
    mut files: BTreeMap<String, String>,
    held: Option<PluginVersion>,
) -> Result<PluginDrift, PublicationError> {
    let folder = folder(key);
    let manifest_path = format!("{folder}/{MANIFEST}");
    let prefix = format!("{folder}/");
    let holds: BTreeMap<String, &[u8]> = tree
        .below(&prefix)
        .map(|(rel, bytes)| (format!("{prefix}{rel}"), bytes))
        .collect();
    let raise = held.is_some();
    let mut version = held.unwrap_or_else(PluginVersion::first);
    files.insert(manifest_path.clone(), manifest(key, name, &version)?);
    if raise && !same(&files, &holds) {
        version = version.raised();
        files.insert(manifest_path.clone(), manifest(key, name, &version)?);
    }
    let removals = holds
        .keys()
        .filter(|path| !files.contains_key(*path))
        .map(PathBuf::from)
        .collect();
    let planned = |path: String, text: String| PlannedFile {
        current: tree.file(&path).map(<[u8]>::to_vec),
        path: PathBuf::from(path),
        text,
    };
    let manifest_text = files.remove(&manifest_path);
    let mut writes: Vec<PlannedFile> = files
        .into_iter()
        .map(|(path, text)| planned(path, text))
        .collect();
    writes.extend(manifest_text.map(|text| planned(manifest_path, text)));
    writes.retain(PlannedFile::differs);
    Ok(PluginDrift {
        version,
        writes,
        removals,
    })
}

/// Whether `link` is `place` or a folder above it.
fn on_the_way(link: &str, place: &str) -> bool {
    place == link
        || place
            .strip_prefix(link)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// Whether `link` lies inside the folder `folder`.
fn within(link: &str, folder: &str) -> bool {
    link.strip_prefix(folder)
        .is_some_and(|rest| rest.starts_with('/'))
}

/// Whether the rendering `files` is what `holds` holds, file for file.
fn same(files: &BTreeMap<String, String>, holds: &BTreeMap<String, &[u8]>) -> bool {
    files.len() == holds.len()
        && files
            .iter()
            .all(|(path, text)| holds.get(path) == Some(&text.as_bytes()))
}

/// The version of the manifest the view holds at `path`: `None` when it
/// holds none, or the finding that it cannot be read or holds no version.
fn held_version(tree: &Committed, path: &str) -> Result<Option<PluginVersion>, PluginFinding> {
    let Some(bytes) = tree.file(path) else {
        return Ok(None);
    };
    let unreadable = |reason: String| PluginFinding::ManifestUnreadable {
        path: path.to_owned(),
        reason,
    };
    let manifest = match serde_json::from_slice::<Json>(bytes) {
        Ok(Json::Object(manifest)) => manifest,
        Ok(_) => return Err(unreadable("it is not a JSON object".to_owned())),
        Err(err) => return Err(unreadable(format!("it is not JSON: {err}"))),
    };
    let version = |version| PluginFinding::ManifestVersion {
        path: path.to_owned(),
        version,
    };
    match manifest.get("version") {
        Some(Json::String(text)) => PluginVersion::parse(text)
            .map(Some)
            .ok_or_else(|| version(Some(text.clone()))),
        Some(other) => Err(version(Some(other.to_string()))),
        None => Err(version(None)),
    }
}

/// The plugin's manifest: its name, its version, its description and the
/// company as its author, as one JSON document.
fn manifest(
    key: &ProjectSlug,
    name: &CompanyName,
    version: &PluginVersion,
) -> Result<String, PublicationError> {
    let manifest = serde_json::json!({
        "name": key.as_str(),
        "version": version.to_string(),
        "description": plugin_description(name),
        "author": { "name": name.as_str() },
    });
    json_document(&manifest, &format!("{}/{MANIFEST}", folder(key)))
}

/// `value` as one JSON document: indented by two spaces, ending in one
/// newline.
fn json_document(value: &Json, file: &str) -> Result<String, PublicationError> {
    serde_json::to_string_pretty(value)
        .map(|text| format!("{text}\n"))
        .map_err(|err| PublicationError::Rendering {
            file: file.to_owned(),
            reason: err.to_string(),
        })
}

/// The catalog the view holds, as read, with its bytes, or the finding
/// that it holds none or one that is no JSON object holding a list of
/// plugins.
fn read_catalog(tree: &Committed) -> Result<(Json, &[u8]), PluginFinding> {
    let Some(bytes) = tree.file(CATALOG) else {
        return Err(PluginFinding::NoCatalog);
    };
    let unreadable = |reason: &str| PluginFinding::CatalogUnreadable {
        reason: reason.to_owned(),
    };
    let document = match serde_json::from_slice::<Json>(bytes) {
        Ok(document @ Json::Object(_)) => document,
        Ok(_) => return Err(unreadable("it is not a JSON object")),
        Err(err) => return Err(unreadable(&format!("it is not JSON: {err}"))),
    };
    if !matches!(document.get("plugins"), Some(Json::Array(_))) {
        return Err(unreadable("it holds no `plugins` list"));
    }
    Ok((document, bytes))
}

/// The catalog `document`, read from `bytes`, with the plugin's entry set,
/// as the file to write; `None` when the entry already holds its values.
fn catalog_entry(
    mut document: Json,
    bytes: &[u8],
    key: &ProjectSlug,
    name: &CompanyName,
) -> Result<Option<PlannedFile>, PublicationError> {
    let Some(Json::Array(plugins)) = document.get_mut("plugins") else {
        return Ok(None);
    };
    let wanted = [
        ("name", Json::from(key.as_str())),
        ("source", Json::from(format!("./{}", folder(key)))),
        ("description", Json::from(plugin_description(name))),
    ];
    let listed = plugins
        .iter_mut()
        .filter_map(Json::as_object_mut)
        .find(|entry| entry.get("name").and_then(Json::as_str) == Some(key.as_str()));
    match listed {
        Some(entry)
            if wanted
                .iter()
                .all(|(member, value)| entry.get(*member) == Some(value)) =>
        {
            return Ok(None);
        }
        Some(entry) => {
            for (member, value) in wanted {
                entry.insert(member.to_owned(), value);
            }
        }
        None => plugins.push(Json::Object(
            wanted
                .into_iter()
                .map(|(member, value)| (member.to_owned(), value))
                .collect(),
        )),
    }
    Ok(Some(PlannedFile {
        path: PathBuf::from(CATALOG),
        text: json_document(&document, CATALOG)?,
        current: Some(bytes.to_vec()),
    }))
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
    fn an_empty_or_null_frontmatter_block_on_a_carried_document_is_a_finding() {
        for block in ["---\n---\n", "---\nnull\n---\n"] {
            let tree = Committed::of(&[
                (INDEX, "expanded: [company.md]\n"),
                ("docs/company.md", &format!("{block}\n# Company\n")),
            ]);
            assert!(
                matches!(
                    findings(&tree).as_slice(),
                    [PluginFinding::FrontmatterInvalid { document, .. }] if document == "docs/company.md"
                ),
                "{block:?}"
            );
        }
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

    const CATALOG_TEXT: &str = "{\"name\": \"acme-tools\", \"owner\": {\"name\": \"Acme\"}, \"plugins\": [{\"name\": \"other\", \"source\": \"./other\", \"extra\": 1}]}";

    /// A company's repository holding `catalog`, one expanded document and
    /// `more`.
    fn repository(catalog: Option<&str>, more: &[(&str, &str)]) -> Committed {
        let mut files = vec![
            (INDEX, "expanded: [company.md]\n"),
            ("docs/company.md", "# Company\n\nWe make things.\n"),
        ];
        files.extend(catalog.map(|text| (CATALOG, text)));
        files.extend_from_slice(more);
        Committed::of(&files)
    }

    /// The catalog with the plugin's entry set, or the finding that it
    /// cannot be read.
    fn catalog(
        tree: &Committed,
        key: &ProjectSlug,
        name: &CompanyName,
    ) -> Result<Result<Option<PlannedFile>, PluginFinding>, PublicationError> {
        match read_catalog(tree) {
            Ok((document, bytes)) => catalog_entry(document, bytes, key, name).map(Ok),
            Err(finding) => Ok(Err(finding)),
        }
    }

    /// What rendering from `tree` changes, which must be knowable.
    fn drift_of(tree: &Committed) -> PluginDrift {
        plan(tree, &key(), &name()).unwrap().drift.unwrap()
    }

    /// `tree` with `plan`'s writes applied and its removals made.
    fn applied(tree: &[(String, String)], plan: &PluginDrift) -> Vec<(String, String)> {
        let mut files: BTreeMap<String, String> = tree.iter().cloned().collect();
        for path in &plan.removals {
            files.remove(path.to_str().unwrap());
        }
        for file in &plan.writes {
            files.insert(file.path.to_str().unwrap().to_owned(), file.text.clone());
        }
        files.into_iter().collect()
    }

    fn view(files: &[(String, String)]) -> Committed {
        let borrowed: Vec<(&str, &str)> = files
            .iter()
            .map(|(path, text)| (path.as_str(), text.as_str()))
            .collect();
        Committed::of(&borrowed)
    }

    fn files_of(tree: &[(&str, &str)]) -> Vec<(String, String)> {
        tree.iter()
            .map(|(path, text)| ((*path).to_owned(), (*text).to_owned()))
            .collect()
    }

    const COMPANY: &str = "# Company\n\nWe make things. Roles: [roles](roles.md).\n";
    const ROLES: &str = "---\ndescription: Who does what.\n---\n\n# Roles\n\nAda builds.\n";

    /// A unit of `document`, current when `current`, with `body`.
    fn unit_of(document: &str, current: bool, body: &str) -> String {
        let prose = docs::split_frontmatter(document).1;
        let digest = super::super::_units::source_digest(if current { prose } else { "older" });
        format!("---\nsource_sha256: {digest}\nmodel: m\n---\n{body}")
    }

    /// A company repository expanding `company.md`, listing `roles.md`,
    /// with `company.md`'s unit `unit`.
    fn with_unit(unit: Option<&str>) -> Vec<(String, String)> {
        let mut files = vec![
            (INDEX, "expanded: [company.md]\nlisted: [roles.md]\n"),
            ("docs/company.md", COMPANY),
            ("docs/roles.md", ROLES),
            (CATALOG, CATALOG_TEXT),
        ];
        files.extend(unit.map(|unit| ("docs/_compressed/company.md", unit)));
        files_of(&files)
    }

    fn rendered_files(tree: &Committed) -> BTreeMap<String, String> {
        match render(tree, &key(), &name()).unwrap() {
            Rendered::Files(files) => files,
            Rendered::Findings(findings) => panic!("findings {findings:?}"),
        }
    }

    const SKILL: &str = "plugins/acme/skills/acme/SKILL.md";
    const CARRIED: &str = "plugins/acme/skills/acme/references/company.md";

    #[test]
    fn a_document_held_as_its_unit_is_framed_with_the_file_carrying_its_prose() {
        let unit = unit_of(
            COMPANY,
            true,
            "- we make things; roles: [roles](roles.md)\n",
        );
        let files = rendered_files(&view(&with_unit(Some(&unit))));
        assert_eq!(
            files[CARRIED],
            "# Company\n\nWe make things. Roles: [roles](roles.md).\n"
        );
        insta::assert_snapshot!(files[SKILL]);
    }

    #[test]
    fn a_stale_unit_holds_its_document_in_prose_in_the_main_file_alone() {
        let unit = unit_of(COMPANY, false, "- we make things\n");
        let files = rendered_files(&view(&with_unit(Some(&unit))));
        assert!(!files.contains_key(CARRIED));
        insta::assert_snapshot!(files[SKILL]);
    }

    #[test]
    fn the_version_rises_when_only_a_unit_changes() {
        let first = with_unit(Some(&unit_of(COMPANY, true, "- we make things\n")));
        let plan1 = drift_of(&view(&first));
        let rendered = applied(&first, &plan1);
        let mut changed = rendered.clone();
        for (path, text) in &mut changed {
            if path == "docs/_compressed/company.md" {
                *text = unit_of(COMPANY, true, "- we make many things\n");
            }
        }
        assert_eq!(drift_of(&view(&changed)).version, plan1.version.raised());
    }

    #[test]
    fn a_version_is_three_numbers_of_ascii_digits() {
        let parsed = PluginVersion::parse("1.20.3").unwrap();
        assert_eq!(parsed.to_string(), "1.20.3");
        assert_eq!(parsed.raised().to_string(), "1.20.4");
        for text in [
            "1.2", "1.2.3.4", "+1.0.0", "1.-2.3", "1..3", "v1.2.3", "1.2.٣", "",
        ] {
            assert_eq!(PluginVersion::parse(text), None, "{text:?}");
        }
    }

    #[test]
    fn the_last_number_rises_by_one_without_bound_and_the_first_two_stay_as_written() {
        let raised = |text: &str| PluginVersion::parse(text).unwrap().raised().to_string();
        assert_eq!(
            raised("2.7.18446744073709551615"),
            "2.7.18446744073709551616"
        );
        assert_eq!(raised("2.7.99"), "2.7.100");
        assert_eq!(raised("01.007.009"), "01.007.010");
        assert_eq!(
            raised("2.7.99999999999999999999999"),
            "2.7.100000000000000000000000"
        );
    }

    #[test]
    fn the_version_is_kept_when_the_rendering_equals_what_the_folder_holds_and_raised_otherwise() {
        let first = files_of(&[
            (INDEX, "expanded: [company.md]\n"),
            ("docs/company.md", "# Company\n\nWe make things.\n"),
            (CATALOG, CATALOG_TEXT),
        ]);
        let plan1 = drift_of(&view(&first));
        let rendered = applied(&first, &plan1);
        let again = drift_of(&view(&rendered));
        assert!(again.writes.is_empty() && again.removals.is_empty());
        assert_eq!(again.version, plan1.version);
        let mut changed = rendered.clone();
        for (path, text) in &mut changed {
            if path == "docs/company.md" {
                *text = "# Company\n\nWe make more things.\n".to_owned();
            }
        }
        let raised = drift_of(&view(&changed));
        assert_eq!(raised.version, plan1.version.raised());
        let mut stray = rendered.clone();
        stray.push(("plugins/acme/notes.md".to_owned(), "old".to_owned()));
        let cleaned = drift_of(&view(&stray));
        assert_eq!(cleaned.removals, [PathBuf::from("plugins/acme/notes.md")]);
        assert_eq!(cleaned.version, plan1.version.raised());
    }

    #[test]
    fn the_manifest_is_written_after_the_plugins_other_files() {
        let tree = repository(Some(CATALOG_TEXT), &[]);
        let paths: Vec<PathBuf> = drift_of(&tree)
            .writes
            .into_iter()
            .map(|file| file.path)
            .collect();
        assert_eq!(
            paths,
            [
                PathBuf::from("plugins/acme/skills/acme/SKILL.md"),
                PathBuf::from("plugins/acme/.claude-plugin/plugin.json"),
            ]
        );
    }

    #[test]
    fn the_catalogs_entry_is_known_without_the_plugins_files() {
        let tree = Committed::of(&[(CATALOG, CATALOG_TEXT)]);
        let plan = plan(&tree, &key(), &name()).unwrap();
        assert_eq!(plan.findings, [PluginFinding::NoIndex]);
        assert_eq!(plan.drift, None);
        assert_eq!(
            plan.catalog.map(|file| file.path),
            Some(PathBuf::from(CATALOG))
        );
    }

    #[test]
    fn the_entry_is_set_by_value_and_every_other_member_keeps_its_value_and_place() {
        let listed = r#"{"name": "acme-tools", "plugins": [{"name": "acme", "version": "9", "source": "./old"}, {"name": "other"}], "owner": {"name": "Acme"}}"#;
        let tree = repository(Some(listed), &[]);
        let file = catalog(&tree, &key(), &name()).unwrap().unwrap().unwrap();
        let written: Json = serde_json::from_str(&file.text).unwrap();
        let wanted: Json = serde_json::json!({
            "name": "acme-tools",
            "plugins": [
                {"name": "acme", "version": "9", "source": "./plugins/acme", "description": plugin_description(&name())},
                {"name": "other"}
            ],
            "owner": {"name": "Acme"}
        });
        assert_eq!(written, wanted);
        let order: Vec<&String> = written["plugins"][0].as_object().unwrap().keys().collect();
        assert_eq!(order, ["name", "version", "source", "description"]);
        // Once set, the entry holds its values: the catalog is left as it is.
        let tree = repository(Some(&file.text), &[]);
        assert_eq!(catalog(&tree, &key(), &name()).unwrap(), Ok(None));
    }

    #[test]
    fn an_entry_the_catalog_does_not_list_is_added_after_the_others() {
        let tree = repository(Some(CATALOG_TEXT), &[]);
        let file = catalog(&tree, &key(), &name()).unwrap().unwrap().unwrap();
        let written: Json = serde_json::from_str(&file.text).unwrap();
        let names: Vec<&str> = written["plugins"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["other", "acme"]);
        assert_eq!(written["plugins"][0]["extra"], 1);
    }

    #[test]
    fn a_missing_or_unreadable_catalog_is_a_finding() {
        assert_eq!(
            catalog(&repository(None, &[]), &key(), &name()).unwrap(),
            Err(PluginFinding::NoCatalog)
        );
        for text in ["[]", "{\"plugins\": {}}", "{"] {
            assert!(
                matches!(
                    catalog(&repository(Some(text), &[]), &key(), &name()).unwrap(),
                    Err(PluginFinding::CatalogUnreadable { .. })
                ),
                "{text:?}"
            );
        }
    }

    #[test]
    fn a_manifest_that_cannot_be_read_or_holds_no_version_of_three_numbers_is_a_finding() {
        let path = "plugins/acme/.claude-plugin/plugin.json";
        let held = |text: &str| held_version(&repository(None, &[(path, text)]), path);
        assert!(matches!(
            held("[]"),
            Err(PluginFinding::ManifestUnreadable { .. })
        ));
        assert!(matches!(
            held("{"),
            Err(PluginFinding::ManifestUnreadable { .. })
        ));
        assert_eq!(
            held("{\"version\": \"1.0\"}"),
            Err(PluginFinding::ManifestVersion {
                path: path.to_owned(),
                version: Some("1.0".to_owned()),
            })
        );
        assert_eq!(
            held("{\"name\": \"acme\"}"),
            Err(PluginFinding::ManifestVersion {
                path: path.to_owned(),
                version: None,
            })
        );
        assert_eq!(
            held("{\"version\": \"2.3.4\"}")
                .unwrap()
                .map(|v| v.to_string())
                .as_deref(),
            Some("2.3.4")
        );
    }

    #[test]
    fn a_link_on_the_way_to_the_plugin_or_the_catalog_or_inside_the_plugin_is_a_finding() {
        for link in [
            "plugins",
            "plugins/acme",
            "plugins/acme/skills/acme/x.md",
            ".claude-plugin",
        ] {
            let tree = repository(Some(CATALOG_TEXT), &[]).with_link(link);
            let plan = plan(&tree, &key(), &name()).unwrap();
            // Behind a link at the plugin's folder nothing is compared.
            assert_eq!(plan.drift.is_none(), link != ".claude-plugin", "{link}");
            assert_eq!(
                plan.findings,
                [PluginFinding::Link {
                    path: link.to_owned()
                }],
                "{link}"
            );
        }
        let elsewhere = repository(Some(CATALOG_TEXT), &[]).with_link("plugins/other");
        assert!(
            plan(&elsewhere, &key(), &name())
                .unwrap()
                .findings
                .is_empty()
        );
    }

    #[test]
    fn a_check_still_knows_what_differs_beside_a_catalog_finding() {
        let plan = plan(&repository(None, &[]), &key(), &name()).unwrap();
        assert_eq!(plan.findings, [PluginFinding::NoCatalog]);
        let written: Vec<PathBuf> = plan
            .drift
            .unwrap()
            .writes
            .into_iter()
            .map(|file| file.path)
            .collect();
        assert_eq!(
            written,
            [
                PathBuf::from("plugins/acme/skills/acme/SKILL.md"),
                PathBuf::from("plugins/acme/.claude-plugin/plugin.json"),
            ]
        );
    }

    #[test]
    fn a_link_inside_the_plugin_leaves_the_catalog_judged() {
        let tree = repository(None, &[]).with_link("plugins/acme/skills/acme/x.md");
        assert_eq!(
            plan(&tree, &key(), &name()).unwrap().findings,
            [
                PluginFinding::Link {
                    path: "plugins/acme/skills/acme/x.md".to_owned()
                },
                PluginFinding::NoCatalog,
            ]
        );
    }

    #[test]
    fn the_shared_readings_findings_and_the_plugins_own_are_reported_together() {
        let tree = Committed::of(&[
            (INDEX, "expanded: [company.md, people.md]\n"),
            (
                "docs/company.md",
                "# Company\n\nSee [notes](../notes.md).\n",
            ),
            (
                "docs/people.md",
                "---\nvisibility: secret\n---\n\n# People\n",
            ),
        ]);
        let found = findings(&tree);
        assert!(
            matches!(
                found.as_slice(),
                [
                    PluginFinding::Reading(_),
                    PluginFinding::UnknownVisibility { document, .. },
                ] if document == "docs/people.md"
            ),
            "{found:?}"
        );
    }
}
