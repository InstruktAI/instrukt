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

use serde_yaml_ng::Value;

use super::_company::{self, COMPANY_PASSAGE, DOCS, Selection, listed_line};
use super::_render::{PlannedFile, RenderFinding, RenderFindingKind, company_home, finish};
use super::_source::{Committed, EntryIndex};
use super::_units::{LEGEND, Unit, frame};
use super::PublicationError;
use crate::config::{
    CONFIG_FILE_NAME, CompanyIdentity, CompanyName, IdentityFinding, RootDocument, classify_root,
};
use crate::docs::{
    self, CompanyFrontmatter, CompanyPath, CompanyReference, CompanyText, company_paths,
};
use crate::ids::ProjectSlug;

mod _catalog;

pub(super) use _catalog::earlier_manifest;
use _catalog::{Catalog, catalog_entry, held_version, new_catalog, read_catalog};

/// The folder below a repository's root that holds its skills.
const SKILLS: &str = "skills";

/// The repository's plugin catalog, from its root.
pub(super) const CATALOG: &str = ".claude-plugin/marketplace.json";

/// The skill's main file, inside the skill's folder.
const SKILL_FILE: &str = "SKILL.md";

/// The folder of the skill that holds each document carried as a file of
/// its own, at its path relative to `docs/`.
const REFERENCES: &str = "references";

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
    /// The company's repository declares no company identity: configuration's
    /// finding, which a rendering, a check and a validation refuse with.
    Identity(IdentityFinding),
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
            Self::Identity(finding) => write!(f, "{CONFIG_FILE_NAME}: {finding}"),
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

/// The company identity the root project layer of `tree` declares, once
/// `tree` is a company's repository holding a company index; otherwise the
/// finding that leaves the plugin unrendered. The identity is read from
/// configuration's operation alone, never from a caller.
pub(super) fn identity(tree: &Committed) -> Result<CompanyIdentity, PluginFinding> {
    let document = tree.file(CONFIG_FILE_NAME);
    let root = classify_root(document);
    let index_absent = tree.entry_index(_company::COMPANY_INDEX) == EntryIndex::Absent;
    match &root {
        RootDocument::Unreadable(reason) => {
            return Err(PluginFinding::UnreadableRoot {
                reason: reason.clone(),
            });
        }
        RootDocument::ProjectLayer {
            company_repository: false,
            ..
        } => return Err(PluginFinding::ProjectRepository),
        root if !company_home(root, index_absent) => return Err(PluginFinding::NoIndex),
        _ => {}
    }
    crate::config::company_identity(document).map_err(PluginFinding::Identity)
}

/// Renders the plugin of the company `identity` names from `tree`.
fn render(tree: &Committed, identity: &CompanyIdentity) -> Result<Rendered, PublicationError> {
    let CompanyIdentity { key, name } = identity;
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
            file: format!("{}/{SKILL_FILE}", folder(key)),
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
            let document = docs::company_body(&unit.body);
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
    /// The skill's folder, relative to the repository's root; `None` when the
    /// repository declares no company identity to name it by.
    pub folder: Option<PathBuf>,
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

/// Reads what `tree` says of the plugin of the company `identity` names, and
/// every finding, comparing nothing.
fn assess<'t>(
    tree: &'t Committed,
    identity: &CompanyIdentity,
) -> Result<Assessment<'t>, PublicationError> {
    let key = &identity.key;
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
    let rendered = render(tree, identity)?;
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

/// Every finding of the company's plugin in `tree`, a view holding the
/// skill's folder and the catalog, without comparing any file. A tree that
/// declares no company identity has that finding alone.
pub(super) fn findings(tree: &Committed) -> Result<Vec<PluginFinding>, PublicationError> {
    match identity(tree) {
        Ok(identity) => Ok(assess(tree, &identity)?.findings),
        Err(finding) => Ok(vec![finding]),
    }
}

/// Plans the rendering of the company's plugin from `tree`, under the
/// company identity its root project layer declares, a view holding the
/// skill's folder and the catalog
/// ([`Committed::working_tree_places`]): every finding, and what the
/// rendering changes wherever that can be known, findings or not. The
/// drift comes first, because the catalog entry carries the version it
/// keeps or raises.
pub(super) fn plan(tree: &Committed) -> Result<PluginPlan, PublicationError> {
    match identity(tree) {
        Ok(identity) => planned(tree, &identity),
        Err(finding) => Ok(PluginPlan {
            folder: None,
            findings: vec![finding],
            drift: None,
            catalog: None,
        }),
    }
}

/// [`plan`], under the company identity already read from `tree`.
fn planned(tree: &Committed, identity: &CompanyIdentity) -> Result<PluginPlan, PublicationError> {
    let CompanyIdentity { key, name } = identity;
    let assessment = assess(tree, identity)?;
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
        folder: Some(PathBuf::from(folder(key))),
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
mod tests;
