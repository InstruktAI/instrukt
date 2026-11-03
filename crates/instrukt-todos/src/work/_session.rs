//! One command's hold on a project's work record: the writer lock, the
//! loaded local items, the GitHub client and its reach, and the shared
//! steps every operation composes — entering an item, pulling it through,
//! committing a change and flushing pending reflection.

use instrukt_core::config::Configuration;
use instrukt_core::fsx::LockGuard;
use instrukt_core::git;
use instrukt_core::ids::{RuntimeName, SessionId, SessionRef, TodoSlug};
use instrukt_core::machine::{self, MachineId};
use instrukt_runner::interfaces::ProjectRef;
use jiff::{SignedDuration, Timestamp};

use super::_error::WorkError;
use super::_github::{GitHub, GitHubError};
use super::_model::{TodoId, TodoRecord, TodoRef};
pub(crate) use super::_reflect::PullFor;
use super::_reflect::{self, Pulled};
use super::_store::{self, IssueRef, LocalItem, Store};
use super::Work;
use crate::evidence::{Actor, TodoStatus};
use instrukt_core::github;

/// The fixed recovery window of withdrawn work.
const RECOVERY_WINDOW: SignedDuration = SignedDuration::from_hours(7 * 24);

/// Why reflection is pending.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PendingReason {
    /// No token in `GITHUB_TOKEN`, `GH_TOKEN` or `gh auth token`.
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

/// Where an item's reflection stands after a command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReflectionStatus {
    Reflected,
    Pending(PendingReason),
    /// A newer remote revision met a local change; `instrukt todo refresh`.
    Conflicted,
}

/// Whether GitHub could be read during a command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitHubRead {
    Read,
    NotRead(PendingReason),
}

enum Remote {
    Client {
        gh: GitHub,
        /// Set when GitHub stopped answering during this command.
        down: Option<GitHubError>,
    },
    Unavailable(PendingReason),
}

pub(crate) struct Session<'w> {
    work: &'w Work,
    _lock: LockGuard,
    pub project: ProjectRef,
    pub config: Configuration,
    store: Store,
    pub items: Vec<LocalItem>,
    remote: Remote,
    /// Live reflected work read directly from GitHub this command.
    remote_live: Option<Vec<(TodoRecord, IssueRef)>>,
    pub now: Timestamp,
    /// The project's default branch.
    pub base: String,
    /// Local items whose recovery window expired when the session opened.
    pub purged: Vec<TodoSlug>,
}

impl<'w> Session<'w> {
    /// Takes the writer lock and loads the project's local record. Purges
    /// withdrawn work whose recovery window has expired.
    pub fn open(work: &'w Work, project: &ProjectRef) -> Result<Self, WorkError> {
        let lock = _store::lock(work.home())?;
        let config = Configuration::load(work.home(), Some(&project.root))?;
        // The address the token is sent to comes from the machine's own
        // layer, never from a repository's.
        let api_url = github::api_url(work.home())?;
        let repository = github::repository(&config, &api_url, &project.root)?;
        let store = Store::open(work.home(), &project.slug)?;
        let items = store.items()?;
        let now = work.now();
        let remote = match github::credentials(work.env()) {
            None => Remote::Unavailable(PendingReason::NoCredentials),
            Some(token) => match store.provider()?.deferred_until {
                Some(until) if until > now => {
                    Remote::Unavailable(PendingReason::RateLimited { until: Some(until) })
                }
                _ => Remote::Client {
                    gh: GitHub::new(&api_url, &repository, token),
                    down: None,
                },
            },
        };
        let base = github::default_branch(&project.root)?;
        let mut session = Self {
            work,
            _lock: lock,
            project: project.clone(),
            config,
            store,
            items,
            remote,
            remote_live: None,
            now,
            base,
            purged: Vec::new(),
        };
        session.purge()?;
        Ok(session)
    }

    pub fn machine(&self) -> Result<MachineId, WorkError> {
        Ok(machine::machine_id(self.work.home())?)
    }

    /// The session the caller runs in: the active runtime's, when one is
    /// active.
    pub fn caller_session(&self) -> Option<SessionRef> {
        let entry = self.config.active_runtime(self.work.env()).ok()?;
        let id = self.work.env().get(&entry.session_id_env)?;
        Some(SessionRef {
            runtime: RuntimeName::parse(&entry.name).ok()?,
            id: SessionId::parse(id).ok()?,
        })
    }

    /// Who acts: the caller's session, or outside one the person git names.
    /// No identity is guessed.
    pub fn actor(&self) -> Result<Actor, WorkError> {
        if let Some(session) = self.caller_session() {
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
        let live = self
            .items
            .iter()
            .position(|it| &it.record.slug == slug && it.record.is_live());
        live.or_else(|| {
            retained
                .then(|| {
                    self.items
                        .iter()
                        .enumerate()
                        .filter(|(_, it)| &it.record.slug == slug)
                        .max_by_key(|(_, it)| it.record.created)
                        .map(|(i, _)| i)
                })
                .flatten()
        })
    }

    pub fn find_id(&self, id: &TodoId) -> Option<usize> {
        self.items.iter().position(|it| &it.record.id == id)
    }

    /// The item `slug` addresses, made local when it is reflected work of
    /// another computer, and pulled through. A conflicted item refuses.
    pub fn enter(&mut self, slug: &TodoSlug) -> Result<usize, WorkError> {
        let index = match self.find(slug, false) {
            Some(index) => index,
            None => self
                .localize_slug(slug)?
                .ok_or_else(|| WorkError::UnknownTodo {
                    project: self.project.slug.clone(),
                    todo: slug.clone(),
                })?,
        };
        self.pull_through(index, PullFor::Change)?;
        Ok(index)
    }

    /// Like [`Session::enter`] for a reference by identity.
    pub fn enter_ref(&mut self, target: &TodoRef) -> Result<usize, WorkError> {
        match self.find_id(&target.id) {
            Some(index) => {
                self.pull_through(index, PullFor::Change)?;
                Ok(index)
            }
            None => self.enter(&target.slug),
        }
    }

    fn localize_slug(&mut self, slug: &TodoSlug) -> Result<Option<usize>, WorkError> {
        let Some((record, issue)) = self
            .remote_live()
            .into_iter()
            .find(|(record, _)| &record.slug == slug)
        else {
            return Ok(None);
        };
        let result = match &self.remote {
            Remote::Client { gh, down: None } => _reflect::localize(gh, record, issue),
            _ => return Ok(None),
        };
        match result {
            Ok(item) => {
                self.store.save(&item)?;
                self.items.push(item);
                Ok(Some(self.items.len() - 1))
            }
            Err(err) => Err(self.github_failure(err)),
        }
    }

    /// Pulls item `index` from GitHub with a conditional request; refuses
    /// when it is (or becomes) conflicted, and, for a change, when GitHub
    /// holds a revision newer than the one this computer last read.
    pub fn pull_through(&mut self, index: usize, purpose: PullFor) -> Result<(), WorkError> {
        let before = self.items[index].clone();
        let result = match &self.remote {
            Remote::Client { gh, down: None } => {
                Some(_reflect::pull(gh, &mut self.items[index], purpose))
            }
            _ => None,
        };
        let newer = matches!(result, Some(Ok(Pulled::Newer)));
        match result {
            Some(Ok(Pulled::Adopted)) => tracing::info!(
                event = "work_pulled",
                project = %self.project.slug,
                todo = %self.items[index].record.slug,
                revision = self.items[index].record.revision,
                "a newer remote revision entered the local record"
            ),
            Some(Err(err)) => self.note(err),
            _ => {}
        }
        if self.items[index] != before {
            self.store.save(&self.items[index])?;
        }
        if self.items[index].reflection.conflicted || newer {
            if newer {
                tracing::debug!(
                    event = "work_change_refused_stale",
                    project = %self.project.slug,
                    todo = %self.items[index].record.slug,
                    "GitHub holds a newer revision than this computer last read; refresh first"
                );
            }
            return Err(WorkError::Conflicted {
                todo: self.items[index].record.slug.clone(),
            });
        }
        Ok(())
    }

    /// Pulls every local item, without refusing conflicted ones.
    pub fn pull_all(&mut self) -> Result<(), WorkError> {
        for index in 0..self.items.len() {
            match self.pull_through(index, PullFor::Read) {
                Ok(()) | Err(WorkError::Conflicted { .. }) => {}
                Err(err) => return Err(err),
            }
        }
        Ok(())
    }

    /// Runs `change` against the record set, then commits every record it
    /// changed: a new revision, pending reflection, saved. A refused change
    /// leaves every record as it was.
    pub fn commit<T>(
        &mut self,
        what: &'static str,
        change: impl FnOnce(&mut Self) -> Result<T, WorkError>,
    ) -> Result<T, WorkError> {
        let before: Vec<TodoRecord> = self.items.iter().map(|it| it.record.clone()).collect();
        let value = match change(self) {
            Ok(value) => value,
            Err(err) => {
                for (item, record) in self.items.iter_mut().zip(before) {
                    item.record = record;
                }
                return Err(err);
            }
        };
        for index in 0..self.items.len() {
            let changed = before
                .get(index)
                .is_none_or(|record| record != &self.items[index].record);
            if !changed {
                continue;
            }
            let item = &mut self.items[index];
            item.record.revision += 1;
            item.reflection.pending = true;
            self.store.save(item)?;
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

    /// Flushes pending reflection of every local item under the lock.
    pub fn flush(&mut self) -> Result<(), WorkError> {
        for index in 0..self.items.len() {
            let owed = self.items[index].reflection.pending
                || _reflect::pull_request_owed(&self.items[index]);
            if !owed || self.items[index].reflection.conflicted {
                continue;
            }
            let before = self.items[index].clone();
            let result = match &self.remote {
                Remote::Client { gh, down: None } => {
                    Some(_reflect::flush(gh, &mut self.items[index], &self.base))
                }
                _ => None,
            };
            if let Some(Err(err)) = result {
                if err.is_reach() {
                    self.note(err);
                } else {
                    tracing::warn!(
                        event = "work_reflection_pending",
                        project = %self.project.slug,
                        todo = %self.items[index].record.slug,
                        error = %err,
                        "reflection failed; the local record keeps the change"
                    );
                    self.items[index].reflection.failure = Some(err.to_string());
                }
            }
            if self.items[index] != before {
                self.store.save(&self.items[index])?;
            }
        }
        Ok(())
    }

    /// Where item `index`'s reflection stands.
    pub fn status(&self, index: usize) -> ReflectionStatus {
        let reflection = &self.items[index].reflection;
        if reflection.conflicted {
            return ReflectionStatus::Conflicted;
        }
        if !reflection.pending {
            return ReflectionStatus::Reflected;
        }
        ReflectionStatus::Pending(match &self.remote {
            Remote::Unavailable(reason) => reason.clone(),
            Remote::Client {
                down: Some(err), ..
            } => reason_of(err),
            Remote::Client { down: None, .. } => PendingReason::Failed(
                reflection
                    .failure
                    .clone()
                    .unwrap_or_else(|| "not yet reflected".to_owned()),
            ),
        })
    }

    /// Whether GitHub answered this command so far.
    pub fn github_read(&self) -> GitHubRead {
        match &self.remote {
            Remote::Unavailable(reason) => GitHubRead::NotRead(reason.clone()),
            Remote::Client {
                down: Some(err), ..
            } => GitHubRead::NotRead(reason_of(err)),
            Remote::Client { down: None, .. } => GitHubRead::Read,
        }
    }

    /// Live reflected work on GitHub (open issues with records), read once
    /// per command; empty when GitHub cannot be read.
    pub fn remote_live(&mut self) -> Vec<(TodoRecord, IssueRef)> {
        if let Some(records) = &self.remote_live {
            return records.clone();
        }
        let result = match &self.remote {
            Remote::Client { gh, down: None } => Some(_reflect::remote_records(gh, true)),
            _ => None,
        };
        let records = match result {
            Some(Ok(records)) => records,
            Some(Err(err)) => {
                self.note(err);
                Vec::new()
            }
            None => Vec::new(),
        };
        self.remote_live = Some(records.clone());
        records
    }

    /// All reflected work on GitHub, retained included; `None` when GitHub
    /// cannot be read.
    pub fn remote_all(&mut self) -> Option<Vec<(TodoRecord, IssueRef)>> {
        let result = match &self.remote {
            Remote::Client { gh, down: None } => _reflect::remote_records(gh, false),
            _ => return None,
        };
        match result {
            Ok(records) => Some(records),
            Err(err) => {
                self.note(err);
                None
            }
        }
    }

    /// A record of reflected work read directly from GitHub, with its
    /// artifact contents, stored nowhere.
    pub fn remote_item(&mut self, slug: &TodoSlug) -> Result<Option<LocalItem>, WorkError> {
        let found = match self.remote_all() {
            Some(records) => records
                .into_iter()
                .filter(|(record, _)| &record.slug == slug)
                .max_by_key(|(record, _)| (record.is_live(), record.created)),
            None => None,
        };
        let Some((record, issue)) = found else {
            return Ok(None);
        };
        let result = match &self.remote {
            Remote::Client { gh, down: None } => _reflect::localize(gh, record, issue),
            _ => return Ok(None),
        };
        match result {
            Ok(item) => Ok(Some(item)),
            Err(err) => Err(self.github_failure(err)),
        }
    }

    /// Replaces item `index` with GitHub's committed record.
    pub fn refresh(&mut self, index: usize) -> Result<(), WorkError> {
        let result = match &self.remote {
            Remote::Client { gh, down: None } => _reflect::refresh(gh, &mut self.items[index]),
            Remote::Client {
                down: Some(err), ..
            } => Err(err.clone()),
            Remote::Unavailable(reason) => {
                return Err(WorkError::refused(format!(
                    "refreshing reads GitHub, which cannot be read now: {}",
                    describe(reason)
                )));
            }
        };
        result.map_err(|err| self.github_failure(err))?;
        self.store.save(&self.items[index])?;
        tracing::info!(
            event = "work_refreshed",
            project = %self.project.slug,
            todo = %self.items[index].record.slug,
            revision = self.items[index].record.revision,
            "local item replaced by GitHub's committed record"
        );
        Ok(())
    }

    /// A record known to this command by identity: a local item's, or live
    /// reflected work's.
    pub fn record_by_id(&mut self, id: &TodoId) -> Option<TodoRecord> {
        if let Some(index) = self.find_id(id) {
            return Some(self.items[index].record.clone());
        }
        self.remote_live()
            .into_iter()
            .map(|(record, _)| record)
            .find(|record| &record.id == id)
    }

    /// The live todo `slug` names, local or reflected.
    pub fn live_ref(&mut self, slug: &TodoSlug) -> Option<TodoRecord> {
        if let Some(index) = self.find(slug, false) {
            return Some(self.items[index].record.clone());
        }
        self.remote_live()
            .into_iter()
            .map(|(record, _)| record)
            .find(|record| &record.slug == slug && record.is_live())
    }

    /// Deletes the item from the local record and this session.
    fn delete(&mut self, index: usize) -> Result<(), WorkError> {
        let item = self.items.remove(index);
        self.store.delete(&item.record.id)
    }

    fn purge(&mut self) -> Result<(), WorkError> {
        let now = self.now;
        let mut index = 0;
        while index < self.items.len() {
            let expired = matches!(
                (&self.items[index].record.status, &self.items[index].record.removal),
                (TodoStatus::Removed, Some(removal))
                    if removal.at.checked_add(RECOVERY_WINDOW).is_ok_and(|end| end <= now)
            );
            if expired {
                let slug = self.items[index].record.slug.clone();
                let id = self.items[index].record.id.clone();
                self.delete(index)?;
                tracing::info!(
                    event = "work_purged",
                    project = %self.project.slug,
                    todo = %slug,
                    id = %id,
                    "withdrawn work past its recovery window purged from the local record"
                );
                self.purged.push(slug);
            } else {
                index += 1;
            }
        }
        Ok(())
    }

    /// Records that GitHub stopped answering; a rate limit is remembered
    /// across commands until the provider permits requests again.
    fn note(&mut self, err: GitHubError) {
        if let GitHubError::RateLimited { until: Some(until) } = &err {
            let saved = self.store.provider().and_then(|mut state| {
                state.deferred_until = Some(*until);
                self.store.save_provider(&state)
            });
            if let Err(save) = saved {
                tracing::warn!(event = "work_rate_limit_unsaved", error = %save, "cannot remember GitHub's rate limit");
            }
        }
        if err.is_reach() {
            tracing::warn!(
                event = "work_github_unavailable",
                project = %self.project.slug,
                error = %err,
                "GitHub is not answering; the local record continues"
            );
            if let Remote::Client { down, .. } = &mut self.remote {
                *down = Some(err);
            }
        } else {
            tracing::warn!(
                event = "work_github_read_failed",
                project = %self.project.slug,
                error = %err,
                "a GitHub read failed"
            );
        }
    }

    /// A GitHub failure of a read that has no local answer.
    fn github_failure(&mut self, err: GitHubError) -> WorkError {
        if err.is_reach() {
            self.note(err.clone());
        }
        WorkError::GitHub(err)
    }
}

fn reason_of(err: &GitHubError) -> PendingReason {
    match err {
        GitHubError::Unreachable(_) => PendingReason::Unreachable,
        GitHubError::RateLimited { until } => PendingReason::RateLimited { until: *until },
        GitHubError::Unauthorized { .. } => PendingReason::Unauthorized,
        other => PendingReason::Failed(other.to_string()),
    }
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
