//! The local work record in the state database's work tables
//! (`state-database`, "Work tables"): each local item is one `todos` row
//! with its prerequisites, artifacts and versions, stamps, holds, session
//! bindings, reflection state and answered inbound commands. Reads take no
//! lock: an ordinary read of a database behind this build's schema migrates
//! it once under the writer lock, while an inspection read, the existence
//! read always among them, brings it forward in memory only. Each write
//! takes the per-computer writer lock for its one transaction alone, and
//! one accepted change commits in one transaction.
//! GitHub reflection serializes on the separate reflection lock, taken
//! before the writer lock and never while it is held.

use std::path::{Path, PathBuf};
use std::time::Duration;

use instrukt_core::fsx::LockGuard;
use instrukt_core::home::Home;
use instrukt_core::ids::{CommitSha, ProjectSlug, RoleName, SessionRef, TodoSlug};
use instrukt_core::machine::MachineId;
use instrukt_core::state_db::rusqlite::{self, Connection, OptionalExtension, Transaction, params};
use instrukt_core::state_db::{self, StateDb, StateDbError, StateRead};
use instrukt_runner::interfaces::{AgentPreference, CatchUp, TaskSchedule, Tier, TodoKind};
use jiff::Timestamp;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use super::_error::WorkError;
use super::_graph::WaitNode;
use super::_model::{
    Artifact, Binding, Claim, Delivery, ReleasedBranch, ReleasedHold, Removal, SCHEMA, TodoId,
    TodoRecord, TodoRef, Version,
};
use crate::evidence::{
    Actor, ArtifactName, ContentDigest, HoldRecord, HoldSource, ModelIdentity, ModelProvenance,
    StampFact, StampRecord, TodoStatus,
};

/// A local item: the record and what this computer knows of its GitHub
/// reflection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LocalItem {
    pub record: TodoRecord,
    pub reflection: ReflectionState,
}

/// What this computer knows of one item's reflection.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ReflectionState {
    pub refreshed_at: Option<Timestamp>,
    /// The issue's number and REST id.
    pub issue: Option<IssueRef>,
    /// The issue's `ETag` at the last read, for conditional reads: a new
    /// comment or label moves it.
    pub etag: Option<String>,
    /// The record revision GitHub last showed: the revision whose issue
    /// update landed.
    pub remote_revision: Option<u64>,
    /// Why the linked pull request's creation, reviews or closing are still
    /// outstanding.
    pub pull_outstanding: Option<String>,
    /// The issue's item on the project board, once added.
    pub board_item: Option<String>,
    /// The board (`<owner>/<number>`) that `board_item` belongs to: an item
    /// id means nothing on another board. A membership recorded without
    /// it is owed again.
    pub board: Option<String>,
    /// The `Record status` the board item was last set to.
    pub board_status: Option<String>,
    /// Why board reflection is still owed; retried on its own.
    pub board_failure: Option<String>,
    /// Comment ids of inbound `/instrukt` commands already answered.
    pub commands_seen: Vec<u64>,
    /// `/instrukt` command comments the last pull found and no command has
    /// answered yet; never stored.
    pub inbound: Vec<serde_json::Value>,
    /// Local changes not yet reflected.
    pub pending: bool,
    /// The linked pull request's number.
    pub pull: Option<u64>,
    /// The build verdicts whose review is on the pull request, by their
    /// stamps' record order.
    pub reviews: Vec<u64>,
    /// Whether delivery closed the pull request.
    pub pull_closed: bool,
    /// The last reflection failure that left the item pending.
    pub failure: Option<String>,
    /// Where the next read of the issue's comments starts: the latest update
    /// time among the comments read before the first control still
    /// outstanding (all of them when none is).
    pub comments_since: Option<String>,
    /// Where the next read of the issue's events starts: the page holding
    /// the last event read before the first control still outstanding (the
    /// last event read when none is), and that event's identity.
    pub events_page: Option<u64>,
    pub events_last: Option<u64>,
    /// The work-branch commit the last publication left on the remote.
    pub published_head: Option<CommitSha>,
    /// Why the work branch's local head is not on the remote.
    pub publication_outstanding: Option<String>,
    /// Requests GitHub refused as invalid, at most one per reflected target.
    pub refusals: Vec<KeptRefusal>,
}

/// A request GitHub refused as invalid, kept for its reflected target
/// (`state-database`, `reflections.refusals`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct KeptRefusal {
    pub target: RefusedTarget,
    /// The identity of the request sent: a digest of what it sent and the
    /// branch head it concerned.
    pub identity: String,
    /// GitHub's validation detail.
    pub detail: String,
}

/// A reflected target a refusal is kept for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "target")]
pub(crate) enum RefusedTarget {
    /// The todo's issue.
    Issue,
    /// The creation of the todo's pull request.
    PullCreation,
    /// One build verdict's review, by its stamp's record order.
    Review { verdict: u64 },
    /// One artifact version, by its record order.
    Artifact { name: ArtifactName, sequence: u64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct IssueRef {
    pub number: u64,
    pub id: u64,
}

/// A project's provider bookkeeping.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ProviderState {
    /// GitHub deferred requests until then (rate limiting).
    pub deferred_until: Option<Timestamp>,
}

/// How long a reflection waits for another process's reflection to let go
/// of the reflection lock.
pub(crate) const REFLECTION_LOCK_TIMEOUT: Duration = Duration::from_secs(60);

/// The per-computer reflection lock `state/reflection.lock`: it serializes
/// GitHub reflection across processes and is held across GitHub requests.
/// Lock order is reflection → writer: it is never requested while the
/// writer lock is held, and it is not re-entrant.
pub(crate) struct ReflectionLock {
    _guard: LockGuard,
}

impl ReflectionLock {
    /// Takes the lock, waiting at most [`REFLECTION_LOCK_TIMEOUT`] for
    /// another reflection; still held then is
    /// [`instrukt_core::fsx::FsError::LockTimeout`],
    /// told apart from a link or an OS failure.
    pub fn acquire(home: &Home) -> Result<Self, WorkError> {
        let path = Self::path(home)?;
        Ok(Self {
            _guard: LockGuard::acquire(&path, REFLECTION_LOCK_TIMEOUT)?,
        })
    }

    fn path(home: &Home) -> Result<PathBuf, WorkError> {
        let path = home.reflection_lock_file();
        state_db::refuse_link("the reflection lock", &path).map_err(StateDbError::from)?;
        Ok(path)
    }
}

/// Why a local change was not written: the row it was computed from
/// changed in between (`state-database`, "Writing").
pub(crate) enum Refusal {
    /// Another command changed the todo's record; the slug the live row
    /// carries now, `None` when the todo is gone from the project.
    Changed(Option<TodoSlug>),
    /// Another live todo of the project carries the slug the change gives
    /// this one.
    Taken,
    /// The todo is being withdrawn on this computer, or its withdrawal was
    /// interrupted: its intent is recorded (`withdrawal_intents`).
    Withdrawing,
}

/// Why a worktree belongs to a withdrawal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum WorktreeTie {
    /// One of the todo's session bindings names it.
    Bound,
    /// It has the todo's work branch checked out.
    OnBranch,
}

/// What a worktree has checked out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Checkout {
    Branch(String),
    Detached(CommitSha),
}

/// One worktree a withdrawal releases: why it belongs to the todo, what it
/// had checked out when the withdrawal checked it, whether it is released
/// yet, how far the withdrawal's own moves of it have come, and its git
/// directory, by which it is recognized ([`WorktreeStage`]). A worktree
/// whose checkout is no longer that one is not the todo's any more.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct IntentWorktree {
    pub path: PathBuf,
    pub tie: WorktreeTie,
    pub checkout: Option<Checkout>,
    pub released: bool,
    /// How far the withdrawal's own moves of it have come; it bounds where
    /// the withdrawal acts on it ([`WorktreeStage::reachable`]).
    #[serde(default, skip_serializing_if = "WorktreeStage::is_at_original")]
    pub stage: WorktreeStage,
    /// Its administrative directory in the repository, which
    /// `git worktree move` keeps (`git rev-parse --absolute-git-dir`):
    /// recorded when the withdrawal checks it, or, for an intent recorded
    /// without it, the first time a run finds the worktree where its stage
    /// says it can be.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_dir: Option<PathBuf>,
}

/// How far a withdrawal's own moves of a worktree have come
/// (`withdrawal_intents`, `worktrees`), which tells where the withdrawal
/// acts on it.
///
/// - **Persistence.** A move is recorded before the git command that makes
///   it ([`Self::MovingTo`], [`Self::RestoringFrom`]); [`Self::Staged`] is
///   recorded after git's worktree list, read back, shows the worktree
///   where the move put it.
/// - **Where it may be acted on.** Only where this withdrawal's own moves
///   can have put it ([`Self::reachable`]), and at its path, where a staged
///   worktree found back there is no longer moved ([`Self::AtOriginal`]).
/// - **Where it is looked for.** Its git directory is also looked for in
///   every other linked worktree git lists, except once staged: until the
///   withdrawal tries to remove it, git keeps its git directory for it, so
///   that git directory anywhere else is the worktree moved there by
///   someone else, left where it is; once staged, its removal may have let
///   git give that git directory to a new checkout anywhere else.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum WorktreeStage {
    /// The withdrawal has not moved it: it is at its path, or was moved by
    /// someone else.
    #[default]
    AtOriginal,
    /// The withdrawal is moving it from its path to this staging location
    /// beside it.
    MovingTo(PathBuf),
    /// The withdrawal moved it here, away from its path, where it is
    /// removed or from where it is moved back; recorded once git lists it
    /// here.
    Staged(PathBuf),
    /// The withdrawal is moving it back from here to its path.
    RestoringFrom(PathBuf),
}

impl WorktreeStage {
    /// Whether the withdrawal has not moved it.
    pub fn is_at_original(&self) -> bool {
        *self == Self::AtOriginal
    }

    /// The places the withdrawal's own moves can have put the worktree at
    /// `path` by this stage, where it acts on it: git moves a worktree into
    /// a directory that took its destination meanwhile, so each destination
    /// also counts with the worktree's name inside it. A staged worktree is
    /// acted on only where it was staged, or at `path` once found back
    /// there.
    pub fn reachable(&self, path: &Path) -> Vec<PathBuf> {
        let inside = |into: &Path, from: &Path| from.file_name().map(|name| into.join(name));
        match self {
            Self::AtOriginal => vec![path.to_path_buf()],
            Self::MovingTo(to) => std::iter::once(path.to_path_buf())
                .chain(std::iter::once(to.clone()))
                .chain(inside(to, path))
                .collect(),
            Self::Staged(at) => vec![at.clone()],
            Self::RestoringFrom(from) => std::iter::once(from.clone())
                .chain(std::iter::once(path.to_path_buf()))
                .chain(inside(path, from))
                .collect(),
        }
    }
}

/// How far a withdrawal's release of its work branch has come
/// (`withdrawal_intents`, `branch_state`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum BranchRelease {
    /// Not deleted yet. A marker found now was created by a deletion whose
    /// receipt was lost, and shows the branch deleted at its commit.
    Pending,
    /// A failed deletion is known to have left only its marker, beside the
    /// branch it did not delete. The marker proves nothing and is removed
    /// before the branch is deleted.
    PartialMarker,
    /// Deleted: by this withdrawal, or found gone or recreated at a commit
    /// that does not descend from `branch_head`.
    Deleted,
}

/// The todo's empty workspace in a withdrawal (`withdrawal_intents`,
/// `workspace`): whether there was one to release when the withdrawal
/// checked, and whether it is released. Only a workspace recorded present
/// is released, so one gone since is recognized as released rather than
/// taken for one that never was.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum WorkspaceRelease {
    /// There was none when the withdrawal checked: nothing to release.
    Absent,
    /// It was there, empty, and is not released yet.
    Present,
    /// It is released: removed by the withdrawal, or found gone.
    Released,
}

/// A withdrawal's durable intent (`state-database`, `withdrawal_intents`):
/// what it releases, recorded before anything is released, so an
/// interrupted withdrawal is completed from it rather than from what
/// remains. Local-only; never reflected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WithdrawalIntent {
    pub todo: TodoId,
    pub branch: Option<String>,
    /// The commit `branch` pointed at when the withdrawal last read it; the
    /// branch is deleted only against this commit, and it is the deleted
    /// branch's recovery commit.
    pub branch_head: Option<CommitSha>,
    pub worktrees: Vec<IntentWorktree>,
    /// Where the todo's workspace is in the checkout the withdrawal was
    /// admitted from: the one workspace it checks and releases, whichever
    /// checkout of the project runs it again.
    pub workspace_path: PathBuf,
    pub workspace: WorkspaceRelease,
    /// How far the release of `branch` has come.
    pub branch_state: BranchRelease,
    pub started_at: Timestamp,
}

impl WithdrawalIntent {
    /// Whether the intent may be closed with the todo left as it was: it
    /// records no working resource released, and no effect under way whose
    /// outcome a later run must still recognize — no worktree the
    /// withdrawal is moving, has moved or is moving back
    /// ([`WorktreeStage`]), and no partial marker not yet taken back.
    pub fn abandonable(&self) -> bool {
        self.workspace != WorkspaceRelease::Released
            && self.branch_state == BranchRelease::Pending
            && self
                .worktrees
                .iter()
                .all(|w| !w.released && w.stage.is_at_original())
    }

    /// Whether `branch` is recorded deleted.
    pub fn branch_deleted(&self) -> bool {
        self.branch_state == BranchRelease::Deleted
    }
}

/// What [`Store::complete_withdrawal`] committed.
pub(crate) enum Completed {
    /// The withdrawn record; its intent is closed.
    Withdrawn(Box<LocalItem>),
    /// The todo was no longer live, delivered or withdrawn elsewhere and
    /// taken in meanwhile; its intent is closed and its record left as it is.
    NotLive(Box<LocalItem>),
    /// The todo is gone from the project; its intent went with it.
    Gone,
}

/// One project's local items in the state database. It holds no lock:
/// each read opens a current-schema reader, and each write takes the writer
/// lock for one transaction and lets it go before returning, so no caller
/// can hold it across GitHub, git or runtime I/O. A writer never saves its
/// copy of a row; it commits its own contribution against the live row,
/// re-read inside its transaction.
pub(crate) struct Store {
    home: Home,
    project: ProjectSlug,
}

impl Store {
    pub fn new(home: &Home, project: &ProjectSlug) -> Self {
        Self {
            home: home.clone(),
            project: project.clone(),
        }
    }

    /// Runs `read` against a current-schema reader, inside one read
    /// transaction: every query of a compound read sees the same committed
    /// state, while writers still commit beside it. `empty` when no
    /// database exists yet.
    fn read<T>(
        &self,
        empty: T,
        read: impl FnOnce(&Connection, &Path, &ProjectSlug) -> Result<T, WorkError>,
    ) -> Result<T, WorkError> {
        let Some(db) = StateRead::open(&self.home)? else {
            return Ok(empty);
        };
        let snapshot = db
            .conn()
            .unchecked_transaction()
            .map_err(StateDbError::sqlite(db.path()))?;
        // Dropping the transaction ends the read; it wrote nothing.
        read(&snapshot, db.path(), &self.project)
    }

    /// Runs `change` in one transaction under the writer lock, released
    /// before this returns; an error rolls everything back.
    pub fn write<T>(
        &self,
        change: impl FnOnce(&Transaction<'_>, &Path, &ProjectSlug) -> Result<T, WorkError>,
    ) -> Result<T, WorkError> {
        let mut db = StateDb::open(&self.home)?;
        let path = db.path().to_path_buf();
        db.write(|tx| change(tx, &path, &self.project))
    }

    /// Runs `within` with the stored record of `id` (`None` when there is
    /// none or it belongs to another project) inside one write transaction
    /// under the writer lock, so no other write lands while it runs. It
    /// writes nothing.
    pub fn hold_record<T>(
        &self,
        id: &TodoId,
        within: impl FnOnce(Option<&TodoRecord>) -> T,
    ) -> Result<T, WorkError> {
        self.write(|tx, path, project| {
            let item = load_own(tx, path, project, id)?;
            Ok(within(item.as_ref().map(|item| &item.record)))
        })
    }

    /// The local items of the project a binding of which recorded one of
    /// `worktrees`, or whose work branch, recorded on the todo or on one of
    /// its attempts, is one of `branches`. Other items are not read.
    pub fn items_naming(
        &self,
        worktrees: &[String],
        branches: &[String],
    ) -> Result<Vec<LocalItem>, WorkError> {
        let worktrees =
            serde_json::to_string(worktrees).map_err(|e| WorkError::refused(e.to_string()))?;
        let branches =
            serde_json::to_string(branches).map_err(|e| WorkError::refused(e.to_string()))?;
        self.read(Vec::new(), |conn, path, project| {
            let sql = StateDbError::sqlite(path);
            let mut statement = conn
                // Each arm starts from the listed values and reaches the
                // matching rows by index; no other todo is visited.
                .prepare_cached(
                    "SELECT t.id FROM json_each(?3) AS r
                     CROSS JOIN todos AS t ON t.branch = r.value
                     WHERE t.project = ?1
                     UNION
                     SELECT t.id FROM json_each(?3) AS r
                     CROSS JOIN session_bindings AS b ON b.branch = r.value
                     CROSS JOIN todos AS t ON t.id = b.todo_id
                     WHERE t.project = ?1
                     UNION
                     SELECT t.id FROM json_each(?2) AS r
                     CROSS JOIN session_bindings AS b ON b.worktree = r.value
                     CROSS JOIN todos AS t ON t.id = b.todo_id
                     WHERE t.project = ?1
                     ORDER BY 1",
                )
                .map_err(&sql)?;
            let ids = statement
                .query_map(params![project.as_str(), worktrees, branches], |row| {
                    row.get::<_, String>(0)
                })
                .map_err(&sql)?
                .collect::<Result<Vec<String>, _>>()
                .map_err(&sql)?;
            let mut items = Vec::with_capacity(ids.len());
            for id in ids {
                items.extend(load_rows(conn, path, "id = ?1", &id)?);
            }
            Ok(items)
        })
    }

    /// The sessions bound to, or holding the execution claim of, the
    /// project's todos that are neither delivered nor removed, whatever the
    /// state of each binding's attempt, in text order.
    pub fn open_work_sessions(&self) -> Result<Vec<SessionRef>, WorkError> {
        let delivered = text_of(&TodoStatus::Delivered);
        let removed = text_of(&TodoStatus::Removed);
        self.read(Vec::new(), |conn, path, project| {
            let sql = StateDbError::sqlite(path);
            let mut statement = conn
                .prepare_cached(
                    "SELECT b.session FROM todos AS t
                     JOIN session_bindings AS b ON b.todo_id = t.id
                     WHERE t.project = ?1 AND t.status NOT IN (?2, ?3)
                       AND b.session IS NOT NULL
                     UNION
                     SELECT t.claim_session FROM todos AS t
                     WHERE t.project = ?1 AND t.status NOT IN (?2, ?3)
                       AND t.claim_session IS NOT NULL
                     ORDER BY 1",
                )
                .map_err(&sql)?;
            let sessions = statement
                .query_map(params![project.as_str(), delivered, removed], |row| {
                    row.get::<_, String>(0)
                })
                .map_err(&sql)?
                .collect::<Result<Vec<String>, _>>()
                .map_err(&sql)?;
            sessions.iter().map(|text| session(path, text)).collect()
        })
    }

    /// Every local item of the project, in identity order.
    pub fn items(&self) -> Result<Vec<LocalItem>, WorkError> {
        self.read(Vec::new(), load_items)
    }

    /// Every local item and the provider state of the project, from one
    /// reader.
    pub fn existence(&self) -> Result<(Vec<LocalItem>, ProviderState), WorkError> {
        self.read(
            (Vec::new(), ProviderState::default()),
            |conn, path, project| {
                Ok((
                    load_items(conn, path, project)?,
                    load_provider(conn, path, project)?,
                ))
            },
        )
    }

    /// The stored row of `id`, `None` when there is none or it belongs to
    /// another project.
    pub fn item(&self, id: &TodoId) -> Result<Option<LocalItem>, WorkError> {
        self.read(None, |conn, path, project| {
            load_own(conn, path, project, id)
        })
    }

    /// The replies of `todo` still to be posted, as (command comment, the
    /// command's time when recorded, reply), oldest first.
    pub fn pending_replies(
        &self,
        todo: &str,
    ) -> Result<Vec<(u64, Option<String>, String)>, WorkError> {
        self.read(Vec::new(), |conn, path, _| {
            query(
                conn,
                path,
                "SELECT comment_id,commanded_at,reply FROM inbound_commands WHERE todo_id=?1 AND reply_pending=1 ORDER BY comment_id",
                todo,
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
        })
    }

    pub fn reply_sent(&self, todo: &str, comment: u64) -> Result<(), WorkError> {
        self.write(|tx, path, _| {
            tx.execute(
                "UPDATE inbound_commands SET reply_pending=0 WHERE todo_id=?1 AND comment_id=?2",
                params![todo, comment],
            )
            .map_err(StateDbError::sqlite(path))?;
            Ok(())
        })
    }

    /// Records the answered command `comment` of the todo whose record
    /// `decided_on` the answer was decided on, and its fixed reply, without
    /// saving the item. `false`, with nothing recorded, while the todo's
    /// withdrawal is open (`withdrawal_intents`) or once its live record is
    /// no longer `decided_on`: the command stays outstanding for a pull
    /// that decides it on the committed record, and the issue's `ETag` is
    /// dropped so that pull reads the command again ([`may_record_inbound`]).
    pub fn retain_reply(
        &self,
        decided_on: &TodoRecord,
        comment: u64,
        commanded_at: Option<&str>,
        answer: &str,
    ) -> Result<bool, WorkError> {
        self.write(|tx, path, project| {
            if !may_record_inbound(tx, path, project, decided_on)? {
                return Ok(false);
            }
            retain_reply(
                tx,
                path,
                decided_on.id.as_str(),
                comment,
                commanded_at,
                answer,
            )?;
            Ok(true)
        })
    }

    /// Records label action `action` on `issue` of the todo whose record
    /// `decided_on` the action was judged on as ignored. `false`, with
    /// nothing recorded, while the todo's withdrawal is open or once its
    /// live record is no longer `decided_on`: the action stays outstanding
    /// for a pull that judges it on the committed record.
    pub fn ignore_label(
        &self,
        decided_on: &TodoRecord,
        issue: u64,
        action: u64,
        kind: &str,
        login: &str,
        at: &str,
    ) -> Result<bool, WorkError> {
        self.write(|tx, path, project| {
            if !may_record_inbound(tx, path, project, decided_on)? {
                return Ok(false);
            }
            tx.execute("INSERT INTO github_label_actions(project,issue_number,action_id,todo_id,action,login,at,outcome) VALUES (?1,?2,?3,?4,?5,?6,?7,'ignored') ON CONFLICT DO NOTHING",params![project.as_str(),issue,action,decided_on.id.as_str(),kind,login,at]).map_err(StateDbError::sqlite(path))?;
            Ok(true)
        })
    }

    /// Drops the stored `ETag` of `id`'s issue: a command its last pull
    /// found was not recorded, and the next pull reads it again.
    pub fn forget_etag(&self, id: &TodoId) -> Result<(), WorkError> {
        self.write(|tx, path, _| forget_etag(tx, path, id))
    }

    pub fn label_seen(&self, issue: u64, action: u64) -> Result<bool, WorkError> {
        self.read(false, |conn, path, project| {
            conn.query_row("SELECT EXISTS(SELECT 1 FROM github_label_actions WHERE project=?1 AND issue_number=?2 AND action_id=?3)",params![project.as_str(),issue,action],|r|r.get(0)).map_err(StateDbError::sqlite(path)).map_err(Into::into)
        })
    }

    /// The board reflection last set up or found for the project, recorded
    /// on this computer; `None` before the first.
    pub fn recorded_board(&self) -> Result<Option<String>, WorkError> {
        self.read(None, |conn, path, project| {
            conn.query_row(
                "SELECT board FROM project_boards WHERE project=?1",
                params![project.as_str()],
                |r| r.get(0),
            )
            .optional()
            .map_err(StateDbError::sqlite(path))
            .map_err(Into::into)
        })
    }

    /// Records `board` (`<owner>/<number>`) as the project's board.
    pub fn record_board(&self, board: &str) -> Result<(), WorkError> {
        self.write(|tx, path, project| {
            tx.execute(
                "INSERT INTO project_boards(project,board) VALUES (?1,?2) ON CONFLICT(project) DO UPDATE SET board=excluded.board",
                params![project.as_str(), board],
            )
            .map_err(StateDbError::sqlite(path))?;
            Ok(())
        })
    }

    /// Removes `id` and every row that belongs to it while `expired` still
    /// holds for its live row; whether it was removed.
    pub fn delete_if(
        &self,
        id: &TodoId,
        expired: impl FnOnce(&LocalItem) -> bool,
    ) -> Result<bool, WorkError> {
        self.write(|tx, path, project| match load_own(tx, path, project, id)? {
            Some(live) if expired(&live) => {
                delete_in(tx, path, id)?;
                Ok(true)
            }
            _ => Ok(false),
        })
    }

    pub fn provider(&self) -> Result<ProviderState, WorkError> {
        self.read(ProviderState::default(), load_provider)
    }

    pub fn save_provider(&self, state: &ProviderState) -> Result<(), WorkError> {
        self.write(|tx, path, project| {
            let sql = match state.deferred_until {
                Some(until) => tx.execute(
                    "INSERT INTO provider_deferrals (project, until) VALUES (?1, ?2)
                     ON CONFLICT (project) DO UPDATE SET until = excluded.until",
                    params![project.as_str(), until.to_string()],
                ),
                None => tx.execute(
                    "DELETE FROM provider_deferrals WHERE project = ?1",
                    params![project.as_str()],
                ),
            };
            sql.map_err(StateDbError::sqlite(path))?;
            Ok(())
        })
    }

    /// Commits what one flush learned of `loaded`, the row it re-read
    /// under the reflection lock, as `reflected`, onto the live row
    /// ([`merge_reflection`]). `None` when the row is gone or belongs to
    /// another project.
    pub fn commit_reflection(
        &self,
        loaded: &LocalItem,
        reflected: &LocalItem,
    ) -> Result<Option<LocalItem>, WorkError> {
        let id = &loaded.record.id;
        self.write(|tx, path, project| {
            let Some(live) = load_own(tx, path, project, id)? else {
                return Ok(None);
            };
            save_in(tx, path, &merge_reflection(&live, loaded, reflected))?;
            Ok(Some(written(tx, path, id)?))
        })
    }

    /// Applies `amend` to the live row's reflection state: a note
    /// reflection keeps on its own. `None` when the row is gone or belongs
    /// to another project.
    pub fn amend_reflection(
        &self,
        id: &TodoId,
        amend: impl FnOnce(&mut ReflectionState),
    ) -> Result<Option<LocalItem>, WorkError> {
        self.write(|tx, path, project| {
            let Some(mut live) = load_own(tx, path, project, id)? else {
                return Ok(None);
            };
            amend(&mut live.reflection);
            save_in(tx, path, &live)?;
            Ok(Some(written(tx, path, id)?))
        })
    }

    /// The stored row of `id` and whether its withdrawal intent is open,
    /// read in one snapshot; `None` when there is no row or it belongs to
    /// another project.
    pub fn item_withdrawing(&self, id: &TodoId) -> Result<Option<(LocalItem, bool)>, WorkError> {
        self.read(None, |conn, path, project| {
            let Some(item) = load_own(conn, path, project, id)? else {
                return Ok(None);
            };
            let withdrawing = load_intent(conn, path, project, id)?.is_some();
            Ok(Some((item, withdrawing)))
        })
    }

    /// The recorded withdrawal intent of `id`, `None` when it has none.
    pub fn withdrawal_intent(&self, id: &TodoId) -> Result<Option<WithdrawalIntent>, WorkError> {
        self.read(None, |conn, path, project| {
            load_intent(conn, path, project, id)
        })
    }

    /// Records `intent`, the withdrawal of the todo the command entered as
    /// `base` under `slug`, before anything is released. Inside the
    /// transaction the project's live todo under `slug` must be `base`'s
    /// identity, its record unchanged since the entry and no conflict mark
    /// new since then ([`admission`]), and no withdrawal of it recorded.
    /// The todo's own row is not written: it stays live, with its claim as
    /// it was or unclaimed when it had none, until the withdrawal completes.
    /// Returns the live row, or why it was refused.
    pub fn open_withdrawal(
        &self,
        base: &LocalItem,
        slug: &TodoSlug,
        intent: &WithdrawalIntent,
    ) -> Result<Result<LocalItem, Refusal>, WorkError> {
        let id = &base.record.id;
        self.write(|tx, path, project| {
            let items = load_items(tx, path, project)?;
            let live = items
                .iter()
                .find(|it| &it.record.slug == slug && it.record.is_live());
            let Some(live) = live.filter(|it| &it.record.id == id) else {
                let now = items
                    .iter()
                    .find(|it| &it.record.id == id)
                    .map(|it| it.record.slug.clone());
                return Ok(Err(Refusal::Changed(now)));
            };
            match admission(live, base) {
                Admission::Changed => {
                    return Ok(Err(Refusal::Changed(Some(live.record.slug.clone()))));
                }
                Admission::Admitted => {}
            }
            if load_intent(tx, path, project, id)?.is_some() {
                return Ok(Err(Refusal::Withdrawing));
            }
            tx.execute(
                "INSERT INTO withdrawal_intents
                    (todo_id, branch, branch_head, worktrees, workspace_path, workspace,
                     branch_state, started_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    id.as_str(),
                    intent.branch,
                    intent.branch_head.as_ref().map(CommitSha::as_str),
                    json_of(&intent.worktrees)?,
                    path_text(path, &intent.workspace_path)?,
                    text_of(&intent.workspace),
                    text_of(&intent.branch_state),
                    intent.started_at.to_string(),
                ],
            )
            .map_err(StateDbError::sqlite(path))?;
            Ok(Ok(live.clone()))
        })
    }

    /// Records what an open withdrawal has learned or done since: the
    /// branch's commit before the branch is deleted, each worktree, the
    /// workspace and the branch once released, a worktree no longer the
    /// todo's left out. `false` when no intent of the todo is recorded.
    pub fn record_withdrawal(&self, intent: &WithdrawalIntent) -> Result<bool, WorkError> {
        self.write(|tx, path, project| {
            if load_intent(tx, path, project, &intent.todo)?.is_none() {
                return Ok(false);
            }
            tx.execute(
                "UPDATE withdrawal_intents
                 SET branch_head = ?2, worktrees = ?3, workspace = ?4, branch_state = ?5
                 WHERE todo_id = ?1",
                params![
                    intent.todo.as_str(),
                    intent.branch_head.as_ref().map(CommitSha::as_str),
                    json_of(&intent.worktrees)?,
                    text_of(&intent.workspace),
                    text_of(&intent.branch_state),
                ],
            )
            .map_err(StateDbError::sqlite(path))?;
            Ok(true)
        })
    }

    /// Closes the withdrawal intent of `id` while it is abandonable
    /// ([`WithdrawalIntent::abandonable`]): the todo stays as it is.
    /// `false`, with the intent kept, when the stored intent records a
    /// released resource or an effect under way; `true` when it is closed or
    /// none is recorded.
    pub fn abandon_withdrawal(&self, id: &TodoId) -> Result<bool, WorkError> {
        self.write(
            |tx, path, project| match load_intent(tx, path, project, id)? {
                Some(intent) if !intent.abandonable() => Ok(false),
                Some(_) => {
                    tx.execute(
                        "DELETE FROM withdrawal_intents WHERE todo_id = ?1",
                        params![id.as_str()],
                    )
                    .map_err(StateDbError::sqlite(path))?;
                    forget_etag(tx, path, id)?;
                    Ok(true)
                }
                None => Ok(true),
            },
        )
    }

    /// Commits the withdrawal of `id` and closes its intent, in one
    /// transaction: the live row is withdrawn at `at` with `branch` and
    /// `worktrees`, what was released, as its `removal`, as a new revision
    /// pending reflection ([`TodoRecord::withdraw`]). The withdrawal is
    /// written onto the live row, whatever reflection committed there
    /// while the resources were released; a row no longer live is left as
    /// it is.
    pub fn complete_withdrawal(
        &self,
        id: &TodoId,
        at: Timestamp,
        branch: Option<ReleasedBranch>,
        worktrees: Vec<PathBuf>,
    ) -> Result<Completed, WorkError> {
        self.write(|tx, path, project| {
            let Some(mut live) = load_own(tx, path, project, id)? else {
                return Ok(Completed::Gone);
            };
            tx.execute(
                "DELETE FROM withdrawal_intents WHERE todo_id = ?1",
                params![id.as_str()],
            )
            .map_err(StateDbError::sqlite(path))?;
            // Commands and label actions found while the intent was open
            // were left outstanding; the issue may not change again before
            // the next pull, so that pull reads it unconditionally.
            forget_etag(tx, path, id)?;
            if !live.record.is_live() {
                return Ok(Completed::NotLive(Box::new(written(tx, path, id)?)));
            }
            let before = live.record.clone();
            live.reflection.etag = None;
            live.record.withdraw(at, branch, worktrees);
            live.record.revision += 1;
            live.reflection.pending = true;
            save_in(tx, path, &live)?;
            fleet_record(
                tx,
                path,
                project,
                &self.home,
                at,
                [(Some(&before), &live.record)],
            )?;
            Ok(Completed::Withdrawn(Box::new(written(tx, path, id)?)))
        })
    }
}

/// Commits the local change `mine` inside `tx`, computed from `base`, the
/// row the command last read or wrote (`None` for a todo it creates),
/// against the live row ([`admission`], [`rebase_local`]). `others` are
/// the todos the same change writes, whose slugs are judged in memory.
/// Returns the written row, or why the change was refused.
pub(crate) fn commit_local(
    tx: &Transaction<'_>,
    path: &Path,
    project: &ProjectSlug,
    base: Option<&LocalItem>,
    mine: &LocalItem,
    others: &[TodoId],
) -> Result<Result<LocalItem, Refusal>, WorkError> {
    let id = &mine.record.id;
    if load_intent(tx, path, project, id)?.is_some() {
        return Ok(Err(Refusal::Withdrawing));
    }
    let next = match base {
        Some(base) => match admit(tx, path, project, base)? {
            Ok(live) => rebase_local(&live, mine),
            Err(refusal) => return Ok(Err(refusal)),
        },
        // An identity another project's row holds is never written over,
        // nor its row handed to this project's session.
        None => match load_one(tx, path, id)? {
            None => mine.clone(),
            Some(other) if &other.record.project != project => {
                return Ok(Err(Refusal::Changed(None)));
            }
            Some(live) => return Ok(Err(Refusal::Changed(Some(live.record.slug)))),
        },
    };
    let renamed = base.is_none_or(|base| base.record.slug != mine.record.slug);
    if renamed && mine.record.is_live() {
        let taken = load_items(tx, path, project)?.into_iter().any(|other| {
            &other.record.id != id
                && !others.contains(&other.record.id)
                && other.record.slug == mine.record.slug
                && other.record.is_live()
        });
        if taken {
            return Ok(Err(Refusal::Taken));
        }
    }
    save_in(tx, path, &next)?;
    Ok(Ok(written(tx, path, id)?))
}

/// Judges, inside `tx`, a change computed from `base`, the row the command
/// last read or wrote, against its todo's live row ([`admission`]): the
/// live row when the change is admitted, else why it is refused. A row of
/// the identity that belongs to another project is reported gone.
pub(crate) fn admit(
    tx: &Transaction<'_>,
    path: &Path,
    project: &ProjectSlug,
    base: &LocalItem,
) -> Result<Result<LocalItem, Refusal>, WorkError> {
    Ok(match load_one(tx, path, &base.record.id)? {
        Some(live) if &live.record.project == project => match admission(&live, base) {
            Admission::Changed => Err(Refusal::Changed(Some(live.record.slug))),
            Admission::Admitted => Ok(live),
        },
        _ => Err(Refusal::Changed(None)),
    })
}

/// A cycle of waiting through one of `through` in `project`'s graph as
/// `tx` holds it: every local row and every listing copy of a todo that is
/// not one. The slugs from the todo back to itself; `None` when the graph
/// is acyclic through each of them.
pub(crate) fn cycle_in(
    tx: &Transaction<'_>,
    path: &Path,
    project: &ProjectSlug,
    through: &[TodoId],
) -> Result<Option<Vec<TodoSlug>>, WorkError> {
    let sql = StateDbError::sqlite(path);
    let mut nodes: Vec<WaitNode> = Vec::new();
    {
        let mut rows = tx
            .prepare("SELECT id, slug, status, group_id FROM todos WHERE project = ?1 ORDER BY id")
            .map_err(&sql)?;
        let found = rows
            .query_map(params![project.as_str()], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            })
            .map_err(&sql)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(&sql)?;
        for (id, slug, status, group) in found {
            let status = value_of::<TodoStatus>(path, &status)?;
            nodes.push(WaitNode {
                id: TodoId::parse(&id).ok_or_else(|| invalid(path, "a todo identity"))?,
                slug: TodoSlug::parse(&slug).map_err(|e| invalid(path, e.to_string()))?,
                after: Vec::new(),
                group: group
                    .map(|g| TodoId::parse(&g).ok_or_else(|| invalid(path, "a todo identity")))
                    .transpose()?,
                holder: status == TodoStatus::StructuralHolder,
                live: matches!(
                    status,
                    TodoStatus::Active | TodoStatus::Iceboxed | TodoStatus::StructuralHolder
                ),
                local: true,
            });
        }
        let mut edges = tx
            .prepare(
                "SELECT p.todo_id, p.prerequisite_id FROM prerequisites p
                 JOIN todos t ON t.id = p.todo_id WHERE t.project = ?1 ORDER BY p.rowid",
            )
            .map_err(&sql)?;
        let edges = edges
            .query_map(params![project.as_str()], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(&sql)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(&sql)?;
        for (todo, prerequisite) in edges {
            let prerequisite =
                TodoId::parse(&prerequisite).ok_or_else(|| invalid(path, "a todo identity"))?;
            if let Some(node) = nodes.iter_mut().find(|n| n.id.as_str() == todo) {
                node.after.push(prerequisite);
            }
        }
    }
    Ok(super::_graph::first_cycle(&nodes, through))
}

/// How each of `ids` stands in `project` as `tx` holds it, for judging a
/// reference to it: its local row's slug and status, else its listing
/// copy's. A todo this computer holds neither of is absent.
pub(crate) fn referenced(
    tx: &Transaction<'_>,
    path: &Path,
    project: &ProjectSlug,
    ids: &[TodoId],
) -> Result<std::collections::BTreeMap<TodoId, (TodoSlug, TodoStatus)>, WorkError> {
    let sql = StateDbError::sqlite(path);
    let mut found = std::collections::BTreeMap::new();
    for id in ids {
        let row: Option<(String, String)> = tx
            .query_row(
                "SELECT slug, status FROM todos WHERE id = ?1 AND project = ?2",
                params![id.as_str(), project.as_str()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(&sql)?;
        if let Some((slug, status)) = row {
            let slug = TodoSlug::parse(&slug).map_err(|e| invalid(path, e.to_string()))?;
            found.insert(id.clone(), (slug, value_of::<TodoStatus>(path, &status)?));
        }
    }
    Ok(found)
}

/// Whether `project` has, among this computer's todos, an active one that
/// is unheld and not a structural holder (`fleet-attention`, "Attention
/// episodes").
pub(crate) fn runnable_remains(
    tx: &Transaction<'_>,
    path: &Path,
    project: &ProjectSlug,
) -> Result<bool, WorkError> {
    tx.query_row(
        "SELECT EXISTS (SELECT 1 FROM todos t
                        WHERE t.project = ?1 AND t.status = 'active'
                          AND NOT EXISTS (SELECT 1 FROM holds h
                                          WHERE h.todo_id = t.id AND h.released_at IS NULL))",
        params![project.as_str()],
        |row| row.get(0),
    )
    .map_err(StateDbError::sqlite(path))
    .map_err(WorkError::from)
}

/// Whether `todo` is active work of `project` on this computer: an active
/// todo or a structural holder.
pub(crate) fn active_todo(
    tx: &Transaction<'_>,
    path: &Path,
    project: &ProjectSlug,
    todo: &TodoSlug,
) -> Result<bool, WorkError> {
    tx.query_row(
        "SELECT EXISTS (SELECT 1 FROM todos
                        WHERE project = ?1 AND slug = ?2
                          AND status IN ('active', 'structural_holder'))",
        params![project.as_str(), todo.as_str()],
        |row| row.get(0),
    )
    .map_err(StateDbError::sqlite(path))
    .map_err(WorkError::from)
}

/// Calls the runner's fleet-attention contract inside `tx` for the change
/// from each `before` row to its written `after` row, answering once
/// whether runnable work remains after all of them.
pub(crate) fn fleet_record<'a>(
    tx: &Transaction<'_>,
    path: &Path,
    project: &ProjectSlug,
    home: &Home,
    now: Timestamp,
    changes: impl IntoIterator<Item = (Option<&'a TodoRecord>, &'a TodoRecord)>,
) -> Result<(), WorkError> {
    let facts: Vec<_> = changes
        .into_iter()
        .filter_map(|(before, after)| super::_fleet::facts(before, after))
        .collect();
    if facts.is_empty() {
        return Ok(());
    }
    let runnable = runnable_remains(tx, path, project)?;
    for facts in facts {
        instrukt_runner::fleet_attention::record(tx, home, now, &facts.with_runnable(runnable))?;
    }
    Ok(())
}

/// The local rows of `project` that refer to todo `id`, as `tx` holds
/// them: those naming it as a prerequisite or as their group.
pub(crate) fn referrers(
    tx: &Transaction<'_>,
    path: &Path,
    project: &ProjectSlug,
    id: &TodoId,
) -> Result<Vec<(TodoId, TodoSlug)>, WorkError> {
    let sql = StateDbError::sqlite(path);
    let mut rows = tx
        .prepare(
            "SELECT t.id, t.slug FROM todos t
             WHERE t.project = ?1
               AND (t.group_id = ?2
                    OR EXISTS (SELECT 1 FROM prerequisites p
                               WHERE p.todo_id = t.id AND p.prerequisite_id = ?2))
             ORDER BY t.id",
        )
        .map_err(&sql)?;
    let found = rows
        .query_map(params![project.as_str(), id.as_str()], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })
        .map_err(&sql)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(&sql)?;
    found
        .into_iter()
        .map(|(id, slug)| {
            Ok((
                TodoId::parse(&id).ok_or_else(|| invalid(path, "a todo identity"))?,
                TodoSlug::parse(&slug).map_err(|e| invalid(path, e.to_string()))?,
            ))
        })
        .collect()
}

/// What a local change's precondition finds on the live row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Admission {
    Admitted,
    /// Another command changed the record.
    Changed,
}

/// The precondition of a local change computed from `base` on the `live`
/// row: the records must be equal, ignoring what reflection learns of
/// artifact versions.
pub(crate) fn admission(live: &LocalItem, base: &LocalItem) -> Admission {
    if !live.record.same_as(&base.record) {
        Admission::Changed
    } else {
        Admission::Admitted
    }
}

/// The carrying step: `mine`'s record with the `live` row's reflection
/// state, and the live artifact address, remote version and content
/// wherever `mine` holds none.
pub(crate) fn carry(live: &LocalItem, mine: &LocalItem) -> LocalItem {
    let mut record = mine.record.clone();
    learn(&mut record, &live.record);
    LocalItem {
        record,
        reflection: live.reflection.clone(),
    }
}

/// Fills in `record` what `from` learned of the same artifact versions,
/// matched by artifact name and version sequence and digest, wherever
/// `record` holds none.
fn learn(record: &mut TodoRecord, from: &TodoRecord) {
    for artifact in &mut record.artifacts {
        let Some(source) = from.artifacts.iter().find(|a| a.name == artifact.name) else {
            continue;
        };
        if artifact.address.is_none() {
            artifact.address = source.address;
        }
        for version in &mut artifact.versions {
            let Some(known) = source
                .versions
                .iter()
                .find(|v| v.sequence == version.sequence && v.digest == version.digest)
            else {
                continue;
            };
            if version.remote.is_none() {
                version.remote = known.remote;
            }
            if version.content.is_none() {
                version.content.clone_from(&known.content);
            }
        }
    }
}

/// A local change `mine` rebased on the `live` row: the carrying step,
/// pending reflection, and every command either has answered.
pub(crate) fn rebase_local(live: &LocalItem, mine: &LocalItem) -> LocalItem {
    let mut next = carry(live, mine);
    next.reflection.pending = true;
    for comment in &mine.reflection.commands_seen {
        if !next.reflection.commands_seen.contains(comment) {
            next.reflection.commands_seen.push(*comment);
        }
    }
    next
}

/// What a flush of `loaded` that produced `reflected` leaves on the `live`
/// row: every reflection field the flush changed takes its value, every
/// other keeps the live one, and the artifact values it learned attach to
/// the matching versions. Pending clears only while the live record is the
/// one reflected; a newer local record stays pending and loses its `ETag`.
pub(crate) fn merge_reflection(
    live: &LocalItem,
    loaded: &LocalItem,
    reflected: &LocalItem,
) -> LocalItem {
    let mut next = live.clone();
    learn(&mut next.record, &reflected.record);
    let (to, from, was) = (
        &mut next.reflection,
        &reflected.reflection,
        &loaded.reflection,
    );
    macro_rules! changed {
        ($($field:ident),*) => {$(
            if from.$field != was.$field {
                to.$field.clone_from(&from.$field);
            }
        )*};
    }
    changed!(
        refreshed_at,
        issue,
        etag,
        remote_revision,
        pull_outstanding,
        board_item,
        board,
        board_status,
        board_failure,
        pending,
        pull,
        reviews,
        comments_since,
        events_page,
        events_last,
        published_head,
        publication_outstanding,
        refusals,
        pull_closed,
        failure
    );
    for comment in &from.commands_seen {
        if !to.commands_seen.contains(comment) {
            to.commands_seen.push(*comment);
        }
    }
    if !live.record.same_as(&loaded.record) {
        to.pending = true;
        to.etag = None;
    }
    next
}

/// `item` as its row stores it: commands found on the last pull are never
/// stored.
pub(crate) fn stored(item: &LocalItem) -> LocalItem {
    let mut row = item.clone();
    row.reflection.inbound = Vec::new();
    row
}

/// The row of `id` as `tx` holds it now, once written.
fn written(tx: &Connection, path: &Path, id: &TodoId) -> Result<LocalItem, WorkError> {
    load_one(tx, path, id)?
        .ok_or_else(|| invalid(path, format!("todo {id} vanished as it was written")))
}

/// Retains the reply `answer` to command `comment` of `todo`, created at
/// `commanded_at` as GitHub gave it, pending until it is posted.
pub(crate) fn retain_reply(
    tx: &Connection,
    path: &Path,
    todo: &str,
    comment: u64,
    commanded_at: Option<&str>,
    answer: &str,
) -> Result<(), WorkError> {
    tx.execute("INSERT INTO inbound_commands(todo_id,comment_id,reply,reply_pending,commanded_at) VALUES (?1,?2,?3,1,?4) ON CONFLICT(todo_id,comment_id) DO UPDATE SET reply=COALESCE(inbound_commands.reply,excluded.reply),reply_pending=CASE WHEN inbound_commands.reply IS NULL THEN 1 ELSE inbound_commands.reply_pending END,commanded_at=COALESCE(inbound_commands.commanded_at,excluded.commanded_at)",params![todo,comment,answer,commanded_at]).map_err(StateDbError::sqlite(path))?;
    Ok(())
}

/// Whether an inbound command or label action decided on `decided_on`
/// may be recorded inside `tx`: the todo's live row still holds that record
/// and no withdrawal of it is open (`withdrawal_intents`). When it may not,
/// the issue's `ETag` is dropped in the same transaction, so the next pull
/// reads the issue's commands and actions again rather than being told
/// nothing changed, and decides them on the committed record.
fn may_record_inbound(
    tx: &Connection,
    path: &Path,
    project: &ProjectSlug,
    decided_on: &TodoRecord,
) -> Result<bool, WorkError> {
    let id = &decided_on.id;
    let current = match load_own(tx, path, project, id)? {
        Some(live) => {
            live.record.same_as(decided_on) && load_intent(tx, path, project, id)?.is_none()
        }
        None => false,
    };
    if !current {
        forget_etag(tx, path, id)?;
    }
    Ok(current)
}

/// Drops the stored `ETag` of `id`'s issue, so its next pull reads the
/// issue unconditionally.
fn forget_etag(tx: &Connection, path: &Path, id: &TodoId) -> Result<(), WorkError> {
    tx.execute(
        "UPDATE reflections SET etag = NULL WHERE todo_id = ?1",
        params![id.as_str()],
    )
    .map_err(StateDbError::sqlite(path))?;
    Ok(())
}

/// The withdrawal intent of `id` in `project`, as `conn` holds it.
fn load_intent(
    conn: &Connection,
    path: &Path,
    project: &ProjectSlug,
    id: &TodoId,
) -> Result<Option<WithdrawalIntent>, WorkError> {
    type Row = (
        Option<String>,
        Option<String>,
        String,
        String,
        String,
        String,
        String,
    );
    let row: Option<Row> = conn
        .query_row(
            "SELECT w.branch, w.branch_head, w.worktrees, w.workspace_path, w.workspace,
                    w.branch_state, w.started_at
             FROM withdrawal_intents w JOIN todos t ON t.id = w.todo_id
             WHERE w.todo_id = ?1 AND t.project = ?2",
            params![id.as_str(), project.as_str()],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                ))
            },
        )
        .optional()
        .map_err(StateDbError::sqlite(path))?;
    let Some((branch, head, worktrees, workspace_path, workspace, branch_state, started_at)) = row
    else {
        return Ok(None);
    };
    Ok(Some(WithdrawalIntent {
        todo: id.clone(),
        branch,
        branch_head: head
            .map(|head| CommitSha::parse(&head).map_err(|e| invalid(path, e.to_string())))
            .transpose()?,
        worktrees: parse_json(path, &worktrees)?,
        workspace_path: PathBuf::from(workspace_path),
        workspace: value_of(path, &workspace)?,
        branch_state: value_of(path, &branch_state)?,
        started_at: time(path, &started_at)?,
    }))
}

fn load_provider(
    conn: &Connection,
    path: &Path,
    project: &ProjectSlug,
) -> Result<ProviderState, WorkError> {
    let until: Option<String> = conn
        .query_row(
            "SELECT until FROM provider_deferrals WHERE project = ?1",
            params![project.as_str()],
            |row| row.get(0),
        )
        .optional()
        .map_err(StateDbError::sqlite(path))?;
    Ok(ProviderState {
        deferred_until: until.map(|text| time(path, &text)).transpose()?,
    })
}

/// Every local item of `project`, read without the writer lock; no
/// database yet is no items.
pub(crate) fn read_items(home: &Home, project: &ProjectSlug) -> Result<Vec<LocalItem>, WorkError> {
    Store::new(home, project).items()
}

/// Local items and provider state of `project`, read
/// through an inspection snapshot: without the writer lock, and with older
/// state brought forward in memory only, so the file stays as found; no
/// database yet is empty state.
pub(crate) fn read_existence_state(
    home: &Home,
    project: &ProjectSlug,
) -> Result<(Vec<LocalItem>, ProviderState), WorkError> {
    Store::new(&home.for_inspection(), project).existence()
}

/// Writes every row of `item` inside `tx`: the `todos` row first, then the
/// rows that belong to it. Artifact versions are retained and never
/// rewritten; only what reflection learns of them changes.
fn save_in(tx: &Connection, path: &Path, item: &LocalItem) -> Result<(), WorkError> {
    let r = &item.record;
    let id = r.id.as_str();
    let sql = StateDbError::sqlite(path);
    let (agent, tier) = match &r.agent {
        Some(pref) => (Some(pref.agent.as_str()), pref.tier.as_ref().map(text_of)),
        None => (None, None),
    };
    let removal = r
        .removal
        .as_ref()
        .map(|removal| {
            json_of(&Resources {
                branch: removal.branch.clone(),
                worktrees: removal.worktrees.clone(),
            })
        })
        .transpose()?;
    tx.execute(
        "INSERT INTO todos (id, project, slug, kind, status, created_at, created_by,
            human_involved, revision, group_id, group_slug, schedule, catch_up, agent, tier,
            claim_machine, claim_session, claim_at, branch, delivered_commit, delivered_at,
            delivered_by, removed_at, removed_from, removal, sequence)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17,
            ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26)
         ON CONFLICT (id) DO UPDATE SET project = excluded.project, slug = excluded.slug,
            kind = excluded.kind, status = excluded.status, created_at = excluded.created_at,
            created_by = excluded.created_by, human_involved = excluded.human_involved,
            revision = excluded.revision, group_id = excluded.group_id,
            group_slug = excluded.group_slug, schedule = excluded.schedule,
            catch_up = excluded.catch_up, agent = excluded.agent, tier = excluded.tier,
            claim_machine = excluded.claim_machine, claim_session = excluded.claim_session,
            claim_at = excluded.claim_at, branch = excluded.branch,
            delivered_commit = excluded.delivered_commit, delivered_at = excluded.delivered_at,
            delivered_by = excluded.delivered_by, removed_at = excluded.removed_at,
            removed_from = excluded.removed_from, removal = excluded.removal,
            sequence = excluded.sequence",
        params![
            id,
            r.project.as_str(),
            r.slug.as_str(),
            text_of(&r.kind),
            text_of(&r.status),
            r.created.to_string(),
            actor_text(&r.created_by),
            r.human_involved,
            r.revision,
            r.group.as_ref().map(|g| g.id.as_str()),
            r.group.as_ref().map(|g| g.slug.as_str()),
            r.schedule.as_ref().map(schedule_text),
            text_of(&r.catch_up),
            agent,
            tier,
            r.claim.as_ref().map(|c| c.machine.to_string()),
            r.claim
                .as_ref()
                .and_then(|c| c.session.as_ref())
                .map(ToString::to_string),
            r.claim.as_ref().map(|c| c.at.to_string()),
            r.branch,
            r.delivery.as_ref().map(|d| d.commit.as_str()),
            r.delivery.as_ref().map(|d| d.at.to_string()),
            r.delivery.as_ref().map(|d| actor_text(&d.by)),
            r.removal.as_ref().map(|d| d.at.to_string()),
            r.removal.as_ref().map(|d| text_of(&d.from)),
            removal,
            r.sequence,
        ],
    )
    .map_err(&sql)?;
    tx.execute(
        "UPDATE todos SET refreshed_at=?2 WHERE id=?1",
        params![id, item.reflection.refreshed_at.map(|t| t.to_string())],
    )
    .map_err(&sql)?;
    for table in ["prerequisites", "stamps", "holds", "session_bindings"] {
        tx.execute(
            &format!("DELETE FROM {table} WHERE todo_id = ?1"),
            params![id],
        )
        .map_err(&sql)?;
    }
    for alias in &r.prior_slugs {
        tx.execute(
            "INSERT INTO todo_aliases(todo_id,slug) VALUES (?1,?2) ON CONFLICT DO NOTHING",
            params![id, alias.as_str()],
        )
        .map_err(&sql)?;
    }
    for action in &r.label_actions {
        tx.execute("INSERT INTO github_label_actions(project,issue_number,action_id,todo_id,action,login,at,outcome) VALUES (?1,?2,?3,?4,?5,?6,?7,?8) ON CONFLICT(project,issue_number,action_id) DO UPDATE SET todo_id=excluded.todo_id,action=excluded.action,login=excluded.login,at=excluded.at,outcome=excluded.outcome", params![r.project.as_str(),action.issue,action.id,id,action.action,action.login,action.at,action.outcome]).map_err(&sql)?;
    }
    for stamp in &r.stamps {
        if matches!(
            stamp.fact,
            StampFact::Approval { .. } | StampFact::ChangesRequested { .. }
        ) {
            let gate = match &stamp.fact {
                StampFact::Approval { gate, .. } | StampFact::ChangesRequested { gate, .. } => {
                    match gate {
                        crate::evidence::ApprovalGate::Prd => "prd",
                        crate::evidence::ApprovalGate::Task => "task",
                    }
                }
                _ => unreachable!(),
            };
            let content = serde_json::json!({"gate":gate,"decision":stamp.fact,"actor":stamp.actor,"evidence":stamp.fact,"at":stamp.at.to_string(),"source_at":stamp.at.to_string(),"arrived_at":stamp.at.to_string(),"source_comment":stamp.source_comment}).to_string();
            tx.execute("INSERT INTO approval_attention(todo_id,decision_sequence,content,state) VALUES (?1,?2,?3,'pending') ON CONFLICT DO NOTHING",params![id,stamp.sequence,content]).map_err(&sql)?;
        }
    }
    for prerequisite in &r.after {
        tx.execute(
            "INSERT OR IGNORE INTO prerequisites (todo_id, prerequisite_id, slug) VALUES (?1, ?2, ?3)",
            params![id, prerequisite.id.as_str(), prerequisite.slug.as_str()],
        )
        .map_err(&sql)?;
    }
    save_artifacts(tx, path, r)?;
    for stamp in &r.stamps {
        let (fact, evidence) = split_fact(&stamp.fact)?;
        tx.execute(
            "INSERT INTO stamps (todo_id, sequence, fact, at, actor, role, model, model_version,
                model_provenance, eval_fingerprint, human_involved, evidence, source_comment)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                id,
                stamp.sequence,
                fact,
                stamp.at.to_string(),
                actor_text(&stamp.actor),
                stamp.role.as_ref().map(RoleName::as_str),
                stamp.model.as_ref().map(|m| m.model.as_str()),
                stamp.model.as_ref().map(|m| m.version.as_str()),
                text_of(&stamp.model_provenance),
                stamp.eval_fingerprint,
                stamp.human_involved,
                evidence,
                stamp.source_comment,
            ],
        )
        .map_err(&sql)?;
    }
    let released = r
        .hold_history
        .iter()
        .map(|h| (&h.hold, Some((h.released, &h.by))));
    for (hold, release) in released.chain(r.hold.iter().map(|h| (h, None))) {
        tx.execute(
            "INSERT INTO holds (todo_id, source, reason, set_by, set_at, released_at, released_by)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                id,
                text_of(&hold.source),
                hold.reason,
                actor_text(&hold.by),
                hold.at.to_string(),
                release.map(|(at, _)| at.to_string()),
                release.map(|(_, by)| actor_text(by)),
            ],
        )
        .map_err(&sql)?;
    }
    for binding in &r.bindings {
        let a = &binding.attempt;
        tx.execute(
            "INSERT INTO session_bindings (todo_id,role,attempt,runtime,semantic_key,session,worktree,branch,base_head,completion_sequence,completed_head,failure_reason,failure_head,failure_workspace,failed_at,released_at,evidence,display_title) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18)",
            params![id,binding.role.as_str(),a.number,a.runtime.as_ref().map(|r|r.as_str()).or_else(||binding.session.as_ref().map(|s|s.runtime.as_str())),a.semantic_key,binding.session.as_ref().map(ToString::to_string),binding.worktree.as_ref().map(|p|p.to_string_lossy()),a.branch,a.base_head.as_ref().map(ToString::to_string),a.completion_sequence,a.completed_head.as_ref().map(ToString::to_string),a.failure_reason,a.failure_head.as_ref().map(ToString::to_string),a.failure_workspace,a.failed_at.map(|t|t.to_string()),a.released_at.map(|t|t.to_string()),serde_json::to_string(binding).map_err(|e|invalid(path,e.to_string()))?,a.native_placement.as_ref().map(|p|p.title.as_str()).or_else(||a.native_assignment.as_ref().map(|a|a.title.as_str()))],
        ).map_err(&sql)?;
    }
    let f = &item.reflection;
    tx.execute(
        "INSERT OR REPLACE INTO reflections (todo_id, issue_number, remote_revision, pending,
            failure, pull_number, issue_id, etag, pull_outstanding, reviews, pull_closed, board,
            board_item, board_status, board_failure, comments_since, events_page, events_last,
            published_head, publication_outstanding, refusals)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17,
            ?18, ?19, ?20, ?21)",
        params![
            id,
            f.issue.map(|i| i.number),
            f.remote_revision,
            f.pending,
            f.failure,
            f.pull,
            f.issue.map(|i| i.id),
            f.etag,
            f.pull_outstanding,
            json_of(&f.reviews)?,
            f.pull_closed,
            f.board,
            f.board_item,
            f.board_status,
            f.board_failure,
            f.comments_since,
            f.events_page,
            f.events_last,
            f.published_head.as_ref().map(CommitSha::as_str),
            f.publication_outstanding,
            json_of(&f.refusals)?,
        ],
    )
    .map_err(&sql)?;
    for comment in &f.commands_seen {
        tx.execute(
            "INSERT OR IGNORE INTO inbound_commands (todo_id, comment_id) VALUES (?1, ?2)",
            params![id, comment],
        )
        .map_err(&sql)?;
    }
    Ok(())
}

/// Removes `id`'s row; the rows that belong to it go with it.
fn delete_in(tx: &Connection, path: &Path, id: &TodoId) -> Result<(), WorkError> {
    tx.execute("DELETE FROM todos WHERE id = ?1", params![id.as_str()])
        .map_err(StateDbError::sqlite(path))?;
    Ok(())
}

fn save_artifacts(tx: &Connection, path: &Path, r: &TodoRecord) -> Result<(), WorkError> {
    let sql = StateDbError::sqlite(path);
    let id = r.id.as_str();
    let names: Vec<&str> = r.artifacts.iter().map(|a| a.name.as_str()).collect();
    let sequences: Vec<u64> = r
        .artifacts
        .iter()
        .flat_map(|a| a.versions.iter().map(|v| v.sequence))
        .collect();
    let kept_names = serde_json::to_string(&names).map_err(|e| invalid(path, e))?;
    let kept_sequences = serde_json::to_string(&sequences).map_err(|e| invalid(path, e))?;
    tx.execute(
        "DELETE FROM artifact_versions WHERE todo_id = ?1
            AND sequence NOT IN (SELECT value FROM json_each(?2))",
        params![id, kept_sequences],
    )
    .map_err(&sql)?;
    tx.execute(
        "DELETE FROM artifacts WHERE todo_id = ?1
            AND name NOT IN (SELECT value FROM json_each(?2))",
        params![id, kept_names],
    )
    .map_err(&sql)?;
    for artifact in &r.artifacts {
        tx.execute(
            "INSERT INTO artifacts (todo_id, name, address) VALUES (?1, ?2, ?3)
             ON CONFLICT (todo_id, name) DO UPDATE SET address = excluded.address
                WHERE address IS NOT excluded.address",
            params![id, artifact.name.as_str(), artifact.address],
        )
        .map_err(&sql)?;
        for version in &artifact.versions {
            tx.execute(
                "INSERT INTO artifact_versions (todo_id, name, sequence, digest, writer,
                    created_at, content, derived_from, grounding, remote)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                 ON CONFLICT (todo_id, sequence) DO UPDATE SET name = excluded.name,
                    digest = excluded.digest, writer = excluded.writer,
                    created_at = excluded.created_at, content = excluded.content,
                    derived_from = excluded.derived_from, grounding = excluded.grounding,
                    remote = excluded.remote
                 WHERE name IS NOT excluded.name OR digest IS NOT excluded.digest
                    OR remote IS NOT excluded.remote OR content IS NOT excluded.content
                    OR grounding IS NOT excluded.grounding
                    OR derived_from IS NOT excluded.derived_from",
                params![
                    id,
                    artifact.name.as_str(),
                    version.sequence,
                    version.digest.as_str(),
                    actor_text(&version.writer),
                    version.created.to_string(),
                    version.content,
                    json_of(&version.derived_from)?,
                    json_of(&version.grounding)?,
                    version.remote,
                ],
            )
            .map_err(&sql)?;
        }
    }
    Ok(())
}

fn load_items(
    conn: &Connection,
    path: &Path,
    project: &ProjectSlug,
) -> Result<Vec<LocalItem>, WorkError> {
    load_rows(conn, path, "project = ?1", project.as_str())
}

/// The stored row of `id` when it belongs to `project`: every row a
/// session reloads or commits by identity is one of its own project's.
fn load_own(
    conn: &Connection,
    path: &Path,
    project: &ProjectSlug,
    id: &TodoId,
) -> Result<Option<LocalItem>, WorkError> {
    Ok(load_one(conn, path, id)?.filter(|item| &item.record.project == project))
}

/// The stored row of `id`, `None` when there is none.
fn load_one(conn: &Connection, path: &Path, id: &TodoId) -> Result<Option<LocalItem>, WorkError> {
    Ok(load_rows(conn, path, "id = ?1", id.as_str())?.pop())
}

/// The `todos` rows matching `filter` (one `?1` parameter, `value`), in
/// identity order, each with the rows that belong to it.
fn load_rows(
    conn: &Connection,
    path: &Path,
    filter: &str,
    value: &str,
) -> Result<Vec<LocalItem>, WorkError> {
    let sql = StateDbError::sqlite(path);
    let mut statement = conn
        .prepare_cached(&format!(
            "SELECT id, project, slug, kind, status, created_at, created_by, human_involved,
                revision, group_id, group_slug, schedule, catch_up, agent, tier,
                claim_machine, claim_session, claim_at, branch, delivered_commit, delivered_at,
                delivered_by, removed_at, removed_from, removal, sequence
             FROM todos WHERE {filter} ORDER BY id"
        ))
        .map_err(&sql)?;
    let rows = statement
        .query_map(params![value], |row| {
            Ok(TodoRow {
                id: row.get(0)?,
                project: row.get(1)?,
                slug: row.get(2)?,
                kind: row.get(3)?,
                status: row.get(4)?,
                created_at: row.get(5)?,
                created_by: row.get(6)?,
                human_involved: row.get(7)?,
                revision: row.get(8)?,
                group_id: row.get(9)?,
                group_slug: row.get(10)?,
                schedule: row.get(11)?,
                catch_up: row.get(12)?,
                agent: row.get(13)?,
                tier: row.get(14)?,
                claim_machine: row.get(15)?,
                claim_session: row.get(16)?,
                claim_at: row.get(17)?,
                branch: row.get(18)?,
                delivered_commit: row.get(19)?,
                delivered_at: row.get(20)?,
                delivered_by: row.get(21)?,
                removed_at: row.get(22)?,
                removed_from: row.get(23)?,
                removal: row.get(24)?,
                sequence: row.get(25)?,
            })
        })
        .map_err(&sql)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(&sql)?;
    rows.into_iter()
        .map(|row| load_item(conn, path, row))
        .collect()
}

struct TodoRow {
    id: String,
    project: String,
    slug: String,
    kind: String,
    status: String,
    created_at: String,
    created_by: String,
    human_involved: Option<bool>,
    revision: u64,
    group_id: Option<String>,
    group_slug: Option<String>,
    schedule: Option<String>,
    catch_up: String,
    agent: Option<String>,
    tier: Option<String>,
    claim_machine: Option<String>,
    claim_session: Option<String>,
    claim_at: Option<String>,
    branch: Option<String>,
    delivered_commit: Option<String>,
    delivered_at: Option<String>,
    delivered_by: Option<String>,
    removed_at: Option<String>,
    removed_from: Option<String>,
    removal: Option<String>,
    sequence: u64,
}

/// The released working resources a withdrawal keeps as evidence, as the
/// `removal` column holds them.
#[derive(Serialize, Deserialize)]
struct Resources {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    branch: Option<ReleasedBranch>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    worktrees: Vec<PathBuf>,
}

fn load_item(conn: &Connection, path: &Path, row: TodoRow) -> Result<LocalItem, WorkError> {
    let id = row.id.clone();
    let todo_id = TodoId::parse(&row.id).ok_or_else(|| invalid(path, "a todo identity"))?;
    let group = match (row.group_id, row.group_slug) {
        (Some(group), Some(slug)) => Some(todo_ref(path, &group, &slug)?),
        _ => None,
    };
    let claim = match (row.claim_machine, row.claim_at) {
        (Some(machine), Some(at)) => Some(Claim {
            machine: MachineId::parse(&machine).ok_or_else(|| invalid(path, "a machine id"))?,
            session: row.claim_session.map(|s| session(path, &s)).transpose()?,
            at: time(path, &at)?,
        }),
        _ => None,
    };
    let delivery = match (row.delivered_commit, row.delivered_at, row.delivered_by) {
        (Some(commit), Some(at), Some(by)) => Some(Delivery {
            commit: CommitSha::parse(&commit).map_err(|e| invalid(path, e.to_string()))?,
            at: time(path, &at)?,
            by: actor(path, &by)?,
        }),
        _ => None,
    };
    let removal = match (row.removed_at, row.removed_from) {
        (Some(at), Some(from)) => {
            let resources: Resources = match row.removal {
                Some(text) => parse_json(path, &text)?,
                None => Resources {
                    branch: None,
                    worktrees: Vec::new(),
                },
            };
            Some(Removal {
                at: time(path, &at)?,
                from: value_of(path, &from)?,
                branch: resources.branch,
                worktrees: resources.worktrees,
            })
        }
        _ => None,
    };
    let agent = row
        .agent
        .map(|agent| {
            Ok::<_, WorkError>(AgentPreference {
                agent: instrukt_core::ids::ProfileName::parse(&agent)
                    .map_err(|e| WorkError::refused(e.to_string()))?,
                tier: row.tier.map(|t| value_of::<Tier>(path, &t)).transpose()?,
            })
        })
        .transpose()?;
    let after = query(
        conn,
        path,
        "SELECT prerequisite_id, slug FROM prerequisites WHERE todo_id = ?1 ORDER BY rowid",
        &id,
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
    )?
    .into_iter()
    .map(|(prerequisite, slug)| todo_ref(path, &prerequisite, &slug))
    .collect::<Result<Vec<_>, _>>()?;
    let mut record = TodoRecord {
        schema: SCHEMA,
        id: todo_id,
        project: ProjectSlug::parse(&row.project).map_err(|e| invalid(path, e.to_string()))?,
        slug: TodoSlug::parse(&row.slug).map_err(|e| invalid(path, e.to_string()))?,
        prior_slugs: query(
            conn,
            path,
            "SELECT slug FROM todo_aliases WHERE todo_id=?1 ORDER BY rowid",
            &id,
            |r| r.get::<_, String>(0),
        )?
        .into_iter()
        .map(|slug| TodoSlug::parse(&slug).map_err(|error| invalid(path, error.to_string())))
        .collect::<Result<Vec<_>, _>>()?,
        kind: value_of::<TodoKind>(path, &row.kind)?,
        status: value_of::<TodoStatus>(path, &row.status)?,
        created: time(path, &row.created_at)?,
        created_by: actor(path, &row.created_by)?,
        human_involved: row.human_involved,
        revision: row.revision,
        sequence: row.sequence,
        artifacts: load_artifacts(conn, path, &id)?,
        stamps: load_stamps(conn, path, &id)?,
        claim,
        bindings: load_bindings(conn, path, &id)?,
        hold: None,
        hold_history: Vec::new(),
        label_actions: query(
            conn,
            path,
            "SELECT issue_number,action_id,action,login,at,outcome FROM github_label_actions WHERE todo_id=?1 AND outcome='accepted' ORDER BY action_id",
            &id,
            |r| {
                Ok(super::_model::LabelAction {
                    issue: r.get(0)?,
                    id: r.get(1)?,
                    action: r.get(2)?,
                    login: r.get(3)?,
                    at: r.get(4)?,
                    outcome: r.get(5)?,
                })
            },
        )?,
        after,
        group,
        schedule: row
            .schedule
            .map(|s| TaskSchedule::parse(&s).map_err(|e| invalid(path, e.to_string())))
            .transpose()?,
        catch_up: value_of::<CatchUp>(path, &row.catch_up)?,
        agent,
        branch: row.branch,
        delivery,
        removal,
    };
    let holds = query(
        conn,
        path,
        "SELECT source, reason, set_by, set_at, released_at, released_by
         FROM holds WHERE todo_id = ?1 ORDER BY rowid",
        &id,
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
            ))
        },
    )?;
    for (source, reason, by, at, released_at, released_by) in holds {
        let hold = HoldRecord {
            source: value_of::<HoldSource>(path, &source)?,
            reason,
            by: actor(path, &by)?,
            at: time(path, &at)?,
        };
        match (released_at, released_by) {
            (Some(released), Some(by)) => record.hold_history.push(ReleasedHold {
                hold,
                released: time(path, &released)?,
                by: actor(path, &by)?,
            }),
            _ => record.hold = Some(hold),
        }
    }
    record.number_unsequenced();
    let mut reflection = load_reflection(conn, path, &id)?;
    let refreshed: Option<String> = conn
        .query_row("SELECT refreshed_at FROM todos WHERE id=?1", [&id], |r| {
            r.get(0)
        })
        .map_err(StateDbError::sqlite(path))?;
    reflection.refreshed_at = refreshed.map(|t| time(path, &t)).transpose()?;
    Ok(LocalItem { record, reflection })
}

fn load_artifacts(conn: &Connection, path: &Path, id: &str) -> Result<Vec<Artifact>, WorkError> {
    let artifacts = query(
        conn,
        path,
        "SELECT name, address FROM artifacts WHERE todo_id = ?1 ORDER BY rowid",
        id,
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<u64>>(1)?)),
    )?;
    let versions = query(
        conn,
        path,
        "SELECT name, sequence, digest, writer, created_at, content, derived_from, grounding,
            remote
         FROM artifact_versions WHERE todo_id = ?1 ORDER BY sequence",
        id,
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, u64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, Option<u64>>(8)?,
            ))
        },
    )?;
    let mut out: Vec<Artifact> = artifacts
        .into_iter()
        .map(|(name, address)| Artifact {
            name: ArtifactName::new(&name),
            address,
            versions: Vec::new(),
        })
        .collect();
    for (name, sequence, digest, writer, created, content, derived_from, grounding, remote) in
        versions
    {
        let version = Version {
            digest: value_of::<ContentDigest>(path, &digest)?,
            writer: actor(path, &writer)?,
            created: time(path, &created)?,
            derived_from: parse_json(path, &derived_from)?,
            sequence,
            grounding: parse_json(path, &grounding)?,
            remote,
            content,
        };
        match out.iter_mut().find(|a| a.name.as_str() == name) {
            Some(artifact) => artifact.versions.push(version),
            None => {
                return Err(invalid(
                    path,
                    format!("a version of unknown artifact {name}"),
                ));
            }
        }
    }
    Ok(out)
}

fn load_stamps(conn: &Connection, path: &Path, id: &str) -> Result<Vec<StampRecord>, WorkError> {
    let rows = query(
        conn,
        path,
        "SELECT sequence, fact, at, actor, role, model, model_version, model_provenance,
            eval_fingerprint, human_involved, evidence,source_comment
         FROM stamps WHERE todo_id = ?1 ORDER BY rowid",
        id,
        |row| {
            Ok((
                row.get::<_, u64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, Option<String>>(8)?,
                row.get::<_, Option<bool>>(9)?,
                row.get::<_, String>(10)?,
                row.get::<_, Option<u64>>(11)?,
            ))
        },
    )?;
    rows.into_iter()
        .map(
            |(
                sequence,
                fact,
                at,
                by,
                role,
                model,
                model_version,
                provenance,
                eval_fingerprint,
                human_involved,
                evidence,
                source_comment,
            )| {
                Ok(StampRecord {
                    source_comment,
                    fact: join_fact(path, &fact, &evidence)?,
                    at: time(path, &at)?,
                    actor: actor(path, &by)?,
                    model: match (model, model_version) {
                        (Some(model), Some(version)) => Some(ModelIdentity { model, version }),
                        _ => None,
                    },
                    eval_fingerprint,
                    role: role
                        .map(|r| RoleName::parse(&r).map_err(|e| invalid(path, e.to_string())))
                        .transpose()?,
                    human_involved,
                    sequence,
                    model_provenance: value_of::<ModelProvenance>(path, &provenance)?,
                })
            },
        )
        .collect()
}

fn load_bindings(conn: &Connection, path: &Path, id: &str) -> Result<Vec<Binding>, WorkError> {
    query(
        conn,
        path,
        "SELECT evidence FROM session_bindings WHERE todo_id=?1 ORDER BY rowid",
        id,
        |row| row.get::<_, String>(0),
    )?
    .into_iter()
    .map(|json| serde_json::from_str(&json).map_err(|e| invalid(path, e.to_string())))
    .collect()
}

fn load_reflection(conn: &Connection, path: &Path, id: &str) -> Result<ReflectionState, WorkError> {
    let sql = StateDbError::sqlite(path);
    type Row = (ReflectionState, String, String, Option<String>);
    let row: Option<Row> = conn
        .query_row(
            "SELECT issue_number, remote_revision, pending, failure, pull_number, issue_id, etag,
                pull_outstanding, reviews, pull_closed, board, board_item, board_status,
                board_failure, comments_since, events_page, events_last, published_head,
                publication_outstanding, refusals
             FROM reflections WHERE todo_id = ?1",
            params![id],
            |row| {
                let number: Option<u64> = row.get(0)?;
                let issue_id: Option<u64> = row.get(5)?;
                Ok((
                    ReflectionState {
                        refreshed_at: None,
                        issue: number
                            .zip(issue_id)
                            .map(|(number, id)| IssueRef { number, id }),
                        remote_revision: row.get(1)?,
                        pending: row.get(2)?,
                        failure: row.get(3)?,
                        pull: row.get(4)?,
                        etag: row.get(6)?,
                        pull_outstanding: row.get(7)?,
                        reviews: Vec::new(),
                        pull_closed: row.get(9)?,
                        board: row.get(10)?,
                        board_item: row.get(11)?,
                        board_status: row.get(12)?,
                        board_failure: row.get(13)?,
                        comments_since: row.get(14)?,
                        events_page: row.get(15)?,
                        events_last: row.get(16)?,
                        published_head: None,
                        publication_outstanding: row.get(18)?,
                        refusals: Vec::new(),
                        commands_seen: Vec::new(),
                        inbound: Vec::new(),
                    },
                    row.get(8)?,
                    row.get(19)?,
                    row.get(17)?,
                ))
            },
        )
        .optional()
        .map_err(&sql)?;
    let mut reflection = match row {
        Some((mut reflection, reviews, refusals, published)) => {
            reflection.reviews = parse_json(path, &reviews)?;
            reflection.refusals = parse_json(path, &refusals)?;
            reflection.published_head = published
                .map(|head| CommitSha::parse(&head).map_err(|e| invalid(path, e)))
                .transpose()?;
            reflection
        }
        None => ReflectionState::default(),
    };
    reflection.commands_seen = query(
        conn,
        path,
        "SELECT comment_id FROM inbound_commands WHERE todo_id = ?1 ORDER BY rowid",
        id,
        |row| row.get::<_, u64>(0),
    )?;
    Ok(reflection)
}

fn query<T>(
    conn: &Connection,
    path: &Path,
    statement: &str,
    id: &str,
    map: impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
) -> Result<Vec<T>, WorkError> {
    let sql = StateDbError::sqlite(path);
    let mut prepared = conn.prepare_cached(statement).map_err(&sql)?;
    let rows = prepared
        .query_map(params![id], map)
        .map_err(&sql)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(&sql)?;
    Ok(rows)
}

/// A stamp fact as the `fact` column (its snake-case kind) and the
/// `evidence` column (the rest of its bound evidence, as JSON).
fn split_fact(fact: &StampFact) -> Result<(String, String), WorkError> {
    let mut value = serde_json::to_value(fact).map_err(|e| invalid(Path::new(""), e))?;
    let kind = value
        .as_object_mut()
        .and_then(|object| object.remove("fact"))
        .and_then(|kind| kind.as_str().map(str::to_owned))
        .ok_or_else(|| invalid(Path::new(""), "a stamp fact without its kind"))?;
    Ok((kind, value.to_string()))
}

fn join_fact(path: &Path, kind: &str, evidence: &str) -> Result<StampFact, WorkError> {
    let mut value: serde_json::Value = parse_json(path, evidence)?;
    let object = value
        .as_object_mut()
        .ok_or_else(|| invalid(path, "stamp evidence that is not an object"))?;
    object.insert("fact".to_owned(), serde_json::Value::from(kind));
    serde_json::from_value(value).map_err(|e| invalid(path, e))
}

/// An actor as the database writes it: `{"session": "<runtime>:<id>"}`,
/// `{"github": "<login>"}` or `{"person": "<name>"}`.
fn actor_text(actor: &Actor) -> String {
    let value = match actor {
        Actor::Session(session) => serde_json::json!({ "session": session.to_string() }),
        Actor::GitHub { login } => serde_json::json!({ "github": login }),
        Actor::Person { name } => serde_json::json!({ "person": name }),
    };
    value.to_string()
}

fn actor(path: &Path, text: &str) -> Result<Actor, WorkError> {
    let value: serde_json::Value = parse_json(path, text)?;
    let field = |name: &str| value.get(name).and_then(serde_json::Value::as_str);
    if let Some(bound) = field("session") {
        return Ok(Actor::Session(session(path, bound)?));
    }
    if let Some(login) = field("github") {
        return Ok(Actor::GitHub {
            login: login.to_owned(),
        });
    }
    if let Some(name) = field("person") {
        return Ok(Actor::Person {
            name: name.to_owned(),
        });
    }
    Err(invalid(path, format!("an actor `{text}`")))
}

fn schedule_text(schedule: &TaskSchedule) -> String {
    match schedule {
        TaskSchedule::At(at) => at.to_string(),
        TaskSchedule::Cron(cron) => format!("cron:{cron}"),
    }
}

fn todo_ref(path: &Path, id: &str, slug: &str) -> Result<TodoRef, WorkError> {
    Ok(TodoRef {
        id: TodoId::parse(id).ok_or_else(|| invalid(path, "a todo identity"))?,
        slug: TodoSlug::parse(slug).map_err(|e| invalid(path, e.to_string()))?,
    })
}

fn session(path: &Path, text: &str) -> Result<SessionRef, WorkError> {
    SessionRef::parse(text).map_err(|e| invalid(path, e.to_string()))
}

fn time(path: &Path, text: &str) -> Result<Timestamp, WorkError> {
    text.parse()
        .map_err(|e: jiff::Error| invalid(path, e.to_string()))
}

/// A unit-like or string-like value as its serialized text (`active`,
/// `feature`, `once`, …).
fn text_of<T: Serialize>(value: &T) -> String {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(text)) => text,
        Ok(other) => other.to_string(),
        Err(_) => String::new(),
    }
}

fn value_of<T: DeserializeOwned>(path: &Path, text: &str) -> Result<T, WorkError> {
    serde_json::from_value(serde_json::Value::from(text)).map_err(|e| invalid(path, e))
}

fn json_of<T: Serialize + ?Sized>(value: &T) -> Result<String, WorkError> {
    serde_json::to_string(value).map_err(|e| invalid(Path::new(""), e))
}

fn parse_json<T: DeserializeOwned>(path: &Path, text: &str) -> Result<T, WorkError> {
    serde_json::from_str(text).map_err(|e| invalid(path, e))
}

/// `value`, a filesystem path, as the text a row stores; a path that is
/// not UTF-8 is refused rather than stored as another path.
fn path_text<'a>(path: &Path, value: &'a Path) -> Result<&'a str, WorkError> {
    value
        .to_str()
        .ok_or_else(|| invalid(path, format!("{} is not a UTF-8 path", value.display())))
}

fn invalid(path: &Path, message: impl std::fmt::Display) -> WorkError {
    WorkError::StateDb(StateDbError::invalid(path, message.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evidence::{Grounding, TodoStatus};
    use crate::work::_model::SCHEMA;
    use instrukt_runner::interfaces::{CatchUp, TodoKind};

    fn version(sequence: u64, content: &str) -> Version {
        Version {
            digest: ContentDigest::of(content),
            writer: Actor::Person { name: "dev".into() },
            created: Timestamp::from_second(sequence as i64).unwrap(),
            derived_from: Vec::new(),
            sequence,
            grounding: Grounding::default(),
            remote: None,
            content: Some(content.to_owned()),
        }
    }

    /// A reflected todo at `revision` whose `log` holds one version per
    /// entry of `contents`.
    fn item(revision: u64, contents: &[&str]) -> LocalItem {
        LocalItem {
            record: TodoRecord {
                schema: SCHEMA,
                id: TodoId::parse(&"11".repeat(16)).unwrap(),
                project: ProjectSlug::parse("demo").unwrap(),
                slug: TodoSlug::parse("list-depth").unwrap(),
                prior_slugs: Vec::new(),
                kind: TodoKind::Feature,
                status: TodoStatus::Active,
                created: Timestamp::from_second(1).unwrap(),
                created_by: Actor::Person { name: "dev".into() },
                human_involved: None,
                revision,
                sequence: contents.len() as u64,
                artifacts: vec![Artifact {
                    name: ArtifactName::new("log"),
                    address: None,
                    versions: contents
                        .iter()
                        .enumerate()
                        .map(|(at, content)| version(at as u64 + 1, content))
                        .collect(),
                }],
                stamps: Vec::new(),
                claim: None,
                bindings: Vec::new(),
                hold: None,
                hold_history: Vec::new(),
                label_actions: Vec::new(),
                after: Vec::new(),
                group: None,
                schedule: None,
                catch_up: CatchUp::Once,
                agent: None,
                branch: None,
                delivery: None,
                removal: None,
            },
            reflection: ReflectionState {
                issue: Some(IssueRef { number: 1, id: 101 }),
                remote_revision: Some(1),
                ..ReflectionState::default()
            },
        }
    }

    /// `item` as reflection left it once GitHub showed its revision: the
    /// artifact's address and every version's remote number learned.
    fn learned(mut item: LocalItem) -> LocalItem {
        let artifact = &mut item.record.artifacts[0];
        artifact.address = Some(7);
        for version in &mut artifact.versions {
            version.remote = Some(version.sequence);
        }
        item.reflection.remote_revision = Some(item.record.revision);
        item.reflection.pending = false;
        item
    }

    #[test]
    fn a_record_another_command_changed_refuses_the_change() {
        let read = item(1, &["started"]);
        let live = item(2, &["started", "joined"]);
        assert_eq!(admission(&live, &read), Admission::Changed);
    }

    #[test]
    fn a_change_is_admitted_onto_a_row_reflection_filled_in_since_the_read() {
        let mut read = item(1, &["started"]);
        read.reflection.pending = true;
        let live = learned(read.clone());
        assert_eq!(admission(&live, &read), Admission::Admitted);
    }

    #[test]
    fn carrying_keeps_the_command_s_own_record_over_a_newer_live_one() {
        let mine = item(2, &["started", "mine"]);
        let live = learned(item(3, &["started", "mine", "theirs"]));
        let carried = carry(&live, &mine);
        assert_eq!(carried.record.revision, 2);
        assert_eq!(carried.record.artifacts[0].versions.len(), 2);
        assert_eq!(carried.reflection, live.reflection);
        assert_eq!(carried.record.artifacts[0].address, Some(7));
        assert_eq!(carried.record.artifacts[0].versions[0].remote, Some(1));
    }

    #[test]
    fn a_rebased_change_keeps_what_reflection_committed_and_is_pending() {
        let read = item(1, &["started"]);
        let live = learned(read.clone());
        let mut mine = read.clone();
        mine.record.revision = 2;
        mine.record.artifacts[0].versions.push(version(2, "joined"));
        mine.reflection.commands_seen.push(40);
        let next = rebase_local(&live, &mine);
        assert_eq!(next.record.revision, 2);
        assert_eq!(next.reflection.remote_revision, Some(1));
        assert!(next.reflection.pending);
        assert_eq!(next.reflection.commands_seen, vec![40]);
        let versions = &next.record.artifacts[0].versions;
        assert_eq!(next.record.artifacts[0].address, Some(7));
        assert_eq!(
            versions.iter().map(|v| v.remote).collect::<Vec<_>>(),
            vec![Some(1), None]
        );
    }

    #[test]
    fn a_flush_of_the_live_record_clears_pending_and_attaches_what_it_learned() {
        let mut loaded = item(2, &["started", "joined"]);
        loaded.reflection.pending = true;
        loaded.reflection.etag = Some("old".into());
        let mut live = loaded.clone();
        live.reflection.refreshed_at = Some(Timestamp::from_second(99).unwrap());
        let mut reflected = learned(loaded.clone());
        reflected.reflection.etag = None;
        let next = merge_reflection(&live, &loaded, &reflected);
        assert!(!next.reflection.pending);
        assert_eq!(next.reflection.remote_revision, Some(2));
        assert_eq!(next.reflection.etag, None);
        assert_eq!(next.reflection.refreshed_at, live.reflection.refreshed_at);
        assert_eq!(next.record, reflected.record);
    }

    #[test]
    fn a_flush_overtaken_by_a_newer_local_record_keeps_it_pending() {
        let mut loaded = item(2, &["started", "joined"]);
        loaded.reflection.pending = true;
        loaded.reflection.etag = Some("old".into());
        let mut live = item(3, &["started", "joined", "later"]);
        live.reflection = loaded.reflection.clone();
        let reflected = learned(loaded.clone());
        let next = merge_reflection(&live, &loaded, &reflected);
        assert_eq!(next.record.revision, 3);
        assert_eq!(next.record.artifacts[0].versions.len(), 3);
        assert!(next.reflection.pending);
        assert_eq!(next.reflection.etag, None);
        assert_eq!(next.reflection.remote_revision, Some(2));
        let remotes: Vec<_> = next.record.artifacts[0]
            .versions
            .iter()
            .map(|v| v.remote)
            .collect();
        assert_eq!(remotes, vec![Some(1), Some(2), None]);
    }
}
