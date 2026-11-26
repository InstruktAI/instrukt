//! A company's Claude plugin
//! (`project/instrukt/spec/feature/publication/company-plugin`): the skill
//! folder `skills/<key>/` at a company's repository's root, whose main file
//! carries the company passage and each `expanded` document in full, and one
//! file for every other document the index selects or a required read
//! reaches, each reference between documents naming where the skill carries
//! the document it names; and the skill's entry in the repository's plugin
//! catalog, which is the plugin's manifest.
//!
//! The documents are read once, by the reading the company's `AGENTS.md` is
//! rendered from ([`super::_company::read`]); every finding of that reading
//! is a finding here. The plugin adds its own: a repository that holds no
//! company index or is a project, a reference to a document the plugin does
//! not carry or to a file that is no document, frontmatter that is no YAML
//! mapping, and a `visibility` that is unknown or a `public` document that
//! depends on an `internal` one.
//!
//! A plan of a rendering ([`plan`]) also keeps or raises the version the
//! catalog entry holds against what `skills/<key>/` holds, sets the entry in
//! the repository's plugin catalog, creating the catalog where there is
//! none, and finds every symbolic link on the way to either. It is a function of the view alone; applying it is
//! [`super::write_plugin`].

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde_json::Value as Json;
use serde_yaml_ng::Value;

use super::_company::{self, COMPANY_PASSAGE, DOCS, Selection, listed_line};
use super::_render::{PlannedFile, RenderFinding, company_home, finish};
use super::_source::{Committed, EntryIndex};
use super::PublicationError;
use crate::config::{CONFIG_FILE_NAME, RootDocument, classify_root};
use crate::docs::{CompanyFrontmatter, CompanyPath, CompanyReference, CompanyText, company_paths};
use crate::ids::ProjectSlug;

/// The folder below a repository's root that holds its skills.
const SKILLS: &str = "skills";

/// The repository's plugin catalog, from its root.
pub(super) const CATALOG: &str = ".claude-plugin/marketplace.json";

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
    /// A symbolic link at or above the skill's folder, inside it, or at or
    /// above the catalog.
    Link { path: String },
    /// The plugin's catalog entry holds a `version` that is not three
    /// numbers: as written, `None` when it holds none.
    EntryVersion { version: Option<String> },
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
                "{path}: a symbolic link, and nothing of the skill or its catalog is read, written or removed through one; replace it with what it points to"
            ),
            Self::EntryVersion {
                version: Some(version),
            } => write!(
                f,
                "{CATALOG}: the plugin's entry has the `version` `{version}`, which is not three numbers; set it to three numbers joined by dots, such as `1.0.0`"
            ),
            Self::EntryVersion { version: None } => write!(
                f,
                "{CATALOG}: the plugin's entry holds no `version` text; set one of three numbers joined by dots, such as `1.0.0`"
            ),
            Self::CatalogUnreadable { reason } => write!(
                f,
                "{CATALOG}: cannot be read as a plugin catalog ({reason}); correct it to a JSON object holding a `plugins` list, and the plugin's entry is set once it can be read"
            ),
        }
    }
}

/// The skill of `key`, named `name`, rendered from `tree`: every file of
/// it, by repository-relative path, or the findings that leave every file as
/// it is.
pub(super) enum Rendered {
    Files(BTreeMap<String, String>),
    Findings(Vec<PluginFinding>),
}

/// The manifest an earlier rendering of the plugin of `key` wrote, from the
/// repository's root: read for its version alone, never written or removed.
pub(super) fn earlier_manifest(key: &ProjectSlug) -> String {
    format!("plugins/{key}/.claude-plugin/plugin.json")
}

/// The folder of the skill of `key`, from the repository's root.
pub(super) fn folder(key: &ProjectSlug) -> String {
    format!("{SKILLS}/{key}")
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
        RootDocument::ProjectLayer {
            company_repository: false,
            ..
        } => {
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
    let carried = Carried::of(&selection);
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
    let skill = folder;
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
            file: format!("{}/{SKILL_FILE}", folder(key)),
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
/// first two stay exactly what the catalog entry holds and the last has no
/// bound.
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

    /// The version a plugin starts from where the repository's catalog
    /// holds no entry for it: 0.1.0.
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
    /// The skill's folder, relative to the repository's root.
    pub folder: PathBuf,
    /// What leaves every file as it is.
    pub findings: Vec<PluginFinding>,
    /// What the rendering changes against what the view holds; `None` when
    /// a finding leaves the rendering, the held version or the skill's
    /// folder unknown. It is applied only when the plan holds no finding.
    pub drift: Option<PluginDrift>,
    /// The catalog with the plugin's entry set, or created where the view
    /// holds none, when that differs from what the view holds. `None` when
    /// the entry already holds its values, or the drift, a finding or a
    /// link leaves the entry's version or the catalog unknown. It is written
    /// last, with the drift, only when the plan holds no finding.
    pub catalog: Option<PlannedFile>,
}

/// What a rendering changes against what the view holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginDrift {
    /// The plugin's version once the rendering is applied.
    pub version: PluginVersion,
    /// Each file of the skill whose content differs from what the view
    /// holds, or that it does not hold, in the order they are written.
    pub writes: Vec<PlannedFile>,
    /// Each file the view holds under the skill's folder that the skill
    /// does not hold, relative to the repository's root.
    pub removals: Vec<PathBuf>,
}

/// The version the plugin's catalog entry holds, as far as it can be known.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Held {
    /// The catalog holds no entry for the plugin, or there is no catalog.
    Absent,
    Version(PluginVersion),
    /// A finding, or a link on the way to the catalog, leaves it unknown.
    Unknown,
}

/// The repository's catalog, as far as it can be read.
enum Catalog<'t> {
    /// The repository holds none.
    Missing,
    /// The catalog as read, with its bytes.
    Read(Json, &'t [u8]),
    /// A finding or a link leaves it unread.
    Unknown,
}

/// What a view says of the plugin before anything is compared.
struct Assessment<'t> {
    findings: Vec<PluginFinding>,
    /// The rendering, unless a finding of the documents prevents it.
    files: Option<BTreeMap<String, String>>,
    held: Held,
    catalog: Catalog<'t>,
    /// Whether a link stands at or above the skill's folder or inside it.
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
    // Nothing behind a link is read: the catalog, and the version its entry
    // holds, are judged unless a link stands at or above it.
    let catalog = if tree.is_link(CATALOG) {
        Catalog::Unknown
    } else {
        match read_catalog(tree) {
            Ok(None) => Catalog::Missing,
            Ok(Some((document, bytes))) => Catalog::Read(document, bytes),
            Err(finding) => {
                findings.push(finding);
                Catalog::Unknown
            }
        }
    };
    let held = match &catalog {
        Catalog::Missing => Held::Absent,
        Catalog::Unknown => Held::Unknown,
        Catalog::Read(document, _) => match held_version(tree, document, key) {
            Ok(None) => Held::Absent,
            Ok(Some(version)) => Held::Version(version),
            Err(finding) => {
                findings.push(finding);
                Held::Unknown
            }
        },
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
/// holding the skill's folder and the catalog, without comparing any file.
pub(super) fn findings(
    tree: &Committed,
    key: &ProjectSlug,
    name: &CompanyName,
) -> Result<Vec<PluginFinding>, PublicationError> {
    Ok(assess(tree, key, name)?.findings)
}

/// Plans the rendering of the plugin of `key`, named `name`, from `tree`,
/// a view holding the skill's folder and the catalog
/// ([`Committed::working_tree_places`]): every finding, and what the
/// rendering changes wherever that can be known, findings or not. The
/// drift comes first, because the catalog entry carries the version it
/// keeps or raises.
pub(super) fn plan(
    tree: &Committed,
    key: &ProjectSlug,
    name: &CompanyName,
) -> Result<PluginPlan, PublicationError> {
    let assessment = assess(tree, key, name)?;
    let drift = match (assessment.files, assessment.held, assessment.folder_linked) {
        (Some(files), Held::Absent, false) => Some(drift(tree, key, files, None)),
        (Some(files), Held::Version(version), false) => {
            Some(drift(tree, key, files, Some(version)))
        }
        _ => None,
    };
    let catalog = match (&drift, assessment.catalog) {
        (Some(drift), Catalog::Read(document, bytes)) => {
            catalog_entry(document, Some(bytes), key, name, &drift.version)?
        }
        (Some(drift), Catalog::Missing) => {
            catalog_entry(new_catalog(key, name), None, key, name, &drift.version)?
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

/// What the rendering `files` changes against what `tree` holds: the
/// version kept from `held` when the rendering equals the skill's folder
/// file for file, and raised otherwise; the files to write, in their order;
/// and the files to remove.
fn drift(
    tree: &Committed,
    key: &ProjectSlug,
    files: BTreeMap<String, String>,
    held: Option<PluginVersion>,
) -> PluginDrift {
    let prefix = format!("{}/", folder(key));
    let holds: BTreeMap<String, &[u8]> = tree
        .below(&prefix)
        .map(|(rel, bytes)| (format!("{prefix}{rel}"), bytes))
        .collect();
    let version = match held {
        Some(held) if !same(&files, &holds) => held.raised(),
        Some(held) => held,
        None => PluginVersion::first(),
    };
    let removals = holds
        .keys()
        .filter(|path| !files.contains_key(*path))
        .map(PathBuf::from)
        .collect();
    let mut writes: Vec<PlannedFile> = files
        .into_iter()
        .map(|(path, text)| PlannedFile {
            current: tree.file(&path).map(<[u8]>::to_vec),
            path: PathBuf::from(path),
            text,
        })
        .collect();
    writes.retain(PlannedFile::differs);
    PluginDrift {
        version,
        writes,
        removals,
    }
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

/// The version the catalog `document`'s entry for `key` holds: `None` when
/// it lists no entry of that name, or the finding that the entry's version
/// is not three numbers. An entry an earlier rendering wrote holds no
/// version: the version of the manifest that rendering left in `tree`
/// ([`earlier_manifest`]) is the one held, and without one that is three
/// numbers the entry's missing version is the finding.
fn held_version(
    tree: &Committed,
    document: &Json,
    key: &ProjectSlug,
) -> Result<Option<PluginVersion>, PluginFinding> {
    let Some(entry) = document["plugins"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|entry| entry.get("name").and_then(Json::as_str) == Some(key.as_str()))
    else {
        return Ok(None);
    };
    match entry.get("version") {
        Some(Json::String(text)) => {
            PluginVersion::parse(text)
                .map(Some)
                .ok_or_else(|| PluginFinding::EntryVersion {
                    version: Some(text.clone()),
                })
        }
        Some(other) => Err(PluginFinding::EntryVersion {
            version: Some(other.to_string()),
        }),
        None => tree
            .file(&earlier_manifest(key))
            .and_then(|bytes| serde_json::from_slice::<Json>(bytes).ok())
            .and_then(|manifest| manifest.get("version")?.as_str().map(PluginVersion::parse))
            .flatten()
            .map(Some)
            .ok_or(PluginFinding::EntryVersion { version: None }),
    }
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

/// The catalog the view holds, as read, with its bytes; `None` when it
/// holds none, or the finding that it is no JSON object holding a list of
/// plugins.
fn read_catalog(tree: &Committed) -> Result<Option<(Json, &[u8])>, PluginFinding> {
    let Some(bytes) = tree.file(CATALOG) else {
        return Ok(None);
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
    Ok(Some((document, bytes)))
}

/// The catalog a repository without one receives: named by the company's
/// key, the company as its owner, listing no plugin yet.
fn new_catalog(key: &ProjectSlug, name: &CompanyName) -> Json {
    serde_json::json!({
        "name": key.as_str(),
        "owner": { "name": name.as_str() },
        "plugins": [],
    })
}

/// The plugin's entry, which is its manifest: its name, the repository's
/// root as its source, its one skill, its version, its description and the
/// company as its author.
fn wanted_entry(
    key: &ProjectSlug,
    name: &CompanyName,
    version: &PluginVersion,
) -> [(&'static str, Json); 7] {
    [
        ("name", Json::from(key.as_str())),
        // The catalog's own repository, its root, as Claude's catalog
        // format names it.
        ("source", Json::from("./")),
        ("strict", Json::from(false)),
        ("skills", serde_json::json!([format!("./{}", folder(key))])),
        ("version", Json::from(version.to_string())),
        ("description", Json::from(plugin_description(name))),
        ("author", serde_json::json!({ "name": name.as_str() })),
    ]
}

/// The catalog `document`, read from `bytes` (`None`: the repository holds
/// none), with the plugin's entry set at `version`, as the file to write;
/// `None` when the entry already holds its values.
fn catalog_entry(
    mut document: Json,
    bytes: Option<&[u8]>,
    key: &ProjectSlug,
    name: &CompanyName,
    version: &PluginVersion,
) -> Result<Option<PlannedFile>, PublicationError> {
    let Some(Json::Array(plugins)) = document.get_mut("plugins") else {
        return Ok(None);
    };
    let wanted = wanted_entry(key, name, version);
    let listed = plugins
        .iter_mut()
        .filter_map(Json::as_object_mut)
        .find(|entry| entry.get("name").and_then(Json::as_str) == Some(key.as_str()));
    match listed {
        Some(entry)
            if bytes.is_some()
                && wanted
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
        current: bytes.map(<[u8]>::to_vec),
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
    fn a_project_layer_declaring_company_repository_renders_the_plugin() {
        let tree = Committed::of(&[
            (
                CONFIG_FILE_NAME,
                "schema_version: 6\nproject: company-acme\ncompany_repository: true\n",
            ),
            (INDEX, "expanded: [company.md]\n"),
            (
                "docs/company.md",
                "---\ndescription: Who Acme is.\n---\n# Company\n",
            ),
        ]);
        assert!(matches!(
            render(&tree, &key(), &name()).unwrap(),
            Rendered::Files(_)
        ));
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

    /// The catalog with the plugin's entry set at `version`, or the
    /// finding that it cannot be read.
    fn catalog(
        tree: &Committed,
        version: &str,
    ) -> Result<Result<Option<PlannedFile>, PluginFinding>, PublicationError> {
        let version = PluginVersion::parse(version).unwrap();
        match read_catalog(tree) {
            Ok(Some((document, bytes))) => {
                catalog_entry(document, Some(bytes), &key(), &name(), &version).map(Ok)
            }
            Ok(None) => catalog_entry(
                new_catalog(&key(), &name()),
                None,
                &key(),
                &name(),
                &version,
            )
            .map(Ok),
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
        let planned = plan(&view(&first), &key(), &name()).unwrap();
        let plan1 = planned.drift.clone().unwrap();
        let rendered = applied_plan(&first, &planned);
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
        stray.push(("skills/acme/notes.md".to_owned(), "old".to_owned()));
        let cleaned = drift_of(&view(&stray));
        assert_eq!(cleaned.removals, [PathBuf::from("skills/acme/notes.md")]);
        assert_eq!(cleaned.version, plan1.version.raised());
    }

    /// `tree` with the plan's skill files and catalog applied.
    fn applied_plan(tree: &[(String, String)], plan: &PluginPlan) -> Vec<(String, String)> {
        let mut files = applied(tree, plan.drift.as_ref().unwrap());
        if let Some(catalog) = &plan.catalog {
            files.retain(|(path, _)| path != CATALOG);
            files.push((CATALOG.to_owned(), catalog.text.clone()));
        }
        files
    }

    #[test]
    fn the_entry_carries_the_version_kept_or_raised_against_the_skill() {
        let first = files_of(&[
            (INDEX, "expanded: [company.md]\n"),
            ("docs/company.md", "# Company\n\nWe make things.\n"),
            (CATALOG, CATALOG_TEXT),
        ]);
        let plan1 = plan(&view(&first), &key(), &name()).unwrap();
        let rendered = applied_plan(&first, &plan1);
        let version = |files: &[(String, String)]| -> PluginVersion {
            let text = &files.iter().find(|(path, _)| path == CATALOG).unwrap().1;
            let catalog: Json = serde_json::from_str(text).unwrap();
            PluginVersion::parse(catalog["plugins"][1]["version"].as_str().unwrap()).unwrap()
        };
        assert_eq!(version(&rendered), PluginVersion::first());
        // Unchanged: nothing is written, the catalog included.
        let again = plan(&view(&rendered), &key(), &name()).unwrap();
        assert_eq!(again.catalog, None);
        assert!(again.drift.unwrap().writes.is_empty());
        // A changed document raises the entry's version.
        let mut changed = rendered.clone();
        for (path, text) in &mut changed {
            if path == "docs/company.md" {
                *text = "# Company\n\nWe make more things.\n".to_owned();
            }
        }
        let raised = plan(&view(&changed), &key(), &name()).unwrap();
        assert_eq!(
            version(&applied_plan(&changed, &raised)),
            PluginVersion::first().raised()
        );
    }

    #[test]
    fn the_skill_stands_at_the_root_and_no_manifest_is_written() {
        let tree = repository(Some(CATALOG_TEXT), &[]);
        let paths: Vec<PathBuf> = drift_of(&tree)
            .writes
            .into_iter()
            .map(|file| file.path)
            .collect();
        assert_eq!(paths, [PathBuf::from("skills/acme/SKILL.md")]);
    }

    #[test]
    fn findings_that_leave_the_version_unknown_leave_the_catalog_unwritten() {
        let tree = Committed::of(&[(CATALOG, CATALOG_TEXT)]);
        let plan = plan(&tree, &key(), &name()).unwrap();
        assert_eq!(plan.findings, [PluginFinding::NoIndex]);
        assert_eq!(plan.drift, None);
        assert_eq!(plan.catalog, None);
    }

    #[test]
    fn the_entry_is_set_by_value_and_every_other_member_keeps_its_value_and_place() {
        let listed = r#"{"name": "acme-tools", "plugins": [{"name": "acme", "version": "9", "category": "knowledge", "source": "./old"}, {"name": "other"}], "owner": {"name": "Acme"}}"#;
        let tree = repository(Some(listed), &[]);
        let file = catalog(&tree, "1.2.3").unwrap().unwrap().unwrap();
        // The entry's values, every other member in its place, and the
        // document's form, as written.
        insta::assert_snapshot!("entry_set_by_value", file.text);
        // Once set, the entry holds its values: the catalog is left as it is.
        let tree = repository(Some(&file.text), &[]);
        assert_eq!(catalog(&tree, "1.2.3").unwrap(), Ok(None));
    }

    #[test]
    fn an_entry_the_catalog_does_not_list_is_added_after_the_others() {
        let tree = repository(Some(CATALOG_TEXT), &[]);
        let file = catalog(&tree, "0.1.0").unwrap().unwrap().unwrap();
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
    fn a_missing_catalog_is_created_and_an_unreadable_one_is_a_finding() {
        let file = catalog(&repository(None, &[]), "0.1.0")
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(file.current, None);
        let written: Json = serde_json::from_str(&file.text).unwrap();
        assert_eq!(written["name"], "acme");
        assert_eq!(written["owner"], serde_json::json!({"name": "Acme"}));
        let names: Vec<&String> = written.as_object().unwrap().keys().collect();
        assert_eq!(names, ["name", "owner", "plugins"]);
        assert_eq!(written["plugins"].as_array().unwrap().len(), 1);
        assert_eq!(written["plugins"][0]["name"], "acme");
        for text in ["[]", "{\"plugins\": {}}", "{"] {
            assert!(
                matches!(
                    catalog(&repository(Some(text), &[]), "0.1.0").unwrap(),
                    Err(PluginFinding::CatalogUnreadable { .. })
                ),
                "{text:?}"
            );
        }
    }

    /// The output of the renderer before the skill moved to the root: its
    /// manifest held the version, and its catalog entry none.
    const EARLIER_MANIFEST: &str = "{\n  \"name\": \"acme\",\n  \"version\": \"0.3.7\",\n  \"description\": \"The knowledge of Acme, as the company keeps it in its own repository.\",\n  \"author\": {\n    \"name\": \"Acme\"\n  }\n}\n";
    const EARLIER_CATALOG: &str = "{\n  \"name\": \"acme-tools\",\n  \"owner\": {\n    \"name\": \"Acme\"\n  },\n  \"plugins\": [\n    {\n      \"name\": \"acme\",\n      \"source\": \"./plugins/acme\",\n      \"description\": \"The knowledge of Acme, as the company keeps it in its own repository.\"\n    }\n  ]\n}\n";

    #[test]
    fn an_earlier_renderings_output_upgrades_with_its_version_and_then_converges() {
        let earlier = files_of(&[
            (INDEX, "expanded: [company.md]\n"),
            ("docs/company.md", "# Company\n\nWe make things.\n"),
            (CATALOG, EARLIER_CATALOG),
            ("plugins/acme/.claude-plugin/plugin.json", EARLIER_MANIFEST),
            (
                "plugins/acme/skills/acme/SKILL.md",
                "an earlier rendering\n",
            ),
        ]);
        let upgrade = plan(&view(&earlier), &key(), &name()).unwrap();
        assert_eq!(upgrade.findings, []);
        // The skill is written at the root, a step above the version the
        // earlier manifest held; its folder is left as it is.
        let drift = upgrade.drift.clone().unwrap();
        let carried = PluginVersion::parse("0.3.7").unwrap().raised();
        assert_eq!(drift.version, carried);
        assert!(drift.removals.is_empty());
        let written: Vec<&Path> = drift
            .writes
            .iter()
            .map(|file| file.path.as_path())
            .collect();
        assert_eq!(written, [Path::new("skills/acme/SKILL.md")]);
        let upgraded = applied_plan(&earlier, &upgrade);
        insta::assert_snapshot!(
            "upgraded_catalog",
            &upgraded.iter().find(|(path, _)| path == CATALOG).unwrap().1
        );
        // What the earlier rendering left is kept as it was.
        let left = |files: &[(String, String)]| -> Vec<(String, String)> {
            let earlier = [
                "plugins/acme/.claude-plugin/plugin.json",
                "plugins/acme/skills/acme/SKILL.md",
            ];
            files
                .iter()
                .filter(|(path, _)| earlier.contains(&path.as_str()))
                .cloned()
                .collect()
        };
        assert_eq!(left(&upgraded), left(&earlier));
        // Rendering again changes nothing.
        let again = plan(&view(&upgraded), &key(), &name()).unwrap();
        assert_eq!(again.findings, []);
        assert_eq!(again.catalog, None);
        let again = again.drift.unwrap();
        assert!(again.writes.is_empty() && again.removals.is_empty());
        assert_eq!(again.version, carried);
    }

    /// The findings of planning a repository whose catalog holds the earlier
    /// versionless entry, beside `more`.
    fn versionless_findings(more: &[(&str, &str)]) -> Vec<PluginFinding> {
        let mut files = vec![
            (INDEX, "expanded: [company.md]\n"),
            ("docs/company.md", "# Company\n"),
            (CATALOG, EARLIER_CATALOG),
        ];
        files.extend_from_slice(more);
        plan(&Committed::of(&files), &key(), &name())
            .unwrap()
            .findings
    }

    #[test]
    fn a_versionless_entry_without_an_earlier_manifest_is_a_finding() {
        assert_eq!(
            versionless_findings(&[]),
            [PluginFinding::EntryVersion { version: None }]
        );
    }

    #[test]
    fn a_versionless_entry_beside_a_manifest_version_that_is_not_three_numbers_is_a_finding() {
        assert_eq!(
            versionless_findings(&[(
                "plugins/acme/.claude-plugin/plugin.json",
                "{\"version\": \"1.0\"}\n"
            )]),
            [PluginFinding::EntryVersion { version: None }]
        );
    }

    #[test]
    fn an_entry_whose_version_is_not_three_numbers_is_a_finding_naming_it() {
        let held = |entry: &str| {
            let document: Json =
                serde_json::from_str(&format!("{{\"plugins\": [{entry}]}}")).unwrap();
            held_version(&Committed::of(&[]), &document, &key())
        };
        assert_eq!(
            held(r#"{"name": "acme", "version": "1.0"}"#),
            Err(PluginFinding::EntryVersion {
                version: Some("1.0".to_owned()),
            })
        );
        assert_eq!(
            held(r#"{"name": "acme"}"#),
            Err(PluginFinding::EntryVersion { version: None })
        );
        assert_eq!(
            held(r#"{"name": "acme", "version": "2.3.4"}"#)
                .unwrap()
                .map(|v| v.to_string())
                .as_deref(),
            Some("2.3.4")
        );
        assert_eq!(held(r#"{"name": "other", "version": "x"}"#), Ok(None));
        let plan = plan(
            &repository(
                Some(r#"{"plugins": [{"name": "acme", "version": "one"}]}"#),
                &[],
            ),
            &key(),
            &name(),
        )
        .unwrap();
        assert_eq!(
            plan.findings,
            [PluginFinding::EntryVersion {
                version: Some("one".to_owned())
            }]
        );
        assert_eq!((plan.drift, plan.catalog), (None, None));
    }

    #[test]
    fn a_link_on_the_way_to_the_skill_or_the_catalog_or_inside_the_skill_is_a_finding() {
        for link in [
            "skills",
            "skills/acme",
            "skills/acme/x.md",
            ".claude-plugin",
        ] {
            let tree = repository(Some(CATALOG_TEXT), &[]).with_link(link);
            let plan = plan(&tree, &key(), &name()).unwrap();
            // Behind a link at the skill's folder, or at the catalog that
            // holds the version, nothing is compared.
            assert!(plan.drift.is_none(), "{link}");
            assert_eq!(
                plan.findings,
                [PluginFinding::Link {
                    path: link.to_owned()
                }],
                "{link}"
            );
        }
        for elsewhere in ["skills/other", "plugins/acme"] {
            let tree = repository(Some(CATALOG_TEXT), &[]).with_link(elsewhere);
            assert!(
                plan(&tree, &key(), &name()).unwrap().findings.is_empty(),
                "{elsewhere}"
            );
        }
    }

    #[test]
    fn a_plugins_folder_is_never_read_written_or_removed() {
        let tree = repository(
            Some(CATALOG_TEXT),
            &[("plugins/acme/skills/acme/SKILL.md", "an earlier layout")],
        );
        let drift = drift_of(&tree);
        assert!(drift.removals.is_empty());
        let written: Vec<&Path> = drift
            .writes
            .iter()
            .map(|file| file.path.as_path())
            .collect();
        assert_eq!(written, [Path::new("skills/acme/SKILL.md")]);
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
