//! Generation per remote-main revision
//! (`docs/local/spec/feature/blueprint/graph-generation.md`): a hook run
//! hands a revision over and returns at once; a detached worker maps it.
//!
//! # Mechanism
//!
//! Beside the graphs in the project's store
//! (`~/.instrukt/state/blueprint/<project>/`):
//!
//! ```text
//! queue.json         the queued revision; a newer handoff replaces it
//! generation.json    the last generation: its SHA and outcome
//! .queue.lock        held while the queue is read or replaced
//! .generation.lock   held while a generation's processes live; one per project
//! .waiter.lock       held by the one worker waiting for .generation.lock
//! engine.pgid        the process group of the engine running, if any
//! .checkout-*/       a running generation's checkout of its revision
//! ```
//!
//! [`Generation::start_generation`] writes the revision as the queued one
//! and starts a detached worker (`instrukt blueprint worker <project>`).
//!
//! **One generation at a time.** `.generation.lock` is held through an open
//! file the worker shares with the engine it runs: the engine's standard
//! input is that file. The lock is released only once the worker and every
//! process of the engine have exited, so a worker killed mid-run leaves the
//! lock held until its engine ends and no second engine starts beside it.
//!
//! **Waiting.** A worker first takes `.waiter.lock`; if another worker
//! already waits, that one will take the queued revision and this one
//! leaves. The waiter waits for `.generation.lock` longer than a worker
//! lets its engine run; a lock still held then belongs to an engine whose
//! worker is gone, so the waiter ends that engine's process group and takes
//! over. The holder generates queued revisions one at a time until the
//! queue is empty, releases the lock and looks once more, so a revision
//! queued while it was releasing is never left behind.
//!
//! **Status.** The last generation's record says `running` while it runs.
//! A reader first tries `.generation.lock`, then reads the record. Holding
//! the lock, a `running` record is one whose processes are all gone
//! (interrupted); not holding it, the record is whatever the generation
//! wrote last. No worker writes the record while the reader holds the
//! lock, so a completed run is never read as interrupted.
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
use std::time::{Duration, Instant};

use instrukt_core::config::{ConfigError, Configuration};
use instrukt_core::fsx::{self, FsError, LockGuard};
use instrukt_core::git;
use instrukt_core::home::Home;
use instrukt_core::ids::{CommitSha, ProjectSlug};
use instrukt_core::interfaces::{GenerationRequest, GraphGeneration, InterfaceError, MainAdvance};
use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use crate::_engine::{Engine, EngineError, kill_group};
use crate::_store::{GraphStore, StoreError};

/// The arguments, after the executable, that start the detached worker;
/// the project slug follows. The CLI routes them to [`run_worker`].
pub const WORKER_COMMAND: [&str; 2] = ["blueprint", "worker"];

const QUEUE_FILE: &str = "queue.json";
const STATUS_FILE: &str = "generation.json";
const QUEUE_LOCK_FILE: &str = ".queue.lock";
const GENERATION_LOCK_FILE: &str = ".generation.lock";
const WAITER_LOCK_FILE: &str = ".waiter.lock";
const ENGINE_GROUP_FILE: &str = "engine.pgid";
const CHECKOUT_PREFIX: &str = ".checkout-";

/// How long the queue's lock is waited for; it is held for one small read
/// or write.
const QUEUE_LOCK_TIMEOUT: Duration = Duration::from_secs(10);
/// How long one engine run may take before its worker ends it.
const ENGINE_TIMEOUT: Duration = Duration::from_secs(30 * 60);
/// How long a waiting worker waits for the generation lock: longer than a
/// live worker lets its engine run, so a lock held past it has no worker.
const WAITER_PATIENCE: Duration = Duration::from_secs(32 * 60);
/// How long a waiter waits for an ended engine to release the lock.
const KILL_GRACE: Duration = Duration::from_secs(30);

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
    Mapped { finished_at_ms: u64 },
    Failed { finished_at_ms: u64, error: String },
}

/// What `generation.json` holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Record {
    sha: CommitSha,
    started_at_ms: u64,
    outcome: Outcome,
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
/// time while it holds the generation lock; leaves at once when another
/// worker already waits for it. `now` is the invocation's clock.
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
        let Some(held) = files.wait_for_generation_lock(project)? else {
            return Ok(());
        };
        drop(waiting);
        while let Some(queued) = files.take()? {
            generate(home, project, &files, &queued, &held, now)?;
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
    let free = files.try_generation_lock()?;
    let Some(record) = files.read::<Record>(&files.status())? else {
        return Ok(None);
    };
    let state = match record.outcome {
        Outcome::Mapped { .. } => GenerationState::Mapped,
        Outcome::Failed { error, .. } => GenerationState::Failed { error },
        Outcome::Running if free.is_some() => GenerationState::Interrupted,
        Outcome::Running => GenerationState::Running,
    };
    drop(free);
    Ok(Some(LastGeneration {
        sha: record.sha,
        state,
    }))
}

/// Generates one revision and records its outcome. Only a failure to
/// record is an error; a failed generation is an outcome.
fn generate(
    home: &Home,
    project: &ProjectSlug,
    files: &Files,
    queued: &Queued,
    held: &File,
    now: &dyn Fn() -> Timestamp,
) -> Result<(), GenerationError> {
    let now_ms = || u64::try_from(now().as_millisecond()).unwrap_or_default();
    let started_at_ms = now_ms();
    files.record(&Record {
        sha: queued.sha.clone(),
        started_at_ms,
        outcome: Outcome::Running,
    })?;
    let outcome = match map(home, project, files, queued, held) {
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
    files.record(&Record {
        sha: queued.sha.clone(),
        started_at_ms,
        outcome,
    })
}

/// Makes the revision's complete graph the latest graph: the stored one,
/// or one the engine produces from its committed tree.
fn map(
    home: &Home,
    project: &ProjectSlug,
    files: &Files,
    queued: &Queued,
    held: &File,
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
            let group = files.dir.join(ENGINE_GROUP_FILE);
            let generated =
                engine.generate(&tree, scratch.path(), ENGINE_TIMEOUT, held, &mut |pid| {
                    if let Err(err) = fsx::atomic_write(&group, pid.to_string().as_bytes()) {
                        tracing::warn!(event = "blueprint_engine_group_unrecorded", error = %err);
                    }
                });
            remove_file(&group);
            generated?
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
        // Readable: it becomes the engine's standard input.
        OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&path)
            .map_err(|source| GenerationError::Read { path, source })
    }

    /// The generation lock if nobody holds it: the open file holding it.
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

    /// Waits for the generation lock (see "Waiting" above); `None` when an
    /// engine left without its worker could not be ended.
    fn wait_for_generation_lock(
        &self,
        project: &ProjectSlug,
    ) -> Result<Option<File>, GenerationError> {
        if let Some(held) = self.wait(WAITER_PATIENCE)? {
            return Ok(Some(held));
        }
        let group = std::fs::read_to_string(self.dir.join(ENGINE_GROUP_FILE))
            .ok()
            .and_then(|text| text.trim().parse::<u32>().ok());
        tracing::warn!(
            event = "blueprint_orphaned_engine_ended",
            project = %project,
            group = ?group,
            "the generation lock outlived any worker's bound; ending the engine holding it"
        );
        if let Some(group) = group {
            kill_group(group);
        }
        let held = self.wait(KILL_GRACE)?;
        if held.is_none() {
            tracing::warn!(
                event = "blueprint_worker_gave_up",
                project = %project,
                "the generation lock is still held; the queued revision waits for the next handoff"
            );
        }
        Ok(held)
    }

    fn wait(&self, patience: Duration) -> Result<Option<File>, GenerationError> {
        let started = Instant::now();
        let mut pause = Duration::from_millis(5);
        loop {
            if let Some(held) = self.try_generation_lock()? {
                return Ok(Some(held));
            }
            if started.elapsed() >= patience {
                return Ok(None);
            }
            std::thread::sleep(pause);
            pause = (pause * 2).min(Duration::from_millis(200));
        }
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

/// Removes a file; one already gone is fine.
fn remove_file(path: &Path) {
    if let Err(err) = std::fs::remove_file(path)
        && err.kind() != io::ErrorKind::NotFound
    {
        tracing::warn!(event = "blueprint_file_not_removed", path = %path.display(), error = %err);
    }
}
