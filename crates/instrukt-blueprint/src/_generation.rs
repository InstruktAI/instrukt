//! Generation per remote-main revision
//! (`docs/project/spec/feature/blueprint/graph-generation.md`): a hook run
//! hands a revision over and returns at once; a detached worker maps it.
//!
//! # Mechanism
//!
//! Beside the graphs in the project's store
//! (`~/.instrukt/state/blueprint/<project>/`):
//!
//! ```text
//! queue.json         the queued revision; a newer handoff replaces it
//! generation.json    the last generation: its SHA, start, engine process
//!                    group and outcome
//! .queue.lock        held while the queue is read or replaced
//! .generation.lock   held by the worker generating; one per project
//! .waiter.lock       held by the one worker waiting to generate
//! .checkout-*/       a running generation's checkout of its revision
//! ```
//!
//! [`Generation::start_generation`] writes the revision as the queued one
//! and starts a detached worker (`instrukt blueprint worker <project>`).
//!
//! **One generation at a time.** A generation runs while its worker holds
//! `.generation.lock` *or* any process of its engine lives. The engine runs
//! in a process group of its own, recorded in `generation.json` while the
//! generation runs; whether the group still has a member is asked of the
//! operating system (`kill -0`), never of the engine, so an engine that
//! closes its input or ignores its parent cannot escape it. When the
//! engine's main process ends, the rest of its group is ended with it.
//!
//! **Waiting and recovery.** A worker first takes `.waiter.lock`; if
//! another worker already waits, that one will take the queued revision
//! and this one leaves. The waiter then looks, repeatedly, at who holds the
//! generation ([`next_step`]): a live worker is waited for unless its
//! *current* generation — measured from that generation's own start — is
//! past the engine's time limit and a grace beyond it; an engine left
//! running by a worker that is gone is waited for until it ends or passes
//! its own time limit, then its group is ended. Only then does the waiter
//! generate. A worker holding the lock generates queued revisions one at a
//! time until the queue is empty, releases the lock and looks once more,
//! so a revision queued while it was releasing is never left behind.
//!
//! **Status.** The last generation's record says `running` while it runs.
//! Workers take `.generation.lock` exclusively; whoever only looks — a
//! status reader or a waiting worker — takes it shared, so looking never
//! looks like generating. A reader first looks at the lock, then reads the
//! record, so a completed run is never read as interrupted. While a worker
//! holds the lock, a `running` record is that worker's: running. Otherwise
//! its worker is gone: running without its worker while its engine group
//! has a member ([`GenerationState::Orphaned`]), interrupted otherwise. A
//! worker that takes over first closes such a record as interrupted.
//!
//! A generation checks the revision's committed tree out into a private
//! directory with git plumbing (never the working tree, no hooks), runs the
//! engine there, validates its output and publishes it through
//! [`GraphStore::publish`] — so a failed, refused or interrupted run never
//! replaces the latest graph. A revision whose complete graph is stored is
//! not generated again; it is made the latest graph again.

use std::fs::{File, OpenOptions, TryLockError};
use std::io;
use std::os::unix::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use instrukt_core::config::{ConfigError, Configuration};
use instrukt_core::fsx::{self, FsError, LockGuard};
use instrukt_core::git;
use instrukt_core::home::Home;
use instrukt_core::ids::{CommitSha, ProjectSlug};
use instrukt_core::interfaces::{GenerationRequest, GraphGeneration, InterfaceError, MainAdvance};
use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use crate::_engine::{Engine, EngineError, group_alive, kill_group};
use crate::_store::{GraphStore, StoreError};

/// The arguments, after the executable, that start the detached worker;
/// the project slug follows. The CLI routes them to [`run_worker`].
pub const WORKER_COMMAND: [&str; 2] = ["blueprint", "worker"];

const QUEUE_FILE: &str = "queue.json";
const STATUS_FILE: &str = "generation.json";
const QUEUE_LOCK_FILE: &str = ".queue.lock";
const GENERATION_LOCK_FILE: &str = ".generation.lock";
const WAITER_LOCK_FILE: &str = ".waiter.lock";
const CHECKOUT_PREFIX: &str = ".checkout-";

/// How long the queue's lock is waited for; it is held for one small read
/// or write.
const QUEUE_LOCK_TIMEOUT: Duration = Duration::from_secs(10);
/// How long one engine run may take before it is ended.
pub(crate) const ENGINE_TIMEOUT: Duration = Duration::from_secs(30 * 60);
/// How long past the engine's limit a live worker's generation may still
/// run (checking out before, publishing after) before a waiter ends it.
pub(crate) const HOLDER_GRACE: Duration = Duration::from_secs(5 * 60);
/// How long a waiter pauses between looks at the generation.
const WAITER_POLL: Duration = Duration::from_millis(100);

/// Generation could not record, start or complete a revision.
#[derive(Debug, thiserror::Error)]
pub enum GenerationError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Fs(#[from] FsError),
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error(transparent)]
    Engine(#[from] EngineError),
    #[error("cannot read {path}: {source}")]
    Read { path: PathBuf, source: io::Error },
    #[error("{path} is not a valid generation record: {source}")]
    Record {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error("cannot check out {sha}: `git {args}` {detail}")]
    Checkout {
        sha: CommitSha,
        args: String,
        detail: String,
    },
    #[error("cannot start the generation worker {program}: {source}")]
    Spawn { program: PathBuf, source: io::Error },
}

/// The last generation's outcome, as `blueprint status` shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GenerationState {
    /// A worker is generating it.
    Running,
    /// Its worker is gone and its engine is still running: no other
    /// generation starts until the engine ends or, past its time limit, a
    /// waiting worker ends it.
    Orphaned,
    /// It started and its worker ended before recording an outcome.
    Interrupted,
    /// Its graph is the latest graph.
    Mapped,
    /// It failed or its output was refused; the latest graph stayed.
    Failed { error: String },
}

/// The last generation of a project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LastGeneration {
    pub sha: CommitSha,
    pub state: GenerationState,
}

impl LastGeneration {
    /// Whether it ended without making its revision the latest graph.
    pub fn is_failure(&self) -> bool {
        matches!(
            self.state,
            GenerationState::Interrupted | GenerationState::Failed { .. }
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Advance {
    Push,
    Pull,
}

/// A revision waiting for generation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Queued {
    sha: CommitSha,
    project_root: PathBuf,
    advance: Advance,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
enum Outcome {
    Running,
    Mapped {
        finished_at_ms: u64,
    },
    Failed {
        finished_at_ms: u64,
        error: String,
    },
    /// Its worker and engine were gone when the next worker took over.
    Interrupted {
        finished_at_ms: u64,
    },
}

/// What `generation.json` holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Record {
    sha: CommitSha,
    started_at_ms: u64,
    /// The engine's process group, once the engine runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    engine_group: Option<u32>,
    outcome: Outcome,
}

/// Who holds the generation, as a waiting worker sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Holder {
    /// A worker holds `.generation.lock`: it is alive.
    Worker,
    /// No worker holds it; the last generation's engine still has a live
    /// process.
    AbandonedEngine,
    /// No worker holds it and no engine of the last generation lives.
    Nobody,
}

/// What a waiting worker does next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Step {
    /// Generate: nothing of an earlier generation is running.
    Generate,
    /// Look again shortly.
    Wait,
    /// End the current generation's engine group, then look again.
    EndEngine,
    /// Leave the queued revision for the next handoff: a live worker is
    /// stuck past its generation's limit with no engine to end.
    GiveUp,
}

/// The waiting worker's decision. `elapsed` is how long the current
/// generation has run, from its own start; `None` when no generation is
/// running. `engine_alive` is whether its engine group has a member.
pub(crate) fn next_step(holder: Holder, engine_alive: bool, elapsed: Option<Duration>) -> Step {
    let past = |limit: Duration| elapsed.is_some_and(|elapsed| elapsed > limit);
    match holder {
        Holder::Nobody => Step::Generate,
        Holder::AbandonedEngine if past(ENGINE_TIMEOUT) => Step::EndEngine,
        Holder::AbandonedEngine => Step::Wait,
        Holder::Worker if !past(ENGINE_TIMEOUT + HOLDER_GRACE) => Step::Wait,
        Holder::Worker if engine_alive => Step::EndEngine,
        Holder::Worker => Step::GiveUp,
    }
}

/// Graph generation for repository's hook runs: records the revision and
/// starts the detached worker that maps it.
#[derive(Debug, Clone)]
pub struct Generation {
    home: Home,
    exe: PathBuf,
}

impl Generation {
    /// `exe` is the instrukt executable the worker runs as.
    pub fn new(home: Home, exe: PathBuf) -> Self {
        Self { home, exe }
    }

    fn start(&self, request: &GenerationRequest) -> Result<(), GenerationError> {
        let files = Files::new(&self.home, &request.project);
        files.enqueue(&Queued {
            sha: request.sha.clone(),
            project_root: request.project_root.clone(),
            advance: match request.advance {
                MainAdvance::Push => Advance::Push,
                MainAdvance::Pull => Advance::Pull,
            },
        })?;
        let mut command = Command::new(&self.exe);
        git::strip_repository_env(&mut command)
            .args(WORKER_COMMAND)
            .arg(request.project.as_str())
            .current_dir(&request.project_root)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            // Its own process group: a terminal's signals to git's group
            // do not reach it.
            .process_group(0);
        #[allow(
            clippy::zombie_processes,
            reason = "detached by design: the hook run returns at once and the worker is reparented when it exits"
        )]
        command.spawn().map_err(|source| GenerationError::Spawn {
            program: self.exe.clone(),
            source,
        })?;
        // Debug: a hook run's stderr is the developer's git output.
        tracing::debug!(
            event = "blueprint_generation_queued",
            project = %request.project,
            sha = %request.sha,
            advance = ?request.advance,
        );
        Ok(())
    }
}

impl GraphGeneration for Generation {
    fn start_generation(&self, request: &GenerationRequest) -> Result<(), InterfaceError> {
        self.start(request).map_err(|err| InterfaceError {
            module: "blueprint",
            message: err.to_string(),
        })
    }
}

/// The detached worker: generates the project's queued revisions one at a
/// time once nothing of an earlier generation runs; leaves at once when
/// another worker already waits. `now` is the invocation's clock.
pub fn run_worker(
    home: &Home,
    project: &ProjectSlug,
    now: &dyn Fn() -> Timestamp,
) -> Result<(), GenerationError> {
    let files = Files::new(home, project);
    loop {
        let Some(waiting) = LockGuard::try_acquire(&files.dir.join(WAITER_LOCK_FILE))? else {
            tracing::debug!(
                event = "blueprint_worker_yielded",
                project = %project,
                "another worker waits; it takes the queued revision"
            );
            return Ok(());
        };
        let Some(held) = files.wait_for_generation(project, now)? else {
            return Ok(());
        };
        drop(waiting);
        while let Some(queued) = files.take()? {
            generate(home, project, &files, &queued, now)?;
        }
        drop(held);
        if files.queued()?.is_none() {
            return Ok(());
        }
    }
}

/// The project's last generation; `None` before the first.
pub fn last_generation(
    home: &Home,
    project: &ProjectSlug,
) -> Result<Option<LastGeneration>, GenerationError> {
    let files = Files::new(home, project);
    // No store yet: nothing has run (and taking the lock must not create it).
    match std::fs::metadata(&files.dir) {
        Ok(_) => {}
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(GenerationError::Read {
                path: files.dir.clone(),
                source,
            });
        }
    }
    // The lock first, the record second: see "Status" above.
    let free = files.probe_generation_lock()?;
    let Some(record) = files.read::<Record>(&files.status())? else {
        return Ok(None);
    };
    let lock_free = free.is_some();
    // The OS is asked only about a running record's engine, and only when
    // no worker holds the lock.
    let alive = lock_free && engine_alive(running_group(&record));
    let state = state_of(&record, lock_free, alive);
    drop(free);
    Ok(Some(LastGeneration {
        sha: record.sha,
        state,
    }))
}

/// What a record says about its generation, given whether the generation
/// lock was free when it was read and whether its engine group lives.
fn state_of(record: &Record, lock_free: bool, engine_alive: bool) -> GenerationState {
    match &record.outcome {
        Outcome::Mapped { .. } => GenerationState::Mapped,
        Outcome::Failed { error, .. } => GenerationState::Failed {
            error: error.clone(),
        },
        Outcome::Interrupted { .. } => GenerationState::Interrupted,
        Outcome::Running if !lock_free => GenerationState::Running,
        Outcome::Running if engine_alive => GenerationState::Orphaned,
        Outcome::Running => GenerationState::Interrupted,
    }
}

/// The engine group of a running generation; `None` for an ended one, and
/// for one whose engine has not started — or whose record predates engine
/// groups, written by a worker that held the lock for its engine's life.
fn running_group(record: &Record) -> Option<u32> {
    (record.outcome == Outcome::Running)
        .then_some(record.engine_group)
        .flatten()
}

/// Who holds the generation: a worker while the lock is held, else the
/// last generation's engine while its group lives, else nobody.
fn holder(lock_free: bool, engine_alive: bool) -> Holder {
    match (lock_free, engine_alive) {
        (false, _) => Holder::Worker,
        (true, true) => Holder::AbandonedEngine,
        (true, false) => Holder::Nobody,
    }
}

/// Whether the engine group `group` still has a member; no group, none. A
/// group that cannot be asked about is reported and counted as alive:
/// guessing it dead could start a second engine beside it.
fn engine_alive(group: Option<u32>) -> bool {
    let Some(group) = group else {
        return false;
    };
    match group_alive(group) {
        Ok(alive) => alive,
        Err(err) => {
            tracing::warn!(
                event = "blueprint_engine_group_unknown",
                group,
                error = %err,
                "cannot tell whether the engine still runs; treating it as running"
            );
            true
        }
    }
}

/// Generates one revision and records its outcome. Only a failure to
/// record is an error; a failed generation is an outcome.
fn generate(
    home: &Home,
    project: &ProjectSlug,
    files: &Files,
    queued: &Queued,
    now: &dyn Fn() -> Timestamp,
) -> Result<(), GenerationError> {
    let now_ms = || millis(now());
    let started_at_ms = now_ms();
    let running = Record {
        sha: queued.sha.clone(),
        started_at_ms,
        engine_group: None,
        outcome: Outcome::Running,
    };
    files.record(&running)?;
    let outcome = match map(home, project, files, queued, &running) {
        Ok(()) => {
            tracing::info!(
                event = "blueprint_generation_mapped",
                project = %project,
                sha = %queued.sha,
                duration_ms = now_ms().saturating_sub(started_at_ms),
            );
            Outcome::Mapped {
                finished_at_ms: now_ms(),
            }
        }
        Err(err) => {
            tracing::warn!(
                event = "blueprint_generation_failed",
                project = %project,
                sha = %queued.sha,
                error = %err,
            );
            Outcome::Failed {
                finished_at_ms: now_ms(),
                error: err.to_string(),
            }
        }
    };
    files
        .record(&Record {
            sha: queued.sha.clone(),
            started_at_ms,
            engine_group: None,
            outcome,
        })
        .inspect_err(|err| {
            tracing::error!(
                event = "blueprint_generation_unrecorded",
                project = %project,
                sha = %queued.sha,
                error = %err,
                "the generation's outcome cannot be recorded"
            );
        })
}

/// Makes the revision's complete graph the latest graph: the stored one,
/// or one the engine produces from its committed tree.
fn map(
    home: &Home,
    project: &ProjectSlug,
    files: &Files,
    queued: &Queued,
    running: &Record,
) -> Result<(), GenerationError> {
    let store = GraphStore::new(home, project);
    let graph = match store.graph(&queued.sha)? {
        Some(graph) => graph,
        None => {
            let engine = Engine::from_setting(
                Configuration::load(home, Some(&queued.project_root))?.blueprint_engine()?,
            );
            std::fs::create_dir_all(&files.dir).map_err(|source| GenerationError::Read {
                path: files.dir.clone(),
                source,
            })?;
            let scratch = tempfile::Builder::new()
                .prefix(CHECKOUT_PREFIX)
                .tempdir_in(&files.dir)
                .map_err(|source| GenerationError::Read {
                    path: files.dir.clone(),
                    source,
                })?;
            let tree = scratch.path().join("tree");
            check_out(&queued.project_root, &queued.sha, scratch.path(), &tree)?;
            // The engine may start only once its group is durably recorded:
            // an engine the record does not name could run beside the next.
            engine.generate(&tree, scratch.path(), ENGINE_TIMEOUT, &mut |group| {
                let record = Record {
                    engine_group: Some(group),
                    ..running.clone()
                };
                files.record(&record).map_err(|err| {
                    tracing::warn!(
                        event = "blueprint_engine_group_unrecorded",
                        group,
                        error = %err,
                        "the engine is not started"
                    );
                    err.to_string()
                })
            })?
        }
    };
    let published = store.publish(&queued.sha, &graph)?;
    for failure in &published.cleanup_failures {
        tracing::warn!(
            event = "blueprint_generation_cleanup_failed",
            path = %failure.path.display(),
            error = %failure.error,
        );
    }
    Ok(())
}

/// Writes the committed tree of `sha` to `tree` through a private index:
/// the repository's index, working tree and hooks are untouched.
fn check_out(
    root: &Path,
    sha: &CommitSha,
    scratch: &Path,
    tree: &Path,
) -> Result<(), GenerationError> {
    let index = scratch.join("index");
    let prefix = format!("{}/", tree.display());
    let tree_ish = format!("{sha}^{{tree}}");
    let steps: [Vec<&str>; 2] = [
        vec!["read-tree", tree_ish.as_str()],
        vec![
            "checkout-index",
            "--all",
            "--force",
            "--prefix",
            prefix.as_str(),
        ],
    ];
    for args in steps {
        let mut command = Command::new("git");
        git::strip_repository_env(&mut command)
            .args(&args)
            .env("GIT_INDEX_FILE", &index)
            .current_dir(root)
            .stdin(Stdio::null());
        let failed = |detail: String| GenerationError::Checkout {
            sha: sha.clone(),
            args: args.join(" "),
            detail,
        };
        let output = command
            .output()
            .map_err(|err| failed(format!("could not run: {err}")))?;
        if !output.status.success() {
            return Err(failed(format!(
                "failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
    }
    Ok(())
}

/// The generation files of one project's store.
struct Files {
    dir: PathBuf,
}

impl Files {
    fn new(home: &Home, project: &ProjectSlug) -> Self {
        Self {
            dir: GraphStore::new(home, project).dir().to_path_buf(),
        }
    }

    fn queue(&self) -> PathBuf {
        self.dir.join(QUEUE_FILE)
    }

    fn status(&self) -> PathBuf {
        self.dir.join(STATUS_FILE)
    }

    fn generation_lock_file(&self) -> Result<File, GenerationError> {
        let path = self.dir.join(GENERATION_LOCK_FILE);
        std::fs::create_dir_all(&self.dir).map_err(|source| GenerationError::Read {
            path: self.dir.clone(),
            source,
        })?;
        OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .map_err(|source| GenerationError::Read { path, source })
    }

    /// Whether no worker holds the generation lock, looked at without
    /// excluding other lookers: the lock taken shared, `None` while a
    /// worker holds it.
    fn probe_generation_lock(&self) -> Result<Option<File>, GenerationError> {
        let file = self.generation_lock_file()?;
        match file.try_lock_shared() {
            Ok(()) => Ok(Some(file)),
            Err(TryLockError::WouldBlock) => Ok(None),
            Err(TryLockError::Error(source)) => Err(GenerationError::Read {
                path: self.dir.join(GENERATION_LOCK_FILE),
                source,
            }),
        }
    }

    /// The generation lock if no worker holds it: the open file holding it.
    fn try_generation_lock(&self) -> Result<Option<File>, GenerationError> {
        let file = self.generation_lock_file()?;
        match file.try_lock() {
            Ok(()) => Ok(Some(file)),
            Err(TryLockError::WouldBlock) => Ok(None),
            Err(TryLockError::Error(source)) => Err(GenerationError::Read {
                path: self.dir.join(GENERATION_LOCK_FILE),
                source,
            }),
        }
    }

    /// Waits until nothing of an earlier generation runs ("Waiting and
    /// recovery" above) and returns the generation lock; `None` when a live
    /// worker is stuck past its generation's limit with no engine to end.
    fn wait_for_generation(
        &self,
        project: &ProjectSlug,
        now: &dyn Fn() -> Timestamp,
    ) -> Result<Option<File>, GenerationError> {
        loop {
            // Looking takes the lock shared, as status does: a waiter
            // looking is never mistaken for a worker generating.
            let probe = self.probe_generation_lock()?;
            let record = self.read::<Record>(&self.status())?;
            let running = record
                .as_ref()
                .filter(|record| record.outcome == Outcome::Running);
            let group = record.as_ref().and_then(running_group);
            let alive = engine_alive(group);
            let holder = holder(probe.is_some(), alive);
            let elapsed = running.map(|record| {
                Duration::from_millis(millis(now()).saturating_sub(record.started_at_ms))
            });
            drop(probe);
            match next_step(holder, alive, elapsed) {
                Step::Generate => {
                    if let Some(lock) = self.take_over(now)? {
                        return Ok(Some(lock));
                    }
                }
                Step::Wait => {}
                Step::EndEngine => {
                    if let Some(group) = group {
                        tracing::warn!(
                            event = "blueprint_overdue_engine_ended",
                            project = %project,
                            group,
                            holder = ?holder,
                            "the running generation is past its time limit; ending its engine"
                        );
                        kill_group(group);
                    }
                }
                Step::GiveUp => {
                    tracing::warn!(
                        event = "blueprint_worker_gave_up",
                        project = %project,
                        "the generating worker is stuck past its limit; the queued revision waits for the next handoff"
                    );
                    return Ok(None);
                }
            }
            std::thread::sleep(WAITER_POLL);
        }
    }

    /// Takes the generation lock exclusively once nothing runs, and closes
    /// a record left `running` by a worker and engine that are gone as
    /// interrupted before anything else is written, so no reader takes this
    /// worker for that generation's. `None` when the lock or a live engine
    /// came in between; the caller looks again.
    fn take_over(&self, now: &dyn Fn() -> Timestamp) -> Result<Option<File>, GenerationError> {
        let Some(lock) = self.try_generation_lock()? else {
            return Ok(None);
        };
        if let Some(record) = self.read::<Record>(&self.status())?
            && record.outcome == Outcome::Running
        {
            if engine_alive(running_group(&record)) {
                return Ok(None);
            }
            self.record(&Record {
                engine_group: None,
                outcome: Outcome::Interrupted {
                    finished_at_ms: millis(now()),
                },
                ..record
            })?;
        }
        Ok(Some(lock))
    }

    fn queue_lock(&self) -> Result<LockGuard, GenerationError> {
        Ok(LockGuard::acquire(
            &self.dir.join(QUEUE_LOCK_FILE),
            QUEUE_LOCK_TIMEOUT,
        )?)
    }

    /// Makes `queued` the queued revision, replacing an older one.
    fn enqueue(&self, queued: &Queued) -> Result<(), GenerationError> {
        let _lock = self.queue_lock()?;
        self.write(&self.queue(), queued)
    }

    /// The queued revision, without taking it.
    fn queued(&self) -> Result<Option<Queued>, GenerationError> {
        let _lock = self.queue_lock()?;
        self.read(&self.queue())
    }

    /// Takes the queued revision off the queue.
    fn take(&self) -> Result<Option<Queued>, GenerationError> {
        let _lock = self.queue_lock()?;
        let queued = self.read(&self.queue())?;
        if queued.is_some() {
            std::fs::remove_file(self.queue()).map_err(|source| GenerationError::Read {
                path: self.queue(),
                source,
            })?;
        }
        Ok(queued)
    }

    fn record(&self, record: &Record) -> Result<(), GenerationError> {
        self.write(&self.status(), record)
    }

    fn write<T: Serialize>(&self, path: &Path, value: &T) -> Result<(), GenerationError> {
        let bytes = serde_json::to_vec_pretty(value).map_err(|source| GenerationError::Record {
            path: path.to_path_buf(),
            source,
        })?;
        Ok(fsx::atomic_write(path, &bytes)?)
    }

    fn read<T: for<'de> Deserialize<'de>>(
        &self,
        path: &Path,
    ) -> Result<Option<T>, GenerationError> {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(source) => {
                return Err(GenerationError::Read {
                    path: path.to_path_buf(),
                    source,
                });
            }
        };
        serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|source| GenerationError::Record {
                path: path.to_path_buf(),
                source,
            })
    }
}

fn millis(at: Timestamp) -> u64 {
    u64::try_from(at.as_millisecond()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINUTE: Duration = Duration::from_secs(60);

    #[test]
    fn nothing_running_is_generated_at_once() {
        assert_eq!(next_step(Holder::Nobody, false, None), Step::Generate);
    }

    #[test]
    fn an_abandoned_engine_within_its_limit_is_waited_for() {
        assert_eq!(
            next_step(Holder::AbandonedEngine, true, Some(ENGINE_TIMEOUT - MINUTE)),
            Step::Wait
        );
    }

    #[test]
    fn an_abandoned_engine_past_its_limit_is_ended() {
        assert_eq!(
            next_step(Holder::AbandonedEngine, true, Some(ENGINE_TIMEOUT + MINUTE)),
            Step::EndEngine
        );
    }

    #[test]
    fn a_live_worker_is_waited_for_while_its_current_generation_is_within_its_limit() {
        // The holder may have run long, but its current generation started
        // 13 minutes ago: elapsed is that generation's own.
        assert_eq!(
            next_step(Holder::Worker, true, Some(13 * MINUTE)),
            Step::Wait
        );
    }

    #[test]
    fn a_live_worker_just_past_the_engine_limit_is_still_given_its_grace() {
        assert_eq!(
            next_step(Holder::Worker, true, Some(ENGINE_TIMEOUT + MINUTE)),
            Step::Wait
        );
    }

    #[test]
    fn a_live_worker_between_generations_is_waited_for() {
        assert_eq!(next_step(Holder::Worker, false, None), Step::Wait);
    }

    #[test]
    fn a_live_worker_past_its_limit_and_grace_has_its_engine_ended() {
        let overdue = Some(ENGINE_TIMEOUT + HOLDER_GRACE + MINUTE);
        assert_eq!(next_step(Holder::Worker, true, overdue), Step::EndEngine);
    }

    #[test]
    fn a_live_worker_past_its_limit_and_grace_with_no_engine_is_given_up_on() {
        let overdue = Some(ENGINE_TIMEOUT + HOLDER_GRACE + MINUTE);
        assert_eq!(next_step(Holder::Worker, false, overdue), Step::GiveUp);
    }

    /// `generation.json` as a worker wrote it before engine groups were
    /// recorded: a running generation with no `engine_group`.
    const OLD_RUNNING_RECORD: &str = r#"{
  "sha": "0123456789abcdef0123456789abcdef01234567",
  "started_at_ms": 1700000000000,
  "outcome": {
    "state": "running"
  }
}"#;

    fn old_running_record() -> Record {
        serde_json::from_str(OLD_RUNNING_RECORD).unwrap()
    }

    #[test]
    fn an_old_running_record_names_no_engine_to_ask_about() {
        assert_eq!(running_group(&old_running_record()), None);
    }

    #[test]
    fn an_old_running_record_with_the_lock_free_is_generated_past() {
        // Its worker held the lock for its engine's whole life: with the
        // lock free, nothing of it runs, and the next revision generates.
        let record = old_running_record();
        let alive = engine_alive(running_group(&record));
        let elapsed = Some(MINUTE);
        assert_eq!(
            next_step(holder(true, alive), alive, elapsed),
            Step::Generate
        );
    }

    #[test]
    fn an_old_running_record_with_the_lock_free_reads_as_interrupted() {
        let record = old_running_record();
        let alive = engine_alive(running_group(&record));
        assert_eq!(state_of(&record, true, alive), GenerationState::Interrupted);
    }

    #[test]
    fn a_record_closed_by_the_next_worker_reads_as_interrupted_whoever_holds_the_lock() {
        let record: Record = serde_json::from_str(
            r#"{"sha": "0123456789abcdef0123456789abcdef01234567", "started_at_ms": 1,
                "outcome": {"state": "interrupted", "finished_at_ms": 2}}"#,
        )
        .unwrap();
        assert_eq!(
            state_of(&record, false, false),
            GenerationState::Interrupted
        );
    }

    #[test]
    fn an_old_running_record_with_the_lock_held_reads_as_running() {
        assert_eq!(
            state_of(&old_running_record(), false, false),
            GenerationState::Running
        );
    }
}
