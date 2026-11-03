//! Repository observations the fact provider needs and the record cannot
//! supply: the assignment worktree, the todo branch here and on the
//! remote, the commit-review settlement of the todo's canonical-branch
//! commits, and whether the todo's work changed since its reviewed commit.
//!
//! Every observation reads git; none writes, except that reading
//! review notes first fetches the remote's notes, as the integrator's
//! reading does (`instrukt_core::repository::read_review_notes`).

use std::path::{Path, PathBuf};

use instrukt_core::git::{self, GitError};
use instrukt_core::ids::{CommitSha, TodoSlug};
use instrukt_core::repository::{self, RepositoryError};

use crate::evidence::TodoEvidence;

use super::_provider::{
    ChangeSinceReview, Observations, WorktreeObservation, current_build_verdict,
};

/// The remote todo branches are published to (the sdlc flavor's
/// `branch.publish` effect pushes `HEAD:{slug}` to `origin`).
const REMOTE: &str = "origin";

/// The commit trailer naming the todo a commit belongs to
/// (`instrukt repo commit --todo`).
const TODO_TRAILER: &str = "Todo";

/// A repository observation failed.
#[derive(Debug, thiserror::Error)]
pub enum ObservationError {
    #[error(transparent)]
    Git(#[from] GitError),
    #[error(transparent)]
    Repository(#[from] RepositoryError),
    #[error("cannot read {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("git printed `{value}` where a commit id belongs ({context})")]
    NotACommit { value: String, context: String },
}

/// Observes the repository at `project_root` for `evidence`'s todo.
pub(crate) fn observe(
    project_root: &Path,
    evidence: &TodoEvidence,
) -> Result<Observations, ObservationError> {
    let slug = &evidence.slug;
    let worktree_dir = worktree_dir(project_root, evidence)?;
    let worktree = match &worktree_dir {
        Some(dir) => Some(WorktreeObservation {
            clean: git::run(dir, ["status", "--porcelain"])?.is_empty(),
            head: resolve(dir, "HEAD")?,
        }),
        None => None,
    };
    let branch_head = resolve(project_root, &format!("refs/heads/{slug}"))?;
    let remote_head = resolve(project_root, &format!("refs/remotes/{REMOTE}/{slug}"))?;
    let unreviewed_claim_commits = unreviewed_claim_commits(project_root, slug)?;
    let current_head = worktree
        .as_ref()
        .and_then(|worktree| worktree.head.clone())
        .or_else(|| branch_head.clone());
    let reviewed_change = match (current_build_verdict(evidence), &current_head) {
        (Some((_, reviewed, _)), Some(head)) => {
            let dir = worktree_dir.as_deref().unwrap_or(project_root);
            Some(change_since(dir, project_root, reviewed, head))
        }
        _ => None,
    };
    Ok(Observations {
        worktree,
        branch_head,
        remote_head,
        unreviewed_claim_commits,
        reviewed_change,
    })
}

/// The assignment worktree the bindings retain, while it exists as a
/// working tree.
fn worktree_dir(
    project_root: &Path,
    evidence: &TodoEvidence,
) -> Result<Option<PathBuf>, ObservationError> {
    let Some(path) = evidence.bindings.iter().rev().find_map(|b| b.worktree()) else {
        return Ok(None);
    };
    // A directory left where the worktree was is not the worktree: only a
    // working tree rooted exactly there counts.
    let path = project_root.join(path);
    let path = match path.canonicalize() {
        Ok(path) => path,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => return Err(ObservationError::Io { path, source }),
    };
    let Some(root) = git::worktree_root(&path)? else {
        return Ok(None);
    };
    let root = root.canonicalize().map_err(|source| ObservationError::Io {
        path: root.clone(),
        source,
    })?;
    Ok((root == path).then_some(path))
}

/// The commit `revision` names in `dir`; `None` when it names none.
fn resolve(dir: &Path, revision: &str) -> Result<Option<CommitSha>, ObservationError> {
    let spec = format!("{revision}^{{commit}}");
    let output = git::run_output(dir, ["rev-parse", "--verify", "--quiet", spec.as_str()])?;
    if !output.status.success() {
        return Ok(None);
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    CommitSha::parse(&text)
        .map(Some)
        .map_err(|_| ObservationError::NotACommit {
            value: text,
            context: format!("rev-parse {revision}"),
        })
}

/// The canonical-branch commits carrying `Todo: <slug>` whose
/// commit-review settlement does not release them, oldest first.
fn unreviewed_claim_commits(
    root: &Path,
    slug: &TodoSlug,
) -> Result<Vec<CommitSha>, ObservationError> {
    let main = git::default_branch(root, REMOTE)?;
    let main_ref = format!("refs/heads/{main}");
    if resolve(root, &main_ref)?.is_none() {
        return Ok(Vec::new());
    }
    let format = format!("--format=%H%x09%(trailers:key={TODO_TRAILER},valueonly,separator=%x2C)");
    let log = git::run(
        root,
        ["log", "--reverse", format.as_str(), main_ref.as_str()],
    )?;
    let mut commits = Vec::new();
    for line in log.lines() {
        let (sha, todos) = line.split_once('\t').unwrap_or((line, ""));
        if todos.split(',').any(|todo| todo.trim() == slug.as_str()) {
            commits.push(sha.to_owned());
        }
    }
    if commits.is_empty() {
        return Ok(Vec::new());
    }
    let notes = repository::read_review_notes(root, &commits)?;
    Ok(notes
        .commits
        .into_iter()
        .filter(|review| !review.settlement.releases())
        .map(|review| review.commit)
        .collect())
}

/// Whether the todo's work changed between `reviewed` and `head`, read in
/// `dir`: unchanged when they are equal; clean upstream merges when every
/// first-parent commit after `reviewed` is a merge of a commit already on
/// the canonical branch whose tree is exactly what git's own merge of its
/// parents produces; changed otherwise. A repository that cannot answer
/// leaves it undetermined.
fn change_since(
    dir: &Path,
    project_root: &Path,
    reviewed: &CommitSha,
    head: &CommitSha,
) -> ChangeSinceReview {
    if reviewed == head {
        return ChangeSinceReview::Unchanged;
    }
    match classify(dir, project_root, reviewed, head) {
        Ok(change) => change,
        Err(err) => {
            tracing::warn!(
                event = "review_change_undetermined",
                reviewed = %reviewed,
                head = %head,
                error = %err,
            );
            ChangeSinceReview::Undetermined
        }
    }
}

fn classify(
    dir: &Path,
    project_root: &Path,
    reviewed: &CommitSha,
    head: &CommitSha,
) -> Result<ChangeSinceReview, ObservationError> {
    if !git::succeeds(
        dir,
        [
            "merge-base",
            "--is-ancestor",
            reviewed.as_str(),
            head.as_str(),
        ],
    )? {
        return Ok(ChangeSinceReview::TodoChanged);
    }
    let main = git::default_branch(project_root, REMOTE)?;
    let canonical = match resolve(dir, &format!("refs/remotes/{REMOTE}/{main}"))? {
        Some(sha) => sha,
        None => match resolve(dir, &format!("refs/heads/{main}"))? {
            Some(sha) => sha,
            None => return Ok(ChangeSinceReview::Undetermined),
        },
    };
    let range = format!("{reviewed}..{head}");
    let listing = git::run(
        dir,
        ["rev-list", "--first-parent", "--parents", range.as_str()],
    )?;
    for line in listing.lines() {
        let ids: Vec<&str> = line.split_whitespace().collect();
        let [commit, first, second] = ids.as_slice() else {
            return Ok(ChangeSinceReview::TodoChanged);
        };
        if !git::succeeds(
            dir,
            ["merge-base", "--is-ancestor", second, canonical.as_str()],
        )? {
            return Ok(ChangeSinceReview::TodoChanged);
        }
        let merged = git::run_output(dir, ["merge-tree", "--write-tree", first, second])?;
        if !merged.status.success() {
            // Exit 1 is a conflicted merge: the commit's resolution is the
            // todo's own change. Any other status is git failing to tell.
            return Ok(if merged.status.code() == Some(1) {
                ChangeSinceReview::TodoChanged
            } else {
                ChangeSinceReview::Undetermined
            });
        }
        let merged_tree = String::from_utf8_lossy(&merged.stdout)
            .lines()
            .next()
            .unwrap_or_default()
            .trim()
            .to_owned();
        let tree = git::run(dir, ["rev-parse", &format!("{commit}^{{tree}}")])?;
        if merged_tree.is_empty() {
            return Ok(ChangeSinceReview::Undetermined);
        }
        if tree.trim() != merged_tree {
            return Ok(ChangeSinceReview::TodoChanged);
        }
    }
    Ok(ChangeSinceReview::CleanUpstreamMerges)
}
