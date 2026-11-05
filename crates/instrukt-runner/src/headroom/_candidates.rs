//! Regenerable output inside a checkout (`disk-headroom`, "Regenerable
//! output"): the outermost Rust build output, Node dependencies and Python
//! environments the checkout tracks nothing inside, each fully read.

use std::collections::HashSet;
use std::os::unix::fs::MetadataExt as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// What kind of regenerable output a candidate is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    RustBuild,
    NodeDependencies,
    PythonEnvironment,
}

/// One directory the guard may remove.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Candidate {
    pub path: PathBuf,
    pub checkout: PathBuf,
    pub kind: Kind,
    /// The space its files occupy, each hard-linked file counted once.
    pub size_bytes: u64,
}

fn exists(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok()
}

/// The kind of regenerable output `dir` (named `name`) is, if any.
fn kind_of(dir: &Path, name: &str) -> Option<Kind> {
    match name {
        "target"
            if ["CACHEDIR.TAG", ".rustc_info.json", "debug/.fingerprint"]
                .iter()
                .any(|marker| exists(&dir.join(marker))) =>
        {
            Some(Kind::RustBuild)
        }
        "node_modules" => Some(Kind::NodeDependencies),
        ".venv" if exists(&dir.join("pyvenv.cfg")) => Some(Kind::PythonEnvironment),
        _ => None,
    }
}

/// The candidates of `checkout`. None when any directory of it could not
/// be read or it contains a primary clone; a nested checkout or
/// repository is its own owner and is not searched.
pub fn in_checkout(checkout: &Path) -> Vec<Candidate> {
    let mut found = Vec::new();
    let mut pending = vec![checkout.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            tracing::warn!(event = "disk_headroom_checkout_unreadable", checkout = %checkout.display(), dir = %dir.display());
            return Vec::new();
        };
        for entry in entries {
            let Ok(entry) = entry else {
                tracing::warn!(event = "disk_headroom_checkout_unreadable", checkout = %checkout.display(), dir = %dir.display());
                return Vec::new();
            };
            let Ok(file_type) = entry.file_type() else {
                return Vec::new();
            };
            if !file_type.is_dir() {
                continue;
            }
            let path = entry.path();
            match std::fs::symlink_metadata(path.join(".git")) {
                Ok(meta) if meta.is_dir() => {
                    tracing::info!(event = "disk_headroom_checkout_holds_clone", checkout = %checkout.display(), clone = %path.display());
                    return Vec::new();
                }
                // A nested checkout or repository: its own owner.
                Ok(_) => continue,
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Vec::new(),
            }
            let name = entry.file_name();
            match name.to_str().and_then(|name| kind_of(&path, name)) {
                Some(kind) => {
                    if let Some(candidate) = confirm(checkout, path, kind) {
                        found.push(candidate);
                    }
                }
                None => pending.push(path),
            }
        }
    }
    found
}

/// `dir` as a candidate once git confirms the checkout tracks nothing
/// inside it and its whole tree was read and holds no checkout or
/// repository.
fn confirm(checkout: &Path, dir: PathBuf, kind: Kind) -> Option<Candidate> {
    let relative = dir.strip_prefix(checkout).ok()?.to_str()?;
    let pathspec = format!(":(literal){relative}");
    match instrukt_core::git::run(checkout, ["ls-files", "-z", "--", pathspec.as_str()]) {
        Ok(tracked) if tracked.is_empty() => {}
        Ok(_) => return None,
        Err(err) => {
            tracing::warn!(event = "disk_headroom_tracking_unconfirmed", dir = %dir.display(), error = %err);
            return None;
        }
    }
    let Some(size_bytes) = survey(&dir) else {
        tracing::warn!(event = "disk_headroom_candidate_unread", dir = %dir.display());
        return None;
    };
    Some(Candidate {
        path: dir,
        checkout: checkout.to_path_buf(),
        kind,
        size_bytes,
    })
}

/// The space the files under `dir` occupy, each hard-linked file counted
/// once; `None` when any of the tree could not be read or it holds a
/// checkout or repository.
fn survey(dir: &Path) -> Option<u64> {
    let mut seen = HashSet::new();
    let mut size = 0u64;
    let mut pending = vec![dir.to_path_buf()];
    while let Some(next) = pending.pop() {
        for entry in std::fs::read_dir(&next).ok()? {
            let entry = entry.ok()?;
            if entry.file_name() == ".git" {
                return None;
            }
            let meta = entry.metadata().ok()?;
            if meta.is_dir() {
                pending.push(entry.path());
            } else if meta.is_file() && (meta.nlink() <= 1 || seen.insert((meta.dev(), meta.ino())))
            {
                size = size.saturating_add(meta.blocks().saturating_mul(512));
            }
        }
    }
    Some(size)
}
