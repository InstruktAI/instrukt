//! The project graph store: complete graphs per SHA in the home, the choice
//! of the latest one, and the one-step publication that makes a new graph
//! the latest (`docs/local/spec/domain/blueprint.md`, "Project graph store").
//!
//! # Layout
//!
//! Under `Home::blueprint_dir(project)` (`~/.instrukt/state/blueprint/<project>/`):
//!
//! ```text
//! <sha>/graph.json   one complete, contract-valid graph per revision
//! latest.json        {"latest": "<sha>", "previous": "<sha>"} — the pointer
//! .staging-<sha>/    a graph being written; never read
//! .lock              held while publishing
//! ```
//!
//! # Invariants
//!
//! - A `<sha>/` directory only ever appears complete: its graph is written
//!   and synced in `.staging-<sha>/`, which is then renamed into place.
//! - The latest graph changes in one step: `latest.json` is replaced
//!   atomically, so a reader sees the old pointer or the new one.
//! - Only a graph that satisfies the contract is published; a failed or
//!   interrupted publication leaves the pointer and the graphs it names as
//!   they were.
//! - After a publication only the latest graph and the one before it remain;
//!   older `<sha>/` directories are removed. A reader that resolved the
//!   pointer just before a publication still finds its graph, which became
//!   the one before.

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use instrukt_core::fsx::{self, FsError, LockGuard};
use instrukt_core::home::Home;
use instrukt_core::ids::{CommitSha, ProjectSlug};
use serde::{Deserialize, Serialize};

use crate::contract::{ContractError, Graph};

const GRAPH_FILE: &str = "graph.json";
const POINTER_FILE: &str = "latest.json";
const LOCK_FILE: &str = ".lock";
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
    #[error(transparent)]
    Fs(#[from] FsError),
}

/// The latest complete graph and the revision it reflects.
#[derive(Debug, Clone)]
pub struct LatestGraph {
    pub sha: CommitSha,
    pub graph: Graph,
}

#[derive(Debug, Serialize, Deserialize)]
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

    /// The latest complete graph; `None` when the project has none yet. An
    /// error means a pointer or graph exists but cannot be read or does not
    /// satisfy the contract.
    pub fn latest(&self) -> Result<Option<LatestGraph>, StoreError> {
        let Some(pointer) = self.pointer()? else {
            return Ok(None);
        };
        let path = self.graph_file(&pointer.latest);
        let bytes = std::fs::read(&path).map_err(|source| StoreError::Read { path, source })?;
        let graph = Graph::from_json(&bytes).map_err(|source| StoreError::Contract {
            sha: pointer.latest.clone(),
            source,
        })?;
        Ok(Some(LatestGraph {
            sha: pointer.latest,
            graph,
        }))
    }

    /// Whether a complete graph of `sha` is stored.
    pub fn has_graph(&self, sha: &CommitSha) -> bool {
        self.graph_file(sha).is_file()
    }

    /// Makes `graph` the complete graph of `sha` and the latest graph, in
    /// one step, then removes every graph but the latest and the one before
    /// it. A graph of `sha` already stored is kept as it is. Serialized with
    /// other publications of the same project by the store's lock.
    pub fn publish(&self, sha: &CommitSha, graph: &Graph) -> Result<(), StoreError> {
        let _lock = LockGuard::acquire(&self.dir.join(LOCK_FILE))?;
        if !self.has_graph(sha) {
            self.write_complete(sha, graph)?;
        }
        let previous = match self.pointer() {
            Ok(Some(old)) if old.latest == *sha => old.previous,
            Ok(Some(old)) => Some(old.latest),
            Ok(None) => None,
            Err(err) => {
                // The store is a cache: an unreadable pointer is replaced.
                tracing::warn!("replacing an unreadable blueprint pointer: {err}");
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
        self.prune(&pointer)
    }

    fn write_complete(&self, sha: &CommitSha, graph: &Graph) -> Result<(), StoreError> {
        let bytes = graph.to_json().map_err(|source| StoreError::Contract {
            sha: sha.clone(),
            source,
        })?;
        let staging = self.dir.join(format!("{STAGING_PREFIX}{sha}"));
        let target = self.dir.join(sha.as_str());
        if staging.exists() {
            // Left behind by an interrupted publication; we hold the lock.
            std::fs::remove_dir_all(&staging).map_err(write(&staging))?;
        }
        std::fs::create_dir_all(&staging).map_err(write(&staging))?;
        let file_path = staging.join(GRAPH_FILE);
        let mut file = std::fs::File::create(&file_path).map_err(write(&file_path))?;
        file.write_all(&bytes).map_err(write(&file_path))?;
        file.sync_all().map_err(write(&file_path))?;
        drop(file);
        if target.exists() {
            // A directory without a complete graph is not a graph.
            std::fs::remove_dir_all(&target).map_err(write(&target))?;
        }
        std::fs::rename(&staging, &target).map_err(write(&target))?;
        Ok(())
    }

    fn prune(&self, keep: &Pointer) -> Result<(), StoreError> {
        let entries = std::fs::read_dir(&self.dir).map_err(|source| StoreError::Read {
            path: self.dir.clone(),
            source,
        })?;
        for entry in entries {
            let entry = entry.map_err(|source| StoreError::Read {
                path: self.dir.clone(),
                source,
            })?;
            let name = entry.file_name();
            let Some(sha) = name.to_str().and_then(|n| CommitSha::parse(n).ok()) else {
                continue;
            };
            if sha == keep.latest || Some(&sha) == keep.previous.as_ref() {
                continue;
            }
            let path = entry.path();
            std::fs::remove_dir_all(&path).map_err(|source| StoreError::Write { path, source })?;
        }
        Ok(())
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

fn write(path: &Path) -> impl Fn(io::Error) -> StoreError + '_ {
    move |source| StoreError::Write {
        path: path.to_path_buf(),
        source,
    }
}

#[cfg(test)]
mod tests {
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
        store.publish(&sha('a'), &graph("first")).unwrap();
        let latest = store.latest().unwrap().unwrap();
        assert_eq!(latest.sha, sha('a'));
        assert_eq!(latest.graph.nodes()[0].id, "first");
        assert!(store.has_graph(&sha('a')));
        assert!(!store.has_graph(&sha('b')));
    }

    #[test]
    fn only_the_latest_and_the_one_before_are_kept() {
        let (_dir, store) = store();
        store.publish(&sha('a'), &graph("a")).unwrap();
        store.publish(&sha('b'), &graph("b")).unwrap();
        store.publish(&sha('c'), &graph("c")).unwrap();
        assert_eq!(
            shas_on_disk(&store),
            [sha('b').to_string(), sha('c').to_string()]
        );
        // Republishing the latest keeps the one before.
        store.publish(&sha('c'), &graph("ignored")).unwrap();
        assert_eq!(
            shas_on_disk(&store),
            [sha('b').to_string(), sha('c').to_string()]
        );
        assert_eq!(store.latest().unwrap().unwrap().graph.nodes()[0].id, "c");
    }

    #[test]
    fn an_interrupted_publication_leaves_the_latest_graph_in_place() {
        let (_dir, store) = store();
        store.publish(&sha('a'), &graph("a")).unwrap();
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
        store.publish(&sha('b'), &graph("b")).unwrap();
        assert_eq!(store.latest().unwrap().unwrap().sha, sha('b'));
        assert!(!store.dir().join(format!(".staging-{}", sha('b'))).exists());
    }

    #[test]
    fn a_graph_breaking_the_contract_on_disk_is_unreadable_not_absent() {
        let (_dir, store) = store();
        store.publish(&sha('a'), &graph("a")).unwrap();
        std::fs::write(
            store.dir().join(sha('a').as_str()).join("graph.json"),
            b"[]",
        )
        .unwrap();
        assert!(matches!(store.latest(), Err(StoreError::Contract { .. })));
        std::fs::remove_dir_all(store.dir().join(sha('a').as_str())).unwrap();
        assert!(matches!(store.latest(), Err(StoreError::Read { .. })));
    }
}
