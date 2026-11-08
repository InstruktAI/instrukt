//! Progressive retrieval: `docs index` lists what the home holds, `docs get`
//! reads it through the read pipeline, as one snippet's closure or as one
//! domain's position index rendered
//! (`project/instrukt/spec/feature/docs/progressive-retrieval`).
//!
//! All read the global index and the publication record; none writes
//! anything. A published snippet is read from the home, a local snippet
//! from the absolute path its index entry carries, with the companions
//! beside either file. Where a listing or a read names a position or a
//! snippet, it names its source: the file or folder in the checkout it was
//! published from.

use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};
use std::time::Instant;

use super::_corpus::{Corpus, Lens, Snippet, SourceFile, View};
use super::_identity::{Scope, Taxonomy, companion_shape, published_source};
use super::_position::{GENERAL, INDEX_FILE, Position, PositionIndex};
use super::_refs::{RefTarget, Reference};
use super::_render::{self, RenderContext, Values, read_confined};
use super::_selection::{InFull, Listing, Selection};
use super::DocsError;
use crate::config::Configuration;
use crate::git;
use crate::home::Home;
use crate::ids::{DomainName, ProjectSlug, SnippetId, TodoSlug};
use crate::index::{GlobalIndex, IndexEntry, IndexScope, ThirdPartyEntry};
use crate::publication::{PublishedRecord, position_owners};

/// What `docs index` lists. Every filter only narrows; an empty list is no
/// filter.
#[derive(Debug, Clone, Copy)]
pub struct IndexRequest<'a> {
    pub home: &'a Home,
    /// The caller's working directory: the project it stands in.
    pub cwd: &'a Path,
    /// Domain names as given (`general` included); an unknown or reserved
    /// one fails the listing.
    pub domains: &'a [String],
    pub areas: &'a [Taxonomy],
    pub third_party: bool,
    pub target: Option<&'a TodoSlug>,
    pub project: Option<&'a ProjectSlug>,
}

/// What `docs get` reads.
#[derive(Debug, Clone, Copy)]
pub struct GetRequest<'a> {
    pub env: &'a crate::config::Env,
    pub home: &'a Home,
    /// The caller's working directory: where read-time commands run, and
    /// the project it stands in.
    pub cwd: &'a Path,
    pub ids: &'a [String],
    pub closure: bool,
    pub target: Option<&'a TodoSlug>,
}

/// What `docs get --domain` reads.
#[derive(Debug, Clone, Copy)]
pub struct DomainRequest<'a> {
    pub env: &'a crate::config::Env,
    pub home: &'a Home,
    /// The caller's working directory: the project it stands in.
    pub cwd: &'a Path,
    /// The domain name as given (`general` included).
    pub domain: &'a str,
}

/// The text `docs get` prints, and whether a configured value in it could
/// not be resolved (the read then reports failure).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reading {
    pub text: String,
    pub unresolved: bool,
}

/// A retained phase belongs to the immutable todo, even when its slug changed.
struct TargetWorkspace {
    project: ProjectSlug,
    root: PathBuf,
    prior_slugs: Vec<TodoSlug>,
    slug: TodoSlug,
}
impl TargetWorkspace {
    fn sources(&self, mut sources: Vec<SourceFile>) -> Result<Vec<SourceFile>, DocsError> {
        // Project ownership names through the immutable todo's rename history for
        // this read only. Authored files and the native phase branch stay untouched.
        for from in self.prior_slugs.iter().filter(|from| *from != &self.slug) {
            let changes = super::_transfer::plan(sources.clone(), from, &self.slug)?;
            sources = sources
                .into_iter()
                .map(|file| match changes.get(&file.path) {
                    Some(change) => SourceFile {
                        path: change.moved_to.clone().unwrap_or(file.path),
                        text: change.text.clone(),
                    },
                    None => file,
                })
                .collect();
        }
        Ok(sources)
    }
}

fn target_workspace(
    home: &Home,
    caller: &Caller,
    target: Option<&TodoSlug>,
) -> Result<Option<TargetWorkspace>, DocsError> {
    use crate::state_db::{
        StateDbError, StateRead,
        rusqlite::{OptionalExtension, params},
    };
    let Some(target) = target else {
        return Ok(None);
    };
    let project = caller
        .project
        .as_ref()
        .ok_or_else(|| DocsError::ConfigUnresolved {
            path: "--target".into(),
            reason: "target documentation needs its project".into(),
        })?;
    let state = StateRead::open(home)?.ok_or_else(|| DocsError::ConfigUnresolved {
        path: target.to_string(),
        reason: "target todo has no work record".into(),
    })?;
    let recorded: bool = state
        .conn()
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM todos WHERE project=?1 AND slug=?2)",
            params![project.as_str(), target.as_str()],
            |r| r.get(0),
        )
        .map_err(StateDbError::sqlite(state.path()))?;
    if !recorded {
        return Err(DocsError::ConfigUnresolved {
            path: target.to_string(),
            reason: "target todo has no work record".into(),
        });
    }
    let branch: Option<String> = state.conn().query_row(
        "SELECT json_extract(b.evidence,'$.branch') FROM session_bindings b JOIN todos t ON t.id=b.todo_id WHERE t.project=?1 AND t.slug=?2 AND json_extract(b.evidence,'$.assignment')='driver' AND json_extract(b.evidence,'$.lane')='prepare' ORDER BY b.attempt DESC LIMIT 1",
        params![project.as_str(),target.as_str()], |r|r.get(0)
    ).optional().map_err(StateDbError::sqlite(state.path()))?.flatten();
    let branch = branch.unwrap_or_else(|| format!("{target}/prepare"));
    let root = caller
        .repo_root
        .as_ref()
        .ok_or_else(|| DocsError::ConfigUnresolved {
            path: target.to_string(),
            reason: "target needs a registered checkout".into(),
        })?;
    let listing = git::run(root, ["worktree", "list", "--porcelain"])?;
    let path = listing
        .split("\n\n")
        .find(|entry| {
            entry
                .lines()
                .any(|l| l == format!("branch refs/heads/{branch}"))
        })
        .and_then(|entry| entry.lines().find_map(|l| l.strip_prefix("worktree ")))
        .map(PathBuf::from);
    let Some(path) = path else {
        let approved:Option<String>=state.conn().query_row("SELECT json_extract(s.evidence,'$.head') FROM stamps s JOIN todos t ON t.id=s.todo_id WHERE t.project=?1 AND t.slug=?2 AND s.fact='preparation_head_reviewed' AND json_extract(s.evidence,'$.verdict')='approve' ORDER BY s.sequence DESC LIMIT 1",params![project.as_str(),target.as_str()],|r|r.get(0)).optional().map_err(StateDbError::sqlite(state.path()))?.flatten();
        if let Some(head) = approved {
            let main = crate::github::default_branch(root)?;
            let remote = format!("refs/remotes/origin/{main}");
            let reference = if git::succeeds(root, ["show-ref", "--verify", "--quiet", &remote])? {
                remote
            } else {
                main
            };
            if git::succeeds(root, ["merge-base", "--is-ancestor", &head, &reference])? {
                return Ok(None);
            }
        }
        return Err(DocsError::ConfigUnresolved{path:target.to_string(),reason:"preparation worktree is missing and its integration is unconfirmed; no main-checkout fallback".into()});
    };
    let mut prior_slugs=state.conn().prepare("SELECT a.slug FROM todo_aliases a JOIN todos t ON t.id=a.todo_id WHERE t.project=?1 AND t.slug=?2 ORDER BY a.rowid").map_err(StateDbError::sqlite(state.path()))?.query_map(params![project.as_str(),target.as_str()],|r|r.get::<_,String>(0)).map_err(StateDbError::sqlite(state.path()))?.collect::<Result<Vec<_>,_>>().map_err(StateDbError::sqlite(state.path()))?.into_iter().map(|slug|TodoSlug::parse(&slug).map_err(|error|DocsError::ConfigUnresolved {path:target.to_string(),reason:error.to_string()})).collect::<Result<Vec<_>,_>>()?;
    if let Some(original) = branch
        .strip_suffix("/prepare")
        .and_then(|slug| TodoSlug::parse(slug).ok())
        && !prior_slugs.contains(&original)
    {
        prior_slugs.push(original);
    }
    Ok(Some(TargetWorkspace {
        project: project.clone(),
        root: path,
        prior_slugs,
        slug: target.clone(),
    }))
}
fn target_index(
    index: &mut GlobalIndex,
    target: Option<&TargetWorkspace>,
    home: &Home,
) -> Result<(), DocsError> {
    if let Some(target) = target {
        let (project, root) = (&target.project, &target.root);
        let corpus = Corpus::from_sources(
            Some(project.clone()),
            home,
            target.sources(super::_corpus::read_tree(root)?)?,
        );
        index.snippets.retain(|e| &e.source_project != project);
        index
            .snippets
            .extend(crate::publication::target_entries(project, &corpus, root));
    }
    Ok(())
}

const AREAS: &str = "principle|concept|policy|procedure|design|spec|adr";

/// The `docs index` listing.
pub fn index(req: &IndexRequest<'_>) -> Result<String, DocsError> {
    let started = Instant::now();
    let caller = Caller::of(req.cwd)?;
    let target = target_workspace(req.home, &caller, req.target)?;
    let Some(mut index) =
        GlobalIndex::load(req.home)?.or_else(|| target.as_ref().map(|_| GlobalIndex::default()))
    else {
        return Ok(nothing_published());
    };

    target_index(&mut index, target.as_ref(), req.home)?;
    let owners = position_owners(req.home)?;
    let valid = valid_domains(&index, &owners);
    let domains = named_domains(req.domains, &valid)?;
    let mut out = vec![
        "# instrukt docs index: snippet ids with their descriptions".to_owned(),
        format!(
            "# Filter with --domain <{}> (repeatable), --areas <{AREAS}>, --third-party, \
             --target <slug>, --project <slug>",
            valid
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
                .join("|")
        ),
    ];
    if let Some(target) = &target {
        let root = &target.root;
        out.push(format!(
            "# Pending source: preparation worktree {}",
            root.display()
        ));
    }
    for domain in &domains {
        let owner = owners.get(domain).cloned().or_else(|| {
            index
                .snippets
                .iter()
                .find(|e| e.scope == IndexScope::Global && e.domain == domain.as_str())
                .map(|e| e.source_project.clone())
        });
        if let Some(owner) = owner {
            out.extend(source_line(
                req.home,
                domain.as_str(),
                &owner,
                &Position::of_domain(domain).dir(),
            )?);
        }
    }
    out.extend(behind_warning(req.home, &caller)?);

    let local_project = req.project.or(caller.project.as_ref());
    if let Some(project) = req.project
        && !index.snippets.iter().any(|e| &e.source_project == project)
        && PublishedRecord::load(req.home, project)?.is_none()
    {
        out.push(String::new());
        out.push(format!("# No project `{project}` has published."));
        return Ok(finish(out));
    }
    if domains.is_empty()
        && let Some(project) = local_project
    {
        out.extend(source_line(
            req.home,
            &format!("project {project}"),
            project,
            &Position::Project.dir(),
        )?);
    }
    out.push(String::new());
    let mut entries: Vec<&IndexEntry> = index
        .snippets
        .iter()
        .filter(|e| match e.scope {
            IndexScope::Global => {
                req.project.is_none()
                    && (domains.is_empty() || domains.iter().any(|d| d.as_str() == e.domain))
            }
            IndexScope::Local => domains.is_empty() && local_project == Some(&e.source_project),
        })
        .filter(|e| {
            req.areas.is_empty()
                || Taxonomy::from_dir(&e.taxonomy).is_some_and(|t| req.areas.contains(&t))
        })
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

    let narrowed = !domains.is_empty()
        || !req.areas.is_empty()
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

/// The domain names a listing or a domain read accepts: every claimed
/// published position and every domain the global index holds.
fn valid_domains(
    index: &GlobalIndex,
    owners: &std::collections::BTreeMap<DomainName, ProjectSlug>,
) -> BTreeSet<String> {
    owners
        .keys()
        .map(|d| d.as_str().to_owned())
        .chain(
            index
                .snippets
                .iter()
                .filter(|e| e.scope == IndexScope::Global)
                .map(|e| e.domain.clone()),
        )
        .collect()
}

/// `given` as domain names, each one of `valid`; an unknown or reserved
/// name fails, naming the valid domains.
fn named_domains(given: &[String], valid: &BTreeSet<String>) -> Result<Vec<DomainName>, DocsError> {
    let mut named = Vec::new();
    let mut unknown = Vec::new();
    for raw in given {
        match DomainName::parse(raw) {
            Ok(domain) if valid.contains(raw) => {
                if !named.contains(&domain) {
                    named.push(domain);
                }
            }
            _ => unknown.push(raw.clone()),
        }
    }
    if unknown.is_empty() {
        Ok(named)
    } else {
        Err(DocsError::UnknownDomains {
            unknown,
            valid: valid.iter().cloned().collect(),
        })
    }
}

/// The line naming where the position `folder` of `owner` is authored: its
/// folder in the checkout it was published from, the published commit and
/// the repository's remote. `None` when `owner` has no publication record.
fn source_line(
    home: &Home,
    name: &str,
    owner: &ProjectSlug,
    folder: &str,
) -> Result<Option<String>, DocsError> {
    let Some(record) = PublishedRecord::load(home, owner)? else {
        return Ok(None);
    };
    Ok(Some(format!(
        "# Source of {name}: {}/ — published commit {} — remote {}",
        record.checkout.join(folder).display(),
        record.commit.as_str(),
        record.remote.as_deref().unwrap_or("none")
    )))
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

/// The `docs get` reading.
pub fn get(req: &GetRequest<'_>) -> Result<Reading, DocsError> {
    let started = Instant::now();
    let caller = Caller::of(req.cwd)?;
    let target = target_workspace(req.home, &caller, req.target)?;
    let Some(mut index) =
        GlobalIndex::load(req.home)?.or_else(|| target.as_ref().map(|_| GlobalIndex::default()))
    else {
        return Err(DocsError::UnknownIds {
            ids: req.ids.to_vec(),
            nothing_published: true,
        });
    };

    target_index(&mut index, target.as_ref(), req.home)?;
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
    // Configured values resolve as `instrukt config get` would for the
    // caller: the effective configuration of the project it stands in, the
    // global layer alone outside one, for every snippet alike.
    let configuration = Configuration::load(req.home, caller.repo_root.as_deref())?;
    let reader = Reader {
        home: req.home,
        index: &index,
        values: Values::read(&configuration, req.env),
        lens: req.target.map_or(Lens::Current, Lens::Target),
        target: target.as_ref(),
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
        unresolved = reading.unresolved,
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
    let not_loaded = graph.names(Strictness::List);
    if !not_loaded.is_empty() {
        out.push(format!(
            "# Required reads (not loaded) that must have been read at least once: {}",
            not_loaded.join(", ")
        ));
    }
    out.extend(warning);
    let mut unresolved = false;
    for loaded in &roots {
        out.push(String::new());
        let (block, failed) = reader.render(loaded)?;
        out.push(block);
        unresolved |= failed;
    }
    for entry in third_party {
        out.push(String::new());
        let (block, failed) = reader.render_third_party(entry)?;
        out.push(block);
        unresolved |= failed;
    }
    Ok(Reading {
        text: finish(out),
        unresolved,
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
        let reason = match (requested, third_party) {
            ([], [doc]) => format!(
                "a closure is read for exactly one snippet; {} is a third-party doc",
                doc.id.as_str()
            ),
            _ => format!(
                "a closure is read for exactly one snippet; {} ids were given",
                requested.len() + third_party.len()
            ),
        };
        return Err(refuse(reason));
    };
    let root = reader.load(entry)?;
    if root.snippet.is_pending() {
        return Err(refuse(format!(
            "it is pending, delivered by: {}",
            slugs(root.snippet.delivered_by())
        )));
    }
    // A procedure an agent is started on cannot run without what it
    // requires; any other snippet is read with what can be read.
    let strictness = if root.snippet.taxonomy() == Taxonomy::Procedure
        && root.snippet.current().frontmatter().entry.is_root()
    {
        Strictness::Refuse
    } else {
        Strictness::Skip
    };
    let graph = reader.follow(std::slice::from_ref(&root), strictness)?;
    let included = graph.names(strictness);
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
    if !graph.left_out.is_empty() {
        out.push(format!(
            "# Required reads left out: {}",
            graph.left_out.join(", ")
        ));
    }
    out.extend(warning);
    let mut unresolved = false;
    // Every read in the order it is listed, then the snippet itself.
    for item in &graph.items {
        if let Reached::Snippet(loaded) = item {
            out.push(String::new());
            let (block, failed) = reader.render(loaded)?;
            out.push(block);
            unresolved |= failed;
        }
    }
    out.push(String::new());
    let (block, failed) = reader.render(&root)?;
    out.push(block);
    unresolved |= failed;
    Ok(Reading {
        text: finish(out),
        unresolved,
    })
}

/// The `docs get --domain` reading: the domain's position index rendered
/// as publication renders that domain's group, every body through the
/// read pipeline.
pub fn get_domain(req: &DomainRequest<'_>) -> Result<Reading, DocsError> {
    let started = Instant::now();
    let caller = Caller::of(req.cwd)?;
    let index = GlobalIndex::load(req.home)?.unwrap_or_default();
    let owners = position_owners(req.home)?;
    let valid = valid_domains(&index, &owners);
    // One name given, so a successful resolution holds exactly one domain.
    let Some(domain) = named_domains(&[req.domain.to_owned()], &valid)?
        .into_iter()
        .next()
    else {
        return Err(DocsError::UnknownDomains {
            unknown: vec![req.domain.to_owned()],
            valid: valid.iter().cloned().collect(),
        });
    };
    let configuration = Configuration::load(req.home, caller.repo_root.as_deref())?;
    let reader = Reader {
        home: req.home,
        index: &index,
        values: Values::read(&configuration, req.env),
        lens: Lens::Current,
        target: None,
    };
    let mut out = vec![format!("# instrukt docs get --domain {domain}")];
    let Some(selection) = reader.domain_index(&domain)? else {
        out.push(format!(
            "# The domain `{domain}` has no {INDEX_FILE}: list its snippets with \
             `instrukt docs index --domain {domain}`."
        ));
        out.extend(behind_warning(req.home, &caller)?);
        return Ok(Reading {
            text: finish(out),
            unresolved: false,
        });
    };
    let mut expansion = Expansion::default();
    reader.expand(selection.expanded(), &mut expansion)?;
    let mut unresolved = false;
    let mut in_full = Vec::new();
    for loaded in &expansion.order {
        let View::Visible { version, .. } = loaded.snippet.view(Lens::Current) else {
            continue;
        };
        let rendered = version.render(Lens::Current, &reader.context(loaded))?;
        unresolved |= !rendered.config_refusals.is_empty();
        in_full.push(InFull {
            id: loaded.entry.id.clone(),
            position: position_name(&loaded.entry.id),
            body: rendered.text,
            trigger: trigger_of(loaded.entry).map(str::to_owned),
        });
    }
    let mut listed = Vec::new();
    for id in selection.listed() {
        match index.snippets.iter().find(|e| &e.id == id) {
            Some(entry) if entry.delivered_by.is_empty() => listed.push(listing(entry)),
            Some(_) => expansion.left_out.push(format!("{id} (pending)")),
            None => expansion
                .left_out
                .push(format!("{id} (names no published snippet)")),
        }
    }
    let mut triggered: Vec<Listing> = index
        .snippets
        .iter()
        .filter(|e| {
            e.scope == IndexScope::Global
                && e.domain == domain.as_str()
                && e.delivered_by.is_empty()
                && trigger_of(e).is_some()
        })
        .map(listing)
        .collect();
    triggered.sort_by(|a, b| a.id.cmp(&b.id));
    if !expansion.left_out.is_empty() {
        out.push(format!(
            "# Required reads left out: {}",
            expansion.left_out.join(", ")
        ));
    }
    out.extend(behind_warning(req.home, &caller)?);
    out.push(String::new());
    let section = Selection {
        position: domain.as_str().to_owned(),
        in_full,
        listed,
        triggered,
    }
    .render();
    out.push(section.trim_end().to_owned());
    if let Some(configured) = super::_selection::responsibilities(
        index
            .snippets
            .iter()
            .filter(|entry| entry.scope == IndexScope::Global && entry.domain == domain.as_str()),
    ) {
        out.push(String::new());
        out.push(configured);
    }
    tracing::debug!(
        event = "docs_get_domain_completed",
        domain = %domain,
        in_full = expansion.order.len(),
        left_out = expansion.left_out.len(),
        unresolved,
        duration_ms = elapsed_ms(started),
    );
    Ok(Reading {
        text: finish(out),
        unresolved,
    })
}

/// The name of the position a snippet belongs to: its domain, or
/// `project`.
fn position_name(id: &SnippetId) -> String {
    id.domain()
        .map_or_else(|| "project".to_owned(), |d| d.as_str().to_owned())
}

/// A delivered procedure's non-empty `activation_trigger`.
fn trigger_of(entry: &IndexEntry) -> Option<&str> {
    (Taxonomy::from_dir(&entry.taxonomy) == Some(Taxonomy::Procedure))
        .then_some(entry.activation_trigger.as_deref())
        .flatten()
        .map(str::trim)
        .filter(|t| !t.is_empty())
}

fn listing(entry: &IndexEntry) -> Listing {
    Listing {
        id: entry.id.clone(),
        description: entry.description.clone(),
        trigger: trigger_of(entry).map(str::to_owned),
    }
}

/// What following required reads does with one that is pending or cannot
/// be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Strictness {
    /// A plain read: everything reached is listed as not loaded.
    List,
    /// A closure of any snippet but a root procedure: such a read is left
    /// out and named.
    Skip,
    /// A closure of a procedure with root frontmatter: such a read refuses
    /// the closure.
    Refuse,
}

/// Everything required reads reached beyond the roots, each once, depth
/// first in declaration order at its first occurrence, and what a closure
/// left out.
struct Graph<'i> {
    items: Vec<Reached<'i>>,
    left_out: Vec<String>,
}

/// One required read reached.
enum Reached<'i> {
    Snippet(Box<Loaded<'i>>),
    /// A bare-domain read: in a closure, its `expanded` set follows it.
    Domain(DomainName),
    /// A reference that names nothing readable, as written.
    Unreadable(String),
}

impl Graph<'_> {
    /// Each read's name, in order: a snippet's id, a domain by its name
    /// (in a plain read, with the command that reads it), an unreadable
    /// reference as written.
    fn names(&self, strictness: Strictness) -> Vec<String> {
        self.items
            .iter()
            .map(|item| match item {
                Reached::Snippet(loaded) => loaded.entry.id.as_str().to_owned(),
                Reached::Domain(domain) if strictness == Strictness::List => {
                    format!("{domain} (`instrukt docs get --domain {domain}`)")
                }
                Reached::Domain(domain) => domain.as_str().to_owned(),
                Reached::Unreadable(raw) => raw.clone(),
            })
            .collect()
    }
}

/// What a walk of required reads has seen so far.
#[derive(Default)]
struct Walk {
    snippets: BTreeSet<SnippetId>,
    domains: BTreeSet<DomainName>,
    unreadable: BTreeSet<String>,
    left_out: Vec<String>,
}

/// A domain read's snippets in full, in traversal order, and what was left
/// out.
#[derive(Default)]
struct Expansion<'i> {
    order: Vec<Loaded<'i>>,
    seen: BTreeSet<SnippetId>,
    domains: BTreeSet<DomainName>,
    left_out: Vec<String>,
}

/// A snippet read from its file, with the repository it belongs to when
/// it is local.
struct Loaded<'i> {
    entry: &'i IndexEntry,
    snippet: Snippet,
}

impl Loaded<'_> {
    fn project(&self) -> Option<&ProjectSlug> {
        (self.entry.scope == IndexScope::Local).then_some(&self.entry.source_project)
    }
}

struct Reader<'i> {
    home: &'i Home,
    index: &'i GlobalIndex,
    values: Values<'i>,
    lens: Lens<'i>,
    target: Option<&'i TargetWorkspace>,
}

impl<'i> Reader<'i> {
    /// Reads the snippet `entry` indexes, with its companions.
    fn load(&self, entry: &'i IndexEntry) -> Result<Loaded<'i>, DocsError> {
        let (mut root, repo_rel, project) = match entry.scope {
            IndexScope::Global => (self.home.docs_dir(), global_rel(entry)?, None),
            IndexScope::Local => {
                let (root, rel) = local_root(entry)?;
                (root, rel, Some(entry.source_project.clone()))
            }
        };
        let targeted = self
            .target
            .filter(|target| target.project == entry.source_project);
        if let Some(target) = targeted {
            root = target.root.clone();
        }
        let outside: fn(PathBuf) -> DocsError = match entry.scope {
            IndexScope::Global => outside_home,
            IndexScope::Local => outside_repository,
        };
        let (on_disk, as_source) = if targeted.is_some() {
            (repo_rel.clone(), repo_rel.clone())
        } else {
            match entry.scope {
                // The home holds `docs/general/…` and `docs/domain/<x>/…` at
                // `<docs>/general/…` and `<docs>/<x>/…`: the id's own path.
                IndexScope::Global => (format!("{}.md", entry.id.as_str()), repo_rel.clone()),
                IndexScope::Local => (repo_rel.clone(), repo_rel.clone()),
            }
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
        let sources = match targeted {
            Some(target) => target.sources(sources)?,
            None => sources,
        };
        let corpus = Corpus::from_sources(project, self.home, sources);
        let snippet =
            corpus
                .snippet(&entry.id)
                .cloned()
                .ok_or_else(|| DocsError::IndexMismatch {
                    id: entry.id.clone(),
                    path: entry.path.clone(),
                })?;
        Ok(Loaded { entry, snippet })
    }

    /// The required reads of the version the lens reads.
    /// Exactly what rendering it reports ([`super::Rendered::required_reads`]),
    /// without resolving its configured values.
    fn reads_of(&self, loaded: &Loaded<'_>) -> Result<Vec<Reference>, DocsError> {
        let View::Visible { version, .. } = loaded.snippet.view(self.lens) else {
            return Ok(Vec::new());
        };
        _render::dependency_reads(version.body(), self.lens, &self.context(loaded))
    }

    /// The published index of the position `domain` addresses; `None` when
    /// it has none.
    fn domain_index(&self, domain: &DomainName) -> Result<Option<PositionIndex>, DocsError> {
        let rel = Path::new(domain.as_str()).join(INDEX_FILE);
        let docs = self.home.docs_dir();
        let Some(text) = read_confined(&docs, &rel, outside_home)? else {
            return Ok(None);
        };
        PositionIndex::parse(&Position::of_domain(domain), &text)
            .map(Some)
            .map_err(|source| DocsError::PositionIndexInvalid {
                path: docs.join(rel),
                source,
            })
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
        Ok(Graph {
            items,
            left_out: walk.left_out,
        })
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
            // What a read that cannot be followed becomes: refused, left out
            // and named, or named as not loaded.
            let mut cannot = |reason: &str, items: &mut Vec<Reached<'i>>| match strictness {
                Strictness::Refuse => Err(refused(reason)),
                Strictness::Skip => {
                    walk.left_out.push(format!("{} ({reason})", read.raw));
                    Ok(())
                }
                Strictness::List => {
                    if walk.unreadable.insert(read.raw.clone()) {
                        items.push(Reached::Unreadable(read.raw.clone()));
                    }
                    Ok(())
                }
            };
            match &read.target {
                Some(RefTarget::Snippet(id)) => {
                    if walk.snippets.contains(id) {
                        continue;
                    }
                    let Some(entry) = self.index.snippets.iter().find(|e| &e.id == id) else {
                        cannot("names no published snippet", items)?;
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
                        ) => {
                            cannot(&format!("cannot be read: {err}"), items)?;
                            continue;
                        }
                        Err(err) => return Err(err),
                    };
                    if loaded.snippet.is_pending() && strictness != Strictness::List {
                        cannot("is still pending", items)?;
                        continue;
                    }
                    // A withheld snippet names no reads (`reads_of`).
                    let nested = self.reads_of(&loaded)?;
                    items.push(Reached::Snippet(Box::new(loaded)));
                    self.visit(nested, strictness, root, walk, items)?;
                }
                Some(RefTarget::Domain(domain)) => {
                    if walk.domains.contains(domain) {
                        continue;
                    }
                    if strictness == Strictness::List {
                        // Read on its own with `docs get --domain`.
                        walk.domains.insert(domain.clone());
                        items.push(Reached::Domain(domain.clone()));
                        continue;
                    }
                    let Some(selection) = self.domain_index(domain)? else {
                        cannot(&format!("names a domain without {INDEX_FILE}"), items)?;
                        continue;
                    };
                    walk.domains.insert(domain.clone());
                    items.push(Reached::Domain(domain.clone()));
                    let expanded = selection
                        .expanded()
                        .iter()
                        .map(|id| Reference {
                            raw: id.as_str().to_owned(),
                            target: Some(RefTarget::Snippet(id.clone())),
                        })
                        .collect();
                    self.visit(expanded, strictness, root, walk, items)?;
                }
                // A path reference or a malformed entry is validation's
                // finding; it names nothing a read can follow.
                Some(RefTarget::Bundle(_) | RefTarget::LocalFile(_)) | None => {
                    cannot("names no snippet or domain", items)?;
                }
            }
        }
        Ok(())
    }

    /// Expands `ids` into `into` in rendering order: each snippet after its
    /// transitive required reads, each once, a bare-domain read standing
    /// for that domain's `expanded` set. A read that is pending or cannot
    /// be read is left out and named.
    fn expand(&self, ids: &[SnippetId], into: &mut Expansion<'i>) -> Result<(), DocsError> {
        for id in ids {
            if !into.seen.insert(id.clone()) {
                continue;
            }
            let Some(entry) = self.index.snippets.iter().find(|e| &e.id == id) else {
                into.left_out
                    .push(format!("{id} (names no published snippet)"));
                continue;
            };
            let loaded = match self.load(entry) {
                Ok(loaded) => loaded,
                Err(
                    err @ (DocsError::MissingLocalFile { .. }
                    | DocsError::MissingPublishedFile { .. }),
                ) => {
                    into.left_out.push(format!("{id} (cannot be read: {err})"));
                    continue;
                }
                Err(err) => return Err(err),
            };
            if loaded.snippet.is_pending() {
                into.left_out.push(format!("{id} (pending)"));
                continue;
            }
            for read in self.reads_of(&loaded)? {
                match &read.target {
                    Some(RefTarget::Snippet(nested)) => {
                        self.expand(std::slice::from_ref(nested), into)?;
                    }
                    Some(RefTarget::Domain(domain)) => {
                        if !into.domains.insert(domain.clone()) {
                            continue;
                        }
                        match self.domain_index(domain)? {
                            Some(selection) => self.expand(selection.expanded(), into)?,
                            None => into.left_out.push(format!("{domain} (no {INDEX_FILE})")),
                        }
                    }
                    Some(RefTarget::Bundle(_) | RefTarget::LocalFile(_)) | None => {
                        into.left_out
                            .push(format!("{} (names no snippet or domain)", read.raw));
                    }
                }
            }
            into.order.push(loaded);
        }
        Ok(())
    }

    fn context<'c>(&'c self, loaded: &'c Loaded<'_>) -> RenderContext<'c> {
        RenderContext {
            home: self.home,
            values: &self.values,
            project: loaded.project(),
        }
    }

    /// Where `entry`'s snippet is authored: its file in the checkout its
    /// owner published from, or in its project.
    fn source(&self, entry: &IndexEntry) -> Result<String, DocsError> {
        if let Some(target) = self.target
            && target.project == entry.source_project
        {
            let root = &target.root;
            let rel = match entry.scope {
                IndexScope::Global => global_rel(entry)?,
                IndexScope::Local => local_root(entry)?.1,
            };
            return Ok(root.join(rel).display().to_string());
        }
        Ok(match entry.scope {
            IndexScope::Local => entry.path.display().to_string(),
            IndexScope::Global => {
                let record = PublishedRecord::load(self.home, &entry.source_project)?;
                match (record, published_source(&entry.id)) {
                    (Some(record), Some(rel)) => record.checkout.join(rel).display().to_string(),
                    _ => NOT_RECORDED.to_owned(),
                }
            }
        })
    }

    /// One snippet's block: its header, then its body through the read
    /// pipeline, or the notice that replaces it; whether a configured value
    /// could not be resolved.
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
            format!("source: {}", self.source(loaded.entry)?),
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
                failed = !rendered.config_refusals.is_empty();
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
            home: self.home,
            values: &self.values,
            project: None,
        };
        let rendered = _render::render(&text, Lens::Current, &cx)?;
        // third-party-docs spec: `global` for the docs of the project that
        // owns `docs/general/`, `project` for any other project's. The owner
        // is the position's claimant, whether or not it has snippets.
        let owns_general = match DomainName::parse(GENERAL) {
            Ok(general) => crate::publication::domain_owner(self.home, &general)?
                .is_some_and(|owner| owner == entry.source_project),
            Err(_) => false,
        };
        let source = PublishedRecord::load(self.home, &entry.source_project)?.map_or_else(
            || NOT_RECORDED.to_owned(),
            |record| {
                record
                    .checkout
                    .join("docs")
                    .join(&entry.path)
                    .display()
                    .to_string()
            },
        );
        let block = [
            "---".to_owned(),
            format!("id: {}", entry.id.as_str()),
            format!("scope: {}", if owns_general { "global" } else { "project" }),
            format!("description: {}", entry.description),
            format!("source: {source}"),
            "---".to_owned(),
            rendered.text.trim_matches('\n').to_owned(),
        ];
        Ok((block.join("\n"), !rendered.config_refusals.is_empty()))
    }
}

/// The source a header names when publication recorded no checkout for
/// the snippet's owner.
const NOT_RECORDED: &str = "(not recorded)";

fn outside_home(path: PathBuf) -> DocsError {
    DocsError::OutsideHome { path }
}

fn outside_repository(path: PathBuf) -> DocsError {
    DocsError::OutsideRepository { path }
}

/// A published entry's file as the repository path it was published from
/// (`docs/general/…`, `docs/domain/<x>/…`), once the path the entry carries
/// relative to the home's docs directory is its id's own path.
fn global_rel(entry: &IndexEntry) -> Result<String, DocsError> {
    let parts: Option<Vec<&str>> = entry
        .path
        .components()
        .map(|c| match c {
            Component::Normal(part) => part.to_str(),
            _ => None,
        })
        .collect();
    let at_identity = parts.is_some_and(|p| p.join("/") == format!("{}.md", entry.id.as_str()));
    match published_source(&entry.id) {
        Some(rel) if at_identity => Ok(rel),
        _ => Err(DocsError::IndexMismatch {
            id: entry.id.clone(),
            path: entry.path.clone(),
        }),
    }
}

/// A local entry's project root and the file's repository path
/// (`docs/project/<taxonomy>/<rest>.md`), from the absolute path the entry
/// carries.
fn local_root(entry: &IndexEntry) -> Result<(PathBuf, String), DocsError> {
    let mismatch = || DocsError::IndexMismatch {
        id: entry.id.clone(),
        path: entry.path.clone(),
    };
    let prefix = format!("project/{}/", entry.source_project.as_str());
    let rest = entry
        .id
        .as_str()
        .strip_prefix(&prefix)
        .ok_or_else(mismatch)?;
    let rel = format!("docs/project/{rest}.md");
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
    let (source, main) = if home.is_development() {
        ("development HEAD", git::run(root, ["rev-parse", "HEAD"])?)
    } else {
        let branch = git::default_branch(root, "origin")?;
        let reference = format!("refs/heads/{branch}");
        (
            "main",
            git::run(
                root,
                ["for-each-ref", "--format=%(objectname)", reference.as_str()],
            )?,
        )
    };
    if main.is_empty() || main == record.commit.as_str() {
        return Ok(None);
    }
    let repair = if home.is_development() {
        "scripts/dev-instrukt sync"
    } else {
        "instrukt sync"
    };
    Ok(Some(format!(
        "# WARNING: publication is behind {source}: {source} is at {main}, the published commit is {}. \
         Run `{repair}` in this project to publish it.",
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
            subscription_home: None,
            subscription_identity: Default::default(),
            subscription_attendance: Default::default(),
            required_reads: Vec::new(),
            parameters: Vec::new(),
            interruptible: false,
        }
    }

    #[test]
    fn a_global_entry_is_read_only_at_its_identity_below_the_home() {
        let id = "acme/spec/a";
        let placed = entry(id, IndexScope::Global, &format!("{id}.md"));
        assert_eq!(
            global_rel(&placed).unwrap(),
            format!("docs/domain/{}", placed.path.display())
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
        let id = "project/demo/spec/feature/a";
        let rel = "docs/project/spec/feature/a.md";
        let root = PathBuf::from("/work/demo");
        let placed = entry(id, IndexScope::Local, &root.join(rel).display().to_string());
        assert_eq!(local_root(&placed).unwrap(), (root, rel.to_owned()));
        for path in [
            "docs/project/spec/feature/a.md",
            "/work/demo/docs/project/spec/feature/b.md",
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
            "project/other/spec/a",
            IndexScope::Local,
            "/x/docs/project/spec/a.md",
        );
        assert!(matches!(
            local_root(&other),
            Err(DocsError::IndexMismatch { .. })
        ));
    }
}
