//! Publication from canonical main, or an explicit development worktree HEAD.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::_agents::{MergeRefusal, merge_key, targets};
use super::_editor::{EditorChange, EditorChangeKind, write_editor_schemas};
use super::_fingerprint;
use super::_patch::{Claim, Contribution, patch};
use super::_render::Renderer;
use super::_source::{self, Committed, Project};
use super::_tree::{self, ClaimRecord, ProjectClaimRecord};
use super::_tx::{LinkOutcome, STAGING_PREFIX, Transaction, destination, own_destination, sweep};
use super::{PublicationError, PublishedRecord, record_path};
use crate::config::{self, Configuration, Layer, Module};
use crate::docs::{
    self, Corpus, Finding, INDEX_FILE, Notice, NoticeCode, Position, PositionIndex,
    ValidationRequest, Values,
};
use crate::fsx::LockGuard;
use crate::git;
use crate::home::Home;
use crate::ids::{CommitSha, DomainName, ProjectSlug};
use crate::index::GlobalIndex;
use crate::interfaces::{Answer, CoreWiring};

/// How long a sync waits for another sync to release the docs lock.
const DOCS_LOCK_TIMEOUT: Duration = Duration::from_secs(300);

/// The docs lock's file name below the home's lock directory.
const DOCS_LOCK: &str = "docs.lock";

/// What one sync is asked to do.
#[derive(Clone, Copy)]
pub struct SyncRequest<'a> {
    pub home: &'a Home,
    /// The caller's working directory: the project it stands in is the one
    /// published.
    pub cwd: &'a Path,
    /// The optional-module implementations available to this invocation.
    /// Publication applies those the committed configuration it publishes
    /// switches on (validation reads them); the caller's working tree
    /// decides nothing.
    pub wiring: &'a CoreWiring,
    /// The invocation's clock reading: when this publication is recorded.
    pub now: jiff::Timestamp,
}

/// Why a sync published nothing although it could run.
#[derive(Debug, Clone, PartialEq)]
pub enum Refusal {
    /// The committed documentation has structural validation findings.
    Findings(Vec<Finding>),
    /// The project claims an id or a position another project owns.
    Claim(Claim),
    /// The committed tree holds a domain folder with a reserved name, by
    /// its repository directory (`docs/domain/general/`).
    ReservedDomain(Vec<String>),
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Refusal::Findings(findings) => write!(
                f,
                "the committed documentation has {} validation finding(s)",
                findings.len()
            ),
            Refusal::Claim(claim) => claim.fmt(f),
            Refusal::ReservedDomain(dirs) => {
                let named: Vec<String> = dirs.iter().map(|dir| format!("`{dir}`")).collect();
                let verb = if named.len() == 1 { "is" } else { "are" };
                write!(
                    f,
                    "{} {verb} named after a reserved name: `general` and `project` are reserved and `third-party` keeps its special case, so no domain may take them",
                    named.join(" and ")
                )
            }
        }
    }
}

/// What a sync did.
#[derive(Debug, Clone, PartialEq)]
pub enum SyncOutcome {
    /// The commit is now the project's published commit.
    Published {
        project: ProjectSlug,
        commit: CommitSha,
    },
    /// The selected source is published and every output was already
    /// current: nothing was written.
    Unchanged {
        project: ProjectSlug,
        commit: CommitSha,
    },
    /// Nothing changed and no commit was recorded.
    Refused {
        project: ProjectSlug,
        commit: CommitSha,
        refusal: Refusal,
    },
}

/// What a sync did, and the leftovers it could not remove.
#[derive(Debug, Clone, PartialEq)]
pub struct SyncReport {
    pub outcome: SyncOutcome,
    /// Copies of replaced or interrupted publications still in the home:
    /// every sync removes those it can, and a later sync tries the rest
    /// again. They are not part of the published corpus.
    pub leftovers: Vec<PathBuf>,
    /// What a publishing sync changed of the editor schemas
    /// in the primary worktree.
    pub editor: Vec<EditorChange>,
    /// What a publishing sync reports without refusing: a domain without
    /// `index.yaml`, a root master without `docs/project/index.yaml`.
    pub notices: Vec<Notice>,
    /// Each global agent file a publishing sync wrote or found current.
    pub global_files: Vec<GlobalDestination>,
    /// Each link to a global agent file a publishing sync ensured in a
    /// folder an instance reads through.
    pub links: Vec<EnsuredLink>,
    /// Each symbolic link that stood at one of publication's own
    /// destinations and was replaced by the file or directory.
    pub replaced_links: Vec<PathBuf>,
    /// What a publishing sync found about the agent files without
    /// refusing: settings left untouched, references a global file keeps.
    pub findings: Vec<SyncFinding>,
}

/// One global agent file a sync wrote or found current.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlobalDestination {
    pub path: PathBuf,
    /// The agent runtime that reads it.
    pub runtime: String,
    /// The configured instances it serves.
    pub instances: Vec<String>,
    /// Whether this sync wrote it; `false` when it already held the
    /// rendering.
    pub written: bool,
}

/// One link to a global agent file a sync ensured.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnsuredLink {
    pub path: PathBuf,
    /// The global agent file it points at.
    pub target: PathBuf,
    pub state: LinkState,
}

/// What a sync found where a link to a global agent file must exist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkState {
    /// Nothing stood there; the link was made.
    Made,
    /// A file, or a link pointing elsewhere, stood there and was replaced.
    Repaired,
    /// The link already pointed at the file.
    Current,
}

/// What a sync reports about an agent file without refusing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncFinding {
    /// A runtime's settings file left untouched: the user's, unparseable or
    /// a symbolic link. Its instruction file is still written.
    SettingsLeft { path: PathBuf, reason: String },
    /// A global agent file written with a reference left for an agent
    /// program to resolve.
    ReferenceKept { path: PathBuf, reference: String },
    /// Per-instance target availability, non-delivery and receipt limitations.
    InstructionTarget { target: super::InstructionTarget },
    /// The selected runtime could not be asked which agent runtimes are
    /// configured: no global agent file was written in this sync.
    RuntimeUnreachable { runtime: String, cause: String },
}

/// A sync that could not run to its end, with the leftovers it found and
/// could not remove: they are reported on failure as on success.
#[derive(Debug, thiserror::Error)]
#[error("{error}")]
pub struct SyncError {
    #[source]
    pub error: Box<PublicationError>,
    /// As [`SyncReport::leftovers`].
    pub leftovers: Vec<PathBuf>,
}

impl From<PublicationError> for SyncError {
    fn from(error: PublicationError) -> Self {
        Self {
            error: Box::new(error),
            leftovers: Vec::new(),
        }
    }
}

/// Publishes the project `request.cwd` stands in from the committed state
/// of canonical main in installed use, or its own committed HEAD in explicit
/// development execution, under the docs lock. A refused or failed sync
/// leaves the previous publication in place and records no commit. Every
/// sync, the no-op included, first removes the leftovers earlier syncs
/// could not, and reports those it still cannot whatever its outcome.
pub fn sync(request: &SyncRequest<'_>) -> Result<SyncReport, SyncError> {
    let started = Instant::now();
    config::migrate(request.home, &Layer::Global).map_err(PublicationError::from)?;
    let mut project = _source::locate(request.cwd)?;
    if request.home.is_development() {
        project.primary = project.worktree.clone();
    }
    config::bring_current(request.home, &Layer::Project(project.worktree.clone()))
        .map_err(PublicationError::from)?;
    let _lock = LockGuard::acquire(&request.home.locks_dir().join(DOCS_LOCK), DOCS_LOCK_TIMEOUT)
        .map_err(PublicationError::from)?;
    let mut leftovers = sweep(request.home.root())?;
    let mut editor = Vec::new();
    let mut notices = Vec::new();
    let mut global_files = Vec::new();
    let mut links = Vec::new();
    let mut replaced_links = Vec::new();
    let mut findings = Vec::new();
    let attempt = (|| {
        let (branch, commit) = if request.home.is_development() {
            let branch = git::run(&project.worktree, ["rev-parse", "--abbrev-ref", "HEAD"])?;
            let sha = git::run(&project.worktree, ["rev-parse", "HEAD"])?;
            let commit =
                CommitSha::parse(sha.trim()).map_err(|_| PublicationError::MainHasNoCommit {
                    dir: project.worktree.clone(),
                    branch: branch.clone(),
                })?;
            (branch, commit)
        } else {
            _source::canonical_main(&project.primary)?
        };
        // Every sync converges: it validates, renders and compares, and
        // writes only what is missing or different, whether or not main
        // moved since the published commit.
        let mut report = Reported {
            leftovers: &mut leftovers,
            editor: &mut editor,
            notices: &mut notices,
            global_files: &mut global_files,
            links: &mut links,
            replaced_links: &mut replaced_links,
            findings: &mut findings,
        };
        let outcome = publish(request, &project, &commit, &mut report)?;
        Ok::<_, PublicationError>((branch, commit, outcome))
    })();
    let (branch, commit, outcome) = match attempt {
        Ok(done) => done,
        Err(error) => {
            return Err(SyncError {
                error: Box::new(error),
                leftovers,
            });
        }
    };
    let label = match &outcome {
        SyncOutcome::Published { .. } => "published",
        SyncOutcome::Unchanged { .. } => "unchanged",
        SyncOutcome::Refused { .. } => "refused",
    };
    if matches!(outcome, SyncOutcome::Published { .. }) {
        tracing::info!(event = "docs_published", project = %project.slug, commit = %commit);
    }
    tracing::debug!(
        event = "sync_completed",
        project = %project.slug,
        branch = %branch,
        commit = %commit,
        outcome = label,
        leftovers = leftovers.len(),
        duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
    );
    Ok(SyncReport {
        outcome,
        leftovers,
        editor,
        notices,
        global_files,
        links,
        replaced_links,
        findings,
    })
}

/// What a publishing sync reports besides its outcome, filled as it runs.
struct Reported<'r> {
    leftovers: &'r mut Vec<PathBuf>,
    editor: &'r mut Vec<EditorChange>,
    notices: &'r mut Vec<Notice>,
    global_files: &'r mut Vec<GlobalDestination>,
    links: &'r mut Vec<EnsuredLink>,
    replaced_links: &'r mut Vec<PathBuf>,
    findings: &'r mut Vec<SyncFinding>,
}

/// Everything a sync computed before it writes anything.
struct Prepared {
    committed: Committed,
    /// The published positions the committed tree holds, by the domain
    /// name each is addressed by.
    held: Vec<DomainName>,
    /// The index as it was, and as this sync leaves it.
    index: GlobalIndex,
    patched: GlobalIndex,
    /// Every claimed position's owner before this sync.
    owners: BTreeMap<DomainName, ProjectSlug>,
    /// Global agent files, each with where it is written.
    global: Vec<GlobalWrite>,
    /// Merged settings files, by where each is written.
    settings: Vec<(PathBuf, Vec<u8>)>,
    /// Links to global agent files, each `(where it stands, what it points
    /// at)`.
    links: Vec<(PathBuf, PathBuf)>,
}

/// One global agent file a sync writes.
struct GlobalWrite {
    /// Where it is written: its home's folders resolved through their links.
    at: PathBuf,
    /// Its path as the runtime's home names it.
    path: PathBuf,
    runtime: String,
    instances: Vec<String>,
    text: String,
}

fn publish(
    request: &SyncRequest<'_>,
    project: &Project,
    commit: &CommitSha,
    report: &mut Reported<'_>,
) -> Result<SyncOutcome, PublicationError> {
    let home = request.home;
    config::migrate(home, &Layer::Global)?;
    config::bring_current(home, &Layer::Project(project.worktree.clone()))?;
    let committed = Committed::read(&project.primary, commit)?;
    let checkout = crate::fsx::temp_dir(tempfile::Builder::new().prefix("instrukt-sync-"))
        .map_err(|source| PublicationError::Io {
            path: std::env::temp_dir(),
            source,
        })?;
    committed.materialize(checkout.path())?;
    // Slug reads migrate the temporary committed layer before consuming it.
    let committed_slug = config::project_slug(checkout.path())?;
    if committed_slug.as_ref() != Some(&project.slug) {
        return Err(PublicationError::SlugMismatch {
            committed: committed_slug.map(|s| s.to_string()).unwrap_or_default(),
            working: project.slug.clone(),
        });
    }
    let refused = |refusal| SyncOutcome::Refused {
        project: project.slug.clone(),
        commit: commit.clone(),
        refusal,
    };
    let (held, reserved) = _tree::held_positions(&committed);
    if !reserved.is_empty() {
        return Ok(refused(Refusal::ReservedDomain(reserved)));
    }
    // Whether `todos` is on is the committed configuration's to say.
    let todos = Configuration::load(home, Some(checkout.path()))?.module_enabled(Module::Todos)?;
    let wiring = CoreWiring {
        todo_existence: request.wiring.todo_existence.clone().filter(|_| todos),
        flavor_roles: request.wiring.flavor_roles.clone().filter(|_| todos),
        ..request.wiring.clone()
    };
    let validation = docs::validate(&ValidationRequest {
        repo_root: checkout.path(),
        home,
        paths: &[],
        wiring: &wiring,
    })?;
    if !validation.findings.is_empty() {
        return Ok(refused(Refusal::Findings(validation.findings)));
    }
    // Sync reports the notices about what it renders; the rest are
    // `instrukt docs validate`'s.
    let mut notices: Vec<Notice> = validation
        .notices
        .into_iter()
        .filter(|n| n.code == NoticeCode::DomainWithoutIndex)
        .collect();
    let config = Configuration::load(home, Some(checkout.path()))?;
    drop(checkout);
    let corpus = Corpus::from_sources(
        Some(project.slug.clone()),
        Some(home),
        committed.doc_sources(),
    );
    let contribution = Contribution::of(&project.slug, &corpus, &held);
    if let Some(claim) = ProjectClaimRecord::load(home, &project.slug)?
        && claim.owner != project.slug
    {
        return Ok(refused(Refusal::Claim(Claim::ProjectOwned {
            project: project.slug.clone(),
            owner: claim.owner,
        })));
    }
    let owners = _tree::claims(home)?;
    let index = GlobalIndex::load(home)?.unwrap_or_default();
    let patched = match patch(&index, &owners, &contribution) {
        Ok(patched) => patched,
        Err(claim) => return Ok(refused(Refusal::Claim(claim))),
    };

    // Rendering reads the committed tree and the home as it stands; it
    // writes nothing, so a failure here leaves everything as it was.
    let owned: BTreeSet<DomainName> = held.iter().cloned().collect();
    let values = Values::publish(&config, wiring.caller_identity.as_deref());
    let indexes = position_indexes(home, &committed, &held, &owners, &project.slug)?;
    let renderer = Renderer {
        home,
        project: &project.slug,
        corpus: &corpus,
        owned: &owned,
        index: &patched,
        indexes: &indexes,
        values: &values,
    };
    let GlobalPlan {
        global,
        settings,
        links,
        findings,
    } = if !home.is_development()
        && held
            .iter()
            .any(|d| Position::of_domain(d) == Position::General)
    {
        let answer = wiring
            .configured_runtimes
            .as_ref()
            .map(|runtimes| runtimes.configured(home, &config::Env::from_process()));
        global_files(home, &renderer, answer)?
    } else {
        GlobalPlan::default()
    };
    *report.findings = findings;
    notices.sort();
    notices.dedup();
    *report.notices = notices;

    let prepared = Prepared {
        committed,
        held,
        index,
        patched,
        owners,
        global,
        settings,
        links,
    };
    preflight(home, project, commit, &prepared)?;
    // Coordinate only publication application with checkout rendering. Validation
    // and preparation retain the separate docs lock, so a checkout inside a
    // pull/rebase never waits for that long-lived preparation phase.
    let _outputs = lock_outputs(home)?;
    let mut tx = Transaction::begin(home.root())?;
    // The editor schemas are the transaction's first steps: a refresh that
    // cannot write, or any later failure, puts them back as they were, and
    // no commit is recorded. The links they replaced are named with them.
    let schemas = if home.is_development() {
        Ok(Vec::new())
    } else {
        write_editor_schemas(&project.primary, &mut tx)
    };
    let schema_links = tx.replaced_links().len();
    let written = schemas.and_then(|schemas| {
        *report.editor = schemas;
        write(home, project, commit, request.now, &prepared, &mut tx)
    });
    match written {
        Ok((global_files, links)) => {
            let changed = tx.changed()
                || report
                    .editor
                    .iter()
                    .any(|change| change.kind != EditorChangeKind::Linked);
            *report.replaced_links = tx.replaced_links()[schema_links..].to_vec();
            report.leftovers.extend(tx.commit());
            *report.global_files = global_files;
            *report.links = links;
            let (project, commit) = (project.slug.clone(), commit.clone());
            Ok(if changed {
                SyncOutcome::Published { project, commit }
            } else {
                SyncOutcome::Unchanged { project, commit }
            })
        }
        Err(err) => {
            tx.rollback();
            Err(err)
        }
    }
}

/// Serialize live publication state and checkout output writes as one snapshot.
pub(super) fn lock_outputs(home: &Home) -> Result<LockGuard, PublicationError> {
    Ok(LockGuard::acquire(
        &home.locks_dir().join("publication-outputs.lock"),
        DOCS_LOCK_TIMEOUT,
    )?)
}

/// Checks every location the sync will write or remove, before the first
/// write. A location the repository or instrukt controls — a rendered
/// output in the project or one recorded for retraction; in the home a
/// published domain, a third-party doc, the index, the eval
/// fingerprint records, the outputs record or the published record — that
/// lies behind a symbolic link refuses the sync; a link at the location
/// itself is replaced by the write. An agent-CLI file in the user's home
/// whose directory's link leads nowhere refuses too. The writes check again
/// as a backstop.
fn preflight(
    home: &Home,
    project: &Project,
    commit: &CommitSha,
    prepared: &Prepared,
) -> Result<(), PublicationError> {
    let root = home.root();
    let mine = |owner: &ProjectSlug| owner == &project.slug;
    let claimed: Vec<&DomainName> = prepared
        .held
        .iter()
        .chain(
            prepared
                .owners
                .iter()
                .filter(|(_, owner)| mine(owner))
                .map(|(domain, _)| domain),
        )
        .collect();
    let mut targets: Vec<PathBuf> = claimed.iter().map(|d| home.domain_dir(d)).collect();
    targets.extend(claimed.iter().map(|d| _tree::claim_path(home, d)));
    let docs = home.docs_dir();
    for index in [&prepared.index, &prepared.patched] {
        targets.extend(
            index
                .third_party
                .iter()
                .filter(|d| mine(&d.source_project))
                .map(|d| docs.join(&d.path)),
        );
    }
    targets.push(home.project_docs_dir(&project.slug));
    targets.push(_tree::project_claim_path(home, &project.slug));
    targets.push(home.index_file());
    targets.extend(_fingerprint::record_paths(home, commit));
    targets.push(record_path(home, &project.slug));
    for target in targets {
        own_destination(root, &relative_to(root, &target)?)?;
    }
    Ok(())
}

/// `(path, content)` pairs of files a sync writes, relative to a root.
type Files<T> = Vec<(PathBuf, T)>;

/// Every published position's `index.yaml` as this sync leaves the
/// computer: the committed ones of the positions the project holds, the
/// published ones of the positions other projects own.
pub(super) fn position_indexes(
    home: &Home,
    committed: &Committed,
    held: &[DomainName],
    owners: &BTreeMap<DomainName, ProjectSlug>,
    project: &ProjectSlug,
) -> Result<BTreeMap<DomainName, PositionIndex>, PublicationError> {
    let mut indexes = BTreeMap::new();
    for (domain, owner) in owners {
        if owner == project {
            continue;
        }
        let position = Position::of_domain(domain);
        let path = home.domain_dir(domain).join(INDEX_FILE);
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
            Err(source) => return Err(PublicationError::Io { path, source }),
        };
        let index = PositionIndex::parse(&position, &text)
            .map_err(|source| PublicationError::Parse { path, source })?;
        indexes.insert(domain.clone(), index);
    }
    for domain in held {
        if let Some(index) = committed_index(committed, &Position::of_domain(domain))? {
            indexes.insert(domain.clone(), index);
        }
    }
    Ok(indexes)
}

/// The committed `index.yaml` of `position`; `None` without one.
pub(super) fn committed_index(
    committed: &Committed,
    position: &Position,
) -> Result<Option<PositionIndex>, PublicationError> {
    let rel = format!("{}/{INDEX_FILE}", position.dir());
    let Some(bytes) = committed.file(&rel) else {
        return Ok(None);
    };
    PositionIndex::parse(position, &String::from_utf8_lossy(bytes))
        .map(Some)
        .map_err(|source| PublicationError::Parse {
            path: PathBuf::from(rel),
            source,
        })
}

/// What a publishing sync writes for the agent programs installed here, and
/// what it reports about them without refusing.
#[derive(Default)]
struct GlobalPlan {
    global: Vec<GlobalWrite>,
    settings: Files<Vec<u8>>,
    links: Vec<(PathBuf, PathBuf)>,
    findings: Vec<SyncFinding>,
}

/// The global agent files of the agent runtimes the selected runtime
/// reports as configured, with the settings files that take their
/// entrypoint-loading keys and the links its instances read through
/// (`project/instrukt/spec/feature/publication/agent-entrypoints`, "Global
/// agent files"). Without a runtime nothing is written; one that cannot be
/// asked writes nothing this sync and is named. Each file is written whole
/// whatever it holds; one that keeps a reference for an agent program to
/// resolve is written as rendered and named. A settings file that cannot
/// take the key, or is a symbolic link, is the user's: it is left untouched
/// and named, and the instruction file is still written.
fn global_files(
    _home: &Home,
    renderer: &Renderer<'_>,
    answer: Option<Answer>,
) -> Result<GlobalPlan, PublicationError> {
    let mut plan = GlobalPlan::default();
    let instances = match answer {
        None => return Ok(plan),
        Some(Answer::Unreachable { runtime, cause }) => {
            plan.findings
                .push(SyncFinding::RuntimeUnreachable { runtime, cause });
            return Ok(plan);
        }
        Some(Answer::Reported(instances)) => instances,
    };
    let targets = targets(&instances)?;
    plan.findings.extend(
        targets
            .instances
            .iter()
            .filter(|target| {
                target.reason.is_some()
                    || target.home_source == crate::interfaces::InstructionHomeSource::Default
                    || !target.assurance.is_empty()
            })
            .cloned()
            .map(|target| SyncFinding::InstructionTarget { target }),
    );
    // One rendering per runtime, with the references it keeps: every file
    // written from it is named with them.
    let mut rendered = BTreeMap::new();
    for file in &targets.files {
        let runtime = file.destination;
        let global = match rendered.entry(file.runtime.clone()) {
            std::collections::btree_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(renderer.global_file(&file.runtime)?)
            }
        };
        plan.findings
            .extend(
                global
                    .references
                    .iter()
                    .map(|reference| SyncFinding::ReferenceKept {
                        path: file.path(),
                        reference: reference.clone(),
                    }),
            );
        let text = if runtime.cli == "cursor" {
            format!(
                "---\ndescription: Instrukt doctrine\nglobs: \"\"\nalwaysApply: true\n---\n\n{}",
                global.text
            )
        } else {
            global.text.clone()
        };
        plan.global.push(GlobalWrite {
            at: destination(&file.home, Path::new(runtime.file))?,
            path: file.path(),
            runtime: file.runtime.clone(),
            instances: file.instances.clone(),
            text,
        });
        let Some(loading) = runtime.loading else {
            continue;
        };
        let shown = file.home.join(loading.settings);
        let path = destination(&file.home, Path::new(loading.settings))?;
        let left = |reason: &str| SyncFinding::SettingsLeft {
            path: shown.clone(),
            reason: reason.to_owned(),
        };
        let existing = match std::fs::symlink_metadata(&path) {
            Ok(meta) if meta.file_type().is_symlink() => {
                plan.findings.push(left(
                    "it is a symbolic link, and nothing is written through a link",
                ));
                continue;
            }
            Ok(_) => Some(std::fs::read(&path).map_err(|source| PublicationError::Io {
                path: path.clone(),
                source,
            })?),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
            Err(source) => return Err(PublicationError::Io { path, source }),
        };
        match merge_key(existing.as_deref(), loading.key, (loading.value)()) {
            Ok(Some(merged)) => plan.settings.push((path, merged)),
            Ok(None) => {}
            Err(MergeRefusal::NotAnObject) => {
                plan.findings.push(left("it is not a JSON object"));
            }
            Err(MergeRefusal::Blocked { member }) => {
                plan.findings
                    .push(left(&format!("its `{member}` is not an object")));
            }
        }
    }
    for link in &targets.links {
        let folder = link.link.parent().unwrap_or(Path::new("/"));
        let name = link
            .link
            .file_name()
            .map(Path::new)
            .unwrap_or(Path::new(""));
        plan.links
            .push((destination(folder, name)?, link.target.clone()));
    }
    Ok(plan)
}

/// The writes of a publishing sync, all through `tx`.
fn write(
    home: &Home,
    project: &Project,
    commit: &CommitSha,
    now: jiff::Timestamp,
    prepared: &Prepared,
    tx: &mut Transaction,
) -> Result<(Vec<GlobalDestination>, Vec<EnsuredLink>), PublicationError> {
    let root = home.root();
    let in_home = |path: &Path| own_destination(root, &relative_to(root, path)?);
    let staging = crate::fsx::temp_dir_in(tempfile::Builder::new().prefix(STAGING_PREFIX), root)
        .map_err(|source| PublicationError::Io {
            path: root.to_path_buf(),
            source,
        })?;
    // Positions: staged whole, then swapped in, each claimed in the
    // ownership record.
    let published: BTreeSet<&DomainName> = prepared.held.iter().collect();
    for domain in &prepared.held {
        let staged = staging.path().join(domain.as_str());
        _tree::stage_position(&prepared.committed, &Position::of_domain(domain), &staged)?;
        tx.replace_dir(&staged, &in_home(&home.domain_dir(domain))?)?;
        let claim = ClaimRecord {
            position: domain.clone(),
            owner: project.slug.clone(),
        };
        let text = serde_yaml_ng::to_string(&claim).map_err(PublicationError::Render)?;
        tx.write(&in_home(&_tree::claim_path(home, domain))?, text.as_bytes())?;
    }
    for (domain, owner) in &prepared.owners {
        if owner == &project.slug && !published.contains(domain) {
            tx.remove_dir(&in_home(&home.domain_dir(domain))?)?;
            tx.remove_file(&in_home(&_tree::claim_path(home, domain))?)?;
        }
    }
    // Project positions use the same transaction and whole-tree replacement.
    let project_dir = in_home(&home.project_docs_dir(&project.slug))?;
    let claim_path = in_home(&_tree::project_claim_path(home, &project.slug))?;
    if prepared.committed.below("docs/project/").next().is_some() {
        let staged = staging.path().join("project").join(project.slug.as_str());
        _tree::stage_position(&prepared.committed, &Position::Project, &staged)?;
        tx.replace_dir(&staged, &project_dir)?;
        let claim = ProjectClaimRecord {
            project: project.slug.clone(),
            owner: project.slug.clone(),
        };
        let text = serde_yaml_ng::to_string(&claim).map_err(PublicationError::Render)?;
        tx.write(&claim_path, text.as_bytes())?;
    } else {
        tx.remove_dir(&project_dir)?;
        tx.remove_file(&claim_path)?;
    }
    // Third-party docs: file by file, owner-scoped.
    let docs = home.docs_dir();
    let mine = |owner: &ProjectSlug| owner == &project.slug;
    for doc in prepared
        .patched
        .third_party
        .iter()
        .filter(|d| mine(&d.source_project))
    {
        let source = Path::new("docs").join(&doc.path);
        if let Some(bytes) = prepared.committed.file(&source.to_string_lossy()) {
            tx.write(&in_home(&docs.join(&doc.path))?, bytes)?;
        }
    }
    let third_party: BTreeSet<&PathBuf> = prepared
        .patched
        .third_party
        .iter()
        .map(|d| &d.path)
        .collect();
    for old in prepared
        .index
        .third_party
        .iter()
        .filter(|d| mine(&d.source_project))
    {
        if !third_party.contains(&old.path) {
            tx.remove_file(&in_home(&docs.join(&old.path))?)?;
        }
    }
    let index_text =
        serde_yaml_ng::to_string(&prepared.patched).map_err(PublicationError::Render)?;
    tx.write(&in_home(&home.index_file())?, index_text.as_bytes())?;

    // Outside the home, from the owner of `docs/general/`: the global agent
    // files with their settings keys, written whole and never removed;
    // nothing records them. A sync writes no file of the repository: its
    // `AGENTS.md` files change only in a commit.
    let mut global_files = Vec::new();
    for file in &prepared.global {
        let written = tx.write(&file.at, file.text.as_bytes())?;
        global_files.push(GlobalDestination {
            path: file.path.clone(),
            runtime: file.runtime.clone(),
            instances: file.instances.clone(),
            written,
        });
    }
    for (path, bytes) in &prepared.settings {
        tx.write(path, bytes)?;
    }
    // The links the instances read through, ensured after their files.
    let mut links = Vec::new();
    for (link, target) in &prepared.links {
        let state = match tx.ensure_link(link, target)? {
            LinkOutcome::Made => LinkState::Made,
            LinkOutcome::Repaired => LinkState::Repaired,
            LinkOutcome::Current => LinkState::Current,
        };
        links.push(EnsuredLink {
            path: link.clone(),
            target: target.clone(),
            state,
        });
    }

    let fingerprint = _fingerprint::compute(home, &prepared.patched, &project.slug, commit)?;
    for (path, bytes) in _fingerprint::records(home, &fingerprint, now)? {
        tx.write(&in_home(&path)?, &bytes)?;
    }
    // The record of the repository files an earlier release wrote is
    // deleted unparsed, whatever it holds, and nothing it lists is touched.
    tx.remove_file(&in_home(&_tree::earlier_outputs_record(
        home,
        &project.slug,
    ))?)?;
    let record = PublishedRecord {
        project: project.slug.clone(),
        commit: commit.clone(),
        checkout: project.primary.clone(),
        remote: remote_url(&project.primary)?,
    };
    let text = serde_yaml_ng::to_string(&record).map_err(PublicationError::Render)?;
    tx.write(
        &in_home(&record_path(home, &project.slug))?,
        text.as_bytes(),
    )?;
    Ok((global_files, links))
}

/// The URL of the `origin` remote of the repository at `dir`; `None` when
/// it has none.
fn remote_url(dir: &Path) -> Result<Option<String>, PublicationError> {
    let output = git::run_output(dir, ["config", "--get", "remote.origin.url"])?;
    let url = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    Ok((output.status.success() && !url.is_empty()).then_some(url))
}

/// `path` relative to `root`, which contains it.
fn relative_to(root: &Path, path: &Path) -> Result<PathBuf, PublicationError> {
    path.strip_prefix(root)
        .map(Path::to_path_buf)
        .map_err(|_| PublicationError::Unconfined {
            path: path.to_path_buf(),
        })
}
