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
//!   commands. The intent records its git directory, which a move keeps,
//!   and its stage ([`WorktreeStage`]): how far this withdrawal's own moves
//!   of it have come, each move recorded before git makes it. It is
//!   recognized by its git directory ([`locate`]) and acted on only at its
//!   path or where its stage says those moves can have put it
//!   ([`acts_at`]): until it is staged, a worktree git lists elsewhere
//!   with its git directory was moved by someone else and is left there,
//!   and, once it is staged, a git directory git lists anywhere but its
//!   path or its staged location may be one git gave a new checkout after
//!   this one was removed, so it is never looked at. Found back at its
//!   path, it is checked there as one never moved. Checked at its path, it
//!   is moved with `git worktree move`
//!   to a staging location beside it; found inside a directory that took
//!   that location meanwhile, it is staged there instead. There it is
//!   looked at once more and removed with `git worktree remove`, or moved
//!   back when it is no longer the todo's or holds uncommitted work — only
//!   to a path nothing occupies. While it is away from its path, a git
//!   command addressed to that path does not reach it; a process already
//!   inside it is not stopped. Until it is staged, it is released once git
//!   lists it nowhere and no longer keeps its git directory; once staged,
//!   once git lists it neither at its staged location nor at its path and
//!   nothing is at its staged location;
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
//! While the intent records a worktree the withdrawal is moving, has moved
//! or is moving back, or a partial marker, it is never abandoned, whatever
//! refuses the run.
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
    BranchRelease, Checkout, IntentWorktree, Store, WithdrawalIntent, WorkspaceRelease,
    WorktreeStage, WorktreeTie,
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
    /// Whether git reports it prunable: the directory git records for it is
    /// gone.
    prunable: bool,
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
    Ok(parse_worktrees(&git::run(
        root,
        ["worktree", "list", "--porcelain"],
    )?))
}

/// The worktrees `git worktree list --porcelain` printed as `listed`.
fn parse_worktrees(listed: &str) -> Vec<Worktree> {
    let mut found: Vec<Worktree> = Vec::new();
    for line in listed.lines() {
        if let Some(path) = line.strip_prefix("worktree ") {
            found.push(Worktree {
                path: PathBuf::from(path),
                head: None,
                branch: None,
                locked: None,
                prunable: false,
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
            } else if line == "prunable" || line.starts_with("prunable ") {
                last.prunable = true;
            }
        }
    }
    found
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

/// The git directory of the worktree at `path`: its administrative
/// directory in the repository, which `git worktree move` keeps. It tells
/// a worktree apart from another one at the same path. Git names it after
/// the worktree's directory name and gives a removed worktree's name to the
/// next one added under it, so it tells nothing about a worktree found
/// where the withdrawal's own moves cannot have put it.
fn git_dir(path: &Path) -> Result<PathBuf, git::GitError> {
    Ok(PathBuf::from(git::run(
        path,
        ["rev-parse", "--absolute-git-dir"],
    )?))
}

/// Whether anything is at `path`; a look that fails is no answer.
fn exists(path: &Path) -> Result<bool, Stop> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(Stop::Unverified(WorkError::Io {
            path: path.to_path_buf(),
            source,
        })),
    }
}

/// Where a worktree the intent records is, as far as the withdrawal can
/// tell it apart from what others did.
#[derive(Debug)]
enum Located {
    /// Where the withdrawal acts on it ([`acts_at`]), at the path git lists
    /// it at.
    At(Worktree),
    /// Somewhere the withdrawal's moves cannot have put it, at the path git
    /// lists it at, while git still keeps its git directory for it
    /// ([`kept_for_it`]): moved there by someone else.
    Elsewhere(Worktree),
    /// Released: git lists it nowhere and no longer keeps its git
    /// directory, or, once staged, git lists it neither where it was staged
    /// nor at its path, and nothing is where it was staged.
    Gone,
}

/// Where the withdrawal acts on the worktree `recorded`: where its stage
/// says the withdrawal's own moves can have put it
/// ([`WorktreeStage::reachable`]), and its path, where a worktree found is
/// no longer moved, whoever put it back there.
fn acts_at(recorded: &IntentWorktree) -> Vec<PathBuf> {
    let mut places = recorded.stage.reachable(&recorded.path);
    if !places.contains(&recorded.path) {
        places.push(recorded.path.clone());
    }
    places
}

/// Whether git still keeps the git directory of the worktree `recorded`
/// for it alone, so that a checkout git lists with that git directory
/// anywhere is this worktree: until the withdrawal tries to remove it, in
/// every stage but [`WorktreeStage::Staged`]. Once staged, its removal may
/// have let git give that git directory to a new checkout.
fn kept_for_it(recorded: &IntentWorktree) -> bool {
    !matches!(recorded.stage, WorktreeStage::Staged(_))
}

/// The worktrees that may be the one `recorded`, in the order they are
/// read: those git lists where the withdrawal acts on it ([`acts_at`]),
/// then, while git keeps its git directory for it ([`kept_for_it`]), every
/// other one, which shows whether someone else moved it. Never the primary
/// checkout, nor one git reports prunable, whose directory is gone.
fn candidates<'a>(listed: &'a [Worktree], recorded: &IntentWorktree) -> Vec<&'a Worktree> {
    let places = acts_at(recorded);
    let mut candidates: Vec<&Worktree> = listed
        .iter()
        .skip(1)
        .filter(|w| !w.prunable)
        .filter(|w| kept_for_it(recorded) || places.contains(&w.path))
        .collect();
    candidates.sort_by_key(|w| {
        places
            .iter()
            .position(|at| *at == w.path)
            .unwrap_or(usize::MAX)
    });
    candidates
}

/// What git's list shows of the worktree whose git directory is `ours`.
#[derive(Debug)]
enum Placed<'a> {
    /// Listed where the withdrawal acts on it ([`acts_at`]).
    At(&'a Worktree),
    /// Listed elsewhere while git keeps its git directory for it
    /// ([`kept_for_it`]).
    Elsewhere(&'a Worktree),
    /// Not among the worktrees read, while git lists one whose git
    /// directory could not be read, with git's diagnostic: it may be that
    /// one.
    Unreadable { at: &'a Worktree, error: &'a str },
    /// Listed nowhere it is looked for, while something of it may remain
    /// (`remains`).
    Kept,
    /// Listed nowhere it is looked for, and nothing of it remains.
    Gone,
}

/// Where the worktree `recorded`, whose git directory is `ours`, is, from
/// `seen` — the [`candidates`] read, each with its git directory or git's
/// diagnostic for why it could not be read — and `remains`: whether
/// anything of it may be left that git does not list where it is looked
/// for; once staged, anything at its staged location, and otherwise its git
/// directory, which git still keeps. Only its own git directory recognizes
/// it: another checkout at its path is not it, and a candidate that could
/// not be read is no proof it is gone. Found where the withdrawal acts on
/// it ([`acts_at`]), it is there, including a staged worktree back at its
/// path; found anywhere else, it was moved there by someone else while git
/// keeps its git directory for it ([`kept_for_it`]), and otherwise, once
/// staged, it may be a new checkout git gave that git directory after
/// removing it, which is never it.
fn place<'a>(
    recorded: &IntentWorktree,
    ours: &Path,
    seen: &'a [(Worktree, Result<PathBuf, String>)],
    remains: bool,
) -> Placed<'a> {
    let places = acts_at(recorded);
    let mut found = seen.iter().filter(|(_, dir)| dir.as_deref() == Ok(ours));
    if let Some((at, _)) = found.clone().find(|(w, _)| places.contains(&w.path)) {
        return Placed::At(at);
    }
    if kept_for_it(recorded)
        && let Some((elsewhere, _)) = found.next()
    {
        return Placed::Elsewhere(elsewhere);
    }
    if let Some((at, Err(error))) = seen.iter().find(|(_, dir)| dir.is_err()) {
        return Placed::Unreadable { at, error };
    }
    if remains { Placed::Kept } else { Placed::Gone }
}

/// Where the worktree `recorded` is, recognized by its git directory
/// ([`place`]), reading each candidate's git directory ([`candidates`])
/// until its own is found. An intent recorded without its git
/// directory learns it first ([`learn_git_dir`]), setting it in `recorded`.
fn locate(listed: &[Worktree], recorded: &mut IntentWorktree) -> Result<Located, Stop> {
    let Some(ours) = learn_git_dir(listed, recorded)? else {
        return unrecognized(recorded);
    };
    let mut seen: Vec<(Worktree, Result<PathBuf, String>)> = Vec::new();
    for candidate in candidates(listed, recorded) {
        let dir = git_dir(&candidate.path).map_err(|err| err.to_string());
        let found = dir.as_deref() == Ok(ours.as_path());
        seen.push((candidate.clone(), dir));
        if found {
            break;
        }
    }
    let staged = match &recorded.stage {
        WorktreeStage::Staged(at) => Some(at.clone()),
        _ => None,
    };
    let remains = match &staged {
        Some(at) => listed.iter().any(|w| &w.path == at) || exists(at)?,
        None => exists(&ours)?,
    };
    match place(recorded, &ours, &seen, remains) {
        Placed::At(found) => Ok(Located::At(found.clone())),
        Placed::Elsewhere(found) => Ok(Located::Elsewhere(found.clone())),
        Placed::Unreadable { at, error } => Err(Stop::Unverified(WorkError::refused(format!(
            "git lists a worktree at {} whose git directory could not be read ({error}); whether it is the worktree {} could not be established",
            at.path.display(),
            recorded.path.display()
        )))),
        Placed::Kept => Err(Stop::Unverified(WorkError::refused(match staged {
            Some(at) => format!(
                "the withdrawal moved the worktree {} to {}, where git lists nothing it recognizes as this worktree while something is still there, listed or not; whether it is released could not be established. Once nothing of it is left at {}, withdraw again",
                recorded.path.display(),
                at.display(),
                at.display()
            ),
            None => {
                let reachable = recorded.stage.reachable(&recorded.path);
                let prunable: Vec<String> = listed
                    .iter()
                    .filter(|w| w.prunable && reachable.contains(&w.path))
                    .map(|w| format!(" (git reports {} prunable)", w.path.display()))
                    .collect();
                format!(
                    "git lists the worktree {} nowhere it can be read, yet still keeps its git directory {}{}: its directory is gone, or was moved without git recording where. If it was moved, run `git worktree repair <where it is>`; if it is gone, run `git worktree prune`; then withdraw again",
                    recorded.path.display(),
                    ours.display(),
                    prunable.concat()
                )
            }
        }))),
        Placed::Gone => Ok(Located::Gone),
    }
}

/// The git directory `recorded` records. An intent recorded without it
/// learns it from the worktree git lists where its stage says it can be,
/// and sets it in `recorded`; `None` when git lists one nowhere there.
fn learn_git_dir(
    listed: &[Worktree],
    recorded: &mut IntentWorktree,
) -> Result<Option<PathBuf>, Stop> {
    if let Some(ours) = &recorded.git_dir {
        return Ok(Some(ours.clone()));
    }
    let found = recorded
        .stage
        .reachable(&recorded.path)
        .into_iter()
        .find_map(|at| listed.iter().find(|w| w.path == at && !w.prunable));
    let Some(found) = found else {
        return Ok(None);
    };
    let ours = git_dir(&found.path)?;
    recorded.git_dir = Some(ours.clone());
    Ok(Some(ours))
}

/// A worktree an intent recorded without its git directory, which git lists
/// nowhere its stage says it can be: gone only while nothing is at any of
/// those places, since a move may have left it there unlisted.
fn unrecognized(recorded: &IntentWorktree) -> Result<Located, Stop> {
    for at in recorded.stage.reachable(&recorded.path) {
        if exists(&at)? {
            return Err(Stop::Unverified(WorkError::refused(format!(
                "git lists the worktree {} nowhere the withdrawal can have put it while {} exists, and the withdrawal recorded no git directory to recognize it by; where it is could not be established. Once nothing of it is left at {}, withdraw again",
                recorded.path.display(),
                at.display(),
                at.display()
            ))));
        }
    }
    Ok(Located::Gone)
}

/// [`locate`] for the worktree `intent` records at `at`, recording the git
/// directory it learned.
fn located(
    root: &Path,
    intent: &mut WithdrawalIntent,
    at: usize,
    recorded: &dyn Fn(&WithdrawalIntent) -> Result<(), WorkError>,
) -> Result<Located, Stop> {
    let learning = intent.worktrees[at].git_dir.is_none();
    let found = locate(&worktrees(root)?, &mut intent.worktrees[at]);
    if learning && intent.worktrees[at].git_dir.is_some() {
        recorded(intent)?;
    }
    found
}

/// The stage of the worktree at `path` once it is found at `found`
/// ([`Located::At`]): back at its path, it is no longer moved;
/// anywhere else, it is staged there — where a move of the withdrawal put
/// it, such as inside a directory that took its destination meanwhile.
fn settled(path: &Path, found: &Path) -> WorktreeStage {
    if found == path {
        WorktreeStage::AtOriginal
    } else {
        WorktreeStage::Staged(found.to_path_buf())
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
/// released, as git lists it now where the withdrawal acts on it.
#[derive(Debug)]
enum WorktreeStep {
    /// It is no longer the todo's: left where it is and out of the
    /// withdrawal.
    Left(Leaving),
    /// It is still the todo's: release it.
    Release(Worktree),
}

/// Why a worktree the intent records is no longer the todo's, left where
/// it is and out of the withdrawal.
#[derive(Debug, PartialEq, Eq)]
enum Leaving {
    /// Its checkout is no longer the one the withdrawal recorded.
    CheckoutChanged,
    /// It has the work branch checked out, while that branch is recorded
    /// deleted: a branch recreated under the same name.
    BranchReplaced,
    /// Someone else moved it to this path ([`Located::Elsewhere`]).
    MovedTo(PathBuf),
}

impl Leaving {
    /// The reason a structured event records.
    fn reason(&self) -> &'static str {
        match self {
            Self::CheckoutChanged => "checkout_changed",
            Self::BranchReplaced => "branch_replaced",
            Self::MovedTo(_) => "moved_elsewhere",
        }
    }
}

/// A worktree is still the todo's only while it has the checkout the
/// withdrawal recorded, and, when that checkout is the work branch, only
/// while that branch is still the one the withdrawal recorded
/// (`branch_gone` false): a checkout of a branch recreated under the same
/// name belongs to whoever recreated it.
fn worktree_step(
    recorded: &IntentWorktree,
    now: Worktree,
    work_branch: Option<&str>,
    branch_gone: bool,
) -> WorktreeStep {
    if now.checkout() != recorded.checkout {
        return WorktreeStep::Left(Leaving::CheckoutChanged);
    }
    if branch_gone && on_branch(recorded, work_branch) {
        return WorktreeStep::Left(Leaving::BranchReplaced);
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
    let mut checked = Vec::with_capacity(owned.len());
    for (worktree, tie) in &owned {
        check_worktree(worktree, &worktree.path).map_err(Stop::cause)?;
        checked.push(IntentWorktree {
            path: worktree.path.clone(),
            tie: *tie,
            checkout: worktree.checkout(),
            released: false,
            stage: WorktreeStage::AtOriginal,
            git_dir: Some(git_dir(&worktree.path)?),
        });
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
        worktrees: checked,
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
///   back first when the withdrawal moved it;
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
///   released, no worktree the withdrawal moved and no partial marker
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
/// A kept intent keeps everything it records, the branch's commit, each
/// worktree's stage and a partial marker included, and the
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
    // A stop the run can prove, while the intent records nothing released
    // and nothing under way, leaves the todo as it was. A stop is proven
    // only before an effect or after one proven to have changed nothing;
    // an effect that may have changed something stops unverified or is
    // recorded in the intent.
    let stopped = |intent: &WithdrawalIntent, stop: Stop| -> WorkError {
        if let Stop::Refused(_) = &stop
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
    let mut at = 0;
    while at < intent.worktrees.len() {
        if intent.worktrees[at].released {
            at += 1;
            continue;
        }
        match release_worktree(root, slug, &mut intent, at, &recorded) {
            Ok(Advance::Released) => at += 1,
            Ok(Advance::Left) => {}
            Err(stop) => return Err(stopped(&intent, stop)),
        }
    }
    let workspace = match workspace_now(&intent.workspace_path) {
        Ok(now) => workspace_step(intent.workspace, now),
        Err(stop) => return Err(stopped(&intent, stop)),
    };
    match workspace {
        WorkspaceStep::Nothing => {}
        WorkspaceStep::Holding(path) => return Err(stopped(&intent, holding_files(&path))),
        WorkspaceStep::Gone | WorkspaceStep::Remove(_) => {
            if let WorkspaceStep::Remove(path) = workspace
                && let Err(source) = instrukt_core::fsx::remove_dir(&path)
            {
                let cause = WorkError::refused(format!(
                    "its workspace {} was not removed ({source})",
                    path.display()
                ));
                return Err(kept(&intent, Stop::Unverified(cause)));
            }
            intent.workspace = WorkspaceRelease::Released;
            if let Err(cause) = recorded(&intent) {
                return Err(kept(&intent, Stop::Unverified(cause)));
            }
        }
    }
    let branch = match intent.branch.clone() {
        Some(name) => match delete_branch(root, &name, &mut intent, &recorded) {
            Ok(branch) => branch,
            Err(stop) => return Err(stopped(&intent, stop)),
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
    /// It is released and recorded.
    Released,
    /// It is no longer the todo's: left where it is and out of the intent.
    Left,
}

/// A stop while the worktree at `path` may be at `staging`: never an
/// abandonment. The refusal names `staging` as where the intent records
/// it; `cause` says what git and the filesystem show of it.
fn stop_while_moved(path: &Path, staging: &Path, stop: Stop) -> Stop {
    let (Stop::Refused(cause) | Stop::Unverified(cause)) = stop;
    Stop::Unverified(WorkError::refused(format!(
        "the withdrawal records its worktree {} at {}, where it moves it to be released: {cause}",
        path.display(),
        staging.display()
    )))
}

/// Where the withdrawal moves, moved or moves back a worktree at `stage`
/// from its path; `None` while it has not moved it.
fn away(stage: &WorktreeStage) -> Option<&Path> {
    match stage {
        WorktreeStage::AtOriginal => None,
        WorktreeStage::MovingTo(at)
        | WorktreeStage::Staged(at)
        | WorktreeStage::RestoringFrom(at) => Some(at),
    }
}

/// Releases the worktree `intent` records at `at`, recording the outcome,
/// through git's own worktree commands, recognizing it by its git directory
/// ([`locate`]) and acting on it only where the withdrawal acts on it
/// ([`acts_at`]). Found at its path, its stage records it not moved, even
/// when it was staged, and it is released from there ([`from_original`]);
/// found where a move of the withdrawal put it, its stage records that
/// place and it is finished there ([`from_staged`]). One git lists
/// elsewhere while git keeps its git directory for it ([`kept_for_it`])
/// was moved there by someone else: it is left where it is and out of the
/// withdrawal.
fn release_worktree(
    root: &Path,
    slug: &TodoSlug,
    intent: &mut WithdrawalIntent,
    at: usize,
    recorded: &dyn Fn(&WithdrawalIntent) -> Result<(), WorkError>,
) -> Result<Advance, Stop> {
    let path = intent.worktrees[at].path.clone();
    let moved = away(&intent.worktrees[at].stage).map(Path::to_path_buf);
    let fail = |stop: Stop| match &moved {
        Some(moved) => stop_while_moved(&path, moved, stop),
        None => stop,
    };
    match located(root, intent, at, recorded).map_err(fail)? {
        Located::Gone => released(intent, at, recorded),
        Located::Elsewhere(found) => left(slug, intent, at, Leaving::MovedTo(found.path), recorded),
        Located::At(found) => {
            let stage = settled(&path, &found.path);
            if intent.worktrees[at].stage != stage {
                intent.worktrees[at].stage = stage;
                recorded(intent).map_err(|cause| fail(Stop::Unverified(cause)))?;
            }
            if found.path == path {
                from_original(root, slug, intent, at, found, recorded)
            } else {
                from_staged(root, slug, intent, at, found, recorded)
            }
        }
    }
}

/// Releases the worktree `intent` records at `at`, which git lists as
/// `now` at its path and the withdrawal has not moved. Still the todo's
/// and clean, it is moved with `git worktree move` to its staging location
/// ([`staging_path`]), its stage recording that move before it is made
/// ([`WorktreeStage::MovingTo`]), and finished there ([`from_staged`]).
///
/// Moving it is what keeps a command addressed to its path from changing
/// its checkout unseen between the last look and the removal: once moved, a
/// git command addressed to its path does not reach it. A process whose
/// working directory was already inside it keeps reaching it at the staging
/// location (`state-database`, "Known caveats"). Whatever the move returns,
/// git's worktree list is read back where the move can have put it: listed
/// at its path, the move did not happen and a move that returned a failure
/// is proven to have changed nothing; listed at its staging location, or
/// inside a directory that appeared there meanwhile, it is staged there and
/// finished there; anything else keeps the move recorded, unverified.
fn from_original(
    root: &Path,
    slug: &TodoSlug,
    intent: &mut WithdrawalIntent,
    at: usize,
    now: Worktree,
    recorded: &dyn Fn(&WithdrawalIntent) -> Result<(), WorkError>,
) -> Result<Advance, Stop> {
    let path = intent.worktrees[at].path.clone();
    note_branch(root, intent, at, recorded)?;
    let worktree = match worktree_step(
        &intent.worktrees[at],
        now,
        intent.branch.as_deref(),
        intent.branch_deleted(),
    ) {
        WorktreeStep::Left(why) => return left(slug, intent, at, why, recorded),
        WorktreeStep::Release(worktree) => worktree,
    };
    check_worktree(&worktree, &path)?;
    let staging = staging_path(&path, &intent.todo)?;
    if exists(&staging)? {
        return Err(Stop::refused(format!(
            "{} is in the way of releasing the worktree {}; remove it, then withdraw again",
            staging.display(),
            path.display()
        )));
    }
    intent.worktrees[at].stage = WorktreeStage::MovingTo(staging.clone());
    recorded(intent)?;
    let (from, to) = (
        path.to_string_lossy().into_owned(),
        staging.to_string_lossy().into_owned(),
    );
    let moved = git::run(root, ["worktree", "move", from.as_str(), to.as_str()]);
    let moving = |stop: Stop| stop_while_moved(&path, &staging, stop);
    // Read back where git has it, whatever the move returned: a directory
    // that appeared at its staging location meanwhile has git move it
    // inside.
    match (moved, located(root, intent, at, recorded)) {
        (_, Ok(Located::At(staged))) if staged.path != path => {
            intent.worktrees[at].stage = WorktreeStage::Staged(staged.path.clone());
            recorded(intent).map_err(|cause| moving(Stop::Unverified(cause)))?;
            from_staged(root, slug, intent, at, staged, recorded)
        }
        (Err(git::GitError::Failed { stderr, .. }), Ok(Located::At(_))) => {
            intent.worktrees[at].stage = WorktreeStage::AtOriginal;
            recorded(intent)?;
            Err(Stop::refused(format!(
                "git kept the worktree {} where it is ({})",
                path.display(),
                stderr.trim()
            )))
        }
        (Err(err), Ok(_)) => Err(moving(Stop::Unverified(WorkError::refused(format!(
            "git returned a failure moving it ({err}); where it is now is recognized when the withdrawal is run again"
        ))))),
        (Ok(_), Ok(_)) => Err(moving(Stop::Unverified(WorkError::refused(
            "git reports moving it, yet does not list it moved",
        )))),
        (_, Err(stop)) => Err(moving(stop)),
    }
}

/// Finishes the release of the worktree `intent` records at `at`, which git
/// lists as `staged` where the withdrawal moved it and its stage records
/// ([`WorktreeStage::Staged`]). Looked at once more there, it is removed
/// while it is still the todo's and clean, and moved back to its path
/// otherwise ([`restore`]). Moved back because it holds uncommitted work or
/// is locked, it refuses the withdrawal naming its path; moved back because
/// it is no longer the todo's, it is left out of the withdrawal. One not
/// moved back — its path taken, or git not moving it — stays recorded
/// where git lists it, named, for the withdrawal run again. A removal that
/// returns a failure keeps the intent, and the refusal says what git and
/// the filesystem show of it afterwards ([`after_failed_removal`]).
fn from_staged(
    root: &Path,
    slug: &TodoSlug,
    intent: &mut WithdrawalIntent,
    at: usize,
    staged: Worktree,
    recorded: &dyn Fn(&WithdrawalIntent) -> Result<(), WorkError>,
) -> Result<Advance, Stop> {
    let path = intent.worktrees[at].path.clone();
    let staging = staged.path.clone();
    let moving = |stop: Stop| stop_while_moved(&path, &staging, stop);
    note_branch(root, intent, at, recorded).map_err(moving)?;
    let worktree = match worktree_step(
        &intent.worktrees[at],
        staged,
        intent.branch.as_deref(),
        intent.branch_deleted(),
    ) {
        WorktreeStep::Release(worktree) => worktree,
        WorktreeStep::Left(why) => {
            restore(root, intent, at, &staging, recorded)?;
            return left(slug, intent, at, why, recorded);
        }
    };
    match check_worktree(&worktree, &path) {
        Ok(()) => {}
        Err(Stop::Refused(cause)) => {
            restore(root, intent, at, &staging, recorded)?;
            return Err(Stop::Refused(cause));
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
    released(intent, at, recorded)
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

/// What leaves the way open for the worktree at `at` to be moved back to
/// its path `path`, taken now: `path` freed, or, when `at` is inside
/// `path`, the worktree moved out of it first, since freeing `path` would
/// take the worktree with it.
fn way_back(path: &Path, at: &Path) -> String {
    if at.starts_with(path) {
        format!(
            "It is inside {}: withdraw again once it is releasable to release it there, or move it out with `git worktree move {} <new path>` before freeing {}",
            path.display(),
            at.display(),
            path.display()
        )
    } else {
        format!(
            "Free {}, then withdraw again to move it back",
            path.display()
        )
    }
}

/// Moves the worktree `intent` records at `at` back from `staging`, where
/// the withdrawal moved it, to its path. Anything at its path now — another
/// checkout git has put there, or any file or directory — keeps it at
/// `staging`, unmoved and staged: git would move it inside that directory.
/// Otherwise its stage records the move back before it is made
/// ([`WorktreeStage::RestoringFrom`]), and afterwards it is looked for by
/// its git directory where that move can have put it ([`located`]): back at
/// its path, it is no longer moved; at `staging`, or inside a directory
/// that took its path between the look and the move, it is staged there and
/// named; listed elsewhere, someone else moved it there, which the refusal
/// names; when that look establishes nothing, the move back stays recorded
/// and the refusal says where it is could not be established, with what
/// kept the look from it.
fn restore(
    root: &Path,
    intent: &mut WithdrawalIntent,
    at: usize,
    staging: &Path,
    recorded: &dyn Fn(&WithdrawalIntent) -> Result<(), WorkError>,
) -> Result<(), Stop> {
    let path = intent.worktrees[at].path.clone();
    let unmoved =
        |why: String| stop_while_moved(&path, staging, Stop::Unverified(WorkError::refused(why)));
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
            "it is no longer releasable, and {} is taken now; it is not moved back and stays at {}. {}",
            path.display(),
            staging.display(),
            way_back(&path, staging)
        )));
    }
    let kept_at = |at: &Path, why: String| {
        stop_while_moved(&path, at, Stop::Unverified(WorkError::refused(why)))
    };
    intent.worktrees[at].stage = WorktreeStage::RestoringFrom(staging.to_path_buf());
    recorded(intent)?;
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
    match located(root, intent, at, recorded) {
        Ok(Located::At(found)) => {
            let found = found.path;
            intent.worktrees[at].stage = settled(&path, &found);
            recorded(intent)?;
            if found == path {
                return Ok(());
            }
            let why = if found == staging {
                format!(
                    "it is no longer releasable and git did not move it back{failure}; it is still at {}. Running the withdrawal again moves it back or releases it",
                    found.display()
                )
            } else {
                format!(
                    "it is no longer releasable, and git moved it to {} instead of {}{failure}, which was taken meanwhile. {}",
                    found.display(),
                    path.display(),
                    way_back(&path, &found)
                )
            };
            Err(kept_at(&found, why))
        }
        Ok(Located::Elsewhere(found)) => Err(kept_at(
            staging,
            format!(
                "it is no longer releasable, and after moving it back{failure} git lists it at {}, where the move cannot have put it: someone else moved it there. Withdraw again to leave it there",
                found.path.display()
            ),
        )),
        Ok(Located::Gone) => Err(kept_at(
            staging,
            format!(
                "it is no longer releasable, and after moving it back{failure} git lists it nowhere the move can have put it and no longer keeps its git directory; where it is could not be established"
            ),
        )),
        Err(stop) => Err(kept_at(
            staging,
            format!(
                "it is no longer releasable, and after moving it back{failure} where it is could not be established: {}",
                stop.cause()
            ),
        )),
    }
}

/// Records the worktree `intent` records at `at` released, no longer moved.
fn released(
    intent: &mut WithdrawalIntent,
    at: usize,
    recorded: &dyn Fn(&WithdrawalIntent) -> Result<(), WorkError>,
) -> Result<Advance, Stop> {
    intent.worktrees[at].released = true;
    intent.worktrees[at].stage = WorktreeStage::AtOriginal;
    recorded(intent)?;
    Ok(Advance::Released)
}

/// Leaves the worktree `intent` records at `at`, no longer the todo's
/// (`why`), out of the withdrawal, where it is, as one event naming its
/// path and, when someone else moved it, where it is now.
fn left(
    slug: &TodoSlug,
    intent: &mut WithdrawalIntent,
    at: usize,
    why: Leaving,
    recorded: &dyn Fn(&WithdrawalIntent) -> Result<(), WorkError>,
) -> Result<Advance, Stop> {
    let moved_to = match &why {
        Leaving::MovedTo(found) => Some(found.display().to_string()),
        Leaving::CheckoutChanged | Leaving::BranchReplaced => None,
    };
    tracing::info!(
        event = "work_withdrawal_worktree_left",
        todo = %slug,
        worktree = %intent.worktrees[at].path.display(),
        reason = why.reason(),
        at = moved_to.as_deref(),
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
) -> Result<(), Stop> {
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
/// recorded pending) or gone; each worktree, recognized by its git
/// directory ([`locate`]): released once it is gone; at its path, never
/// moved off it or back there, no longer moved; anywhere else the
/// withdrawal's moves can have put it, such as inside a directory a move
/// put it in, staged there; and the workspace it recorded present, at the
/// location it recorded, that is gone. A worktree staged is finished from
/// there by its release, and one someone else moved elsewhere is left by
/// it; a partial marker by the branch's deletion.
fn recognize(root: &Path, intent: &mut WithdrawalIntent) -> Result<(), WorkError> {
    if let (Some(name), BranchRelease::Pending) = (intent.branch.clone(), intent.branch_state) {
        let now = branch_now(root, intent, &name)?;
        note_gone(intent, &now);
    }
    if intent.worktrees.iter().any(|w| !w.released) {
        let listed = worktrees(root)?;
        for worktree in intent.worktrees.iter_mut().filter(|w| !w.released) {
            match locate(&listed, worktree).map_err(Stop::cause)? {
                Located::At(found) => worktree.stage = settled(&worktree.path, &found.path),
                Located::Elsewhere(_) => {}
                Located::Gone => {
                    worktree.released = true;
                    worktree.stage = WorktreeStage::AtOriginal;
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
/// each worktree the withdrawal is moving, has moved or is moving back, and
/// a partial marker not yet taken back.
fn underway(intent: &WithdrawalIntent) -> Vec<String> {
    let mut open: Vec<String> = intent
        .worktrees
        .iter()
        .filter(|w| !w.released)
        .filter_map(|w| {
            let path = w.path.display();
            match &w.stage {
                WorktreeStage::AtOriginal => None,
                WorktreeStage::MovingTo(to) => Some(format!(
                    "its worktree {path}, being moved to {} to be released",
                    to.display()
                )),
                WorktreeStage::Staged(at) => Some(format!(
                    "its worktree {path} at {}, where it is moved to be released",
                    at.display()
                )),
                WorktreeStage::RestoringFrom(from) => Some(format!(
                    "its worktree {path}, being moved back from {}",
                    from.display()
                )),
            }
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
                            DeletionAfterFailure::Ineffective => Stop::Refused(not_deleted()),
                            DeletionAfterFailure::MovedOn => kept(format!(
                                "it moved on from {head} while it was released ({failure})"
                            )),
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
            stage: WorktreeStage::AtOriginal,
            git_dir: Some(PathBuf::from(OURS)),
        }
    }

    /// The git directory of the worktree the tests' intent records.
    const OURS: &str = "/repo/.git/worktrees/tree";

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
            prunable: false,
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

    /// The staging location of the worktree the tests' intent records.
    const STAGING: &str = "/work/.instrukt-wd-tree";

    /// The worktree the tests' intent records, at `stage`.
    fn at_stage(stage: WorktreeStage) -> IntentWorktree {
        IntentWorktree {
            stage,
            ..recorded(Checkout::Branch("depth/work".to_owned()))
        }
    }

    fn staged() -> IntentWorktree {
        at_stage(WorktreeStage::Staged(PathBuf::from(STAGING)))
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
    fn a_failed_branch_deletion_that_left_its_marker_beside_a_branch_that_moved_on_is_unsettled() {
        let (head, other) = (sha('a'), sha('b'));
        assert_eq!(
            deletion_after_failure(&head, Some(&head), Some(&other)),
            DeletionAfterFailure::Unsettled
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
            listed(on_work),
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
            listed(on_work),
            Some("depth/work"),
            true,
        );
        assert!(matches!(step, WorktreeStep::Left(Leaving::BranchReplaced)));
    }

    #[test]
    fn a_worktree_whose_checkout_changed_is_left() {
        let step = worktree_step(
            &recorded(Checkout::Branch("depth/work".to_owned())),
            listed(Checkout::Branch("other".to_owned())),
            Some("depth/work"),
            false,
        );
        assert!(matches!(step, WorktreeStep::Left(Leaving::CheckoutChanged)));
    }

    #[test]
    fn a_detached_worktree_stays_the_todo_s_whatever_became_of_the_work_branch() {
        let detached = Checkout::Detached(sha('b'));
        let step = worktree_step(
            &recorded(detached.clone()),
            listed(detached),
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

    /// `seen` with each listed worktree read with git directory `dir`.
    fn read(listed: &[(&Path, &str)]) -> Vec<(Worktree, Result<PathBuf, String>)> {
        listed
            .iter()
            .map(|(at, dir)| (listed_at(at), Ok(PathBuf::from(dir))))
            .collect()
    }

    /// Another worktree's git directory.
    const OTHER: &str = "/repo/.git/worktrees/tree1";

    #[test]
    fn a_worktree_never_moved_is_found_at_its_path_by_its_own_git_directory() {
        let tree = at_stage(WorktreeStage::AtOriginal);
        let seen = read(&[(&tree.path, OURS)]);
        assert!(matches!(
            place(&tree, Path::new(OURS), &seen, true),
            Placed::At(found) if found.path == tree.path
        ));
    }

    #[test]
    fn a_worktree_the_withdrawal_never_moved_listed_elsewhere_was_moved_by_someone_else() {
        let tree = at_stage(WorktreeStage::AtOriginal);
        let operator = Path::new("/operator/path");
        let seen = read(&[(operator, OURS)]);
        assert!(matches!(
            place(&tree, Path::new(OURS), &seen, true),
            Placed::Elsewhere(found) if found.path == operator
        ));
    }

    #[test]
    fn another_checkout_at_its_path_is_not_taken_for_it() {
        let tree = at_stage(WorktreeStage::AtOriginal);
        let seen = read(&[(&tree.path, OTHER)]);
        assert!(matches!(
            place(&tree, Path::new(OURS), &seen, true),
            Placed::Kept
        ));
    }

    #[test]
    fn a_worktree_listed_nowhere_whose_git_directory_is_gone_is_gone() {
        assert!(matches!(
            place(
                &at_stage(WorktreeStage::AtOriginal),
                Path::new(OURS),
                &[],
                false
            ),
            Placed::Gone
        ));
    }

    #[test]
    fn a_worktree_being_moved_is_found_at_its_staging_location() {
        let tree = at_stage(WorktreeStage::MovingTo(PathBuf::from(STAGING)));
        let seen = read(&[(Path::new(STAGING), OURS)]);
        assert!(matches!(
            place(&tree, Path::new(OURS), &seen, true),
            Placed::At(found) if found.path == Path::new(STAGING)
        ));
    }

    #[test]
    fn a_worktree_whose_move_did_not_happen_is_found_at_its_path() {
        let tree = at_stage(WorktreeStage::MovingTo(PathBuf::from(STAGING)));
        let seen = read(&[(&tree.path, OURS)]);
        assert!(matches!(
            place(&tree, Path::new(OURS), &seen, true),
            Placed::At(found) if found.path == tree.path
        ));
    }

    #[test]
    fn a_worktree_a_move_put_inside_a_directory_at_its_staging_location_is_found_there() {
        let tree = at_stage(WorktreeStage::MovingTo(PathBuf::from(STAGING)));
        let nested = Path::new(STAGING).join("tree");
        let seen = read(&[(Path::new(STAGING), OTHER), (&nested, OURS)]);
        assert!(matches!(
            place(&tree, Path::new(OURS), &seen, true),
            Placed::At(found) if found.path == nested
        ));
    }

    #[test]
    fn a_worktree_a_move_back_put_inside_a_directory_at_its_path_is_found_there() {
        let tree = at_stage(WorktreeStage::RestoringFrom(PathBuf::from(STAGING)));
        let nested = tree.path.join(".instrukt-wd-tree");
        let seen = read(&[(&tree.path, OTHER), (&nested, OURS)]);
        assert!(matches!(
            place(&tree, Path::new(OURS), &seen, true),
            Placed::At(found) if found.path == nested
        ));
    }

    #[test]
    fn a_staged_worktree_is_found_where_it_was_staged() {
        let tree = staged();
        let seen = read(&[(Path::new(STAGING), OURS)]);
        assert!(matches!(
            place(&tree, Path::new(OURS), &seen, true),
            Placed::At(found) if found.path == Path::new(STAGING)
        ));
    }

    #[test]
    fn its_git_directory_given_to_a_new_checkout_elsewhere_after_its_staged_removal_is_not_it() {
        let tree = staged();
        let replacement = Path::new("/unrelated/parent/tree");
        let seen = read(&[(replacement, OURS)]);
        assert!(matches!(
            place(&tree, Path::new(OURS), &seen, false),
            Placed::Gone
        ));
    }

    #[test]
    fn a_staged_worktree_moved_back_to_its_path_is_found_there() {
        let tree = staged();
        let seen = read(&[(&tree.path, OURS)]);
        assert!(matches!(
            place(&tree, Path::new(OURS), &seen, false),
            Placed::At(found) if found.path == tree.path
        ));
    }

    #[test]
    fn a_worktree_being_moved_that_git_lists_elsewhere_was_moved_by_someone_else() {
        let tree = at_stage(WorktreeStage::MovingTo(PathBuf::from(STAGING)));
        let operator = Path::new("/operator/path");
        let seen = read(&[(operator, OURS)]);
        assert!(matches!(
            place(&tree, Path::new(OURS), &seen, true),
            Placed::Elsewhere(found) if found.path == operator
        ));
    }

    #[test]
    fn a_staged_worktree_git_no_longer_lists_with_something_left_where_it_was_staged_is_not_gone() {
        assert!(matches!(
            place(&staged(), Path::new(OURS), &[], true),
            Placed::Kept
        ));
    }

    #[test]
    fn a_worktree_found_by_its_git_directory_is_found_beside_one_whose_git_directory_is_unreadable()
    {
        let tree = at_stage(WorktreeStage::MovingTo(PathBuf::from(STAGING)));
        let seen = vec![
            (listed_at(&tree.path), Err("cannot read".to_owned())),
            (listed_at(Path::new(STAGING)), Ok(PathBuf::from(OURS))),
        ];
        assert!(matches!(
            place(&tree, Path::new(OURS), &seen, true),
            Placed::At(found) if found.path == Path::new(STAGING)
        ));
    }

    #[test]
    fn a_listed_worktree_whose_git_directory_is_unreadable_leaves_where_it_is_unestablished() {
        let tree = at_stage(WorktreeStage::AtOriginal);
        let diagnostic = "fatal: not a git repository".to_owned();
        let seen = vec![(listed_at(&tree.path), Err(diagnostic.clone()))];
        assert!(matches!(
            place(&tree, Path::new(OURS), &seen, false),
            Placed::Unreadable { at, error } if at.path == tree.path && *error == diagnostic
        ));
    }

    #[test]
    fn a_worktree_found_at_its_path_is_no_longer_moved() {
        let path = Path::new("/work/tree");
        assert_eq!(settled(path, path), WorktreeStage::AtOriginal);
    }

    #[test]
    fn a_worktree_found_away_from_its_path_is_staged_there() {
        let nested = Path::new("/work/tree/.instrukt-wd-tree");
        assert_eq!(
            settled(Path::new("/work/tree"), nested),
            WorktreeStage::Staged(nested.to_path_buf())
        );
    }

    #[test]
    fn the_primary_checkout_is_never_a_candidate() {
        let tree = at_stage(WorktreeStage::AtOriginal);
        let listed = [listed_at(&tree.path), listed_at(Path::new("/work/other"))];
        let paths: Vec<&Path> = candidates(&listed, &tree)
            .iter()
            .map(|w| w.path.as_path())
            .collect();
        assert_eq!(paths, [Path::new("/work/other")]);
    }

    #[test]
    fn a_worktree_git_reports_prunable_is_never_a_candidate() {
        let tree = at_stage(WorktreeStage::AtOriginal);
        let gone = Worktree {
            prunable: true,
            ..listed_at(&tree.path)
        };
        let listed = [listed_at(Path::new("/repo")), gone];
        assert!(candidates(&listed, &tree).is_empty());
    }

    #[test]
    fn a_worktree_the_withdrawal_never_moved_is_read_at_its_path_first_then_everywhere_else() {
        let tree = at_stage(WorktreeStage::AtOriginal);
        let listed = [
            listed_at(Path::new("/repo")),
            listed_at(Path::new("/operator/path")),
            listed_at(&tree.path),
        ];
        let paths: Vec<&Path> = candidates(&listed, &tree)
            .iter()
            .map(|w| w.path.as_path())
            .collect();
        assert_eq!(paths, [tree.path.as_path(), Path::new("/operator/path")]);
    }

    #[test]
    fn a_staged_worktree_is_read_only_where_it_was_staged_then_at_its_path() {
        let tree = staged();
        let listed = [
            listed_at(Path::new("/repo")),
            listed_at(&tree.path),
            listed_at(Path::new("/unrelated/parent/tree")),
            listed_at(Path::new(STAGING)),
        ];
        let paths: Vec<&Path> = candidates(&listed, &tree)
            .iter()
            .map(|w| w.path.as_path())
            .collect();
        assert_eq!(paths, [Path::new(STAGING), tree.path.as_path()]);
    }

    #[test]
    fn a_worktree_being_moved_back_is_read_where_the_move_can_have_put_it_then_everywhere_else() {
        let tree = at_stage(WorktreeStage::RestoringFrom(PathBuf::from(STAGING)));
        let listed = [
            listed_at(Path::new("/repo")),
            listed_at(Path::new("/operator/path")),
            listed_at(&tree.path),
            listed_at(Path::new(STAGING)),
        ];
        let paths: Vec<&Path> = candidates(&listed, &tree)
            .iter()
            .map(|w| w.path.as_path())
            .collect();
        assert_eq!(
            paths,
            [
                Path::new(STAGING),
                tree.path.as_path(),
                Path::new("/operator/path")
            ]
        );
    }

    #[test]
    fn a_worktree_git_reports_prunable_is_parsed_prunable() {
        let listed = parse_worktrees(
            "worktree /repo\nHEAD aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\nbranch refs/heads/main\n\nworktree /work/tree\nHEAD aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\ndetached\nprunable gitdir file points to non-existent location\n",
        );
        assert_eq!(
            listed.iter().map(|w| w.prunable).collect::<Vec<_>>(),
            [false, true]
        );
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
    fn an_intent_recording_a_worktree_the_withdrawal_is_moving_is_never_abandoned() {
        let moving = WithdrawalIntent {
            worktrees: vec![at_stage(WorktreeStage::MovingTo(PathBuf::from(STAGING)))],
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
}
