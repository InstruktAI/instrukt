//! Worktree standing (`project/instrukt/spec/domain/workflow`, "Worktree
//! standing"): whether each todo worktree git lists for a project on this
//! computer is delivered, idle or under way, derived from the todo's record
//! and the repository and stored nowhere. The runner reclaims by it
//! (`project/instrukt/spec/feature/runner/worktree-reclamation`).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use instrukt_core::git::{self, GitError};
use instrukt_core::ids::CommitSha;
use instrukt_runner::interfaces::{DeliveredRemains, Lane, TodoWorktree, WorktreeStanding};

use crate::evidence::{StampFact, TodoEvidence, TodoStatus, artifact_names};

use super::_provider::{Observations, finalize_handoff_pending};

/// The remote work branches are published to.
const REMOTE: &str = "origin";

/// Where a todo with a lifecycle stands, before git says what of a
/// delivered todo remains.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Standing {
    UnderWay,
    Idle,
    Delivered,
}

/// The standing of `evidence`'s todo, given the heads `observed` holds of
/// its work branch; `None` for a todo with no lifecycle: a structural
/// holder, or a withdrawn todo, whose withdrawal releases its worktree.
pub(crate) fn standing(evidence: &TodoEvidence, observed: &Observations) -> Option<Standing> {
    match evidence.status {
        TodoStatus::StructuralHolder | TodoStatus::Removed => None,
        TodoStatus::Delivered => Some(Standing::Delivered),
        TodoStatus::Iceboxed => Some(Standing::Idle),
        TodoStatus::Active => Some(
            if evidence.hold.is_some()
                || finalize_handoff_pending(evidence, observed)
                || back_to_preparation(evidence)
            {
                Standing::Idle
            } else {
                Standing::UnderWay
            },
        ),
    }
}

/// Whether the todo's work has gone back to preparation: it has build
/// evidence, and its latest preparation evidence is later in the record's
/// order than its latest build evidence. A kickback is neither, and its
/// reason is never read.
fn back_to_preparation(evidence: &TodoEvidence) -> bool {
    use artifact_names::{ARCHITECTURE, DIAGNOSIS, IMPLEMENTATION_PLAN, INPUT, PRD};
    let build = evidence
        .stamps
        .iter()
        .filter(|stamp| {
            matches!(
                stamp.fact,
                StampFact::BuildStarted { .. }
                    | StampFact::BuildComplete { .. }
                    | StampFact::BuildVerdict { .. }
                    | StampFact::DeferralsProcessed { .. }
                    | StampFact::FinalizeReady { .. }
            )
        })
        .map(|stamp| stamp.sequence)
        .max();
    let Some(build) = build else {
        return false;
    };
    let prepared_artifact = evidence
        .artifacts
        .iter()
        .filter(|artifact| {
            [INPUT, PRD, DIAGNOSIS, ARCHITECTURE, IMPLEMENTATION_PLAN]
                .contains(&artifact.name.as_str())
        })
        .flat_map(|artifact| artifact.versions.iter().map(|version| version.sequence));
    let prepared_verdict = evidence
        .stamps
        .iter()
        .filter(|stamp| {
            matches!(
                stamp.fact,
                StampFact::PreparationVerdict { .. } | StampFact::PreparationHeadReviewed { .. }
            )
        })
        .map(|stamp| stamp.sequence);
    prepared_artifact
        .chain(prepared_verdict)
        .max()
        .is_some_and(|prepared| prepared > build)
}

/// A worktree of the project as git lists it, other than its primary one.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ListedWorktree {
    path: PathBuf,
}

/// What git lists for a project on this computer: its worktrees other than
/// the primary one, its local branches and its branches on `origin`.
#[derive(Debug, Default)]
pub(crate) struct Listed {
    worktrees: Vec<ListedWorktree>,
    local: BTreeMap<String, CommitSha>,
    origin: BTreeMap<String, CommitSha>,
}

impl Listed {
    /// Reads it from the repository at `root`.
    pub(crate) fn read(root: &Path) -> Result<Self, GitError> {
        let mut listed = Self {
            worktrees: parse_worktrees(&git::run(root, ["worktree", "list", "--porcelain"])?),
            ..Self::default()
        };
        let origin = format!("refs/remotes/{REMOTE}/");
        for line in git::run(
            root,
            [
                "for-each-ref",
                "--format=%(objectname) %(refname)",
                "refs/heads",
                origin.as_str(),
            ],
        )?
        .lines()
        {
            let Some((sha, name)) = line.split_once(' ') else {
                continue;
            };
            let Ok(sha) = CommitSha::parse(sha) else {
                continue;
            };
            if let Some(branch) = name.strip_prefix("refs/heads/") {
                listed.local.insert(branch.to_owned(), sha);
            } else if let Some(branch) = name.strip_prefix(&origin)
                && branch != "HEAD"
            {
                listed.origin.insert(branch.to_owned(), sha);
            }
        }
        Ok(listed)
    }
}

/// The worktrees `git worktree list --porcelain` printed, the primary one
/// and those git reports prunable left out.
fn parse_worktrees(listing: &str) -> Vec<ListedWorktree> {
    let mut found: Vec<(ListedWorktree, bool)> = Vec::new();
    for line in listing.lines() {
        if let Some(path) = line.strip_prefix("worktree ") {
            found.push((
                ListedWorktree {
                    path: PathBuf::from(path),
                },
                false,
            ));
        } else if let Some((_, prunable)) = found.last_mut()
            && (line == "prunable" || line.starts_with("prunable "))
        {
            *prunable = true;
        }
    }
    found
        .into_iter()
        .skip(1)
        .filter(|(_, prunable)| !prunable)
        .map(|(worktree, _)| worktree)
        .collect()
}

/// The todo's work branch: the one its record names, else the one its
/// latest work-lane attempt was given.
fn work_branch(evidence: &TodoEvidence) -> Option<&str> {
    evidence.branch.as_deref().or_else(|| {
        evidence
            .bindings
            .iter()
            .rev()
            .filter(|binding| matches!(binding.attempt.lane, Some(Lane::Work | Lane::Prepare)))
            .find_map(|binding| binding.attempt.branch.as_deref())
    })
}

/// The worktree paths the todo's bindings retained.
fn recorded_paths(evidence: &TodoEvidence) -> BTreeSet<PathBuf> {
    evidence
        .bindings
        .iter()
        .filter_map(|binding| binding.worktree())
        .map(|path| same_path(path))
        .collect()
}

/// `path` as the filesystem resolves it, so two spellings of one directory
/// compare equal; as given when it cannot be resolved.
fn same_path(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

/// Each todo of `evidence` whose worktree, local work branch or work branch
/// on `origin` is in `listed`, with its standing. A worktree is the todo's
/// only at a path its bindings recorded: a checkout that merely has the work
/// branch checked out is no assignment's. A branch is the todo's by name.
/// Each listed worktree and branch is attached to one todo: a live todo's
/// record is read before any other, so it wins over an earlier todo that
/// carried the same name.
pub(crate) fn worktree_standings(evidence: &[TodoEvidence], listed: &Listed) -> Vec<TodoWorktree> {
    let mut ordered: Vec<&TodoEvidence> = evidence.iter().collect();
    ordered.sort_by_key(|todo| !matches!(todo.status, TodoStatus::Active | TodoStatus::Iceboxed));
    let mut taken_worktrees: BTreeSet<PathBuf> = BTreeSet::new();
    let mut taken_branches: BTreeSet<String> = BTreeSet::new();
    let mut answers = Vec::new();
    for todo in ordered {
        let branch = work_branch(todo)
            .filter(|name| !taken_branches.contains(*name))
            .map(str::to_owned);
        let paths = recorded_paths(todo);
        let worktree = listed
            .worktrees
            .iter()
            .filter(|listed| !taken_worktrees.contains(&listed.path))
            .find(|listed| paths.contains(&same_path(&listed.path)))
            .map(|listed| listed.path.clone());
        let local = branch.as_ref().and_then(|name| listed.local.get(name));
        let origin = branch.as_ref().and_then(|name| listed.origin.get(name));
        if worktree.is_none() && local.is_none() && origin.is_none() {
            continue;
        }
        let observed = Observations {
            branch_head: local.cloned(),
            remote_head: origin.cloned(),
            ..Observations::default()
        };
        let Some(standing) = standing(todo, &observed) else {
            continue;
        };
        if let Some(path) = &worktree {
            taken_worktrees.insert(path.clone());
        }
        if let Some(name) = &branch {
            taken_branches.insert(name.clone());
        }
        answers.push(TodoWorktree {
            todo: todo.slug.clone(),
            identity: todo.id.clone(),
            revision: todo.revision,
            standing: match standing {
                Standing::UnderWay => WorktreeStanding::UnderWay,
                Standing::Idle => WorktreeStanding::Idle,
                Standing::Delivered => WorktreeStanding::Delivered(DeliveredRemains {
                    worktree: worktree.is_some(),
                    local_branch: local.is_some(),
                    origin_branch: origin.is_some(),
                }),
            },
            worktree,
            branch,
        });
    }
    answers
}

#[cfg(test)]
mod tests {
    use instrukt_runner::interfaces::TodoKind;

    use super::super::_testing::{digest, sha, todo};
    use super::*;
    use crate::evidence::{PreparationReview, ReviewVerdict};

    fn under(evidence: &TodoEvidence) -> Option<Standing> {
        standing(evidence, &Observations::default())
    }

    #[test]
    fn a_parked_todo_is_idle() {
        let parked = todo(TodoKind::Change).status(TodoStatus::Iceboxed).build();
        assert_eq!(under(&parked), Some(Standing::Idle));
    }

    #[test]
    fn a_held_todo_is_idle() {
        assert_eq!(
            under(&todo(TodoKind::Change).held().build()),
            Some(Standing::Idle)
        );
    }

    #[test]
    fn a_todo_whose_finalized_hand_off_stands_for_its_head_is_idle() {
        let handed = todo(TodoKind::Change)
            .stamped(StampFact::FinalizeReady { commit: sha('a') })
            .build();
        let observed = Observations {
            branch_head: Some(sha('a')),
            ..Observations::default()
        };
        assert_eq!(standing(&handed, &observed), Some(Standing::Idle));
    }

    #[test]
    fn a_todo_whose_preparation_evidence_follows_its_build_evidence_is_idle() {
        let returned = todo(TodoKind::Change)
            .stamped(StampFact::BuildComplete { head: sha('a') })
            .artifact(artifact_names::IMPLEMENTATION_PLAN, "plan-v2")
            .build();
        assert_eq!(under(&returned), Some(Standing::Idle));
    }

    #[test]
    fn a_todo_kicked_back_without_new_preparation_evidence_stays_under_way() {
        let kicked = todo(TodoKind::Change)
            .artifact(artifact_names::IMPLEMENTATION_PLAN, "plan-v1")
            .stamped(StampFact::PreparationVerdict {
                phase: PreparationReview::PlanReview,
                verdict: ReviewVerdict::Approve,
                target: digest("plan-v1"),
                report: digest("ok"),
            })
            .stamped(StampFact::BuildComplete { head: sha('a') })
            .stamped(StampFact::Kickback {
                reason: "red CI".into(),
                input: digest("input"),
                invalidated_plan: Some(digest("plan-v1")),
                invalidated_reviews: vec![PreparationReview::PlanReview],
            })
            .build();
        assert_eq!(under(&kicked), Some(Standing::UnderWay));
    }

    #[test]
    fn a_delivered_todo_stands_delivered() {
        let delivered = todo(TodoKind::Change).status(TodoStatus::Delivered).build();
        assert_eq!(under(&delivered), Some(Standing::Delivered));
    }

    #[test]
    fn a_structural_holder_has_no_standing() {
        let holder = todo(TodoKind::Change)
            .status(TodoStatus::StructuralHolder)
            .build();
        assert_eq!(under(&holder), None);
    }
}
