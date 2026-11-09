//! Structural validation of a repository's documentation
//! (`project/instrukt/spec/feature/docs/snippet-validation`).
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
    relative, unplaced_code,
};
use super::_finding::{Finding, FindingCode, Notice, NoticeCode};
use super::_frontmatter;
use super::_identity::{
    BaselineOwner, Classification, Placement, Scope, Taxonomy, companion_shape, place,
    published_origin, published_source,
};
use super::_markers::Problem;
use super::_position::{GENERAL, INDEX_FILE, Position, PositionIndex, is_reserved_domain_folder};
use super::_refs::RefTarget;
use super::_schema::{self, EffectiveSchema, Schemas};
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

/// What one validation run reports: every finding, ordered by file, and,
/// apart from them, every notice.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Validation {
    pub findings: Vec<Finding>,
    /// Reports that never fail the run.
    pub notices: Vec<Notice>,
}

/// Validates the documentation of `request.repo_root`'s working tree and
/// returns every finding and notice. Changes nothing.
pub fn validate(request: &ValidationRequest<'_>) -> Result<Validation, DocsError> {
    let started = Instant::now();
    let scope = scope_of(request.repo_root, request.paths)?;
    let corpus = Corpus::load(request.repo_root, request.home)?;
    let schemas = Schemas::load(request.repo_root)?;
    let config = Configuration::load(request.home, Some(request.repo_root))?;
    // The flavor-role lookup is wired exactly when the configuration being
    // validated switches `todos` on: the working tree's for `docs
    // validate`, the committed one's at publication.
    if request.wiring.flavor_roles.is_some() {
        // The workflow comes from the domain `workflow.domain` names: the
        // repository's own authored one, or one published on the computer.
        let domain = config.workflow_domain()?;
        let authored = request
            .repo_root
            .join(Position::of_domain(&domain).dir())
            .is_dir();
        if !authored && !request.home.domain_dir(&domain).is_dir() {
            return Err(DocsError::DomainUnpublished {
                domain,
                required_by: "`workflow.domain`".to_owned(),
            });
        }
    }
    let env = CheckEnv {
        repo_root: request.repo_root,
        wiring: request.wiring,
        schemas: &schemas,
    };
    let (mut findings, mut notices) = check_all(&corpus, &env)?;
    notices.extend(unreferenced(&corpus, request.repo_root)?);
    super::_directives::check(
        &corpus,
        request.repo_root,
        &super::_render::Values::publish(&config, request.wiring.caller_identity.as_deref()),
        &mut findings,
    )?;
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
        notices.retain(|notice| scope.iter().any(|s| notice.path.starts_with(s)));
    }
    notices.sort();
    notices.dedup();
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
        notices = notices.len(),
        by_code = %by_code,
        duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
    );
    Ok(Validation { findings, notices })
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
    /// The working tree's position schemas.
    pub schemas: &'a Schemas,
}

/// Every finding of `corpus`, unscoped and unordered.
#[cfg(test)]
pub(super) fn check(corpus: &Corpus, env: &CheckEnv<'_>) -> Result<Vec<Finding>, DocsError> {
    Ok(check_all(corpus, env)?.0)
}

/// Every finding and notice of `corpus`, unscoped and unordered.
pub(super) fn check_all(
    corpus: &Corpus,
    env: &CheckEnv<'_>,
) -> Result<(Vec<Finding>, Vec<Notice>), DocsError> {
    let mut out = Vec::new();
    let mut notices = Vec::new();
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
    out.extend(env.schemas.findings());
    let mut reads = RequiredReads::new(corpus, env.repo_root);
    for snippet in corpus.snippets() {
        check_implied(snippet.current(), &mut reads, &mut out)?;
        for next in snippet.companions().iter().filter_map(|c| c.next()) {
            check_implied(next, &mut reads, &mut out)?;
        }
    }
    for manifest in corpus.baselines() {
        check_baseline(corpus, manifest, &mut reads, &mut out)?;
    }
    let published = crate::index::GlobalIndex::load(corpus.home())?;
    let mut known = Known {
        reads,
        index: published.as_ref(),
    };
    super::_index_check::check(corpus, env.repo_root, &mut known, &mut out, &mut notices)?;
    let states = procedure_states(corpus)?;
    check_procedures(&states, env, &mut out)?;
    check_workflows(&states, env, &mut out)?;
    check_slug_existence(corpus, env.wiring, &mut out)?;
    Ok((out, notices))
}

/// What the position indexes may name: the snippets validation knows and
/// their closures.
struct Known<'c, 'i> {
    reads: RequiredReads<'c>,
    index: Option<&'i crate::index::GlobalIndex>,
}

impl super::_index_check::Snippets for Known<'_, '_> {
    fn pending(&self, id: &SnippetId) -> Option<bool> {
        super::_index_check::pending_in(self.reads.corpus, self.reads.repo_root, self.index, id)
    }

    fn closure(&mut self, id: &SnippetId) -> Result<BTreeSet<SnippetId>, DocsError> {
        self.reads.closure(&[id])
    }
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
        for (code, value) in _frontmatter::check(
            &version.frontmatter().raw,
            version.frontmatter_text(),
            Some(class.taxonomy),
            &stem,
        ) {
            push(out, path, code, value);
        }
        if class.scope == Scope::Project
            && !version.frontmatter().entry.subscriptions.is_empty()
            && version
                .frontmatter()
                .entry
                .subscription_home
                .is_some_and(|home| home != crate::subscription::SubscriptionHome::Project)
        {
            push(
                out,
                path,
                FindingCode::SubscriptionReachInvalid,
                Some("subscription_home: project required at project position".into()),
            );
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
    let effective = match _schema::folder_of(path) {
        Some((position, folder)) => env.schemas.effective(&position, class.taxonomy, &folder),
        None => Schemas::default().effective(&Position::Project, class.taxonomy, ""),
    };
    check_structure(class.taxonomy, &effective, version, out);
    check_references(corpus, env, class, version, out);
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

fn check_structure(
    taxonomy: Taxonomy,
    effective: &EffectiveSchema,
    version: &Version,
    out: &mut Vec<Finding>,
) {
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
    for required in &effective.required {
        if !normalized.contains(&_text::normalize_title(&required.title)) {
            push(
                out,
                path,
                FindingCode::SectionMissing,
                Some(format!(
                    "{}, required by {}",
                    required.title, required.imposed_by
                )),
            );
        }
    }
    let folder = path
        .parent()
        .map(|p| super::_corpus::relative(Path::new(""), p))
        .unwrap_or_default();
    for title in &h2 {
        if !effective.allows(title) {
            push(
                out,
                path,
                FindingCode::SectionUnknown,
                Some(format!(
                    "{title}, outside the schema of {folder}; see `instrukt docs schema {folder}`"
                )),
            );
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
    env: &CheckEnv<'_>,
    class: &Classification,
    version: &Version,
    out: &mut Vec<Finding>,
) {
    let path = version.path();
    let lines = _text::lines(version.body());
    let links = super::_refs::links(&lines);
    let link_lines: BTreeSet<usize> = links.iter().map(|link| link.line).collect();
    for link in &links {
        if let Some(finding) = reference_link_finding(corpus, env.repo_root, path, link) {
            out.push(finding);
        }
    }
    // Link destinations in reference entries are checked above. Other
    // paths outside code remain reference-by-path findings.
    let mut by_path = BTreeSet::new();
    for raw in _text::references(&lines)
        .into_iter()
        .filter(|raw| !link_lines.contains(&raw.line))
    {
        push(
            out,
            path,
            FindingCode::ReferenceByPath,
            Some(raw.target.clone()),
        );
        by_path.insert(raw.target);
    }
    for token in path_tokens(&lines, &link_lines) {
        push(out, path, FindingCode::ReferenceByPath, Some(token));
    }
    for id in see_also_ids(&lines) {
        if !corpus.resolves(&RefTarget::Snippet(id.clone())) {
            push(
                out,
                path,
                FindingCode::ReferenceUnresolved,
                Some(id.to_string()),
            );
        }
    }
    for reference in version.required_reads() {
        // A path entry is already reported as a path.
        if by_path.contains(&reference.raw) {
            continue;
        }
        if super::_refs::is_snippet_path(&reference.raw) {
            push(
                out,
                path,
                FindingCode::ReferenceByPath,
                Some(reference.raw.clone()),
            );
            continue;
        }
        if let Some(RefTarget::Domain(domain)) = &reference.target {
            check_domain_read(corpus, env, path, domain, out);
            continue;
        }
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

fn reference_link_finding(
    corpus: &Corpus,
    repo_root: &Path,
    path: &Path,
    link: &super::_refs::ReferenceLink,
) -> Option<Finding> {
    let Ok(id) = SnippetId::parse(&link.text) else {
        return Some(Finding::new(
            path,
            FindingCode::ReferenceForm,
            Some(link.text.clone()),
        ));
    };
    let target = Path::new(&link.target);
    let resolved = repo_root
        .join(path.parent().unwrap_or(Path::new("")))
        .join(target)
        .canonicalize();
    let expected = corpus
        .snippet(&id)
        .and_then(|snippet| repo_root.join(snippet.current().path()).canonicalize().ok());
    let valid = !target.is_absolute()
        && !link.target.is_empty()
        && matches!((resolved, expected, repo_root.canonicalize()),
            (Ok(actual), Some(expected), Ok(root))
                if actual.starts_with(&root) && actual == expected);
    (!valid).then(|| {
        Finding::new(
            path,
            FindingCode::ReferenceLinkTarget,
            Some(format!("[{}]({})", link.text, link.target)),
        )
    })
}

/// A bare-domain required read names `general` or a domain of the
/// repository or published on the computer, and that domain has an
/// `index.yaml`, whose `expanded` set it stands for.
fn check_domain_read(
    corpus: &Corpus,
    env: &CheckEnv<'_>,
    path: &Path,
    domain: &DomainName,
    out: &mut Vec<Finding>,
) {
    let known = domain.as_str() == GENERAL
        || super::_index_check::holds(env.repo_root, domain)
        || corpus.published_domains().contains(domain)
        || corpus.home().domain_dir(domain).is_dir();
    if !known {
        push(
            out,
            path,
            FindingCode::ReferenceDomainUnpublished,
            Some(format!(
                "{domain}: domain `{domain}` is not published on this computer; adopt a docs repository that publishes it"
            )),
        );
        return;
    }
    match super::_index_check::domain_index_text(corpus, env.repo_root, domain) {
        Ok(Some(_)) => {}
        Ok(None) | Err(_) => push(
            out,
            path,
            FindingCode::ReferenceDomainWithoutIndex,
            Some(domain.to_string()),
        ),
    }
}

/// The tokens of prose, outside code and comments, that are a path to a
/// snippet file without an `@`: `docs/<position>/…/<x>.md` or
/// `~/.instrukt/docs/…/<x>.md` naming a snippet's place.
fn path_tokens(lines: &[_text::Line<'_>], link_lines: &BTreeSet<usize>) -> Vec<String> {
    let mut out = Vec::new();
    for (_, line) in lines
        .iter()
        .enumerate()
        .filter(|(index, line)| !line.fenced && !link_lines.contains(index))
    {
        let visible = _text::without_comments(&_text::mask_inline_code(line.text));
        for word in visible.split_whitespace() {
            let token = word.trim_matches(|c: char| {
                matches!(
                    c,
                    '(' | ')' | '[' | ']' | '<' | '>' | ',' | ';' | ':' | '"' | '\'' | '*'
                )
            });
            let token = token
                .strip_suffix('.')
                .filter(|t| t.ends_with(".md"))
                .unwrap_or(token);
            if token.starts_with('@') || !token.ends_with(".md") {
                continue;
            }
            let repo_rel = if token.starts_with("docs/") {
                Some(token.to_owned())
            } else {
                token
                    .strip_prefix("~/.instrukt/docs/")
                    .and_then(published_origin)
            };
            if repo_rel.is_some_and(|rel| matches!(place(&rel, None), Some(Placement::Snippet(_))))
            {
                out.push(token.to_owned());
            }
        }
    }
    out
}

/// The ids the `See Also` entries of `lines` open with.
fn see_also_ids(lines: &[_text::Line<'_>]) -> Vec<SnippetId> {
    let headings = _text::headings(lines);
    let Some(start) = headings
        .iter()
        .position(|h| h.level == 2 && _text::normalize_title(&h.title) == "see also")
    else {
        return Vec::new();
    };
    let from = headings[start].line + 1;
    let to = headings
        .iter()
        .skip(start + 1)
        .find(|h| h.level <= 2)
        .map_or(lines.len(), |h| h.line);
    lines[from..to]
        .iter()
        .filter(|line| !line.fenced)
        .filter_map(|line| {
            let text = _text::without_comments(line.text);
            let item = ["- ", "* ", "+ "]
                .iter()
                .find_map(|bullet| text.trim().strip_prefix(bullet))
                .map(str::to_owned)?;
            let first = item.split_whitespace().next()?.trim_matches('`');
            SnippetId::parse(first).ok()
        })
        .collect()
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
        Some(target) if !corpus.resolves(target) => match unpublished_domain(corpus, target) {
            Some(domain) => push(
                out,
                path,
                FindingCode::ReferenceDomainUnpublished,
                Some(format!(
                    "{raw}: domain `{domain}` is not published on this computer; adopt a docs repository that publishes it"
                )),
            ),
            None => push(
                out,
                path,
                FindingCode::ReferenceUnresolved,
                Some(raw.to_owned()),
            ),
        },
        Some(_) => {}
    }
}

/// The domain of a reference into the published home that neither this
/// repository nor any adopted docs repository on the computer publishes.
fn unpublished_domain(corpus: &Corpus, target: &RefTarget) -> Option<DomainName> {
    let domain = match target {
        RefTarget::Bundle(domain) => domain.clone(),
        RefTarget::Snippet(id) if !target.is_local() => {
            DomainName::parse(id.as_str().split('/').next()?).ok()?
        }
        _ => return None,
    };
    let published =
        corpus.published_domains().contains(&domain) || corpus.home().domain_dir(&domain).is_dir();
    (!published).then_some(domain)
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

/// The target of an autolink `<target>` making up the whole entry.
fn autolink_target(source: &str) -> Option<&str> {
    let target = source.strip_prefix('<')?.strip_suffix('>')?;
    let valid =
        !target.is_empty() && !target.contains(|c: char| c == '<' || c == '>' || c.is_whitespace());
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
        let url = link_target(&source)
            .or_else(|| autolink_target(&source))
            .unwrap_or(&source);
        let web = url.starts_with("https://") || url.starts_with("http://");
        if !web && !is_doc_service_id(&source) {
            push(out, path, FindingCode::SourceInvalid, Some(source.clone()));
        }
    }
}

/// The taxonomy an identity spells.
/// A finding for each required read of `version` that the closure of
/// another of its required reads already brings, naming that read.
fn check_implied(
    version: &Version,
    reads: &mut RequiredReads<'_>,
    out: &mut Vec<Finding>,
) -> Result<(), DocsError> {
    let named: Vec<(&str, RefTarget)> = version
        .required_reads()
        .iter()
        .filter_map(|r| match &r.target {
            Some(t @ (RefTarget::Snippet(_) | RefTarget::Bundle(_) | RefTarget::Domain(_))) => {
                Some((r.raw.as_str(), t.clone()))
            }
            _ => None,
        })
        .collect();
    let mut closures: Vec<Option<BTreeSet<SnippetId>>> = vec![None; named.len()];
    for (index, (raw, target)) in named.iter().enumerate() {
        let required = match target {
            RefTarget::Snippet(id) => BTreeSet::from([id.clone()]),
            RefTarget::Domain(_) => reads.reach(vec![target.clone()])?,
            _ => continue,
        };
        if required.is_empty() {
            continue;
        }
        for other in 0..named.len() {
            if other == index || named[other].1 == *target {
                continue;
            }
            if closures[other].is_none() {
                let start = match &named[other].1 {
                    RefTarget::Snippet(of) => reads.of(of)?,
                    bundle => vec![bundle.clone()],
                };
                closures[other] = Some(reads.reach(start)?);
            }
            if closures[other]
                .as_ref()
                .is_some_and(|c| required.is_subset(c))
            {
                push(
                    out,
                    version.path(),
                    FindingCode::RequiredReadImplied,
                    Some(format!("{raw}, implied by {}", named[other].0)),
                );
                break;
            }
        }
    }
    Ok(())
}

/// A notice for each delivered snippet of the repository that no
/// `index.yaml` names and no snippet names as a required read — in the
/// working tree, or published on the computer for a domain the repository
/// does not publish.
fn unreferenced(corpus: &Corpus, repo_root: &Path) -> Result<Vec<Notice>, DocsError> {
    let mut named: BTreeSet<SnippetId> = BTreeSet::new();
    let owned = corpus.published_domains();
    let mut positions: Vec<Position> = owned.iter().map(Position::of_domain).collect();
    positions.push(Position::Project);
    for position in &positions {
        let file = repo_root.join(position.dir()).join(INDEX_FILE);
        if let Some(text) = read_home(&file)? {
            named.extend(index_ids(position, &text));
        }
    }
    for snippet in corpus.snippets() {
        named.extend(snippet_targets(snippet.current()));
        for next in snippet.companions().iter().filter_map(|c| c.next()) {
            named.extend(snippet_targets(next));
        }
    }
    let mut candidates: Vec<&super::Snippet> = corpus
        .snippets()
        .iter()
        .filter(|s| !s.is_pending() && !named.contains(s.id()))
        .collect();
    if !candidates.is_empty() {
        let published = published_references(corpus, owned)?;
        candidates.retain(|s| !published.contains(s.id()));
    }
    Ok(candidates
        .into_iter()
        .map(|s| Notice {
            path: s.path().to_path_buf(),
            code: NoticeCode::UnreferencedSnippet,
            value: Some(s.id().to_string()),
        })
        .collect())
}

/// The ids an `index.yaml` names, through `expanded`, `listed` or
/// `agents`; nothing for one that cannot be parsed.
fn index_ids(position: &Position, text: &str) -> Vec<SnippetId> {
    let Ok(index) = PositionIndex::parse(position, text) else {
        return Vec::new();
    };
    let mut ids: Vec<SnippetId> = index
        .expanded()
        .iter()
        .chain(index.listed())
        .cloned()
        .collect();
    if let PositionIndex::General(general) = &index {
        ids.extend(general.agents.values().flatten().cloned());
    }
    ids
}

/// The ids the computer's published domains other than `owned` name
/// through their `index.yaml` or as a required read of one of their
/// snippets.
fn published_references(
    corpus: &Corpus,
    owned: &BTreeSet<DomainName>,
) -> Result<BTreeSet<SnippetId>, DocsError> {
    let mut named = BTreeSet::new();
    let docs = corpus.home().docs_dir();
    let Ok(entries) = std::fs::read_dir(&docs) else {
        return Ok(named);
    };
    let mut domains: Vec<DomainName> = entries
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|e| DomainName::parse(&e.file_name().to_string_lossy()).ok())
        .filter(|d| !owned.contains(d))
        .collect();
    domains.sort();
    for domain in domains {
        let position = Position::of_domain(&domain);
        let dir = corpus.home().domain_dir(&domain);
        if let Some(text) = read_home(&dir.join(INDEX_FILE))? {
            named.extend(index_ids(&position, &text));
        }
        for entry in walkdir::WalkDir::new(&dir)
            .follow_links(false)
            .sort_by_file_name()
        {
            let entry = entry.map_err(|err| DocsError::Walk {
                path: err.path().unwrap_or(&dir).to_path_buf(),
                message: err.to_string(),
            })?;
            if !entry.file_type().is_file() || entry.path().extension().is_none_or(|e| e != "md") {
                continue;
            }
            let Some(text) = read_home(entry.path())? else {
                continue;
            };
            let home_rel = super::_corpus::relative(&docs, entry.path());
            let Some(source) = published_origin(&home_rel) else {
                continue;
            };
            let version = Version::parse(
                &source,
                &text,
                None,
                corpus.project(),
                corpus.home().user_home(),
            );
            named.extend(snippet_targets(&version));
        }
    }
    Ok(named)
}

fn taxonomy_of(id: &SnippetId) -> Option<Taxonomy> {
    let parts: Vec<&str> = id.as_str().split('/').collect();
    let dir = if parts.first() == Some(&"project") {
        parts.get(2)
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
    /// The repository validated: the domain indexes it holds stand for
    /// its bare-domain reads.
    repo_root: &'c Path,
    cache: HashMap<SnippetId, Vec<RefTarget>>,
}

impl<'c> RequiredReads<'c> {
    fn new(corpus: &'c Corpus, repo_root: &'c Path) -> Self {
        Self {
            corpus,
            repo_root,
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
        if id.is_project() {
            return Ok(None);
        }
        let Some(source) = published_source(id) else {
            return Ok(None);
        };
        let file = self.corpus.home().docs_dir().join(format!("{id}.md"));
        Ok(read_home(&file)?.map(|text| {
            Version::parse(
                &source,
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
        let mut domains = BTreeSet::new();
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
                RefTarget::Domain(domain) => {
                    if domains.insert(domain.clone()) {
                        let expanded = super::_index_check::domain_expanded(
                            self.corpus,
                            self.repo_root,
                            &domain,
                        )?;
                        stack.extend(expanded.into_iter().map(RefTarget::Snippet));
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
        .filter(|t| {
            matches!(
                t,
                RefTarget::Snippet(_) | RefTarget::Bundle(_) | RefTarget::Domain(_)
            )
        })
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
    subscribed: bool,
    explicit_role: bool,
    required: Vec<SnippetId>,
}

fn state(id: &SnippetId, version: &Version, repo_path: Option<PathBuf>) -> ProcedureState {
    let entry = &version.frontmatter().entry;
    ProcedureState {
        id: id.clone(),
        repo_path,
        root: entry.is_root(),
        role: entry.role.clone(),
        subscribed: !entry.subscriptions.is_empty(),
        explicit_role: version.frontmatter().raw.contains_key("role"),
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
            if entry.file_type().is_file()
                && let Some(rel) = published_origin(&relative(&docs, entry.path()))
            {
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
        if let Some(role) = &s.role
            && s.explicit_role
        {
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
        for s in states.iter().filter(|s| !s.subscribed) {
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

/// The `workflow/` directories of the repository's published positions
/// (`docs/general/workflow/`, `docs/domain/<x>/workflow/`), as the domain
/// name they are addressed by and repository-relative path. Symbolic links
/// are not followed.
fn workflow_dirs(repo_root: &Path) -> Result<Vec<(String, PathBuf)>, DocsError> {
    let docs = confined_docs(repo_root)?;
    let real_dir =
        |path: &Path| std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_dir());
    let mut out = Vec::new();
    if real_dir(&docs.join(GENERAL)) && real_dir(&docs.join(GENERAL).join("workflow")) {
        out.push((
            GENERAL.to_owned(),
            PathBuf::from(format!("docs/{GENERAL}/workflow")),
        ));
    }
    let domains = docs.join("domain");
    let entries = match std::fs::read_dir(&domains) {
        Ok(entries) => entries,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(out),
        Err(source) => {
            return Err(DocsError::Read {
                path: domains,
                source,
            });
        }
    };
    for entry in entries {
        let entry = entry.map_err(|source| DocsError::Read {
            path: domains.clone(),
            source,
        })?;
        let domain = entry.file_name().to_string_lossy().into_owned();
        if is_reserved_domain_folder(&domain) {
            continue;
        }
        if real_dir(&entry.path()) && real_dir(&entry.path().join("workflow")) {
            out.push((
                domain.clone(),
                PathBuf::from(format!("docs/domain/{domain}/workflow")),
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
        for s in states.iter().filter(|s| !s.subscribed) {
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
        let _ = agents;
        check(
            corpus,
            &CheckEnv {
                repo_root: root,
                wiring,
                schemas: &Schemas::default(),
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
        let c = corpus(&[("docs/project/procedure/p.md", &procedure(PENDING_BUILDER))]);
        assert!(run(&c, &CoreWiring::default(), &[]).is_empty());
    }

    #[test]
    fn with_todos_on_unknown_slugs_and_undeclared_roles_are_findings() {
        let c = corpus(&[("docs/project/procedure/p.md", &procedure(PENDING_BUILDER))]);
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
            ("docs/project/adr/0001-a.md", &adr),
            (
                "docs/project/adr/0002-b.md",
                &clean.replace("number: 1", "number: 2"),
            ),
            ("docs/project/adr/0002-b.later.md", ""),
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
                Path::new("docs/project/adr/0001-a.md"),
                Path::new("docs/project/adr/0001-a.md"),
                Path::new("docs/project/adr/0002-b.later.md"),
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

    fn source_findings(entry: &str) -> Vec<FindingCode> {
        let doc = format!("---\ndescription: d\n---\n# Tool\n\n## Sources\n\n- {entry}\n");
        codes(&run(
            &corpus(&[("docs/third-party/tool/usage.md", &doc)]),
            &CoreWiring::default(),
            &[],
        ))
    }

    #[test]
    fn a_source_written_as_an_autolink_is_a_web_link() {
        assert!(source_findings("<https://tool.example/docs>").is_empty());
    }

    #[test]
    fn an_autolink_to_something_other_than_the_web_is_refused() {
        assert_eq!(
            source_findings("<mailto:someone@tool.example>"),
            [FindingCode::SourceInvalid]
        );
    }

    #[test]
    fn a_source_that_is_not_a_link_is_refused() {
        assert_eq!(
            source_findings("somewhere on the web"),
            [FindingCode::SourceInvalid]
        );
    }

    #[test]
    fn a_manifest_naming_a_bundle_cannot_also_name_what_it_carries() {
        let concept = "---\ndescription: d\n---\n# C — Concept\n\n## What\n\n## Why\n";
        let c = corpus(&[
            ("docs/domain/acme/concept/a.md", concept),
            (
                "docs/domain/acme/baseline.md",
                "@~/.instrukt/docs/acme/concept/a.md\n",
            ),
            (
                "docs/project/baseline.md",
                "@~/.instrukt/docs/acme/bundle.md\n@~/.instrukt/docs/acme/concept/a.md\n",
            ),
        ]);
        let found = run(&c, &CoreWiring::default(), &[]);
        assert_eq!(codes(&found), [FindingCode::BaselineCarried]);
        assert_eq!(found[0].path, Path::new("docs/project/baseline.md"));
    }

    const CONCEPT: &str = "---\ndescription: d\n---\n# C — Concept\n\n## What\n\n## Why\n";

    fn local_manifest_findings(files: &[(&str, &str)]) -> Vec<(FindingCode, PathBuf)> {
        let mut all = vec![
            ("docs/domain/acme/concept/a.md", CONCEPT),
            (
                "docs/domain/acme/baseline.md",
                "@~/.instrukt/docs/acme/concept/a.md\n",
            ),
            (
                "docs/project/baseline.md",
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
            PathBuf::from("docs/project/baseline.md"),
        )];
        let nested = local_manifest_findings(&[(
            "docs/domain/outer/baseline.md",
            "@~/.instrukt/docs/acme/bundle.md\n",
        )]);
        assert_eq!(nested, carried);
    }

    #[test]
    fn bundles_naming_each_other_end_the_expansion() {
        let c = corpus(&[
            ("docs/domain/acme/concept/a.md", CONCEPT),
            (
                "docs/domain/acme/baseline.md",
                "@~/.instrukt/docs/acme/concept/a.md\n@~/.instrukt/docs/outer/bundle.md\n",
            ),
            (
                "docs/domain/outer/baseline.md",
                "@~/.instrukt/docs/acme/bundle.md\n",
            ),
            (
                "docs/project/baseline.md",
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
                PathBuf::from("docs/domain/acme/baseline.md")
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
        let result = check(
            &c,
            &CheckEnv {
                repo_root: Path::new("/nonexistent-repo"),
                wiring: &CoreWiring::default(),
                schemas: &Schemas::default(),
            },
        );
        assert!(matches!(result, Err(DocsError::Read { path, .. }) if path == file));
    }

    fn repo_with_workflow() -> tempfile::TempDir {
        let repo = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(repo.path().join("docs/domain/acme/workflow")).unwrap();
        std::fs::write(repo.path().join("docs/domain/acme/workflow/work.yaml"), "").unwrap();
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
            Path::new("docs/domain/acme/workflow/actions.yaml")
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
                "docs/domain/acme/procedure/build.md",
                &procedure("role: worker\nactivation_trigger: t\n"),
            ),
            ("docs/domain/acme/procedure/helper.md", &procedure("")),
            (
                "docs/domain/acme/procedure/rogue.md",
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
                    Path::new("docs/domain/acme/workflow/work.yaml")
                ),
                (
                    FindingCode::ActionProcedureWithoutRole,
                    Path::new("docs/domain/acme/workflow/actions.yaml")
                ),
                (
                    FindingCode::ActionProcedureUnresolved,
                    Path::new("docs/domain/acme/workflow/actions.yaml")
                ),
                (
                    FindingCode::ActionTargetNotProcedure,
                    Path::new("docs/domain/acme/workflow/actions.yaml")
                ),
                (
                    FindingCode::RoleUndeclared,
                    Path::new("docs/domain/acme/procedure/rogue.md")
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
        .findings
    }

    #[test]
    fn requires_naming_a_module_that_is_off_is_a_configuration_finding() {
        let found = requires_run("");
        assert_eq!(codes(&found), [FindingCode::RequiresModuleOff]);
        assert_eq!(found[0].path, Path::new(CONFIG_FILE_NAME));
        assert!(requires_run("modules:\n  todos: true\n").is_empty());
    }
}
