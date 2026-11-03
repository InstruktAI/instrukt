//! The local work record in `~/.instrukt/state/work/`: one file per todo
//! of a project, written whole and atomically, under the per-computer
//! writer lock that also serializes reflection (`github-record`, "Writer
//! and convergence").

use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use instrukt_core::fsx::{self, LockGuard};
use instrukt_core::home::Home;
use instrukt_core::ids::ProjectSlug;
use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use super::_error::WorkError;
use super::_model::{TodoId, TodoRecord};

/// How long a command waits for another instrukt process holding the
/// writer lock.
const LOCK_TIMEOUT: Duration = Duration::from_secs(60);

/// The writer lock's file, below `state/work/`.
const LOCK_FILE: &str = ".writer.lock";

/// A project's reflection bookkeeping file, below its directory.
const PROVIDER_FILE: &str = "provider.json";

/// A local item: the record and what this computer knows of its GitHub
/// reflection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct LocalItem {
    pub record: TodoRecord,
    #[serde(default)]
    pub reflection: ReflectionState,
}

/// What this computer knows of one item's reflection.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ReflectionState {
    /// The issue's number and REST id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issue: Option<IssueRef>,
    /// The comment carrying the committed revision this computer last
    /// wrote or read: the head of the issue's revision chain then.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub record_comment: Option<u64>,
    /// The issue's `ETag` at the last read, for conditional pulls: a new
    /// revision comment moves it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub etag: Option<String>,
    /// The confirmed remote revision: the record revision GitHub holds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_revision: Option<u64>,
    /// Why the linked pull request's creation, reviews or closing are still
    /// outstanding.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pull_outstanding: Option<String>,
    /// Comment ids of inbound `/instrukt` commands already answered.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub commands_seen: Vec<u64>,
    /// Local changes not yet reflected.
    #[serde(default)]
    pub pending: bool,
    /// A newer remote revision met a pending local change.
    #[serde(default)]
    pub conflicted: bool,
    /// The linked pull request's number.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pull: Option<u64>,
    /// How many build-review verdicts the pull request carries.
    #[serde(default)]
    pub reviews_reflected: usize,
    /// Whether delivery closed the pull request.
    #[serde(default)]
    pub pull_closed: bool,
    /// The last reflection failure that left the item pending.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct IssueRef {
    pub number: u64,
    pub id: u64,
}

/// A project's provider bookkeeping.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ProviderState {
    /// GitHub deferred requests until then (rate limiting).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deferred_until: Option<Timestamp>,
}

/// The writer lock, held for a whole command or runner call.
pub(crate) fn lock(home: &Home) -> Result<LockGuard, WorkError> {
    let dir = home.work_state_dir();
    refuse_links(home, &dir)?;
    LockGuard::acquire(&dir.join(LOCK_FILE), LOCK_TIMEOUT).map_err(WorkError::from)
}

/// One project's directory of local items.
pub(crate) struct Store {
    dir: PathBuf,
}

impl Store {
    /// The project's store; checks that no part of its path inside the
    /// home is a symbolic link.
    pub fn open(home: &Home, project: &ProjectSlug) -> Result<Self, WorkError> {
        let dir = home.work_state_dir().join(project.as_str());
        refuse_links(home, &dir)?;
        Ok(Self { dir })
    }

    /// Every local item, in identity order. A missing directory is an
    /// empty store.
    pub fn items(&self) -> Result<Vec<LocalItem>, WorkError> {
        let entries = match std::fs::read_dir(&self.dir) {
            Ok(entries) => entries,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(source) => return Err(io_error(&self.dir, source)),
        };
        let mut paths = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|source| io_error(&self.dir, source))?;
            let path = entry.path();
            let is_item = path
                .file_name()
                .and_then(|name| name.to_str())
                .and_then(|name| name.strip_suffix(".json"))
                .is_some_and(|stem| TodoId::parse(stem).is_some());
            if is_item {
                paths.push(path);
            }
        }
        paths.sort();
        paths.iter().map(|path| read_item(path)).collect()
    }

    pub fn save(&self, item: &LocalItem) -> Result<(), WorkError> {
        let path = self.item_path(&item.record.id);
        refuse_link(&path)?;
        let text = serde_json::to_vec_pretty(item).map_err(|source| WorkError::Corrupt {
            path: path.clone(),
            message: source.to_string(),
        })?;
        fsx::atomic_write(&path, &text).map_err(WorkError::from)
    }

    pub fn delete(&self, id: &TodoId) -> Result<(), WorkError> {
        let path = self.item_path(id);
        refuse_link(&path)?;
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(source) => Err(io_error(&path, source)),
        }
    }

    pub fn provider(&self) -> Result<ProviderState, WorkError> {
        let path = self.dir.join(PROVIDER_FILE);
        refuse_link(&path)?;
        match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|source| WorkError::Corrupt {
                path,
                message: source.to_string(),
            }),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(ProviderState::default()),
            Err(source) => Err(io_error(&path, source)),
        }
    }

    pub fn save_provider(&self, state: &ProviderState) -> Result<(), WorkError> {
        let path = self.dir.join(PROVIDER_FILE);
        refuse_link(&path)?;
        let text = serde_json::to_vec_pretty(state).map_err(|source| WorkError::Corrupt {
            path: path.clone(),
            message: source.to_string(),
        })?;
        fsx::atomic_write(&path, &text).map_err(WorkError::from)
    }

    fn item_path(&self, id: &TodoId) -> PathBuf {
        self.dir.join(format!("{id}.json"))
    }
}

fn read_item(path: &Path) -> Result<LocalItem, WorkError> {
    refuse_link(path)?;
    let bytes = std::fs::read(path).map_err(|source| io_error(path, source))?;
    let mut item: LocalItem =
        serde_json::from_slice(&bytes).map_err(|source| WorkError::Corrupt {
            path: path.to_path_buf(),
            message: source.to_string(),
        })?;
    item.record.number_unsequenced();
    Ok(item)
}

/// Refuses a symbolic link at `dir` or any of its ancestors inside the
/// home (the home itself included).
fn refuse_links(home: &Home, dir: &Path) -> Result<(), WorkError> {
    let mut current = Some(dir);
    while let Some(path) = current {
        refuse_link(path)?;
        if path == home.root() {
            break;
        }
        current = path
            .parent()
            .filter(|parent| parent.starts_with(home.root()));
    }
    Ok(())
}

fn refuse_link(path: &Path) -> Result<(), WorkError> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => Err(WorkError::Symlink {
            path: path.to_path_buf(),
        }),
        Ok(_) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(io_error(path, source)),
    }
}

fn io_error(path: &Path, source: io::Error) -> WorkError {
    WorkError::Io {
        path: path.to_path_buf(),
        source,
    }
}
