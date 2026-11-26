//! Compressed units (`project/instrukt/spec/feature/publication/compressed-units`;
//! `project/instrukt/spec/domain/publication`, Compressed units of a
//! repository): the place for units, a unit's format, the source digest of
//! delivered prose, each unit's state, and the content findings that keep a
//! unit from standing in for its source.
//!
//! Every reader decides the same way from the same bytes: a unit is read
//! once — strict UTF-8, its frontmatter split once and decoded strictly —
//! and every check and every rendering reads the one body that read
//! produced. instrukt checks mechanical facts only (digests, directives,
//! references, places and links), never what a unit's text means.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::Deserialize;
use sha2::{Digest, Sha256};

use super::_render::AGENTS_FILE;
use super::_source::Committed;
use super::PublicationError;
use crate::config::{CONFIG_FILE_NAME, RootDocument, classify_root};
use crate::docs::{
    self, CompanyReference, Corpus, Directive, Lens, Position, PositionIndex, RefTarget, Reference,
    View,
};
use crate::home::Home;
use crate::ids::{DomainName, ProjectSlug, SnippetId};

/// The place for units in a repository: every unit stands at its source's
/// path relative to `docs/` below it.
pub(super) const PLACE: &str = "docs/_compressed";

/// The folder name reserved for units: the place's own name in a
/// repository, and inside a published position in the home.
pub(super) const RESERVED: &str = "_compressed";

/// The legend of the units' notation (`general/spec/agent-shorthand`,
/// Compressed-unit notation): one fixed passage a rendering that holds at
/// least one unit carries once, in its opening, before its first document.
pub(super) const LEGEND: &str = "Compressed sections (headed by document name) hold rules, not prose: `MUST` required, `NEVER` forbidden, `MAY` permitted, `PREFER` default unless reasoned; ` -- ` reason; `<term> = <meaning>` defines; `->` then, `|` or, `vs` tension; ids, commands, paths, code, configured values verbatim. Read the named document first for cases, wording or reasons left open.";

/// A unit's `body` held where its source would be: one heading naming the
/// source, `name`, at the level of the source's own title, then the body.
/// Framing reads nothing of the body's prose.
pub(super) fn frame(name: &str, body: &str) -> String {
    format!("# {name}\n\n{}\n", body.trim_matches('\n'))
}

/// The SHA-256 of `bytes`, in lowercase hexadecimal.
pub(super) fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// The source digest of `prose`, a source's delivered prose: the SHA-256 of
/// its text with its line endings as line feeds.
pub(super) fn source_digest(prose: &str) -> String {
    sha256_hex(prose.replace("\r\n", "\n").as_bytes())
}

/// The source digest of the snippet whose whole published text stands at
/// `rel` below the home's published tree `docs`, read through no symbolic
/// link; why it cannot be read otherwise.
pub(crate) fn published_digest(docs: &Path, rel: &Path) -> Result<String, String> {
    let path = super::_tx::confined(docs, rel).map_err(|err| err.to_string())?;
    let text =
        std::fs::read_to_string(&path).map_err(|err| format!("{}: {err}", path.display()))?;
    Ok(source_digest(&docs::current_contract_body(&text)))
}

/// Where the unit of the source at `source`, a repository-relative path
/// below `docs/`, belongs; `None` for a path outside `docs/`.
pub(super) fn unit_path(source: &str) -> Option<String> {
    source
        .strip_prefix("docs/")
        .map(|rel| format!("{PLACE}/{rel}"))
}

/// The frontmatter a unit declares. No other key is allowed.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Declared {
    source_sha256: String,
    model: String,
}

/// One unit as read: the facts its frontmatter declares and the body after
/// it, the one text every check and rendering of the unit reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Unit {
    /// Where it was read from: repository-relative, or in the home.
    pub path: String,
    /// The source digest it declares: 64 lowercase hexadecimal digits.
    pub source_sha256: String,
    /// The model it declares, non-empty.
    pub model: String,
    pub body: String,
}

impl Unit {
    /// The unit `bytes` hold, read from `path`, or the unreadable finding.
    pub(super) fn parse(path: &str, bytes: &[u8]) -> Result<Self, UnitFinding> {
        let unreadable = |reason: String| UnitFinding {
            path: path.to_owned(),
            kind: UnitFindingKind::Unreadable { reason },
        };
        let text = std::str::from_utf8(bytes)
            .map_err(|_| unreadable("it is not valid UTF-8".to_owned()))?;
        let (frontmatter, body) = docs::split_frontmatter(text);
        let frontmatter =
            frontmatter.ok_or_else(|| unreadable("it opens with no frontmatter".to_owned()))?;
        let declared: Declared = serde_yaml_ng::from_str(frontmatter)
            .map_err(|err| unreadable(format!("its frontmatter cannot be read: {err}")))?;
        let digest_ok = declared.source_sha256.len() == 64
            && declared
                .source_sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        if !digest_ok {
            return Err(unreadable(
                "its `source_sha256` is not 64 lowercase hexadecimal digits".to_owned(),
            ));
        }
        if declared.model.trim().is_empty() {
            return Err(unreadable("its `model` is empty".to_owned()));
        }
        Ok(Self {
            path: path.to_owned(),
            source_sha256: declared.source_sha256,
            model: declared.model,
            body: body.to_owned(),
        })
    }

    /// The SHA-256 of the unit's body.
    pub(super) fn body_digest(&self) -> String {
        sha256_hex(self.body.as_bytes())
    }
}

/// Something that keeps a unit from standing in for its source, naming the
/// unit, or the folder or link it concerns.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct UnitFinding {
    pub path: String,
    pub kind: UnitFindingKind,
}

/// What was found (`project/instrukt/spec/domain/publication`, Unit
/// findings).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum UnitFindingKind {
    /// The unit is not valid UTF-8, or its frontmatter is missing, carries
    /// another key, or lacks or malforms one, or it cannot be read.
    Unreadable { reason: String },
    /// A symbolic link at, above or inside the place units are read from.
    Link,
    /// A position holds an entry named `_compressed`.
    ReservedFolder,
    /// The unit carries `@exec` or `@include`.
    ForbiddenDirective { directive: String },
    /// The unit's `@config` paths, as a multiset, differ from its source's.
    ConfiguredValues {
        unit: Vec<String>,
        source: Vec<String>,
    },
    /// The unit carries a reference its source does not.
    Reference { reference: String },
    /// The unit, as one rendering holds it, breaks a rule that rendering
    /// applies to its source.
    RenderingRule { rule: String },
}

impl UnitFindingKind {
    /// The finding's name, as `instrukt docs units --json` reports it.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Unreadable { .. } => "unreadable",
            Self::Link => "link",
            Self::ReservedFolder => "reserved-folder",
            Self::ForbiddenDirective { .. } => "forbidden-directive",
            Self::ConfiguredValues { .. } => "configured-values",
            Self::Reference { .. } => "references",
            Self::RenderingRule { .. } => "rendering-rule",
        }
    }

    /// What the finding says about its path, as a clause.
    pub fn detail(&self) -> String {
        match self {
            Self::Unreadable { reason } => format!("cannot be read as a unit: {reason}"),
            Self::Link => "is a symbolic link, through which no unit is read".to_owned(),
            Self::ReservedFolder => format!(
                "is named `{RESERVED}`, which is reserved for units and never part of a position"
            ),
            Self::ForbiddenDirective { directive } => {
                format!("carries `{directive}`, which no rendering supports")
            }
            Self::ConfiguredValues { unit, source } => format!(
                "reads the configured values [{}] where its source reads [{}]",
                unit.join(", "),
                source.join(", ")
            ),
            Self::Reference { reference } => {
                format!("references `{reference}`, which its source does not")
            }
            Self::RenderingRule { rule } => format!("breaks a rule of the rendering: {rule}"),
        }
    }
}

impl std::fmt::Display for UnitFinding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.path, self.kind.detail())
    }
}

/// A required read as unit checks compare it: what it names, or its text
/// when it names nothing.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum ReadKey {
    Target(RefTarget),
    Raw(String),
}

impl ReadKey {
    fn of(reference: &Reference) -> Self {
        reference
            .target
            .clone()
            .map_or_else(|| Self::Raw(reference.raw.clone()), Self::Target)
    }
}

/// What a source's unit is compared with.
#[derive(Debug, Clone)]
enum Parity {
    Snippet {
        project: Option<ProjectSlug>,
        configs: Vec<String>,
        links: BTreeSet<String>,
        reads: BTreeSet<ReadKey>,
        visible: BTreeSet<String>,
    },
    Company {
        configs: Vec<String>,
        references: Vec<CompanyReference>,
        reads: Vec<String>,
    },
}

/// What an entrypoint rendering holds in full: a delivered snippet or a
/// company document, with its delivered prose's digest.
#[derive(Debug, Clone)]
pub(super) struct Source {
    /// Its canonical id, or a company document's path.
    pub name: String,
    /// Its file, repository-relative.
    pub path: String,
    /// Its source digest.
    pub digest: String,
    parity: Parity,
}

impl Source {
    /// The snippet `id` at `path` whose version body is `body` and whose
    /// delivered prose under `lens` is `prose`, its source digest computed
    /// from that prose: what the repository readers, which publish nothing,
    /// decide currency by.
    pub(super) fn snippet(
        id: &SnippetId,
        path: &str,
        body: &str,
        prose: &str,
        lens: Lens<'_>,
        project: Option<&ProjectSlug>,
    ) -> Self {
        Self::with_digest(id, path, body, prose, lens, project, source_digest(prose))
    }

    /// The published snippet `id` at `path`, under the current contract,
    /// whose source digest is `digest` as its owner's sync recorded it in the
    /// global index (`project/instrukt/spec/feature/publication/compressed-units`,
    /// Lineage): its prose is read for the unit's checks, never digested again.
    pub(super) fn recorded(
        id: &SnippetId,
        path: &str,
        body: &str,
        prose: &str,
        project: Option<&ProjectSlug>,
        digest: &str,
    ) -> Self {
        Self::with_digest(
            id,
            path,
            body,
            prose,
            Lens::Current,
            project,
            digest.to_owned(),
        )
    }

    fn with_digest(
        id: &SnippetId,
        path: &str,
        body: &str,
        prose: &str,
        lens: Lens<'_>,
        project: Option<&ProjectSlug>,
        digest: String,
    ) -> Self {
        Self {
            name: id.as_str().to_owned(),
            path: path.to_owned(),
            digest,
            parity: Parity::Snippet {
                project: project.cloned(),
                configs: config_paths(prose),
                links: docs::reference_link_targets(prose).into_iter().collect(),
                reads: docs::lens_required_reads(body, lens, project)
                    .iter()
                    .map(ReadKey::of)
                    .collect(),
                visible: docs::visible_references(prose).into_iter().collect(),
            },
        }
    }

    /// The company document at `path` whose whole text is `text`.
    pub(super) fn company(path: &str, text: &str) -> Self {
        let (_, prose) = docs::split_frontmatter(text);
        Self {
            name: path.to_owned(),
            path: path.to_owned(),
            digest: source_digest(prose),
            parity: Parity::Company {
                configs: config_paths(prose),
                references: docs::company_references(prose),
                reads: docs::company_document(prose).reads,
            },
        }
    }

    /// Where this source's unit belongs in its repository.
    pub(super) fn unit_path(&self) -> String {
        unit_path(&self.path).unwrap_or_else(|| format!("{PLACE}/{}", self.path))
    }

    /// The content findings of `unit`, a current unit of this source: the
    /// directives it forbids, its configured values against the source's,
    /// and each reference it carries that the source does not.
    fn findings(&self, unit: &Unit) -> Vec<UnitFinding> {
        let found = |kind| UnitFinding {
            path: unit.path.clone(),
            kind,
        };
        let mut out: Vec<UnitFinding> = docs::directives(&unit.body)
            .into_iter()
            .filter_map(|directive| match directive {
                Directive::Exec { command, .. } => Some(format!("@exec: {command}")),
                Directive::Include { path, .. } => Some(format!("@include: {path}")),
                Directive::Config { .. } => None,
            })
            .map(|directive| found(UnitFindingKind::ForbiddenDirective { directive }))
            .collect();
        let unit_configs = config_paths(&unit.body);
        let source_configs = match &self.parity {
            Parity::Snippet { configs, .. } | Parity::Company { configs, .. } => configs,
        };
        if &unit_configs != source_configs {
            out.push(found(UnitFindingKind::ConfiguredValues {
                unit: unit_configs,
                source: source_configs.clone(),
            }));
        }
        let mut extra = |reference: String| {
            let finding = found(UnitFindingKind::Reference { reference });
            if !out.contains(&finding) {
                out.push(finding);
            }
        };
        match &self.parity {
            Parity::Snippet {
                project,
                links,
                reads,
                visible,
                ..
            } => {
                for link in docs::reference_link_targets(&unit.body) {
                    if !links.contains(&link) {
                        extra(link);
                    }
                }
                for read in docs::body_required_reads(&unit.body, project.as_ref()) {
                    if !reads.contains(&ReadKey::of(&read)) {
                        extra(read.raw);
                    }
                }
                for reference in docs::visible_references(&unit.body) {
                    if !visible.contains(&reference) {
                        extra(format!("@{reference}"));
                    }
                }
            }
            Parity::Company {
                references, reads, ..
            } => {
                for reference in docs::company_references(&unit.body) {
                    if !references.contains(&reference) {
                        extra(match reference {
                            CompanyReference::Name(name) | CompanyReference::Path(name) => name,
                        });
                    }
                }
                for read in docs::company_document(&unit.body).reads {
                    if !reads.contains(&read) {
                        extra(read);
                    }
                }
            }
        }
        out
    }
}

/// The `@config` paths of `text`, sorted: a multiset.
fn config_paths(text: &str) -> Vec<String> {
    let mut paths: Vec<String> = docs::directives(text)
        .into_iter()
        .filter_map(|directive| match directive {
            Directive::Config { path, .. } => Some(path),
            _ => None,
        })
        .collect();
    paths.sort();
    paths
}

/// What a reader found where a source's unit belongs.
pub(super) type UnitRead = Option<Result<Unit, UnitFinding>>;

/// How a rendering holds one source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Decision {
    /// Its current unit, with no finding.
    Unit(Unit),
    /// Its prose, and why.
    Prose(Fallback),
}

/// Why a source is held in its prose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Fallback {
    /// It has no unit.
    Missing,
    /// Its unit declares another source digest.
    Stale(Unit),
    /// Its unit has findings.
    Findings(Vec<UnitFinding>),
}

/// How `source` is held, given what was read where its unit belongs.
pub(super) fn decide(read: UnitRead, source: &Source) -> Decision {
    match read {
        None => Decision::Prose(Fallback::Missing),
        Some(Err(finding)) => Decision::Prose(Fallback::Findings(vec![finding])),
        Some(Ok(unit)) if unit.source_sha256 != source.digest => {
            Decision::Prose(Fallback::Stale(unit))
        }
        Some(Ok(unit)) => {
            let findings = source.findings(&unit);
            if findings.is_empty() {
                Decision::Unit(unit)
            } else {
                Decision::Prose(Fallback::Findings(findings))
            }
        }
    }
}

/// The units of one view of a repository: read from its place for units
/// alone, never through a symbolic link at, above or inside it.
pub(super) struct RepositoryUnits<'v> {
    view: &'v Committed,
}

impl<'v> RepositoryUnits<'v> {
    pub(super) fn of(view: &'v Committed) -> Self {
        Self { view }
    }

    /// What stands where the unit of the source at `source` belongs.
    pub(super) fn read(&self, source: &str) -> UnitRead {
        let path = unit_path(source)?;
        if let Some(link) = self.link_on(&path) {
            return Some(Err(UnitFinding {
                path: link,
                kind: UnitFindingKind::Link,
            }));
        }
        self.view.file(&path).map(|bytes| Unit::parse(&path, bytes))
    }

    /// How `source` is held.
    pub(super) fn decide(&self, source: &Source) -> Decision {
        decide(self.read(&source.path), source)
    }

    /// The link at or above `path`, if any.
    fn link_on(&self, path: &str) -> Option<String> {
        let mut prefix = path;
        loop {
            if self.view.links().any(|link| link == prefix) {
                return Some(prefix.to_owned());
            }
            prefix = prefix.rsplit_once('/')?.0;
        }
    }

    /// Every link at, above or inside the place for units.
    pub(super) fn link_findings(&self) -> Vec<UnitFinding> {
        self.view
            .links()
            .filter(|link| {
                *link == "docs" || *link == PLACE || link.starts_with(&format!("{PLACE}/"))
            })
            .map(|link| UnitFinding {
                path: link.to_owned(),
                kind: UnitFindingKind::Link,
            })
            .collect()
    }

    /// Every unit file of the view, by repository-relative path.
    pub(super) fn files(&self) -> impl Iterator<Item = String> + '_ {
        self.view
            .below("docs/_compressed/")
            .map(|(rel, _)| format!("{PLACE}/{rel}"))
    }
}

/// The place for the units of `position` in a repository:
/// `docs/_compressed/<the position's path below docs/>`.
pub(super) fn position_place(position: &Position) -> String {
    let dir = position.dir();
    format!("{PLACE}/{}", dir.strip_prefix("docs/").unwrap_or(&dir))
}

/// The content findings of the units a sync publishes: every unit below
/// the place of each of `positions`, and every link at, above or inside
/// those places. A unit of a snippet the view holds is compared with it
/// while it is current; any other unit is only read.
pub(super) fn published_findings(
    view: &Committed,
    corpus: &Corpus,
    positions: &[Position],
    project: Option<&ProjectSlug>,
) -> Vec<UnitFinding> {
    let places: Vec<String> = positions.iter().map(position_place).collect();
    let within = |path: &str| {
        places
            .iter()
            .any(|place| path == place || path.starts_with(&format!("{place}/")))
    };
    let units = RepositoryUnits::of(view);
    let snippets: BTreeMap<String, &docs::Snippet> = corpus
        .snippets()
        .iter()
        .map(|snippet| (snippet.path().display().to_string(), snippet))
        .collect();
    let mut found: BTreeSet<UnitFinding> = units
        .link_findings()
        .into_iter()
        .filter(|finding| within(&finding.path) || finding.path == "docs" || finding.path == PLACE)
        .collect();
    for file in units.files().filter(|file| within(file)) {
        let source_path = format!("docs/{}", &file[PLACE.len() + 1..]);
        let visible =
            snippets
                .get(&source_path)
                .and_then(|snippet| match snippet.view(Lens::Current) {
                    View::Visible { version, body } => Some(Source::snippet(
                        snippet.id(),
                        &source_path,
                        version.body(),
                        &body,
                        Lens::Current,
                        project,
                    )),
                    View::Withheld { .. } | View::Retired => None,
                });
        match (units.read(&source_path), visible) {
            (Some(read), Some(source)) => {
                if let Decision::Prose(Fallback::Findings(findings)) = decide(Some(read), &source) {
                    found.extend(findings);
                }
            }
            (Some(Err(finding)), None) => {
                found.insert(finding);
            }
            _ => {}
        }
    }
    found.into_iter().collect()
}

/// The units a published position holds in the home: read from
/// `<position>/_compressed/` alone, never through a symbolic link at, above
/// or inside it below the home's published tree.
pub(super) fn read_home(home: &Home, position: &DomainName, rel: &str) -> UnitRead {
    let relative = Path::new(position.as_str()).join(RESERVED).join(rel);
    let shown = home.docs_dir().join(&relative).display().to_string();
    let found = |kind| {
        Some(Err(UnitFinding {
            path: shown.clone(),
            kind,
        }))
    };
    let path = match super::_tx::confined(&home.docs_dir(), &relative) {
        Ok(path) => path,
        Err(PublicationError::Unconfined { path }) => {
            return Some(Err(UnitFinding {
                path: path.display().to_string(),
                kind: UnitFindingKind::Link,
            }));
        }
        Err(err) => {
            return found(UnitFindingKind::Unreadable {
                reason: err.to_string(),
            });
        }
    };
    match std::fs::read(&path) {
        Ok(bytes) => Some(Unit::parse(&shown, &bytes)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
        Err(err) => found(UnitFindingKind::Unreadable {
            reason: err.to_string(),
        }),
    }
}

/// Every entry of a position the view holds — file, folder or link — whose
/// path has a segment named `_compressed`: each a finding naming the path
/// up to that segment, once.
pub(super) fn reserved_folders(view: &Committed) -> Vec<UnitFinding> {
    let paths = view
        .below("docs/")
        .map(|(rel, _)| format!("docs/{rel}"))
        .chain(view.links().map(str::to_owned));
    let mut found = BTreeSet::new();
    for path in paths {
        let Some(within) = position_rest(&path) else {
            continue;
        };
        let mut at = path.len() - within.len();
        for segment in within.split('/') {
            at += segment.len();
            if segment == RESERVED {
                found.insert(path[..at].to_owned());
                break;
            }
            at += 1;
        }
    }
    found
        .into_iter()
        .map(|path| UnitFinding {
            path,
            kind: UnitFindingKind::ReservedFolder,
        })
        .collect()
}

/// The part of `path` inside its position — below `docs/general/`,
/// `docs/domain/<x>/` or `docs/project/` — or `None` outside every position.
fn position_rest(path: &str) -> Option<&str> {
    if let Some(rest) = path
        .strip_prefix("docs/general/")
        .or_else(|| path.strip_prefix("docs/project/"))
    {
        return Some(rest);
    }
    path.strip_prefix("docs/domain/")?
        .split_once('/')
        .map(|(_, rest)| rest)
}

/// What one view of a repository holds for its units: every source its
/// entrypoint indexes inline, and every snippet it holds by path.
pub(super) struct RepositorySources {
    /// Each source, in traversal order, once.
    pub sources: Vec<Source>,
    /// Each domain snippet the view holds, visible through the lens, by
    /// its path: its unit is orphaned only when the snippet no longer
    /// exists, and current or stale otherwise.
    pub domain_snippets: BTreeMap<String, Option<Source>>,
}

/// The sources of the repository in `view`, read through `lens`: the
/// closure of `docs/general/index.yaml`'s `expanded` and `agents` ids, of
/// `docs/project/index.yaml`'s `expanded` ids, and of a company index's
/// `expanded` documents, restricted to delivered sources the view holds.
pub(super) fn repository_sources(
    view: &Committed,
    lens: Lens<'_>,
) -> Result<RepositorySources, PublicationError> {
    let root = classify_root(view.file(CONFIG_FILE_NAME));
    let company_absent =
        view.entry_index(super::_company::COMPANY_INDEX) == super::_source::EntryIndex::Absent;
    if super::_render::company_home(&root, company_absent) {
        let mut sources = Vec::new();
        if let Some(reading) = super::_company::read(view, Path::new(AGENTS_FILE)) {
            for path in &reading.selection.held {
                if let Some(selected) = reading.selection.documents.get(path) {
                    sources.push(Source::company(path, &selected.text));
                }
            }
        }
        return Ok(RepositorySources {
            sources,
            domain_snippets: BTreeMap::new(),
        });
    }
    let project = match root {
        RootDocument::ProjectLayer(project) => project,
        _ => None,
    };
    let corpus = Corpus::from_sources(project.clone(), None, view.doc_sources());
    let (held, _reserved) = super::_tree::held_positions(view);
    let mut walk = Walk {
        view,
        corpus: &corpus,
        lens,
        project: project.as_ref(),
        held: held.into_iter().collect(),
        seen: BTreeSet::new(),
        domains: BTreeSet::new(),
        sources: Vec::new(),
    };
    if let Some(PositionIndex::General(general)) =
        super::_sync::committed_index(view, &Position::General)?
    {
        walk.expand(&general.expanded)?;
        for ids in general.agents.values() {
            walk.expand(ids)?;
        }
    }
    if project.is_some()
        && let Some(index) = super::_sync::committed_index(view, &Position::Project)?
    {
        walk.expand(index.expanded())?;
    }
    let mut domain_snippets = BTreeMap::new();
    for snippet in corpus.snippets() {
        let path = snippet.path().display().to_string();
        if !path.starts_with("docs/domain/") {
            continue;
        }
        let source = match snippet.view(lens) {
            View::Visible { version, body } => Some(Source::snippet(
                snippet.id(),
                &path,
                version.body(),
                &body,
                lens,
                project.as_ref(),
            )),
            View::Withheld { .. } | View::Retired => None,
        };
        domain_snippets.insert(path, source);
    }
    Ok(RepositorySources {
        sources: walk.sources,
        domain_snippets,
    })
}

/// The closure walk of [`repository_sources`].
struct Walk<'a> {
    view: &'a Committed,
    corpus: &'a Corpus,
    lens: Lens<'a>,
    project: Option<&'a ProjectSlug>,
    held: BTreeSet<DomainName>,
    seen: BTreeSet<SnippetId>,
    domains: BTreeSet<DomainName>,
    sources: Vec<Source>,
}

impl Walk<'_> {
    fn expand(&mut self, ids: &[SnippetId]) -> Result<(), PublicationError> {
        for id in ids {
            self.follow(id)?;
        }
        Ok(())
    }

    /// `id` after its required reads, once, when the view holds it and the
    /// lens shows it; a bare-domain read as that domain's `expanded` set.
    fn follow(&mut self, id: &SnippetId) -> Result<(), PublicationError> {
        if !self.seen.insert(id.clone()) || !self.holds(id) {
            return Ok(());
        }
        let Some(snippet) = self.corpus.snippet(id) else {
            return Ok(());
        };
        let View::Visible { version, body } = snippet.view(self.lens) else {
            return Ok(());
        };
        for read in docs::lens_required_reads(version.body(), self.lens, self.project) {
            match read.target {
                Some(RefTarget::Snippet(nested)) => self.follow(&nested)?,
                Some(RefTarget::Domain(domain)) => {
                    if self.held.contains(&domain)
                        && self.domains.insert(domain.clone())
                        && let Some(index) =
                            super::_sync::committed_index(self.view, &Position::of_domain(&domain))?
                    {
                        self.expand(index.expanded())?;
                    }
                }
                _ => {}
            }
        }
        let path = snippet.path().display().to_string();
        self.sources.push(Source::snippet(
            id,
            &path,
            version.body(),
            &body,
            self.lens,
            self.project,
        ));
        Ok(())
    }

    /// Whether the view holds `id`'s position, as the committed rendering
    /// decides it.
    fn holds(&self, id: &SnippetId) -> bool {
        match id.domain() {
            Some(domain) => self.held.contains(&domain),
            None => self.project.is_some_and(|project| {
                id.as_str()
                    .strip_prefix("project/")
                    .and_then(|rest| rest.split_once('/'))
                    .is_some_and(|(slug, _)| slug == project.as_str())
            }),
        }
    }
}

/// What the inspection of a repository's units names
/// (`project/instrukt/spec/feature/publication/compressed-units`,
/// Inspecting units).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UnitsReport {
    /// Each source whose unit declares another source digest, in traversal
    /// order.
    pub stale: Vec<UnitWork>,
    /// Each source without a unit, in traversal order.
    pub missing: Vec<UnitWork>,
    /// Each orphaned unit, by path: one whose source no longer exists, or
    /// whose `general`, project or company source no entrypoint index of
    /// the repository inlines. A domain snippet's unit is orphaned only
    /// when the snippet no longer exists, since another repository's
    /// always-on selection may inline it.
    pub orphaned: Vec<String>,
    /// Each finding the inspection can establish from the checkout, once.
    pub findings: Vec<UnitFinding>,
}

/// One source whose unit needs writing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnitWork {
    /// Its canonical id, or a company document's path.
    pub source: String,
    /// Its file, repository-relative.
    pub file: String,
    /// Where its unit belongs, repository-relative.
    pub unit: String,
    /// The digest of its current delivered prose: the source digest a
    /// renewed unit declares.
    pub source_sha256: String,
}

/// What an inspection's report comes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnitsOutcome {
    /// Every source that needs a unit has a current one; nothing is
    /// orphaned or found.
    NothingToCompress,
    /// It names stale, missing or orphaned units and no finding.
    Work,
    /// It names a finding.
    Findings,
}

impl UnitsReport {
    pub fn outcome(&self) -> UnitsOutcome {
        if !self.findings.is_empty() {
            UnitsOutcome::Findings
        } else if self.stale.is_empty() && self.missing.is_empty() && self.orphaned.is_empty() {
            UnitsOutcome::NothingToCompress
        } else {
            UnitsOutcome::Work
        }
    }
}

/// The inspection of the units of the repository in `view`, its sources
/// read through `lens`. Reads the view alone and writes nothing.
pub(super) fn inspect(view: &Committed, lens: Lens<'_>) -> Result<UnitsReport, PublicationError> {
    let RepositorySources {
        sources,
        domain_snippets,
    } = repository_sources(view, lens)?;
    let units = RepositoryUnits::of(view);
    let mut report = UnitsReport::default();
    let mut findings: BTreeSet<UnitFinding> = BTreeSet::new();
    let mut covered = BTreeSet::new();
    let mut note = |decision: Decision, source: &Source, report: &mut UnitsReport| {
        let work = || UnitWork {
            source: source.name.clone(),
            file: source.path.clone(),
            unit: source.unit_path(),
            source_sha256: source.digest.clone(),
        };
        match decision {
            Decision::Unit(_) => {}
            Decision::Prose(Fallback::Missing) => report.missing.push(work()),
            Decision::Prose(Fallback::Stale(_)) => report.stale.push(work()),
            Decision::Prose(Fallback::Findings(found)) => findings.extend(found),
        }
    };
    for source in &sources {
        covered.insert(source.unit_path());
        note(units.decide(source), source, &mut report);
    }
    for file in units.files() {
        if covered.contains(&file) {
            continue;
        }
        let source_path = format!("docs/{}", &file[PLACE.len() + 1..]);
        match domain_snippets.get(&source_path) {
            Some(Some(source)) => note(units.decide(source), source, &mut report),
            Some(None) => {}
            None => report.orphaned.push(file),
        }
    }
    report.orphaned.sort();
    findings.extend(units.link_findings());
    findings.extend(reserved_folders(view));
    report.findings = findings.into_iter().collect();
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::TodoSlug;

    const LAYER: &str = "schema_version: 6\nproject: demo\n";

    /// A general policy whose rules are `rules`.
    fn policy(name: &str, rules: &str) -> String {
        format!(
            "---\ndescription: About {name}.\n---\n\n# {name} — Policy\n\n## Rules\n\n{rules}\n"
        )
    }

    /// A unit declaring `digest` with `body`.
    fn unit(digest: &str, body: &str) -> String {
        format!("---\nsource_sha256: {digest}\nmodel: test-model\n---\n{body}")
    }

    /// The delivered prose of `text`: its body after the frontmatter.
    fn prose(text: &str) -> String {
        docs::split_frontmatter(text).1.to_owned()
    }

    /// A project tree whose general index expands `expanded`, with `files`.
    fn tree(expanded: &[&str], files: &[(&str, &str)]) -> Committed {
        let index = format!(
            "expanded:\n{}",
            expanded
                .iter()
                .map(|id| format!("  - {id}\n"))
                .collect::<String>()
        );
        let mut all: Vec<(&str, &str)> = vec![
            (CONFIG_FILE_NAME, LAYER),
            ("docs/general/index.yaml", index.as_str()),
        ];
        all.extend_from_slice(files);
        Committed::of(&all)
    }

    fn inspected(view: &Committed) -> UnitsReport {
        inspect(view, Lens::Current).unwrap()
    }

    fn sources(view: &Committed) -> Vec<Source> {
        repository_sources(view, Lens::Current).unwrap().sources
    }

    #[test]
    fn a_current_unit_stands_in_for_its_source() {
        let text = policy("Alpha", "Do alpha.");
        let digest = source_digest(&prose(&text));
        let compressed = unit(&digest, "# general/policy/alpha\n- do alpha; reason\n");
        let view = tree(
            &["general/policy/alpha"],
            &[
                ("docs/general/policy/alpha.md", &text),
                ("docs/_compressed/general/policy/alpha.md", &compressed),
            ],
        );
        let source = &sources(&view)[0];
        let Decision::Unit(held) = RepositoryUnits::of(&view).decide(source) else {
            panic!("the current unit is not held");
        };
        assert_eq!(held.path, "docs/_compressed/general/policy/alpha.md");
        assert_eq!(inspected(&view), UnitsReport::default());
    }

    #[test]
    fn a_unit_declaring_another_digest_is_stale() {
        let text = policy("Alpha", "Do alpha.");
        let compressed = unit(&source_digest("older prose"), "- do alpha\n");
        let view = tree(
            &["general/policy/alpha"],
            &[
                ("docs/general/policy/alpha.md", &text),
                ("docs/_compressed/general/policy/alpha.md", &compressed),
            ],
        );
        let report = inspected(&view);
        assert_eq!(
            report.stale,
            [UnitWork {
                source: "general/policy/alpha".to_owned(),
                file: "docs/general/policy/alpha.md".to_owned(),
                unit: "docs/_compressed/general/policy/alpha.md".to_owned(),
                source_sha256: source_digest(&prose(&text)),
            }]
        );
        assert_eq!(report.outcome(), UnitsOutcome::Work);
    }

    #[test]
    fn an_inlined_source_without_a_unit_is_missing() {
        let text = policy("Alpha", "Do alpha.");
        let view = tree(
            &["general/policy/alpha"],
            &[("docs/general/policy/alpha.md", &text)],
        );
        let report = inspected(&view);
        let missing: Vec<&str> = report.missing.iter().map(|w| w.source.as_str()).collect();
        assert_eq!(missing, ["general/policy/alpha"]);
    }

    #[test]
    fn a_unit_whose_source_is_gone_or_not_inlined_is_orphaned() {
        let text = policy("Beta", "Do beta.");
        let compressed = unit(&source_digest(&prose(&text)), "- do beta\n");
        let view = tree(
            &[],
            &[
                ("docs/general/policy/beta.md", &text),
                ("docs/_compressed/general/policy/beta.md", &compressed),
                ("docs/_compressed/general/policy/gone.md", &compressed),
            ],
        );
        assert_eq!(
            inspected(&view).orphaned,
            [
                "docs/_compressed/general/policy/beta.md",
                "docs/_compressed/general/policy/gone.md",
            ]
        );
    }

    #[test]
    fn a_domain_snippets_unit_is_orphaned_only_once_the_snippet_is_gone() {
        let text = policy("Gamma", "Do gamma.");
        let compressed = unit(&source_digest(&prose(&text)), "- do gamma\n");
        let view = tree(
            &[],
            &[
                ("docs/domain/sdlc/policy/gamma.md", &text),
                ("docs/_compressed/domain/sdlc/policy/gamma.md", &compressed),
                ("docs/_compressed/domain/sdlc/policy/gone.md", &compressed),
            ],
        );
        assert_eq!(
            inspected(&view).orphaned,
            ["docs/_compressed/domain/sdlc/policy/gone.md"]
        );
    }

    #[test]
    fn a_source_with_crlf_line_endings_digests_as_its_line_feed_form() {
        assert_eq!(
            source_digest("# A\r\n\r\nrule\r\n"),
            source_digest("# A\n\nrule\n")
        );
    }

    #[test]
    fn a_target_lens_digests_the_source_as_the_todo_delivers_it() {
        let text = policy(
            "Alpha",
            "<!-- planned:demo-todo -->\nAdded rule.\n<!-- /planned:demo-todo -->\n\
             <!-- planned-change:demo-todo -->\nOld rule.\n<!-- change:demo-todo -->\nNew rule.\n\
             <!-- /planned-change:demo-todo -->",
        );
        let delivered = policy("Alpha", "Added rule.\nNew rule.");
        let current = policy("Alpha", "Old rule.");
        let view = tree(
            &["general/policy/alpha"],
            &[("docs/general/policy/alpha.md", &text)],
        );
        let slug = TodoSlug::parse("demo-todo").unwrap();
        let target = repository_sources(&view, Lens::Target(&slug))
            .unwrap()
            .sources;
        assert_eq!(target[0].digest, source_digest(&prose(&delivered)));
        assert_eq!(sources(&view)[0].digest, source_digest(&prose(&current)));
    }

    /// The finding kinds `compressed` gives as the unit of a source whose
    /// rules are `rules`.
    fn findings(rules: &str, compressed_body: &str) -> Vec<UnitFindingKind> {
        let text = policy("Alpha", rules);
        let compressed = unit(&source_digest(&prose(&text)), compressed_body);
        let view = tree(
            &["general/policy/alpha"],
            &[
                ("docs/general/policy/alpha.md", &text),
                ("docs/_compressed/general/policy/alpha.md", &compressed),
            ],
        );
        inspected(&view)
            .findings
            .into_iter()
            .map(|finding| finding.kind)
            .collect()
    }

    /// The finding kind a unit file holding `bytes` gives.
    fn unreadable(bytes: &[u8]) -> bool {
        matches!(
            Unit::parse("u.md", bytes),
            Err(UnitFinding {
                kind: UnitFindingKind::Unreadable { .. },
                ..
            })
        )
    }

    #[test]
    fn a_unit_whose_frontmatter_carries_another_key_is_unreadable() {
        let digest = source_digest("x");
        assert!(unreadable(
            format!("---\nsource_sha256: {digest}\nmodel: m\nnote: x\n---\nbody\n").as_bytes()
        ));
    }

    #[test]
    fn a_unit_declaring_an_uppercase_digest_is_unreadable() {
        let digest = source_digest("x").to_uppercase();
        assert!(unreadable(
            format!("---\nsource_sha256: {digest}\nmodel: m\n---\nbody\n").as_bytes()
        ));
    }

    #[test]
    fn a_unit_declaring_an_empty_model_is_unreadable() {
        let digest = source_digest("x");
        assert!(unreadable(
            format!("---\nsource_sha256: {digest}\nmodel: \"\"\n---\nbody\n").as_bytes()
        ));
    }

    #[test]
    fn a_unit_that_is_not_utf8_is_unreadable() {
        let mut bytes = unit(&source_digest("x"), "- a rule\n").into_bytes();
        assert!(Unit::parse("u.md", &bytes).is_ok());
        bytes.insert(bytes.len() - 1, 0xff);
        assert!(unreadable(&bytes));
    }

    #[test]
    fn a_unit_carrying_an_exec_directive_is_a_finding() {
        assert_eq!(
            findings("Do alpha.", "<!-- @exec: rm -rf / -->\n"),
            [UnitFindingKind::ForbiddenDirective {
                directive: "@exec: rm -rf /".to_owned()
            }]
        );
    }

    #[test]
    fn a_unit_reading_a_configured_value_its_source_does_not_is_a_finding() {
        assert_eq!(
            findings("Do alpha.", "<!-- @config: secrets.token -->\n"),
            [UnitFindingKind::ConfiguredValues {
                unit: vec!["secrets.token".to_owned()],
                source: Vec::new(),
            }]
        );
    }

    #[test]
    fn a_unit_reading_a_configured_value_fewer_times_than_its_source_is_a_finding() {
        assert_eq!(
            findings(
                "<!-- @config: runner.name -->\n\n<!-- @config: runner.name -->",
                "<!-- @config: runner.name -->\n"
            ),
            [UnitFindingKind::ConfiguredValues {
                unit: vec!["runner.name".to_owned()],
                source: vec!["runner.name".to_owned(), "runner.name".to_owned()],
            }]
        );
    }

    #[test]
    fn a_unit_with_its_sources_configured_values_has_no_finding() {
        assert!(
            findings(
                "<!-- @config: runner.name -->",
                "- name: <!-- @config: runner.name -->\n"
            )
            .is_empty()
        );
    }

    #[test]
    fn a_unit_requiring_a_read_its_source_does_not_is_a_finding() {
        assert_eq!(
            findings("Do alpha.", "## Required reads\n\n- general/policy/other\n"),
            [UnitFindingKind::Reference {
                reference: "general/policy/other".to_owned()
            }]
        );
    }

    #[test]
    fn a_unit_carrying_an_at_reference_its_source_does_not_is_a_finding() {
        assert_eq!(
            findings("Do alpha.", "@docs/general/policy/other.md\n"),
            [UnitFindingKind::Reference {
                reference: "@docs/general/policy/other.md".to_owned()
            }]
        );
    }

    /// `body` checked as the unit of a source whose rules are `rules`, and
    /// rendered by the unit-body resolver in a committed rendering.
    fn smuggled(
        rules: &str,
        body: &str,
    ) -> (
        Vec<UnitFindingKind>,
        Result<docs::Rendered, docs::DocsError>,
    ) {
        let values = docs::Values::committed();
        let cx = docs::RenderContext {
            home: None,
            values: &values,
            project: None,
        };
        (findings(rules, body), docs::render_unit_body(body, &cx))
    }

    #[test]
    fn a_fence_opened_on_the_first_line_hides_a_directive_from_checks_and_resolver_alike() {
        let (found, rendered) = smuggled(
            "Do alpha.",
            "```\n---\n<!-- @config: secrets.token -->\n---\n```\n",
        );
        assert!(found.is_empty());
        insta::assert_snapshot!(rendered.unwrap().text);
    }

    #[test]
    fn an_exec_fenced_after_a_pending_marker_pair_stays_inert_and_counts_as_fenced() {
        let (found, rendered) = smuggled(
            "Do alpha.",
            "<!-- planned:demo-todo -->\n<!-- /planned:demo-todo -->\n```\n<!-- @exec: rm -rf / -->\n```\n",
        );
        assert!(found.is_empty());
        insta::assert_snapshot!(rendered.unwrap().text);
    }

    #[test]
    fn a_configured_value_between_pending_markers_is_counted_and_resolved() {
        let (found, rendered) = smuggled(
            "<!-- @config: runner.name -->",
            "<!-- planned:demo-todo -->\n<!-- @config: runner.name -->\n<!-- /planned:demo-todo -->\n",
        );
        assert!(found.is_empty());
        assert!(matches!(
            rendered,
            Err(docs::DocsError::ConfigUnresolved { path, .. }) if path == "runner.name"
        ));
    }

    #[test]
    fn a_link_inside_the_place_for_units_is_a_finding() {
        let text = policy("Alpha", "Do alpha.");
        let view = tree(
            &["general/policy/alpha"],
            &[("docs/general/policy/alpha.md", &text)],
        )
        .with_link("docs/_compressed/general");
        let report = inspected(&view);
        assert_eq!(
            report.findings,
            [UnitFinding {
                path: "docs/_compressed/general".to_owned(),
                kind: UnitFindingKind::Link,
            }]
        );
        assert_eq!(report.outcome(), UnitsOutcome::Findings);
    }

    #[test]
    fn an_entry_named_compressed_in_any_position_is_a_finding() {
        let text = policy("Alpha", "Do alpha.");
        let view = tree(
            &[],
            &[
                ("docs/general/_compressed", &text),
                ("docs/project/spec/_compressed/a.md", &text),
            ],
        )
        .with_link("docs/domain/sdlc/_compressed");
        let paths: Vec<String> = reserved_folders(&view)
            .into_iter()
            .map(|finding| finding.path)
            .collect();
        assert_eq!(
            paths,
            [
                "docs/domain/sdlc/_compressed",
                "docs/general/_compressed",
                "docs/project/spec/_compressed",
            ]
        );
    }

    #[test]
    fn a_position_holding_a_compressed_folder_is_a_finding() {
        let text = policy("Alpha", "Do alpha.");
        let view = tree(
            &[],
            &[("docs/domain/sdlc/policy/_compressed/alpha.md", &text)],
        );
        assert_eq!(
            reserved_folders(&view),
            [UnitFinding {
                path: "docs/domain/sdlc/policy/_compressed".to_owned(),
                kind: UnitFindingKind::ReservedFolder,
            }]
        );
    }
}
