//! Structural validation of a repository's documentation
//! (`local/projects/instrukt/spec/feature/docs/snippet-validation`).
//!
//! Validation judges shape, placement and form only, never prose. It reads
//! the corpus, the home for domains the repository does not publish, and
//! — when the `todos` module is on — todo existence and flavor roles
//! through the interfaces core declares.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;

use super::_corpus::{
    BaselineManifest, Corpus, ManifestLine, Stray, Version, relative, required_reads_section,
    unplaced_code,
};
use super::_finding::{Finding, FindingCode};
use super::_frontmatter;
use super::_identity::{
    BaselineOwner, Classification, Placement, Scope, Taxonomy, companion_shape, place,
};
use super::_markers::Problem;
use super::_refs::RefTarget;
use super::_text::{self, compile};
use super::DocsError;
use crate::home::Home;
use crate::ids::{DomainName, RoleName, SnippetId, TodoSlug};
use crate::interfaces::CoreWiring;

/// What one validation run covers.
#[derive(Debug, Clone, Copy)]
pub struct ValidationRequest<'a> {
    /// The root of the working tree to validate.
    pub repo_root: &'a Path,
    pub home: &'a Home,
    /// Files or directories to limit the run to; empty for the whole
    /// `docs/`. Absolute, or relative to `repo_root`.
    pub paths: &'a [PathBuf],
    /// The optional modules wired for this invocation.
    pub wiring: &'a CoreWiring,
}

/// Validates the documentation of `request.repo_root`'s working tree and
/// returns every finding, ordered by file. Changes nothing.
pub fn validate(request: &ValidationRequest<'_>) -> Result<Vec<Finding>, DocsError> {
    let scope = scope_of(request.repo_root, request.paths)?;
    let corpus = Corpus::load(request.repo_root, request.home)?;
    let mut findings = check(&corpus, request.repo_root, request.wiring)?;
    if !scope.is_empty() {
        findings.retain(|finding| {
            std::iter::once(&finding.path)
                .chain(&finding.involves)
                .any(|path| scope.iter().any(|s| path.starts_with(s)))
        });
    }
    findings.sort_by(|a, b| (&a.path, a.code, &a.value).cmp(&(&b.path, b.code, &b.value)));
    findings.dedup();
    Ok(findings)
}

/// The scope paths, repository-relative. A path that does not exist is an
/// error.
fn scope_of(repo_root: &Path, paths: &[PathBuf]) -> Result<Vec<PathBuf>, DocsError> {
    paths
        .iter()
        .map(|path| {
            let absolute = if path.is_absolute() {
                path.clone()
            } else {
                repo_root.join(path)
            };
            if !absolute.exists() {
                return Err(DocsError::NoSuchPath { path: path.clone() });
            }
            let canonical = absolute.canonicalize().unwrap_or(absolute);
            let root = repo_root
                .canonicalize()
                .unwrap_or_else(|_| repo_root.to_path_buf());
            Ok(PathBuf::from(relative(&root, &canonical)))
        })
        .collect()
}

/// Every finding of `corpus`, unscoped and unordered.
pub(super) fn check(
    corpus: &Corpus,
    repo_root: &Path,
    wiring: &CoreWiring,
) -> Result<Vec<Finding>, DocsError> {
    let mut out = Vec::new();
    for stray in corpus.strays() {
        match stray {
            Stray::Unplaced { path, why } => {
                let (code, value) = unplaced_code(why);
                out.push(Finding::new(path, code, value));
            }
            Stray::Companion {
                class,
                slug,
                version,
                path,
            } => {
                out.push(Finding::new(
                    path,
                    FindingCode::PendingSlugInvalid,
                    Some(slug.clone()),
                ));
                if let Some(version) = version {
                    check_version(corpus, class, version, &mut out);
                }
            }
        }
    }
    for snippet in corpus.snippets() {
        check_version(corpus, snippet.class(), snippet.current(), &mut out);
        for companion in snippet.companions() {
            if let Some(next) = companion.next() {
                check_version(corpus, snippet.class(), next, &mut out);
            }
        }
    }
    for doc in corpus.third_party() {
        check_third_party(doc.version(), doc.sources(), &mut out);
    }
    let mut reads = RequiredReads::new(corpus);
    for manifest in corpus.baselines() {
        check_baseline(corpus, manifest, &mut reads, &mut out);
    }
    check_procedures(corpus, repo_root, wiring, &mut out)?;
    check_slug_existence(corpus, wiring, &mut out)?;
    Ok(out)
}

fn push(out: &mut Vec<Finding>, path: &Path, code: FindingCode, value: Option<String>) {
    out.push(Finding::new(path, code, value));
}

fn check_version(
    corpus: &Corpus,
    class: &Classification,
    version: &Version,
    out: &mut Vec<Finding>,
) {
    let path = version.path();
    if version.frontmatter_valid() {
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy())
            .unwrap_or_default();
        for (code, value) in
            _frontmatter::check(&version.frontmatter().raw, Some(class.taxonomy), &stem)
        {
            push(out, path, code, value);
        }
    } else {
        push(out, path, FindingCode::FrontmatterInvalid, None);
    }
    check_structure(class.taxonomy, version, out);
    check_references(corpus, class, version, out);
    for problem in &version.markers().problems {
        let (code, value) = match problem {
            Problem::Inline(m) => (FindingCode::MarkerInline, m),
            Problem::Nested(m) => (FindingCode::MarkerNested, m),
            Problem::Unbalanced(m) => (FindingCode::MarkerUnbalanced, m),
            Problem::Overlapping(m) => (FindingCode::MarkerOverlapping, m),
            Problem::InvalidSlug(s) => (FindingCode::PendingSlugInvalid, s),
        };
        push(out, path, code, Some(value.clone()));
    }
}

fn check_structure(taxonomy: Taxonomy, version: &Version, out: &mut Vec<Finding>) {
    let path = version.path();
    let lines = _text::lines(version.body());
    let headings = _text::headings(&lines);
    let first = lines.iter().position(|line| !line.text.trim().is_empty());
    let first_h1 =
        first.and_then(|index| headings.iter().find(|h| h.line == index && h.level == 1));
    match first_h1 {
        Some(h1) => {
            let suffix = format!("— {}", taxonomy.title_suffix());
            if !h1.title.ends_with(&suffix) {
                push(
                    out,
                    path,
                    FindingCode::H1TypeMismatch,
                    Some(h1.title.clone()),
                );
            }
        }
        None if headings.iter().any(|h| h.level == 1) => {
            push(out, path, FindingCode::H1NotFirst, None);
        }
        None => push(out, path, FindingCode::H1Missing, None),
    }
    let h2: Vec<String> = headings
        .iter()
        .filter(|h| h.level == 2)
        .map(|h| h.title.clone())
        .collect();
    let normalized: Vec<String> = h2.iter().map(|t| _text::normalize_title(t)).collect();
    for required in taxonomy.required_sections() {
        if !normalized.contains(&_text::normalize_title(required)) {
            push(
                out,
                path,
                FindingCode::SectionMissing,
                Some((*required).to_owned()),
            );
        }
    }
    let allowed: BTreeSet<String> = taxonomy
        .required_sections()
        .iter()
        .chain(taxonomy.optional_sections())
        .chain(&["Required reads", "See also"])
        .map(|s| _text::normalize_title(s))
        .collect();
    for (title, norm) in h2.iter().zip(&normalized) {
        if !allowed.contains(norm) {
            push(out, path, FindingCode::SectionUnknown, Some(title.clone()));
        }
    }
    if normalized.iter().any(|t| t == "see also")
        && normalized.last().is_some_and(|t| t != "see also")
    {
        push(out, path, FindingCode::SeeAlsoNotLast, None);
    }
    for heading in &headings {
        if _text::normalize_title(&heading.title) != "required reads" {
            continue;
        }
        if heading.level != 2 {
            push(
                out,
                path,
                FindingCode::RequiredReadsLevel,
                Some(heading.title.clone()),
            );
        } else if normalized.first().is_some_and(|t| t != "required reads") {
            push(out, path, FindingCode::RequiredReadsNotFirst, None);
        }
    }
}

fn check_references(
    corpus: &Corpus,
    class: &Classification,
    version: &Version,
    out: &mut Vec<Finding>,
) {
    let path = version.path();
    let lines = _text::lines(version.body());
    let section = required_reads_section(&lines);
    for raw in _text::references(&lines) {
        if !section.as_ref().is_some_and(|s| s.contains(&raw.line)) {
            push(
                out,
                path,
                FindingCode::ReferenceOutsideRequiredReads,
                Some(raw.target),
            );
        }
    }
    for reference in version.required_reads() {
        check_target(
            corpus,
            path,
            class.scope == Scope::Global,
            &reference.raw,
            reference.target.as_ref(),
            out,
        );
    }
}

fn check_target(
    corpus: &Corpus,
    path: &Path,
    published: bool,
    raw: &str,
    target: Option<&RefTarget>,
    out: &mut Vec<Finding>,
) {
    match target {
        None => push(out, path, FindingCode::ReferenceForm, Some(raw.to_owned())),
        Some(target) if published && target.is_local() => {
            push(
                out,
                path,
                FindingCode::ReferenceLocalFromPublished,
                Some(raw.to_owned()),
            );
        }
        Some(target) if !corpus.resolves(target) => {
            push(
                out,
                path,
                FindingCode::ReferenceUnresolved,
                Some(raw.to_owned()),
            );
        }
        Some(_) => {}
    }
}

static DOC_SERVICE_ID: LazyLock<Regex> =
    LazyLock::new(|| compile(r"^/[A-Za-z0-9_-]+/[A-Za-z0-9_.-]+$"));
static MARKDOWN_LINK: LazyLock<Regex> = LazyLock::new(|| compile(r"^\[[^\]]+\]\(([^)\s]+)\)$"));

fn check_third_party(version: &Version, sources: Vec<String>, out: &mut Vec<Finding>) {
    let path = version.path();
    if !version.frontmatter_valid() {
        push(out, path, FindingCode::FrontmatterInvalid, None);
    } else if version.frontmatter().description.is_none() {
        push(out, path, FindingCode::DescriptionMissing, None);
    }
    for source in sources {
        let url = MARKDOWN_LINK
            .captures(&source)
            .and_then(|caps| caps.get(1))
            .map_or(source.as_str(), |m| m.as_str());
        let web = url.starts_with("https://") || url.starts_with("http://");
        if !web && !DOC_SERVICE_ID.is_match(&source) {
            push(out, path, FindingCode::SourceInvalid, Some(source.clone()));
        }
    }
}

/// The taxonomy an identity spells.
fn taxonomy_of(id: &SnippetId) -> Option<Taxonomy> {
    let parts: Vec<&str> = id.as_str().split('/').collect();
    let dir = if parts.first() == Some(&"local") {
        parts.get(3)
    } else {
        parts.get(1)
    };
    dir.and_then(|d| Taxonomy::from_dir(d))
}

/// Required reads by identity, across the corpus and — for domains the
/// repository does not publish — the home.
struct RequiredReads<'c> {
    corpus: &'c Corpus,
    cache: HashMap<SnippetId, Vec<SnippetId>>,
}

impl<'c> RequiredReads<'c> {
    fn new(corpus: &'c Corpus) -> Self {
        Self {
            corpus,
            cache: HashMap::new(),
        }
    }

    fn of(&mut self, id: &SnippetId) -> Vec<SnippetId> {
        if let Some(found) = self.cache.get(id) {
            return found.clone();
        }
        let reads = match self.corpus.snippet(id) {
            Some(snippet) => snippet_targets(snippet.current()),
            None => self
                .home_version(id)
                .map(|v| snippet_targets(&v))
                .unwrap_or_default(),
        };
        self.cache.insert(id.clone(), reads.clone());
        reads
    }

    fn home_version(&self, id: &SnippetId) -> Option<Version> {
        if id.as_str().starts_with("local/") {
            return None;
        }
        let file = self.corpus.home().docs_dir().join(format!("{id}.md"));
        let text = std::fs::read_to_string(file).ok()?;
        Some(Version::parse(
            &format!("docs/global/{id}.md"),
            &text,
            taxonomy_of(id),
            self.corpus.project(),
            self.corpus.home().user_home(),
        ))
    }

    /// Everything reachable from `roots` through required reads, the roots
    /// themselves excluded unless reached.
    fn closure(&mut self, roots: &[&SnippetId]) -> BTreeSet<SnippetId> {
        let mut seen = BTreeSet::new();
        let mut stack: Vec<SnippetId> = roots.iter().flat_map(|id| self.of(id)).collect();
        while let Some(id) = stack.pop() {
            if seen.insert(id.clone()) {
                stack.extend(self.of(&id));
            }
        }
        seen
    }
}

fn snippet_targets(version: &Version) -> Vec<SnippetId> {
    version
        .required_reads()
        .iter()
        .filter_map(|r| match &r.target {
            Some(RefTarget::Snippet(id)) => Some(id.clone()),
            _ => None,
        })
        .collect()
}

fn manifest_ids(manifest: &BaselineManifest) -> Vec<(String, SnippetId)> {
    manifest
        .entries()
        .into_iter()
        .filter_map(|r| match &r.target {
            Some(RefTarget::Snippet(id)) => Some((r.raw.clone(), id.clone())),
            _ => None,
        })
        .collect()
}

fn check_baseline(
    corpus: &Corpus,
    manifest: &BaselineManifest,
    reads: &mut RequiredReads<'_>,
    out: &mut Vec<Finding>,
) {
    let path = manifest.path();
    let published = matches!(manifest.owner(), BaselineOwner::Domain(_));
    let mut highest: Option<Taxonomy> = None;
    for line in manifest.lines() {
        match line {
            ManifestLine::Invalid(text) => {
                push(
                    out,
                    path,
                    FindingCode::BaselineLineInvalid,
                    Some(text.clone()),
                );
            }
            ManifestLine::Entry(reference) => {
                check_target(
                    corpus,
                    path,
                    published,
                    &reference.raw,
                    reference.target.as_ref(),
                    out,
                );
                let taxonomy = match &reference.target {
                    Some(RefTarget::Snippet(id)) => taxonomy_of(id),
                    _ => None,
                };
                if let Some(taxonomy) = taxonomy {
                    if highest.is_some_and(|h| taxonomy < h) {
                        push(
                            out,
                            path,
                            FindingCode::BaselineOrder,
                            Some(reference.raw.clone()),
                        );
                    }
                    highest = highest.max(Some(taxonomy));
                }
            }
        }
    }
    check_carried(corpus, manifest, reads, out);
}

/// What a domain's bundle carries is its `baseline.md` compiled: each
/// entry with its transitive required reads. The compiled manifest names a
/// snippet another of its entries already brings in; the progressive
/// manifest names one the compiled manifest carries; any manifest names
/// one twice.
fn check_carried(
    corpus: &Corpus,
    manifest: &BaselineManifest,
    reads: &mut RequiredReads<'_>,
    out: &mut Vec<Finding>,
) {
    let entries = manifest_ids(manifest);
    let compiled = match manifest.owner() {
        BaselineOwner::Domain(_) => !manifest.is_progressive(),
        BaselineOwner::Project => false,
    };
    let mut carried: BTreeSet<SnippetId> = BTreeSet::new();
    if let (BaselineOwner::Domain(domain), true) = (manifest.owner(), manifest.is_progressive()) {
        let base: Vec<SnippetId> = domain_baseline(corpus, domain)
            .map(|m| manifest_ids(m).into_iter().map(|(_, id)| id).collect())
            .unwrap_or_default();
        let roots: Vec<&SnippetId> = base.iter().collect();
        carried = reads.closure(&roots);
        carried.extend(base);
    }
    for (index, (raw, id)) in entries.iter().enumerate() {
        let earlier = entries[..index].iter().any(|(_, other)| other == id);
        let brought_in = compiled && {
            let others: Vec<&SnippetId> = entries
                .iter()
                .filter(|(_, other)| other != id)
                .map(|(_, other)| other)
                .collect();
            reads.closure(&others).contains(id)
        };
        if earlier || brought_in || carried.contains(id) {
            push(
                out,
                manifest.path(),
                FindingCode::BaselineCarried,
                Some(raw.clone()),
            );
        }
    }
}

fn domain_baseline<'c>(corpus: &'c Corpus, domain: &DomainName) -> Option<&'c BaselineManifest> {
    corpus.baselines().iter().find(|m| {
        !m.is_progressive() && matches!(m.owner(), BaselineOwner::Domain(d) if d == domain)
    })
}

/// One state of a procedure: its live file or a non-empty companion.
struct ProcedureState {
    id: SnippetId,
    /// Repository-relative for this repository's files; `None` for a
    /// published file read from the home.
    repo_path: Option<PathBuf>,
    root: bool,
    role: Option<String>,
    required: Vec<SnippetId>,
}

fn state(id: &SnippetId, version: &Version, repo_path: Option<PathBuf>) -> ProcedureState {
    let entry = &version.frontmatter().entry;
    ProcedureState {
        id: id.clone(),
        repo_path,
        root: entry.is_root(),
        role: entry.role.clone(),
        required: snippet_targets(version),
    }
}

fn procedure_states(corpus: &Corpus) -> Vec<ProcedureState> {
    let mut states = Vec::new();
    for snippet in corpus
        .snippets()
        .iter()
        .filter(|s| s.taxonomy() == Taxonomy::Procedure)
    {
        states.push(state(
            snippet.id(),
            snippet.current(),
            Some(snippet.path().to_path_buf()),
        ));
        for companion in snippet.companions() {
            if let Some(next) = companion.next() {
                states.push(state(
                    snippet.id(),
                    next,
                    Some(companion.path().to_path_buf()),
                ));
            }
        }
    }
    states.extend(home_procedure_states(corpus));
    states
}

/// Procedures the home publishes for domains this repository does not
/// publish, companions included.
fn home_procedure_states(corpus: &Corpus) -> Vec<ProcedureState> {
    let docs = corpus.home().docs_dir();
    let Ok(domains) = std::fs::read_dir(&docs) else {
        return Vec::new();
    };
    let mut files: BTreeMap<String, PathBuf> = BTreeMap::new();
    for domain in domains.filter_map(Result::ok) {
        let name = domain.file_name().to_string_lossy().into_owned();
        let Ok(domain_name) = DomainName::parse(&name) else {
            continue;
        };
        if corpus.published_domains().contains(&domain_name) {
            continue;
        }
        let root = domain.path().join("procedure");
        for entry in walkdir::WalkDir::new(&root)
            .into_iter()
            .filter_map(Result::ok)
        {
            if entry.file_type().is_file() {
                let rel = format!("docs/global/{}", relative(&docs, entry.path()));
                files.insert(rel, entry.path().to_path_buf());
            }
        }
    }
    let mut states = Vec::new();
    for (rel, file) in &files {
        let id_rel = match companion_shape(rel) {
            Some((parent, _)) if files.contains_key(&parent) => parent,
            _ => rel.clone(),
        };
        let Some(Placement::Snippet(class)) = place(&id_rel, None) else {
            continue;
        };
        let Ok(text) = std::fs::read_to_string(file) else {
            continue;
        };
        if text.is_empty() {
            continue;
        }
        let version = Version::parse(
            rel,
            &text,
            Some(Taxonomy::Procedure),
            corpus.project(),
            corpus.home().user_home(),
        );
        states.push(state(&class.id, &version, None));
    }
    states
}

fn check_procedures(
    corpus: &Corpus,
    repo_root: &Path,
    wiring: &CoreWiring,
    out: &mut Vec<Finding>,
) -> Result<(), DocsError> {
    let states = procedure_states(corpus);
    let mut by_role: BTreeMap<&str, BTreeSet<&SnippetId>> = BTreeMap::new();
    for s in &states {
        if let Some(role) = &s.role {
            by_role.entry(role.as_str()).or_default().insert(&s.id);
        }
    }
    for (role, ids) in by_role.iter().filter(|(_, ids)| ids.len() > 1) {
        let named = ids
            .iter()
            .map(|id| id.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let paths: Vec<PathBuf> = states
            .iter()
            .filter(|s| s.role.as_deref() == Some(*role))
            .filter_map(|s| s.repo_path.clone())
            .collect();
        for path in &paths {
            let mut finding = Finding::new(
                path,
                FindingCode::RoleShared,
                Some(format!("{role} ({named})")),
            );
            finding.involves = paths.iter().filter(|p| *p != path).cloned().collect();
            out.push(finding);
        }
    }
    let by_id: BTreeMap<&SnippetId, Vec<&ProcedureState>> =
        states.iter().fold(BTreeMap::new(), |mut map, s| {
            map.entry(&s.id).or_insert_with(Vec::new).push(s);
            map
        });
    for (id, group) in &by_id {
        let Some(folder) = id.as_str().strip_suffix("/overview") else {
            continue;
        };
        let required: BTreeSet<&SnippetId> = group
            .iter()
            .filter(|s| s.root)
            .flat_map(|s| s.required.iter())
            .collect();
        for sub in required {
            let in_folder = sub
                .as_str()
                .rsplit_once('/')
                .is_some_and(|(parent, _)| parent == folder);
            if !in_folder || sub == *id {
                continue;
            }
            let sub_states = by_id.get(sub).map(Vec::as_slice).unwrap_or_default();
            if !sub_states.iter().any(|s| !s.root) {
                continue;
            }
            let anchors: Vec<PathBuf> = group
                .iter()
                .filter(|s| s.root)
                .chain(sub_states.iter())
                .filter_map(|s| s.repo_path.clone())
                .collect();
            if let Some((first, rest)) = anchors.split_first() {
                let mut finding = Finding::new(
                    first,
                    FindingCode::SubprocedureRequired,
                    Some(format!("{id} requires {sub}")),
                );
                finding.involves = rest.to_vec();
                out.push(finding);
            }
        }
    }
    if let Some(lookup) = &wiring.flavor_roles {
        let declared: BTreeSet<RoleName> = lookup.declared_roles(repo_root)?.into_iter().collect();
        for s in &states {
            let (Some(role), Some(path)) = (&s.role, &s.repo_path) else {
                continue;
            };
            let known = RoleName::parse(role).is_ok_and(|r| declared.contains(&r));
            if !known {
                push(out, path, FindingCode::RoleUndeclared, Some(role.clone()));
            }
        }
    }
    Ok(())
}

/// Every pending slug of the corpus, with the files naming it.
fn pending_slugs(corpus: &Corpus) -> BTreeMap<TodoSlug, BTreeSet<PathBuf>> {
    let mut out: BTreeMap<TodoSlug, BTreeSet<PathBuf>> = BTreeMap::new();
    let mut add = |slug: &TodoSlug, path: &Path| {
        out.entry(slug.clone())
            .or_default()
            .insert(path.to_path_buf());
    };
    for snippet in corpus.snippets() {
        for slug in snippet.delivered_by() {
            add(slug, snippet.path());
        }
        for slug in snippet.current().planned_slugs() {
            add(slug, snippet.path());
        }
        for companion in snippet.companions() {
            add(companion.slug(), companion.path());
            if let Some(next) = companion.next() {
                for slug in next.planned_slugs() {
                    add(slug, companion.path());
                }
            }
        }
    }
    out
}

fn check_slug_existence(
    corpus: &Corpus,
    wiring: &CoreWiring,
    out: &mut Vec<Finding>,
) -> Result<(), DocsError> {
    let (Some(lookup), Some(project)) = (&wiring.todo_existence, corpus.project()) else {
        return Ok(());
    };
    for (slug, paths) in pending_slugs(corpus) {
        if lookup.todo_exists(project, &slug)? {
            continue;
        }
        for path in paths {
            push(
                out,
                &path,
                FindingCode::PendingSlugUnknown,
                Some(slug.to_string()),
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::Arc;

    use super::*;
    use crate::docs::SourceFile;
    use crate::ids::ProjectSlug;
    use crate::interfaces::{FlavorRoles, InterfaceError, TodoExistence};

    struct Todos(Vec<&'static str>);

    impl TodoExistence for Todos {
        fn todo_exists(&self, _: &ProjectSlug, slug: &TodoSlug) -> Result<bool, InterfaceError> {
            Ok(self.0.contains(&slug.as_str()))
        }
    }

    struct Roles(Vec<&'static str>);

    impl FlavorRoles for Roles {
        fn declared_roles(&self, _: &Path) -> Result<Vec<RoleName>, InterfaceError> {
            Ok(self
                .0
                .iter()
                .filter_map(|r| RoleName::parse(r).ok())
                .collect())
        }
    }

    fn corpus(files: &[(&str, &str)]) -> Corpus {
        Corpus::from_sources(
            ProjectSlug::parse("demo").ok(),
            &Home::at_user_home(PathBuf::from("/nonexistent-home")),
            files.iter().map(|(path, text)| SourceFile {
                path: (*path).to_owned(),
                text: (*text).to_owned(),
            }),
        )
    }

    const PROCEDURE: &str = "---\ndescription: d\nrole: builder\nactivation_trigger: t\n\
                             delivered_by: [known, unknown]\n---\n# P — Procedure\n\n## Goal\n\n## Preconditions\n\n\
                             ## Steps\n\n## Outputs\n\n## Recovery\n";

    fn codes(findings: &[Finding]) -> Vec<(FindingCode, Option<String>)> {
        findings.iter().map(|f| (f.code, f.value.clone())).collect()
    }

    #[test]
    fn with_todos_off_neither_existence_nor_roles_are_asked() {
        let c = corpus(&[("docs/local/procedure/p.md", PROCEDURE)]);
        let found = check(&c, Path::new("/repo"), &CoreWiring::default()).unwrap();
        assert!(found.is_empty(), "{found:?}");
    }

    #[test]
    fn with_todos_on_unknown_slugs_and_undeclared_roles_are_findings() {
        let c = corpus(&[("docs/local/procedure/p.md", PROCEDURE)]);
        let wiring = CoreWiring {
            todo_existence: Some(Arc::new(Todos(vec!["known"]))),
            flavor_roles: Some(Arc::new(Roles(vec!["reviewer"]))),
            ..CoreWiring::default()
        };
        let found = check(&c, Path::new("/repo"), &wiring).unwrap();
        assert_eq!(
            codes(&found),
            [
                (FindingCode::RoleUndeclared, Some("builder".into())),
                (FindingCode::PendingSlugUnknown, Some("unknown".into())),
            ]
        );
        let declared = CoreWiring {
            flavor_roles: Some(Arc::new(Roles(vec!["builder"]))),
            ..CoreWiring::default()
        };
        assert!(check(&c, Path::new("/repo"), &declared).unwrap().is_empty());
    }
}
