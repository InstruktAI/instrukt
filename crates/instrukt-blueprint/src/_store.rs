//! The project graph store: complete graphs per SHA in the home, the choice
//! of the latest one, and the one-step publication that makes a new graph
//! the latest (`docs/project/spec/domain/blueprint.md`, "Project graph store").
//!
//! # Layout
//!
//! Under `Home::blueprint_dir(project)` (`~/.instrukt/state/blueprint/<project>/`):
//!
//! ```text
//! <sha>/graph.json   one complete, contract-valid graph per revision
//! latest.json        {"latest": "<sha>", "previous": "<sha>"} — the pointer
//! .staging-<sha>/    a graph being written; never read
//! .lock              held exclusively for a whole publication
//! .prune.lock         shared by readers while they resolve; exclusive while pruning
//! ```
//!
//! # Invariants
//!
//! - A `<sha>/` directory only ever appears complete: its graph is written
//!   and synced in `.staging-<sha>/`, which is then renamed into place. A
//!   `<sha>/` whose graph does not satisfy the contract counts as absent and
//!   is replaced by the next publication of that revision.
//! - The latest graph changes in one step: `latest.json` is replaced
//!   atomically, so a reader sees the old pointer or the new one.
//! - Only a graph that satisfies the contract is published; a failed or
//!   interrupted publication leaves the pointer and the graphs it names as
//!   they were.
//! - After a publication only the latest graph and the one before it remain;
//!   older `<sha>/` directories are removed. The pointer swap is the commit
//!   point: a failure to remove an old graph afterwards does not undo the
//!   publication and is reported in [`Published`].
//! - A reader sees the old graph or the new one, never a failure from a
//!   swap. It holds `.prune.lock` shared from reading the pointer until the
//!   graph file it names is open; pruning holds it exclusively, so a graph a
//!   reader resolved cannot be removed before the reader has it open. Once
//!   open, the file stays readable even if pruning then unlinks it. Readers
//!   do not wait for a publication's writes, only for pruning itself.

use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use instrukt_core::fsx::{self, FsError, LockGuard, SharedLockGuard};
use instrukt_core::home::Home;
use instrukt_core::ids::{CommitSha, ProjectSlug};
use serde::{Deserialize, Serialize};

use crate::_contract::{ContractError, Graph};

/// How long publishing waits for another publish of the same project's
/// store. The lock covers the whole publication: serializing and writing
/// the graph with its `sync_all`, the rename into place, the pointer swap
/// and pruning.
const PUBLISH_LOCK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// How long a reader waits for pruning, and pruning for readers, on
/// `.prune.lock`; both hold it only for a few filesystem calls.
const PRUNE_LOCK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

const GRAPH_FILE: &str = "graph.json";
const POINTER_FILE: &str = "latest.json";
const LOCK_FILE: &str = ".lock";
const PRUNE_LOCK_FILE: &str = ".prune.lock";
const STAGING_PREFIX: &str = ".staging-";

/// The store could not be read or written.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("cannot read {path}: {source}")]
    Read { path: PathBuf, source: io::Error },
    #[error("cannot write {path}: {source}")]
    Write { path: PathBuf, source: io::Error },
    #[error("{path} is not a valid latest-graph pointer: {source}")]
    Pointer {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error("the graph of {sha} does not satisfy the contract: {source}")]
    Contract {
        sha: CommitSha,
        source: ContractError,
    },
    #[error("{path} is not a directory; the blueprint store needs one there")]
    NotADirectory { path: PathBuf },
    #[error(transparent)]
    Fs(#[from] FsError),
}

impl StoreError {
    /// Whether the error is about the latest graph itself — its pointer or
    /// its file missing or not satisfying the contract — rather than about
    /// storage that cannot be used. The spec answers the former as "no
    /// graph"; the latter is a failure to report.
    pub fn is_unreadable_graph(&self) -> bool {
        match self {
            Self::Contract { .. } | Self::Pointer { .. } => true,
            Self::Read { source, .. } => source.kind() == io::ErrorKind::NotFound,
            Self::Write { .. } | Self::NotADirectory { .. } | Self::Fs(_) => false,
        }
    }
}

/// The latest complete graph and the revision it reflects.
#[derive(Debug, Clone)]
pub struct LatestGraph {
    pub sha: CommitSha,
    pub graph: Graph,
}

/// A publication that committed: the graph is the latest. Old graphs that
/// could not be removed afterwards are listed; they cost space, not
/// correctness, and the next publication tries again.
#[derive(Debug, Default)]
#[must_use]
pub struct Published {
    pub cleanup_failures: Vec<CleanupFailure>,
}

/// An old graph directory pruning could not remove.
#[derive(Debug)]
pub struct CleanupFailure {
    pub path: PathBuf,
    pub error: io::Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Pointer {
    latest: CommitSha,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    previous: Option<CommitSha>,
}

/// One project's graph store.
#[derive(Debug, Clone)]
pub struct GraphStore {
    dir: PathBuf,
}

impl GraphStore {
    /// The store of `project` in `home`. Touches nothing on disk.
    pub fn new(home: &Home, project: &ProjectSlug) -> Self {
        Self {
            dir: home.blueprint_dir(project),
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The latest complete graph; `None` when the project has none yet (no
    /// store, or a store without a pointer). An error means the store cannot
    /// be used, or a pointer exists whose graph cannot be read or does not
    /// satisfy the contract ([`StoreError::is_unreadable_graph`]).
    pub fn latest(&self) -> Result<Option<LatestGraph>, StoreError> {
        if !self.store_exists()? {
            return Ok(None);
        }
        let (sha, mut file, path) = {
            let _shared =
                SharedLockGuard::acquire(&self.dir.join(PRUNE_LOCK_FILE), PRUNE_LOCK_TIMEOUT)?;
            let Some(pointer) = self.pointer()? else {
                return Ok(None);
            };
            let path = self.graph_file(&pointer.latest);
            let file = std::fs::File::open(&path).map_err(|source| StoreError::Read {
                path: path.clone(),
                source,
            })?;
            (pointer.latest, file, path)
        };
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)
            .map_err(|source| StoreError::Read { path, source })?;
        let graph = Graph::from_json(&bytes).map_err(|source| StoreError::Contract {
            sha: sha.clone(),
            source,
        })?;
        Ok(Some(LatestGraph { sha, graph }))
    }

    /// Whether a complete graph of `sha` is stored: present, readable and
    /// satisfying the contract. A missing or contract-breaking graph is
    /// `false`; storage that cannot be read is an error.
    pub fn has_graph(&self, sha: &CommitSha) -> Result<bool, StoreError> {
        Ok(self.graph(sha)?.is_some())
    }

    /// The complete graph of `sha`; `None` when it is missing or does not
    /// satisfy the contract. Storage that cannot be read is an error.
    pub fn graph(&self, sha: &CommitSha) -> Result<Option<Graph>, StoreError> {
        let path = self.graph_file(sha);
        match std::fs::read(&path) {
            Ok(bytes) => Ok(Graph::from_json(&bytes).ok()),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(source) => Err(StoreError::Read { path, source }),
        }
    }

    /// `false` only when the store's directory genuinely does not exist;
    /// anything else at that path, or a failure to look, is an error.
    fn store_exists(&self) -> Result<bool, StoreError> {
        match std::fs::metadata(&self.dir) {
            Ok(meta) if meta.is_dir() => Ok(true),
            Ok(_) => Err(StoreError::NotADirectory {
                path: self.dir.clone(),
            }),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(source) => Err(StoreError::Read {
                path: self.dir.clone(),
                source,
            }),
        }
    }

    /// Makes `graph` the complete graph of `sha` and the latest graph, in
    /// one step, then removes every graph but the latest and the one before
    /// it. A complete graph of `sha` already stored is kept as it is; one
    /// that does not satisfy the contract is replaced by `graph`. Serialized
    /// with other publications of the same project by the store's lock.
    ///
    /// `Ok` means the pointer now names `sha`; `Err` means it is unchanged.
    /// Pruning failures after the swap are reported in [`Published`].
    pub fn publish(&self, sha: &CommitSha, graph: &Graph) -> Result<Published, StoreError> {
        // Refuse storage that is not a directory before creating anything.
        self.store_exists()?;
        let _lock = LockGuard::acquire(&self.dir.join(LOCK_FILE), PUBLISH_LOCK_TIMEOUT)?;
        if !self.has_graph(sha)? {
            self.write_complete(sha, graph)?;
        }
        let previous = match self.pointer() {
            Ok(Some(old)) if old.latest == *sha => old.previous,
            Ok(Some(old)) => Some(old.latest),
            Ok(None) => None,
            Err(err) => {
                // The store is a cache: an unreadable pointer is replaced.
                tracing::warn!(
                    event = "blueprint_pointer_replaced",
                    store = %self.dir.display(),
                    error = %err,
                    "replacing an unreadable latest-graph pointer"
                );
                None
            }
        };
        let pointer = Pointer {
            latest: sha.clone(),
            previous,
        };
        let bytes = serde_json::to_vec_pretty(&pointer).map_err(|source| StoreError::Pointer {
            path: self.dir.join(POINTER_FILE),
            source,
        })?;
        fsx::atomic_write(&self.dir.join(POINTER_FILE), &bytes)?;
        Ok(self.prune(&pointer))
    }

    fn write_complete(&self, sha: &CommitSha, graph: &Graph) -> Result<(), StoreError> {
        let bytes = graph.to_json().map_err(|source| StoreError::Contract {
            sha: sha.clone(),
            source,
        })?;
        let staging = self.dir.join(format!("{STAGING_PREFIX}{sha}"));
        let target = self.dir.join(sha.as_str());
        // Left behind by an interrupted publication; we hold the lock.
        remove_if_present(&staging)?;
        std::fs::create_dir_all(&staging).map_err(write(&staging))?;
        let file_path = staging.join(GRAPH_FILE);
        let mut file = std::fs::File::create(&file_path).map_err(write(&file_path))?;
        file.write_all(&bytes).map_err(write(&file_path))?;
        file.sync_all().map_err(write(&file_path))?;
        drop(file);
        // A directory without a complete graph is not a graph.
        remove_if_present(&target)?;
        std::fs::rename(&staging, &target).map_err(write(&target))?;
        Ok(())
    }

    /// Removes every graph but the two `keep` names. Runs after the
    /// pointer swap, so it never fails the publication; what it could not
    /// remove is returned and logged.
    fn prune(&self, keep: &Pointer) -> Published {
        let mut published = Published::default();
        let lock_path = self.dir.join(PRUNE_LOCK_FILE);
        let _exclusive = match LockGuard::acquire(&lock_path, PRUNE_LOCK_TIMEOUT) {
            Ok(guard) => guard,
            Err(err) => {
                published.cleanup_failures.push(CleanupFailure {
                    path: lock_path,
                    error: io::Error::other(err),
                });
                return self.logged(published);
            }
        };
        let entries = match std::fs::read_dir(&self.dir) {
            Ok(entries) => entries,
            Err(error) => {
                published.cleanup_failures.push(CleanupFailure {
                    path: self.dir.clone(),
                    error,
                });
                return self.logged(published);
            }
        };
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    published.cleanup_failures.push(CleanupFailure {
                        path: self.dir.clone(),
                        error,
                    });
                    continue;
                }
            };
            let name = entry.file_name();
            let Some(sha) = name.to_str().and_then(|n| CommitSha::parse(n).ok()) else {
                continue;
            };
            if sha == keep.latest || Some(&sha) == keep.previous.as_ref() {
                continue;
            }
            let path = entry.path();
            if let Err(error) = std::fs::remove_dir_all(&path) {
                published
                    .cleanup_failures
                    .push(CleanupFailure { path, error });
            }
        }
        self.logged(published)
    }

    fn logged(&self, published: Published) -> Published {
        for failure in &published.cleanup_failures {
            tracing::warn!(
                event = "blueprint_prune_failed",
                path = %failure.path.display(),
                error = %failure.error,
                "an old graph could not be removed after publication"
            );
        }
        published
    }

    fn pointer(&self) -> Result<Option<Pointer>, StoreError> {
        let path = self.dir.join(POINTER_FILE);
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(source) => return Err(StoreError::Read { path, source }),
        };
        serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|source| StoreError::Pointer { path, source })
    }

    fn graph_file(&self, sha: &CommitSha) -> PathBuf {
        self.dir.join(sha.as_str()).join(GRAPH_FILE)
    }
}

/// Removes the directory at `path`; one that does not exist is fine.
fn remove_if_present(path: &Path) -> Result<(), StoreError> {
    match std::fs::remove_dir_all(path) {
        Err(err) if err.kind() != io::ErrorKind::NotFound => Err(write(path)(err)),
        _ => Ok(()),
    }
}

fn write(path: &Path) -> impl Fn(io::Error) -> StoreError + '_ {
    move |source| StoreError::Write {
        path: path.to_path_buf(),
        source,
    }
}

#[cfg(test)]
mod tests {
    use notify::Watcher;
    use serde_json::json;

    use super::*;

    fn sha(c: char) -> CommitSha {
        CommitSha::parse(&c.to_string().repeat(40)).unwrap()
    }

    fn graph(marker: &str) -> Graph {
        Graph::from_value(json!({
            "marker": marker,
            "nodes": [{"id": marker, "source_file": "a.rs"}],
            "links": []
        }))
        .unwrap()
    }

    fn store() -> (tempfile::TempDir, GraphStore) {
        let dir = tempfile::tempdir().unwrap();
        let home = Home::at_user_home(dir.path().to_path_buf());
        let store = GraphStore::new(&home, &ProjectSlug::parse("demo").unwrap());
        (dir, store)
    }

    /// Publishes and requires a clean publication.
    fn publish_ok(store: &GraphStore, sha: &CommitSha, graph: &Graph) {
        let published = store.publish(sha, graph).unwrap();
        assert!(published.cleanup_failures.is_empty(), "{published:?}");
    }

    fn shas_on_disk(store: &GraphStore) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(store.dir())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| CommitSha::parse(n).is_ok())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn an_empty_store_has_no_latest_graph() {
        let (_dir, store) = store();
        assert!(store.latest().unwrap().is_none());
    }

    #[test]
    fn publishing_makes_the_graph_latest_and_names_its_sha() {
        let (_dir, store) = store();
        publish_ok(&store, &sha('a'), &graph("first"));
        let latest = store.latest().unwrap().unwrap();
        assert_eq!(latest.sha, sha('a'));
        assert_eq!(latest.graph.nodes()[0].id, "first");
        assert!(store.has_graph(&sha('a')).unwrap());
        assert!(!store.has_graph(&sha('b')).unwrap());
    }

    #[test]
    fn only_the_latest_and_the_one_before_are_kept() {
        let (_dir, store) = store();
        publish_ok(&store, &sha('a'), &graph("a"));
        publish_ok(&store, &sha('b'), &graph("b"));
        publish_ok(&store, &sha('c'), &graph("c"));
        assert_eq!(
            shas_on_disk(&store),
            [sha('b').to_string(), sha('c').to_string()]
        );
        // Republishing the latest keeps the one before.
        publish_ok(&store, &sha('c'), &graph("ignored"));
        assert_eq!(
            shas_on_disk(&store),
            [sha('b').to_string(), sha('c').to_string()]
        );
        assert_eq!(store.latest().unwrap().unwrap().graph.nodes()[0].id, "c");
    }

    #[test]
    fn an_interrupted_publication_leaves_the_latest_graph_in_place() {
        let (_dir, store) = store();
        publish_ok(&store, &sha('a'), &graph("a"));
        // A staging directory and a half-made sha directory, as a crash
        // before the rename or a stray copy would leave them.
        std::fs::create_dir_all(store.dir().join(format!(".staging-{}", sha('b')))).unwrap();
        std::fs::write(
            store
                .dir()
                .join(format!(".staging-{}", sha('b')))
                .join("graph.json"),
            b"{\"nodes\": [",
        )
        .unwrap();
        let latest = store.latest().unwrap().unwrap();
        assert_eq!(latest.sha, sha('a'));
        // The next publication of that revision completes normally.
        publish_ok(&store, &sha('b'), &graph("b"));
        assert_eq!(store.latest().unwrap().unwrap().sha, sha('b'));
        assert!(!store.dir().join(format!(".staging-{}", sha('b'))).exists());
    }

    #[test]
    fn a_graph_breaking_the_contract_on_disk_is_unreadable_not_absent() {
        let (_dir, store) = store();
        publish_ok(&store, &sha('a'), &graph("a"));
        std::fs::write(
            store.dir().join(sha('a').as_str()).join("graph.json"),
            b"[]",
        )
        .unwrap();
        assert!(matches!(store.latest(), Err(StoreError::Contract { .. })));
        std::fs::remove_dir_all(store.dir().join(sha('a').as_str())).unwrap();
        assert!(matches!(store.latest(), Err(StoreError::Read { .. })));
    }

    #[test]
    fn an_invalid_graph_already_on_disk_is_replaced_not_promoted() {
        let (_dir, store) = store();
        publish_ok(&store, &sha('a'), &graph("a"));
        let stray = store.dir().join(sha('b').as_str());
        std::fs::create_dir_all(&stray).unwrap();
        std::fs::write(stray.join("graph.json"), b"[]").unwrap();
        assert!(!store.has_graph(&sha('b')).unwrap());

        publish_ok(&store, &sha('b'), &graph("b"));
        let latest = store.latest().unwrap().unwrap();
        assert_eq!(latest.sha, sha('b'));
        assert_eq!(latest.graph.nodes()[0].id, "b");
    }

    #[cfg(unix)]
    #[test]
    fn a_pruning_failure_after_the_swap_is_reported_and_the_publication_stands() {
        use std::os::unix::fs::PermissionsExt;

        let (_dir, store) = store();
        publish_ok(&store, &sha('a'), &graph("a"));
        publish_ok(&store, &sha('b'), &graph("b"));
        let old = store.dir().join(sha('a').as_str());
        std::fs::set_permissions(&old, std::fs::Permissions::from_mode(0o555)).unwrap();

        let published = store.publish(&sha('c'), &graph("c"));
        std::fs::set_permissions(&old, std::fs::Permissions::from_mode(0o755)).unwrap();

        let published = published.unwrap();
        let failed: Vec<_> = published
            .cleanup_failures
            .iter()
            .map(|f| f.path.clone())
            .collect();
        assert_eq!(failed, [old]);
        assert_eq!(store.latest().unwrap().unwrap().sha, sha('c'));
        // The next publication removes what was left.
        publish_ok(&store, &sha('d'), &graph("d"));
        assert_eq!(
            shas_on_disk(&store),
            [sha('c').to_string(), sha('d').to_string()]
        );
    }

    #[test]
    fn a_graph_stays_readable_while_a_reader_holds_the_prune_lock() {
        let (_dir, store) = store();
        publish_ok(&store, &sha('a'), &graph("a"));
        publish_ok(&store, &sha('b'), &graph("b"));
        // A reader between reading the pointer (naming `a` before `b` was
        // published, say) and opening its graph holds this lock shared.
        let reader = SharedLockGuard::acquire(
            &store.dir().join(PRUNE_LOCK_FILE),
            std::time::Duration::from_secs(5),
        )
        .unwrap();
        // Arm before publication so even an immediate pointer swap is observed.
        let (send, events) = std::sync::mpsc::channel();
        let mut watcher =
            notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                if !event.as_ref().is_ok_and(|event| event.kind.is_access()) {
                    // The test may have completed when a final event is delivered.
                    let _ = send.send(event);
                }
            })
            .unwrap();
        watcher
            .watch(store.dir(), notify::RecursiveMode::NonRecursive)
            .unwrap();
        // Publishing `c` would prune `a`.
        let publisher = store.clone();
        let publication = std::thread::spawn(move || publisher.publish(&sha('c'), &graph("c")));

        // The swap is not held up by readers: `c` becomes the latest graph.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while store.latest().unwrap().map(|latest| latest.sha) != Some(sha('c')) {
            assert!(
                std::time::Instant::now() < deadline,
                "the publication never swapped"
            );
            events
                .recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
                .unwrap()
                .unwrap();
        }
        // Pruning waits behind the reader: `a` is still a complete graph.
        assert!(store.has_graph(&sha('a')).unwrap());
        assert!(!publication.is_finished());

        drop(reader);
        let published = publication.join().unwrap().unwrap();
        assert!(published.cleanup_failures.is_empty(), "{published:?}");
        assert!(!store.has_graph(&sha('a')).unwrap());
        assert_eq!(
            shas_on_disk(&store),
            [sha('b').to_string(), sha('c').to_string()]
        );
    }

    #[test]
    fn a_file_where_the_store_belongs_is_an_error_not_an_empty_store() {
        let (_dir, store) = store();
        std::fs::create_dir_all(store.dir().parent().unwrap()).unwrap();
        std::fs::write(store.dir(), b"not a store").unwrap();
        let latest = store.latest();
        assert!(
            matches!(latest, Err(StoreError::NotADirectory { .. })),
            "{latest:?}"
        );
        assert!(!latest.unwrap_err().is_unreadable_graph());
        let published = store.publish(&sha('a'), &graph("a"));
        assert!(
            matches!(published, Err(StoreError::NotADirectory { .. })),
            "{published:?}"
        );
        // Above the store, a file makes the path itself unusable.
        let (_dir, store) = self::store();
        let state = store.dir().parent().unwrap().parent().unwrap();
        std::fs::create_dir_all(state.parent().unwrap()).unwrap();
        std::fs::write(state, b"not a directory").unwrap();
        let latest = store.latest();
        assert!(matches!(latest, Err(StoreError::Read { .. })), "{latest:?}");
        assert!(!latest.unwrap_err().is_unreadable_graph());
    }

    #[test]
    fn a_missing_graph_the_pointer_still_names_is_unreadable() {
        let (_dir, store) = store();
        publish_ok(&store, &sha('a'), &graph("a"));
        std::fs::remove_dir_all(store.dir().join(sha('a').as_str())).unwrap();
        assert!(matches!(store.latest(), Err(StoreError::Read { .. })));
    }
}
