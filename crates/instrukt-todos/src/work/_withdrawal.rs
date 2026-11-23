//! Releasing a withdrawn todo's working resources — its worktrees, empty
//! workspace and work branch — outside the writer lock, under the
//! withdrawal lock and against its recorded intent (`todos`, "Parking,
//! withdrawal and delivery"; `state-database`, `withdrawal_intents`).

use std::path::Path;

use instrukt_core::git;
use instrukt_core::ids::{CommitSha, TodoSlug};

use super::_error::WorkError;
use super::_model::{ReleasedBranch, TodoRecord};
use super::_store::{Checkout, IntentWorktree, Store, WithdrawalIntent, WorktreeTie};

/// The working resources a withdrawal releases, as checked before any is
/// released (`todos`, "Parking, withdrawal and delivery").
pub(super) struct Checked {
    /// The todo's worktrees, none released yet.
    pub worktrees: Vec<IntentWorktree>,
    /// Its work branch, when it exists, and the commit it points at.
    pub branch: Option<String>,
    pub head: Option<CommitSha>,
}

/// What a withdrawal released, kept as recovery evidence: the worktrees
/// removed, and the branch deleted with the commit it pointed at.
pub(super) struct Released {
    pub worktrees: Vec<std::path::PathBuf>,
    pub branch: Option<ReleasedBranch>,
}

/// A worktree of the project as git lists it.
struct Worktree {
    path: std::path::PathBuf,
    /// The commit it has checked out.
    head: Option<CommitSha>,
    /// The branch it has checked out.
    branch: Option<String>,
    /// Why it is locked, when it is (`""` for no reason given).
    locked: Option<String>,
}

impl Worktree {
    /// What it has checked out: its branch, else the commit it is
    /// detached at.
    fn checkout(&self) -> Option<Checkout> {
        match (&self.branch, &self.head) {
            (Some(branch), _) => Some(Checkout::Branch(branch.clone())),
            (None, Some(head)) => Some(Checkout::Detached(head.clone())),
            (None, None) => None,
        }
    }
}

/// The project's worktrees, the primary one first.
fn worktrees(root: &Path) -> Result<Vec<Worktree>, git::GitError> {
    let listed = git::run(root, ["worktree", "list", "--porcelain"])?;
    let mut found: Vec<Worktree> = Vec::new();
    for line in listed.lines() {
        if let Some(path) = line.strip_prefix("worktree ") {
            found.push(Worktree {
                path: std::path::PathBuf::from(path),
                head: None,
                branch: None,
                locked: None,
            });
        } else if let Some(last) = found.last_mut() {
            if let Some(head) = line.strip_prefix("HEAD ") {
                last.head = CommitSha::parse(head).ok();
            } else if let Some(branch) = line.strip_prefix("branch refs/heads/") {
                last.branch = Some(branch.to_owned());
            } else if line == "locked" {
                last.locked = Some(String::new());
            } else if let Some(reason) = line.strip_prefix("locked ") {
                last.locked = Some(reason.to_owned());
            }
        }
    }
    Ok(found)
}

/// Refuses releasing `worktree` when git keeps it by its lock or it holds
/// uncommitted work, naming where it is (`UC-TD5`).
fn check_worktree(worktree: &Worktree) -> Result<(), WorkError> {
    if let Some(reason) = &worktree.locked {
        let why = if reason.is_empty() {
            String::new()
        } else {
            format!(" ({reason})")
        };
        return Err(WorkError::refused(format!(
            "the worktree {} is locked{why}; unlock it with `git worktree unlock`, then withdraw again",
            worktree.path.display()
        )));
    }
    let changes = git::run(&worktree.path, ["status", "--porcelain"])?;
    if !changes.trim().is_empty() {
        return Err(WorkError::refused(format!(
            "the worktree {} holds uncommitted changes; commit or discard them, then withdraw again",
            worktree.path.display()
        )));
    }
    Ok(())
}

/// The todo's workspace `<project>/workspace/<slug>/` when it is present:
/// an empty one is released with the withdrawal, one holding files refuses
/// it.
fn empty_workspace(root: &Path, slug: &TodoSlug) -> Result<Option<std::path::PathBuf>, WorkError> {
    let workspace = root.join("workspace").join(slug.as_str());
    let present = match std::fs::symlink_metadata(&workspace) {
        Ok(meta) if meta.file_type().is_symlink() => {
            return Err(WorkError::Symlink { path: workspace });
        }
        Ok(meta) => meta.is_dir(),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => false,
        Err(source) => {
            return Err(WorkError::Io {
                path: workspace,
                source,
            });
        }
    };
    if !present {
        return Ok(None);
    }
    let mut entries = std::fs::read_dir(&workspace).map_err(|source| WorkError::Io {
        path: workspace.clone(),
        source,
    })?;
    if entries.next().is_some() {
        return Err(WorkError::refused(format!(
            "the workspace {} still holds files; keep or remove them, then withdraw again",
            workspace.display()
        )));
    }
    Ok(Some(workspace))
}

/// Checks the working resources a withdrawn todo owns, releasing nothing:
/// each worktree its session bindings returned that is a worktree of this
/// project, any other worktree with its work branch checked out, its
/// workspace, and its work branch with the commit it points at. Only git's
/// own worktree list and the recorded branch are acted on; a path that is
/// not one of the project's worktrees is left alone. A worktree with
/// uncommitted work or locked, a workspace holding files or a work branch
/// checked out in the project refuses the withdrawal.
pub(super) fn plan_release(root: &Path, record: &TodoRecord) -> Result<Checked, WorkError> {
    let listed = worktrees(root)?;
    let main = listed.first().map(|w| w.path.clone());
    let mut owned: Vec<(&Worktree, WorktreeTie)> = Vec::new();
    for worktree in &listed {
        let bound = record
            .bindings
            .iter()
            .any(|b| b.worktree.as_ref() == Some(&worktree.path));
        let on_branch = worktree.branch.is_some() && worktree.branch == record.branch;
        let tie = if bound {
            WorktreeTie::Bound
        } else {
            WorktreeTie::OnBranch
        };
        if (bound || on_branch)
            && main.as_ref() != Some(&worktree.path)
            && !owned.iter().any(|(o, _)| o.path == worktree.path)
        {
            owned.push((worktree, tie));
        }
    }
    for (worktree, _) in &owned {
        check_worktree(worktree)?;
    }
    empty_workspace(root, &record.slug)?;
    let (branch, head) = match &record.branch {
        Some(name) => match branch_head(root, name)? {
            Some(head) => {
                let current = match git::run(root, ["symbolic-ref", "--quiet", "--short", "HEAD"]) {
                    Ok(current) => Some(current.trim().to_owned()),
                    Err(git::GitError::Failed { .. }) => None,
                    Err(err) => return Err(err.into()),
                };
                if current.as_deref() == Some(name.as_str()) {
                    return Err(WorkError::refused(format!(
                        "the work branch `{name}` is checked out in the project; switch away from it, then withdraw again"
                    )));
                }
                (Some(name.clone()), Some(head))
            }
            None => (None, None),
        },
        None => (None, None),
    };
    Ok(Checked {
        worktrees: owned
            .iter()
            .map(|(w, tie)| IntentWorktree {
                path: w.path.clone(),
                tie: *tie,
                checkout: w.checkout(),
                released: false,
            })
            .collect(),
        branch,
        head,
    })
}

/// The commit work branch `name` points at, `None` when there is no such
/// branch.
fn branch_head(root: &Path, name: &str) -> Result<Option<CommitSha>, git::GitError> {
    let reference = format!("refs/heads/{name}");
    match git::run(
        root,
        ["rev-parse", "--verify", "--quiet", reference.as_str()],
    ) {
        Ok(sha) => Ok(CommitSha::parse(sha.trim()).ok()),
        Err(git::GitError::Failed { .. }) => Ok(None),
        Err(err) => Err(err),
    }
}

/// Releases what `intent` records for the withdrawal of `slug`, outside the
/// writer lock and under the withdrawal lock: each worktree not yet
/// released, the empty workspace, then the work branch. Each outcome is
/// recorded in the intent as it happens, so a later run recognizes it.
///
/// A branch an earlier run deleted is recognized first: gone, or recreated
/// at a commit that does not descend from the one the intent holds, it is
/// recorded as deleted at that commit and a recreated branch is left alone.
/// Each worktree must still have checked out what it had when the
/// withdrawal checked it: one that does not is no longer the todo's and is
/// left where it is, out of the withdrawal. It is checked again just before
/// it goes, and git's own refusals protect what changes after that. A
/// worktree already gone counts as released. A branch that moved on from
/// the commit the intent holds is recorded at, and deleted against, the
/// commit it moved to.
///
/// Anything kept refuses. With nothing released yet the intent is closed and
/// the todo stays as it was; otherwise the intent keeps everything it
/// records, the branch's commit included, and the refusal names what was
/// released.
pub(super) fn release(
    root: &Path,
    slug: &TodoSlug,
    store: &Store,
    mut intent: WithdrawalIntent,
) -> Result<Released, WorkError> {
    let refused = |intent: &WithdrawalIntent, cause: WorkError| -> WorkError {
        if !intent.released_any() {
            match store.abandon_withdrawal(&intent.todo) {
                Ok(true) => return cause,
                Ok(false) => {}
                Err(err) => return err,
            }
        }
        match store.record_withdrawal(intent) {
            Ok(true) => WorkError::refused(format!(
                "`{slug}` is not withdrawn yet: {cause} (already released: {}); make its work safe, then run `instrukt todo remove {slug}` again to complete the withdrawal",
                released_so_far(intent).join(", ")
            )),
            Ok(false) => WorkError::refused(format!(
                "`{slug}`'s withdrawal record vanished while its resources were released; {cause}"
            )),
            Err(err) => err,
        }
    };
    let recorded = |intent: &WithdrawalIntent| -> Result<(), WorkError> {
        if store.record_withdrawal(intent)? {
            Ok(())
        } else {
            Err(WorkError::refused(
                "its withdrawal record vanished while its resources were released",
            ))
        }
    };
    if let (Some(name), false) = (intent.branch.clone(), intent.branch_deleted) {
        match branch_now(root, &name, intent.branch_head.as_ref()) {
            Ok(BranchNow::Gone) => {
                intent.branch_deleted = true;
                if let Err(cause) = recorded(&intent) {
                    return Err(refused(&intent, cause));
                }
            }
            Ok(BranchNow::At(_)) => {}
            Err(err) => return Err(refused(&intent, err.into())),
        }
    }
    let listed = match worktrees(root) {
        Ok(listed) => listed,
        Err(err) => return Err(refused(&intent, err.into())),
    };
    let mut at = 0;
    while at < intent.worktrees.len() {
        if intent.worktrees[at].released {
            at += 1;
            continue;
        }
        let path = intent.worktrees[at].path.clone();
        let Some(worktree) = listed.iter().find(|w| w.path == path) else {
            intent.worktrees[at].released = true;
            at += 1;
            continue;
        };
        if worktree.checkout() != intent.worktrees[at].checkout {
            tracing::info!(
                event = "work_withdrawal_worktree_left",
                todo = %slug,
                worktree = %path.display(),
                "a worktree no longer has the checkout the withdrawal recorded; it is left where it is"
            );
            intent.worktrees.remove(at);
            if let Err(cause) = recorded(&intent) {
                return Err(refused(&intent, cause));
            }
            continue;
        }
        if let Err(cause) = check_worktree(worktree) {
            return Err(refused(&intent, cause));
        }
        let target = path.to_string_lossy().into_owned();
        match git::run(root, ["worktree", "remove", target.as_str()]) {
            Ok(_) => {
                intent.worktrees[at].released = true;
                if let Err(cause) = recorded(&intent) {
                    return Err(refused(&intent, cause));
                }
            }
            Err(git::GitError::Failed { stderr, .. }) => {
                let cause = WorkError::refused(format!(
                    "git kept its worktree {} ({})",
                    path.display(),
                    stderr.trim()
                ));
                return Err(refused(&intent, cause));
            }
            Err(err) => return Err(refused(&intent, err.into())),
        }
        at += 1;
    }
    if !intent.workspace_released {
        match empty_workspace(root, slug) {
            Ok(Some(workspace)) => {
                if let Err(source) = instrukt_core::fsx::remove_dir(&workspace) {
                    let cause = WorkError::refused(format!(
                        "its workspace {} was not removed ({source})",
                        workspace.display()
                    ));
                    return Err(refused(&intent, cause));
                }
                intent.workspace_released = true;
                if let Err(cause) = recorded(&intent) {
                    return Err(refused(&intent, cause));
                }
            }
            Ok(None) => {}
            Err(cause) => return Err(refused(&intent, cause)),
        }
    }
    let branch = match intent.branch.clone() {
        Some(name) => match delete_branch(root, &name, &mut intent, &recorded) {
            Ok(branch) => branch,
            Err(cause) => return Err(refused(&intent, cause)),
        },
        None => None,
    };
    Ok(Released {
        worktrees: intent.worktrees.into_iter().map(|w| w.path).collect(),
        branch,
    })
}

/// What `intent` records as released, for a refusal to name.
fn released_so_far(intent: &WithdrawalIntent) -> Vec<String> {
    let mut released: Vec<String> = intent
        .worktrees
        .iter()
        .filter(|w| w.released)
        .map(|w| format!("its worktree {}", w.path.display()))
        .collect();
    if intent.workspace_released {
        released.push("its workspace".to_owned());
    }
    if let (true, Some(name)) = (intent.branch_deleted, &intent.branch) {
        match &intent.branch_head {
            Some(head) => released.push(format!("its work branch `{name}` at {head}")),
            None => released.push(format!("its work branch `{name}`")),
        }
    }
    released
}

/// Where work branch `name`, recorded at `recorded`, stands now.
enum BranchNow {
    /// Absent, or recreated at a commit that does not descend from
    /// `recorded`: the recorded branch is gone.
    Gone,
    /// At `recorded` or a commit descending from it.
    At(CommitSha),
}

/// Where work branch `name`, recorded at `recorded`, stands now: a tip
/// that differs from `recorded` is the same branch moved on only when it
/// descends from it (`git merge-base --is-ancestor`).
fn branch_now(
    root: &Path,
    name: &str,
    recorded: Option<&CommitSha>,
) -> Result<BranchNow, git::GitError> {
    let Some(head) = branch_head(root, name)? else {
        return Ok(BranchNow::Gone);
    };
    let Some(recorded) = recorded else {
        return Ok(BranchNow::At(head));
    };
    if &head == recorded {
        return Ok(BranchNow::At(head));
    }
    match git::run(
        root,
        [
            "merge-base",
            "--is-ancestor",
            recorded.as_str(),
            head.as_str(),
        ],
    ) {
        Ok(_) => Ok(BranchNow::At(head)),
        // Not an ancestor, or a commit this repository no longer has: the
        // tip cannot be the recorded branch moved on.
        Err(git::GitError::Failed { .. }) => Ok(BranchNow::Gone),
        Err(err) => Err(err),
    }
}

/// Deletes work branch `name` once no worktree has it checked out, with
/// git's compare-and-delete against the commit `intent` records for it,
/// recorded first when the branch moved on since. A branch already deleted,
/// gone, or recreated at an unrelated commit is released at the commit the
/// intent holds and a recreated one is left as it is. Each outcome is
/// recorded through `recorded`. Returns the branch and its last commit,
/// `None` when there is none to record.
fn delete_branch(
    root: &Path,
    name: &str,
    intent: &mut WithdrawalIntent,
    recorded: &dyn Fn(&WithdrawalIntent) -> Result<(), WorkError>,
) -> Result<Option<ReleasedBranch>, WorkError> {
    let deleted = |intent: &WithdrawalIntent| {
        intent.branch_head.clone().map(|head| ReleasedBranch {
            name: name.to_owned(),
            head,
        })
    };
    if intent.branch_deleted {
        return Ok(deleted(intent));
    }
    let kept = |why: String| WorkError::refused(format!("its work branch `{name}` is kept: {why}"));
    let head = match branch_now(root, name, intent.branch_head.as_ref())? {
        BranchNow::Gone => {
            intent.branch_deleted = true;
            recorded(intent)?;
            return Ok(deleted(intent));
        }
        BranchNow::At(head) => head,
    };
    let listed = worktrees(root)?;
    if let Some(user) = listed.iter().find(|w| w.branch.as_deref() == Some(name)) {
        return Err(kept(format!(
            "the worktree {} has it checked out",
            user.path.display()
        )));
    }
    if intent.branch_head.as_ref() != Some(&head) {
        intent.branch_head = Some(head.clone());
        recorded(intent)?;
    }
    let reference = format!("refs/heads/{name}");
    tracing::trace!(
        event = "work_branch_delete_attempted",
        branch = name,
        head = %head,
        "deleting a withdrawn todo's work branch against its recorded commit"
    );
    match git::run(
        root,
        ["update-ref", "-d", reference.as_str(), head.as_str()],
    ) {
        Ok(_) => {}
        Err(git::GitError::Failed { stderr, .. }) => {
            return Err(kept(format!(
                "it moved on from {head} while it was released ({})",
                stderr.trim()
            )));
        }
        Err(err) => return Err(err.into()),
    }
    intent.branch_deleted = true;
    recorded(intent)?;
    // Git's own branch deletion also drops the branch's configuration; a
    // branch without any has none to drop.
    let section = format!("branch.{name}");
    if let Err(err) = git::run(root, ["config", "--remove-section", section.as_str()]) {
        tracing::debug!(event = "work_branch_config_kept", branch = name, error = %err, "no branch configuration removed");
    }
    Ok(Some(ReleasedBranch {
        name: name.to_owned(),
        head,
    }))
}

/// The per-computer withdrawal lock `state/withdrawal.lock`, held while a
/// withdrawal's intent is open, so a withdrawal in progress is told apart
/// from an interrupted one: only its holder releases resources or closes an
/// intent. Taken before the writer lock and never while it is held;
/// released before the withdrawal's reflection.
pub(super) struct WithdrawalLock {
    _guard: instrukt_core::fsx::LockGuard,
}

impl WithdrawalLock {
    /// How long a withdrawal waits for another one on this computer.
    const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

    pub fn acquire(home: &instrukt_core::home::Home) -> Result<Self, WorkError> {
        let path = home.withdrawal_lock_file();
        instrukt_core::state_db::refuse_link("the withdrawal lock", &path)
            .map_err(instrukt_core::state_db::StateDbError::from)?;
        match instrukt_core::fsx::LockGuard::acquire(&path, Self::TIMEOUT) {
            Ok(guard) => Ok(Self { _guard: guard }),
            Err(instrukt_core::fsx::FsError::LockTimeout { .. }) => Err(WorkError::refused(
                "another withdrawal on this computer is still releasing working resources; nothing was done. Run the command again once it has ended",
            )),
            Err(err) => Err(err.into()),
        }
    }
}
