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

use crate::evidence::{
    Actor, ContentDigest, Grounding, StampFact, TodoEvidence, VerificationDisposition,
};

use super::_provider::{
    ChangeSinceReview, CodeChange, DRIFT_ARTIFACTS, GroundingChange, Observations,
    WorktreeObservation, current_build_verdict, dependency_change, handoff_disposition,
    is_independent_reviewer,
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
    let current_head = branch_head.clone();
    let reviewed_change = match (current_build_verdict(evidence), &current_head) {
        (Some((_, reviewed, _)), Some(head)) => {
            let dir = worktree_dir.as_deref().unwrap_or(project_root);
            Some(change_since(dir, project_root, reviewed, head))
        }
        _ => None,
    };
    let owned_root = owned_snippet_root(project_root, evidence)?;
    let (grounding, judged_grounding) =
        grounding(home, project_root, &owned_root, evidence, prerequisites)?;
    Ok(Observations {
        worktree_path: worktree_dir,
        worktree,
        branch_head,
        remote_head,
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
        match reproduce_merge(dir, commit, first, second)? {
            MergeReproduction::Reproduced => {}
            MergeReproduction::Differs | MergeReproduction::Conflicted => {
                return Ok(ChangeSinceReview::TodoChanged);
            }
            MergeReproduction::Undetermined => return Ok(ChangeSinceReview::Undetermined),
        }
    }
    Ok(ChangeSinceReview::CleanUpstreamMerges)
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

/// Invariant 2: readiness requires an independent reviewer's `APPROVE` stamp
/// bound to the exact candidate commit SHA and current `review-findings` digest.
pub(crate) fn readiness_approved(
    record: &crate::work::TodoRecord,
    _root: &Path,
    candidate: &CommitSha,
    caller: &instrukt_core::ids::SessionRef,
) -> bool {
    let evidence = crate::work::_ports::to_evidence(record.clone(), Vec::new());
    let Some((verdict, target, report)) = current_build_verdict(&evidence) else {
        return false;
    };
    if verdict != crate::evidence::BuildVerdict::Approve
        || target != candidate
        || record.current_digest(crate::evidence::artifact_names::REVIEW_FINDINGS) != Some(report)
    {
        return false;
    }
    record
        .stamps
        .iter()
        .rev()
        .find(|s| matches!(s.fact, StampFact::BuildVerdict { .. }))
        .is_some_and(|s| independent(record, &s.actor, Some(caller)))
}

/// The handoff half of invariant 2 over the work record: a current
/// verification disposition admitting handoff at `candidate`, as
/// [`handoff_disposition`] judges it (the reason when not).
pub(crate) fn handoff_admitted(
    record: &crate::work::TodoRecord,
    candidate: &CommitSha,
    caller: &instrukt_core::ids::SessionRef,
) -> Result<(), String> {
    let evidence = crate::work::_ports::to_evidence(record.clone(), Vec::new());
    handoff_disposition(&evidence, candidate, Some(caller))
}

/// Whether `actor` is independent of the todo's claim holder, driver and
/// writer roles (and of `caller`, when given), as
/// [`is_independent_reviewer`] judges it over the record's bindings.
fn independent(
    record: &crate::work::TodoRecord,
    actor: &Actor,
    caller: Option<&instrukt_core::ids::SessionRef>,
) -> bool {
    let bindings: Vec<(
        instrukt_core::ids::RoleName,
        instrukt_runner::interfaces::AssignmentKind,
        Option<&instrukt_core::ids::SessionRef>,
    )> = record
        .bindings
        .iter()
        .map(|b| (b.role.clone(), b.attempt.assignment, b.session.as_ref()))
        .collect();
    is_independent_reviewer(
        record.claim.as_ref().and_then(|c| c.session.as_ref()),
        &bindings,
        actor,
        caller,
    )
}

/// The verifier role, whose bound attempt records a live verification
/// disposition (`delivery-cycle`, "Live verification disposition").
pub(crate) const VERIFIER_ROLE: &str = "verifier";

/// Who may record which verification disposition for `commit`, and which
/// may follow another for it (`delivery-cycle`, "Live verification
/// disposition"): the single owner of the rule on both of work's paths.
/// `binding_role` is the role of the binding `caller` acts under: a bound
/// `verifier` attempt, or the caller's live driver binding.
///
/// - pass, fail, blocked: only an independent bound `verifier`;
/// - Tier 0 skip: that verifier, or the claim-holding orchestrator;
/// - hotfix deferral: the claim-holding orchestrator alone.
///
/// While the latest disposition a `verifier` recorded for `commit` is fail,
/// the orchestrator's skip and deferral are refused; while it is blocked,
/// its skip is. Every refusal names its door.
pub(crate) fn verification_admission(
    record: &crate::work::TodoRecord,
    caller: &instrukt_core::ids::SessionRef,
    binding_role: &instrukt_core::ids::RoleName,
    disposition: VerificationDisposition,
    commit: &CommitSha,
) -> Result<(), String> {
    const NEW_VERIFIER: &str = "a pass, fail or blocked comes from a new independent `verifier` attempt, and a Tier 0 skip or hotfix deferral from the claim-holding orchestrator";
    if binding_role.as_str() == VERIFIER_ROLE {
        if !independent(record, &Actor::Session(caller.clone()), None) {
            return Err(format!(
                "this verifier is not independent: its session holds the claim, drives the todo or wrote this change; {NEW_VERIFIER}"
            ));
        }
        if disposition == VerificationDisposition::HotfixDeferred {
            return Err(
                "a hotfix deferral is the claim-holding orchestrator's alone; a verifier records pass, fail, blocked or tier_0_skip"
                    .to_owned(),
            );
        }
        return Ok(());
    }
    let holds_claim = record
        .claim
        .as_ref()
        .is_some_and(|claim| claim.session.as_ref() == Some(caller));
    if !holds_claim {
        return Err(format!(
            "a verification disposition is recorded by an independent bound verifier or the claim-holding orchestrator, and this session is neither; {NEW_VERIFIER}"
        ));
    }
    if !matches!(
        disposition,
        VerificationDisposition::Tier0Skip | VerificationDisposition::HotfixDeferred
    ) {
        return Err(format!(
            "the orchestrator records a Tier 0 skip or a hotfix deferral, never a {disposition}: {NEW_VERIFIER}"
        ));
    }
    let verifier_recorded = record
        .stamps
        .iter()
        .rev()
        .find_map(|stamp| match &stamp.fact {
            StampFact::VerificationDisposition {
                disposition,
                commit: recorded,
                ..
            } if recorded == commit
                && stamp.role.as_ref().map(|r| r.as_str()) == Some(VERIFIER_ROLE) =>
            {
                Some(*disposition)
            }
            _ => None,
        });
    match (verifier_recorded, disposition) {
        (Some(VerificationDisposition::Fail), _) => Err(format!(
            "a verifier recorded fail for commit {commit}, which a {disposition} does not wave through; what clears it is a new candidate commit, or a new verifier attempt on this commit"
        )),
        (Some(VerificationDisposition::Blocked), VerificationDisposition::Tier0Skip) => {
            Err(format!(
                "a verifier recorded blocked for commit {commit}, which a tier_0_skip does not wave through; what clears it is a new candidate commit, a new verifier attempt on this commit, or a hotfix deferral naming its tracked follow-up (`--verification hotfix_deferred --follow-up <slug>`)"
            ))
        }
        _ => Ok(()),
    }
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

    mod admission {
        use super::super::super::_testing::{session, sha};
        use super::*;
        use crate::evidence::{ModelProvenance, StampRecord};
        use crate::work::{Attempt, Binding, TodoRecord};
        use VerificationDisposition::{Blocked, Fail, HotfixDeferred, Pass, Tier0Skip};
        use instrukt_core::ids::RoleName;
        use instrukt_runner::interfaces::AssignmentKind;

        fn role(name: &str) -> RoleName {
            RoleName::parse(name).unwrap()
        }

        fn binding(name: &str, who: &str, assignment: AssignmentKind) -> Binding {
            Binding {
                role: role(name),
                session: Some(session(who)),
                worktree: None,
                attempt: Attempt {
                    assignment,
                    ..Attempt::default()
                },
            }
        }

        /// A todo claimed by `orchestrator`, driving it, which `builder`
        /// built and `verifier` (also its earlier build reviewer) verifies.
        fn record() -> TodoRecord {
            let mut record: TodoRecord = serde_json::from_value(serde_json::json!({
                "schema": 4, "id": "ab".repeat(16), "project": "demo", "slug": "ship-it",
                "kind": "change", "status": "active", "created": "2026-01-01T00:00:00Z",
                "created_by": {"person": {"name": "dev"}}, "revision": 1,
                "claim": {"machine": "cd".repeat(16), "session": "cli:orchestrator",
                          "at": "2026-01-02T00:00:00Z"},
            }))
            .unwrap();
            record.bindings = vec![
                binding("todo-lead", "orchestrator", AssignmentKind::Driver),
                binding("builder", "builder", AssignmentKind::Workspace),
                binding("build-reviewer", "verifier", AssignmentKind::Workspace),
                binding(VERIFIER_ROLE, "verifier", AssignmentKind::Workspace),
            ];
            record
        }

        /// `disposition` for `commit`, recorded by `who` as `recorded_as`.
        fn recorded(
            record: &mut TodoRecord,
            who: &str,
            recorded_as: &str,
            disposition: VerificationDisposition,
            commit: &CommitSha,
        ) {
            let sequence = record.next_sequence();
            record.stamps.push(StampRecord {
                source_comment: None,
                fact: StampFact::VerificationDisposition {
                    disposition,
                    commit: commit.clone(),
                    report: ContentDigest::of("findings"),
                    follow_up: None,
                },
                at: jiff::Timestamp::from_second(1_800_000_000).unwrap(),
                actor: Actor::Session(session(who)),
                model: None,
                eval_fingerprint: None,
                role: Some(role(recorded_as)),
                human_involved: None,
                sequence,
                model_provenance: ModelProvenance::Unknown,
            });
        }

        fn admit(
            record: &TodoRecord,
            who: &str,
            acting_as: &str,
            disposition: VerificationDisposition,
            commit: &CommitSha,
        ) -> Result<(), String> {
            verification_admission(record, &session(who), &role(acting_as), disposition, commit)
        }

        #[test]
        fn an_independent_verifier_records_all_but_a_deferral() {
            let record = record();
            let head = sha('a');
            for disposition in [Pass, Fail, Blocked, Tier0Skip] {
                assert_eq!(
                    admit(&record, "verifier", VERIFIER_ROLE, disposition, &head),
                    Ok(())
                );
            }
            assert!(admit(&record, "verifier", VERIFIER_ROLE, HotfixDeferred, &head).is_err());
        }

        #[test]
        fn a_verifier_that_wrote_or_drives_the_change_records_nothing() {
            let mut record = record();
            record
                .bindings
                .push(binding(VERIFIER_ROLE, "builder", AssignmentKind::Workspace));
            record.bindings.push(binding(
                VERIFIER_ROLE,
                "orchestrator",
                AssignmentKind::Workspace,
            ));
            let head = sha('a');
            for who in ["builder", "orchestrator"] {
                for disposition in [Pass, Tier0Skip] {
                    let admitted = admit(&record, who, VERIFIER_ROLE, disposition, &head);
                    assert!(admitted.is_err(), "{who} {disposition}");
                }
            }
        }

        #[test]
        fn the_claim_holder_records_only_a_skip_or_a_deferral() {
            let record = record();
            let head = sha('a');
            for disposition in [Tier0Skip, HotfixDeferred] {
                assert_eq!(
                    admit(&record, "orchestrator", "todo-lead", disposition, &head),
                    Ok(())
                );
            }
            for disposition in [Pass, Fail, Blocked] {
                let admitted = admit(&record, "orchestrator", "todo-lead", disposition, &head);
                assert!(admitted.is_err(), "{disposition}");
            }
            assert!(admit(&record, "builder", "todo-lead", Tier0Skip, &head).is_err());
        }

        #[test]
        fn a_verifiers_fail_or_blocked_limits_the_orchestrator_on_that_commit_only() {
            let head = sha('a');
            let mut failed = record();
            // The verifier first served as build reviewer: its disposition's
            // recorded role, not its first binding, keys the limit.
            recorded(&mut failed, "verifier", VERIFIER_ROLE, Fail, &head);
            for disposition in [Tier0Skip, HotfixDeferred] {
                let admitted = admit(&failed, "orchestrator", "todo-lead", disposition, &head);
                assert!(admitted.is_err(), "{disposition}");
            }
            assert_eq!(
                admit(&failed, "orchestrator", "todo-lead", Tier0Skip, &sha('b')),
                Ok(())
            );

            let mut blocked = record();
            recorded(&mut blocked, "verifier", VERIFIER_ROLE, Blocked, &head);
            assert!(admit(&blocked, "orchestrator", "todo-lead", Tier0Skip, &head).is_err());
            assert_eq!(
                admit(&blocked, "orchestrator", "todo-lead", HotfixDeferred, &head),
                Ok(())
            );

            // Only the latest disposition a verifier recorded limits.
            recorded(&mut failed, "verifier", VERIFIER_ROLE, Pass, &head);
            assert_eq!(
                admit(&failed, "orchestrator", "todo-lead", Tier0Skip, &head),
                Ok(())
            );
            // An orchestrator's own disposition is no verifier's.
            let mut skipped = record();
            recorded(&mut skipped, "orchestrator", "todo-lead", Tier0Skip, &head);
            assert_eq!(
                admit(&skipped, "orchestrator", "todo-lead", HotfixDeferred, &head),
                Ok(())
            );
        }
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
