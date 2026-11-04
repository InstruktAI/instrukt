//! `instrukt sync`: one project's publication from its canonical main.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::_agents::{MergeRefusal, TARGETS, merge_key};
use super::_compile::{self, Compiler};
use super::_fingerprint;
use super::_patch::{Claim, Contribution, patch};
use super::_source::{self, Committed, MASTER_FILE_NAME, Project};
use super::_tree::{self, BUNDLE_FILE, Outputs, PublisherRecord};
use super::_tx::{STAGING_PREFIX, Transaction, confined, resolved, sweep};
use super::{PublicationError, PublishedRecord, record_path};
use crate::config::{self, ConfigError, Configuration, Layer, Module};
use crate::docs::{self, Corpus, Finding, ValidationRequest, Values};
use crate::fsx::{LockGuard, atomic_write};
use crate::home::Home;
use crate::ids::{CommitSha, DomainName, ProjectSlug};
use crate::index::GlobalIndex;
use crate::interfaces::CoreWiring;

/// How long a sync waits for another sync to release the docs lock.
const DOCS_LOCK_TIMEOUT: Duration = Duration::from_secs(300);

/// The docs lock's file name below the home's lock directory.
const DOCS_LOCK: &str = "docs.lock";

/// Domain names the published tree keeps for itself.
const RESERVED_DOMAINS: [&str; 1] = ["third-party"];

/// The domain whose owner writes the global agent entrypoints.
const GENERAL: &str = "general";

/// The `general` owner's global master.
const GLOBAL_MASTER: &str = "agents/AGENTS.global.md";

/// The compiled file beside a master.
const AGENTS_FILE: &str = "AGENTS.md";

/// What one sync is asked to do.
#[derive(Clone, Copy)]
pub struct SyncRequest<'a> {
    pub home: &'a Home,
    /// The caller's working directory: the project it stands in is the one
    /// published.
    pub cwd: &'a Path,
    /// Republish even when main equals the published commit.
    pub force: bool,
    /// The optional modules wired for this invocation (validation reads
    /// them).
    pub wiring: &'a CoreWiring,
    /// The invocation's clock reading: when this publication is recorded.
    pub now: jiff::Timestamp,
}

/// Why a sync published nothing although it could run.
#[derive(Debug, Clone, PartialEq)]
pub enum Refusal {
    /// The committed documentation has structural validation findings.
    Findings(Vec<Finding>),
    /// The project claims an id or a domain another project owns.
    Claim(Claim),
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
    /// Main equals the published commit; nothing changed.
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
/// of its canonical main, under the docs lock. A refused or failed sync
/// leaves the previous publication in place and records no commit. Every
/// sync, the no-op included, first removes the leftovers earlier syncs
/// could not, and reports those it still cannot whatever its outcome.
pub fn sync(request: &SyncRequest<'_>) -> Result<SyncReport, SyncError> {
    let started = Instant::now();
    let project = _source::locate(request.cwd)?;
    let _lock = LockGuard::acquire(&request.home.locks_dir().join(DOCS_LOCK), DOCS_LOCK_TIMEOUT)
        .map_err(PublicationError::from)?;
    let mut leftovers = sweep(request.home.root())?;
    let attempt = (|| {
        let (branch, commit) = _source::canonical_main(&project.primary)?;
        let previous = PublishedRecord::load(request.home, &project.slug)?;
        let outcome = if !request.force && previous.is_some_and(|r| r.commit == commit) {
            SyncOutcome::Unchanged {
                project: project.slug.clone(),
                commit: commit.clone(),
            }
        } else {
            publish(request, &project, &commit, &mut leftovers)?
        };
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
    tracing::debug!(
        event = "sync_completed",
        project = %project.slug,
        branch = %branch,
        commit = %commit,
        forced = request.force,
        outcome = label,
        leftovers = leftovers.len(),
        duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
    );
    Ok(SyncReport { outcome, leftovers })
}

/// Everything a sync computed before it writes anything.
struct Prepared {
    committed: Committed,
    publishes: Vec<config::PublishedDomain>,
    /// The index as it was, and as this sync leaves it.
    index: GlobalIndex,
    patched: GlobalIndex,
    publishers: BTreeMap<DomainName, PublisherRecord>,
    /// Each published domain's compiled bundle.
    bundles: BTreeMap<DomainName, String>,
    /// Compiled `AGENTS.md` files, relative to the primary worktree.
    agents: Vec<(PathBuf, String)>,
    /// Global agent entrypoints, relative to the user's home directory.
    entrypoints: Vec<(PathBuf, String)>,
    /// Merged settings files, relative to the user's home directory.
    settings: Vec<(PathBuf, Vec<u8>)>,
}

fn publish(
    request: &SyncRequest<'_>,
    project: &Project,
    commit: &CommitSha,
    leftovers: &mut Vec<PathBuf>,
) -> Result<SyncOutcome, PublicationError> {
    let home = request.home;
    config::migrate(home, &Layer::Global)?;
    config::migrate(home, &Layer::Project(project.worktree.clone()))?;
    let committed = Committed::read(&project.primary, commit)?;
    let checkout = tempfile::Builder::new()
        .prefix("instrukt-sync-")
        .tempdir()
        .map_err(|source| PublicationError::Io {
            path: std::env::temp_dir(),
            source,
        })?;
    committed.materialize(checkout.path())?;
    // The committed layer is migrated in the temporary checkout only when
    // it needs it: migration takes a lock named after the layer's path, and
    // a fresh checkout path would leave a new lock file per sync.
    let committed_slug = match config::project_slug(checkout.path()) {
        Err(ConfigError::OlderSchema { .. }) => {
            config::migrate(home, &Layer::Project(checkout.path().to_path_buf()))?;
            config::project_slug(checkout.path())?
        }
        other => other?,
    };
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
    let findings = docs::validate(&ValidationRequest {
        repo_root: checkout.path(),
        home,
        paths: &[],
        wiring: request.wiring,
    })?;
    if !findings.is_empty() {
        return Ok(refused(Refusal::Findings(findings)));
    }
    let config = Configuration::load(home, Some(checkout.path()))?;
    drop(checkout);
    let publishes = config.publishes()?;
    if let Some(reserved) = publishes
        .iter()
        .find(|p| RESERVED_DOMAINS.contains(&p.domain.as_str()))
    {
        return Err(PublicationError::ReservedDomain {
            domain: reserved.domain.to_string(),
        });
    }
    let corpus = Corpus::from_sources(Some(project.slug.clone()), home, committed.doc_sources());
    let fragments: Vec<String> = committed
        .below("docs/global/snippets/")
        .filter_map(|(rel, _)| rel.strip_suffix(".md"))
        .filter(|name| !name.split('/').any(|segment| segment.starts_with('_')))
        .map(str::to_owned)
        .collect();
    let domains: Vec<DomainName> = publishes.iter().map(|p| p.domain.clone()).collect();
    let contribution = Contribution::of(
        &project.slug,
        &corpus,
        &domains,
        &fragments,
        &project.primary,
    );
    let publishers = _tree::publishers(home)?;
    let owners: BTreeMap<DomainName, ProjectSlug> = publishers
        .iter()
        .map(|(domain, record)| (domain.clone(), record.publisher.clone()))
        .collect();
    let index = GlobalIndex::load(home)?.unwrap_or_default();
    let patched = match patch(&index, &owners, &contribution) {
        Ok(patched) => patched,
        Err(claim) => return Ok(refused(Refusal::Claim(claim))),
    };

    // Compilation reads the committed tree and the home as it stands; it
    // writes nothing, so a failure here leaves everything as it was.
    let owned: BTreeSet<DomainName> = domains.iter().cloned().collect();
    let values = Values::publish(&config);
    let events = config.module_enabled(Module::Events)?;
    let mut compiler = Compiler::new(
        home,
        &project.slug,
        &corpus,
        &owned,
        &patched,
        events,
        &values,
        &project.primary,
    );
    let mut bundles = BTreeMap::new();
    for domain in &domains {
        bundles.insert(domain.clone(), compiler.bundle(domain)?);
    }
    let mut agents = Vec::new();
    for master in committed.masters() {
        let text = String::from_utf8_lossy(committed.file(master).unwrap_or_default()).into_owned();
        let compiled = compiler.master(master, &text)?;
        let dir = Path::new(master).parent().unwrap_or(Path::new(""));
        if master == MASTER_FILE_NAME {
            agents.extend(project_overrides(home, &compiled)?);
        }
        agents.push((dir.join(AGENTS_FILE), compiled));
    }
    let mut triggers: BTreeMap<DomainName, String> = publishers
        .iter()
        .filter(|(_, record)| record.publisher != project.slug)
        .map(|(domain, record)| (domain.clone(), record.trigger.clone()))
        .collect();
    for domain in &publishes {
        triggers.insert(domain.domain.clone(), domain.trigger.clone());
    }
    let (entrypoints, settings) = if owned.iter().any(|d| d.as_str() == GENERAL) {
        global_entrypoints(home, &committed, &mut compiler, &patched, &triggers)?
    } else {
        (Vec::new(), Vec::new())
    };

    let prepared = Prepared {
        committed,
        publishes,
        index,
        patched,
        publishers,
        bundles,
        agents,
        entrypoints,
        settings,
    };
    preflight(home, project, commit, &prepared)?;
    let mut tx = Transaction::begin(home.root())?;
    match write(home, project, commit, request.now, &prepared, &mut tx) {
        Ok(()) => {
            leftovers.extend(tx.commit());
            Ok(SyncOutcome::Published {
                project: project.slug.clone(),
                commit: commit.clone(),
            })
        }
        Err(err) => {
            tx.rollback();
            Err(err)
        }
    }
}

/// Checks every location the sync will write or remove, before the first
/// write. A location the repository or instrukt controls — a compiled
/// output in the project or one recorded for retraction; in the home a
/// published domain, a fragment, a third-party doc, the index, the eval
/// fingerprint records, the outputs record or the published record — that
/// is, or lies behind, a symbolic link refuses the sync. An agent-CLI file
/// in the user's home whose link leads nowhere does too. The writes check
/// again as a backstop.
fn preflight(
    home: &Home,
    project: &Project,
    commit: &CommitSha,
    prepared: &Prepared,
) -> Result<(), PublicationError> {
    let previous = Outputs::load(home, &project.slug)?;
    for rel in prepared
        .agents
        .iter()
        .map(|(rel, _)| rel)
        .chain(&previous.agents)
    {
        confined(&project.primary, rel)?;
    }
    let user_home = home.user_home();
    for rel in prepared
        .entrypoints
        .iter()
        .map(|(rel, _)| rel)
        .chain(prepared.settings.iter().map(|(rel, _)| rel))
        .chain(&previous.entrypoints)
    {
        resolved(user_home, rel)?;
    }
    let root = home.root();
    let mine = |owner: &ProjectSlug| owner == &project.slug;
    let domains = prepared.publishes.iter().map(|p| &p.domain).chain(
        prepared
            .publishers
            .iter()
            .filter(|(_, record)| mine(&record.publisher))
            .map(|(domain, _)| domain),
    );
    let mut targets: Vec<PathBuf> = domains.map(|d| home.domain_dir(d)).collect();
    let docs = home.docs_dir();
    for index in [&prepared.index, &prepared.patched] {
        targets.extend(
            index
                .fragments
                .iter()
                .filter(|f| mine(&f.source_project))
                .map(|f| docs.join(&f.path)),
        );
        targets.extend(
            index
                .third_party
                .iter()
                .filter(|d| mine(&d.source_project))
                .map(|d| docs.join(&d.path)),
        );
    }
    targets.push(home.index_file());
    targets.extend(_fingerprint::record_paths(home, commit));
    targets.push(Outputs::record_path(home, &project.slug));
    targets.push(record_path(home, &project.slug));
    for target in targets {
        confined(root, &relative_to(root, &target)?)?;
    }
    Ok(())
}

/// The project override instruction files of the supported agent CLIs
/// installed here that read one, each holding the root `AGENTS.md`'s
/// compiled text, relative to the primary worktree.
fn project_overrides(home: &Home, root_agents: &str) -> Result<Files<String>, PublicationError> {
    let mut overrides = Vec::new();
    for target in TARGETS {
        let Some(file) = target.project_override else {
            continue;
        };
        if resolved(home.user_home(), Path::new(target.dir))?.is_dir() {
            overrides.push((PathBuf::from(file), root_agents.to_owned()));
        }
    }
    Ok(overrides)
}

/// `(path, content)` pairs of files a sync writes, relative to a root.
type Files<T> = Vec<(PathBuf, T)>;

/// The compiled global entrypoint of every supported agent CLI installed
/// here, and the settings files that take its entrypoint-loading key. A
/// settings file that cannot take the key fails the sync before anything
/// is written.
fn global_entrypoints(
    home: &Home,
    committed: &Committed,
    compiler: &mut Compiler<'_>,
    index: &GlobalIndex,
    triggers: &BTreeMap<DomainName, String>,
) -> Result<(Files<String>, Files<Vec<u8>>), PublicationError> {
    let Some(master) = committed.file(GLOBAL_MASTER) else {
        return Ok((Vec::new(), Vec::new()));
    };
    let master = String::from_utf8_lossy(master).into_owned();
    let inlined = compiler.inlined_bundles(GLOBAL_MASTER, &master)?;
    let roster = _compile::roster(index, triggers, &inlined)?;
    let user_home = home.user_home();
    let mut entrypoints = Vec::new();
    let mut settings = Vec::new();
    for target in TARGETS {
        let dir = resolved(user_home, Path::new(target.dir))?;
        if !dir.is_dir() {
            tracing::debug!(event = "sync_agent_skipped", agent = target.name, dir = %dir.display());
            continue;
        }
        let addendum_file = target.addendum();
        let addendum = committed
            .file(&addendum_file)
            .map(|bytes| String::from_utf8_lossy(bytes).into_owned());
        let compiled = compiler.entrypoint(
            (GLOBAL_MASTER, &master),
            addendum
                .as_deref()
                .map(|text| (addendum_file.as_str(), text)),
            &roster,
        )?;
        entrypoints.push((Path::new(target.dir).join(target.entrypoint), compiled));
        if let Some(loading) = target.loading {
            let path = resolved(user_home, Path::new(loading.settings))?;
            let existing = match std::fs::read(&path) {
                Ok(bytes) => Some(bytes),
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
                Err(source) => return Err(PublicationError::Io { path, source }),
            };
            match merge_key(existing.as_deref(), loading.key, (loading.value)()) {
                Ok(Some(merged)) => settings.push((PathBuf::from(loading.settings), merged)),
                Ok(None) => {}
                Err(refusal) => {
                    return Err(PublicationError::Settings {
                        path,
                        reason: match refusal {
                            MergeRefusal::NotAnObject => "it is not a JSON object".to_owned(),
                            MergeRefusal::Blocked { member } => {
                                format!("its `{member}` is not an object")
                            }
                        },
                    });
                }
            }
        }
    }
    Ok((entrypoints, settings))
}

/// The writes of a publishing sync, all through `tx`.
fn write(
    home: &Home,
    project: &Project,
    commit: &CommitSha,
    now: jiff::Timestamp,
    prepared: &Prepared,
    tx: &mut Transaction,
) -> Result<(), PublicationError> {
    let root = home.root();
    let in_home = |path: &Path| confined(root, &relative_to(root, path)?);
    let staging = tempfile::Builder::new()
        .prefix(STAGING_PREFIX)
        .tempdir_in(root)
        .map_err(|source| PublicationError::Io {
            path: root.to_path_buf(),
            source,
        })?;
    // Domains: staged whole with their record and bundle, then swapped in.
    let published: BTreeSet<&DomainName> = prepared.publishes.iter().map(|p| &p.domain).collect();
    for domain in &prepared.publishes {
        let record = PublisherRecord {
            domain: domain.domain.clone(),
            publisher: project.slug.clone(),
            trigger: domain.trigger.clone(),
        };
        let staged = staging.path().join(domain.domain.as_str());
        _tree::stage_domain(&prepared.committed, &record, &staged)?;
        if let Some(bundle) = prepared.bundles.get(&domain.domain) {
            atomic_write(&staged.join(BUNDLE_FILE), bundle.as_bytes())?;
        }
        tx.replace_dir(&staged, &in_home(&home.domain_dir(&domain.domain))?)?;
    }
    for (domain, record) in &prepared.publishers {
        if record.publisher == project.slug && !published.contains(domain) {
            tx.remove_dir(&in_home(&home.domain_dir(domain))?)?;
        }
    }
    // Fragments and third-party docs: file by file, owner-scoped.
    let docs = home.docs_dir();
    let mine = |owner: &ProjectSlug| owner == &project.slug;
    for fragment in prepared
        .patched
        .fragments
        .iter()
        .filter(|f| mine(&f.source_project))
    {
        let source = format!("docs/global/snippets/{}.md", fragment.name);
        if let Some(bytes) = prepared.committed.file(&source) {
            tx.write(&in_home(&docs.join(&fragment.path))?, bytes)?;
        }
    }
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
    let fragments: BTreeSet<&PathBuf> =
        prepared.patched.fragments.iter().map(|f| &f.path).collect();
    for old in prepared
        .index
        .fragments
        .iter()
        .filter(|f| mine(&f.source_project))
    {
        if !fragments.contains(&old.path) {
            tx.remove_file(&in_home(&docs.join(&old.path))?)?;
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

    // Outside the home: the project's AGENTS.md files and, from the owner
    // of `general`, the global agent entrypoints with their settings keys.
    let previous = Outputs::load(home, &project.slug)?;
    let mut outputs = Outputs::default();
    for (rel, text) in &prepared.agents {
        tx.write(&confined(&project.primary, rel)?, text.as_bytes())?;
        outputs.agents.push(rel.clone());
    }
    for old in previous
        .agents
        .iter()
        .filter(|old| !outputs.agents.contains(old))
    {
        tx.remove_file(&confined(&project.primary, old)?)?;
    }
    let user_home = home.user_home();
    for (rel, text) in &prepared.entrypoints {
        tx.write(&resolved(user_home, rel)?, text.as_bytes())?;
        outputs.entrypoints.push(rel.clone());
    }
    for (rel, bytes) in &prepared.settings {
        tx.write(&resolved(user_home, rel)?, bytes)?;
    }
    for old in previous
        .entrypoints
        .iter()
        .filter(|old| !outputs.entrypoints.contains(old))
    {
        tx.remove_file(&resolved(user_home, old)?)?;
    }

    let fingerprint = _fingerprint::compute(home, &prepared.patched, &project.slug, commit, now)?;
    for (path, bytes) in _fingerprint::records(home, &fingerprint)? {
        tx.write(&in_home(&path)?, &bytes)?;
    }
    let (path, bytes) = outputs.rendered(home, &project.slug)?;
    tx.write(&in_home(&path)?, &bytes)?;
    let record = PublishedRecord {
        project: project.slug.clone(),
        commit: commit.clone(),
        checkout: project.primary.clone(),
    };
    let text = serde_yaml_ng::to_string(&record).map_err(PublicationError::Render)?;
    tx.write(
        &in_home(&record_path(home, &project.slug))?,
        text.as_bytes(),
    )?;
    Ok(())
}

/// `path` relative to `root`, which contains it.
fn relative_to(root: &Path, path: &Path) -> Result<PathBuf, PublicationError> {
    path.strip_prefix(root)
        .map(Path::to_path_buf)
        .map_err(|_| PublicationError::Unconfined {
            path: path.to_path_buf(),
        })
}
