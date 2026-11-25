//! One command's view of a project's work record: the loaded local items
//! with the rows they were read from, the GitHub client and its reach, and
//! the shared steps every operation composes — entering an item, pulling it
//! through, committing a change and flushing pending reflection. It holds
//! no lock: each local write takes the writer lock for its transaction,
//! and reflection takes the reflection lock across its GitHub requests.

use instrukt_core::config::{BoardRef, Configuration};
use instrukt_core::git;
use instrukt_core::ids::{SessionRef, TodoSlug};
use instrukt_core::machine::{self, MachineId};
use instrukt_core::state_db::rusqlite::Transaction;
use instrukt_runner::interfaces::ProjectRef;
use jiff::{SignedDuration, Timestamp};

use super::_envelope::status_name as status_word;
use super::_error::WorkError;
use super::_github::{GitHub, GitHubError};
use super::_inbound;
use super::_model::{TodoAddress, TodoId, TodoRecord, TodoRef};
use super::_reflect;
use super::_store::{self, IssueRef, LocalItem, ProviderState, ReflectionLock, Refusal, Store};
use super::Work;
use crate::evidence::{
    Actor, ApprovalGate, ApprovedBy, ModelIdentity, ModelProvenance, StampFact, StampRecord,
    TodoStatus, artifact_names,
};
use instrukt_core::github;
use instrukt_core::publication::EvalFingerprint;
use instrukt_runner::events::{Event, EventLog};
use instrukt_runner::sessions::Sessions;

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

/// Where an item's reflection stands after a command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReflectionStatus {
    Reflected,
    Pending(PendingReason),
}

/// Whether GitHub could be read during a command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitHubRead {
    Read,
    NotRead(PendingReason),
}

enum Remote {
    Client {
        gh: Box<GitHub>,
        /// Set when GitHub stopped answering during this command.
        down: Option<GitHubError>,
    },
    Unavailable(PendingReason),
}

pub(crate) struct Session<'w> {
    work: &'w Work,
    pub project: ProjectRef,
    pub config: Configuration,
    store: Store,
    pub items: Vec<LocalItem>,
    /// Each stored item's row as this session last read or wrote it: the
    /// base every commit of the item is judged against.
    bases: std::collections::BTreeMap<TodoId, LocalItem>,
    /// Why this command's reflection could not take the reflection lock:
    /// another process reflected for longer than the wait.
    reflection_busy: Option<String>,
    remote: Remote,
    /// The todo the command acts on: the first one it entered. A refusal
    /// of a change that also writes related todos sends the caller back to
    /// it.
    target: Option<TodoId>,
    pub now: Timestamp,
    /// The project's default branch.
    pub base: String,
    /// Local items whose recovery window expired when the session opened.
    pub purged: Vec<TodoSlug>,
    /// A cockpit setup failure retained independently of item reflection.
    pub board_setup_failure: Option<String>,
    /// Documentation files this command rewrote in the working tree.
    pub docs_changed: Vec<std::path::PathBuf>,
    /// A group parent's pending documentation owed to its first child.
    pub pending_transfer: Option<(TodoSlug, TodoSlug)>,
}

impl<'w> Session<'w> {
    /// Loads the project's local record. It takes no lock, except that a
    /// database behind this build's schema migrates once under the writer
    /// lock, and withdrawn work whose recovery window has expired is purged,
    /// each item in a writer transaction of its own.
    pub fn open(work: &'w Work, project: &ProjectRef) -> Result<Self, WorkError> {
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
        let (items, provider) = store.existence()?;
        let now = work.now();
        // Anchored here, before the capability lock is taken, so a wait for
        // it is part of every GitHub answer's elapsed time.
        let clock = super::_apps::Clock::starting(now);
        let remote = match unavailable(work, &provider, now) {
            Some(reason) => Remote::Unavailable(reason),
            None => match GitHub::for_app(
                {
                    work.github.begin_pass().map_err(GitHubError::from)?;
                    work.github.clone()
                },
                &api_url,
                &repository,
                None,
                github::Operation::Record,
                clock,
            ) {
                Ok(gh) => Remote::Client {
                    gh: Box::new(gh),
                    down: None,
                },
                Err(err) => Remote::Unavailable(reason_of(&err)),
            },
        };
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
            reflection_busy: None,
            remote,
            target: None,
            now,
            base,
            purged: Vec::new(),
            docs_changed: Vec::new(),
            board_setup_failure: None,
            pending_transfer: None,
        };
        session.purge()?;
        Ok(session)
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
        match Sessions::new(self.work.home(), &self.config, self.work.env()).model(session) {
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

    /// The local item `slug` addresses, with what people did on its issue
    /// applied before the command acts on it (`github-record`, "Reading
    /// GitHub"). A todo this computer's record does not hold is unknown.
    pub fn enter(&mut self, slug: &TodoSlug) -> Result<usize, WorkError> {
        let Some(index) = self.find(slug, false) else {
            return Err(self.absent(slug));
        };
        if self.target.is_none() {
            self.target = Some(self.items[index].record.id.clone());
        }
        self.pull_through(index)?;
        Ok(index)
    }

    /// Like [`Session::enter`] for a reference by identity.
    pub fn enter_ref(&mut self, target: &TodoRef) -> Result<usize, WorkError> {
        match self.find_id(&target.id) {
            Some(index) => {
                self.pull_through(index)?;
                Ok(index)
            }
            None => self.enter(&target.slug),
        }
    }

    /// Reads item `index`'s issue for people's controls and applies them
    /// (`github-record`, Reading GitHub): `hold` label actions and
    /// `/instrukt` commands, each once, and output labels someone changed
    /// are reflected again. An unchanged issue is not read further; a
    /// changed one is read from the stored comment and event positions on.
    /// The positions, and the issue's `ETag`, move only past controls
    /// applied or answered: a control still outstanding is read again.
    /// Nothing read from GitHub changes the record otherwise, and a GitHub
    /// that cannot be read leaves the item as this computer holds it.
    ///
    /// No writer lock is held across the requests: the row is re-read
    /// before them, and what the read learned of the issue is committed
    /// onto the live row's reflection state. An item carrying this
    /// command's uncommitted change is not read.
    pub fn pull_through(&mut self, index: usize) -> Result<(), WorkError> {
        let id = self.items[index].record.id.clone();
        if self.uncommitted(index) {
            return Ok(());
        }
        if let Some(live) = self.store.item(&id)? {
            self.take_row(index, live);
        }
        let mut read = self.items[index].reflection.clone();
        let result = match &self.remote {
            Remote::Client { gh, down: None } => _reflect::read_issue(gh, &mut read),
            _ => return Ok(()),
        };
        let fresh = match result {
            Err(err) => {
                self.note(err);
                return Ok(());
            }
            Ok(Some(fresh)) => fresh,
            Ok(None) => {
                if read.issue.is_none() && self.items[index].reflection.issue.is_some() {
                    // The issue is gone: the next flush reflects the item
                    // again.
                    let row = self.store.amend_reflection(&id, |r| *r = read.clone())?;
                    self.learned(row);
                    return Ok(());
                }
                return self.post_pending_replies(index);
            }
        };
        let fresh_etag = read.etag.take();
        let commands: Vec<u64> = read
            .inbound
            .iter()
            .filter_map(|comment| comment["id"].as_u64())
            .collect();
        self.items[index].reflection.inbound = std::mem::take(&mut read.inbound);
        // While the todo's withdrawal is open, what the issue asks of it
        // waits: the first read after the withdrawal commits answers it as
        // the withdrawn todo (`state-database`, `withdrawal_intents`). An
        // answer recorded later is recorded only on the record it was
        // decided on, with no withdrawal open ([`Store::retain_reply`]).
        if self.store.withdrawal_intent(&id)?.is_some() {
            self.items[index].reflection.inbound.clear();
            return Ok(());
        }
        let events = self.answer_label_actions(index)?;
        self.answer_commands(index)?;
        self.restore_output_labels(index, &fresh.issue)?;
        let seen = &self.items[index].reflection.commands_seen;
        let comments: Vec<(Timestamp, bool)> = fresh
            .comments
            .iter()
            .map(|(comment, at)| (*at, commands.contains(comment) && !seen.contains(comment)))
            .collect();
        let held = self.items[index].reflection.clone();
        let comments_since = super::_positions::next_comments_since(
            held.comments_since
                .as_deref()
                .and_then(|at| at.parse::<Timestamp>().ok()),
            &comments,
        )
        .map(|at| at.to_string());
        let (events_page, events_last, events_outstanding) = match &events {
            Some(events) => {
                let (page, last) = super::_positions::next_events_position(
                    (held.events_page, held.events_last),
                    events,
                );
                (
                    page,
                    last,
                    events.iter().any(|(_, _, outstanding)| *outstanding),
                )
            }
            None => (held.events_page, held.events_last, true),
        };
        let outstanding =
            events_outstanding || comments.iter().any(|(_, outstanding)| *outstanding);
        let etag = (!outstanding).then_some(fresh_etag).flatten();
        tracing::debug!(
            event = "work_issue_read",
            project = %self.project.slug,
            todo = %self.items[index].record.slug,
            comments = fresh.comments.len(),
            events = events.as_ref().map_or(0, Vec::len),
            outstanding,
            "people's controls read from the todo's issue"
        );
        self.amend(index, |r| {
            r.comments_since.clone_from(&comments_since);
            r.events_page = events_page;
            r.events_last = events_last;
            r.etag.clone_from(&etag);
        })
    }

    /// Whether item `index` carries a change of this command not yet
    /// committed: its record differs from the row it was read from, or it
    /// has no row yet.
    fn uncommitted(&self, index: usize) -> bool {
        let item = &self.items[index];
        self.bases
            .get(&item.record.id)
            .is_none_or(|base| base.record != item.record)
    }

    /// Makes `row`, read or written just now, item `index` and its base.
    fn take_row(&mut self, index: usize, row: LocalItem) {
        self.bases.insert(row.record.id.clone(), row.clone());
        self.items[index] = row;
    }

    /// Answers the approvers' `/instrukt` commands the last pull found on
    /// item `index`'s issue (`github-record`, "Writer and convergence"). An
    /// approval applies only when its digest prefix matches the gate's
    /// current evidence, attributed to the approver's GitHub login; a
    /// change request is recorded with its reason. A command from anyone not
    /// in `github.approvers` with write access changes nothing. Every
    /// command is answered on the issue, once.
    fn answer_commands(&mut self, index: usize) -> Result<(), WorkError> {
        let commands = std::mem::take(&mut self.items[index].reflection.inbound);
        if self.items[index].reflection.issue.is_none() {
            return Ok(());
        }
        let approvers = self
            .config
            .github_approvers()?
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        for comment in commands {
            let Some(id) = comment["id"].as_u64() else {
                continue;
            };
            let login = comment["user"]["login"]
                .as_str()
                .unwrap_or_default()
                .to_owned();
            if let Some(prior) = self.items[index]
                .record
                .stamps
                .iter()
                .find(|stamp| stamp.source_comment == Some(id))
                .cloned()
            {
                let original_login = match &prior.actor {
                    Actor::GitHub { login } => login.as_str(),
                    _ => login.as_str(),
                };
                let (gate, target) = match &prior.fact {
                    StampFact::Approval { gate, target, .. } => (*gate, Some(target)),
                    StampFact::ChangesRequested { gate, .. } => (*gate, None),
                    _ => {
                        return Err(WorkError::refused(
                            "a decision source identifies an incompatible stamp; inspect the work record",
                        ));
                    }
                };
                let gate = match gate {
                    ApprovalGate::Prd => "prd",
                    ApprovalGate::Task => "task",
                };
                let answer = match target {
                    Some(target) => format!(
                        "@{original_login}: recorded your approval of {gate} `{}`.",
                        _inbound::shown(target)
                    ),
                    None => format!("@{original_login}: recorded your change request on {gate}."),
                };
                if !self.answered(index, id, comment["created_at"].as_str(), &answer)? {
                    break;
                }
                continue;
            }
            let write_access = if approvers.contains(&login) {
                let result = match &self.remote {
                    Remote::Client { gh, down: None } => gh.can_write(&login),
                    _ => break,
                };
                match result {
                    Ok(allowed) => allowed,
                    Err(error) => {
                        tracing::warn!(event="approval_actor_unverified",login,error=%error);
                        self.amend(index, |reflection| reflection.etag = None)?;
                        self.note(error);
                        break;
                    }
                }
            } else {
                false
            };
            let answer = if !write_access {
                format!(
                    "@{login}: only an approver listed in `github.approvers` with write access to this repository can approve or request changes; nothing changed."
                )
            } else {
                match _inbound::parse(comment["body"].as_str().unwrap_or_default()) {
                    Err(why) => format!("@{login}: {why}; nothing changed."),
                    Ok(command) => match self.apply_command(
                        index,
                        id,
                        comment["created_at"].as_str(),
                        &login,
                        command,
                    ) {
                        Ok(answer) => answer,
                        Err(err) => {
                            // Not recorded: the command stays outstanding,
                            // and the next pull reads it again.
                            let todo = self.items[index].record.id.clone();
                            self.store.forget_etag(&todo)?;
                            return Err(err);
                        }
                    },
                }
            };
            if !self.answered(index, id, comment["created_at"].as_str(), &answer)? {
                break;
            }
        }
        self.post_pending_replies(index)
    }

    /// Posts item `index`'s replies still pending, also when its issue
    /// read as unchanged. Replies are posted only by the holder of the
    /// reflection lock. A busy lock leaves them pending: a later flush of
    /// this project that takes the lock attempts the pending replies of each
    /// item, and any it cannot post stay pending for the flush after it.
    fn post_pending_replies(&mut self, index: usize) -> Result<(), WorkError> {
        let todo = self.items[index].record.id.to_string();
        let Some(issue) = self.items[index].reflection.issue else {
            return Ok(());
        };
        if self.store.pending_replies(&todo)?.is_empty() {
            return Ok(());
        }
        match ReflectionLock::try_acquire(self.work.home())? {
            Some(lock) => self.post_replies(issue, &todo, &lock),
            None => Ok(()),
        }
    }

    /// Records command `comment` of item `index` as answered with `answer`;
    /// `false`, with nothing recorded, when the todo's withdrawal opened
    /// meanwhile: the command and those after it stay outstanding.
    fn answered(
        &mut self,
        index: usize,
        comment: u64,
        commanded_at: Option<&str>,
        answer: &str,
    ) -> Result<bool, WorkError> {
        let id = self.items[index].record.id.clone();
        if !self
            .store
            .retain_reply(&self.items[index].record, comment, commanded_at, answer)?
        {
            return Ok(false);
        }
        let rows = std::iter::once(&mut self.items[index]).chain(self.bases.get_mut(&id));
        for item in rows {
            if !item.reflection.commands_seen.contains(&comment) {
                item.reflection.commands_seen.push(comment);
            }
        }
        Ok(true)
    }

    /// Applies `amend` to item `index`'s stored reflection state and takes
    /// the row it leaves.
    fn amend(
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

    /// Posts the pending command replies of `todo` on `issue`; the
    /// reflection lock is the proof that no other holder posts them.
    fn post_replies(
        &mut self,
        issue: IssueRef,
        todo: &str,
        _proof: &ReflectionLock,
    ) -> Result<(), WorkError> {
        for (source, commanded_at, text) in self.store.pending_replies(todo)? {
            let result = match &self.remote {
                Remote::Client { gh, down: None } => {
                    _reflect::reply(gh, issue, source, commanded_at.as_deref(), &text)
                }
                _ => break,
            };
            match result {
                Ok(()) => self.store.reply_sent(todo, source)?,
                Err(error) => {
                    tracing::warn!(event="github_command_reply_pending",source,error=%error);
                    self.note(error);
                    break;
                }
            }
        }
        Ok(())
    }

    /// Marks item `index` pending when `snapshot`, its issue as just read,
    /// carries other output labels than the record shows.
    fn restore_output_labels(
        &mut self,
        index: usize,
        snapshot: &serde_json::Value,
    ) -> Result<(), WorkError> {
        let expected = super::_envelope::issue(&self.items[index].record).labels;
        let actual = snapshot["labels"]
            .as_array()
            .map(|labels| {
                labels
                    .iter()
                    .filter_map(|l| l["name"].as_str())
                    .filter(|l| super::_reflect::output_label(l))
                    .collect::<std::collections::BTreeSet<_>>()
            })
            .unwrap_or_default();
        if actual != expected.iter().map(String::as_str).collect() {
            self.amend(index, |reflection| reflection.pending = true)?;
        }
        Ok(())
    }

    /// Applies the attributable `hold` label actions read from item
    /// `index`'s issue once each, in their source order. Returns each event
    /// read as (page, identity, outstanding) — an action not yet applied or
    /// recorded as ignored is outstanding, and so is every event after it —
    /// or `None` when the events could not be read.
    fn answer_label_actions(
        &mut self,
        index: usize,
    ) -> Result<Option<Vec<super::_positions::EventRead>>, WorkError> {
        let Some(issue) = self.items[index].reflection.issue else {
            return Ok(Some(Vec::new()));
        };
        let result = match &self.remote {
            Remote::Client { gh, down: None } => {
                _reflect::read_events(gh, issue, &self.items[index].reflection)
            }
            _ => return Ok(None),
        };
        let events = match result {
            Ok(events) => events,
            Err(error) => {
                tracing::warn!(event="github_label_actions_unreadable",issue=issue.number,error=%error);
                self.note(error);
                return Ok(None);
            }
        };
        let mut read = Vec::with_capacity(events.len());
        let mut blocked = false;
        for (page, action) in events {
            let Some(id) = action["id"].as_u64() else {
                continue;
            };
            if blocked {
                read.push((page, id, true));
                continue;
            }
            let applied = self.answer_label_action(index, issue, id, &action)?;
            blocked = !applied;
            read.push((page, id, !applied));
        }
        Ok(Some(read))
    }

    /// Applies one event `action` (identity `id`) read from item `index`'s
    /// `issue`; `false` while it stays outstanding: its actor's access
    /// could not be verified, GitHub stopped answering, or the todo's
    /// record changed or its withdrawal opened before the outcome was
    /// recorded.
    fn answer_label_action(
        &mut self,
        index: usize,
        issue: IssueRef,
        id: u64,
        action: &serde_json::Value,
    ) -> Result<bool, WorkError> {
        if action["label"]["name"].as_str() != Some("hold") {
            return Ok(true);
        }
        let (Some(login), Some(kind), Some(at)) = (
            action["actor"]["login"].as_str(),
            action["event"].as_str(),
            action["created_at"].as_str(),
        ) else {
            tracing::warn!(event = "github_hold_unattributed", issue = issue.number);
            return Ok(true);
        };
        if !matches!(kind, "labeled" | "unlabeled")
            || self.items[index]
                .record
                .label_actions
                .iter()
                .any(|a| a.issue == issue.number && a.id == id)
            || self.store.label_seen(issue.number, id)?
        {
            return Ok(true);
        }
        let source_at = match at.parse::<Timestamp>() {
            Ok(at) => at,
            Err(error) => {
                tracing::warn!(event="github_hold_invalid_time",error=%error);
                return Ok(true);
            }
        };
        let permission = match &self.remote {
            Remote::Client { gh, down: None } => gh.can_write(login),
            _ => return Ok(false),
        };
        let allowed = match permission {
            Ok(allowed) => allowed,
            Err(error) => {
                tracing::warn!(event="github_hold_actor_unverified",login,error=%error);
                return Ok(false);
            }
        };
        let admitted = allowed && self.items[index].record.status != TodoStatus::StructuralHolder;
        if !admitted {
            return self.store.ignore_label(
                &self.items[index].record,
                issue.number,
                id,
                kind,
                login,
                at,
            );
        }
        let actor = Actor::GitHub {
            login: login.to_owned(),
        };
        self.commit("github hold label", |s| {
            let record = &mut s.items[index].record;
            if let Some(previous) = record.hold.take() {
                record.hold_history.push(super::_model::ReleasedHold {
                    hold: previous,
                    released: source_at,
                    by: actor.clone(),
                });
            }
            if kind == "labeled" {
                record.hold = Some(crate::evidence::HoldRecord {
                    source: crate::evidence::HoldSource::GitHub,
                    reason: format!("held from GitHub by {login}"),
                    by: actor,
                    at: source_at,
                });
            }
            record.label_actions.push(super::_model::LabelAction {
                issue: issue.number,
                id,
                action: kind.into(),
                login: login.into(),
                at: at.into(),
                outcome: "accepted".into(),
            });
            Ok(())
        })?;
        Ok(true)
    }

    /// Records an approver's command; returns the answer for the issue.
    fn apply_command(
        &mut self,
        index: usize,
        comment_id: u64,
        source_at: Option<&str>,
        login: &str,
        command: _inbound::Command,
    ) -> Result<String, WorkError> {
        let record = &self.items[index].record;
        if record.status != TodoStatus::Active {
            return Ok(format!(
                "@{login}: `{}` is not active work; nothing changed.",
                record.slug
            ));
        }
        let gate = match &command {
            _inbound::Command::Approve { gate, .. }
            | _inbound::Command::RequestChanges { gate, .. } => *gate,
        };
        let gate_name = match gate {
            ApprovalGate::Prd => "prd",
            ApprovalGate::Task => "task",
        };
        let evidence = match gate {
            ApprovalGate::Prd => record.current_digest(artifact_names::PRD).cloned(),
            ApprovalGate::Task => record.current_digest(artifact_names::TASK_PLAN).cloned(),
        };
        let (fact, answer) = match command {
            _inbound::Command::Approve { prefix, .. } => {
                let Some(current) = evidence else {
                    return Ok(format!(
                        "@{login}: there is no {gate_name} evidence to approve; nothing changed."
                    ));
                };
                if !_inbound::digest_hex(&current).starts_with(&prefix) {
                    return Ok(format!(
                        "@{login}: the {gate_name} evidence changed since `{prefix}`; review it and approve the current one again: `/instrukt approve {gate_name} {}`. Nothing changed.",
                        _inbound::shown(&current)
                    ));
                }
                (
                    StampFact::Approval {
                        gate,
                        by: ApprovedBy::Human,
                        target: current.clone(),
                        carried_from: None,
                        reasoning: None,
                    },
                    format!(
                        "@{login}: recorded your approval of {gate_name} `{}`.",
                        _inbound::shown(&current)
                    ),
                )
            }
            _inbound::Command::RequestChanges { reason, .. } => (
                StampFact::ChangesRequested {
                    gate,
                    target: evidence,
                    reason,
                },
                format!("@{login}: recorded your change request on {gate_name}."),
            ),
        };
        let now = self.now;
        let actor = Actor::GitHub {
            login: login.to_owned(),
        };
        let fingerprint = EvalFingerprint::current(self.work.home())?.map(|f| f.fingerprint);
        let todo_id = record.id.to_string();
        let sequence = record.sequence + 1;
        let notice=serde_json::json!({"decision":fact,"gate":gate_name,"actor":actor,"evidence":fact,"at":now.to_string(),"source_at":source_at,"arrived_at":now.to_string(),"source_comment":comment_id}).to_string();
        self.commit_with("inbound command", |s| {
            s.items[index].reflection.commands_seen.push(comment_id);
            let record = &mut s.items[index].record;
            let sequence = record.next_sequence();
            record.stamps.push(StampRecord {
                source_comment:Some(comment_id),
                fact,
                at: now,
                actor,
                model: None,
                eval_fingerprint: fingerprint,
                role: None,
                human_involved: None,
                sequence,
                model_provenance: ModelProvenance::OutsideSession,
            });
            Ok(())
        },|tx| {
            tx.execute("INSERT INTO approval_attention(todo_id,decision_sequence,content,state) VALUES (?1,?2,?3,'pending') ON CONFLICT(todo_id,decision_sequence) DO UPDATE SET content=excluded.content WHERE approval_attention.state='pending' AND approval_attention.input_id IS NULL",instrukt_core::state_db::rusqlite::params![todo_id,sequence,notice]).map_err(instrukt_core::state_db::StateDbError::sqlite(std::path::Path::new(tx.path().unwrap_or(""))))?;
            _store::retain_reply(tx,std::path::Path::new(tx.path().unwrap_or("")),&todo_id,comment_id,source_at,&answer)?;
            Ok(())
        })?;
        tracing::info!(
            event = "work_inbound_command",
            project = %self.project.slug,
            todo = %self.items[index].record.slug,
            login,
            "an approver's command was recorded"
        );
        Ok(answer)
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
    /// `change` runs before the transaction, holding no lock, so the GitHub
    /// reads it makes wait on nothing local.
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
    /// it is gone from the project: what a flush confirmed, whatever another
    /// command or reflection committed since this command last read it.
    /// Read in the same snapshot, whether the todo's withdrawal intent is
    /// open (`withdrawal_intents`).
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

    /// Flushes pending reflection of every stored item of the project under
    /// the reflection lock (`work-dispatch`, `github-record`): each row is
    /// re-read under the lock, reflected from a working copy with no writer
    /// lock held, and what reflection learned is committed onto the live
    /// row (`_store::merge_reflection`). A reflection another process holds
    /// for longer than the wait reflects nothing for this command; the
    /// session takes the reflection state the store records, so a revision
    /// that reflection confirmed meanwhile reads as reflected, and work still
    /// unconfirmed stays pending for a later flush.
    pub fn flush(&mut self) -> Result<(), WorkError> {
        let lock = match ReflectionLock::acquire(self.work.home()) {
            Ok(lock) => lock,
            Err(WorkError::Fs(busy @ instrukt_core::fsx::FsError::LockTimeout { .. })) => {
                tracing::warn!(
                    event = "work_reflection_busy",
                    project = %self.project.slug,
                    error = %busy,
                    "another reflection holds the reflection lock; this command reflects nothing and leaves what is unconfirmed to a later flush"
                );
                self.reflection_busy = Some(busy.to_string());
                for row in self.store.items()? {
                    self.learned(Some(row));
                }
                return Ok(());
            }
            Err(error) => return Err(error),
        };
        let board_result = (|| {
            // A board the developer names in the project layer leads; otherwise
            // the one this computer recorded. Setup reconciles either with
            // GitHub, finding an existing board before creating one.
            let configured = self.config.board()?;
            let recorded = self.store.recorded_board()?;
            let known = configured
                .clone()
                .or_else(|| recorded.as_deref().and_then(BoardRef::parse));
            let gh = match &self.remote {
                Remote::Client { gh, down: None } => gh,
                Remote::Client {
                    down: Some(error), ..
                } => return Err(error.clone().into()),
                Remote::Unavailable(reason) => return Err(WorkError::refused(describe(reason))),
            };
            let account = gh.for_role(None, github::Operation::Board);
            let board = super::_board::setup(&account, gh, known.as_ref())?;
            // The reference is the machine's record: reflection never writes
            // the committed project layer.
            if configured.as_ref() != Some(&board)
                && recorded.as_deref() != Some(&board.to_string())
            {
                self.store.record_board(&board.to_string())?;
            }
            super::_board::configure(&account, &board)?;
            Ok(board)
        })();
        let (board, board_error) = match board_result {
            Ok(board) => (Some(board), None),
            Err(error) => {
                if let WorkError::GitHub(
                    gh_error @ (GitHubError::RateLimited { .. }
                    | GitHubError::App(github::AppError::RateLimited { .. })),
                ) = &error
                {
                    self.note(gh_error.clone());
                }
                tracing::warn!(event="work_board_setup_pending", error=%error);
                (None, Some(error.to_string()))
            }
        };
        self.board_setup_failure = board_error.clone();
        for loaded in self.store.items()? {
            let id = loaded.record.id.clone();
            let mut reflected = loaded.clone();
            if !reflected.reflection.pending {
                reflected.reflection.failure = None;
            }
            if let Some(error) = &board_error {
                reflected.reflection.board_failure = Some(error.clone());
            }
            if let Some(issue) = reflected.reflection.issue {
                self.post_replies(issue, id.as_str(), &lock)?;
            }
            let owed = reflected.reflection.pending
                || _reflect::pull_request_owed(&reflected)
                || (board.is_some() && reflected.reflection.issue.is_some());
            if owed {
                // A todo shown as held already needs you: its record is not
                // projected and no approver is resolved for it.
                let needs_you = super::_ports::waits_on_person(
                    super::_envelope::record_status(&reflected.record),
                    || {
                        Ok::<_, instrukt_core::config::ConfigError>(
                            crate::workflow::waiting_for_human_approval(
                                &super::_ports::to_evidence(reflected.record.clone(), Vec::new()),
                                self.config
                                    .approver(instrukt_core::config::ApprovalGate::Prd)?,
                                self.config
                                    .approver(instrukt_core::config::ApprovalGate::Task)?,
                            ),
                        )
                    },
                )?;
                let result = match &self.remote {
                    Remote::Client { gh, down: None } => Some(_reflect::flush(
                        gh,
                        &mut reflected,
                        &self.base,
                        board.as_ref(),
                        needs_you,
                    )),
                    _ => None,
                };
                if let Some(Err(err)) = result {
                    if err.is_reach() {
                        self.note(err);
                    } else {
                        tracing::warn!(
                            event = "work_reflection_pending",
                            project = %self.project.slug,
                            todo = %reflected.record.slug,
                            error = %err,
                            "reflection failed; the local record keeps the change"
                        );
                        reflected.reflection.failure = Some(err.to_string());
                    }
                }
                if let Some(error) = &board_error {
                    reflected.reflection.board_failure = Some(error.clone());
                }
            }
            // A row this flush left as it was may still be news to the
            // session: another process can have reflected it already.
            let row = if _store::stored(&reflected) != _store::stored(&loaded) {
                self.store.commit_reflection(&loaded, &reflected)?
            } else {
                Some(loaded)
            };
            self.learned(row);
        }
        drop(lock);
        Ok(())
    }

    /// Gives the session's own item of `row`, a row reflection just
    /// committed, what reflection learned: its reflection state and learned
    /// artifact values, on the session's own record. A record another
    /// command wrote meanwhile is never taken.
    fn learned(&mut self, row: Option<LocalItem>) {
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

    /// Where item `index`'s reflection stands.
    pub fn status(&self, index: usize) -> ReflectionStatus {
        let unread = match &self.remote {
            Remote::Unavailable(reason) => Some(reason.clone()),
            Remote::Client {
                down: Some(err), ..
            } => Some(reason_of(err)),
            Remote::Client { down: None, .. } => {
                self.reflection_busy.clone().map(PendingReason::Failed)
            }
        };
        status_of(&self.items[index], unread)
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

    /// Records that GitHub stopped answering; a rate limit is remembered
    /// across commands until the provider permits requests again.
    fn note(&mut self, err: GitHubError) {
        if let GitHubError::RateLimited { until: Some(until) }
        | GitHubError::App(github::AppError::RateLimited { until: Some(until) }) = &err
        {
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

/// Where `item`'s reflection stands; `unread` is why GitHub cannot be
/// reflected to this command, `None` when it can.
pub(crate) fn status_of(item: &LocalItem, unread: Option<PendingReason>) -> ReflectionStatus {
    let reflection = &item.reflection;
    // Content GitHub refused stays outstanding, with GitHub's reason, until
    // it changes (`github-record`, Writer and convergence).
    if !reflection.refusals.is_empty() {
        let refused: Vec<String> = reflection
            .refusals
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
    if !reflection.pending && !pull_owed {
        return ReflectionStatus::Reflected;
    }
    ReflectionStatus::Pending(match unread {
        Some(reason) => reason,
        None if reflection.pending => PendingReason::Failed(
            reflection
                .failure
                .clone()
                .unwrap_or_else(|| "not yet reflected".to_owned()),
        ),
        None => PendingReason::Failed(
            reflection
                .pull_outstanding
                .clone()
                .unwrap_or_else(|| "the pull request is not yet reflected".to_owned()),
        ),
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

/// Why GitHub cannot be reached for `work` at `now`, read from this
/// computer alone: a rate limit GitHub set, or App credentials that cannot
/// be used. `None` when a client can be built.
pub(crate) fn unavailable(
    work: &Work,
    provider: &ProviderState,
    now: Timestamp,
) -> Option<PendingReason> {
    match provider.deferred_until {
        Some(until) if until > now => Some(PendingReason::RateLimited { until: Some(until) }),
        _ => work
            .github
            .validate()
            .err()
            .map(|err| reason_of(&GitHubError::from(err))),
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

pub(crate) fn reason_of(err: &GitHubError) -> PendingReason {
    match err {
        GitHubError::Unreachable(_) => PendingReason::Unreachable,
        GitHubError::App(github::AppError::Unreachable) => PendingReason::Unreachable,
        GitHubError::App(github::AppError::RateLimited { until }) => {
            PendingReason::RateLimited { until: *until }
        }
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
