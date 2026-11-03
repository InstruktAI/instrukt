//! The runner's files in `state/runner/` (architecture overview, "The
//! home"): the event log and its cursors, conditional-request validators,
//! watched-ref observations, standing-agent session bindings, the runner
//! lock and the last wake-up.
//!
//! Every path the runner writes is named here. Writes are whole-file and
//! atomic, and refuse a target that is a symbolic link: the runner never
//! follows a link planted in the home.

use std::path::{Path, PathBuf};
use std::time::Duration;

use instrukt_core::fsx::{self, FsError, LockGuard};
use instrukt_core::home::Home;
use instrukt_core::ids::ProjectSlug;
use serde::Serialize;
use serde::de::DeserializeOwned;

/// How long a writer waits for another process holding a state lock.
pub(crate) const STATE_LOCK_WAIT: Duration = Duration::from_secs(10);

/// A runner state file could not be read or written.
#[derive(Debug, thiserror::Error)]
pub enum StateError {
    #[error("cannot read {}: {source}", path.display())]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{} is not readable runner state: {message}", path.display())]
    Corrupt { path: PathBuf, message: String },
    #[error("{} is a symbolic link; the runner does not write through links", path.display())]
    Symlink { path: PathBuf },
    #[error(transparent)]
    Fs(#[from] FsError),
}

/// Paths into one home's runner state.
#[derive(Debug, Clone)]
pub struct RunnerState {
    dir: PathBuf,
}

impl RunnerState {
    pub fn new(home: &Home) -> Self {
        Self {
            dir: home.runner_state_dir(),
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The machine's runner lock: one runner invocation at a time.
    pub fn runner_lock(&self) -> PathBuf {
        self.dir.join("runner.lock")
    }

    /// The lock serializing event-log and cursor writes.
    pub(crate) fn events_lock(&self) -> PathBuf {
        self.dir.join("events.lock")
    }

    /// The event log, one JSON record per line.
    pub(crate) fn events_log(&self) -> PathBuf {
        self.dir.join("events.jsonl")
    }

    /// Per-consumer cursors into the event log.
    pub(crate) fn cursors(&self) -> PathBuf {
        self.dir.join("cursors.json")
    }

    /// The last wake-up.
    pub(crate) fn last_wakeup(&self) -> PathBuf {
        self.dir.join("last-wakeup")
    }

    /// The watched refs last observed for `project`.
    pub(crate) fn watched(&self, project: &ProjectSlug) -> PathBuf {
        self.dir.join("watch").join(format!("{project}.json"))
    }

    /// The conditional-request validators and poll intervals for `project`.
    pub(crate) fn validators(&self, project: &ProjectSlug) -> PathBuf {
        self.dir.join("validators").join(format!("{project}.json"))
    }

    /// The executions the runner dispatched and watches.
    pub(crate) fn executions(&self) -> PathBuf {
        self.dir.join("executions.json")
    }

    /// Standing-agent starts begun and not yet bound.
    pub(crate) fn standing_starts(&self) -> PathBuf {
        self.dir.join("standing-starts.json")
    }

    /// The failed task firings already reported as anomalies.
    pub(crate) fn failed_firings(&self) -> PathBuf {
        self.dir.join("failed-firings.json")
    }

    /// Standing-agent session bindings, keyed by (procedure, project).
    pub(crate) fn standing(&self) -> PathBuf {
        self.dir.join("standing.json")
    }
}

/// Reads JSON state at `path`; `None` when the file does not exist. Any
/// other failure is an error: only "not found" is absence.
pub(crate) fn read_json<T: DeserializeOwned>(path: &Path) -> Result<Option<T>, StateError> {
    let Some(text) = read_text(path)? else {
        return Ok(None);
    };
    serde_json::from_str(&text)
        .map(Some)
        .map_err(|err| StateError::Corrupt {
            path: path.to_path_buf(),
            message: err.to_string(),
        })
}

/// Reads text state at `path`; `None` when the file does not exist.
pub(crate) fn read_text(path: &Path) -> Result<Option<String>, StateError> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(StateError::Read {
            path: path.to_path_buf(),
            source,
        }),
    }
}

/// Writes `value` as JSON to `path`, atomically.
pub(crate) fn write_json<T: Serialize + ?Sized>(path: &Path, value: &T) -> Result<(), StateError> {
    let text = serde_json::to_string_pretty(value).map_err(|err| StateError::Corrupt {
        path: path.to_path_buf(),
        message: err.to_string(),
    })?;
    write_bytes(path, text.as_bytes())
}

/// Writes `bytes` to `path` atomically, refusing a symbolic link there.
pub(crate) fn write_bytes(path: &Path, bytes: &[u8]) -> Result<(), StateError> {
    refuse_symlink(path)?;
    fsx::atomic_write(path, bytes)?;
    Ok(())
}

/// Refuses `path` when it is a symbolic link; a missing path is fine.
pub(crate) fn refuse_symlink(path: &Path) -> Result<(), StateError> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => Err(StateError::Symlink {
            path: path.to_path_buf(),
        }),
        Ok(_) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(StateError::Read {
            path: path.to_path_buf(),
            source,
        }),
    }
}

/// Takes the exclusive lock at `path`, waiting a bounded time.
pub(crate) fn lock(path: &Path) -> Result<LockGuard, StateError> {
    refuse_symlink(path)?;
    Ok(LockGuard::acquire(path, STATE_LOCK_WAIT)?)
}
