//! Repository observations the fact provider needs and the record cannot
//! supply: the assignment worktree, the todo branch here and on the
//! remote, the commit-review settlement of the todo's canonical-branch
//! commits, whether the todo's work changed since its reviewed commit, and
//! what each tracked artifact's recorded grounding shows against current
//! code, snippets and prerequisite artifacts.
//!
//! Every observation reads git; none writes, except that reading
//! review notes first fetches the remote's notes, as the integrator's
//! reading does (`instrukt_core::repository::read_review_notes`).

use std::path::{Path, PathBuf};

use std::collections::BTreeMap;

use instrukt_core::docs::{Corpus, DocsError};
use instrukt_core::git::{self, GitError};
use instrukt_core::home::Home;
use instrukt_core::ids::{CommitSha, SnippetId, TodoSlug};
use instrukt_core::repository::{self, RepositoryError};

use crate::evidence::{ContentDigest, Grounding, StampFact, TodoEvidence};

use super::_provider::{
    ChangeSinceReview, CodeChange, DRIFT_FACTS, GroundingChange, Observations, WorktreeObservation,
    current_build_verdict,
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
    #[error(transparent)]
    Docs(#[from] DocsError),
    #[error("git printed `{value}` where a commit id belongs ({context})")]
    NotACommit { value: String, context: String },
}

/// Observes the repository at `project_root` for `evidence`'s todo.
/// `prerequisites` are the prerequisite todos' evidence the record has.
pub(crate) fn observe(
    home: &Home,
    project_root: &Path,
    evidence: &TodoEvidence,
    prerequisites: &[TodoEvidence],
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
    let (grounding, judged_grounding) = grounding(home, project_root, evidence, prerequisites)?;
    Ok(Observations {
        worktree,
        branch_head,
        remote_head,
        unreviewed_claim_commits,
        reviewed_change,
        grounding,
        judged_grounding,
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
    // Upstream is the published canonical branch. Local main is not
    // upstream evidence: an unpublished merge of it is not a clean upstream
    // merge, so without the remote-tracking branch nothing is determined.
    let Some(canonical) = resolve(dir, &format!("refs/remotes/{REMOTE}/{main}"))? else {
        return Ok(ChangeSinceReview::Undetermined);
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

/// What each tracked artifact's current version's grounding shows now.
fn grounding(
    home: &Home,
    project_root: &Path,
    evidence: &TodoEvidence,
    prerequisites: &[TodoEvidence],
) -> Result<(Changes, Changes), ObservationError> {
    let main = git::default_branch(project_root, REMOTE)?;
    let canonical = resolve(project_root, &format!("refs/remotes/{REMOTE}/{main}"))?;
    let mut reader = Current {
        home,
        project_root,
        canonical,
        corpus: None,
        prerequisites,
    };
    let mut versions = BTreeMap::new();
    let mut judged = BTreeMap::new();
    for (name, _) in DRIFT_FACTS {
        let Some(version) = evidence
            .artifacts
            .iter()
            .find(|artifact| artifact.name.as_str() == name)
            .and_then(|artifact| artifact.versions.last())
        else {
            continue;
        };
        versions.insert((*name).to_owned(), reader.change(&version.grounding)?);
        // The evidence the latest drift judgment of the artifact assessed,
        // when it recorded any.
        let assessed = evidence
            .stamps
            .iter()
            .rev()
            .find_map(|stamp| match &stamp.fact {
                StampFact::DriftReviewed {
                    artifact,
                    grounding,
                    ..
                } if artifact.as_str() == name => Some(grounding),
                _ => None,
            });
        if let Some(assessed) = assessed.filter(|g| **g != Grounding::default()) {
            judged.insert((*name).to_owned(), reader.change(assessed)?);
        }
    }
    Ok((versions, judged))
}

/// What recorded grounding shows now, by tracked artifact name.
type Changes = BTreeMap<String, GroundingChange>;

/// Current evidence, read once per call, to compare recorded grounding
/// with.
struct Current<'a> {
    home: &'a Home,
    project_root: &'a Path,
    canonical: Option<CommitSha>,
    corpus: Option<Corpus>,
    prerequisites: &'a [TodoEvidence],
}

impl Current<'_> {
    /// What `recorded` shows against current evidence.
    fn change(&mut self, recorded: &Grounding) -> Result<GroundingChange, ObservationError> {
        let code = code_change(self.project_root, recorded, self.canonical.as_ref());
        let mut snippets = false;
        for snippet in recorded
            .referenced_snippets
            .iter()
            .chain(&recorded.owned_snippets)
        {
            if self.corpus.is_none() {
                self.corpus = Some(Corpus::load(self.project_root, self.home)?);
            }
            let current = match &self.corpus {
                Some(corpus) => snippet_digest(self.home, self.project_root, corpus, &snippet.id)?,
                None => None,
            };
            snippets |= current != snippet.digest;
        }
        let dependencies = recorded.dependencies.iter().any(|dependency| {
            let current = self
                .prerequisites
                .iter()
                .find(|todo| todo.slug == dependency.todo)
                .and_then(|todo| {
                    todo.artifacts
                        .iter()
                        .find(|artifact| artifact.name == dependency.artifact)
                })
                .and_then(|artifact| artifact.versions.last())
                .map(|version| &version.digest);
            current != dependency.digest.as_ref()
        });
        Ok(GroundingChange {
            code,
            snippets,
            dependencies,
        })
    }
}

/// Whether a non-merge commit between the recorded canonical head and the
/// current one changes a referenced path (exactly, or under a directory
/// prefix). Any failure to compare is uncomparable, never unchanged.
fn code_change(
    project_root: &Path,
    recorded: &Grounding,
    canonical: Option<&CommitSha>,
) -> CodeChange {
    if recorded.referenced_paths.is_empty() {
        return CodeChange::Unchanged;
    }
    let (Some(then), Some(now)) = (&recorded.head, canonical) else {
        return CodeChange::Uncomparable;
    };
    if then == now {
        return CodeChange::Unchanged;
    }
    let range = format!("{then}..{now}");
    let mut args = vec![
        "log".to_owned(),
        "--no-merges".to_owned(),
        "--format=%H".to_owned(),
        range,
        "--".to_owned(),
    ];
    args.extend(recorded.referenced_paths.iter().cloned());
    match git::run_output(project_root, &args) {
        Ok(output) if output.status.success() => {
            if output.stdout.iter().all(u8::is_ascii_whitespace) {
                CodeChange::Unchanged
            } else {
                CodeChange::Changed
            }
        }
        Ok(output) => {
            tracing::warn!(
                event = "grounding_uncomparable",
                then = %then,
                now = %now,
                stderr = %String::from_utf8_lossy(&output.stderr).trim(),
            );
            CodeChange::Uncomparable
        }
        Err(err) => {
            tracing::warn!(event = "grounding_uncomparable", then = %then, now = %now, error = %err);
            CodeChange::Uncomparable
        }
    }
}

/// A snippet's current digest, read where work read it when recording:
/// the project's own file, else the published one; `None` when it does
/// not resolve.
fn snippet_digest(
    home: &Home,
    project_root: &Path,
    corpus: &Corpus,
    id: &SnippetId,
) -> Result<Option<ContentDigest>, ObservationError> {
    let path = match corpus.snippet(id) {
        Some(snippet) => project_root.join(snippet.path()),
        None => home.docs_dir().join(format!("{id}.md")),
    };
    match std::fs::read_to_string(&path) {
        Ok(text) => Ok(Some(ContentDigest::of(&text))),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(ObservationError::Io { path, source }),
    }
}
