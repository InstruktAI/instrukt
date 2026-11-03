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
//! .generation.lock   held by the worker generating; one per project
//! .checkout-*/       a running generation's checkout of its revision
//! ```
//!
//! [`Generation::start_generation`] writes the revision as the queued one
//! and starts a detached worker (`instrukt blueprint worker <project>`).
//! The worker takes `.generation.lock`; if another worker holds it, that
//! one will take the queued revision after its current one, and this
//! worker leaves. The holder generates queued revisions one at a time
//! until the queue is empty, releases the lock and looks once more, so a
//! revision queued while it was releasing is never left behind.
//!
//! A generation checks the revision's committed tree out into a private
//! directory with git plumbing (never the working tree, no hooks), runs the
//! engine there, validates its output and publishes it through
//! [`GraphStore::publish`] — so a failed, refused or interrupted run never
//! replaces the latest graph. A revision whose complete graph is stored is
//! not generated again; it is made the latest graph again.

use std::io;
use std::os::unix::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use instrukt_core::config::{ConfigError, Configuration};
use instrukt_core::fsx::{self, FsError, LockGuard};
use instrukt_core::git;
use instrukt_core::home::Home;
use instrukt_core::ids::{CommitSha, ProjectSlug};
use instrukt_core::interfaces::{GenerationRequest, GraphGeneration, InterfaceError, MainAdvance};
use serde::{Deserialize, Serialize};

use crate::_engine::{Engine, EngineError};
use crate::_store::{GraphStore, StoreError};

/// The arguments, after the executable, that start the detached worker;
/// the project slug follows. The CLI routes them to [`run_worker`].
pub const WORKER_COMMAND: [&str; 2] = ["blueprint", "worker"];

const QUEUE_FILE: &str = "queue.json";
const STATUS_FILE: &str = "generation.json";
const QUEUE_LOCK_FILE: &str = ".queue.lock";
const GENERATION_LOCK_FILE: &str = ".generation.lock";
const CHECKOUT_PREFIX: &str = ".checkout-";

/// How long the queue's lock is waited for; it is held for one small read
/// or write.
const QUEUE_LOCK_TIMEOUT: Duration = Duration::from_secs(10);
/// How long a starting worker waits for the generation lock before
/// leaving the queued revision to the worker that holds it.
const WORKER_LOCK_WAIT: Duration = Duration::from_secs(2);
/// How long one engine run may take before it is ended.
const ENGINE_TIMEOUT: Duration = Duration::from_secs(30 * 60);

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
/// worker holds it.
pub fn run_worker(home: &Home, project: &ProjectSlug) -> Result<(), GenerationError> {
    let files = Files::new(home, project);
    loop {
        let lock = match LockGuard::acquire(&files.generation_lock(), WORKER_LOCK_WAIT) {
            Ok(lock) => lock,
            Err(FsError::LockTimeout { .. }) => {
                tracing::debug!(
                    event = "blueprint_worker_yielded",
                    project = %project,
                    "another worker generates; it takes the queued revision"
                );
                return Ok(());
            }
            Err(err) => return Err(err.into()),
        };
        while let Some(queued) = files.take()? {
            generate(home, project, &files, &queued)?;
        }
        drop(lock);
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
    let Some(record) = files.read::<Record>(&files.status())? else {
        return Ok(None);
    };
    let state = match record.outcome {
        Outcome::Mapped { .. } => GenerationState::Mapped,
        Outcome::Failed { error, .. } => GenerationState::Failed { error },
        // Running only while a worker holds the generation lock.
        Outcome::Running => match LockGuard::try_acquire(&files.generation_lock())? {
            Some(_free) => GenerationState::Interrupted,
            None => GenerationState::Running,
        },
    };
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
) -> Result<(), GenerationError> {
    let started_at_ms = now_ms();
    files.record(&Record {
        sha: queued.sha.clone(),
        started_at_ms,
        outcome: Outcome::Running,
    })?;
    let outcome = match map(home, project, files, queued) {
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
            engine.generate(&tree, scratch.path(), ENGINE_TIMEOUT)?
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

    fn generation_lock(&self) -> PathBuf {
        self.dir.join(GENERATION_LOCK_FILE)
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

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sha(c: char) -> CommitSha {
        CommitSha::parse(&c.to_string().repeat(40)).unwrap()
    }

    fn files() -> (tempfile::TempDir, Home, ProjectSlug, Files) {
        let dir = tempfile::tempdir().unwrap();
        let home = Home::at_user_home(dir.path().to_path_buf());
        let project = ProjectSlug::parse("demo").unwrap();
        let files = Files::new(&home, &project);
        (dir, home, project, files)
    }

    fn queued(c: char) -> Queued {
        Queued {
            sha: sha(c),
            project_root: PathBuf::from("/repo"),
            advance: Advance::Push,
        }
    }

    #[test]
    fn a_newer_handoff_replaces_the_queued_revision_and_taking_empties_the_queue() {
        let (_dir, _home, _project, files) = files();
        files.enqueue(&queued('a')).unwrap();
        files.enqueue(&queued('b')).unwrap();
        assert_eq!(files.take().unwrap(), Some(queued('b')));
        assert_eq!(files.take().unwrap(), None);
    }

    #[test]
    fn a_running_record_without_a_worker_holding_the_lock_is_interrupted() {
        let (_dir, home, project, files) = files();
        assert_eq!(last_generation(&home, &project).unwrap(), None);
        files
            .record(&Record {
                sha: sha('a'),
                started_at_ms: 1,
                outcome: Outcome::Running,
            })
            .unwrap();
        let held = LockGuard::try_acquire(&files.generation_lock())
            .unwrap()
            .unwrap();
        assert_eq!(
            last_generation(&home, &project).unwrap().unwrap().state,
            GenerationState::Running
        );
        drop(held);
        // A child another test forks in this process may briefly hold a
        // copy of the lock's descriptor until it execs; wait that out.
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let last = loop {
            let last = last_generation(&home, &project).unwrap().unwrap();
            if last.state != GenerationState::Running || std::time::Instant::now() > deadline {
                break last;
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        assert_eq!(last.state, GenerationState::Interrupted);
        assert!(last.is_failure());
    }
}
