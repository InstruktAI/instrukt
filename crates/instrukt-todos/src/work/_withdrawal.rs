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
//!   or holds uncommitted work — only to a path nothing occupies, and
//!   recognized by its git directory wherever git then lists it. Once it is
//!   moved, a git command addressed to its path finds no worktree there and
//!   changes nothing; a process already inside it is not stopped. It is
//!   released once git lists it at neither location;
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
//!   marker and the branch still at the commit means nothing changed; for a
//!   marker beside the untouched branch, recording it as a partial marker
//!   ([`BranchRelease::PartialMarker`]) is attempted before it is taken
//!   back, so a marker a failed removal leaves is not read as the deletion,
//!   and it is removed, with the branch recorded pending again, before the
//!   branch is deleted; anything else is left for the withdrawal run again
//!   to recognize. Any other marker is removed only once the intent records
//!   the deletion.
//!
//! While the intent records a worktree at its staging location or a
//! partial marker, it is never abandoned, whatever refuses the run.
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
        (Some(parent), Some(_)) => Ok(parent.join(staging_name(path, todo))),
        _ => Err(Stop::Unverified(WorkError::refused(format!(
            "the worktree {} has no directory beside it to be released from",
            path.display()
        )))),
    }
}

/// The name of the staging location of the worktree at `path`: the todo's
/// identity and a digest of the worktree's whole path, so it has the same
/// short length whatever the worktree is called — well within the 255
/// bytes a file name may have — and differs for each worktree of the todo
/// in one directory.
fn staging_name(path: &Path, todo: &TodoId) -> String {
    use sha2::Digest as _;
    let digest = sha2::Sha256::digest(path.as_os_str().as_encoded_bytes());
    format!(
        ".instrukt-wd-{}-{}",
        todo.as_str(),
        hex::encode(&digest[..8])
    )
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
///   the todo as it was when the intent is abandonable — it records nothing
///   released, no worktree at its staging location and no partial marker
///   ([`WithdrawalIntent::abandonable`]) — and no effect was attempted, or
///   each one attempted is proven to have changed nothing: a branch update
///   the refs read back show did nothing, a worktree move git's list shows
///   did not happen, a worktree git's list shows moved back to its path.
///   Otherwise it keeps the intent;
/// - an unverified observation or effect ([`Stop::Unverified`]) keeps the
///   intent, even when it records nothing released, because what git or the
///   filesystem holds was not established;
/// - a branch update that left only its marker keeps the intent, whether
///   or not the marker is taken back ([`delete_branch`]).
///
/// A kept intent keeps everything it records, the branch's commit, any
/// worktree's staging location and a partial marker included, and the
/// refusal names what is recorded released, what is recorded under way
/// ([`underway`]), and, for an unverified stop, that the rest could not be
/// established.
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
                let underway = match underway(intent) {
                    open if open.is_empty() => String::new(),
                    open => format!("; still recorded under way: {}", open.join(", ")),
                };
                match stop {
                    Stop::Refused(cause) => {
                        let released = if released.is_empty() {
                            "nothing is released yet".to_owned()
                        } else {
                            format!("already released: {}", released.join(", "))
                        };
                        WorkError::refused(format!(
                            "`{slug}` is not withdrawn yet: {cause} ({released}{underway}); make its work safe, then run `instrukt todo remove {slug}` again to complete the withdrawal"
                        ))
                    }
                    Stop::Unverified(cause) => {
                        let recorded = if released.is_empty() {
                            format!("it records nothing released{underway}")
                        } else {
                            format!("it records as released {}{underway}", released.join(", "))
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
    // nothing released and nothing under way, leaves the todo as it was.
    let stopped = |intent: &WithdrawalIntent, acted: bool, stop: Stop| -> WorkError {
        if let Stop::Refused(_) = &stop
            && !acted
            && intent.abandonable()
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
    /// abandonment. The refusal names `staging` as where the intent records
    /// it; `cause` says what git and the filesystem show of it.
    fn moving(path: &Path, staging: &Path, stop: Stop) -> Self {
        let stop = match stop {
            Stop::Refused(cause) | Stop::Unverified(cause) => {
                Stop::Unverified(WorkError::refused(format!(
                    "the withdrawal records its worktree {} at {}, where it moves it to be released: {cause}",
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
/// Moving it is what keeps a command addressed to its path from changing
/// its checkout unseen between the last look and the removal: once moved, a
/// git command addressed to its path finds no worktree there. A process
/// whose working directory was already inside it keeps reaching it at the
/// staging location (`state-database`, "Known caveats"). Whatever the move
/// returns, git's worktree list is read back: listed at its path with
/// nothing at its staging location, the move did not happen and the
/// refusal is proven to have changed nothing; listed at its staging
/// location, it is finished there; anything else keeps it recorded at its
/// staging location, unverified.
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
/// is no longer the todo's, it is left out of the withdrawal. One not moved
/// back — its path taken, or git not moving it — stays recorded where git
/// lists it, named, for the withdrawal run again ([`move_back`]). A removal
/// that returns a failure keeps the intent, and the refusal says what git
/// and the filesystem show of it afterwards ([`after_failed_removal`]).
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
        let failure = match err {
            git::GitError::Failed { stderr, .. } => stderr.trim().to_owned(),
            other => other.to_string(),
        };
        let cause = WorkError::refused(format!(
            "git returned a failure removing it ({failure}); {}",
            after_failed_removal(root, &staging)
        ));
        return Err(moving(Stop::Unverified(cause)));
    }
    released(intent, at, true, recorded)
}

/// What git and the filesystem show of a worktree at `staging` after its
/// `git worktree remove` returned a failure, which may come after git
/// removed the checkout, while it cleaned up its own records: only what
/// the look establishes.
fn after_failed_removal(root: &Path, staging: &Path) -> String {
    let listed = worktrees(root).map(|listed| listed.iter().any(|w| w.path == staging));
    let present = match std::fs::symlink_metadata(staging) {
        Ok(_) => Ok(true),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(err),
    };
    let at = staging.display();
    match (listed, present) {
        (Ok(true), _) => format!("git still lists it at {at}"),
        (Ok(false), Ok(true)) => format!("git no longer lists it, and {at} is still there"),
        (Ok(false), Ok(false)) => format!("git no longer lists it, and {at} is gone"),
        (Err(err), _) => format!("where it is now could not be established ({err})"),
        (_, Err(err)) => {
            format!(
                "git no longer lists it; whether {at} is still there could not be established ({err})"
            )
        }
    }
}

/// The git directory of the worktree at `path`, which `git worktree move`
/// keeps: what tells a worktree from another one at the same path.
fn git_dir(path: &Path) -> Result<PathBuf, git::GitError> {
    Ok(PathBuf::from(git::run(
        path,
        ["rev-parse", "--absolute-git-dir"],
    )?))
}

/// Where a moved-back worktree is, from `seen`: the places it may be that
/// git lists, each with its git directory.
#[derive(Debug, PartialEq, Eq)]
enum Back {
    /// At its path, `path`: moved back.
    Restored,
    /// Somewhere else git lists it: where the move left it, or put it.
    Elsewhere(PathBuf),
    /// Not among the places looked at.
    Unknown,
}

/// The worktree whose git directory is `ours` is back at `path` only when
/// git lists it there; another checkout at `path` is not it.
fn back_step(path: &Path, ours: &Path, seen: &[(PathBuf, PathBuf)]) -> Back {
    match seen.iter().find(|(_, dir)| dir == ours) {
        Some((at, _)) if at == path => Back::Restored,
        Some((at, _)) => Back::Elsewhere(at.clone()),
        None => Back::Unknown,
    }
}

/// Moves the worktree `intent` records at `at` back from `staging` to its
/// path. Anything at its path now — another checkout git has put there, or
/// any file or directory — keeps it at `staging`, unmoved: git would move
/// it inside that directory. After the move, git's list is read back and
/// the worktree recognized by its git directory ([`back_step`]): back at
/// its path, its staging location is cleared; anywhere else git lists it,
/// that place is recorded as its staging location and named; not found,
/// its staging location stays recorded and the refusal says where it is
/// could not be established.
fn move_back(
    root: &Path,
    intent: &mut WithdrawalIntent,
    at: usize,
    staging: &Path,
    recorded: &dyn Fn(&WithdrawalIntent) -> Result<(), WorkError>,
) -> Result<(), Attempt> {
    let path = intent.worktrees[at].path.clone();
    let unmoved =
        |why: String| Attempt::moving(&path, staging, Stop::Unverified(WorkError::refused(why)));
    let ours = git_dir(staging).map_err(|err| {
        unmoved(format!(
            "it is no longer releasable and its git directory could not be read ({err}); it is not moved back"
        ))
    })?;
    let listed = worktrees(root).map_err(|err| {
        unmoved(format!(
            "it is no longer releasable and git's worktrees could not be read ({err}); it is not moved back"
        ))
    })?;
    let occupied = listed.iter().any(|w| w.path == path)
        || match std::fs::symlink_metadata(&path) {
            Ok(_) => true,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => false,
            Err(err) => {
                return Err(unmoved(format!(
                    "it is no longer releasable and whether {} is free could not be established ({err}); it is not moved back",
                    path.display()
                )));
            }
        };
    if occupied {
        return Err(unmoved(format!(
            "it is no longer releasable, and {} is taken now; it is not moved back and stays at {}. Free {}, then withdraw again to move it back",
            path.display(),
            staging.display(),
            path.display()
        )));
    }
    let (from, to) = (
        staging.to_string_lossy().into_owned(),
        path.to_string_lossy().into_owned(),
    );
    let moved = git::run(root, ["worktree", "move", from.as_str(), to.as_str()]);
    let failure = match &moved {
        Ok(_) => String::new(),
        Err(git::GitError::Failed { stderr, .. }) => format!(" ({})", stderr.trim()),
        Err(other) => format!(" ({other})"),
    };
    // Read back where git has it, whatever the move returned: a directory
    // that appeared at its path meanwhile has git move it inside.
    let nested = staging.file_name().map(|name| path.join(name));
    let seen = worktrees(root).map(|listed| {
        listed
            .iter()
            .filter(|w| w.path == path || w.path == staging || Some(&w.path) == nested.as_ref())
            .filter_map(|w| git_dir(&w.path).ok().map(|dir| (w.path.clone(), dir)))
            .collect::<Vec<_>>()
    });
    let kept_at = |at: &Path, why: String| {
        Attempt::moving(&path, at, Stop::Unverified(WorkError::refused(why)))
    };
    match seen.map(|seen| back_step(&path, &ours, &seen)) {
        Ok(Back::Restored) => {
            intent.worktrees[at].staging = None;
            recorded(intent).map_err(|cause| Attempt {
                stop: Box::new(Stop::Unverified(cause)),
                removing: true,
            })
        }
        Ok(Back::Elsewhere(found)) => {
            intent.worktrees[at].staging = Some(found.clone());
            recorded(intent).map_err(|cause| Attempt {
                stop: Box::new(Stop::Unverified(cause)),
                removing: true,
            })?;
            let why = if found == staging {
                format!(
                    "it is no longer releasable and git did not move it back{failure}; it is still at {}. Running the withdrawal again moves it back or releases it",
                    found.display()
                )
            } else {
                format!(
                    "it is no longer releasable, and git moved it to {} instead of {}{failure}, which was taken meanwhile. Free {}, then withdraw again to move it back",
                    found.display(),
                    path.display(),
                    path.display()
                )
            };
            Err(kept_at(&found, why))
        }
        Ok(Back::Unknown) => Err(kept_at(
            staging,
            format!(
                "it is no longer releasable, and after moving it back{failure} git lists it neither at {} nor at {}; where it is could not be established",
                path.display(),
                staging.display()
            ),
        )),
        Err(err) => Err(kept_at(
            staging,
            format!(
                "it is no longer releasable, and after moving it back{failure} git's worktrees could not be read ({err}); where it is could not be established"
            ),
        )),
    }
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

/// What `intent` records under way, which keeps it open whatever else is
/// released ([`WithdrawalIntent::abandonable`]), for a refusal to name:
/// each worktree recorded at its staging location, and a partial marker not
/// yet taken back.
fn underway(intent: &WithdrawalIntent) -> Vec<String> {
    let mut open: Vec<String> = intent
        .worktrees
        .iter()
        .filter(|w| !w.released)
        .filter_map(|w| {
            w.staging.as_ref().map(|staging| {
                format!(
                    "its worktree {} at {}, where it is moved to be released",
                    w.path.display(),
                    staging.display()
                )
            })
        })
        .collect();
    if intent.branch_state == BranchRelease::PartialMarker {
        open.push(format!(
            "the marker {} a failed deletion of its work branch left, removed before the branch is deleted",
            marker(&intent.todo)
        ));
    }
    open
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
/// nothing, the attempt does not count as one. For a marker it left beside
/// the untouched branch, recording it partial is attempted first; it is then
/// removed, and recorded pending again once it is gone. The intent is kept
/// either way, as it is for anything else, for the withdrawal run again.
/// Returns the branch and its last commit, `None` when there is none to
/// record.
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
                                // Recording the marker partial is attempted
                                // before it is taken back, so a marker a failed
                                // removal leaves is not read as the deletion. The
                                // marker is taken back whether or not that write
                                // succeeded: once it is gone there is nothing to
                                // misread. The intent kept with this stop writes
                                // the outcome again; only a marker left with both
                                // writes failed is read as the deletion
                                // (`state-database`, "Known caveats").
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

/// The per-computer withdrawal lock `state/withdrawal.lock`, held by a
/// withdrawal command from before it records or resumes its intent until it
/// closes it or returns with the intent kept, so a withdrawal in progress is
/// told apart from a recorded intent no command is acting on: only its
/// holder releases resources or closes an intent. Taken before the writer lock and never while it is held;
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

    fn todo() -> TodoId {
        TodoId::parse(&"0".repeat(32)).unwrap()
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

    fn listed_at(path: &Path) -> Worktree {
        Worktree {
            path: path.to_path_buf(),
            ..listed(Checkout::Branch("depth/work".to_owned()))
        }
    }

    fn intent(head: CommitSha) -> WithdrawalIntent {
        WithdrawalIntent {
            todo: todo(),
            branch: Some("depth/work".to_owned()),
            branch_head: Some(head),
            worktrees: Vec::new(),
            workspace_path: PathBuf::from("/project/workspace/depth"),
            workspace: WorkspaceRelease::Absent,
            branch_state: BranchRelease::Pending,
            started_at: "2026-11-23T09:00:00Z".parse().unwrap(),
        }
    }

    fn staged() -> IntentWorktree {
        IntentWorktree {
            staging: Some(PathBuf::from("/work/.instrukt-wd-tree")),
            ..recorded(Checkout::Branch("depth/work".to_owned()))
        }
    }

    #[test]
    fn a_workspace_recorded_present_and_gone_is_released() {
        assert_eq!(
            workspace_step(WorkspaceRelease::Present, WorkspaceNow::Absent),
            WorkspaceStep::Gone
        );
    }

    #[test]
    fn an_empty_workspace_recorded_present_is_removed() {
        let path = PathBuf::from("/project/workspace/depth");
        assert_eq!(
            workspace_step(WorkspaceRelease::Present, WorkspaceNow::Empty(path.clone())),
            WorkspaceStep::Remove(path)
        );
    }

    #[test]
    fn a_workspace_recorded_absent_is_never_taken_for_released() {
        let path = PathBuf::from("/project/workspace/depth");
        assert_eq!(
            workspace_step(WorkspaceRelease::Absent, WorkspaceNow::Empty(path)),
            WorkspaceStep::Nothing
        );
    }

    #[test]
    fn files_in_the_workspace_refuse_whatever_the_intent_recorded_of_it() {
        let path = PathBuf::from("/project/workspace/depth");
        assert_eq!(
            workspace_step(
                WorkspaceRelease::Released,
                WorkspaceNow::Holding(path.clone())
            ),
            WorkspaceStep::Holding(path)
        );
    }

    #[test]
    fn a_failed_branch_deletion_that_left_the_branch_and_no_marker_changed_nothing() {
        let head = sha('a');
        assert_eq!(
            deletion_after_failure(&head, None, Some(&head)),
            DeletionAfterFailure::Ineffective
        );
    }

    #[test]
    fn a_failed_branch_deletion_beside_a_branch_that_moved_on_changed_nothing() {
        let (head, other) = (sha('a'), sha('b'));
        assert_eq!(
            deletion_after_failure(&head, None, Some(&other)),
            DeletionAfterFailure::MovedOn
        );
    }

    #[test]
    fn a_failed_branch_deletion_that_left_only_its_marker_is_marker_only() {
        let head = sha('a');
        assert_eq!(
            deletion_after_failure(&head, Some(&head), Some(&head)),
            DeletionAfterFailure::MarkerOnly
        );
    }

    #[test]
    fn a_failed_branch_deletion_with_the_branch_gone_is_unsettled() {
        let head = sha('a');
        assert_eq!(
            deletion_after_failure(&head, Some(&head), None),
            DeletionAfterFailure::Unsettled
        );
    }

    #[test]
    fn a_checkout_of_the_recorded_work_branch_is_released() {
        let on_work = Checkout::Branch("depth/work".to_owned());
        let step = worktree_step(
            &recorded(on_work.clone()),
            Some(listed(on_work)),
            Some("depth/work"),
            false,
        );
        assert!(matches!(step, WorktreeStep::Release(_)));
    }

    #[test]
    fn a_checkout_of_a_work_branch_recorded_deleted_is_left() {
        let on_work = Checkout::Branch("depth/work".to_owned());
        let step = worktree_step(
            &recorded(on_work.clone()),
            Some(listed(on_work)),
            Some("depth/work"),
            true,
        );
        assert!(matches!(step, WorktreeStep::Left));
    }

    #[test]
    fn a_worktree_whose_checkout_changed_is_left() {
        let step = worktree_step(
            &recorded(Checkout::Branch("depth/work".to_owned())),
            Some(listed(Checkout::Branch("other".to_owned()))),
            Some("depth/work"),
            false,
        );
        assert!(matches!(step, WorktreeStep::Left));
    }

    #[test]
    fn a_worktree_git_no_longer_lists_is_released() {
        let step = worktree_step(
            &recorded(Checkout::Branch("depth/work".to_owned())),
            None,
            Some("depth/work"),
            false,
        );
        assert!(matches!(step, WorktreeStep::Gone));
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
    fn a_marker_records_the_branch_deleted_at_the_marker_s_commit() {
        let mut deleted = intent(sha('b'));
        assert!(note_gone(&mut deleted, &BranchNow::Deleted(sha('a'))));
        assert_eq!(
            (deleted.branch_head, deleted.branch_state),
            (Some(sha('a')), BranchRelease::Deleted)
        );
    }

    #[test]
    fn a_gone_branch_is_recorded_deleted_at_the_recorded_commit() {
        let mut gone = intent(sha('a'));
        assert!(note_gone(&mut gone, &BranchNow::Gone));
        assert_eq!(
            (gone.branch_head, gone.branch_state),
            (Some(sha('a')), BranchRelease::Deleted)
        );
    }

    #[test]
    fn a_live_branch_is_left_pending() {
        let mut live = intent(sha('a'));
        assert!(!note_gone(&mut live, &BranchNow::Live(sha('b'))));
        assert_eq!(
            (live.branch_head, live.branch_state),
            (Some(sha('a')), BranchRelease::Pending)
        );
    }

    #[test]
    fn a_marker_found_while_the_branch_is_pending_shows_its_deletion() {
        assert_eq!(
            marker_step(BranchRelease::Pending, Some(sha('a'))),
            MarkerStep::Deletion(sha('a'))
        );
    }

    #[test]
    fn a_partial_marker_is_removed_and_never_read_as_the_deletion() {
        assert_eq!(
            marker_step(BranchRelease::PartialMarker, Some(sha('a'))),
            MarkerStep::Remove(sha('a'))
        );
    }

    #[test]
    fn a_marker_whose_deletion_is_recorded_is_removed() {
        assert_eq!(
            marker_step(BranchRelease::Deleted, Some(sha('a'))),
            MarkerStep::Remove(sha('a'))
        );
    }

    #[test]
    fn no_marker_shows_nothing() {
        assert_eq!(
            marker_step(BranchRelease::Pending, None),
            MarkerStep::Nothing
        );
    }

    #[test]
    fn a_worktree_not_moved_is_found_at_its_path() {
        let tree = recorded(Checkout::Branch("depth/work".to_owned()));
        assert!(matches!(
            place(&[listed_at(&tree.path)], &tree, false),
            Some(Located::AtPath(_))
        ));
    }

    #[test]
    fn a_moved_worktree_is_found_at_its_staging_location() {
        let tree = staged();
        let staging = tree.staging.clone().unwrap();
        assert!(matches!(
            place(&[listed_at(&staging)], &tree, true),
            Some(Located::Staged(_))
        ));
    }

    #[test]
    fn a_worktree_listed_at_its_path_with_nothing_at_its_staging_location_was_not_moved() {
        let tree = staged();
        assert!(matches!(
            place(&[listed_at(&tree.path)], &tree, false),
            Some(Located::AtPath(_))
        ));
    }

    #[test]
    fn a_worktree_listed_at_its_path_while_its_staging_location_exists_is_no_answer() {
        let tree = staged();
        assert!(place(&[listed_at(&tree.path)], &tree, true).is_none());
    }

    #[test]
    fn a_worktree_git_lists_nowhere_is_found_nowhere() {
        assert!(matches!(
            place(&[], &staged(), true),
            Some(Located::Neither)
        ));
    }

    #[test]
    fn an_intent_with_nothing_released_or_under_way_is_abandonable() {
        assert!(intent(sha('a')).abandonable());
    }

    #[test]
    fn an_intent_recording_a_partial_marker_is_never_abandoned() {
        let partial = WithdrawalIntent {
            branch_state: BranchRelease::PartialMarker,
            ..intent(sha('a'))
        };
        assert!(!partial.abandonable());
    }

    #[test]
    fn an_intent_recording_a_worktree_at_its_staging_location_is_never_abandoned() {
        let moving = WithdrawalIntent {
            worktrees: vec![staged()],
            ..intent(sha('a'))
        };
        assert!(!moving.abandonable());
    }

    #[test]
    fn the_staging_name_of_a_worktree_named_at_the_file_name_limit_fits_in_a_file_name() {
        let path = PathBuf::from("/work").join("w".repeat(240));
        assert!(staging_name(&path, &todo()).len() <= 255);
    }

    #[test]
    fn worktrees_of_one_todo_in_one_directory_have_distinct_staging_names() {
        let (one, two) = (PathBuf::from("/work/one"), PathBuf::from("/work/two"));
        assert_ne!(staging_name(&one, &todo()), staging_name(&two, &todo()));
    }

    #[test]
    fn a_worktree_git_lists_at_its_path_by_its_own_git_directory_is_moved_back() {
        let (path, ours) = (
            PathBuf::from("/work/tree"),
            PathBuf::from("/repo/.git/worktrees/tree"),
        );
        assert_eq!(
            back_step(&path, &ours, &[(path.clone(), ours.clone())]),
            Back::Restored
        );
    }

    #[test]
    fn a_worktree_put_inside_another_checkout_at_its_path_is_found_there_not_taken_for_moved_back()
    {
        let path = PathBuf::from("/work/tree");
        let nested = path.join(".instrukt-wd-tree");
        let ours = PathBuf::from("/repo/.git/worktrees/tree");
        let seen = [
            (path.clone(), PathBuf::from("/repo/.git/worktrees/tree1")),
            (nested.clone(), ours.clone()),
        ];
        assert_eq!(back_step(&path, &ours, &seen), Back::Elsewhere(nested));
    }

    #[test]
    fn a_worktree_git_lists_at_none_of_the_places_looked_at_is_unknown() {
        let path = PathBuf::from("/work/tree");
        let seen = [(path.clone(), PathBuf::from("/repo/.git/worktrees/tree1"))];
        assert_eq!(
            back_step(&path, Path::new("/repo/.git/worktrees/tree"), &seen),
            Back::Unknown
        );
    }
}
