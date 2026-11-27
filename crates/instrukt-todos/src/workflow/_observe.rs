//! Read repository/worktree evidence for pure workflow derivation. Git
//! observations establish current heads, cleanliness and artifact grounding;
//! review outcomes come from the work record, without fetching git notes.

use std::path::{Path, PathBuf};

use std::collections::{BTreeMap, BTreeSet};

use instrukt_core::docs::{Corpus, DocsError};
use instrukt_core::git::{self, GitError};
use instrukt_core::home::Home;
use instrukt_core::ids::{CommitSha, SnippetId, TodoSlug};
use instrukt_core::repository::RepositoryError;

use crate::evidence::{Actor, BuildVerdict, ContentDigest, Grounding, StampFact, TodoEvidence};

use super::_provider::{
    CodeChange, DRIFT_ARTIFACTS, GroundingChange, Observations, WorktreeObservation,
    current_build_verdict, dependency_change, handoff_disposition, is_independent_reviewer,
};

/// The remote todo branches are published to (the sdlc flavor's
/// `branch.publish` effect pushes `HEAD` to `origin`).
const REMOTE: &str = "origin";

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
    #[error(transparent)]
    Review(#[from] crate::work::review::ReviewError),
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
    let branch_name = evidence
        .branch
        .as_deref()
        .or_else(|| {
            super::_provider::selected_workspace(evidence).and_then(|b| b.attempt.branch.as_deref())
        })
        .unwrap_or(slug.as_str());
    let branch_head = resolve(project_root, &format!("refs/heads/{branch_name}"))?;
    let remote_head = resolve(
        project_root,
        &format!("refs/remotes/{REMOTE}/{branch_name}"),
    )?;
    let candidate = worktree
        .as_ref()
        .and_then(|tree| tree.head.as_ref())
        .or(branch_head.as_ref());
    let commit_writers = match candidate {
        Some(head)
            if evidence.stamps.iter().any(|stamp| {
                matches!(
                    stamp.fact,
                    StampFact::BuildVerdict { .. } | StampFact::VerificationDisposition { .. }
                )
            }) =>
        {
            commit_writers(project_root, evidence, head)?
        }
        _ => BTreeSet::new(),
    };
    let owned_root = owned_snippet_root(project_root, evidence)?;
    let (grounding, judged_grounding) =
        grounding(home, project_root, &owned_root, evidence, prerequisites)?;
    Ok(Observations {
        worktree_path: worktree_dir,
        worktree,
        branch_head,
        remote_head,
        commit_writers,
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
    let path = if evidence.status == crate::evidence::TodoStatus::Delivered {
        let listing = git::run(project_root, ["worktree", "list", "--porcelain"])?;
        evidence
            .bindings
            .iter()
            .filter(|b| b.attempt.lane == Some(instrukt_runner::interfaces::Lane::Work))
            .filter_map(|b| b.worktree())
            .find(|p| {
                listing
                    .lines()
                    .any(|line| line == format!("worktree {}", p.display()))
            })
            .cloned()
    } else {
        super::_provider::selected_workspace(evidence)
            .and_then(|b| b.worktree())
            .cloned()
    };
    let Some(path) = path else {
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

/// What git's own merge of a merge commit's two parents says of the commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MergeReproduction {
    /// The commit's tree is exactly the merge of its parents.
    Reproduced,
    /// The parents merge cleanly to another tree.
    Differs,
    /// The parents conflict: the commit's resolution is its own change.
    Conflicted,
    /// Git could not tell.
    Undetermined,
}

/// Reproduces merge `commit` from its parents `first` and `second` in `dir`.
pub(crate) fn reproduce_merge(
    dir: &Path,
    commit: &str,
    first: &str,
    second: &str,
) -> Result<MergeReproduction, ObservationError> {
    let merged = git::run_output(dir, ["merge-tree", "--write-tree", first, second])?;
    if !merged.status.success() {
        // Exit 1 is a conflicted merge. Any other status is git failing to tell.
        return Ok(if merged.status.code() == Some(1) {
            MergeReproduction::Conflicted
        } else {
            MergeReproduction::Undetermined
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
        return Ok(MergeReproduction::Undetermined);
    }
    Ok(if tree.trim() == merged_tree {
        MergeReproduction::Reproduced
    } else {
        MergeReproduction::Differs
    })
}

impl From<crate::work::_grounding::GroundingIoError> for ObservationError {
    fn from(err: crate::work::_grounding::GroundingIoError) -> Self {
        match err {
            crate::work::_grounding::GroundingIoError::Git(err) => ObservationError::Git(err),
            crate::work::_grounding::GroundingIoError::Io { path, source } => {
                ObservationError::Io { path, source }
            }
        }
    }
}

/// The live checkout holding this todo's own pending snippets: the active
/// work-lane or preparation-lane worktree when it exists, else `project_root`.
fn owned_snippet_root(
    project_root: &Path,
    evidence: &TodoEvidence,
) -> Result<PathBuf, ObservationError> {
    let candidates = crate::work::_grounding::candidate_worktrees(
        evidence
            .bindings
            .iter()
            .map(|b| (b.attempt.lane, b.worktree().map(PathBuf::as_path))),
    );
    Ok(crate::work::_grounding::owned_snippet_root(
        project_root,
        &candidates,
    )?)
}

/// What each tracked artifact's current version's grounding shows now.
fn grounding(
    home: &Home,
    project_root: &Path,
    owned_root: &Path,
    evidence: &TodoEvidence,
    prerequisites: &[TodoEvidence],
) -> Result<(Changes, Changes), ObservationError> {
    let main = git::default_branch(project_root, REMOTE)?;
    let canonical = resolve(project_root, &format!("refs/remotes/{REMOTE}/{main}"))?;
    let mut reader = Current {
        home,
        project_root,
        owned_root,
        todo: &evidence.slug,
        canonical,
        corpus: None,
        owned_corpus: None,
        prerequisites,
    };
    let mut versions = BTreeMap::new();
    let mut judged = BTreeMap::new();
    for name in DRIFT_ARTIFACTS {
        let Some(version) = evidence
            .artifacts
            .iter()
            .find(|artifact| artifact.name.as_str() == name)
            .and_then(|artifact| artifact.versions.last())
        else {
            continue;
        };
        versions.insert(name.to_owned(), reader.change(&version.grounding)?);
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
            judged.insert(name.to_owned(), reader.change(assessed)?);
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
    owned_root: &'a Path,
    todo: &'a TodoSlug,
    canonical: Option<CommitSha>,
    corpus: Option<Corpus>,
    owned_corpus: Option<Corpus>,
    prerequisites: &'a [TodoEvidence],
}

impl Current<'_> {
    /// What `recorded` shows against current evidence.
    fn change(&mut self, recorded: &Grounding) -> Result<GroundingChange, ObservationError> {
        let code = code_change(self.project_root, recorded, self.canonical.as_ref());
        let mut unresolved = self.missing_path(recorded)?;
        let mut snippets = false;
        let owned_ids: BTreeSet<&SnippetId> =
            recorded.owned_snippets.iter().map(|s| &s.id).collect();
        for snippet in &recorded.referenced_snippets {
            if owned_ids.contains(&snippet.id) {
                continue;
            }
            if self.corpus.is_none() {
                self.corpus = Some(Corpus::load(self.project_root, self.home)?);
            }
            let current = match &self.corpus {
                Some(corpus) => snippet_digest(self.home, self.project_root, corpus, &snippet.id)?,
                None => None,
            };
            unresolved |= current.is_none();
            snippets |= current != snippet.digest;
        }
        for snippet in &recorded.owned_snippets {
            let current = if self.owned_root == self.project_root {
                if self.corpus.is_none() {
                    self.corpus = Some(Corpus::load(self.project_root, self.home)?);
                }
                match &self.corpus {
                    Some(corpus) => crate::work::_grounding::owned_snippet_digest(
                        self.project_root,
                        corpus,
                        self.todo,
                        &snippet.id,
                    )?,
                    None => None,
                }
            } else {
                if self.owned_corpus.is_none() {
                    self.owned_corpus = Some(Corpus::load(self.owned_root, self.home)?);
                }
                match &self.owned_corpus {
                    Some(corpus) => crate::work::_grounding::owned_snippet_digest(
                        self.owned_root,
                        corpus,
                        self.todo,
                        &snippet.id,
                    )?,
                    None => None,
                }
            };
            snippets |= current != snippet.digest;
        }
        let (dependencies, dependency_unresolved) =
            dependency_change(&recorded.dependencies, self.prerequisites);
        unresolved |= dependency_unresolved;
        Ok(GroundingChange {
            code,
            snippets,
            dependencies,
            unresolved,
        })
    }

    /// A missing path is unresolved unless its absence at the recorded head
    /// establishes it as a planned creation. Code comparison handles a missing
    /// canonical head and intervening commits that touched a planned path.
    fn missing_path(&self, recorded: &Grounding) -> Result<bool, ObservationError> {
        let Some(now) = &self.canonical else {
            return Ok(false);
        };
        for path in &recorded.referenced_paths {
            let path = path.trim_end_matches('/');
            match tree_contains_path(self.project_root, now, path)? {
                Some(true) => continue,
                None => return Ok(true),
                Some(false) => {}
            }
            let Some(then) = &recorded.head else {
                return Ok(true);
            };
            if then != now && tree_contains_path(self.project_root, then, path)? != Some(false) {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

/// A successful empty listing proves absence. An unreadable commit or tree
/// cannot establish whether the path existed.
fn tree_contains_path(dir: &Path, head: &CommitSha, path: &str) -> Result<Option<bool>, GitError> {
    let commit = format!("{head}^{{commit}}");
    let output = git::run_output(
        dir,
        [
            "--literal-pathspecs",
            "ls-tree",
            "--full-tree",
            &commit,
            "--",
            path,
        ],
    )?;
    Ok(output.status.success().then_some(!output.stdout.is_empty()))
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

/// The commit writers identified by Git from the recorded build or workspace base.
fn commit_writers(
    root: &Path,
    evidence: &TodoEvidence,
    candidate: &CommitSha,
) -> Result<BTreeSet<String>, ObservationError> {
    let recorded_base = evidence
        .stamps
        .iter()
        .find_map(|stamp| match &stamp.fact {
            StampFact::BuildStarted { base, .. } => Some(base),
            _ => None,
        })
        .or_else(|| {
            evidence
                .bindings
                .iter()
                .find_map(|binding| binding.attempt.base_head.as_ref())
        });
    let base = match recorded_base {
        Some(base) => Some(base.clone()),
        None => {
            let branch = git::default_branch(root, REMOTE)?;
            match resolve(root, &format!("refs/remotes/{REMOTE}/{branch}"))? {
                Some(base) => Some(base),
                None => resolve(root, &format!("refs/heads/{branch}"))?,
            }
        }
    };
    let range = match base {
        Some(base) if &base != candidate => format!("{base}..{candidate}"),
        Some(_) => format!("{candidate}^!"),
        None => candidate.to_string(),
    };
    Ok(crate::work::review::commit_sessions(root, &[&range])?
        .into_iter()
        .flat_map(|(_, sessions)| sessions)
        .collect())
}

pub(crate) fn require_independent(
    record: &crate::work::TodoRecord,
    root: &Path,
    candidate: &CommitSha,
    actor: &Actor,
) -> Result<(), crate::work::WorkError> {
    let evidence = crate::work::_ports::to_evidence(record.clone(), Vec::new());
    let writers = commit_writers(root, &evidence, candidate)
        .map_err(|err| crate::work::WorkError::refused(err.to_string()))?;
    if is_independent_reviewer(&evidence, actor, &writers) {
        Ok(())
    } else {
        Err(crate::work::WorkError::refused(
            "approval and verification pass require an independent session distinct from the claimant, drivers, and writers",
        ))
    }
}

/// Check both exact-commit stamps against the current findings and writer evidence.
pub(crate) fn check_handoff_evidence(
    record: &crate::work::TodoRecord,
    root: &Path,
    candidate: &CommitSha,
) -> Result<(), crate::work::WorkError> {
    use crate::work::WorkError;
    let evidence = crate::work::_ports::to_evidence(record.clone(), Vec::new());
    let writers = commit_writers(root, &evidence, candidate)
        .map_err(|err| WorkError::refused(err.to_string()))?;
    let approved = current_build_verdict(&evidence).is_some_and(|(verdict, target, report)| {
        verdict == BuildVerdict::Approve
            && target == candidate
            && record.current_digest(crate::evidence::artifact_names::REVIEW_FINDINGS)
                == Some(report)
            && record
                .stamps
                .iter()
                .rev()
                .find(|stamp| matches!(stamp.fact, StampFact::BuildVerdict { .. }))
                .is_some_and(|stamp| is_independent_reviewer(&evidence, &stamp.actor, &writers))
    });
    if !approved {
        return Err(WorkError::refused(
            "readiness requires an independent reviewer's APPROVE stamp bound to this exact commit SHA and current review-findings",
        ));
    }
    handoff_disposition(&evidence, candidate, &writers).map_err(|why| {
        WorkError::refused(format!(
            "readiness requires a current verification disposition: {why}"
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    struct History {
        repo: TempDir,
        then: CommitSha,
        now: CommitSha,
    }

    impl History {
        fn new() -> Self {
            let repo = tempfile::tempdir().unwrap();
            let dir = repo.path();
            git::run(dir, ["init", "-q", "-b", "main"]).unwrap();
            git::run(dir, ["config", "user.name", "Test"]).unwrap();
            git::run(dir, ["config", "user.email", "test@example.com"]).unwrap();
            git::run(dir, ["config", "commit.gpgsign", "false"]).unwrap();
            std::fs::create_dir(dir.join("old")).unwrap();
            std::fs::write(dir.join("old/file"), "tracked").unwrap();
            std::fs::write(dir.join("stable.txt"), "tracked").unwrap();
            git::run(dir, ["add", "old/file", "stable.txt"]).unwrap();
            git::run(dir, ["commit", "-qm", "record original files"]).unwrap();
            let then = resolve(dir, "HEAD").unwrap().unwrap();
            git::run(dir, ["mv", "old", "new"]).unwrap();
            git::run(dir, ["commit", "-qm", "move original file"]).unwrap();
            let now = resolve(dir, "HEAD").unwrap().unwrap();
            Self { repo, then, now }
        }

        fn change(&self, head: Option<&CommitSha>, paths: &[&str]) -> GroundingChange {
            let root = self.repo.path();
            let mut current = Current {
                home: &Home::at_user_home(root.to_owned()),
                project_root: root,
                owned_root: root,
                todo: &TodoSlug::parse("planned-file").unwrap(),
                canonical: Some(self.now.clone()),
                corpus: None,
                owned_corpus: None,
                prerequisites: &[],
            };
            current
                .change(&Grounding {
                    head: head.cloned(),
                    referenced_paths: paths.iter().map(|path| (*path).to_owned()).collect(),
                    ..Grounding::default()
                })
                .unwrap()
        }
    }

    #[test]
    fn an_unrelated_commit_leaves_planned_paths_fresh() {
        let history = History::new();
        assert_eq!(
            history.change(Some(&history.then), &["future.snap", "planned/"]),
            GroundingChange::default()
        );
    }

    #[test]
    fn an_unrelated_commit_leaves_an_existing_file_fresh() {
        let history = History::new();
        let change = history.change(Some(&history.then), &["stable.txt"]);
        assert!(!change.unresolved);
        assert_eq!(change.code, CodeChange::Unchanged);
    }

    #[test]
    fn creating_a_planned_file_is_code_drift() {
        let history = History::new();
        assert_eq!(
            history.change(Some(&history.then), &["new/file"]),
            GroundingChange {
                code: CodeChange::Changed,
                ..GroundingChange::default()
            }
        );
    }

    #[test]
    fn deleting_a_grounded_file_is_unresolved_code_drift() {
        let history = History::new();
        assert_eq!(
            history.change(Some(&history.then), &["future.snap", "old/file"]),
            GroundingChange {
                code: CodeChange::Changed,
                unresolved: true,
                ..GroundingChange::default()
            }
        );
    }

    #[test]
    fn deleting_a_grounded_directory_prefix_is_unresolved_code_drift() {
        let history = History::new();
        assert_eq!(
            history.change(Some(&history.then), &["old/"]),
            GroundingChange {
                code: CodeChange::Changed,
                unresolved: true,
                ..GroundingChange::default()
            }
        );
    }

    #[test]
    fn a_missing_path_without_a_recorded_head_remains_unresolved() {
        let history = History::new();
        assert_eq!(
            history.change(None, &["future.snap"]),
            GroundingChange {
                code: CodeChange::Uncomparable,
                unresolved: true,
                ..GroundingChange::default()
            }
        );
    }

    #[test]
    fn a_missing_path_with_an_unknown_recorded_head_remains_unresolved() {
        let history = History::new();
        let unknown = super::super::_testing::sha('a');
        assert_eq!(
            history.change(Some(&unknown), &["future.snap"]),
            GroundingChange {
                code: CodeChange::Uncomparable,
                unresolved: true,
                ..GroundingChange::default()
            }
        );
    }
}
