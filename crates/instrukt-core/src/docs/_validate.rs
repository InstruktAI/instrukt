//! Structural validation of a repository's documentation
//! (`local/projects/instrukt/spec/feature/docs/snippet-validation`).
//!
//! Validation judges shape, placement and form only, never prose. It reads
//! the corpus, the effective configuration (startable agents, `requires`),
//! the home for domains the repository does not publish, the authored
//! workflow flavors through the flavor schema, and — when the `todos`
//! module is on — todo existence and flavor roles.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::Instant;

use super::_corpus::{
    BaselineManifest, Corpus, ManifestLine, Stray, Version, confined_docs, manifest_lines,
    relative, required_reads_section, unplaced_code,
};
use super::_finding::{Finding, FindingCode};
use super::_frontmatter;
use super::_identity::{
    BaselineOwner, Classification, Placement, Scope, Taxonomy, companion_shape, place,
};
use super::_markers::Problem;
use super::_refs::RefTarget;
use super::_text;
use super::DocsError;
use crate::config::{CONFIG_FILE_NAME, Configuration};
use crate::home::Home;
use crate::ids::{DomainName, SnippetId, TodoSlug};
use crate::interfaces::{CoreWiring, FlavorCheck};

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
    let started = Instant::now();
    let scope = scope_of(request.repo_root, request.paths)?;
    let corpus = Corpus::load(request.repo_root, request.home)?;
    let config = Configuration::load(request.home, Some(request.repo_root))?;
    let agents: BTreeSet<String> = config
        .runtimes()
        .iter()
        .filter_map(|entry| entry.agent.clone())
        .collect();
    let env = CheckEnv {
        repo_root: request.repo_root,
        wiring: request.wiring,
        agents: &agents,
    };
    let mut findings = check(&corpus, &env)?;
    for module in config.requires()? {
        if !config.module_enabled(module)? {
            findings.push(Finding::new(
                CONFIG_FILE_NAME,
                FindingCode::RequiresModuleOff,
                Some(module.key().to_owned()),
            ));
        }
    }
    if !scope.is_empty() {
        findings.retain(|finding| {
            std::iter::once(&finding.path)
                .chain(&finding.involves)
                .any(|path| scope.iter().any(|s| path.starts_with(s)))
        });
    }
    findings.sort_by(|a, b| (&a.path, a.code, &a.value).cmp(&(&b.path, b.code, &b.value)));
    findings.dedup();
    let mut by_code: BTreeMap<&str, usize> = BTreeMap::new();
    for finding in &findings {
        *by_code.entry(finding.code.as_str()).or_default() += 1;
    }
    let by_code = by_code
        .iter()
        .map(|(code, count)| format!("{code}={count}"))
        .collect::<Vec<_>>()
        .join(",");
    tracing::debug!(
        event = "docs_validate_completed",
        repo_root = %request.repo_root.display(),
        scoped = !scope.is_empty(),
        findings = findings.len(),
        by_code = %by_code,
        duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
    );
    Ok(findings)
}

/// The scope paths, repository-relative. A path that does not exist is an
/// error.
fn scope_of(repo_root: &Path, paths: &[PathBuf]) -> Result<Vec<PathBuf>, DocsError> {
    let root = repo_root.canonicalize().map_err(|source| DocsError::Read {
        path: repo_root.to_path_buf(),
        source,
    })?;
    paths
        .iter()
        .map(|path| {
            let absolute = if path.is_absolute() {
                path.clone()
            } else {
                repo_root.join(path)
            };
            let canonical = absolute
                .canonicalize()
                .map_err(|source| match source.kind() {
                    ErrorKind::NotFound => DocsError::NoSuchPath { path: path.clone() },
                    _ => DocsError::Read {
                        path: absolute.clone(),
                        source,
                    },
                })?;
            if !canonical.starts_with(&root) {
                return Err(DocsError::OutsideRepository { path: path.clone() });
            }
            Ok(PathBuf::from(relative(&root, &canonical)))
        })
        .collect()
}

/// What validation reads besides the corpus.
pub(super) struct CheckEnv<'a> {
    pub repo_root: &'a Path,
    pub wiring: &'a CoreWiring,
    /// The agents the configured runtime mapping can start.
    pub agents: &'a BTreeSet<String>,
}

/// Every finding of `corpus`, unscoped and unordered.
pub(super) fn check(corpus: &Corpus, env: &CheckEnv<'_>) -> Result<Vec<Finding>, DocsError> {
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
                push(
                    &mut out,
                    path,
                    FindingCode::PendingSlugInvalid,
                    Some(slug.clone()),
                );
                if class.taxonomy == Taxonomy::Adr {
                    push(
                        &mut out,
                        path,
                        FindingCode::AdrPendingCarrier,
                        Some("companion".into()),
                    );
                }
                if let Some(version) = version {
                    check_version(corpus, env, class, version, &mut out);
                }
            }
        }
    }
    for snippet in corpus.snippets() {
        check_version(corpus, env, snippet.class(), snippet.current(), &mut out);
        for companion in snippet.companions() {
            if snippet.taxonomy() == Taxonomy::Adr {
                push(
                    &mut out,
                    companion.path(),
                    FindingCode::AdrPendingCarrier,
                    Some("companion".into()),
                );
            }
            if let Some(next) = companion.next() {
                check_version(corpus, env, snippet.class(), next, &mut out);
            }
        }
    }
    for doc in corpus.third_party() {
        check_third_party(doc.version(), doc.sources(), &mut out);
    }
    let mut reads = RequiredReads::new(corpus);
    for manifest in corpus.baselines() {
        check_baseline(corpus, manifest, &mut reads, &mut out)?;
    }
    let states = procedure_states(corpus)?;
    check_procedures(&states, env, &mut out)?;
    check_workflows(&states, env, &mut out)?;
    check_slug_existence(corpus, env.wiring, &mut out)?;
    Ok(out)
}

fn push(out: &mut Vec<Finding>, path: &Path, code: FindingCode, value: Option<String>) {
    out.push(Finding::new(path, code, value));
}

fn check_version(
    corpus: &Corpus,
    env: &CheckEnv<'_>,
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
        if let Some(dispatch) = &version.frontmatter().entry.dispatch {
            for agent in dispatch.agents.iter().filter(|a| !env.agents.contains(*a)) {
                push(
                    out,
                    path,
                    FindingCode::DispatchAgentUnknown,
                    Some(agent.clone()),
                );
            }
        }
    } else {
        push(out, path, FindingCode::FrontmatterInvalid, None);
    }
    if class.taxonomy == Taxonomy::Adr {
        if version.frontmatter().raw.contains_key("delivered_by") {
            push(
                out,
                path,
                FindingCode::AdrPendingCarrier,
                Some("delivered_by".into()),
            );
        }
        if !version.markers().slugs.is_empty() || !version.markers().blocks.is_empty() {
            push(
                out,
                path,
                FindingCode::AdrPendingCarrier,
                Some("planned".into()),
            );
        }
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

/// A documentation-service identifier: `/<org>/<project>`.
fn is_doc_service_id(source: &str) -> bool {
    let Some(rest) = source.strip_prefix('/') else {
        return false;
    };
    let Some((org, project)) = rest.split_once('/') else {
        return false;
    };
    !org.is_empty()
        && !project.is_empty()
        && org
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
        && project
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
}

/// The target of a Markdown link `[text](target)` making up the whole entry.
fn link_target(source: &str) -> Option<&str> {
    let rest = source.strip_prefix('[')?;
    let (text, rest) = rest.split_once("](")?;
    let target = rest.strip_suffix(')')?;
    let valid = !text.is_empty()
        && !text.contains(']')
        && !target.is_empty()
        && !target.contains(|c: char| c == ')' || c.is_whitespace());
    valid.then_some(target)
}

fn check_third_party(version: &Version, sources: Vec<String>, out: &mut Vec<Finding>) {
    let path = version.path();
    if !version.frontmatter_valid() {
        push(out, path, FindingCode::FrontmatterInvalid, None);
    } else {
        if version.frontmatter().description.is_none() {
            push(out, path, FindingCode::DescriptionMissing, None);
        }
        for key in version.frontmatter().raw.keys() {
            if key.as_str() != Some("description") {
                let name = key
                    .as_str()
                    .map_or_else(|| format!("{key:?}"), str::to_owned);
                push(
                    out,
                    path,
                    FindingCode::ThirdPartyFieldNotAllowed,
                    Some(name),
                );
            }
        }
    }
    for source in sources {
        let url = link_target(&source).unwrap_or(&source);
        let web = url.starts_with("https://") || url.starts_with("http://");
        if !web && !is_doc_service_id(&source) {
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

/// Reads a home file; `None` when it does not exist, an error for any other
/// failure.
fn read_home(path: &Path) -> Result<Option<String>, DocsError> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(None),
        Err(source) => Err(DocsError::Read {
            path: path.to_path_buf(),
            source,
        }),
    }
}

/// Required reads by identity, across the corpus and — for domains the
/// repository does not publish — the home. A bundle a read names stands for
/// everything that bundle carries.
struct RequiredReads<'c> {
    corpus: &'c Corpus,
    cache: HashMap<SnippetId, Vec<RefTarget>>,
}

impl<'c> RequiredReads<'c> {
    fn new(corpus: &'c Corpus) -> Self {
        Self {
            corpus,
            cache: HashMap::new(),
        }
    }

    /// The snippets and bundles `id`'s required reads name.
    fn of(&mut self, id: &SnippetId) -> Result<Vec<RefTarget>, DocsError> {
        if let Some(found) = self.cache.get(id) {
            return Ok(found.clone());
        }
        let reads = match self.corpus.snippet(id) {
            Some(snippet) => read_targets(snippet.current()),
            None => self
                .home_version(id)?
                .map(|v| read_targets(&v))
                .unwrap_or_default(),
        };
        self.cache.insert(id.clone(), reads.clone());
        Ok(reads)
    }

    fn home_version(&self, id: &SnippetId) -> Result<Option<Version>, DocsError> {
        if id.as_str().starts_with("local/") {
            return Ok(None);
        }
        let file = self.corpus.home().docs_dir().join(format!("{id}.md"));
        Ok(read_home(&file)?.map(|text| {
            Version::parse(
                &format!("docs/global/{id}.md"),
                &text,
                taxonomy_of(id),
                self.corpus.project(),
                self.corpus.home().user_home(),
            )
        }))
    }

    /// The snippets and bundles a domain's `baseline.md` names: this
    /// repository's manifest for a domain it publishes, the home's otherwise.
    fn manifest(&self, domain: &DomainName) -> Result<Vec<RefTarget>, DocsError> {
        let lines = match domain_baseline(self.corpus, domain) {
            Some(manifest) => manifest.lines().to_vec(),
            None => {
                let file = self.corpus.home().domain_dir(domain).join("baseline.md");
                read_home(&file)?
                    .map(|text| {
                        manifest_lines(&text, self.corpus.project(), self.corpus.home().user_home())
                    })
                    .unwrap_or_default()
            }
        };
        Ok(lines
            .into_iter()
            .filter_map(|line| match line {
                ManifestLine::Entry(r) => r.target,
                ManifestLine::Invalid(_) => None,
            })
            .filter(|t| matches!(t, RefTarget::Snippet(_) | RefTarget::Bundle(_)))
            .collect())
    }

    /// Every snippet reachable from `start` — required reads followed
    /// transitively, each bundle expanded to its manifest — each visited
    /// once, so cycles end.
    fn reach(&mut self, start: Vec<RefTarget>) -> Result<BTreeSet<SnippetId>, DocsError> {
        let mut seen = BTreeSet::new();
        let mut bundles = BTreeSet::new();
        let mut stack = start;
        while let Some(target) = stack.pop() {
            match target {
                RefTarget::Snippet(id) => {
                    if seen.insert(id.clone()) {
                        stack.extend(self.of(&id)?);
                    }
                }
                RefTarget::Bundle(domain) => {
                    if bundles.insert(domain.clone()) {
                        stack.extend(self.manifest(&domain)?);
                    }
                }
                RefTarget::LocalFile(_) => {}
            }
        }
        Ok(seen)
    }

    /// Everything reachable from `roots` through required reads, the roots
    /// themselves excluded unless reached.
    fn closure(&mut self, roots: &[&SnippetId]) -> Result<BTreeSet<SnippetId>, DocsError> {
        let mut start = Vec::new();
        for root in roots {
            start.extend(self.of(root)?);
        }
        self.reach(start)
    }

    /// What a domain's bundle carries: its `baseline.md` entries with their
    /// transitive required reads, nested bundles expanded.
    fn bundle(&mut self, domain: &DomainName) -> Result<BTreeSet<SnippetId>, DocsError> {
        self.reach(vec![RefTarget::Bundle(domain.clone())])
    }
}

/// The snippets and bundles a version's required reads name.
fn read_targets(version: &Version) -> Vec<RefTarget> {
    version
        .required_reads()
        .iter()
        .filter_map(|r| r.target.clone())
        .filter(|t| matches!(t, RefTarget::Snippet(_) | RefTarget::Bundle(_)))
        .collect()
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
) -> Result<(), DocsError> {
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
    check_carried(manifest, reads, out)
}

/// What a domain's bundle carries is its `baseline.md` compiled: each
/// entry with its transitive required reads. A finding when the compiled
/// manifest names a snippet another of its entries already brings in, when
/// a progressive manifest names one its domain's bundle carries, when any
/// manifest names one a bundle it also names carries, or names one twice.
fn check_carried(
    manifest: &BaselineManifest,
    reads: &mut RequiredReads<'_>,
    out: &mut Vec<Finding>,
) -> Result<(), DocsError> {
    let entries = manifest_ids(manifest);
    let compiled = match manifest.owner() {
        BaselineOwner::Domain(_) => !manifest.is_progressive(),
        BaselineOwner::Project => false,
    };
    let mut carried: BTreeSet<SnippetId> = BTreeSet::new();
    if let (BaselineOwner::Domain(domain), true) = (manifest.owner(), manifest.is_progressive()) {
        carried.extend(reads.bundle(domain)?);
    }
    for reference in manifest.entries() {
        if let Some(RefTarget::Bundle(domain)) = &reference.target {
            carried.extend(reads.bundle(domain)?);
        }
    }
    for (index, (raw, id)) in entries.iter().enumerate() {
        let earlier = entries[..index].iter().any(|(_, other)| other == id);
        let brought_in = if compiled {
            let others: Vec<&SnippetId> = entries
                .iter()
                .filter(|(_, other)| other != id)
                .map(|(_, other)| other)
                .collect();
            reads.closure(&others)?.contains(id)
        } else {
            false
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
    Ok(())
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

fn procedure_states(corpus: &Corpus) -> Result<Vec<ProcedureState>, DocsError> {
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
    states.extend(home_procedure_states(corpus)?);
    Ok(states)
}

/// Procedures the home publishes for domains this repository does not
/// publish, companions included. A home without published docs, or a
/// domain without procedures, has none; any other read failure is an
/// error, because a dropped procedure could hide a role clash.
fn home_procedure_states(corpus: &Corpus) -> Result<Vec<ProcedureState>, DocsError> {
    let docs = corpus.home().docs_dir();
    let domains = match std::fs::read_dir(&docs) {
        Ok(domains) => domains,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => return Err(DocsError::Read { path: docs, source }),
    };
    let mut files: BTreeMap<String, PathBuf> = BTreeMap::new();
    for domain in domains {
        let domain = domain.map_err(|source| DocsError::Read {
            path: docs.clone(),
            source,
        })?;
        let name = domain.file_name().to_string_lossy().into_owned();
        let Ok(domain_name) = DomainName::parse(&name) else {
            continue;
        };
        // The published tree holds files beside its domains (the index).
        let is_dir = domain
            .file_type()
            .map_err(|source| DocsError::Read {
                path: domain.path(),
                source,
            })?
            .is_dir();
        if !is_dir {
            continue;
        }
        if corpus.published_domains().contains(&domain_name) {
            continue;
        }
        let root = domain.path().join("procedure");
        for entry in walkdir::WalkDir::new(&root) {
            let entry = match entry {
                Ok(entry) => entry,
                Err(err)
                    if err
                        .io_error()
                        .is_some_and(|e| e.kind() == ErrorKind::NotFound) =>
                {
                    continue;
                }
                Err(err) => {
                    return Err(DocsError::Walk {
                        path: err.path().unwrap_or(&root).to_path_buf(),
                        message: err.to_string(),
                    });
                }
            };
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
        let text = std::fs::read_to_string(file).map_err(|source| DocsError::Read {
            path: file.clone(),
            source,
        })?;
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
    Ok(states)
}

fn check_procedures(
    states: &[ProcedureState],
    env: &CheckEnv<'_>,
    out: &mut Vec<Finding>,
) -> Result<(), DocsError> {
    let mut by_role: BTreeMap<&str, BTreeSet<&SnippetId>> = BTreeMap::new();
    for s in states {
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
    let by_id = states_by_id(states);
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
    if let Some(lookup) = &env.wiring.flavor_roles {
        let declared: BTreeSet<String> = lookup
            .declared_roles(env.repo_root)?
            .into_iter()
            .map(|r| r.as_str().to_owned())
            .collect();
        for s in states {
            let (Some(role), Some(path)) = (&s.role, &s.repo_path) else {
                continue;
            };
            if !declared.contains(role) {
                push(out, path, FindingCode::RoleUndeclared, Some(role.clone()));
            }
        }
    }
    Ok(())
}

fn states_by_id(states: &[ProcedureState]) -> BTreeMap<&SnippetId, Vec<&ProcedureState>> {
    states.iter().fold(BTreeMap::new(), |mut map, s| {
        map.entry(&s.id).or_insert_with(Vec::new).push(s);
        map
    })
}

/// The `docs/global/<domain>/workflow/` directories of the repository, as
/// domain and repository-relative path. Symbolic links are not followed.
fn workflow_dirs(repo_root: &Path) -> Result<Vec<(String, PathBuf)>, DocsError> {
    let global = confined_docs(repo_root)?.join("global");
    let entries = match std::fs::read_dir(&global) {
        Ok(entries) => entries,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => {
            return Err(DocsError::Read {
                path: global,
                source,
            });
        }
    };
    let mut out = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| DocsError::Read {
            path: global.clone(),
            source,
        })?;
        let domain = entry.file_name().to_string_lossy().into_owned();
        let dir = entry.path().join("workflow");
        let real_dir =
            |path: &Path| std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_dir());
        if real_dir(&entry.path()) && real_dir(&dir) {
            out.push((
                domain.clone(),
                PathBuf::from(format!("docs/global/{domain}/workflow")),
            ));
        }
    }
    out.sort();
    Ok(out)
}

/// Each authored flavor is checked against the workflow schema; a valid one
/// against the procedures: every flow's `orchestrator` role is declared by a
/// procedure, every action naming a procedure names one declaring a `role`,
/// and every procedure of the domain declares one of the flavor's roles.
fn check_workflows(
    states: &[ProcedureState],
    env: &CheckEnv<'_>,
    out: &mut Vec<Finding>,
) -> Result<(), DocsError> {
    let Some(schema) = &env.wiring.flavor_schema else {
        return Ok(());
    };
    let by_id = states_by_id(states);
    for (domain, rel_dir) in workflow_dirs(env.repo_root)? {
        let declarations = match schema.check(&env.repo_root.join(&rel_dir))? {
            FlavorCheck::Invalid(findings) => {
                for finding in findings {
                    let value = if finding.location.is_empty() {
                        finding.message
                    } else {
                        format!("{}: {}", finding.location, finding.message)
                    };
                    push(
                        out,
                        &rel_dir.join(&finding.file),
                        FindingCode::WorkflowSchemaInvalid,
                        Some(value),
                    );
                }
                continue;
            }
            FlavorCheck::Valid(declarations) => declarations,
        };
        for (flow, role) in &declarations.orchestrators {
            if states
                .iter()
                .any(|s| s.role.as_deref() == Some(role.as_str()))
            {
                continue;
            }
            let flow_file = rel_dir.join(format!("{flow}.yaml"));
            let path = if env.repo_root.join(&flow_file).is_file() {
                flow_file
            } else {
                rel_dir.clone()
            };
            push(
                out,
                &path,
                FindingCode::OrchestratorWithoutProcedure,
                Some(format!("{flow}: {role}")),
            );
        }
        for (action, id) in &declarations.action_procedures {
            let actions_file = rel_dir.join("actions.yaml");
            let value = Some(format!("{action}: {id}"));
            if taxonomy_of(id) != Some(Taxonomy::Procedure) {
                push(
                    out,
                    &actions_file,
                    FindingCode::ActionTargetNotProcedure,
                    value,
                );
                continue;
            }
            let Some(group) = by_id.get(id) else {
                push(
                    out,
                    &actions_file,
                    FindingCode::ActionProcedureUnresolved,
                    value,
                );
                continue;
            };
            if group.iter().any(|s| s.role.is_some()) {
                continue;
            }
            let mut finding =
                Finding::new(actions_file, FindingCode::ActionProcedureWithoutRole, value);
            finding.involves = group.iter().filter_map(|s| s.repo_path.clone()).collect();
            out.push(finding);
        }
        let roles: BTreeSet<&str> = declarations.roles.iter().map(|r| r.as_str()).collect();
        let prefix = format!("{domain}/");
        for s in states {
            let (Some(role), Some(path)) = (&s.role, &s.repo_path) else {
                continue;
            };
            if s.id.as_str().starts_with(&prefix) && !roles.contains(role.as_str()) {
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
    use crate::ids::{ProjectSlug, RoleName};
    use crate::interfaces::{
        FlavorDeclarations, FlavorFinding, FlavorRoles, FlavorSchema, InterfaceError, TodoExistence,
    };

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

    struct Schema(FlavorCheck);

    impl FlavorSchema for Schema {
        fn check(&self, _: &Path) -> Result<FlavorCheck, InterfaceError> {
            Ok(self.0.clone())
        }
    }

    fn corpus_at(home: &Path, files: &[(&str, &str)]) -> Corpus {
        Corpus::from_sources(
            ProjectSlug::parse("demo").ok(),
            &Home::at_user_home(home.to_path_buf()),
            files.iter().map(|(path, text)| SourceFile {
                path: (*path).to_owned(),
                text: (*text).to_owned(),
            }),
        )
    }

    fn corpus(files: &[(&str, &str)]) -> Corpus {
        corpus_at(Path::new("/nonexistent-home"), files)
    }

    fn run(corpus: &Corpus, wiring: &CoreWiring, agents: &[&str]) -> Vec<Finding> {
        run_at(Path::new("/nonexistent-repo"), corpus, wiring, agents)
    }

    fn run_at(root: &Path, corpus: &Corpus, wiring: &CoreWiring, agents: &[&str]) -> Vec<Finding> {
        let agents: BTreeSet<String> = agents.iter().map(|a| (*a).to_owned()).collect();
        check(
            corpus,
            &CheckEnv {
                repo_root: root,
                wiring,
                agents: &agents,
            },
        )
        .unwrap()
    }

    fn codes(findings: &[Finding]) -> Vec<FindingCode> {
        findings.iter().map(|f| f.code).collect()
    }

    const SECTIONS: &str = "## Goal\n\n## Preconditions\n\n## Steps\n\n## Outputs\n\n## Recovery\n";

    fn procedure(front: &str) -> String {
        format!("---\ndescription: d\n{front}---\n# P — Procedure\n\n{SECTIONS}")
    }

    const PENDING_BUILDER: &str =
        "role: builder\nactivation_trigger: t\ndelivered_by: [known, unknown]\n";

    #[test]
    fn with_todos_off_neither_existence_nor_roles_are_asked() {
        let c = corpus(&[("docs/local/procedure/p.md", &procedure(PENDING_BUILDER))]);
        assert!(run(&c, &CoreWiring::default(), &[]).is_empty());
    }

    #[test]
    fn with_todos_on_unknown_slugs_and_undeclared_roles_are_findings() {
        let c = corpus(&[("docs/local/procedure/p.md", &procedure(PENDING_BUILDER))]);
        let wiring = CoreWiring {
            todo_existence: Some(Arc::new(Todos(vec!["known"]))),
            flavor_roles: Some(Arc::new(Roles(vec!["reviewer"]))),
            ..CoreWiring::default()
        };
        assert_eq!(
            codes(&run(&c, &wiring, &[])),
            [FindingCode::RoleUndeclared, FindingCode::PendingSlugUnknown]
        );
        let declared = CoreWiring {
            flavor_roles: Some(Arc::new(Roles(vec!["builder"]))),
            ..CoreWiring::default()
        };
        assert!(run(&c, &declared, &[]).is_empty());
    }

    #[test]
    fn dispatch_agents_must_be_startable() {
        let c = corpus(&[(
            "docs/local/procedure/p.md",
            &procedure(
                "role: r\nactivation_trigger: t\ndispatch: {agent: [claude, ghost], thinking_mode: med}\n",
            ),
        )]);
        let found = run(&c, &CoreWiring::default(), &["claude"]);
        assert_eq!(codes(&found), [FindingCode::DispatchAgentUnknown]);
        assert!(run(&c, &CoreWiring::default(), &["claude", "ghost"]).is_empty());
    }

    const ADR: &str = "---\ndescription: d\ndate: 2026-01-02\nnumber: 1\n{extra}---\n# A — ADR\n\n\
                       ## Context\n\n{body}## Decision\n\n## Consequences\n";

    #[test]
    fn adrs_carry_no_pending_material() {
        let adr = ADR.replace("{extra}", "delivered_by: [later]\n").replace(
            "{body}",
            "<!-- planned:later -->\nx\n<!-- /planned:later -->\n\n",
        );
        let clean = ADR.replace("{extra}", "").replace("{body}", "");
        let c = corpus(&[
            ("docs/local/adr/0001-a.md", &adr),
            (
                "docs/local/adr/0002-b.md",
                &clean.replace("number: 1", "number: 2"),
            ),
            ("docs/local/adr/0002-b.later.md", ""),
        ]);
        let found = run(&c, &CoreWiring::default(), &[]);
        let carriers: Vec<&Path> = found
            .iter()
            .filter(|f| f.code == FindingCode::AdrPendingCarrier)
            .map(|f| f.path.as_path())
            .collect();
        assert_eq!(
            carriers,
            [
                Path::new("docs/local/adr/0001-a.md"),
                Path::new("docs/local/adr/0001-a.md"),
                Path::new("docs/local/adr/0002-b.later.md"),
            ]
        );
    }

    #[test]
    fn third_party_docs_author_only_a_description() {
        let c = corpus(&[(
            "docs/third-party/tool/usage.md",
            "---\ndescription: d\nid: x\nrole: r\ndelivered_by: [a]\n---\n# Tool\n",
        )]);
        let found = run(&c, &CoreWiring::default(), &[]);
        assert_eq!(codes(&found), [FindingCode::ThirdPartyFieldNotAllowed; 3]);
    }

    #[test]
    fn a_manifest_naming_a_bundle_cannot_also_name_what_it_carries() {
        let concept = "---\ndescription: d\n---\n# C — Concept\n\n## What\n\n## Why\n";
        let c = corpus(&[
            ("docs/global/acme/concept/a.md", concept),
            (
                "docs/global/acme/baseline.md",
                "@~/.instrukt/docs/acme/concept/a.md\n",
            ),
            (
                "docs/local/baseline.md",
                "@~/.instrukt/docs/acme/bundle.md\n@~/.instrukt/docs/acme/concept/a.md\n",
            ),
        ]);
        let found = run(&c, &CoreWiring::default(), &[]);
        assert_eq!(codes(&found), [FindingCode::BaselineCarried]);
        assert_eq!(found[0].path, Path::new("docs/local/baseline.md"));
    }

    const CONCEPT: &str = "---\ndescription: d\n---\n# C — Concept\n\n## What\n\n## Why\n";

    fn concept_reading(read: &str) -> String {
        format!(
            "---\ndescription: d\n---\n# C — Concept\n\n## Required reads\n\n- @{read}\n\n## What\n\n## Why\n"
        )
    }

    fn local_manifest_findings(files: &[(&str, &str)]) -> Vec<(FindingCode, PathBuf)> {
        let mut all = vec![
            ("docs/global/acme/concept/a.md", CONCEPT),
            (
                "docs/global/acme/baseline.md",
                "@~/.instrukt/docs/acme/concept/a.md\n",
            ),
            (
                "docs/local/baseline.md",
                "@~/.instrukt/docs/outer/bundle.md\n@~/.instrukt/docs/acme/concept/a.md\n",
            ),
        ];
        all.extend_from_slice(files);
        run(&corpus(&all), &CoreWiring::default(), &[])
            .into_iter()
            .map(|f| (f.code, f.path))
            .collect()
    }

    #[test]
    fn a_bundle_naming_another_bundle_carries_what_that_bundle_carries() {
        let carried = vec![(
            FindingCode::BaselineCarried,
            PathBuf::from("docs/local/baseline.md"),
        )];
        let nested = local_manifest_findings(&[(
            "docs/global/outer/baseline.md",
            "@~/.instrukt/docs/acme/bundle.md\n",
        )]);
        assert_eq!(nested, carried);
        let through_reads = concept_reading("~/.instrukt/docs/acme/bundle.md");
        let via_required_read = local_manifest_findings(&[
            ("docs/global/outer/concept/o.md", &through_reads),
            (
                "docs/global/outer/baseline.md",
                "@~/.instrukt/docs/outer/concept/o.md\n",
            ),
        ]);
        assert_eq!(via_required_read, carried);
    }

    #[test]
    fn bundles_naming_each_other_end_the_expansion() {
        let c = corpus(&[
            ("docs/global/acme/concept/a.md", CONCEPT),
            (
                "docs/global/acme/baseline.md",
                "@~/.instrukt/docs/acme/concept/a.md\n@~/.instrukt/docs/outer/bundle.md\n",
            ),
            (
                "docs/global/outer/baseline.md",
                "@~/.instrukt/docs/acme/bundle.md\n",
            ),
            (
                "docs/local/baseline.md",
                "@~/.instrukt/docs/outer/bundle.md\n",
            ),
        ]);
        // The cycle ends; acme's own entry is carried back to it through
        // outer's bundle, and nothing else is reported.
        let found: Vec<(FindingCode, PathBuf)> = run(&c, &CoreWiring::default(), &[])
            .into_iter()
            .map(|f| (f.code, f.path))
            .collect();
        assert_eq!(
            found,
            [(
                FindingCode::BaselineCarried,
                PathBuf::from("docs/global/acme/baseline.md")
            )]
        );
    }

    #[cfg(unix)]
    #[test]
    fn an_unreadable_published_procedure_fails_the_run() {
        use std::os::unix::fs::PermissionsExt;
        let home = tempfile::tempdir().unwrap();
        let dir = home.path().join(".instrukt/docs/other/procedure");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("p.md");
        std::fs::write(&file, procedure("role: builder\nactivation_trigger: t\n")).unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o000)).unwrap();
        if std::fs::read_to_string(&file).is_ok() {
            return; // running with privileges that ignore file modes
        }
        let c = corpus_at(home.path(), &[]);
        let agents = BTreeSet::new();
        let result = check(
            &c,
            &CheckEnv {
                repo_root: Path::new("/nonexistent-repo"),
                wiring: &CoreWiring::default(),
                agents: &agents,
            },
        );
        assert!(matches!(result, Err(DocsError::Read { path, .. }) if path == file));
    }

    fn repo_with_workflow() -> tempfile::TempDir {
        let repo = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(repo.path().join("docs/global/acme/workflow")).unwrap();
        std::fs::write(repo.path().join("docs/global/acme/workflow/work.yaml"), "").unwrap();
        repo
    }

    fn schema(check: FlavorCheck) -> CoreWiring {
        CoreWiring {
            flavor_schema: Some(Arc::new(Schema(check))),
            ..CoreWiring::default()
        }
    }

    #[test]
    fn workflow_schema_findings_are_reported_in_their_file() {
        let repo = repo_with_workflow();
        let wiring = schema(FlavorCheck::Invalid(vec![FlavorFinding {
            file: PathBuf::from("actions.yaml"),
            location: "roles".into(),
            message: "is empty".into(),
        }]));
        let found = run_at(repo.path(), &corpus(&[]), &wiring, &[]);
        assert_eq!(codes(&found), [FindingCode::WorkflowSchemaInvalid]);
        assert_eq!(
            found[0].path,
            Path::new("docs/global/acme/workflow/actions.yaml")
        );
    }

    #[test]
    fn a_valid_flavor_is_checked_against_the_procedures() {
        let repo = repo_with_workflow();
        let role = |r: &str| RoleName::parse(r).unwrap();
        let id = |i: &str| SnippetId::parse(i).unwrap();
        let wiring = schema(FlavorCheck::Valid(FlavorDeclarations {
            roles: vec![role("worker"), role("lead")],
            orchestrators: vec![("work".into(), role("lead"))],
            action_procedures: vec![
                ("build".into(), id("acme/procedure/build")),
                ("help".into(), id("acme/procedure/helper")),
                ("vanish".into(), id("acme/procedure/missing")),
                ("misnamed".into(), id("acme/concept/idea")),
            ],
        }));
        let c = corpus(&[
            (
                "docs/global/acme/procedure/build.md",
                &procedure("role: worker\nactivation_trigger: t\n"),
            ),
            ("docs/global/acme/procedure/helper.md", &procedure("")),
            (
                "docs/global/acme/procedure/rogue.md",
                &procedure("role: rogue\nactivation_trigger: t\n"),
            ),
        ]);
        let found = run_at(repo.path(), &c, &wiring, &[]);
        let got: Vec<(FindingCode, &Path)> =
            found.iter().map(|f| (f.code, f.path.as_path())).collect();
        assert_eq!(
            got,
            [
                (
                    FindingCode::OrchestratorWithoutProcedure,
                    Path::new("docs/global/acme/workflow/work.yaml")
                ),
                (
                    FindingCode::ActionProcedureWithoutRole,
                    Path::new("docs/global/acme/workflow/actions.yaml")
                ),
                (
                    FindingCode::ActionProcedureUnresolved,
                    Path::new("docs/global/acme/workflow/actions.yaml")
                ),
                (
                    FindingCode::ActionTargetNotProcedure,
                    Path::new("docs/global/acme/workflow/actions.yaml")
                ),
                (
                    FindingCode::RoleUndeclared,
                    Path::new("docs/global/acme/procedure/rogue.md")
                ),
            ]
        );
    }

    fn requires_run(project_modules: &str) -> Vec<Finding> {
        let home = tempfile::tempdir().unwrap();
        let repo = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(home.path().join(".instrukt")).unwrap();
        std::fs::write(
            home.path().join(".instrukt/instrukt.yaml"),
            "schema_version: 1\nmodules:\n  todos: false\n",
        )
        .unwrap();
        std::fs::write(
            repo.path().join(CONFIG_FILE_NAME),
            format!("schema_version: 1\nproject: demo\nrequires: [todos]\n{project_modules}"),
        )
        .unwrap();
        let home = Home::at_user_home(home.path().to_path_buf());
        validate(&ValidationRequest {
            repo_root: repo.path(),
            home: &home,
            paths: &[],
            wiring: &CoreWiring::default(),
        })
        .unwrap()
    }

    #[test]
    fn requires_naming_a_module_that_is_off_is_a_configuration_finding() {
        let found = requires_run("");
        assert_eq!(codes(&found), [FindingCode::RequiresModuleOff]);
        assert_eq!(found[0].path, Path::new(CONFIG_FILE_NAME));
        assert!(requires_run("modules:\n  todos: true\n").is_empty());
    }
}
