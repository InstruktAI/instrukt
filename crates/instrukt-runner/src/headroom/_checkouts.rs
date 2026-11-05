//! The checkouts the guard may clean (`disk-headroom`, "Checkouts the
//! guard may clean"): linked checkouts, found through the projects in
//! publication state and under the configured roots, never reached
//! through a symbolic link.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use instrukt_core::home::Home;
use instrukt_core::publication::PublishedRecord;

/// What a directory's `.git` entry makes it.
enum GitEntry {
    /// No `.git`: an ordinary directory.
    None,
    /// `.git` is a directory: a primary clone.
    Primary,
    /// `.git` is a file: a linked checkout.
    Linked,
    /// `.git` is something else, or could not be read.
    Unknown,
}

fn git_entry(dir: &Path) -> GitEntry {
    match std::fs::symlink_metadata(dir.join(".git")) {
        Ok(meta) if meta.is_dir() => GitEntry::Primary,
        Ok(meta) if meta.is_file() => GitEntry::Linked,
        Ok(_) => GitEntry::Unknown,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => GitEntry::None,
        Err(_) => GitEntry::Unknown,
    }
}

/// Whether any component of `path` is a symbolic link.
fn reached_through_link(path: &Path) -> bool {
    path.ancestors().any(|ancestor| {
        std::fs::symlink_metadata(ancestor).map_or(true, |meta| meta.file_type().is_symlink())
    })
}

/// A linked checkout at `path` the guard may consider: its `.git` is a
/// file and no component of its path is a symbolic link.
fn eligible(path: &Path) -> bool {
    matches!(git_entry(path), GitEntry::Linked) && !reached_through_link(path)
}

/// The worktrees git lists for the repository `checkout` belongs to;
/// `None` when git cannot list them.
fn registered_worktrees(checkout: &Path) -> Option<Vec<PathBuf>> {
    match instrukt_core::git::run(checkout, ["worktree", "list", "--porcelain"]) {
        Ok(listing) => Some(
            listing
                .lines()
                .filter_map(|line| line.strip_prefix("worktree "))
                .map(PathBuf::from)
                .collect(),
        ),
        Err(err) => {
            tracing::warn!(event = "disk_headroom_worktrees_unlisted", checkout = %checkout.display(), error = %err);
            None
        }
    }
}

/// Every linked checkout the guard may clean, in path order.
pub fn discover(home: &Home, roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut found = BTreeSet::new();
    match PublishedRecord::load_all(home) {
        Ok(records) => {
            for record in records {
                if !record.checkout.is_dir() {
                    continue;
                }
                for worktree in registered_worktrees(&record.checkout).unwrap_or_default() {
                    if eligible(&worktree) {
                        found.insert(worktree);
                    }
                }
            }
        }
        Err(err) => {
            tracing::warn!(event = "disk_headroom_publication_unreadable", error = %err);
        }
    }
    for root in roots {
        match under_root(root) {
            Some(checkouts) => found.extend(checkouts),
            None => tracing::warn!(event = "disk_headroom_root_unreadable", root = %root.display()),
        }
    }
    found.into_iter().collect()
}

/// The linked checkouts under `root`: those registered by each primary
/// clone found there and lying inside it, and those found there directly.
/// The search descends into no symbolic link and stops at each checkout.
/// `None` when any part of the root, or a primary clone's worktree list,
/// could not be read: none of its checkouts are then candidates.
fn under_root(root: &Path) -> Option<Vec<PathBuf>> {
    if reached_through_link(root) {
        return None;
    }
    let mut checkouts = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        match git_entry(&dir) {
            GitEntry::Primary => {
                for worktree in registered_worktrees(&dir)? {
                    if worktree.starts_with(root) && eligible(&worktree) {
                        checkouts.push(worktree);
                    }
                }
                continue;
            }
            GitEntry::Linked => {
                if eligible(&dir) {
                    checkouts.push(dir);
                }
                continue;
            }
            GitEntry::Unknown => return None,
            GitEntry::None => {}
        }
        for entry in std::fs::read_dir(&dir).ok()? {
            let entry = entry.ok()?;
            if entry.file_type().ok()?.is_dir() {
                pending.push(entry.path());
            }
        }
    }
    Some(checkouts)
}
