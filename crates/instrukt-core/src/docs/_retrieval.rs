//! Progressive retrieval: `docs index` lists what the home holds, `docs get`
//! reads it through the read pipeline
//! (`local/projects/instrukt/spec/feature/docs/progressive-retrieval`).
//!
//! Both read the global index and the publication record; neither writes
//! anything. A published snippet is read from the home, a local snippet
//! from the absolute path its index entry carries, with the companions
//! beside either file.

use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};
use std::time::Instant;

use super::_corpus::{Corpus, Lens, ManifestLine, Snippet, SourceFile, View, manifest_lines};
use super::_identity::{Scope, Taxonomy, companion_shape};
use super::_refs::{RefTarget, Reference};
use super::_render::{self, ExecPolicy, Fragments, RenderContext, read_confined};
use super::DocsError;
use crate::git;
use crate::home::Home;
use crate::ids::{DomainName, ProjectSlug, SnippetId, TodoSlug};
use crate::index::{GlobalIndex, IndexEntry, IndexScope, ThirdPartyEntry};
use crate::publication::PublishedRecord;

/// What `docs index` lists. Every filter only narrows; an empty list is no
/// filter.
#[derive(Debug, Clone, Copy)]
pub struct IndexRequest<'a> {
    pub home: &'a Home,
    /// The caller's working directory: the project it stands in.
    pub cwd: &'a Path,
    pub domains: &'a [DomainName],
    pub areas: &'a [Taxonomy],
    pub baseline_only: bool,
    pub third_party: bool,
    pub target: Option<&'a TodoSlug>,
    pub project: Option<&'a ProjectSlug>,
}

/// What `docs get` reads.
#[derive(Debug, Clone, Copy)]
pub struct GetRequest<'a> {
    pub home: &'a Home,
    /// The caller's working directory: where read-time commands run, and
    /// the project it stands in.
    pub cwd: &'a Path,
    pub ids: &'a [String],
    pub closure: bool,
    pub target: Option<&'a TodoSlug>,
}

/// The text `docs get` prints, and whether a read-time command failed in
/// it (the read then reports failure).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reading {
    pub text: String,
    pub exec_failed: bool,
}

const AREAS: &str = "principle|concept|policy|procedure|design|spec|adr";

/// The `docs index` listing.
pub fn index(req: &IndexRequest<'_>) -> Result<String, DocsError> {
    let started = Instant::now();
    let caller = Caller::of(req.cwd)?;
    let Some(index) = GlobalIndex::load(req.home)? else {
        return Ok(nothing_published());
    };
    let valid: BTreeSet<&str> = index
        .snippets
        .iter()
        .filter(|e| e.scope == IndexScope::Global)
        .map(|e| e.domain.as_str())
        .collect();
    let unknown: Vec<String> = req
        .domains
        .iter()
        .filter(|d| !valid.contains(d.as_str()))
        .map(|d| d.as_str().to_owned())
        .collect();
    if !unknown.is_empty() {
        return Err(DocsError::UnknownDomains {
            unknown,
            valid: valid.iter().map(|d| (*d).to_owned()).collect(),
        });
    }
    let mut out = vec![
        "# instrukt docs index: snippet ids with their descriptions".to_owned(),
        format!(
            "# Filter with --domains <{}>, --areas <{AREAS}>, --baseline-only, \
             --third-party, --target <slug>, --project <slug>",
            valid.iter().copied().collect::<Vec<_>>().join("|")
        ),
    ];
    out.extend(behind_warning(req.home, &caller)?);
    out.push(String::new());

    let local_project = req.project.or(caller.project.as_ref());
    if let Some(project) = req.project
        && !index.snippets.iter().any(|e| &e.source_project == project)
        && PublishedRecord::load(req.home, project)?.is_none()
    {
        out.push(format!("# No project `{project}` has published."));
        return Ok(finish(out));
    }
    let baseline = if req.baseline_only {
        Some(baseline_ids(req, &valid, &caller)?)
    } else {
        None
    };
    let mut entries: Vec<&IndexEntry> = index
        .snippets
        .iter()
        .filter(|e| match e.scope {
            IndexScope::Global => {
                req.project.is_none()
                    && (req.domains.is_empty()
                        || req.domains.iter().any(|d| d.as_str() == e.domain))
            }
            IndexScope::Local => req.domains.is_empty() && local_project == Some(&e.source_project),
        })
        .filter(|e| {
            req.areas.is_empty()
                || Taxonomy::from_dir(&e.taxonomy).is_some_and(|t| req.areas.contains(&t))
        })
        .filter(|e| baseline.as_ref().is_none_or(|ids| ids.contains(&e.id)))
        .filter(|e| {
            req.target
                .is_none_or(|slug| e.delivered_by.contains(slug) || e.superseded_by.contains(slug))
        })
        .collect();
    entries.sort_by(|a, b| entry_order(a).cmp(&entry_order(b)));
    if entries.is_empty() {
        out.push("# No snippets match.".to_owned());
    }
    out.extend(entries.iter().map(|e| entry_line(e)));

    let narrowed = !req.domains.is_empty()
        || !req.areas.is_empty()
        || req.baseline_only
        || req.target.is_some()
        || req.project.is_some();
    if req.third_party && !narrowed {
        let mut third: Vec<&ThirdPartyEntry> = index.third_party.iter().collect();
        third.sort_by(|a, b| a.id.cmp(&b.id));
        out.push(String::new());
        out.push("# Third-party documentation (not part of the taxonomy)".to_owned());
        out.extend(
            third
                .iter()
                .map(|e| format!("{}: {}", e.id.as_str(), e.description)),
        );
    }
    out.push(String::new());
    out.push("# Read the snippets you need: instrukt docs get <id> [<id>…]".to_owned());
    out.push(
        "# Pending documentation is flagged, never hidden: `(delivered by: <slugs>)` marks a \
         snippet not yet delivered, `(superseded by: <slugs>)` a live snippet with pending \
         changes. `instrukt docs get` reads the current contract; `--target <slug>` on `index` \
         and `get` shows the documentation as that todo will deliver it."
            .to_owned(),
    );
    tracing::debug!(
        event = "docs_index_completed",
        listed = entries.len(),
        duration_ms = elapsed_ms(started),
    );
    Ok(finish(out))
}

fn nothing_published() -> String {
    "# Nothing is published yet: run `instrukt sync` in a project to publish its documentation.\n"
        .to_owned()
}

fn finish(lines: Vec<String>) -> String {
    let mut text = lines.join("\n");
    text.push('\n');
    text
}

fn entry_order(e: &IndexEntry) -> (bool, Option<Taxonomy>, u8, &SnippetId) {
    let taxonomy = Taxonomy::from_dir(&e.taxonomy);
    let scope = match e.scope {
        IndexScope::Global => 0,
        IndexScope::Local => 1,
    };
    (taxonomy.is_none(), taxonomy, scope, &e.id)
}

fn entry_line(e: &IndexEntry) -> String {
    let mut line = format!("{}: {}", e.id.as_str(), e.description);
    if !e.delivered_by.is_empty() {
        line.push_str(&format!(" (delivered by: {})", slugs(&e.delivered_by)));
    }
    if !e.superseded_by.is_empty() {
        line.push_str(&format!(" (superseded by: {})", slugs(&e.superseded_by)));
    }
    line
}

fn slugs<'a>(slugs: impl IntoIterator<Item = &'a TodoSlug>) -> String {
    slugs
        .into_iter()
        .map(TodoSlug::as_str)
        .collect::<Vec<_>>()
        .join(", ")
}

/// The ids every baseline manifest in view names: the published domains'
/// `baseline.md` and `baseline-progressive.md`, and the listed project's
/// `docs/local/baseline.md`.
fn baseline_ids(
    req: &IndexRequest<'_>,
    domains: &BTreeSet<&str>,
    caller: &Caller,
) -> Result<BTreeSet<SnippetId>, DocsError> {
    let user_home = req.home.user_home();
    let mut manifests: Vec<(String, Option<ProjectSlug>)> = Vec::new();
    let docs = req.home.docs_dir();
    for domain in domains {
        for name in ["baseline.md", "baseline-progressive.md"] {
            let text = read_confined(&docs, &Path::new(domain).join(name), outside_home)?;
            manifests.extend(text.map(|text| (text, None)));
        }
    }
    let local = match req.project {
        Some(project) => PublishedRecord::load(req.home, project)?
            .map(|record| (record.checkout, project.clone())),
        None => caller.repo_root.clone().zip(caller.project.clone()),
    };
    if let Some((root, project)) = local {
        let path = root.join("docs/local/baseline.md");
        match std::fs::read_to_string(&path) {
            Ok(text) => manifests.push((text, Some(project))),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => return Err(DocsError::Read { path, source }),
        }
    }
    Ok(manifests
        .iter()
        .flat_map(|(text, project)| manifest_lines(text, project.as_ref(), user_home))
        .filter_map(|line| match line {
            ManifestLine::Entry(Reference {
                target: Some(RefTarget::Snippet(id)),
                ..
            }) => Some(id),
            _ => None,
        })
        .collect())
}

/// The `docs get` reading.
pub fn get(req: &GetRequest<'_>) -> Result<Reading, DocsError> {
    let started = Instant::now();
    let caller = Caller::of(req.cwd)?;
    let Some(index) = GlobalIndex::load(req.home)? else {
        return Err(DocsError::UnknownIds {
            ids: req.ids.to_vec(),
            nothing_published: true,
        });
    };
    let mut requested: Vec<&IndexEntry> = Vec::new();
    let mut third_party: Vec<&ThirdPartyEntry> = Vec::new();
    let mut unknown = Vec::new();
    for id in dedup(req.ids) {
        if let Some(entry) = index.snippets.iter().find(|e| e.id.as_str() == id) {
            requested.push(entry);
        } else if let Some(entry) = index.third_party.iter().find(|e| e.id.as_str() == id) {
            third_party.push(entry);
        } else {
            unknown.push(id.to_owned());
        }
    }
    if !unknown.is_empty() {
        return Err(DocsError::UnknownIds {
            ids: unknown,
            nothing_published: false,
        });
    }
    let reader = Reader {
        home: req.home,
        index: &index,
        exec: ExecPolicy::read(req.cwd),
        lens: req.target.map_or(Lens::Current, Lens::Target),
    };
    let warning = behind_warning(req.home, &caller)?;
    let reading = if req.closure {
        closure(&reader, req, &requested, &third_party, warning)?
    } else {
        plain(&reader, req, &requested, &third_party, warning)?
    };
    tracing::debug!(
        event = "docs_get_completed",
        requested = req.ids.len(),
        closure = req.closure,
        exec_failed = reading.exec_failed,
        duration_ms = elapsed_ms(started),
    );
    Ok(reading)
}

fn dedup(ids: &[String]) -> Vec<&str> {
    let mut seen = BTreeSet::new();
    ids.iter()
        .map(String::as_str)
        .filter(|id| seen.insert(*id))
        .collect()
}

fn plain<'i>(
    reader: &Reader<'i>,
    req: &GetRequest<'_>,
    requested: &[&'i IndexEntry],
    third_party: &[&ThirdPartyEntry],
    warning: Option<String>,
) -> Result<Reading, DocsError> {
    let roots: Vec<Loaded<'_>> = requested
        .iter()
        .map(|e| reader.load(e))
        .collect::<Result<_, _>>()?;
    let graph = reader.follow(&roots, Strictness::List)?;
    let lens = req
        .target
        .map_or_else(String::new, |slug| format!(" --target {slug}"));
    let mut out = vec![format!("# instrukt docs get{lens}: {}", req.ids.join(" "))];
    let not_loaded = graph.names(reader.home);
    if !not_loaded.is_empty() {
        out.push(format!(
            "# Required reads (not loaded) that must have been read at least once: {}",
            not_loaded.join(", ")
        ));
    }
    out.extend(warning);
    let mut exec_failed = false;
    for loaded in &roots {
        out.push(String::new());
        let (block, failed) = reader.render(loaded)?;
        out.push(block);
        exec_failed |= failed;
    }
    for entry in third_party {
        out.push(String::new());
        let (block, failed) = reader.render_third_party(entry)?;
        out.push(block);
        exec_failed |= failed;
    }
    Ok(Reading {
        text: finish(out),
        exec_failed,
    })
}

fn closure<'i>(
    reader: &Reader<'i>,
    req: &GetRequest<'_>,
    requested: &[&'i IndexEntry],
    third_party: &[&ThirdPartyEntry],
    warning: Option<String>,
) -> Result<Reading, DocsError> {
    let refuse = |reason: String| DocsError::ClosureRefused {
        id: req.ids.join(" "),
        reason,
    };
    if req.target.is_some() {
        return Err(refuse(
            "a closure reads the current contract; it takes no --target".into(),
        ));
    }
    let ([entry], []) = (requested, third_party) else {
        return Err(refuse(format!(
            "a closure is read for exactly one procedure; {} ids were given",
            requested.len() + third_party.len()
        )));
    };
    if Taxonomy::from_dir(&entry.taxonomy) != Some(Taxonomy::Procedure) {
        return Err(refuse(format!(
            "a closure is read for exactly one procedure; this is a {}",
            entry.taxonomy
        )));
    }
    let root = reader.load(entry)?;
    if root.snippet.is_pending() {
        return Err(refuse(format!(
            "it is pending, delivered by: {}",
            slugs(root.snippet.delivered_by())
        )));
    }
    let strictness = if root.snippet.current().frontmatter().entry.is_root() {
        Strictness::Refuse
    } else {
        Strictness::Skip
    };
    let graph = reader.follow(std::slice::from_ref(&root), strictness)?;
    let included = graph.names(reader.home);
    let mut out = vec![format!(
        "# instrukt docs get --closure: {}",
        entry.id.as_str()
    )];
    if !included.is_empty() {
        out.push(format!(
            "# Required reads (included): {}",
            included.join(", ")
        ));
    }
    out.extend(warning);
    let mut exec_failed = false;
    // Every read in the order it is listed, then the procedure itself.
    for item in &graph.items {
        out.push(String::new());
        match item {
            Reached::Bundle(domain) => out.push(reader.render_bundle(domain)?),
            Reached::Snippet(loaded) => {
                let (block, failed) = reader.render(loaded)?;
                out.push(block);
                exec_failed |= failed;
            }
            // A closure refuses or leaves out what cannot be read.
            Reached::Unreadable(_) => {}
        }
    }
    out.push(String::new());
    let (block, failed) = reader.render(&root)?;
    out.push(block);
    exec_failed |= failed;
    Ok(Reading {
        text: finish(out),
        exec_failed,
    })
}

fn bundle_path(home: &Home, domain: &DomainName) -> String {
    home.domain_dir(domain)
        .join("bundle.md")
        .display()
        .to_string()
}

/// What following required reads does with one that is pending or cannot
/// be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Strictness {
    /// A plain read: everything reached is listed as not loaded.
    List,
    /// A closure of a supporting procedure: such a read is left out.
    Skip,
    /// A closure of a procedure with root frontmatter: such a read refuses
    /// the closure.
    Refuse,
}

/// Everything required reads reached beyond the roots, each once, depth
/// first in declaration order at its first occurrence.
struct Graph<'i> {
    items: Vec<Reached<'i>>,
}

/// One required read reached.
enum Reached<'i> {
    Snippet(Box<Loaded<'i>>),
    /// A published domain's bundle, which has no id: named by its path.
    Bundle(DomainName),
    /// A reference that names nothing readable, as written.
    Unreadable(String),
}

impl Graph<'_> {
    /// Each read's name, in order: a snippet's id, a bundle's path, an
    /// unreadable reference as written.
    fn names(&self, home: &Home) -> Vec<String> {
        self.items
            .iter()
            .map(|item| match item {
                Reached::Snippet(loaded) => loaded.entry.id.as_str().to_owned(),
                Reached::Bundle(domain) => bundle_path(home, domain),
                Reached::Unreadable(raw) => raw.clone(),
            })
            .collect()
    }
}

/// What a walk of required reads has seen so far.
#[derive(Default)]
struct Walk {
    snippets: BTreeSet<SnippetId>,
    bundles: BTreeSet<DomainName>,
    unreadable: BTreeSet<String>,
}

/// A snippet read from its file, with the repository it belongs to when
/// it is local.
struct Loaded<'i> {
    entry: &'i IndexEntry,
    snippet: Snippet,
    repo_root: Option<PathBuf>,
}

impl Loaded<'_> {
    fn project(&self) -> Option<&ProjectSlug> {
        (self.entry.scope == IndexScope::Local).then_some(&self.entry.source_project)
    }
}

struct Reader<'i> {
    home: &'i Home,
    index: &'i GlobalIndex,
    exec: ExecPolicy,
    lens: Lens<'i>,
}

impl<'i> Reader<'i> {
    /// Reads the snippet `entry` indexes, with its companions.
    fn load(&self, entry: &'i IndexEntry) -> Result<Loaded<'i>, DocsError> {
        let (root, repo_rel, project) = match entry.scope {
            IndexScope::Global => (self.home.docs_dir(), global_rel(entry)?, None),
            IndexScope::Local => {
                let (root, rel) = local_root(entry)?;
                (root, rel, Some(entry.source_project.clone()))
            }
        };
        let outside: fn(PathBuf) -> DocsError = match entry.scope {
            IndexScope::Global => outside_home,
            IndexScope::Local => outside_repository,
        };
        let (on_disk, as_source) = match entry.scope {
            // The home holds `docs/global/<domain>/…` at `<docs>/<domain>/…`.
            IndexScope::Global => (
                repo_rel.trim_start_matches("docs/global/").to_owned(),
                repo_rel.clone(),
            ),
            IndexScope::Local => (repo_rel.clone(), repo_rel.clone()),
        };
        let missing = || match entry.scope {
            IndexScope::Global => DocsError::MissingPublishedFile {
                project: entry.source_project.clone(),
                path: root.join(&on_disk),
            },
            IndexScope::Local => DocsError::MissingLocalFile {
                project: entry.source_project.clone(),
                path: root.join(&on_disk),
            },
        };
        if entry.scope == IndexScope::Local {
            match std::fs::symlink_metadata(&root) {
                Ok(_) => {}
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Err(missing()),
                Err(source) => return Err(DocsError::Read { path: root, source }),
            }
        }
        let text = read_confined(&root, Path::new(&on_disk), outside)?.ok_or_else(missing)?;
        let mut sources = vec![SourceFile {
            path: as_source.clone(),
            text,
        }];
        let file = root.join(&on_disk);
        let dir = file.parent().unwrap_or(&root);
        let listing = std::fs::read_dir(dir).map_err(|source| DocsError::Read {
            path: dir.to_path_buf(),
            source,
        })?;
        let source_dir = as_source.rsplit_once('/').map_or("", |(dir, _)| dir);
        let disk_dir = on_disk.rsplit_once('/').map_or("", |(dir, _)| dir);
        for item in listing {
            let item = item.map_err(|source| DocsError::Read {
                path: dir.to_path_buf(),
                source,
            })?;
            let Some(name) = item.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            let candidate = format!("{source_dir}/{name}");
            if !matches!(companion_shape(&candidate), Some((parent, _)) if parent == as_source) {
                continue;
            }
            let rel = format!("{disk_dir}/{name}");
            if let Some(text) = read_confined(&root, Path::new(&rel), outside)? {
                sources.push(SourceFile {
                    path: candidate,
                    text,
                });
            }
        }
        let corpus = Corpus::from_sources(project, self.home, sources);
        let snippet =
            corpus
                .snippet(&entry.id)
                .cloned()
                .ok_or_else(|| DocsError::IndexMismatch {
                    id: entry.id.clone(),
                    path: entry.path.clone(),
                })?;
        Ok(Loaded {
            entry,
            snippet,
            repo_root: (entry.scope == IndexScope::Local).then_some(root),
        })
    }

    /// The required reads of the version the lens reads.
    /// Exactly what rendering it reports ([`super::Rendered::required_reads`]),
    /// fragments included, without running its read-time commands.
    fn reads_of(&self, loaded: &Loaded<'_>) -> Result<Vec<Reference>, DocsError> {
        let View::Visible { version, .. } = loaded.snippet.view(self.lens) else {
            return Ok(Vec::new());
        };
        _render::dependency_reads(version.body(), self.lens, &self.context(loaded))
    }

    /// Follows required reads from `roots`, transitively, by what each
    /// names: depth first in declaration order, each read once, at its
    /// first occurrence. A local snippet's reads reach its own project's
    /// local snippets and published snippets only.
    fn follow(&self, roots: &[Loaded<'i>], strictness: Strictness) -> Result<Graph<'i>, DocsError> {
        let mut walk = Walk {
            snippets: roots.iter().map(|l| l.entry.id.clone()).collect(),
            ..Walk::default()
        };
        let mut items = Vec::new();
        let root = roots
            .first()
            .map_or_else(String::new, |l| l.entry.id.as_str().to_owned());
        for loaded in roots {
            let reads = self.reads_of(loaded)?;
            self.visit(reads, strictness, &root, &mut walk, &mut items)?;
        }
        Ok(Graph { items })
    }

    fn visit(
        &self,
        reads: Vec<Reference>,
        strictness: Strictness,
        root: &str,
        walk: &mut Walk,
        items: &mut Vec<Reached<'i>>,
    ) -> Result<(), DocsError> {
        for read in reads {
            let refused = |reason: &str| DocsError::ClosureRefused {
                id: root.to_owned(),
                reason: format!("its required read `{}` {reason}", read.raw),
            };
            let mut unreadable = |items: &mut Vec<Reached<'i>>| {
                if walk.unreadable.insert(read.raw.clone()) {
                    items.push(Reached::Unreadable(read.raw.clone()));
                }
            };
            match &read.target {
                Some(RefTarget::Snippet(id)) => {
                    if walk.snippets.contains(id) {
                        continue;
                    }
                    let Some(entry) = self.index.snippets.iter().find(|e| &e.id == id) else {
                        match strictness {
                            Strictness::Refuse => {
                                return Err(refused("names no published snippet"));
                            }
                            Strictness::Skip => {}
                            Strictness::List => unreadable(items),
                        }
                        continue;
                    };
                    walk.snippets.insert(id.clone());
                    let loaded = match self.load(entry) {
                        Ok(loaded) => loaded,
                        // Only a missing file is unreadable; any other
                        // failure is the read's own error.
                        Err(
                            err @ (DocsError::MissingLocalFile { .. }
                            | DocsError::MissingPublishedFile { .. }),
                        ) => match strictness {
                            Strictness::List => {
                                unreadable(items);
                                continue;
                            }
                            Strictness::Refuse => {
                                return Err(refused(&format!("cannot be read: {err}")));
                            }
                            Strictness::Skip => return Err(err),
                        },
                        Err(err) => return Err(err),
                    };
                    if loaded.snippet.is_pending() {
                        match strictness {
                            Strictness::Refuse => return Err(refused("is still pending")),
                            Strictness::Skip => continue,
                            Strictness::List => {}
                        }
                    }
                    // A withheld snippet names no reads (`reads_of`).
                    let nested = self.reads_of(&loaded)?;
                    items.push(Reached::Snippet(Box::new(loaded)));
                    self.visit(nested, strictness, root, walk, items)?;
                }
                Some(RefTarget::Bundle(domain)) => {
                    if walk.bundles.contains(domain) {
                        continue;
                    }
                    let path = self.home.domain_dir(domain).join("bundle.md");
                    let readable = read_confined(
                        &self.home.docs_dir(),
                        &Path::new(domain.as_str()).join("bundle.md"),
                        outside_home,
                    )?
                    .is_some();
                    match (readable, strictness) {
                        (true, _) | (false, Strictness::List) => {
                            walk.bundles.insert(domain.clone());
                            items.push(Reached::Bundle(domain.clone()));
                        }
                        (false, Strictness::Refuse) => {
                            return Err(refused(&format!(
                                "names a bundle that is not published ({})",
                                path.display()
                            )));
                        }
                        (false, Strictness::Skip) => {}
                    }
                }
                // A manifest is not a required read; a malformed reference
                // is validation's finding.
                Some(RefTarget::LocalFile(_)) | None => {
                    if strictness == Strictness::Refuse {
                        return Err(refused("names no published snippet or bundle"));
                    }
                }
            }
        }
        Ok(())
    }

    fn context<'c>(&'c self, loaded: &'c Loaded<'_>) -> RenderContext<'c> {
        RenderContext {
            fragments: Fragments::Home(self.home),
            exec: &self.exec,
            repo_root: loaded.repo_root.as_deref(),
            project: loaded.project(),
        }
    }

    /// One snippet's block: its header, then its body through the read
    /// pipeline, or the notice that replaces it; whether a read-time
    /// command failed.
    fn render(&self, loaded: &Loaded<'_>) -> Result<(String, bool), DocsError> {
        let snippet = &loaded.snippet;
        let id = snippet.id().as_str();
        let mut block = vec![
            "---".to_owned(),
            format!("id: {id}"),
            format!("type: {}", snippet.taxonomy()),
            format!("domain: {}", snippet.domain()),
            format!(
                "scope: {}",
                match snippet.scope() {
                    Scope::Global => "global",
                    Scope::Project => "project",
                }
            ),
            format!("description: {}", loaded.entry.description),
            "---".to_owned(),
        ];
        let mut failed = false;
        match snippet.view(self.lens) {
            View::Withheld { delivered_by } => {
                block.push(format!(
                    "# NOTE: This snippet is pending; its body is withheld until it is delivered \
                     by: {}.",
                    slugs(delivered_by)
                ));
                block.push(format!(
                    "# Read it as that todo will deliver it: instrukt docs get {id} --target <slug>"
                ));
            }
            View::Retired => {}
            View::Visible { version, .. } => {
                let rendered = version.render(self.lens, &self.context(loaded))?;
                failed = !rendered.exec_failures.is_empty();
                block.push(rendered.text.trim_matches('\n').to_owned());
                let pending = snippet.superseded_by();
                if self.lens == Lens::Current && !pending.is_empty() {
                    block.push(String::new());
                    block.push(format!(
                        "# NOTE: This snippet has pending changes owned by: {}. The text above is \
                         the current contract.",
                        slugs(&pending)
                    ));
                    block.push(format!(
                        "# Read a todo's view of it: instrukt docs get {id} --target <slug>"
                    ));
                }
            }
        }
        Ok((block.join("\n"), failed))
    }

    fn render_third_party(&self, entry: &ThirdPartyEntry) -> Result<(String, bool), DocsError> {
        let docs = self.home.docs_dir();
        let text = read_confined(&docs, &entry.path, outside_home)?.ok_or_else(|| {
            DocsError::MissingPublishedFile {
                project: entry.source_project.clone(),
                path: docs.join(&entry.path),
            }
        })?;
        let cx = RenderContext {
            fragments: Fragments::Home(self.home),
            exec: &self.exec,
            repo_root: None,
            project: None,
        };
        let rendered = _render::render(&text, Lens::Current, &cx)?;
        // third-party-docs spec: `global` for the docs of the project that
        // owns the `general` domain, `project` for any other project's.
        let owns_general = self.index.snippets.iter().any(|e| {
            e.scope == IndexScope::Global
                && e.domain == "general"
                && e.source_project == entry.source_project
        });
        let block = [
            "---".to_owned(),
            format!("id: {}", entry.id.as_str()),
            format!("scope: {}", if owns_general { "global" } else { "project" }),
            format!("description: {}", entry.description),
            "---".to_owned(),
            rendered.text.trim_matches('\n').to_owned(),
        ];
        Ok((block.join("\n"), !rendered.exec_failures.is_empty()))
    }

    fn render_bundle(&self, domain: &DomainName) -> Result<String, DocsError> {
        let rel = Path::new(domain.as_str()).join("bundle.md");
        let docs = self.home.docs_dir();
        let text = read_confined(&docs, &rel, outside_home)?.ok_or_else(|| DocsError::Read {
            path: docs.join(&rel),
            source: std::io::ErrorKind::NotFound.into(),
        })?;
        Ok([
            "---".to_owned(),
            format!("bundle: {domain}"),
            format!("path: {}", docs.join(&rel).display()),
            "---".to_owned(),
            text.trim_matches('\n').to_owned(),
        ]
        .join("\n"))
    }
}

fn outside_home(path: PathBuf) -> DocsError {
    DocsError::OutsideHome { path }
}

fn outside_repository(path: PathBuf) -> DocsError {
    DocsError::OutsideRepository { path }
}

/// A global entry's file as a repository path (`docs/global/<rel>`), from
/// the path the entry carries relative to the home's docs directory.
fn global_rel(entry: &IndexEntry) -> Result<String, DocsError> {
    let parts: Option<Vec<&str>> = entry
        .path
        .components()
        .map(|c| match c {
            Component::Normal(part) => part.to_str(),
            _ => None,
        })
        .collect();
    let rel = parts.map(|p| format!("docs/global/{}", p.join("/")));
    match rel {
        Some(rel) if rel == format!("docs/global/{}.md", entry.id.as_str()) => Ok(rel),
        _ => Err(DocsError::IndexMismatch {
            id: entry.id.clone(),
            path: entry.path.clone(),
        }),
    }
}

/// A local entry's project root and the file's repository path
/// (`docs/local/<taxonomy>/<rest>.md`), from the absolute path the entry
/// carries.
fn local_root(entry: &IndexEntry) -> Result<(PathBuf, String), DocsError> {
    let mismatch = || DocsError::IndexMismatch {
        id: entry.id.clone(),
        path: entry.path.clone(),
    };
    let prefix = format!("local/projects/{}/", entry.source_project.as_str());
    let rest = entry
        .id
        .as_str()
        .strip_prefix(&prefix)
        .ok_or_else(mismatch)?;
    let rel = format!("docs/local/{rest}.md");
    if !entry.path.is_absolute() || !entry.path.ends_with(&rel) {
        return Err(mismatch());
    }
    let depth = Path::new(&rel).components().count();
    let root = entry
        .path
        .ancestors()
        .nth(depth)
        .ok_or_else(mismatch)?
        .to_path_buf();
    Ok((root, rel))
}

/// Where the caller stands: the repository containing its working
/// directory and the project that repository declares.
struct Caller {
    repo_root: Option<PathBuf>,
    project: Option<ProjectSlug>,
}

impl Caller {
    fn of(cwd: &Path) -> Result<Self, DocsError> {
        let Some(root) = git::worktree_root(cwd)? else {
            return Ok(Self {
                repo_root: None,
                project: None,
            });
        };
        let project = crate::config::project_slug(&root)?;
        Ok(Self {
            repo_root: Some(root),
            project,
        })
    }
}

/// The warning that the caller's project has commits on main publication
/// has not published.
fn behind_warning(home: &Home, caller: &Caller) -> Result<Option<String>, DocsError> {
    let (Some(root), Some(project)) = (&caller.repo_root, &caller.project) else {
        return Ok(None);
    };
    let Some(record) = PublishedRecord::load(home, project)? else {
        return Ok(None);
    };
    let branch = git::default_branch(root, "origin")?;
    let reference = format!("refs/heads/{branch}");
    let main = git::run(
        root,
        ["for-each-ref", "--format=%(objectname)", reference.as_str()],
    )?;
    if main.is_empty() || main == record.commit.as_str() {
        return Ok(None);
    }
    Ok(Some(format!(
        "# WARNING: publication is behind main: main is at {main}, the published commit is {}. \
         Run `instrukt sync` in this project to publish it.",
        record.commit.as_str()
    )))
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, scope: IndexScope, path: &str) -> IndexEntry {
        IndexEntry {
            id: SnippetId::parse(id).unwrap(),
            description: String::new(),
            taxonomy: "spec".into(),
            scope,
            domain: String::new(),
            path: PathBuf::from(path),
            source_project: ProjectSlug::parse("demo").unwrap(),
            visibility: None,
            delivered_by: Vec::new(),
            superseded_by: Vec::new(),
            activation_trigger: None,
            role: None,
            subscriptions: Vec::new(),
            parameters: Vec::new(),
            dispatch: None,
            event_delivery: None,
            interruptible: false,
        }
    }

    #[test]
    fn a_global_entry_is_read_only_at_its_identity_below_the_home() {
        let id = "acme/spec/a";
        let placed = entry(id, IndexScope::Global, &format!("{id}.md"));
        assert_eq!(
            global_rel(&placed).unwrap(),
            format!("docs/global/{}", placed.path.display())
        );
        for path in ["../../etc/passwd", "/etc/passwd", "acme/spec/b.md"] {
            assert!(
                matches!(
                    global_rel(&entry(id, IndexScope::Global, path)),
                    Err(DocsError::IndexMismatch { .. })
                ),
                "{path}"
            );
        }
    }

    #[test]
    fn a_local_entry_names_its_project_root_by_the_path_it_carries() {
        let id = "local/projects/demo/spec/feature/a";
        let rel = "docs/local/spec/feature/a.md";
        let root = PathBuf::from("/work/demo");
        let placed = entry(id, IndexScope::Local, &root.join(rel).display().to_string());
        assert_eq!(local_root(&placed).unwrap(), (root, rel.to_owned()));
        for path in [
            "docs/local/spec/feature/a.md",
            "/work/demo/docs/local/spec/feature/b.md",
        ] {
            assert!(
                matches!(
                    local_root(&entry(id, IndexScope::Local, path)),
                    Err(DocsError::IndexMismatch { .. })
                ),
                "{path}"
            );
        }
        let other = entry(
            "local/projects/other/spec/a",
            IndexScope::Local,
            "/x/docs/local/spec/a.md",
        );
        assert!(matches!(
            local_root(&other),
            Err(DocsError::IndexMismatch { .. })
        ));
    }
}
