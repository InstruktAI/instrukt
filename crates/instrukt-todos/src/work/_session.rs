//! One command's local view of a project's work record: the loaded local
//! items with the rows they were read from, and the shared local steps
//! every operation composes — entering an item and committing a change to
//! SQLite. It holds no lock and never contacts GitHub or touches
//! `reflection.lock`: each local write takes the writer lock for its
//! transaction, and current-schema local reads take no lock at all.

use instrukt_core::config::Configuration;
use instrukt_core::git;
use instrukt_core::github;
use instrukt_core::ids::{SessionRef, TodoSlug};
use instrukt_core::machine::{self, MachineId};
use instrukt_core::state_db::rusqlite::Transaction;
use instrukt_runner::events::{Event, EventLog};
use instrukt_runner::interfaces::ProjectRef;
use instrukt_runner::sessions::Sessions;
use jiff::{SignedDuration, Timestamp};

use super::_envelope::status_name as status_word;
use super::_error::WorkError;
use super::_model::{TodoAddress, TodoId, TodoRecord, TodoRef};
use super::_reflect;
use super::_store::{self, LocalItem, Refusal, Store};
use super::Work;
use crate::evidence::{Actor, ModelIdentity, ModelProvenance, TodoStatus};

/// The fixed recovery window of withdrawn work.
pub(crate) const RECOVERY_WINDOW: SignedDuration = SignedDuration::from_hours(7 * 24);

/// Why reflection is pending.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PendingReason {
    /// The machine has no usable App credentials.
    NoCredentials,
    Unreachable,
    /// GitHub deferred requests, until the time it named.
    RateLimited {
        until: Option<Timestamp>,
    },
    /// GitHub refused the token.
    Unauthorized,
    /// A request failed; the message says how.
    Failed(String),
}

/// Where an item's reflection stands, read from the local state database.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReflectionStatus {
    Reflected,
    Pending(PendingReason),
}

pub(crate) struct Session<'w> {
    pub(super) work: &'w Work,
    pub project: ProjectRef,
    pub config: Configuration,
    pub(super) store: Store,
    pub items: Vec<LocalItem>,
    /// Each stored item's row as this session last read or wrote it: the
    /// base every commit of the item is judged against.
    pub(super) bases: std::collections::BTreeMap<TodoId, LocalItem>,
    /// The todo the command acts on: the first one it entered. A refusal
    /// of a change that also writes related todos sends the caller back to
    /// it.
    target: Option<TodoId>,
    pub now: Timestamp,
    /// The project's default branch.
    pub base: String,
    /// Local items whose recovery window expired when the session opened.
    pub purged: Vec<TodoSlug>,
    /// Documentation files this command rewrote in the working tree.
    pub docs_changed: Vec<std::path::PathBuf>,
    /// A group parent's pending documentation owed to its first child.
    pub pending_transfer: Option<(TodoSlug, TodoSlug)>,
}

impl<'w> Session<'w> {
    /// Loads the project's local record for a writing command or retention
    /// purge: checks the code host locally without contacting GitHub, migrates
    /// an older database once under the writer lock, and purges withdrawn work
    /// past its recovery window.
    pub fn open(work: &'w Work, project: &ProjectRef) -> Result<Self, WorkError> {
        Self::open_parts(work, project, true).map(|(session, _, _)| session)
    }

    /// Loads the project's local record for a read-only command or port:
    /// checks the code host locally without contacting GitHub, skips purging
    /// expired withdrawn records (filtering them out in memory instead), and
    /// acquires no lock on a current-schema database (`state-database`,
    /// "Writing").
    pub fn open_read(work: &'w Work, project: &ProjectRef) -> Result<Self, WorkError> {
        Self::open_parts(work, project, false).map(|(session, _, _)| session)
    }

    pub(super) fn open_parts(
        work: &'w Work,
        project: &ProjectRef,
        purge: bool,
    ) -> Result<(Self, String, String), WorkError> {
        let store = Store::new(work.home(), &project.slug);
        let config = Configuration::load(work.home(), Some(&project.root))?;
        // The address the token is sent to comes from the machine's own
        // layer, never from a repository's.
        let api_url = github::api_url(work.home())?;
        let repository = match github::code_host(&config, &api_url, &project.root)? {
            github::CodeHost::GitHub { repository } => repository,
            github::CodeHost::Elsewhere { reason } => {
                return Err(WorkError::CodeNotOnGitHub { reason });
            }
        };
        let now = work.now();
        let mut items = store.items()?;
        if !purge {
            items.retain(|it| !past_recovery(&it.record, now));
        }
        let base = github::default_branch(&project.root)?;
        let bases = items
            .iter()
            .map(|item| (item.record.id.clone(), item.clone()))
            .collect();
        let mut session = Self {
            work,
            project: project.clone(),
            config,
            store,
            items,
            bases,
            target: None,
            now,
            base,
            purged: Vec::new(),
            docs_changed: Vec::new(),
            pending_transfer: None,
        };
        if purge {
            session.purge()?;
        }
        Ok((session, api_url, repository))
    }

    /// Reloads current local items from SQLite without purging, refreshing
    /// `items` and `bases` for a runner reflection phase.
    pub(super) fn reload(&mut self) -> Result<(), WorkError> {
        let now = self.now;
        let items: Vec<LocalItem> = self
            .store
            .items()?
            .into_iter()
            .filter(|it| !past_recovery(&it.record, now))
            .collect();
        self.bases = items
            .iter()
            .map(|item| (item.record.id.clone(), item.clone()))
            .collect();
        self.items = items;
        Ok(())
    }

    pub fn home(&self) -> &instrukt_core::home::Home {
        self.work.home()
    }

    pub fn machine(&self) -> Result<MachineId, WorkError> {
        Ok(machine::machine_id(self.work.home())?)
    }

    /// The session the caller runs in: the active runtime's, when one is
    /// active.
    pub fn caller_session(&self) -> Result<Option<SessionRef>, WorkError> {
        Ok(self.config.caller(self.work.env())?)
    }

    /// The model a stamp by `actor` records: for a session, what its
    /// runtime's `model` template reports, else explicitly unknown;
    /// outside a session, none.
    pub fn model_of(&self, actor: &Actor) -> (Option<ModelIdentity>, ModelProvenance) {
        let Actor::Session(session) = actor else {
            return (None, ModelProvenance::OutsideSession);
        };
        match Sessions::new(
            self.work.home(),
            &self.config,
            self.work.env(),
            self.work.runtimes(),
        )
        .model(session)
        {
            Some((model, version)) => (
                Some(ModelIdentity { model, version }),
                ModelProvenance::Reported,
            ),
            None => (None, ModelProvenance::Unknown),
        }
    }

    /// Who acts: the caller's session, or outside one the person git names.
    /// No identity is guessed.
    pub fn actor(&self) -> Result<Actor, WorkError> {
        if let Some(session) = self.caller_session()? {
            return Ok(Actor::Session(session));
        }
        match git::run(&self.project.root, ["config", "user.name"]) {
            Ok(name) if !name.trim().is_empty() => Ok(Actor::Person {
                name: name.trim().to_owned(),
            }),
            Ok(_) | Err(git::GitError::Failed { .. }) => Err(WorkError::refused(
                "cannot name who acts: no agent session is active and git has no `user.name`",
            )),
            Err(err) => Err(err.into()),
        }
    }

    /// The local item addressed by `slug`: the live one, or with
    /// `retained`, else the latest delivered or removed one.
    pub fn find(&self, slug: &TodoSlug, retained: bool) -> Option<usize> {
        find_in(&self.items, slug, retained)
    }

    pub fn find_id(&self, id: &TodoId) -> Option<usize> {
        self.items.iter().position(|it| &it.record.id == id)
    }

    /// The local item `slug` addresses. A todo this computer's record does
    /// not hold is unknown. Reads nothing from GitHub.
    pub fn enter(&mut self, slug: &TodoSlug) -> Result<usize, WorkError> {
        let Some(index) = self.find(slug, false) else {
            return Err(self.absent(slug));
        };
        if self.target.is_none() {
            self.target = Some(self.items[index].record.id.clone());
        }
        Ok(index)
    }

    /// Like [`Session::enter`] for a reference by identity.
    pub fn enter_ref(&mut self, target: &TodoRef) -> Result<usize, WorkError> {
        match self.find_id(&target.id) {
            Some(index) => Ok(index),
            None => self.enter(&target.slug),
        }
    }

    /// Whether item `index` carries a change of this session not yet
    /// committed: its record differs from the row it was read from, or it
    /// has no row yet.
    pub(super) fn uncommitted(&self, index: usize) -> bool {
        let item = &self.items[index];
        self.bases
            .get(&item.record.id)
            .is_none_or(|base| base.record != item.record)
    }

    /// Makes `row`, read or written just now, item `index` and its base.
    pub(super) fn take_row(&mut self, index: usize, row: LocalItem) {
        self.bases.insert(row.record.id.clone(), row.clone());
        self.items[index] = row;
    }

    /// Applies `amend` to item `index`'s stored reflection state and takes
    /// the row it leaves.
    pub(super) fn amend(
        &mut self,
        index: usize,
        amend: impl Fn(&mut _store::ReflectionState),
    ) -> Result<(), WorkError> {
        amend(&mut self.items[index].reflection);
        let id = self.items[index].record.id.clone();
        let row = self.store.amend_reflection(&id, &amend)?;
        self.learned(row);
        Ok(())
    }

    /// Runs `change` against the record set, then commits every record it
    /// changed: a new revision, pending reflection, saved. A grouping's
    /// documentation transfer (`pending_transfer`) is the last step, once
    /// every check has passed and every record is written. Everything
    /// commits in one transaction; a refused or failed change — a transfer
    /// included — leaves every record and the documentation as they were.
    pub fn commit<T>(
        &mut self,
        what: &'static str,
        change: impl FnOnce(&mut Self) -> Result<T, WorkError>,
    ) -> Result<T, WorkError> {
        self.commit_with_events(what, None, change)
    }

    /// [`Session::commit`], appending `events` to the runner's event log in
    /// the same transaction (`state-database`, "Writing").
    pub fn commit_with_events<T>(
        &mut self,
        what: &'static str,
        events: Option<(&dyn EventLog, &[Event])>,
        change: impl FnOnce(&mut Self) -> Result<T, WorkError>,
    ) -> Result<T, WorkError> {
        self.commit_with(what, change, |tx| {
            if let Some((log, events)) = events {
                for event in events {
                    let appended = log.append(tx, event)?;
                    tracing::info!(
                        event = "work_event_logged",
                        name = event.name(),
                        project = %event.project(),
                        appended = ?appended,
                        "an event committed with the work record change"
                    );
                }
            }
            Ok(())
        })
    }

    /// Commits a work change and its related runner rows together. The
    /// runner owns the additional writes; work owns their transaction.
    /// `change` runs before the transaction, holding no lock.
    pub fn commit_with<T>(
        &mut self,
        what: &'static str,
        change: impl FnOnce(&mut Self) -> Result<T, WorkError>,
        write_related: impl FnOnce(&Transaction<'_>) -> Result<(), WorkError>,
    ) -> Result<T, WorkError> {
        let before: Vec<LocalItem> = self.items.clone();
        let bases_before = self.bases.clone();
        let value = match change(self) {
            Ok(value) => value,
            Err(err) => {
                self.restore(before, &bases_before);
                return Err(err);
            }
        };
        let mut changed = Vec::new();
        for index in 0..self.items.len() {
            let differs = before
                .get(index)
                .is_none_or(|prior| prior.record != self.items[index].record);
            if differs {
                let item = &mut self.items[index];
                item.record.revision += 1;
                item.reflection.pending = true;
                changed.push(index);
            }
        }
        let transfer = self.pending_transfer.take();
        let written = self.write_change(&changed, write_related, transfer.as_ref());
        let docs = match written {
            Ok(docs) => docs,
            Err(err) => {
                self.restore(before, &bases_before);
                return Err(err);
            }
        };
        self.docs_changed.extend(docs);
        for &index in &changed {
            let item = &self.items[index];
            tracing::info!(
                event = "work_recorded",
                change = what,
                project = %item.record.project,
                todo = %item.record.slug,
                id = %item.record.id,
                revision = item.record.revision,
                "work record changed"
            );
        }
        Ok(value)
    }

    /// Writes the changed items, related runner rows and documentation
    /// transfer in one transaction; returns the documentation files the transfer
    /// rewrote. Each item commits against its live row, re-read inside the
    /// transaction (`_store::commit_local`). Nothing commits unless
    /// everything succeeded: documentation moves only once every row is
    /// written, and a failing transfer writes nothing of its own. A change
    /// that moves waiting edges is refused whole when the graph the
    /// transaction holds would wait on itself through them.
    fn write_change(
        &mut self,
        changed: &[usize],
        write_related: impl FnOnce(&Transaction<'_>) -> Result<(), WorkError>,
        transfer: Option<&(TodoSlug, TodoSlug)>,
    ) -> Result<Vec<std::path::PathBuf>, WorkError> {
        let others: Vec<TodoId> = changed
            .iter()
            .map(|&index| self.items[index].record.id.clone())
            .collect();
        // The todos whose waiting edges this change moves: prerequisites, a
        // group, or becoming or ceasing to be a holder, which waits on its
        // children (`dependencies-and-holds`).
        let rewired: Vec<TodoId> = changed
            .iter()
            .map(|&index| &self.items[index].record)
            .filter(|mine| match self.bases.get(&mine.id) {
                Some(base) => {
                    base.record.after != mine.after
                        || base.record.group != mine.group
                        || (base.record.status == TodoStatus::StructuralHolder)
                            != (mine.status == TodoStatus::StructuralHolder)
                }
                None => !mine.after.is_empty() || mine.group.is_some(),
            })
            .map(|mine| mine.id.clone())
            .collect();
        // The references the change newly records, as it writes them: the
        // slug on each new prerequisite edge or group, with the status this
        // command judged the target by (`None` for a todo it holds no record
        // of). What it records — the slug on an edge, an iceboxed
        // prerequisite's warning — follows from them, so a target another
        // command renamed, parked, released or withdrawn meanwhile refuses
        // the change whole. The todos the change itself writes are judged by
        // their own commit.
        let known = self.known_records();
        let mut referenced: Vec<(TodoId, TodoSlug, Option<TodoStatus>)> = Vec::new();
        for &index in changed {
            let record = &self.items[index].record;
            let base = self.bases.get(&record.id).map(|base| &base.record);
            for target in record.after.iter().chain(record.group.iter()) {
                let recorded = base.is_some_and(|base| {
                    base.after
                        .iter()
                        .chain(base.group.iter())
                        .any(|r| r.id == target.id)
                });
                if recorded
                    || others.contains(&target.id)
                    || referenced.iter().any(|(id, _, _)| id == &target.id)
                {
                    continue;
                }
                let status = known.iter().find(|r| r.id == target.id).map(|r| r.status);
                referenced.push((target.id.clone(), target.slug.clone(), status));
            }
        }
        // The todos the change renames: every local row that refers to one
        // must be a row the change rewrites, so a reference another command
        // added meanwhile never keeps the old name.
        let renamed: Vec<(TodoId, TodoSlug)> = changed
            .iter()
            .map(|&index| &self.items[index].record)
            .filter_map(|mine| {
                let base = self.bases.get(&mine.id)?;
                (base.record.slug != mine.slug).then(|| (mine.id.clone(), base.record.slug.clone()))
            })
            .collect();
        // The sessions the change newly records on bindings, relative to
        // each row as this command read it.
        let recorded: Vec<(&SessionRef, &str)> = changed
            .iter()
            .flat_map(|&index| {
                let mine = &self.items[index].record;
                let base = self
                    .bases
                    .get(&mine.id)
                    .map(|base| base.record.bindings.as_slice());
                super::_assignment::newly_recorded(&mine.bindings, base)
            })
            .collect();
        let result = self.store.write(|tx, path, project| {
            let mut rows = Vec::new();
            for &index in changed {
                let mine = &self.items[index];
                let base = self.bases.get(&mine.record.id);
                match _store::commit_local(tx, path, project, base, mine, &others)? {
                    Ok(row) => rows.push((index, row)),
                    Err(Refusal::Changed(now)) => {
                        return Err(self.changed_refusal(mine, base, now));
                    }
                    Err(Refusal::Taken) => {
                        return Err(WorkError::refused(format!(
                            "a live todo is already named `{}`; nothing of this change was written",
                            mine.record.slug
                        )));
                    }
                    Err(Refusal::Withdrawing) => {
                        let read = base.map_or(&mine.record.slug, |base| &base.record.slug);
                        return Err(withdrawing_refusal(read));
                    }
                }
            }
            // A session the change records on a binding must still resolve
            // to one assignment, judged against every binding this
            // transaction holds (`roles-and-capabilities`).
            super::_assignment::admit_recorded(tx, path, &recorded)?;
            let ids: Vec<TodoId> = referenced.iter().map(|(id, _, _)| id.clone()).collect();
            let now = _store::referenced(tx, path, project, &ids)?;
            for (id, slug, status) in &referenced {
                let stands = match (now.get(id), status) {
                    (Some((live, _)), None) => live == slug,
                    (Some((live, held)), Some(read)) => live == slug && held == read,
                    (None, None) => true,
                    (None, Some(_)) => false,
                };
                if !stands {
                    let renamed = now
                        .get(id)
                        .map(|(now, _)| now)
                        .filter(|now| *now != slug)
                        .map_or_else(String::new, |now| format!(" and is now named `{now}`"));
                    return Err(WorkError::refused(format!(
                        "`{slug}`, which this change refers to, changed while this command ran{renamed}; nothing of it was written. {}: it acts on the todos' current records",
                        self.door()
                    )));
                }
            }
            for (id, read) in &renamed {
                for (referrer, slug) in _store::referrers(tx, path, project, id)? {
                    if !others.contains(&referrer) {
                        return Err(WorkError::refused(format!(
                            "`{slug}` came to refer to `{read}` while this command renamed it; nothing of it was written. {}: it rewrites every reference to the todo",
                            self.door()
                        )));
                    }
                }
            }
            // The change was judged against the graph as this command read
            // it; what commits is the graph this transaction holds, with
            // every edge and membership other commands committed since.
            if !rewired.is_empty()
                && let Some(cycle) = _store::cycle_in(tx, path, project, &rewired)?
            {
                let names: Vec<String> = cycle.iter().map(ToString::to_string).collect();
                return Err(WorkError::refused(format!(
                    "this change makes `{}` wait on itself: {}; nothing of it was written",
                    names[0],
                    names.join(" → ")
                )));
            }
            _store::fleet_record(
                tx,
                path,
                project,
                self.work.home(),
                self.now,
                rows.iter().map(|(_, row)| {
                    let before = self.bases.get(&row.record.id).map(|base| &base.record);
                    (before, &row.record)
                }),
            )?;
            write_related(tx)?;
            let docs = match transfer {
                Some((parent, child)) => {
                    super::_ops::transfer_documentation(&self.project, parent, child)?
                }
                None => Vec::new(),
            };
            Ok((rows, docs))
        });
        let (rows, docs) = result?;
        for (index, row) in rows {
            let inbound = std::mem::take(&mut self.items[index].reflection.inbound);
            self.take_row(index, row);
            self.items[index].reflection.inbound = inbound;
        }
        Ok(docs)
    }

    /// Item `index`'s row as the state database holds it now, `None` when
    /// it is gone from the project, whatever another command or reflection
    /// committed since this command last read it. Read in the same snapshot,
    /// whether the todo's withdrawal intent is open (`withdrawal_intents`).
    pub fn confirmed(&self, index: usize) -> Result<Option<(LocalItem, bool)>, WorkError> {
        self.store.item_withdrawing(&self.items[index].record.id)
    }

    /// Refuses acting on item `index` while its withdrawal is recorded and
    /// not ended (`withdrawal_intents`): its working resources may be going.
    pub fn refuse_withdrawing(&self, index: usize) -> Result<(), WorkError> {
        let record = &self.items[index].record;
        if self.store.withdrawal_intent(&record.id)?.is_some() {
            return Err(withdrawing_refusal(&record.slug));
        }
        Ok(())
    }

    /// Item `index`'s row as this command last read or wrote it, `None`
    /// for an item it has not stored.
    pub fn base(&self, index: usize) -> Option<LocalItem> {
        self.bases.get(&self.items[index].record.id).cloned()
    }

    /// The error for `refusal`, met by a change of item `index` that this
    /// command committed outside [`Session::commit`].
    pub fn refusal(&mut self, index: usize, refusal: Refusal) -> WorkError {
        let mine = self.items[index].clone();
        let base = self.bases.get(&mine.record.id).cloned();
        match refusal {
            Refusal::Changed(now) => self.changed_refusal(&mine, base.as_ref(), now),
            Refusal::Taken => WorkError::refused(format!(
                "a live todo is already named `{}`; nothing of this change was written",
                mine.record.slug
            )),
            Refusal::Withdrawing => {
                withdrawing_refusal(base.as_ref().map_or(&mine.record.slug, |b| &b.record.slug))
            }
        }
    }

    /// Makes `row`, a change of item `index` this command committed outside
    /// [`Session::commit`], the item and its base, and logs it as `what`.
    pub fn take_written(&mut self, index: usize, row: LocalItem, what: &'static str) {
        let inbound = std::mem::take(&mut self.items[index].reflection.inbound);
        self.take_row(index, row);
        self.items[index].reflection.inbound = inbound;
        let item = &self.items[index];
        tracing::info!(
            event = "work_recorded",
            change = what,
            project = %item.record.project,
            todo = %item.record.slug,
            id = %item.record.id,
            revision = item.record.revision,
            "work record changed"
        );
    }

    /// The refusal of a change whose write found the live row of `mine`,
    /// read as `base`, changed by another command; `now` is the slug the
    /// live row carries, `None` when it is gone. The todo is named as read,
    /// since a rename this change makes was never written, and by the slug
    /// it carries now when another command renamed it. The door is the
    /// command run again on its target: under the target's current name
    /// when the changed row is the target, and on the target as the caller
    /// named it when the changed row is a related todo the change also wrote.
    fn changed_refusal(
        &self,
        mine: &LocalItem,
        base: Option<&LocalItem>,
        now: Option<TodoSlug>,
    ) -> WorkError {
        let read = base.map_or(&mine.record.slug, |base| &base.record.slug);
        let renamed = now.filter(|now| now != read);
        let related = self
            .target
            .as_ref()
            .filter(|target| *target != &mine.record.id);
        if related.is_none() {
            return WorkError::refused(match renamed {
                Some(now) => format!(
                    "`{read}` changed while this command ran and is now named `{now}`; nothing of it was written. Run the command again on `{now}`: it acts on the todo's current record"
                ),
                None => format!(
                    "`{read}` changed while this command ran; nothing of it was written. Run the command again: it acts on the todo's current record"
                ),
            });
        }
        let changed = match renamed {
            Some(now) => format!(
                "`{read}`, which this change also writes, changed while this command ran and is now named `{now}`"
            ),
            None => {
                format!("`{read}`, which this change also writes, changed while this command ran")
            }
        };
        WorkError::refused(format!(
            "{changed}; nothing of it was written. {}: it acts on the todos' current records",
            self.door()
        ))
    }

    /// The door of a change refused for a related record: the command run
    /// again on its target, as the caller named it.
    fn door(&self) -> String {
        let target = self.target.as_ref().and_then(|target| {
            self.bases
                .get(target)
                .map(|base| &base.record.slug)
                .or_else(|| {
                    self.find_id(target)
                        .map(|index| &self.items[index].record.slug)
                })
        });
        match target {
            Some(slug) => format!("Run the command again on `{slug}`"),
            None => "Run the command again".to_owned(),
        }
    }

    /// Makes todo `id` the command's target: the todo a refusal of a change
    /// that also writes related todos sends the caller back to. Without it,
    /// the first todo the command entered is.
    pub fn set_target(&mut self, id: &TodoId) {
        self.target = Some(id.clone());
    }

    /// Appends `event` to the runner's event log in a transaction of its
    /// own.
    pub fn log_event(&mut self, log: &dyn EventLog, event: &Event) -> Result<(), WorkError> {
        let appended = self.store.write(|tx, _, _| Ok(log.append(tx, event)?))?;
        tracing::info!(
            event = "work_event_logged",
            name = event.name(),
            project = %event.project(),
            appended = ?appended,
            "an event logged on its own"
        );
        Ok(())
    }

    /// Puts the in-memory record set back as it was before a change, and
    /// drops the change's pending documentation transfer. A row a pull
    /// committed during the change stays the item, as the store holds it.
    fn restore(
        &mut self,
        before: Vec<LocalItem>,
        bases_before: &std::collections::BTreeMap<TodoId, LocalItem>,
    ) {
        self.items = before;
        self.pending_transfer = None;
        for index in 0..self.items.len() {
            let id = &self.items[index].record.id;
            if let Some(row) = self.bases.get(id)
                && bases_before.get(id) != Some(row)
            {
                self.items[index] = row.clone();
            }
        }
    }

    /// Gives the session's own item of `row`, a row reflection just
    /// committed, what reflection learned: its reflection state and learned
    /// artifact values, on the session's own record. A record another
    /// command wrote meanwhile is never taken.
    pub(super) fn learned(&mut self, row: Option<LocalItem>) {
        let Some(row) = row else { return };
        let Some(index) = self.find_id(&row.record.id) else {
            return;
        };
        let inbound = std::mem::take(&mut self.items[index].reflection.inbound);
        let item = _store::carry(&row, &self.items[index]);
        if item.record == row.record {
            self.bases.insert(row.record.id.clone(), row);
        }
        self.items[index] = item;
        self.items[index].reflection.inbound = inbound;
    }

    /// Where item `index`'s reflection stands, read from the local state
    /// database alone.
    pub fn status(&self, index: usize) -> ReflectionStatus {
        status_of(&self.items[index])
    }

    /// Every record this computer holds for the project. Reads nothing
    /// from GitHub.
    pub fn known_records(&self) -> Vec<TodoRecord> {
        self.items.iter().map(|it| it.record.clone()).collect()
    }

    /// The local item `address` names: a plain slug means the live item;
    /// `<slug>@<prefix>` the exact entry, retained ones included. A plain
    /// slug with only retained entries is refused naming their addresses.
    pub fn locate(&self, address: &TodoAddress) -> Result<usize, WorkError> {
        let slug = &address.slug;
        if address.identity.is_none() {
            if let Some(index) = self.find(slug, false) {
                return Ok(index);
            }
            return Err(retained_only(&self.items, slug).unwrap_or_else(|| self.absent(slug)));
        }
        exact_local(&self.items, address)?.ok_or_else(|| self.absent(slug))
    }

    /// The error for a todo this computer's record does not hold.
    pub fn absent(&self, slug: &TodoSlug) -> WorkError {
        WorkError::UnknownTodo {
            project: self.project.slug.clone(),
            todo: slug.clone(),
        }
    }

    /// A record this computer holds by identity.
    pub fn record_by_id(&self, id: &TodoId) -> Option<TodoRecord> {
        self.items
            .iter()
            .find(|it| &it.record.id == id)
            .map(|it| it.record.clone())
    }

    /// The live todo `slug` names among the local items.
    pub fn live_ref(&self, slug: &TodoSlug) -> Option<TodoRecord> {
        self.find(slug, false)
            .map(|index| self.items[index].record.clone())
    }

    /// Deletes withdrawn work past its recovery window from the local record
    /// and this session. A session with nothing expired writes nothing.
    fn purge(&mut self) -> Result<(), WorkError> {
        let now = self.now;
        let mut index = 0;
        while index < self.items.len() {
            if past_recovery(&self.items[index].record, now) {
                let slug = self.items[index].record.slug.clone();
                let id = self.items[index].record.id.clone();
                let deleted = self
                    .store
                    .delete_if(&id, |live| past_recovery(&live.record, now))?;
                if !deleted {
                    // Another process purged or restored it meanwhile: the
                    // session holds what the store holds of this project
                    // now. An identity another project's row holds by now
                    // is gone from this one (`Store::item`).
                    if let Some(live) = self.store.item(&id)? {
                        self.take_row(index, live);
                        index += 1;
                        continue;
                    }
                }
                self.items.remove(index);
                self.bases.remove(&id);
                if deleted {
                    tracing::info!(
                        event = "work_purged",
                        project = %self.project.slug,
                        todo = %slug,
                        id = %id,
                        "withdrawn work past its recovery window purged from the local record"
                    );
                    self.purged.push(slug);
                }
            } else {
                index += 1;
            }
        }
        Ok(())
    }
}

/// The refusal when `what` names several entries: each by its exact
/// address.
pub(crate) fn several<'r>(what: &str, records: impl Iterator<Item = &'r TodoRecord>) -> WorkError {
    let addresses: Vec<String> = records
        .map(|record| format!("`{}`", TodoAddress::exact(record)))
        .collect();
    WorkError::refused(format!(
        "{what} names several entries; address one exactly: {}",
        addresses.join(", ")
    ))
}

/// The refusal of a change to `slug` while its withdrawal is recorded and
/// not ended (`withdrawal_intents`).
pub(crate) fn withdrawing_refusal(slug: &TodoSlug) -> WorkError {
    WorkError::refused(format!(
        "`{slug}` is being withdrawn on this computer, or its withdrawal was interrupted; nothing of this change was written. Run `instrukt todo remove {slug}` to complete the withdrawal, or run this command again once it has ended"
    ))
}

/// Where `item`'s reflection stands, read from the local state database alone.
pub(crate) fn status_of(item: &LocalItem) -> ReflectionStatus {
    let reflection = &item.reflection;
    // Content GitHub refused stays outstanding, with GitHub's reason, until
    // it changes: for an artifact version, until a later version of that
    // artifact is on GitHub (`github-record`, Writer and convergence).
    let outstanding: Vec<&_store::KeptRefusal> = reflection
        .refusals
        .iter()
        .filter(|kept| !superseded(&item.record, &kept.target))
        .collect();
    if !outstanding.is_empty() {
        let refused: Vec<String> = outstanding
            .iter()
            .map(|kept| {
                format!(
                    "GitHub refused {}: {}",
                    refused_target(&kept.target),
                    kept.detail
                )
            })
            .collect();
        return ReflectionStatus::Pending(PendingReason::Failed(refused.join("; ")));
    }
    if let Some(error) = &reflection.failure {
        return ReflectionStatus::Pending(PendingReason::Failed(error.clone()));
    }
    // Outstanding pull-request creation, reviews or closing are
    // reflection still owed.
    let pull_owed = _reflect::pull_request_owed(item);
    if let Some(error) = &reflection.board_failure {
        return ReflectionStatus::Pending(PendingReason::Failed(error.clone()));
    }
    // A work branch that cannot be published stays the todo's outstanding
    // reason, also once its pull request owes nothing more.
    if let Some(why) = &reflection.publication_outstanding {
        return ReflectionStatus::Pending(PendingReason::Failed(why.clone()));
    }
    if !reflection.pending && !pull_owed {
        return ReflectionStatus::Reflected;
    }
    ReflectionStatus::Pending(if reflection.pending {
        PendingReason::Failed("not yet reflected".to_owned())
    } else {
        PendingReason::Failed(
            reflection
                .pull_outstanding
                .clone()
                .unwrap_or_else(|| "the pull request is not yet reflected".to_owned()),
        )
    })
}

/// Whether a refused `target` no longer holds the todo's reflection back:
/// a refused artifact version is followed by the artifact's next version,
/// so a later version of the same artifact that GitHub has confirmed
/// supersedes it. The refusal itself stays kept, so the refused version is
/// never sent again unchanged.
pub(crate) fn superseded(record: &TodoRecord, target: &_store::RefusedTarget) -> bool {
    let _store::RefusedTarget::Artifact { name, sequence } = target else {
        return false;
    };
    record.artifact(name.as_str()).is_some_and(|artifact| {
        artifact
            .versions
            .iter()
            .any(|v| v.sequence > *sequence && v.remote.is_some())
    })
}

/// A reflected target in words.
fn refused_target(target: &_store::RefusedTarget) -> String {
    match target {
        _store::RefusedTarget::Issue => "the issue".to_owned(),
        _store::RefusedTarget::PullCreation => "the pull request".to_owned(),
        _store::RefusedTarget::Review { verdict } => {
            format!("the review of build verdict {verdict}")
        }
        _store::RefusedTarget::Artifact { name, sequence } => {
            format!("artifact `{name}` version {sequence}")
        }
    }
}

/// Whether `record` is withdrawn work whose recovery window has passed at
/// `now`.
pub(crate) fn past_recovery(record: &TodoRecord, now: Timestamp) -> bool {
    matches!(
        (&record.status, &record.removal),
        (TodoStatus::Removed, Some(removal))
            if removal.at.checked_add(RECOVERY_WINDOW).is_ok_and(|end| end <= now)
    )
}

/// The refusal for a plain `slug` that names no live item in `items` but
/// retained ones: each by its exact address. `None` when it names none.
pub(crate) fn retained_only(items: &[LocalItem], slug: &TodoSlug) -> Option<WorkError> {
    let retained: Vec<String> = items
        .iter()
        .map(|it| &it.record)
        .filter(|r| &r.slug == slug)
        .map(|r| format!("`{}` ({})", TodoAddress::exact(r), status_word(r.status)))
        .collect();
    (!retained.is_empty()).then(|| {
        WorkError::refused(format!(
            "no live todo is named `{slug}`; address a retained entry exactly: {}",
            retained.join(", ")
        ))
    })
}

/// The one item of `items` an exact `address` names; several are refused
/// naming their identities.
pub(crate) fn exact_local(
    items: &[LocalItem],
    address: &TodoAddress,
) -> Result<Option<usize>, WorkError> {
    let local: Vec<usize> = items
        .iter()
        .enumerate()
        .filter(|(_, it)| address.names(&it.record))
        .map(|(i, _)| i)
        .collect();
    match local.as_slice() {
        [index] => Ok(Some(*index)),
        [] => Ok(None),
        many => {
            let ids: Vec<String> = many
                .iter()
                .map(|i| items[*i].record.id.to_string())
                .collect();
            Err(WorkError::refused(format!(
                "`{address}` names several entries; give more of the identity: {}",
                ids.join(", ")
            )))
        }
    }
}

/// The local item addressed by `slug` in `items`: the live one, or with
/// `retained`, else the latest delivered or removed one.
pub(crate) fn find_in(items: &[LocalItem], slug: &TodoSlug, retained: bool) -> Option<usize> {
    let live = items
        .iter()
        .position(|it| &it.record.slug == slug && it.record.is_live());
    live.or_else(|| {
        retained
            .then(|| {
                items
                    .iter()
                    .enumerate()
                    .filter(|(_, it)| &it.record.slug == slug)
                    .max_by_key(|(_, it)| it.record.created)
                    .map(|(i, _)| i)
            })
            .flatten()
    })
}

/// A pending reason in words.
pub fn describe(reason: &PendingReason) -> String {
    match reason {
        PendingReason::NoCredentials => "no credentials".to_owned(),
        PendingReason::Unreachable => "GitHub is unreachable".to_owned(),
        PendingReason::RateLimited { until: Some(until) } => {
            format!(
                "GitHub rate limited until {}",
                until.strftime("%Y-%m-%dT%H:%M:%SZ")
            )
        }
        PendingReason::RateLimited { until: None } => "GitHub rate limited".to_owned(),
        PendingReason::Unauthorized => "GitHub refused the credentials".to_owned(),
        PendingReason::Failed(message) => message.clone(),
    }
}
