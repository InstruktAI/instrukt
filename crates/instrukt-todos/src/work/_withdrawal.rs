//! Releasing a withdrawn todo's working resources — its worktrees, empty
//! workspace and work branch — outside the writer lock, under the
//! withdrawal lock and against its recorded intent (`todos`, "Parking,
//! withdrawal and delivery"; `state-database`, `withdrawal_intents`).
//!
//! Each release is an external effect a stopped process may complete
//! without recording it. The intent and git together tell, before and
//! after each effect, whether it has happened:
//!
//! - a worktree the intent records is released once git no longer lists
//!   it. It is looked at and removed only while the lock git takes to
//!   change its `HEAD` is held ([`HeadLock`]), so what it has checked out
//!   cannot change between the last look and the removal;
//! - the workspace the intent records, at the location it was admitted
//!   at, is released once it is gone when the intent recorded it present —
//!   an absent one is never taken for released, and one holding files is
//!   never released;
//! - the work branch is deleted in one `git update-ref --stdin` call that
//!   creates the withdrawal's marker ref (`refs/instrukt/withdrawals/<id>`)
//!   at the commit deleted against, then compare-and-deletes the branch.
//!   The marker is removed only after the intent records the deletion, so a
//!   marker found with the branch gone tells this withdrawal's deletion, and
//!   the commit it deleted, from a branch someone else deleted, moved on or
//!   recreated under the same name. A call that returns a failure is read
//!   back: the branch still there at the commit means nothing was deleted;
//!   anything else is left for the withdrawal run again to recognize.
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
    Checkout, IntentWorktree, Store, WithdrawalIntent, WorkspaceRelease, WorktreeTie,
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
    /// nothing. A withdrawal that has released nothing may be abandoned on
    /// it.
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
/// uncommitted work, naming where it is (`UC-TD5`). A status git could not
/// read is no answer.
fn check_worktree(worktree: &Worktree) -> Result<(), Stop> {
    if let Some(reason) = &worktree.locked {
        let why = if reason.is_empty() {
            String::new()
        } else {
            format!(" ({reason})")
        };
        return Err(Stop::refused(format!(
            "the worktree {} is locked{why}; unlock it with `git worktree unlock`, then withdraw again",
            worktree.path.display()
        )));
    }
    let changes = git::run(&worktree.path, ["status", "--porcelain"])?;
    if !changes.trim().is_empty() {
        return Err(Stop::refused(format!(
            "the worktree {} holds uncommitted changes; commit or discard them, then withdraw again",
            worktree.path.display()
        )));
    }
    Ok(())
}

/// The lock git takes to change a linked worktree's `HEAD`, created
/// exclusively as git creates it: while it is held, no git command can
/// change what the worktree has checked out (`git switch` and
/// `git checkout` refuse on it). With the `files` ref storage that is the
/// worktree's `HEAD.lock`; with `reftable` it is the lock of the worktree's
/// own reference table list. Removed when dropped, unless the worktree's
/// removal took it with the worktree's administrative directory.
struct HeadLock {
    path: Option<PathBuf>,
}

impl HeadLock {
    fn take(worktree: &Path) -> Result<Self, Stop> {
        let admin = PathBuf::from(git::run(worktree, ["rev-parse", "--absolute-git-dir"])?);
        let storage = git::run_output(worktree, ["config", "--get", "extensions.refStorage"])?;
        let path = match (
            storage.status.code(),
            String::from_utf8_lossy(&storage.stdout).trim(),
        ) {
            (Some(1), _) | (Some(0), "files") => admin.join("HEAD.lock"),
            (Some(0), "reftable") => admin.join("reftable").join("tables.list.lock"),
            (Some(0), other) => {
                return Err(Stop::Unverified(WorkError::refused(format!(
                    "the worktree {}'s checkout cannot be held while it is released: its repository keeps references in `{other}`",
                    worktree.display()
                ))));
            }
            _ => {
                return Err(Stop::Unverified(WorkError::refused(format!(
                    "git could not tell how the worktree {} keeps its references ({})",
                    worktree.display(),
                    String::from_utf8_lossy(&storage.stderr).trim()
                ))));
            }
        };
        let mut exclusive = std::fs::OpenOptions::new();
        exclusive.write(true).create_new(true);
        match instrukt_core::fsx::open(&exclusive, &path) {
            Ok(_) => Ok(Self { path: Some(path) }),
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                Err(Stop::Unverified(WorkError::refused(format!(
                    "another git command is changing what the worktree {} has checked out ({} exists)",
                    worktree.display(),
                    path.display()
                ))))
            }
            Err(source) => Err(Stop::Unverified(WorkError::Io { path, source })),
        }
    }

    /// The worktree is removed: its administrative directory, and the lock
    /// in it, went with it. Nothing is left to remove.
    fn removed(mut self) {
        self.path = None;
    }
}

impl Drop for HeadLock {
    fn drop(&mut self) {
        let Some(path) = self.path.take() else {
            return;
        };
        match instrukt_core::fsx::remove_file(&path) {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => tracing::warn!(
                event = "work_head_lock_kept",
                path = %path.display(),
                error = %err,
                "a worktree's HEAD lock taken by a withdrawal was not removed; git refuses to change that worktree's checkout until it is"
            ),
        }
    }
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
    if entries.next().is_some() {
        return Ok(WorkspaceNow::Holding(workspace.to_path_buf()));
    }
    Ok(WorkspaceNow::Empty(workspace.to_path_buf()))
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
/// deletion that outlives a lost receipt. Removed once the intent records
/// the deletion.
fn marker(todo: &TodoId) -> String {
    format!("refs/instrukt/withdrawals/{}", todo.as_str())
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

fn branch_now(
    root: &Path,
    todo: &TodoId,
    name: &str,
    recorded: Option<&CommitSha>,
) -> Result<BranchNow, WorkError> {
    if let Some(at) = ref_tip(root, &marker(todo))? {
        return Ok(BranchNow::Deleted(at));
    }
    let Some(tip) = ref_tip(root, &format!("refs/heads/{name}"))? else {
        return Ok(BranchNow::Gone);
    };
    match recorded {
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
            intent.branch_deleted = true;
            true
        }
        BranchNow::Gone => {
            intent.branch_deleted = true;
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
        check_worktree(worktree).map_err(Stop::cause)?;
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
///   ([`worktree_step`]), checked and removed under its [`HeadLock`]; one
///   that is not is left out;
/// - the work branch, still the recorded one, is recorded at the commit it
///   points at when it moved on to a descendant, then deleted against that
///   commit together with its marker; gone or replaced, it is recorded
///   deleted at the recorded commit, and anything carrying its name is left
///   as it is.
///
/// A stop the run can prove ([`Stop::Refused`]), while no effect was
/// attempted or each one attempted is proven to have changed nothing, and
/// the intent records nothing released, closes the intent and leaves the
/// todo as it was. Any other stop keeps the intent with everything it
/// records, the branch's commit included, and the refusal names what is
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
    // A stop the run can prove, before any effect was attempted, while the
    // intent records nothing released, leaves the todo as it was.
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
                // A removal git returned a failure for may have removed
                // part of the worktree: never an abandonment.
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

impl<E: Into<Stop>> From<E> for Attempt {
    fn from(err: E) -> Self {
        Self {
            stop: Box::new(err.into()),
            removing: false,
        }
    }
}

/// Releases the worktree `intent` records at `at`, recording the outcome.
/// It is looked at, checked and removed under its [`HeadLock`], so the
/// checkout found still the todo's is the checkout removed: a switch that
/// comes after the look fails on the lock and changes nothing.
fn release_worktree(
    root: &Path,
    slug: &TodoSlug,
    intent: &mut WithdrawalIntent,
    at: usize,
    recorded: &dyn Fn(&WithdrawalIntent) -> Result<(), WorkError>,
) -> Result<Advance, Attempt> {
    let path = intent.worktrees[at].path.clone();
    let listed = worktrees(root)?;
    let lock = match listed.iter().find(|w| w.path == path) {
        Some(_) => Some(HeadLock::take(&path)?),
        None => None,
    };
    // Read again now that its checkout cannot change: the worktree, and
    // the branch it has checked out when that is the work branch.
    let now = match lock {
        Some(_) => worktrees(root)?.into_iter().find(|w| w.path == path),
        None => None,
    };
    if now.is_some()
        && !intent.branch_deleted
        && on_branch(&intent.worktrees[at], intent.branch.as_deref())
        && let Some(name) = intent.branch.clone()
    {
        let seen = branch_now(root, &intent.todo, &name, intent.branch_head.as_ref())?;
        if note_gone(intent, &seen) {
            recorded(intent)?;
        }
    }
    match worktree_step(
        &intent.worktrees[at],
        now,
        intent.branch.as_deref(),
        intent.branch_deleted,
    ) {
        WorktreeStep::Gone => {
            intent.worktrees[at].released = true;
            recorded(intent)?;
            Ok(Advance::Released { removed: false })
        }
        WorktreeStep::Left => {
            tracing::info!(
                event = "work_withdrawal_worktree_left",
                todo = %slug,
                worktree = %path.display(),
                "a worktree is no longer the todo's; it is left where it is"
            );
            intent.worktrees.remove(at);
            recorded(intent)?;
            Ok(Advance::Left)
        }
        WorktreeStep::Release(worktree) => {
            check_worktree(&worktree)?;
            let target = path.to_string_lossy().into_owned();
            if let Err(err) = git::run(root, ["worktree", "remove", target.as_str()]) {
                let cause = match err {
                    git::GitError::Failed { stderr, .. } => WorkError::refused(format!(
                        "git kept its worktree {} ({})",
                        path.display(),
                        stderr.trim()
                    )),
                    other => other.into(),
                };
                return Err(Attempt {
                    stop: Box::new(Stop::Unverified(cause)),
                    removing: true,
                });
            }
            if let Some(lock) = lock {
                lock.removed();
            }
            intent.worktrees[at].released = true;
            recorded(intent).map_err(|cause| Attempt {
                stop: Box::new(Stop::Unverified(cause)),
                removing: true,
            })?;
            Ok(Advance::Released { removed: true })
        }
    }
}

/// Brings `intent`, left by an earlier run, up to date with what that run
/// did and could not record, and with what happened to its resources
/// since: the branch deleted by it (its marker) or gone, each worktree git
/// no longer lists, and the workspace it recorded present, at the location
/// it recorded, that is gone.
fn recognize(root: &Path, intent: &mut WithdrawalIntent) -> Result<(), WorkError> {
    if let (Some(name), false) = (intent.branch.clone(), intent.branch_deleted) {
        let now = branch_now(root, &intent.todo, &name, intent.branch_head.as_ref())?;
        note_gone(intent, &now);
    }
    if intent.worktrees.iter().any(|w| !w.released) {
        let listed = worktrees(root)?;
        for worktree in intent.worktrees.iter_mut().filter(|w| !w.released) {
            if !listed.iter().any(|w| w.path == worktree.path) {
                worktree.released = true;
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
    if let (true, Some(name)) = (intent.branch_deleted, &intent.branch) {
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
    /// the marker took effect, and it is taken back.
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
/// A call that returns a failure is never taken for a deletion: the refs
/// are read back ([`deletion_after_failure`]). Proven to have changed
/// nothing, the attempt does not count as one. A marker it left beside the
/// untouched branch is removed again, and the intent is kept, as it is for
/// anything else, with the marker, for the withdrawal run again.
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
    if !intent.branch_deleted {
        let kept = |why: String| Stop::refused(format!("its work branch `{name}` is kept: {why}"));
        match branch_now(root, &intent.todo, name, intent.branch_head.as_ref())? {
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
                                git::run(
                                    root,
                                    ["update-ref", "-d", marker.as_str(), head.as_str()],
                                )?;
                                Stop::Unverified(not_deleted())
                            }
                            DeletionAfterFailure::Unsettled => {
                                Stop::Unverified(WorkError::refused(format!(
                                    "git returned a failure deleting its work branch `{name}` at {head} ({failure}); what it changed is recognized when the withdrawal is run again"
                                )))
                            }
                        },
                    );
                }
                intent.branch_deleted = true;
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
    if let Some(at) = ref_tip(root, &marker)? {
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
            branch_deleted: false,
            started_at: "2026-11-23T09:00:00Z".parse().unwrap(),
        };
        let mut deleted = intent(sha('b'));
        assert!(note_gone(&mut deleted, &BranchNow::Deleted(sha('a'))));
        assert_eq!(
            (deleted.branch_head, deleted.branch_deleted),
            (Some(sha('a')), true)
        );
        let mut gone = intent(sha('a'));
        assert!(note_gone(&mut gone, &BranchNow::Gone));
        assert_eq!(
            (gone.branch_head, gone.branch_deleted),
            (Some(sha('a')), true)
        );
        let mut live = intent(sha('a'));
        assert!(!note_gone(&mut live, &BranchNow::Live(sha('b'))));
        assert_eq!(
            (live.branch_head, live.branch_deleted),
            (Some(sha('a')), false)
        );
    }
}
