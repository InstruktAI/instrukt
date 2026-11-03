//! The parsed documentation of one repository: snippets with their
//! pending material, baseline manifests, fragments and third-party docs.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use super::_finding::FindingCode;
use super::_frontmatter::{Frontmatter, parse_mapping};
use super::_identity::{
    BaselineOwner, Classification, DocDomain, Placement, Scope, Taxonomy, Unplaced,
    companion_shape, place,
};
use super::_markers::{self, Scan, Side};
use super::_refs::{self, RefTarget, Reference};
use super::_text::{self, Directive};
use super::DocsError;
use crate::home::Home;
use crate::ids::{DomainName, ProjectSlug, SnippetId, ThirdPartyId, TodoSlug};

/// One documentation file handed to the model: its path relative to the
/// repository root (`/`-separated, beginning `docs/`) and its text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFile {
    pub path: String,
    pub text: String,
}

/// One version of a snippet: the live file, or a companion's next version.
#[derive(Debug, Clone, PartialEq)]
pub struct Version {
    path: PathBuf,
    frontmatter: Frontmatter,
    /// Whether the frontmatter block, when present, is a YAML mapping.
    frontmatter_valid: bool,
    body: String,
    required_reads: Vec<Reference>,
    directives: Vec<Directive>,
    markers: Scan,
}

impl Version {
    pub(super) fn parse(
        path: &str,
        text: &str,
        taxonomy: Option<Taxonomy>,
        project: Option<&ProjectSlug>,
        user_home: &Path,
    ) -> Self {
        let split = _text::split_frontmatter(text);
        let mapping = parse_mapping(split.frontmatter);
        let frontmatter_valid = mapping.is_some();
        let frontmatter = Frontmatter::read(mapping.unwrap_or_default(), taxonomy);
        let body = split.body.to_owned();
        let lines = _text::lines(&body);
        let section = required_reads_section(&lines);
        let required_reads = _text::references(&lines)
            .into_iter()
            .filter(|r| section.as_ref().is_some_and(|s| s.contains(&r.line)))
            .map(|r| _refs::parse(&r.target, project, user_home))
            .collect();
        let directives = _text::directives(&lines);
        let markers = _markers::scan(&body);
        Self {
            path: PathBuf::from(path),
            frontmatter,
            frontmatter_valid,
            body,
            required_reads,
            directives,
            markers,
        }
    }

    /// The file, relative to the repository root.
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn frontmatter(&self) -> &Frontmatter {
        &self.frontmatter
    }

    /// The text after the frontmatter, as authored (pending markers,
    /// directives and the `Required reads` section included).
    pub fn body(&self) -> &str {
        &self.body
    }

    /// The references of the `## Required reads` section, in order.
    pub fn required_reads(&self) -> &[Reference] {
        &self.required_reads
    }

    /// The `@include` and `@exec` directives of `body()`, with their spans
    /// in it.
    pub fn directives(&self) -> &[Directive] {
        &self.directives
    }

    /// The todo slugs this version's planned blocks name.
    pub fn planned_slugs(&self) -> &BTreeSet<TodoSlug> {
        &self.markers.slugs
    }

    pub(super) fn markers(&self) -> &Scan {
        &self.markers
    }

    pub(super) fn frontmatter_valid(&self) -> bool {
        self.frontmatter_valid
    }

    /// `body()` with its planned blocks resolved for `lens`.
    pub fn lens_body(&self, lens: Lens<'_>) -> String {
        _markers::resolve(&self.body, &self.markers, |block| match lens {
            Lens::Target(slug) if block.slug == slug.as_str() => Side::Delivered,
            _ => Side::Current,
        })
    }
}

/// The line indices of the `## Required reads` section: from its H2 to the
/// next heading of level 1 or 2.
pub(super) fn required_reads_section(lines: &[_text::Line<'_>]) -> Option<std::ops::Range<usize>> {
    let headings = _text::headings(lines);
    let start = headings
        .iter()
        .position(|h| h.level == 2 && _text::normalize_title(&h.title) == "required reads")?;
    let from = headings.get(start).map_or(0, |h| h.line);
    let to = headings
        .iter()
        .skip(start + 1)
        .find(|h| h.level <= 2)
        .map_or(lines.len(), |h| h.line);
    Some(from + 1..to)
}

/// A companion `<stem>.<slug>.md` beside a live snippet.
#[derive(Debug, Clone, PartialEq)]
pub struct Companion {
    slug: TodoSlug,
    path: PathBuf,
    next: Option<Version>,
}

impl Companion {
    /// The todo that delivers it.
    pub fn slug(&self) -> &TodoSlug {
        &self.slug
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The complete next version; `None` for an empty companion, which
    /// retires the snippet.
    pub fn next(&self) -> Option<&Version> {
        self.next.as_ref()
    }
}

/// The view a read takes on pending material.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lens<'a> {
    /// The current contract.
    Current,
    /// The documentation as it will be once this todo delivers.
    Target(&'a TodoSlug),
}

/// What a lens shows of one snippet.
#[derive(Debug, Clone, PartialEq)]
pub enum View<'s> {
    /// A pending snippet outside its lens: the body is withheld.
    Withheld { delivered_by: &'s [TodoSlug] },
    /// The lens's todo retires the snippet (an empty companion).
    Retired,
    /// The version the lens reads, with its body's planned blocks resolved.
    Visible { version: &'s Version, body: String },
}

/// A snippet: its derived classification, its live version and the
/// companions carrying its next versions.
#[derive(Debug, Clone, PartialEq)]
pub struct Snippet {
    class: Classification,
    current: Version,
    companions: Vec<Companion>,
}

impl Snippet {
    pub fn id(&self) -> &SnippetId {
        &self.class.id
    }

    pub fn taxonomy(&self) -> Taxonomy {
        self.class.taxonomy
    }

    pub fn scope(&self) -> Scope {
        self.class.scope
    }

    pub fn domain(&self) -> &DocDomain {
        &self.class.domain
    }

    /// The live file, relative to the repository root.
    pub fn path(&self) -> &Path {
        self.current.path()
    }

    pub fn current(&self) -> &Version {
        &self.current
    }

    pub fn companions(&self) -> &[Companion] {
        &self.companions
    }

    /// The todos that deliver this snippet; empty for a live snippet.
    pub fn delivered_by(&self) -> &[TodoSlug] {
        &self.current.frontmatter.delivered_by
    }

    /// Whether the snippet is pending (carries `delivered_by`).
    pub fn is_pending(&self) -> bool {
        !self.delivered_by().is_empty()
    }

    /// The todos with pending material for this snippet: its companions'
    /// slugs and its planned blocks' slugs. Derived, never authored.
    pub fn superseded_by(&self) -> BTreeSet<TodoSlug> {
        self.companions
            .iter()
            .map(|c| c.slug.clone())
            .chain(self.current.markers.slugs.iter().cloned())
            .collect()
    }

    /// Whether `slug` delivers this snippet or has pending material for it.
    pub fn has_material_for(&self, slug: &TodoSlug) -> bool {
        self.delivered_by().contains(slug)
            || self.companions.iter().any(|c| &c.slug == slug)
            || self.current.markers.names(slug)
    }

    /// What `lens` shows of this snippet: the pending gate, then the
    /// lens's companion, then the planned blocks.
    pub fn view(&self, lens: Lens<'_>) -> View<'_> {
        let reaches = matches!(lens, Lens::Target(slug) if self.has_material_for(slug));
        if self.is_pending() && !reaches {
            return View::Withheld {
                delivered_by: self.delivered_by(),
            };
        }
        if let Lens::Target(slug) = lens
            && let Some(companion) = self.companions.iter().find(|c| &c.slug == slug)
        {
            return match &companion.next {
                None => View::Retired,
                Some(version) => View::Visible {
                    version,
                    body: version.lens_body(lens),
                },
            };
        }
        View::Visible {
            version: &self.current,
            body: self.current.lens_body(lens),
        }
    }

    pub(super) fn class(&self) -> &Classification {
        &self.class
    }
}

/// One line of a baseline manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ManifestLine {
    Entry(Reference),
    Invalid(String),
}

/// A baseline manifest: the snippets every agent of a domain or project
/// reads.
#[derive(Debug, Clone, PartialEq)]
pub struct BaselineManifest {
    owner: BaselineOwner,
    progressive: bool,
    path: PathBuf,
    lines: Vec<ManifestLine>,
}

impl BaselineManifest {
    pub fn owner(&self) -> &BaselineOwner {
        &self.owner
    }

    /// Whether this is the progressive manifest (`baseline-progressive.md`).
    pub fn is_progressive(&self) -> bool {
        self.progressive
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The references the manifest names, in order.
    pub fn entries(&self) -> Vec<&Reference> {
        self.lines
            .iter()
            .filter_map(|line| match line {
                ManifestLine::Entry(reference) => Some(reference),
                ManifestLine::Invalid(_) => None,
            })
            .collect()
    }

    pub(super) fn lines(&self) -> &[ManifestLine] {
        &self.lines
    }
}

/// A curated third-party doc.
#[derive(Debug, Clone, PartialEq)]
pub struct ThirdPartyDoc {
    id: ThirdPartyId,
    version: Version,
}

impl ThirdPartyDoc {
    /// Its identity: `third-party/` and its path below the third-party
    /// root, without `.md`.
    pub fn id(&self) -> &ThirdPartyId {
        &self.id
    }

    pub fn path(&self) -> &Path {
        self.version.path()
    }

    pub fn description(&self) -> Option<&str> {
        self.version.frontmatter.description.as_deref()
    }

    pub fn body(&self) -> &str {
        self.version.body()
    }

    /// The entries of its `## Sources` section.
    pub fn sources(&self) -> Vec<String> {
        let lines = _text::lines(self.version.body());
        let headings = _text::headings(&lines);
        let Some(start) = headings
            .iter()
            .position(|h| _text::normalize_title(&h.title) == "sources")
        else {
            return Vec::new();
        };
        let from = headings.get(start).map_or(0, |h| h.line + 1);
        let to = headings.get(start + 1).map_or(lines.len(), |h| h.line);
        lines
            .get(from..to)
            .unwrap_or_default()
            .iter()
            .filter_map(|line| {
                let item = line.text.trim();
                item.strip_prefix("- ")
                    .or_else(|| item.strip_prefix("* "))
                    .map(|s| s.trim().to_owned())
            })
            .collect()
    }

    pub(super) fn version(&self) -> &Version {
        &self.version
    }
}

/// A file placed where no document lives, or a companion whose slug is
/// malformed; kept for validation.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Stray {
    Unplaced {
        path: PathBuf,
        why: Unplaced,
    },
    /// A companion with a malformed slug: validated as a full snippet of
    /// its parent's classification.
    Companion {
        class: Classification,
        slug: String,
        version: Option<Box<Version>>,
        path: PathBuf,
    },
}

/// The documentation of one repository, parsed.
#[derive(Debug, Clone)]
pub struct Corpus {
    project: Option<ProjectSlug>,
    home: Home,
    snippets: Vec<Snippet>,
    by_id: HashMap<SnippetId, usize>,
    baselines: Vec<BaselineManifest>,
    fragments: BTreeMap<String, String>,
    third_party: Vec<ThirdPartyDoc>,
    owned_domains: BTreeSet<DomainName>,
    paths: BTreeSet<String>,
    strays: Vec<Stray>,
}

impl Corpus {
    /// Parses the working tree of the repository rooted at `repo_root`:
    /// every Markdown file below its `docs/`, skipping directories whose
    /// name starts with `_`. The project slug is the one its
    /// `instrukt.yaml` declares.
    pub fn load(repo_root: &Path, home: &Home) -> Result<Self, DocsError> {
        let project = crate::config::project_slug(repo_root)?;
        let sources = read_tree(repo_root)?;
        Ok(Self::from_sources(project, home, sources))
    }

    /// Builds the model from files already read — the working tree, or a
    /// committed revision.
    pub fn from_sources(
        project: Option<ProjectSlug>,
        home: &Home,
        sources: impl IntoIterator<Item = SourceFile>,
    ) -> Self {
        let files: BTreeMap<String, String> = sources
            .into_iter()
            .filter(|s| place(&s.path, None).is_some())
            .map(|s| (s.path, s.text))
            .collect();
        let mut corpus = Self {
            project,
            home: home.clone(),
            snippets: Vec::new(),
            by_id: HashMap::new(),
            baselines: Vec::new(),
            fragments: BTreeMap::new(),
            third_party: Vec::new(),
            owned_domains: BTreeSet::new(),
            paths: files.keys().cloned().collect(),
            strays: Vec::new(),
        };
        let mut companions: Vec<(String, String, String)> = Vec::new();
        for (path, text) in &files {
            if let Some((parent, slug)) = companion_shape(path)
                && files.contains_key(&parent)
            {
                companions.push((parent, slug, path.clone()));
                continue;
            }
            corpus.add(path, text);
        }
        for (parent, slug, path) in companions {
            let text = files.get(&path).map(String::as_str).unwrap_or_default();
            corpus.attach(&parent, &slug, &path, text);
        }
        corpus.snippets.sort_by(|a, b| a.class.id.cmp(&b.class.id));
        corpus.by_id = corpus
            .snippets
            .iter()
            .enumerate()
            .map(|(index, snippet)| (snippet.class.id.clone(), index))
            .collect();
        corpus
    }

    fn add(&mut self, path: &str, text: &str) {
        let user_home = self.home.user_home().to_path_buf();
        if let Some(domain) = owned_domain(path) {
            self.owned_domains.insert(domain);
        }
        match place(path, self.project.as_ref()) {
            Some(Placement::Snippet(class)) => {
                let current = Version::parse(
                    path,
                    text,
                    Some(class.taxonomy),
                    self.project.as_ref(),
                    &user_home,
                );
                self.snippets.push(Snippet {
                    class,
                    current,
                    companions: Vec::new(),
                });
            }
            Some(Placement::Baseline { owner, progressive }) => {
                let lines = manifest_lines(text, self.project.as_ref(), &user_home);
                self.baselines.push(BaselineManifest {
                    owner,
                    progressive,
                    path: PathBuf::from(path),
                    lines,
                });
            }
            Some(Placement::Fragment(name)) => {
                let body = _text::split_frontmatter(text).body.to_owned();
                self.fragments.insert(name, body);
            }
            Some(Placement::ThirdParty(id)) => {
                let version = Version::parse(path, text, None, self.project.as_ref(), &user_home);
                self.third_party.push(ThirdPartyDoc { id, version });
            }
            Some(Placement::Unplaced(why)) => self.strays.push(Stray::Unplaced {
                path: PathBuf::from(path),
                why,
            }),
            Some(Placement::Workflow) | None => {}
        }
    }

    fn attach(&mut self, parent: &str, slug: &str, path: &str, text: &str) {
        let parent_index = self
            .snippets
            .iter()
            .position(|s| s.current.path == Path::new(parent));
        let Some(parent_index) = parent_index else {
            // The parent is no snippet: the file stands on its own.
            self.add(path, text);
            return;
        };
        let class = self.snippets[parent_index].class.clone();
        let user_home = self.home.user_home().to_path_buf();
        let next = (!text.is_empty()).then(|| {
            Version::parse(
                path,
                text,
                Some(class.taxonomy),
                self.project.as_ref(),
                &user_home,
            )
        });
        match TodoSlug::parse(slug) {
            Ok(slug) => {
                let companions = &mut self.snippets[parent_index].companions;
                companions.push(Companion {
                    slug,
                    path: PathBuf::from(path),
                    next,
                });
                companions.sort_by(|a, b| a.slug.cmp(&b.slug));
            }
            Err(_) => self.strays.push(Stray::Companion {
                class,
                slug: slug.to_owned(),
                version: next.map(Box::new),
                path: PathBuf::from(path),
            }),
        }
    }

    /// The slug the repository declares; `None` when it declares none.
    pub fn project(&self) -> Option<&ProjectSlug> {
        self.project.as_ref()
    }

    /// Every snippet, ordered by identity. Companions are not listed; they
    /// hang off their snippet.
    pub fn snippets(&self) -> &[Snippet] {
        &self.snippets
    }

    pub fn snippet(&self, id: &SnippetId) -> Option<&Snippet> {
        self.by_id
            .get(id)
            .and_then(|&index| self.snippets.get(index))
    }

    pub fn baselines(&self) -> &[BaselineManifest] {
        &self.baselines
    }

    pub fn third_party(&self) -> &[ThirdPartyDoc] {
        &self.third_party
    }

    /// The body of the fragment an `@include` path
    /// (`docs/snippets/<name>.md`) names, from this repository's
    /// `docs/global/snippets/`.
    pub fn fragment(&self, include_path: &str) -> Option<&str> {
        let name = include_path
            .strip_prefix("docs/snippets/")?
            .strip_suffix(".md")?;
        self.fragments.get(name).map(String::as_str)
    }

    /// The domains this repository publishes: those with a directory below
    /// its `docs/global/`.
    pub fn published_domains(&self) -> &BTreeSet<DomainName> {
        &self.owned_domains
    }

    pub(super) fn strays(&self) -> &[Stray] {
        &self.strays
    }

    pub(super) fn home(&self) -> &Home {
        &self.home
    }

    /// Whether a reference target names an existing file: a snippet of
    /// this repository, a local file of it, or — for a domain this
    /// repository does not publish — a file or domain in the home.
    pub fn resolves(&self, target: &RefTarget) -> bool {
        match target {
            RefTarget::Snippet(id) if target.is_local() => self.snippet(id).is_some(),
            RefTarget::Snippet(id) => match self.owner_of(id) {
                Some(true) => self.snippet(id).is_some(),
                _ => self
                    .home
                    .docs_dir()
                    .join(format!("{}.md", id.as_str()))
                    .is_file(),
            },
            RefTarget::Bundle(domain) => {
                self.owned_domains.contains(domain) || self.home.domain_dir(domain).is_dir()
            }
            RefTarget::LocalFile(path) => self.paths.contains(path),
        }
    }

    /// Whether this repository publishes the domain of a published id;
    /// `None` when the id has no domain segment.
    fn owner_of(&self, id: &SnippetId) -> Option<bool> {
        let domain = id.as_str().split('/').next()?;
        let domain = DomainName::parse(domain).ok()?;
        Some(self.owned_domains.contains(&domain))
    }
}

fn owned_domain(path: &str) -> Option<DomainName> {
    let rest = path.strip_prefix("docs/global/")?;
    let (domain, _) = rest.split_once('/')?;
    if domain == "snippets" {
        return None;
    }
    DomainName::parse(domain).ok()
}

/// Every Markdown file below `<repo_root>/docs/`, skipping `_` directories.
pub(super) fn read_tree(repo_root: &Path) -> Result<Vec<SourceFile>, DocsError> {
    let docs = confined_docs(repo_root)?;
    if !docs.is_dir() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    // Symbolic links below `docs/` are not followed: their entries are not
    // files, so nothing outside the repository is read or later rewritten.
    let walker = walkdir::WalkDir::new(&docs)
        .follow_links(false)
        .follow_root_links(false)
        .sort_by_file_name()
        .into_iter()
        .filter_entry(|entry| {
            entry.depth() == 0
                || !entry.file_name().to_string_lossy().starts_with('_')
                || entry.file_type().is_file()
        });
    for entry in walker {
        let entry = entry.map_err(|err| DocsError::Walk {
            path: err.path().unwrap_or(&docs).to_path_buf(),
            message: err.to_string(),
        })?;
        if !entry.file_type().is_file() || entry.path().extension().is_none_or(|e| e != "md") {
            continue;
        }
        let rel = relative(repo_root, entry.path());
        let text = std::fs::read_to_string(entry.path()).map_err(|source| DocsError::Read {
            path: entry.path().to_path_buf(),
            source,
        })?;
        out.push(SourceFile { path: rel, text });
    }
    Ok(out)
}

/// `<repo_root>/docs`, refused when it is a symbolic link: documentation is
/// read and folded only inside the repository.
pub(super) fn confined_docs(repo_root: &Path) -> Result<PathBuf, DocsError> {
    let docs = repo_root.join("docs");
    match std::fs::symlink_metadata(&docs) {
        Ok(meta) if meta.file_type().is_symlink() => {
            Err(DocsError::OutsideRepository { path: docs })
        }
        Ok(_) => Ok(docs),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(docs),
        Err(source) => Err(DocsError::Read { path: docs, source }),
    }
}

/// The lines of a baseline manifest: each non-empty line is one `@`
/// reference, or invalid.
pub(super) fn manifest_lines(
    text: &str,
    project: Option<&ProjectSlug>,
    user_home: &Path,
) -> Vec<ManifestLine> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| match line.strip_prefix('@') {
            Some(raw) if !raw.is_empty() && !raw.contains(char::is_whitespace) => {
                ManifestLine::Entry(_refs::parse(raw, project, user_home))
            }
            _ => ManifestLine::Invalid(line.to_owned()),
        })
        .collect()
}

/// `path` relative to `root`, `/`-separated.
pub(super) fn relative(root: &Path, path: &Path) -> String {
    let rel = path.strip_prefix(root).unwrap_or(path);
    rel.components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

/// The finding codes for a placement that yields no identity.
pub(super) fn unplaced_code(why: &Unplaced) -> (FindingCode, Option<String>) {
    match why {
        Unplaced::UnknownTaxonomy(dir) => (FindingCode::UnknownTaxonomy, Some(dir.clone())),
        Unplaced::MissingDomain(segment) => (FindingCode::MissingDomain, Some(segment.clone())),
        Unplaced::UndeclaredProject => (FindingCode::UndeclaredProject, None),
        Unplaced::Outside => (FindingCode::Unplaced, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home() -> Home {
        Home::at_user_home(PathBuf::from("/nonexistent-home"))
    }

    fn corpus(files: &[(&str, &str)]) -> Corpus {
        Corpus::from_sources(
            ProjectSlug::parse("demo").ok(),
            &home(),
            files.iter().map(|(path, text)| SourceFile {
                path: (*path).to_owned(),
                text: (*text).to_owned(),
            }),
        )
    }

    fn slug(s: &str) -> TodoSlug {
        TodoSlug::parse(s).unwrap()
    }

    const BEFORE: &str = "\n# A — Spec\n\n## What it is\n\nx\n";
    const ADDED: &str = "added\n";
    const AFTER: &str = "\n## Canonical fields\n\ny\n";
    const NEXT: &str = "next\n";

    fn live_body() -> String {
        format!("{BEFORE}<!-- planned:add-it -->\n{ADDED}<!-- /planned:add-it -->\n{AFTER}")
    }

    fn live() -> String {
        format!("---\ndescription: d\n---\n{}", live_body())
    }

    fn with_companions() -> Corpus {
        corpus(&[
            ("docs/local/spec/a.md", &live()),
            (
                "docs/local/spec/a.rework.md",
                &format!("---\ndescription: d\n---\n{NEXT}"),
            ),
            ("docs/local/spec/a.retire-it.md", ""),
        ])
    }

    #[test]
    fn companions_and_planned_blocks_supersede_a_live_snippet() {
        let c = with_companions();
        assert_eq!(c.snippets().len(), 1);
        let expected: BTreeSet<TodoSlug> =
            [slug("add-it"), slug("retire-it"), slug("rework")].into();
        assert_eq!(c.snippets()[0].superseded_by(), expected);
    }

    /// The path of the version a view reads and the body it shows.
    fn shown(view: View<'_>) -> (PathBuf, String) {
        match view {
            View::Visible { version, body } => (version.path().to_path_buf(), body),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn lenses_show_current_target_and_retired_views() {
        let c = with_companions();
        let s = &c.snippets()[0];
        let live = PathBuf::from("docs/local/spec/a.md");
        // The live body's own lines: 0-5 before, 6 open, 7 added, 8 close,
        // 9-12 after.
        let text = live_body();
        let lines: Vec<&str> = text.split_inclusive('\n').collect();
        let pick = |indices: &[usize]| -> String { indices.iter().map(|i| lines[*i]).collect() };
        let before_and_after = [0, 1, 2, 3, 4, 5, 9, 10, 11, 12];
        assert_eq!(
            shown(s.view(Lens::Current)),
            (live.clone(), pick(&before_and_after))
        );
        let add = slug("add-it");
        let with_addition = [0, 1, 2, 3, 4, 5, 7, 9, 10, 11, 12];
        assert_eq!(
            shown(s.view(Lens::Target(&add))),
            (live, pick(&with_addition))
        );
        let rework = slug("rework");
        let (path, _) = shown(s.view(Lens::Target(&rework)));
        assert_eq!(path, PathBuf::from("docs/local/spec/a.rework.md"));
        let retire = slug("retire-it");
        assert_eq!(s.view(Lens::Target(&retire)), View::Retired);
    }

    #[test]
    fn a_pending_snippet_is_withheld_outside_its_lens() {
        let c = corpus(&[(
            "docs/local/spec/new.md",
            "---\ndescription: d\ndelivered_by: [build-it]\n---\n# N — Spec\n",
        )]);
        let s = &c.snippets()[0];
        assert!(matches!(s.view(Lens::Current), View::Withheld { .. }));
        let other = slug("other");
        assert!(matches!(
            s.view(Lens::Target(&other)),
            View::Withheld { .. }
        ));
        let mine = slug("build-it");
        assert!(matches!(s.view(Lens::Target(&mine)), View::Visible { .. }));
    }

    #[test]
    fn required_reads_are_read_from_their_section_only() {
        let c = corpus(&[(
            "docs/local/policy/p.md",
            "---\ndescription: d\n---\n# P — Policy\n\n## Required reads\n\n- @docs/local/policy/q.md\n\
             - @~/.instrukt/docs/sdlc/bundle.md\n\n## Rules\n\n@docs/local/policy/r.md\n",
        )]);
        let targets: Vec<_> = c.snippets()[0]
            .current()
            .required_reads()
            .iter()
            .map(|r| r.target.clone())
            .collect();
        assert_eq!(
            targets,
            [
                Some(RefTarget::Snippet(
                    SnippetId::parse("local/projects/demo/policy/q").unwrap()
                )),
                Some(RefTarget::Bundle(DomainName::parse("sdlc").unwrap())),
            ]
        );
    }

    #[test]
    fn fragments_are_found_by_their_include_path() {
        let c = corpus(&[("docs/global/snippets/step.md", "shared text\n")]);
        assert!(c.fragment("docs/snippets/step.md").is_some());
        assert_eq!(c.fragment("docs/snippets/none.md"), None);
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_docs_directory_is_refused() {
        let outside = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(outside.path().join("local/spec")).unwrap();
        std::fs::write(outside.path().join("local/spec/x.md"), "x\n").unwrap();
        let repo = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), repo.path().join("docs")).unwrap();
        assert!(matches!(
            Corpus::load(repo.path(), &home()),
            Err(DocsError::OutsideRepository { .. })
        ));
    }
}
