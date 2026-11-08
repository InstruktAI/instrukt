//! The local work record in the state database's work tables
//! (`state-database`, "Work tables"): each local item is one `todos` row
//! with its prerequisites, artifacts and versions, stamps, holds, session
//! bindings, reflection state and answered inbound commands. Every write
//! runs under the per-computer writer lock the store holds for as long as
//! it is open, and one accepted change commits in one transaction.

use std::path::{Path, PathBuf};

use instrukt_core::home::Home;
use instrukt_core::ids::{CommitSha, ProjectSlug, RoleName, SessionRef, TodoSlug};
use instrukt_core::machine::MachineId;
use instrukt_core::state_db::rusqlite::{self, Connection, OptionalExtension, Transaction, params};
use instrukt_core::state_db::{StateDb, StateDbError, StateRead};
use instrukt_runner::interfaces::{AgentPreference, CatchUp, TaskSchedule, Tier, TodoKind};
use jiff::Timestamp;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use super::_error::WorkError;
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
    /// The comment carrying the committed revision this computer last
    /// wrote or read: the head of the issue's revision chain then.
    pub record_comment: Option<u64>,
    /// The issue's `ETag` at the last read, for conditional pulls: a new
    /// revision comment moves it.
    pub etag: Option<String>,
    /// The confirmed remote revision: the record revision GitHub holds.
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
    /// A newer remote revision met a pending local change.
    pub conflicted: bool,
    /// The linked pull request's number.
    pub pull: Option<u64>,
    /// How many build-review verdicts the pull request carries.
    pub reviews_reflected: usize,
    /// Whether delivery closed the pull request.
    pub pull_closed: bool,
    /// The last reflection failure that left the item pending.
    pub failure: Option<String>,
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

/// One project's local items, open for writing: the writer lock is held
/// while the store lives.
pub(crate) struct Store {
    db: StateDb,
    project: ProjectSlug,
}

impl Store {
    /// Takes the writer lock and opens the state database for `project`.
    pub fn open(home: &Home, project: &ProjectSlug) -> Result<Self, WorkError> {
        Ok(Self {
            db: StateDb::open(home)?,
            project: project.clone(),
        })
    }

    pub fn path(&self) -> &Path {
        self.db.path()
    }

    /// Retain an answered command and its fixed confirmation atomically.
    pub fn save_with_reply(
        &mut self,
        item: &LocalItem,
        comment: u64,
        answer: &str,
    ) -> Result<(), WorkError> {
        let path = self.db.path().to_path_buf();
        self.db.write(|tx| {
            save_in(tx, &path, item)?;
            retain_reply(tx, &path, item.record.id.as_str(), comment, answer)
        })
    }
    pub fn pending_replies(&self, todo: &str) -> Result<Vec<(u64, String)>, WorkError> {
        query(
            self.db.conn(),
            self.db.path(),
            "SELECT comment_id,reply FROM inbound_commands WHERE todo_id=?1 AND reply_pending=1 ORDER BY comment_id",
            todo,
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
    }
    pub fn reply_sent(&mut self, todo: &str, comment: u64) -> Result<(), WorkError> {
        let path = self.db.path().to_path_buf();
        self.db.write(|tx| {
            tx.execute(
                "UPDATE inbound_commands SET reply_pending=0 WHERE todo_id=?1 AND comment_id=?2",
                params![todo, comment],
            )
            .map_err(StateDbError::sqlite(&path))?;
            Ok(())
        })
    }

    pub fn ignore_label(
        &mut self,
        issue: u64,
        action: u64,
        todo: &str,
        kind: &str,
        login: &str,
        at: &str,
    ) -> Result<(), WorkError> {
        let path = self.db.path().to_path_buf();
        let project = &self.project;
        self.db.write(|tx| {tx.execute("INSERT INTO github_label_actions(project,issue_number,action_id,todo_id,action,login,at,outcome) VALUES (?1,?2,?3,?4,?5,?6,?7,'ignored') ON CONFLICT DO NOTHING",params![project.as_str(),issue,action,todo,kind,login,at]).map_err(StateDbError::sqlite(&path))?;Ok(())})
    }
    pub fn label_seen(&self, issue: u64, action: u64) -> Result<bool, WorkError> {
        self.db.conn().query_row("SELECT EXISTS(SELECT 1 FROM github_label_actions WHERE project=?1 AND issue_number=?2 AND action_id=?3)",params![self.project.as_str(),issue,action],|r|r.get(0)).map_err(StateDbError::sqlite(self.db.path())).map_err(Into::into)
    }
    /// Every local item of the project, in identity order.
    pub fn items(&self) -> Result<Vec<LocalItem>, WorkError> {
        load_items(self.db.conn(), self.db.path(), &self.project)
    }

    /// Writes `item` in a transaction of its own.
    pub fn save(&mut self, item: &LocalItem) -> Result<(), WorkError> {
        let path = self.db.path().to_path_buf();
        self.db.write(|tx| save_in(tx, &path, item))
    }

    /// Removes `id` and every row that belongs to it.
    pub fn delete(&mut self, id: &TodoId) -> Result<(), WorkError> {
        let path = self.db.path().to_path_buf();
        self.db.write(|tx| delete_in(tx, &path, id))
    }

    /// Begins one transaction for a change spanning several records.
    pub fn transaction(&mut self) -> Result<Transaction<'_>, WorkError> {
        Ok(self.db.transaction()?)
    }

    pub fn provider(&self) -> Result<ProviderState, WorkError> {
        let path = self.db.path();
        let until: Option<String> = self
            .db
            .conn()
            .query_row(
                "SELECT until FROM provider_deferrals WHERE project = ?1",
                params![self.project.as_str()],
                |row| row.get(0),
            )
            .optional()
            .map_err(StateDbError::sqlite(path))?;
        Ok(ProviderState {
            deferred_until: until.map(|text| time(path, &text)).transpose()?,
        })
    }

    pub fn save_provider(&mut self, state: &ProviderState) -> Result<(), WorkError> {
        let path = self.db.path().to_path_buf();
        let project = self.project.clone();
        self.db.write(|tx| {
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
            sql.map_err(StateDbError::sqlite(&path))?;
            Ok(())
        })
    }
}

/// The last known copy of a todo that is not a local item here, as the
/// latest pickup listing or refresh read it (`state-database`,
/// `listing_copies`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ListingCopy {
    pub issue_number: u64,
    /// The reflected record as read, without artifact contents.
    pub record: TodoRecord,
    /// When it was read: the copy's age.
    pub copied_at: Timestamp,
}

impl Store {
    /// The project's listing copies, oldest todo first.
    pub fn copies(&self) -> Result<Vec<ListingCopy>, WorkError> {
        load_copies(self.db.conn(), self.db.path(), &self.project)
    }

    /// Replaces the project's listing copies with `copies`: what one
    /// pickup listing read.
    pub fn replace_copies(&mut self, copies: &[ListingCopy]) -> Result<(), WorkError> {
        let path = self.db.path().to_path_buf();
        let project = self.project.clone();
        self.db.write(|tx| {
            tx.execute(
                "DELETE FROM listing_copies WHERE project = ?1",
                params![project.as_str()],
            )
            .map_err(StateDbError::sqlite(&path))?;
            for copy in copies {
                save_copy_in(tx, &path, &project, copy)?;
            }
            Ok(())
        })
    }

    /// Replaces the one copy of `copy`'s issue: what a refresh of a todo
    /// that is not a local item read.
    pub fn save_copy(&mut self, copy: &ListingCopy) -> Result<(), WorkError> {
        let path = self.db.path().to_path_buf();
        let project = self.project.clone();
        self.db.write(|tx| save_copy_in(tx, &path, &project, copy))
    }

    /// Removes the copy of issue `number`: its todo became a local item.
    pub fn drop_copy(&mut self, number: u64) -> Result<(), WorkError> {
        let path = self.db.path().to_path_buf();
        let project = self.project.clone();
        self.db.write(|tx| {
            tx.execute(
                "DELETE FROM listing_copies WHERE project = ?1 AND issue_number = ?2",
                params![project.as_str(), number],
            )
            .map_err(StateDbError::sqlite(&path))?;
            Ok(())
        })
    }
}

pub(crate) fn retain_reply(
    tx: &Connection,
    path: &Path,
    todo: &str,
    comment: u64,
    answer: &str,
) -> Result<(), WorkError> {
    tx.execute("INSERT INTO inbound_commands(todo_id,comment_id,reply,reply_pending) VALUES (?1,?2,?3,1) ON CONFLICT(todo_id,comment_id) DO UPDATE SET reply=COALESCE(inbound_commands.reply,excluded.reply),reply_pending=CASE WHEN inbound_commands.reply IS NULL THEN 1 ELSE inbound_commands.reply_pending END",params![todo,comment,answer]).map_err(StateDbError::sqlite(path))?;
    Ok(())
}

fn save_copy_in(
    tx: &Connection,
    path: &Path,
    project: &ProjectSlug,
    copy: &ListingCopy,
) -> Result<(), WorkError> {
    let r = &copy.record;
    tx.execute(
        "INSERT INTO listing_copies
             (project, issue_number, slug, kind, status, remote_revision, claim_machine, record, copied_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT (project, issue_number) DO UPDATE SET
             slug = excluded.slug, kind = excluded.kind, status = excluded.status,
             remote_revision = excluded.remote_revision, claim_machine = excluded.claim_machine,
             record = excluded.record, copied_at = excluded.copied_at",
        params![
            project.as_str(),
            copy.issue_number,
            r.slug.as_str(),
            text_of(&r.kind),
            text_of(&r.status),
            r.revision,
            r.claim.as_ref().map(|c| c.machine.to_string()),
            json_of(&r.without_contents())?,
            copy.copied_at.to_string(),
        ],
    )
    .map_err(StateDbError::sqlite(path))?;
    Ok(())
}

fn load_copies(
    conn: &Connection,
    path: &Path,
    project: &ProjectSlug,
) -> Result<Vec<ListingCopy>, WorkError> {
    let sql = StateDbError::sqlite(path);
    let mut statement = conn
        .prepare(
            "SELECT issue_number, record, copied_at FROM listing_copies
             WHERE project = ?1 ORDER BY issue_number",
        )
        .map_err(&sql)?;
    let rows = statement
        .query_map(params![project.as_str()], |row| {
            Ok((
                row.get::<_, u64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(&sql)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(&sql)?;
    rows.into_iter()
        .map(|(issue_number, record, copied_at)| {
            Ok(ListingCopy {
                issue_number,
                record: parse_json(path, &record)?,
                copied_at: time(path, &copied_at)?,
            })
        })
        .collect()
}

/// Every local item of `project`, read without the writer lock; no
/// database yet is no items.
pub(crate) fn read_items(home: &Home, project: &ProjectSlug) -> Result<Vec<LocalItem>, WorkError> {
    match StateRead::open(home)? {
        Some(read) => load_items(read.conn(), read.path(), project),
        None => Ok(Vec::new()),
    }
}

/// Writes every row of `item` inside `tx`: the `todos` row first, then the
/// rows that belong to it. Artifact versions are retained and never
/// rewritten; only what reflection learns of them changes.
pub(crate) fn save_in(tx: &Connection, path: &Path, item: &LocalItem) -> Result<(), WorkError> {
    let r = &item.record;
    let id = r.id.as_str();
    let sql = StateDbError::sqlite(path);
    // A todo that is a local item has no listing copy.
    if let Some(issue) = item.reflection.issue {
        tx.execute(
            "DELETE FROM listing_copies WHERE project = ?1 AND issue_number = ?2",
            params![r.project.as_str(), issue.number],
        )
        .map_err(&sql)?;
    }
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
            params![id,binding.role.as_str(),a.number,a.runtime.map(|r|r.name()).or_else(||binding.session.as_ref().map(|s|s.runtime.as_str())),a.semantic_key,binding.session.as_ref().map(ToString::to_string),binding.worktree.as_ref().map(|p|p.to_string_lossy()),a.branch,a.base_head.as_ref().map(ToString::to_string),a.completion_sequence,a.completed_head.as_ref().map(ToString::to_string),a.failure_reason,a.failure_head.as_ref().map(ToString::to_string),a.failure_workspace,a.failed_at.map(|t|t.to_string()),a.released_at.map(|t|t.to_string()),serde_json::to_string(binding).map_err(|e|invalid(path,e.to_string()))?,a.native_assignment.as_ref().map(|a|a.title.as_str())],
        ).map_err(&sql)?;
    }
    let f = &item.reflection;
    tx.execute(
        "INSERT OR REPLACE INTO reflections (todo_id, issue_number, remote_revision, pending,
            conflicted, failure, pull_number, issue_id, record_comment, etag, pull_outstanding,
            reviews_reflected, pull_closed, board, board_item, board_status, board_failure)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
        params![
            id,
            f.issue.map(|i| i.number),
            f.remote_revision,
            f.pending,
            f.conflicted,
            f.failure,
            f.pull,
            f.issue.map(|i| i.id),
            f.record_comment,
            f.etag,
            f.pull_outstanding,
            f.reviews_reflected as u64,
            f.pull_closed,
            f.board,
            f.board_item,
            f.board_status,
            f.board_failure,
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
pub(crate) fn delete_in(tx: &Connection, path: &Path, id: &TodoId) -> Result<(), WorkError> {
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
    let sql = StateDbError::sqlite(path);
    let mut statement = conn
        .prepare(
            "SELECT id, project, slug, kind, status, created_at, created_by, human_involved,
                revision, group_id, group_slug, schedule, catch_up, agent, tier,
                claim_machine, claim_session, claim_at, branch, delivered_commit, delivered_at,
                delivered_by, removed_at, removed_from, removal, sequence
             FROM todos WHERE project = ?1 ORDER BY id",
        )
        .map_err(&sql)?;
    let rows = statement
        .query_map(params![project.as_str()], |row| {
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
    let mut reflection = conn
        .query_row(
            "SELECT issue_number, remote_revision, pending, conflicted, failure, pull_number,
                issue_id, record_comment, etag, pull_outstanding, reviews_reflected, pull_closed,
                board, board_item, board_status, board_failure
             FROM reflections WHERE todo_id = ?1",
            params![id],
            |row| {
                let number: Option<u64> = row.get(0)?;
                let issue_id: Option<u64> = row.get(6)?;
                Ok(ReflectionState {
                    refreshed_at: None,
                    issue: number
                        .zip(issue_id)
                        .map(|(number, id)| IssueRef { number, id }),
                    remote_revision: row.get(1)?,
                    pending: row.get(2)?,
                    conflicted: row.get(3)?,
                    failure: row.get(4)?,
                    pull: row.get(5)?,
                    record_comment: row.get(7)?,
                    etag: row.get(8)?,
                    pull_outstanding: row.get(9)?,
                    reviews_reflected: row.get::<_, u64>(10)? as usize,
                    pull_closed: row.get(11)?,
                    board: row.get(12)?,
                    board_item: row.get(13)?,
                    board_status: row.get(14)?,
                    board_failure: row.get(15)?,
                    commands_seen: Vec::new(),
                    inbound: Vec::new(),
                })
            },
        )
        .optional()
        .map_err(&sql)?
        .unwrap_or_default();
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

fn invalid(path: &Path, message: impl std::fmt::Display) -> WorkError {
    WorkError::StateDb(StateDbError::invalid(path, message.to_string()))
}
