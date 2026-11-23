//! Releasing a withdrawn todo's working resources — its worktrees, empty
//! workspace and work branch — outside the writer lock, under the
//! withdrawal lock and against its recorded intent (`todos`, "Parking,
//! withdrawal and delivery"; `state-database`, `withdrawal_intents`).
//!
//! Each release is an external effect a stopped process may complete
//! without recording it. The intent and git together tell, before and
//! after each effect, whether it has happened:
//!
//! - a worktree the intent records is released through git's own worktree
//!   commands. Checked at its path, it is moved with `git worktree move` to
//!   a staging location beside it, which the intent records before the
//!   move; there it is looked at once more and removed with
//!   `git worktree remove`, or moved back when it is no longer the todo's
//!   or holds uncommitted work. Once it is moved, a git command addressed to
//!   its path finds no worktree there and changes nothing. It is released
//!   once git lists it at neither location;
//! - the workspace the intent records, at the location it was admitted
//!   at, is released once it is gone when the intent recorded it present —
//!   an absent one is never taken for released, and one holding files is
//!   never released;
//! - the work branch is deleted in one `git update-ref --stdin` call that
//!   creates the withdrawal's marker ref (`refs/instrukt/withdrawals/<id>`)
//!   at the commit deleted against, then compare-and-deletes the branch.
//!   While the intent records the branch pending, a marker found with the
//!   branch gone tells this withdrawal's deletion, and the commit it
//!   deleted, from a branch someone else deleted, moved on or recreated
//!   under the same name. A call that returns a failure is read back: no
//!   marker and the branch still at the commit means nothing changed; a
//!   marker beside the untouched branch is recorded as a partial marker
//!   ([`BranchRelease::PartialMarker`]) before it is taken back, so a marker
//!   a failed removal leaves is never read as the deletion, and it is
//!   removed, with the branch recorded pending again, before the branch is
//!   deleted; anything else is left for the withdrawal run again to
//!   recognize. Any other marker is removed only once the intent records
//!   the deletion.
//!
//! A run therefore first recognizes what an earlier run did and records it,
//! then acts. A git or filesystem observation that fails, or an effect that
//! returns a failure without its outcome being proven, is never an absence
//! or an answer: it leaves the intent as it was, with what it records, for
//! the withdrawal to be run again ([`Stop::Unverified`]).

use std::path::{Path, PathBuf};

use instrukt_core::git;
use instrukt_core::ids::{CommitSha, TodoSlug};

use super::_error::WorkError;
use super::_model::{ReleasedBranch, TodoId, TodoRecord};
use super::_store::{
    BranchRelease, Checkout, IntentWorktree, Store, WithdrawalIntent, WorkspaceRelease, WorktreeTie,
};

/// The working resources a withdrawal releases, as checked before any is
/// released (`todos`, "Parking, withdrawal and delivery").
pub(super) struct Checked {
    /// The todo's worktrees, none released yet.
    pub worktrees: Vec<IntentWorktree>,
    /// Where its workspace is, in the checkout the withdrawal is admitted
    /// from, and whether it is there, empty, to release.
    pub workspace_path: PathBuf,
    pub workspace: WorkspaceRelease,
    /// Its work branch, when it exists, and the commit it points at.
    pub branch: Option<String>,
    pub head: Option<CommitSha>,
}

/// What a withdrawal released, kept as recovery evidence: the worktrees
/// removed, and the branch deleted with the commit it pointed at.
pub(super) struct Released {
    pub worktrees: Vec<PathBuf>,
    pub branch: Option<ReleasedBranch>,
}

/// Why releasing stopped short.
#[derive(Debug)]
enum Stop {
    /// Git or the filesystem positively shows that a resource must be kept
    /// — uncommitted work, a lock, files in the workspace, the branch
    /// checked out or moved on — or that an attempted effect changed
    /// nothing or was taken back. A withdrawal that has released nothing may
    /// be abandoned on it.
    Refused(WorkError),
    /// What git or the filesystem holds could not be established, or an
    /// effect returned a failure whose outcome is not proven. Nothing is
    /// concluded from it: the intent keeps everything it records.
    Unverified(WorkError),
}

impl Stop {
    fn refused(reason: impl Into<String>) -> Self {
        Self::Refused(WorkError::refused(reason))
    }

    fn cause(self) -> WorkError {
        match self {
            Self::Refused(cause) | Self::Unverified(cause) => cause,
        }
    }
}

impl From<WorkError> for Stop {
    fn from(err: WorkError) -> Self {
        Self::Unverified(err)
    }
}

impl From<git::GitError> for Stop {
    fn from(err: git::GitError) -> Self {
        Self::Unverified(err.into())
    }
}

/// A worktree of the project as git lists it.
#[derive(Debug, Clone)]
struct Worktree {
    path: PathBuf,
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
                path: PathBuf::from(path),
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
/// uncommitted work, naming it as the worktree at `named`, where it is
/// released from (`UC-TD5`). A status git could not read is no answer.
fn check_worktree(worktree: &Worktree, named: &Path) -> Result<(), Stop> {
    if let Some(reason) = &worktree.locked {
        let why = if reason.is_empty() {
            String::new()
        } else {
            format!(" ({reason})")
        };
        return Err(Stop::refused(format!(
            "the worktree {} is locked{why}; unlock it with `git worktree unlock`, then withdraw again",
            named.display()
        )));
    }
    let changes = git::run(&worktree.path, ["status", "--porcelain"])?;
    if !changes.trim().is_empty() {
        return Err(Stop::refused(format!(
            "the worktree {} holds uncommitted changes; commit or discard them, then withdraw again",
            named.display()
        )));
    }
    Ok(())
}

/// Where the withdrawal of `todo` moves the worktree at `path` to release
/// it: beside it, in the same directory, so git moves it with one rename.
fn staging_path(path: &Path, todo: &TodoId) -> Result<PathBuf, Stop> {
    match (path.parent(), path.file_name()) {
        (Some(parent), Some(name)) => Ok(parent.join(format!(
            ".instrukt-withdrawal-{}-{}",
            todo.as_str(),
            name.to_string_lossy()
        ))),
        _ => Err(Stop::Unverified(WorkError::refused(format!(
            "the worktree {} has no directory beside it to be released from",
            path.display()
        )))),
    }
}

/// Where git lists a worktree the intent records.
#[derive(Debug)]
enum Located {
    /// At its staging location: the withdrawal moved it there.
    Staged(Worktree),
    /// At its own path, and nothing is at its staging location: not moved.
    AtPath(Worktree),
    /// Nowhere: git no longer has it.
    Neither,
}

/// Where `listed`, the project's worktrees as git lists them now, has the
/// worktree `recorded`, with `staging_exists` telling whether anything is
/// at its recorded staging location. `None` when it is listed at its path
/// while its staging location exists: git may have moved it without
/// recording where, so that is no answer.
fn place(listed: &[Worktree], recorded: &IntentWorktree, staging_exists: bool) -> Option<Located> {
    if let Some(staging) = &recorded.staging
        && let Some(staged) = listed.iter().find(|w| &w.path == staging)
    {
        return Some(Located::Staged(staged.clone()));
    }
    match listed.iter().find(|w| w.path == recorded.path) {
        None => Some(Located::Neither),
        Some(_) if recorded.staging.is_some() && staging_exists => None,
        Some(at_path) => Some(Located::AtPath(at_path.clone())),
    }
}

/// [`place`], looking at the recorded staging location when git does not
/// list the worktree there.
fn locate(listed: &[Worktree], recorded: &IntentWorktree) -> Result<Located, Stop> {
    let staging_exists = match &recorded.staging {
        Some(staging) if !listed.iter().any(|w| &w.path == staging) => {
            match std::fs::symlink_metadata(staging) {
                Ok(_) => true,
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => false,
                Err(source) => {
                    return Err(Stop::Unverified(WorkError::Io {
                        path: staging.clone(),
                        source,
                    }));
                }
            }
        }
        _ => false,
    };
    place(listed, recorded, staging_exists).ok_or_else(|| {
        let staging = recorded
            .staging
            .as_deref()
            .unwrap_or(&recorded.path)
            .display();
        Stop::Unverified(WorkError::refused(format!(
            "git lists the worktree {} at its own path while {staging} exists, where the withdrawal moves it to be released; if git moved it there without recording it, run `git worktree repair {staging}`, otherwise remove {staging}; then withdraw again",
            recorded.path.display()
        )))
    })
}

/// The todo's workspace `<project>/workspace/<slug>/` as it is now.
#[derive(Debug, Clone, PartialEq, Eq)]
enum WorkspaceNow {
    /// There is no workspace directory.
    Absent,
    /// It is there and empty.
    Empty(PathBuf),
    /// It is there and holds files.
    Holding(PathBuf),
}

/// Where the todo's workspace is in the checkout at `root`.
fn workspace_path(root: &Path, slug: &TodoSlug) -> PathBuf {
    root.join("workspace").join(slug.as_str())
}

/// The workspace at `workspace` as it is now. Something other than a
/// directory there is kept; a read that fails is no answer.
fn workspace_now(workspace: &Path) -> Result<WorkspaceNow, Stop> {
    let present = match std::fs::symlink_metadata(workspace) {
        Ok(meta) if meta.file_type().is_symlink() => {
            return Err(Stop::Refused(WorkError::Symlink {
                path: workspace.to_path_buf(),
            }));
        }
        Ok(meta) => meta.is_dir(),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => false,
        Err(source) => {
            return Err(Stop::Unverified(WorkError::Io {
                path: workspace.to_path_buf(),
                source,
            }));
        }
    };
    if !present {
        return Ok(WorkspaceNow::Absent);
    }
    let mut entries = std::fs::read_dir(workspace).map_err(|source| {
        Stop::Unverified(WorkError::Io {
            path: workspace.to_path_buf(),
            source,
        })
    })?;
    match entries.next() {
        Some(Ok(_)) => Ok(WorkspaceNow::Holding(workspace.to_path_buf())),
        None => Ok(WorkspaceNow::Empty(workspace.to_path_buf())),
        Some(Err(source)) => Err(Stop::Unverified(WorkError::Io {
            path: workspace.to_path_buf(),
            source,
        })),
    }
}

/// The refusal for a workspace holding files.
fn holding_files(workspace: &Path) -> Stop {
    Stop::refused(format!(
        "the workspace {} still holds files; keep or remove them, then withdraw again",
        workspace.display()
    ))
}

/// What a run does with the todo's workspace, recorded as `recorded`, now
/// that it is `now`.
#[derive(Debug, PartialEq, Eq)]
enum WorkspaceStep {
    /// Nothing: there was none to release, or it is released.
    Nothing,
    /// It was recorded present and is gone: it is released.
    Gone,
    /// Remove it.
    Remove(PathBuf),
    /// It holds files now: the withdrawal is refused.
    Holding(PathBuf),
}

/// Files in the workspace refuse the withdrawal whatever the intent
/// recorded of it; otherwise only a workspace recorded present is
/// released, so one never there is not taken for released.
fn workspace_step(recorded: WorkspaceRelease, now: WorkspaceNow) -> WorkspaceStep {
    match (recorded, now) {
        (_, WorkspaceNow::Holding(path)) => WorkspaceStep::Holding(path),
        (WorkspaceRelease::Absent | WorkspaceRelease::Released, _) => WorkspaceStep::Nothing,
        (WorkspaceRelease::Present, WorkspaceNow::Absent) => WorkspaceStep::Gone,
        (WorkspaceRelease::Present, WorkspaceNow::Empty(path)) => WorkspaceStep::Remove(path),
    }
}

/// What a run does with a worktree the intent records and has not
/// released, as git lists it now (`None`: not listed).
#[derive(Debug)]
enum WorktreeStep {
    /// Git no longer lists it: it is released.
    Gone,
    /// It is no longer the todo's: left where it is and out of the
    /// withdrawal.
    Left,
    /// It is still the todo's: release it.
    Release(Worktree),
}

/// A worktree is still the todo's only while it has the checkout the
/// withdrawal recorded, and, when that checkout is the work branch, only
/// while that branch is still the one the withdrawal recorded
/// (`branch_gone` false): a checkout of a branch recreated under the same
/// name belongs to whoever recreated it.
fn worktree_step(
    recorded: &IntentWorktree,
    now: Option<Worktree>,
    work_branch: Option<&str>,
    branch_gone: bool,
) -> WorktreeStep {
    let Some(now) = now else {
        return WorktreeStep::Gone;
    };
    if now.checkout() != recorded.checkout || (branch_gone && on_branch(recorded, work_branch)) {
        return WorktreeStep::Left;
    }
    WorktreeStep::Release(now)
}

/// Whether `recorded` had the work branch `branch` checked out when the
/// withdrawal checked it.
fn on_branch(recorded: &IntentWorktree, branch: Option<&str>) -> bool {
    matches!(
        (&recorded.checkout, branch),
        (Some(Checkout::Branch(on)), Some(branch)) if on == branch
    )
}

/// The ref the deletion of a withdrawn todo's work branch creates in the
/// same `update-ref` call, at the commit it deleted against: proof of the
/// deletion that outlives a lost receipt, while the intent records the
/// branch pending. Removed once the intent records the deletion, or as a
/// partial marker before the branch is deleted.
fn marker(todo: &TodoId) -> String {
    format!("refs/instrukt/withdrawals/{}", todo.as_str())
}

/// What a marker found at `at` means, with the branch's release recorded
/// as `state`.
#[derive(Debug, PartialEq, Eq)]
enum MarkerStep {
    /// No marker: nothing.
    Nothing,
    /// The branch pending: a deletion whose receipt was lost created it,
    /// and the branch was deleted at its commit.
    Deletion(CommitSha),
    /// A partial marker, which proves nothing, or one whose deletion the
    /// intent records: removed.
    Remove(CommitSha),
}

fn marker_step(state: BranchRelease, at: Option<CommitSha>) -> MarkerStep {
    match (state, at) {
        (_, None) => MarkerStep::Nothing,
        (BranchRelease::Pending, Some(at)) => MarkerStep::Deletion(at),
        (BranchRelease::PartialMarker | BranchRelease::Deleted, Some(at)) => MarkerStep::Remove(at),
    }
}

/// The commit ref `reference` points at; `None` when git reports no such
/// ref. A failure to read it is an error.
fn ref_tip(root: &Path, reference: &str) -> Result<Option<CommitSha>, WorkError> {
    let listed = git::run(
        root,
        [
            "for-each-ref",
            "--format=%(objectname) %(refname)",
            reference,
        ],
    )?;
    for line in listed.lines() {
        if let Some((sha, name)) = line.split_once(' ')
            && name == reference
        {
            let sha = CommitSha::parse(sha).map_err(|err| {
                WorkError::refused(format!(
                    "git reports {reference} at {sha}, not a commit: {err}"
                ))
            })?;
            return Ok(Some(sha));
        }
    }
    Ok(None)
}

/// Whether `tip` is `recorded` or descends from it. A `recorded` commit
/// git positively reports absent is no ancestor of anything it has; a
/// question git could not answer, such as an object store it could not
/// read, is an error ([`git::answers`]).
fn descends(root: &Path, recorded: &CommitSha, tip: &CommitSha) -> Result<bool, WorkError> {
    if recorded == tip {
        return Ok(true);
    }
    if !git::answers(root, ["cat-file", "-e", recorded.as_str()])? {
        return Ok(false);
    }
    Ok(git::answers(
        root,
        [
            "merge-base",
            "--is-ancestor",
            recorded.as_str(),
            tip.as_str(),
        ],
    )?)
}

/// Where the work branch a withdrawal releases stands now.
#[derive(Debug, PartialEq, Eq)]
enum BranchNow {
    /// This withdrawal's deletion left its marker: the branch was deleted
    /// at this commit, whatever carries its name now.
    Deleted(CommitSha),
    /// No branch carries the name, or the one that does points at a commit
    /// that does not descend from the recorded one: the recorded branch is
    /// gone.
    Gone,
    /// The branch points at the recorded commit or a descendant of it.
    Live(CommitSha),
}

/// Where `intent`'s work branch `name` stands now. Its marker shows a
/// deletion only while the intent records the branch pending
/// ([`marker_step`]).
fn branch_now(root: &Path, intent: &WithdrawalIntent, name: &str) -> Result<BranchNow, WorkError> {
    if let MarkerStep::Deletion(at) =
        marker_step(intent.branch_state, ref_tip(root, &marker(&intent.todo))?)
    {
        return Ok(BranchNow::Deleted(at));
    }
    let Some(tip) = ref_tip(root, &format!("refs/heads/{name}"))? else {
        return Ok(BranchNow::Gone);
    };
    match intent.branch_head.as_ref() {
        Some(recorded) if !descends(root, recorded, &tip)? => Ok(BranchNow::Gone),
        _ => Ok(BranchNow::Live(tip)),
    }
}

/// Records in `intent` that its branch is gone, at the commit
/// `branch_now` found it deleted at, if any; `false` when the branch is
/// still the recorded one.
fn note_gone(intent: &mut WithdrawalIntent, now: &BranchNow) -> bool {
    match now {
        BranchNow::Deleted(at) => {
            intent.branch_head = Some(at.clone());
            intent.branch_state = BranchRelease::Deleted;
            true
        }
        BranchNow::Gone => {
            intent.branch_state = BranchRelease::Deleted;
            true
        }
        BranchNow::Live(_) => false,
    }
}

/// Checks the working resources a withdrawn todo owns, releasing nothing:
/// each worktree its session bindings returned that is a worktree of this
/// project, any other worktree with its work branch checked out, its
/// workspace in the checkout at `root`, and its work branch with the commit
/// it points at. Only git's own worktree list and the recorded branch are
/// acted on; a path that is not one of the project's worktrees is left
/// alone. A worktree with uncommitted work or locked, a workspace holding
/// files or a work branch checked out in the project refuses the
/// withdrawal.
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
        check_worktree(worktree, &worktree.path).map_err(Stop::cause)?;
    }
    let path = workspace_path(root, &record.slug);
    let workspace = match workspace_now(&path).map_err(Stop::cause)? {
        WorkspaceNow::Absent => WorkspaceRelease::Absent,
        WorkspaceNow::Empty(_) => WorkspaceRelease::Present,
        WorkspaceNow::Holding(path) => return Err(holding_files(&path).cause()),
    };
    let (branch, head) = match &record.branch {
        Some(name) => match ref_tip(root, &format!("refs/heads/{name}"))? {
            Some(head) => {
                let current = git::run(root, ["rev-parse", "--symbolic-full-name", "HEAD"])?;
                if current.trim() == format!("refs/heads/{name}") {
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
                staging: None,
            })
            .collect(),
        workspace_path: path,
        workspace,
        branch,
        head,
    })
}

/// Releases what `intent` records for the withdrawal of `slug`, outside the
/// writer lock and under the withdrawal lock, recording each outcome in the
/// intent as it happens. `fresh` is an intent this command recorded, which
/// nothing has acted on before. The checkout at `root` only lists the
/// project's worktrees and branches, which every checkout of the project
/// shares; the workspace acted on is the one the intent recorded.
///
/// An intent an earlier run left is first brought up to date with what
/// that run did ([`recognize`]). Then each worktree not yet released, the
/// workspace, and the work branch are released in that order, each
/// observed again just before it is acted on:
///
/// - a worktree is released only while it is still the todo's
///   ([`worktree_step`]): checked at its path, moved to its staging
///   location, looked at and checked again there, and removed there
///   ([`release_worktree`]); one that is not the todo's is left out, moved
///   back first when it was moved;
/// - the work branch, still the recorded one, is recorded at the commit it
///   points at when it moved on to a descendant, then deleted against that
///   commit together with its marker; gone or replaced, it is recorded
///   deleted at the recorded commit, and anything carrying its name is left
///   as it is ([`delete_branch`]).
///
/// How a stop ends the run:
///
/// - a positive refusal ([`Stop::Refused`]) closes the intent and leaves
///   the todo as it was when the intent records nothing released and no
///   effect was attempted, or each one attempted is proven to have changed
///   nothing or was taken back: a branch update the refs read back show did
///   nothing, a worktree move git refused, a worktree moved back. Otherwise
///   it keeps the intent;
/// - an unverified observation or effect ([`Stop::Unverified`]) keeps the
///   intent, even when it records nothing released, because what git or the
///   filesystem holds was not established;
/// - a branch update that left only its marker keeps the intent with the
///   marker recorded as partial before it is taken back ([`delete_branch`]).
///
/// A kept intent keeps everything it records, the branch's commit and any
/// worktree's staging location included, and the refusal names what is
/// recorded released, and, for an unverified stop, that the rest could not
/// be established.
pub(super) fn release(
    root: &Path,
    slug: &TodoSlug,
    store: &Store,
    mut intent: WithdrawalIntent,
    fresh: bool,
) -> Result<Released, WorkError> {
    let kept = |intent: &WithdrawalIntent, stop: Stop| -> WorkError {
        match store.record_withdrawal(intent) {
            Ok(true) => {
                let released = released_so_far(intent);
                match stop {
                    Stop::Refused(cause) => {
                        let released = if released.is_empty() {
                            "nothing is released yet".to_owned()
                        } else {
                            format!("already released: {}", released.join(", "))
                        };
                        WorkError::refused(format!(
                            "`{slug}` is not withdrawn yet: {cause} ({released}); make its work safe, then run `instrukt todo remove {slug}` again to complete the withdrawal"
                        ))
                    }
                    Stop::Unverified(cause) => {
                        let recorded = if released.is_empty() {
                            "it records nothing released".to_owned()
                        } else {
                            format!("it records as released {}", released.join(", "))
                        };
                        WorkError::refused(format!(
                            "`{slug}` is not withdrawn yet: {cause}. Its withdrawal stays recorded ({recorded}); what could not be established is left as it is. Once the cause is resolved, run `instrukt todo remove {slug}` again to complete the withdrawal"
                        ))
                    }
                }
            }
            Ok(false) => WorkError::refused(format!(
                "`{slug}`'s withdrawal record vanished while its resources were released; {}",
                stop.cause()
            )),
            Err(err) => err,
        }
    };
    // A stop the run can prove, while no effect was attempted or each one
    // attempted is proven to have changed nothing, and the intent records
    // nothing released, leaves the todo as it was.
    let stopped = |intent: &WithdrawalIntent, acted: bool, stop: Stop| -> WorkError {
        if let Stop::Refused(_) = &stop
            && !acted
            && !intent.released_any()
        {
            match store.abandon_withdrawal(&intent.todo) {
                Ok(true) => return stop.cause(),
                Ok(false) => {}
                Err(err) => return err,
            }
        }
        kept(intent, stop)
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
    if !fresh {
        // What an earlier run cannot be told not to have done keeps the
        // intent: an observation that fails here is never an abandonment.
        if let Err(cause) = recognize(root, &mut intent).and_then(|()| recorded(&intent)) {
            return Err(kept(&intent, Stop::Unverified(cause)));
        }
    }
    let mut acted = false;
    let mut at = 0;
    while at < intent.worktrees.len() {
        if intent.worktrees[at].released {
            at += 1;
            continue;
        }
        match release_worktree(root, slug, &mut intent, at, &recorded) {
            Ok(Advance::Released { removed }) => {
                acted |= removed;
                at += 1;
            }
            Ok(Advance::Left) => {}
            Err(Attempt { stop, removing }) => {
                // A worktree moved, or a move or removal git returned a
                // failure for, may be anywhere between its path and its
                // removal: never an abandonment.
                let stop = match *stop {
                    Stop::Refused(cause) if removing => Stop::Unverified(cause),
                    other => other,
                };
                return Err(stopped(&intent, acted || removing, stop));
            }
        }
    }
    let workspace = match workspace_now(&intent.workspace_path) {
        Ok(now) => workspace_step(intent.workspace, now),
        Err(stop) => return Err(stopped(&intent, acted, stop)),
    };
    match workspace {
        WorkspaceStep::Nothing => {}
        WorkspaceStep::Holding(path) => return Err(stopped(&intent, acted, holding_files(&path))),
        WorkspaceStep::Gone | WorkspaceStep::Remove(_) => {
            if let WorkspaceStep::Remove(path) = workspace {
                acted = true;
                if let Err(source) = instrukt_core::fsx::remove_dir(&path) {
                    let cause = WorkError::refused(format!(
                        "its workspace {} was not removed ({source})",
                        path.display()
                    ));
                    return Err(kept(&intent, Stop::Unverified(cause)));
                }
            }
            intent.workspace = WorkspaceRelease::Released;
            if let Err(cause) = recorded(&intent) {
                return Err(kept(&intent, Stop::Unverified(cause)));
            }
        }
    }
    let branch = match intent.branch.clone() {
        Some(name) => match delete_branch(root, &name, &mut intent, &recorded, &mut acted) {
            Ok(branch) => branch,
            Err(stop) => return Err(stopped(&intent, acted, stop)),
        },
        None => None,
    };
    Ok(Released {
        worktrees: intent.worktrees.into_iter().map(|w| w.path).collect(),
        branch,
    })
}

/// How the release of one worktree ended.
enum Advance {
    /// It is released and recorded; `removed` when this run removed it.
    Released { removed: bool },
    /// It is no longer the todo's: left where it is and out of the intent.
    Left,
}

/// Why the release of one worktree stopped, and whether its removal had
/// been started.
struct Attempt {
    stop: Box<Stop>,
    removing: bool,
}

impl Attempt {
    /// A stop while the worktree at `path` may be at `staging`: never an
    /// abandonment, and the refusal names where it is kept.
    fn moving(path: &Path, staging: &Path, stop: Stop) -> Self {
        let stop = match stop {
            Stop::Refused(cause) | Stop::Unverified(cause) => {
                Stop::Unverified(WorkError::refused(format!(
                    "its worktree {} is moved to {} to be released, and kept there: {cause}",
                    path.display(),
                    staging.display()
                )))
            }
        };
        Self {
            stop: Box::new(stop),
            removing: true,
        }
    }
}

impl<E: Into<Stop>> From<E> for Attempt {
    fn from(err: E) -> Self {
        Self {
            stop: Box::new(err.into()),
            removing: false,
        }
    }
}

/// Releases the worktree `intent` records at `at`, recording the outcome,
/// through git's own worktree commands. One found at its path, still the
/// todo's and clean, is moved to its staging location, recorded before the
/// move ([`staging_path`]), and finished there ([`finish_staged`]); one an
/// earlier run left at its staging location is finished there directly.
///
/// Moving it is what keeps its checkout from changing unseen between the
/// last look and the removal: once moved, a git command addressed to its
/// path finds no worktree there, and one that ran before the move has
/// ended or is seen at the staging location. A move git refuses leaves the
/// worktree where it was; the refusal is proven to have changed nothing.
fn release_worktree(
    root: &Path,
    slug: &TodoSlug,
    intent: &mut WithdrawalIntent,
    at: usize,
    recorded: &dyn Fn(&WithdrawalIntent) -> Result<(), WorkError>,
) -> Result<Advance, Attempt> {
    let path = intent.worktrees[at].path.clone();
    let now = match locate(&worktrees(root)?, &intent.worktrees[at])? {
        Located::Staged(staged) => return finish_staged(root, slug, intent, at, staged, recorded),
        Located::AtPath(listed) => {
            if intent.worktrees[at].staging.take().is_some() {
                recorded(intent)?;
            }
            Some(listed)
        }
        Located::Neither => None,
    };
    if now.is_some() {
        note_branch(root, intent, at, recorded)?;
    }
    let worktree = match worktree_step(
        &intent.worktrees[at],
        now,
        intent.branch.as_deref(),
        intent.branch_deleted(),
    ) {
        WorktreeStep::Gone => return released(intent, at, false, recorded),
        WorktreeStep::Left => return left(slug, intent, at, recorded),
        WorktreeStep::Release(worktree) => worktree,
    };
    check_worktree(&worktree, &path)?;
    let staging = staging_path(&path, &intent.todo)?;
    match std::fs::symlink_metadata(&staging) {
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Ok(_) => {
            return Err(Stop::refused(format!(
                "{} is in the way of releasing the worktree {}; remove it, then withdraw again",
                staging.display(),
                path.display()
            ))
            .into());
        }
        Err(source) => {
            return Err(Stop::Unverified(WorkError::Io {
                path: staging,
                source,
            })
            .into());
        }
    }
    intent.worktrees[at].staging = Some(staging.clone());
    recorded(intent)?;
    let (from, to) = (
        path.to_string_lossy().into_owned(),
        staging.to_string_lossy().into_owned(),
    );
    let moved = git::run(root, ["worktree", "move", from.as_str(), to.as_str()]);
    // Read back where git has it, whatever the move returned.
    let found = worktrees(root)
        .map_err(Stop::from)
        .and_then(|listed| locate(&listed, &intent.worktrees[at]));
    match (moved, found) {
        (_, Ok(Located::Staged(staged))) => finish_staged(root, slug, intent, at, staged, recorded),
        (Err(git::GitError::Failed { stderr, .. }), Ok(Located::AtPath(_))) => {
            intent.worktrees[at].staging = None;
            recorded(intent)?;
            Err(Stop::refused(format!(
                "git kept the worktree {} where it is ({})",
                path.display(),
                stderr.trim()
            ))
            .into())
        }
        (Err(err), Ok(_)) => Err(Attempt::moving(
            &path,
            &staging,
            Stop::Unverified(WorkError::refused(format!(
                "git returned a failure moving it ({err}); where it is now is recognized when the withdrawal is run again"
            ))),
        )),
        (Ok(_), Ok(_)) => Err(Attempt::moving(
            &path,
            &staging,
            Stop::Unverified(WorkError::refused(
                "git moved it but does not list it there",
            )),
        )),
        (_, Err(stop)) => Err(Attempt::moving(&path, &staging, stop)),
    }
}

/// Finishes the release of the worktree `intent` records at `at`, which git
/// lists as `staged` at its staging location: looked at once more there, it
/// is removed while it is still the todo's and clean, and moved back to its
/// path otherwise. Moved back because it holds uncommitted work or is
/// locked, it refuses the withdrawal naming its path; moved back because it
/// is no longer the todo's, it is left out of the withdrawal. One git does
/// not move back stays recorded at its staging location, named, for the
/// withdrawal run again.
fn finish_staged(
    root: &Path,
    slug: &TodoSlug,
    intent: &mut WithdrawalIntent,
    at: usize,
    staged: Worktree,
    recorded: &dyn Fn(&WithdrawalIntent) -> Result<(), WorkError>,
) -> Result<Advance, Attempt> {
    let path = intent.worktrees[at].path.clone();
    let staging = staged.path.clone();
    let moving = |stop: Stop| Attempt::moving(&path, &staging, stop);
    note_branch(root, intent, at, recorded).map_err(|a| moving(*a.stop))?;
    let worktree = match worktree_step(
        &intent.worktrees[at],
        Some(staged),
        intent.branch.as_deref(),
        intent.branch_deleted(),
    ) {
        WorktreeStep::Release(worktree) => worktree,
        WorktreeStep::Gone => return released(intent, at, false, recorded),
        WorktreeStep::Left => {
            move_back(root, intent, at, &staging, recorded)?;
            return left(slug, intent, at, recorded);
        }
    };
    match check_worktree(&worktree, &path) {
        Ok(()) => {}
        Err(Stop::Refused(cause)) => {
            move_back(root, intent, at, &staging, recorded)?;
            return Err(Stop::Refused(cause).into());
        }
        Err(stop) => return Err(moving(stop)),
    }
    let target = staging.to_string_lossy().into_owned();
    if let Err(err) = git::run(root, ["worktree", "remove", target.as_str()]) {
        let cause = match err {
            git::GitError::Failed { stderr, .. } => {
                WorkError::refused(format!("git kept it ({})", stderr.trim()))
            }
            other => other.into(),
        };
        return Err(moving(Stop::Unverified(cause)));
    }
    released(intent, at, true, recorded)
}

/// Moves the worktree `intent` records at `at` back from `staging` to its
/// path. One git does not move back stays recorded there, named.
fn move_back(
    root: &Path,
    intent: &mut WithdrawalIntent,
    at: usize,
    staging: &Path,
    recorded: &dyn Fn(&WithdrawalIntent) -> Result<(), WorkError>,
) -> Result<(), Attempt> {
    let path = intent.worktrees[at].path.clone();
    let (from, to) = (
        staging.to_string_lossy().into_owned(),
        path.to_string_lossy().into_owned(),
    );
    if let Err(err) = git::run(root, ["worktree", "move", from.as_str(), to.as_str()]) {
        let failure = match err {
            git::GitError::Failed { stderr, .. } => stderr.trim().to_owned(),
            other => other.to_string(),
        };
        return Err(Attempt::moving(
            &path,
            staging,
            Stop::Unverified(WorkError::refused(format!(
                "it is no longer releasable and git did not move it back ({failure}); running the withdrawal again moves it back or releases it"
            ))),
        ));
    }
    intent.worktrees[at].staging = None;
    recorded(intent).map_err(|cause| Attempt {
        stop: Box::new(Stop::Unverified(cause)),
        removing: true,
    })
}

/// Records the worktree `intent` records at `at` released; `removed` when
/// this run removed it.
fn released(
    intent: &mut WithdrawalIntent,
    at: usize,
    removed: bool,
    recorded: &dyn Fn(&WithdrawalIntent) -> Result<(), WorkError>,
) -> Result<Advance, Attempt> {
    intent.worktrees[at].released = true;
    intent.worktrees[at].staging = None;
    recorded(intent).map_err(|cause| Attempt {
        stop: Box::new(Stop::Unverified(cause)),
        removing: removed,
    })?;
    Ok(Advance::Released { removed })
}

/// Leaves the worktree `intent` records at `at`, no longer the todo's, out
/// of the withdrawal, where it is.
fn left(
    slug: &TodoSlug,
    intent: &mut WithdrawalIntent,
    at: usize,
    recorded: &dyn Fn(&WithdrawalIntent) -> Result<(), WorkError>,
) -> Result<Advance, Attempt> {
    tracing::info!(
        event = "work_withdrawal_worktree_left",
        todo = %slug,
        worktree = %intent.worktrees[at].path.display(),
        "a worktree is no longer the todo's; it is left where it is"
    );
    intent.worktrees.remove(at);
    recorded(intent)?;
    Ok(Advance::Left)
}

/// Records the work branch gone or deleted when the worktree `intent`
/// records at `at`, still listed, had it checked out: a checkout of a
/// branch recreated under the same name is not the todo's.
fn note_branch(
    root: &Path,
    intent: &mut WithdrawalIntent,
    at: usize,
    recorded: &dyn Fn(&WithdrawalIntent) -> Result<(), WorkError>,
) -> Result<(), Attempt> {
    if !intent.branch_deleted()
        && on_branch(&intent.worktrees[at], intent.branch.as_deref())
        && let Some(name) = intent.branch.clone()
    {
        let seen = branch_now(root, intent, &name)?;
        if note_gone(intent, &seen) {
            recorded(intent)?;
        }
    }
    Ok(())
}

/// Brings `intent`, left by an earlier run, up to date with what that run
/// did and could not record, and with what happened to its resources
/// since: the branch deleted by it (its marker, while the branch is
/// recorded pending) or gone, each worktree git lists at neither its path
/// nor its staging location, a move to its staging location that did not
/// happen, and the workspace it recorded present, at the location it
/// recorded, that is gone. A worktree git lists at its staging location is
/// finished from there by its release; a partial marker by the branch's
/// deletion.
fn recognize(root: &Path, intent: &mut WithdrawalIntent) -> Result<(), WorkError> {
    if let (Some(name), BranchRelease::Pending) = (intent.branch.clone(), intent.branch_state) {
        let now = branch_now(root, intent, &name)?;
        note_gone(intent, &now);
    }
    if intent.worktrees.iter().any(|w| !w.released) {
        let listed = worktrees(root)?;
        for worktree in intent.worktrees.iter_mut().filter(|w| !w.released) {
            match locate(&listed, worktree).map_err(Stop::cause)? {
                Located::Staged(_) => {}
                Located::AtPath(_) => worktree.staging = None,
                Located::Neither => {
                    worktree.released = true;
                    worktree.staging = None;
                }
            }
        }
    }
    let now = workspace_now(&intent.workspace_path).map_err(Stop::cause)?;
    if workspace_step(intent.workspace, now) == WorkspaceStep::Gone {
        intent.workspace = WorkspaceRelease::Released;
    }
    Ok(())
}

/// What `intent` records as released, for a refusal to name.
fn released_so_far(intent: &WithdrawalIntent) -> Vec<String> {
    let mut released: Vec<String> = intent
        .worktrees
        .iter()
        .filter(|w| w.released)
        .map(|w| format!("its worktree {}", w.path.display()))
        .collect();
    if intent.workspace == WorkspaceRelease::Released {
        released.push(format!("its workspace {}", intent.workspace_path.display()));
    }
    if let (true, Some(name)) = (intent.branch_deleted(), &intent.branch) {
        match &intent.branch_head {
            Some(head) => released.push(format!("its work branch `{name}` at {head}")),
            None => released.push(format!("its work branch `{name}`")),
        }
    }
    released
}

/// What the refs read back after an `update-ref` call that returned a
/// failure show it did.
#[derive(Debug, PartialEq, Eq)]
enum DeletionAfterFailure {
    /// No marker, and the branch still at the commit deleted against:
    /// nothing changed.
    Ineffective,
    /// No marker, and the branch at another commit: it moved on, and
    /// nothing changed.
    MovedOn,
    /// The marker was created and the branch is still at the commit: only
    /// the marker took effect. It is recorded partial, then taken back.
    MarkerOnly,
    /// Anything else — the branch gone, with or without the marker: the
    /// outcome is left for the withdrawal run again to recognize.
    Unsettled,
}

fn deletion_after_failure(
    head: &CommitSha,
    marker: Option<&CommitSha>,
    branch: Option<&CommitSha>,
) -> DeletionAfterFailure {
    match (marker, branch) {
        (None, Some(tip)) if tip == head => DeletionAfterFailure::Ineffective,
        (None, Some(_)) => DeletionAfterFailure::MovedOn,
        (Some(at), Some(tip)) if at == head && tip == head => DeletionAfterFailure::MarkerOnly,
        _ => DeletionAfterFailure::Unsettled,
    }
}

/// Deletes work branch `name` once no worktree has it checked out: in one
/// `update-ref` call against the commit `intent` records for it, recorded
/// first when the branch moved on since, together with the withdrawal's
/// marker at that commit. A branch already deleted, gone, or recreated at
/// an unrelated commit is released at the commit the intent holds, and
/// whatever carries its name is left as it is. Each outcome is recorded
/// through `recorded`; the marker is removed once the deletion is.
///
/// A partial marker an earlier run recorded is removed first, and the
/// branch recorded pending again, before the branch is looked at: it is
/// never read as the deletion.
///
/// A call that returns a failure is never taken for a deletion: the refs
/// are read back ([`deletion_after_failure`]). Proven to have changed
/// nothing, the attempt does not count as one. A marker it left beside the
/// untouched branch is first recorded partial, then removed, and recorded
/// pending again once it is gone; the intent is kept either way, as it is
/// for anything else, for the withdrawal run again. Returns the branch and
/// its last commit, `None` when there is none to record.
fn delete_branch(
    root: &Path,
    name: &str,
    intent: &mut WithdrawalIntent,
    recorded: &dyn Fn(&WithdrawalIntent) -> Result<(), WorkError>,
    acted: &mut bool,
) -> Result<Option<ReleasedBranch>, Stop> {
    let marker = marker(&intent.todo);
    if intent.branch_state == BranchRelease::PartialMarker {
        if let MarkerStep::Remove(at) = marker_step(intent.branch_state, ref_tip(root, &marker)?) {
            git::run(root, ["update-ref", "-d", marker.as_str(), at.as_str()])?;
        }
        intent.branch_state = BranchRelease::Pending;
        recorded(intent)?;
    }
    if !intent.branch_deleted() {
        let kept = |why: String| Stop::refused(format!("its work branch `{name}` is kept: {why}"));
        match branch_now(root, intent, name)? {
            BranchNow::Live(head) => {
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
                tracing::trace!(
                    event = "work_branch_delete_attempted",
                    branch = name,
                    head = %head,
                    "deleting a withdrawn todo's work branch against its recorded commit"
                );
                let attempted_before = *acted;
                *acted = true;
                let transaction =
                    format!("create {marker} {head}\ndelete refs/heads/{name} {head}\n");
                if let Err(err) =
                    git::run_with_input(root, ["update-ref", "--stdin"], transaction.as_bytes())
                {
                    let failure = match &err {
                        git::GitError::Failed { stderr, .. } => stderr.trim().to_owned(),
                        other => other.to_string(),
                    };
                    let marked = ref_tip(root, &marker)?;
                    let tip = ref_tip(root, &format!("refs/heads/{name}"))?;
                    let not_deleted = || {
                        WorkError::refused(format!(
                            "git did not delete its work branch `{name}`, which is still at {head} ({failure})"
                        ))
                    };
                    return Err(
                        match deletion_after_failure(&head, marked.as_ref(), tip.as_ref()) {
                            DeletionAfterFailure::Ineffective => {
                                *acted = attempted_before;
                                Stop::Refused(not_deleted())
                            }
                            DeletionAfterFailure::MovedOn => {
                                *acted = attempted_before;
                                kept(format!(
                                    "it moved on from {head} while it was released ({failure})"
                                ))
                            }
                            DeletionAfterFailure::MarkerOnly => {
                                // Recorded partial before it is taken back, so a
                                // marker a failed removal leaves is not read as
                                // the deletion. A record that fails is tried again
                                // when the intent is kept.
                                intent.branch_state = BranchRelease::PartialMarker;
                                let noted = recorded(intent);
                                match git::run(
                                    root,
                                    ["update-ref", "-d", marker.as_str(), head.as_str()],
                                ) {
                                    Ok(_) => {
                                        intent.branch_state = BranchRelease::Pending;
                                        Stop::Unverified(not_deleted())
                                    }
                                    Err(err) => {
                                        let recorded = match noted {
                                            Ok(()) => String::new(),
                                            Err(cause) => format!(
                                                "; recording it as left by a failed deletion failed too ({cause})"
                                            ),
                                        };
                                        Stop::Unverified(WorkError::refused(format!(
                                            "{}; the marker {marker} it left beside the branch was not removed ({err}){recorded}. Run the withdrawal again to remove the marker and delete the branch",
                                            not_deleted()
                                        )))
                                    }
                                }
                            }
                            DeletionAfterFailure::Unsettled => {
                                Stop::Unverified(WorkError::refused(format!(
                                    "git returned a failure deleting its work branch `{name}` at {head} ({failure}); what it changed is recognized when the withdrawal is run again"
                                )))
                            }
                        },
                    );
                }
                intent.branch_state = BranchRelease::Deleted;
                recorded(intent)?;
                // Git's own branch deletion also drops the branch's
                // configuration; a branch without any has none to drop.
                let section = format!("branch.{name}");
                if let Err(err) = git::run(root, ["config", "--remove-section", section.as_str()]) {
                    tracing::debug!(event = "work_branch_config_kept", branch = name, error = %err, "no branch configuration removed");
                }
            }
            gone => {
                note_gone(intent, &gone);
                recorded(intent)?;
            }
        }
    }
    // The intent records the deletion: the marker has done its work.
    if let MarkerStep::Remove(at) = marker_step(intent.branch_state, ref_tip(root, &marker)?) {
        git::run(root, ["update-ref", "-d", marker.as_str(), at.as_str()])?;
    }
    Ok(intent.branch_head.clone().map(|head| ReleasedBranch {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn sha(c: char) -> CommitSha {
        CommitSha::parse(&c.to_string().repeat(40)).unwrap()
    }

    fn recorded(checkout: Checkout) -> IntentWorktree {
        IntentWorktree {
            path: PathBuf::from("/work/tree"),
            tie: WorktreeTie::OnBranch,
            checkout: Some(checkout),
            released: false,
            staging: None,
        }
    }

    fn listed(checkout: Checkout) -> Worktree {
        let (branch, head) = match checkout {
            Checkout::Branch(branch) => (Some(branch), Some(sha('a'))),
            Checkout::Detached(head) => (None, Some(head)),
        };
        Worktree {
            path: PathBuf::from("/work/tree"),
            head,
            branch,
            locked: None,
        }
    }

    #[test]
    fn a_workspace_gone_since_it_was_recorded_present_is_released_one_never_there_is_not() {
        let path = PathBuf::from("/project/workspace/depth");
        assert_eq!(
            workspace_step(WorkspaceRelease::Present, WorkspaceNow::Absent),
            WorkspaceStep::Gone
        );
        assert_eq!(
            workspace_step(WorkspaceRelease::Present, WorkspaceNow::Empty(path.clone())),
            WorkspaceStep::Remove(path.clone())
        );
        assert_eq!(
            workspace_step(WorkspaceRelease::Absent, WorkspaceNow::Absent),
            WorkspaceStep::Nothing
        );
        assert_eq!(
            workspace_step(WorkspaceRelease::Absent, WorkspaceNow::Empty(path)),
            WorkspaceStep::Nothing
        );
    }

    #[test]
    fn files_in_the_workspace_refuse_whatever_the_intent_recorded_of_it() {
        let path = PathBuf::from("/project/workspace/depth");
        for recorded in [
            WorkspaceRelease::Absent,
            WorkspaceRelease::Present,
            WorkspaceRelease::Released,
        ] {
            assert_eq!(
                workspace_step(recorded, WorkspaceNow::Holding(path.clone())),
                WorkspaceStep::Holding(path.clone())
            );
        }
    }

    #[test]
    fn a_failed_branch_deletion_counts_as_a_deletion_only_when_git_shows_nothing_of_it() {
        let (head, other) = (sha('a'), sha('b'));
        assert_eq!(
            deletion_after_failure(&head, None, Some(&head)),
            DeletionAfterFailure::Ineffective
        );
        assert_eq!(
            deletion_after_failure(&head, None, Some(&other)),
            DeletionAfterFailure::MovedOn
        );
        assert_eq!(
            deletion_after_failure(&head, Some(&head), Some(&head)),
            DeletionAfterFailure::MarkerOnly
        );
        for (marker, branch) in [
            (Some(&head), None),
            (None, None),
            (Some(&head), Some(&other)),
        ] {
            assert_eq!(
                deletion_after_failure(&head, marker, branch),
                DeletionAfterFailure::Unsettled
            );
        }
    }

    #[test]
    fn a_checkout_of_the_work_branch_is_the_todo_s_only_while_that_branch_is_the_recorded_one() {
        let on_work = Checkout::Branch("depth/work".to_owned());
        let step = |gone| {
            worktree_step(
                &recorded(on_work.clone()),
                Some(listed(on_work.clone())),
                Some("depth/work"),
                gone,
            )
        };
        assert!(matches!(step(false), WorktreeStep::Release(_)));
        assert!(matches!(step(true), WorktreeStep::Left));
    }

    #[test]
    fn a_worktree_whose_checkout_changed_is_left_and_one_git_no_longer_lists_is_released() {
        let on_work = Checkout::Branch("depth/work".to_owned());
        let switched = worktree_step(
            &recorded(on_work.clone()),
            Some(listed(Checkout::Branch("other".to_owned()))),
            Some("depth/work"),
            false,
        );
        assert!(matches!(switched, WorktreeStep::Left));
        let gone = worktree_step(&recorded(on_work), None, Some("depth/work"), true);
        assert!(matches!(gone, WorktreeStep::Gone));
    }

    #[test]
    fn a_detached_worktree_stays_the_todo_s_whatever_became_of_the_work_branch() {
        let detached = Checkout::Detached(sha('b'));
        let step = worktree_step(
            &recorded(detached.clone()),
            Some(listed(detached)),
            Some("depth/work"),
            true,
        );
        assert!(matches!(step, WorktreeStep::Release(_)));
    }

    #[test]
    fn the_marker_records_the_branch_deleted_at_its_own_commit_and_a_gone_branch_at_the_recorded_one()
     {
        let intent = |head| WithdrawalIntent {
            todo: TodoId::parse(&"0".repeat(32)).unwrap(),
            branch: Some("depth/work".to_owned()),
            branch_head: Some(head),
            worktrees: Vec::new(),
            workspace_path: PathBuf::from("/project/workspace/depth"),
            workspace: WorkspaceRelease::Absent,
            branch_state: BranchRelease::Pending,
            started_at: "2026-11-23T09:00:00Z".parse().unwrap(),
        };
        let mut deleted = intent(sha('b'));
        assert!(note_gone(&mut deleted, &BranchNow::Deleted(sha('a'))));
        assert_eq!(
            (deleted.branch_head, deleted.branch_state),
            (Some(sha('a')), BranchRelease::Deleted)
        );
        let mut gone = intent(sha('a'));
        assert!(note_gone(&mut gone, &BranchNow::Gone));
        assert_eq!(
            (gone.branch_head, gone.branch_state),
            (Some(sha('a')), BranchRelease::Deleted)
        );
        let mut live = intent(sha('a'));
        assert!(!note_gone(&mut live, &BranchNow::Live(sha('b'))));
        assert_eq!(
            (live.branch_head, live.branch_state),
            (Some(sha('a')), BranchRelease::Pending)
        );
    }

    #[test]
    fn a_marker_shows_a_deletion_only_while_the_branch_is_pending_and_a_partial_one_is_removed() {
        let at = sha('a');
        assert_eq!(
            marker_step(BranchRelease::Pending, Some(at.clone())),
            MarkerStep::Deletion(at.clone())
        );
        assert_eq!(
            marker_step(BranchRelease::PartialMarker, Some(at.clone())),
            MarkerStep::Remove(at.clone())
        );
        assert_eq!(
            marker_step(BranchRelease::Deleted, Some(at.clone())),
            MarkerStep::Remove(at)
        );
        for state in [
            BranchRelease::Pending,
            BranchRelease::PartialMarker,
            BranchRelease::Deleted,
        ] {
            assert_eq!(marker_step(state, None), MarkerStep::Nothing);
        }
    }

    #[test]
    fn a_worktree_is_found_at_its_staging_location_its_path_or_nowhere() {
        let staging = PathBuf::from("/work/.instrukt-withdrawal-tree");
        let mut tree = recorded(Checkout::Branch("depth/work".to_owned()));
        let at = |path: &Path| Worktree {
            path: path.to_path_buf(),
            head: Some(sha('a')),
            branch: Some("depth/work".to_owned()),
            locked: None,
        };
        let at_path = [at(&tree.path)];
        // Not moved: found at its path, or nowhere.
        assert!(matches!(
            place(&at_path, &tree, false),
            Some(Located::AtPath(_))
        ));
        assert!(matches!(place(&[], &tree, false), Some(Located::Neither)));
        // Moved: found at its staging location.
        tree.staging = Some(staging.clone());
        assert!(matches!(
            place(&[at(&staging)], &tree, true),
            Some(Located::Staged(_))
        ));
        // Listed at its path with nothing at its staging location: the move
        // did not happen. With something there, git may have moved it
        // without recording where: no answer.
        assert!(matches!(
            place(&at_path, &tree, false),
            Some(Located::AtPath(_))
        ));
        assert!(place(&at_path, &tree, true).is_none());
        // Listed nowhere: released, whatever is at its staging location.
        assert!(matches!(place(&[], &tree, true), Some(Located::Neither)));
    }
}
