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
use super::_reflect::{self, HeadOrigin, Pulled};
use super::_store::{
    self, IssueRef, ListingCopy, LocalItem, ProviderState, ReflectionLock, Refusal, Store, Swapped,
};
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
const RECOVERY_WINDOW: SignedDuration = SignedDuration::from_hours(7 * 24);

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
    /// A newer remote revision met a local change; `instrukt todo refresh`.
    Conflicted,
}

/// Whether GitHub could be read during a command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitHubRead {
    Read,
    NotRead(PendingReason),
}

/// Why an artifact version's content is still missing after GitHub was
/// asked for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Unrestored {
    /// GitHub answered and does not serve the recorded content.
    NotServed,
    /// GitHub could not be read for it, for the reason given.
    Unread(PendingReason),
}

enum Remote {
    Client {
        gh: Box<GitHub>,
        /// Set when GitHub stopped answering during this command.
        down: Option<GitHubError>,
    },
    Unavailable(PendingReason),
}

/// Where [`Session::locate`] found an entry.
pub(crate) enum Located {
    Local(usize),
    /// A todo that is not a local item: as GitHub holds it now when
    /// `refreshed`, else its last known copy. Never a local item.
    Remote {
        item: Box<LocalItem>,
        refreshed: bool,
        copied_at: Timestamp,
    },
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
    /// The last known copies of the project's todos that are not local
    /// items (`listing_copies`).
    pub copies: Vec<ListingCopy>,
    /// The pickup listing read this command: every open todo issue with
    /// the head of its revision chain. `None` until read.
    listed: Option<Vec<(TodoRecord, IssueRef)>>,
    /// The todos refreshed for this command's relationship checks.
    current: Vec<TodoId>,
    /// The todo the command acts on: the first one it entered. A refusal
    /// of a change that also writes related todos sends the caller back to
    /// it.
    target: Option<TodoId>,
    /// A listing of reflected work GitHub failed this command, short of
    /// being unreachable: what is not local is then undetermined, never
    /// absent.
    lookup_failed: Option<GitHubError>,
    pub incompatible: std::collections::BTreeSet<TodoId>,
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
    /// What workflow said of a group parent's preparation, asked before
    /// the session opened.
    pub parent_preparation: super::_ops::ParentPreparation,
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
        let (items, copies, provider) = store.existence()?;
        let now = work.now();
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
                now,
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
            copies,
            listed: None,
            current: Vec::new(),
            target: None,
            lookup_failed: None,
            incompatible: Default::default(),
            now,
            base,
            purged: Vec::new(),
            docs_changed: Vec::new(),
            board_setup_failure: None,
            pending_transfer: None,
            parent_preparation: super::_ops::ParentPreparation::NotAsked,
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

    /// The item `slug` addresses, refreshed from GitHub before the command
    /// acts on it (`github-record`, "Interest-driven refresh"): a local item
    /// is pulled through; a todo known only as a listing copy is read from
    /// GitHub and becomes a local item. A conflicted item refuses.
    pub fn enter(&mut self, slug: &TodoSlug) -> Result<usize, WorkError> {
        let index = match self.find(slug, false) {
            Some(index) => index,
            None => self.localize_slug(slug)?,
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

    /// Makes the live todo `slug`, known here only as a listing copy, a
    /// local item as GitHub holds it now. Acting on it needs its committed
    /// record and contents, so a GitHub that cannot be read refuses.
    fn localize_slug(&mut self, slug: &TodoSlug) -> Result<usize, WorkError> {
        let Some(copy) = self.known_copy(slug) else {
            return Err(self.absent(slug));
        };
        if !matches!(self.remote, Remote::Client { down: None, .. }) {
            return Err(self.unreadable(slug));
        }
        match self.refresh_copy(copy.issue_number)? {
            Some(item) if &item.record.slug == slug && item.record.is_live() => self.adopt(item),
            _ => Err(self.absent(slug)),
        }
    }

    /// The refusal for acting on `slug`, known here only as a listing copy,
    /// while GitHub cannot be read.
    fn unreadable(&self, slug: &TodoSlug) -> WorkError {
        let cause = match self.github_read() {
            GitHubRead::NotRead(reason) => describe(&reason),
            GitHubRead::Read => "GitHub's answer was not read".to_owned(),
        };
        WorkError::refused(format!(
            "`{slug}` is not on this computer, and acting on it needs its record from GitHub, which cannot be read now: {cause}"
        ))
    }

    /// Pulls item `index` from GitHub with a conditional request: a newer
    /// remote revision enters the local record, so the command acts on the
    /// todo's current state. A newer revision meeting a pending local
    /// change marks it conflicted, and a conflicted item refuses. A GitHub
    /// that cannot be read leaves the item as this computer holds it.
    ///
    /// No lock is held across the request. The row is re-read before it
    /// and the result is committed only while the row is unchanged; a row
    /// that changed meanwhile is pulled once more. A head that cannot be
    /// this computer's own post ([`_reflect::origin`]) is another
    /// computer's: that conflict is committed at once, and the command
    /// refuses. A head that may be this computer's own, posted by a
    /// reflection that has not recorded it yet, has its conflict judgment
    /// committed only while this command holds the reflection lock, taken
    /// without waiting. A busy
    /// lock leaves that head unjudged and the command proceeds on the local
    /// record; the revision check of this todo's next reflection, the
    /// command's own flush included, judges it (`github-record`,
    /// "Interest-driven refresh"). An item carrying this command's
    /// uncommitted change is judged as held here.
    pub fn pull_through(&mut self, index: usize) -> Result<(), WorkError> {
        let id = self.items[index].record.id.clone();
        if !self.current.contains(&id) {
            self.current.push(id.clone());
        }
        if self.uncommitted(index) {
            return self.refuse_conflicted(index);
        }
        if let Some(live) = self.store.item(&id)? {
            self.take_row(index, live);
        }
        for attempt in 0..2 {
            let base = self.items[index].clone();
            let mut pulled = base.clone();
            let result = match &self.remote {
                Remote::Client { gh, down: None } => Some(_reflect::pull(gh, &mut pulled)),
                _ => None,
            };
            if matches!(&result, Some(Ok(_))) && pulled.reflection.issue.is_some() {
                pulled.reflection.refreshed_at = Some(self.now);
            }
            let adopted = matches!(&result, Some(Ok(Pulled::Adopted)));
            match &result {
                Some(Err(err @ GitHubError::RecordSchema { .. })) => {
                    self.incompatible.insert(id.clone());
                    self.lookup_failure(err.clone());
                    return Err(err.clone().into());
                }
                Some(Err(err)) => self.note(err.clone()),
                _ => {}
            }
            let mut judging = None;
            if let Some(Ok(Pulled::Conflicted {
                origin: HeadOrigin::PossiblyOwn,
            })) = &result
            {
                judging = ReflectionLock::try_acquire(self.work.home())?;
                if judging.is_none() {
                    tracing::info!(
                        event = "work_conflict_left_to_reflection",
                        project = %self.project.slug,
                        todo = %pulled.record.slug,
                        "a reflection holds the reflection lock; the head is left unjudged, for the revision check of this todo's next reflection"
                    );
                    pulled = base.clone();
                }
            }
            let inbound = std::mem::take(&mut pulled.reflection.inbound);
            if _store::stored(&pulled) == _store::stored(&base) {
                self.items[index].reflection.inbound = inbound;
                break;
            }
            let swapped = self.store.swap(&base, &pulled)?;
            drop(judging);
            match swapped {
                Swapped::Applied(row) => {
                    if adopted {
                        tracing::info!(
                            event = "work_pulled",
                            project = %self.project.slug,
                            todo = %row.record.slug,
                            revision = row.record.revision,
                            "a newer remote revision entered the local record"
                        );
                    }
                    self.take_row(index, *row);
                    self.items[index].reflection.inbound = inbound;
                    break;
                }
                Swapped::Changed(Some(live)) => {
                    self.take_row(index, *live);
                    if attempt > 0 {
                        tracing::warn!(
                            event = "work_pull_superseded",
                            project = %self.project.slug,
                            todo = %self.items[index].record.slug,
                            "the todo kept changing; the command acts on the local record"
                        );
                    }
                }
                Swapped::Changed(None) => break,
            }
        }
        if !self.items[index].reflection.conflicted {
            self.answer_label_actions(index)?;
            self.answer_commands(index)?;
            self.restore_output_labels(index)?;
        }
        self.refuse_conflicted(index)
    }

    /// Refuses a command on item `index` when it is conflicted.
    fn refuse_conflicted(&self, index: usize) -> Result<(), WorkError> {
        if self.items[index].reflection.conflicted {
            return Err(WorkError::Conflicted {
                todo: self.items[index].record.slug.clone(),
            });
        }
        Ok(())
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

    /// Whether GitHub could be read for this command's refreshes so far.
    pub fn refreshed(&self) -> bool {
        matches!(self.remote, Remote::Client { down: None, .. })
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
                self.answered(index, id, &answer)?;
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
                    Ok(command) => self.apply_command(
                        index,
                        id,
                        comment["created_at"].as_str(),
                        &login,
                        command,
                    )?,
                }
            };
            self.answered(index, id, &answer)?;
        }
        // Replies are posted only by the holder of the reflection lock. A
        // busy lock leaves them pending: a later flush of this project that
        // takes the lock attempts the pending replies of each item whose
        // remote record is compatible, and any it cannot post stay pending
        // for the flush after it.
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

    /// Records command `comment` of item `index` as answered with `answer`.
    fn answered(&mut self, index: usize, comment: u64, answer: &str) -> Result<(), WorkError> {
        let id = self.items[index].record.id.clone();
        self.store.retain_reply(id.as_str(), comment, answer)?;
        let rows = std::iter::once(&mut self.items[index]).chain(self.bases.get_mut(&id));
        for item in rows {
            if !item.reflection.commands_seen.contains(&comment) {
                item.reflection.commands_seen.push(comment);
            }
        }
        Ok(())
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
        for (source, text) in self.store.pending_replies(todo)? {
            let result = match &self.remote {
                Remote::Client { gh, down: None } => _reflect::reply(gh, issue, source, &text),
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

    fn restore_output_labels(&mut self, index: usize) -> Result<(), WorkError> {
        let Some(issue) = self.items[index].reflection.issue else {
            return Ok(());
        };
        let snapshot = match &self.remote {
            Remote::Client { gh, down: None } => gh.read(&format!("/issues/{}", issue.number)),
            _ => return Ok(()),
        };
        let snapshot = match snapshot {
            Ok(snapshot) => snapshot,
            Err(error) => {
                self.note(error);
                return Ok(());
            }
        };
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

    /// Apply attributable label actions once, preserving their source ordering.
    fn answer_label_actions(&mut self, index: usize) -> Result<(), WorkError> {
        let Some(issue) = self.items[index].reflection.issue else {
            return Ok(());
        };
        let result = match &self.remote {
            Remote::Client { gh, down: None } => {
                gh.list(&format!("/issues/{}/events", issue.number))
            }
            _ => return Ok(()),
        };
        let mut actions = match result {
            Ok(actions) => actions,
            Err(error) => {
                tracing::warn!(event="github_label_actions_unreadable",issue=issue.number,error=%error);
                return Ok(());
            }
        };
        actions.sort_by_key(|a| a["id"].as_u64().unwrap_or(0));
        for action in actions {
            if action["label"]["name"].as_str() != Some("hold") {
                continue;
            }
            let (Some(id), Some(login), Some(kind), Some(at)) = (
                action["id"].as_u64(),
                action["actor"]["login"].as_str(),
                action["event"].as_str(),
                action["created_at"].as_str(),
            ) else {
                tracing::warn!(event = "github_hold_unattributed", issue = issue.number);
                continue;
            };
            if !matches!(kind, "labeled" | "unlabeled")
                || self.items[index]
                    .record
                    .label_actions
                    .iter()
                    .any(|a| a.issue == issue.number && a.id == id)
                || self.store.label_seen(issue.number, id)?
            {
                continue;
            }
            let source_at = match at.parse::<Timestamp>() {
                Ok(at) => at,
                Err(error) => {
                    tracing::warn!(event="github_hold_invalid_time",error=%error);
                    continue;
                }
            };
            let permission = match &self.remote {
                Remote::Client { gh, down: None } => gh.can_write(login),
                _ => return Ok(()),
            };
            let allowed = match permission {
                Ok(allowed) => allowed,
                Err(error) => {
                    tracing::warn!(event="github_hold_actor_unverified",login,error=%error);
                    break;
                }
            };
            let admitted =
                allowed && self.items[index].record.status != TodoStatus::StructuralHolder;
            if !admitted {
                self.store.ignore_label(
                    issue.number,
                    id,
                    self.items[index].record.id.as_str(),
                    kind,
                    login,
                    at,
                )?;
                continue;
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
        }
        Ok(())
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
            _store::retain_reply(tx,std::path::Path::new(tx.path().unwrap_or("")),&todo_id,comment_id,&answer)?;
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

    /// Pulls every local item, without refusing conflicted ones.
    pub fn pull_all(&mut self) -> Result<(), WorkError> {
        for index in 0..self.items.len() {
            match self.pull_through(index) {
                Ok(()) | Err(WorkError::Conflicted { .. }) => {}
                Err(WorkError::GitHub(error @ GitHubError::RecordSchema { .. })) => {
                    self.incompatible
                        .insert(self.items[index].record.id.clone());
                    self.current.retain(|id| id != &self.items[index].record.id);
                    tracing::warn!(event="work_incompatible_record",error=%error);
                }
                Err(err) => return Err(err),
            }
        }
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

    fn schema_ready(&self) -> Result<(), WorkError> {
        if self.current.iter().any(|id| self.incompatible.contains(id)) {
            return Err(self.lookup_failed.clone().map(WorkError::GitHub).unwrap_or_else(||WorkError::refused("a record this operation depends on has an incompatible schema; upgrade instrukt")));
        }
        Ok(())
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
        self.schema_ready()?;
        let before: Vec<LocalItem> = self.items.clone();
        let bases_before = self.bases.clone();
        let value = match change(self) {
            Ok(value) => value,
            Err(err) => {
                self.restore(before, &bases_before);
                return Err(err);
            }
        };
        if let Err(error) = self.schema_ready() {
            self.restore(before, &bases_before);
            return Err(error);
        }
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
            Err((err, conflicted)) => {
                self.restore(before, &bases_before);
                if let Some(live) = conflicted
                    && let Some(index) = self.find_id(&live.record.id)
                {
                    self.take_row(index, *live);
                }
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
    /// written, and a failing transfer writes nothing of its own. A todo a
    /// reflection marked conflicted since this command read it refuses the
    /// whole change, returning its live row. A change that moves waiting
    /// edges is refused whole when the graph the transaction holds, local
    /// rows and listing copies alike, would wait on itself through them.
    fn write_change(
        &mut self,
        changed: &[usize],
        write_related: impl FnOnce(&Transaction<'_>) -> Result<(), WorkError>,
        transfer: Option<&(TodoSlug, TodoSlug)>,
    ) -> Result<Vec<std::path::PathBuf>, (WorkError, Option<Box<LocalItem>>)> {
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
        let mut conflicted = None;
        let result = self.store.write(|tx, path, project| {
            let mut rows = Vec::new();
            for &index in changed {
                let mine = &self.items[index];
                let base = self.bases.get(&mine.record.id);
                match _store::commit_local(tx, path, project, base, mine, &others)? {
                    Ok(row) => rows.push((index, row)),
                    Err(Refusal::Conflicted(live)) => {
                        let todo = live.record.slug.clone();
                        conflicted = Some(live);
                        return Err(WorkError::Conflicted { todo });
                    }
                    Err(Refusal::Changed(now)) => {
                        return Err(self.changed_refusal(mine, base, now));
                    }
                    Err(Refusal::Taken) => {
                        return Err(WorkError::refused(format!(
                            "a live todo is already named `{}`; nothing of this change was written",
                            mine.record.slug
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
        let (rows, docs) = result.map_err(|err| (err, conflicted))?;
        for (index, row) in rows {
            let inbound = std::mem::take(&mut self.items[index].reflection.inbound);
            self.take_row(index, row);
            self.items[index].reflection.inbound = inbound;
        }
        Ok(docs)
    }

    /// Commits a change of item `index` whose making has local effects that
    /// may run only once the change is admitted, all in one writer
    /// transaction: the live row is judged against the row this command
    /// read first, so a refused change runs none of them; then `effect`
    /// makes the change on the item's record, running its effects, and the
    /// record commits as a new revision pending reflection. An error from
    /// `effect` writes nothing; it answers for whatever effects it already
    /// had. `effect` runs under the writer lock and so makes no network
    /// request.
    pub fn commit_admitted<T>(
        &mut self,
        what: &'static str,
        index: usize,
        effect: impl FnOnce(&mut TodoRecord) -> Result<T, WorkError>,
    ) -> Result<T, WorkError> {
        self.schema_ready()?;
        let mut mine = self.items[index].clone();
        let Some(base) = self.bases.get(&mine.record.id).cloned() else {
            return Err(WorkError::refused(format!(
                "`{}` has no stored record to change",
                mine.record.slug
            )));
        };
        let outcome = self.store.write(|tx, path, project| {
            if let Err(refusal) = _store::admit(tx, path, project, &base)? {
                return Ok(Err(refusal));
            }
            let value = effect(&mut mine.record)?;
            mine.record.revision += 1;
            mine.reflection.pending = true;
            Ok(
                _store::commit_local(tx, path, project, Some(&base), &mine, &[])?
                    .map(|row| (value, row)),
            )
        })?;
        let (value, row) = match outcome {
            Ok(done) => done,
            Err(Refusal::Conflicted(live)) => {
                let todo = live.record.slug.clone();
                self.take_row(index, *live);
                return Err(WorkError::Conflicted { todo });
            }
            Err(Refusal::Changed(now)) => {
                return Err(self.changed_refusal(&mine, Some(&base), now));
            }
            Err(Refusal::Taken) => {
                return Err(WorkError::refused(format!(
                    "a live todo is already named `{}`; nothing of this change was written",
                    mine.record.slug
                )));
            }
        };
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
        Ok(value)
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
        let Some(target) = related else {
            return WorkError::refused(match renamed {
                Some(now) => format!(
                    "`{read}` changed while this command ran and is now named `{now}`; nothing of it was written. Run the command again on `{now}`: it acts on the todo's current record"
                ),
                None => format!(
                    "`{read}` changed while this command ran; nothing of it was written. Run the command again: it acts on the todo's current record"
                ),
            });
        };
        let changed = match renamed {
            Some(now) => format!(
                "`{read}`, which this change also writes, changed while this command ran and is now named `{now}`"
            ),
            None => {
                format!("`{read}`, which this change also writes, changed while this command ran")
            }
        };
        let door = match self.bases.get(target) {
            Some(target) => format!("Run the command again on `{}`", target.record.slug),
            None => "Run the command again".to_owned(),
        };
        WorkError::refused(format!(
            "{changed}; nothing of it was written. {door}: it acts on the todos' current records"
        ))
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
            let account = gh.for_role(None, github::Operation::Board)?;
            let board = super::_board::setup(&account, known.as_ref())?;
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
            // A known incompatible remote must stop before acknowledgements.
            if let Remote::Client { gh, down: None } = &self.remote
                && loaded.reflection.issue.is_some()
            {
                match _reflect::check_compatible(gh, &loaded) {
                    Err(error @ GitHubError::RecordSchema { .. }) => {
                        let failure = error.to_string();
                        let row = self
                            .store
                            .amend_reflection(&id, |r| r.failure = Some(failure))?;
                        self.learned(row);
                        continue;
                    }
                    Err(error) => self.note(error),
                    Ok(_) => {}
                }
            }
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
            if owed && !reflected.reflection.conflicted {
                let needs_you = super::_envelope::record_status(&reflected.record) == "held"
                    || crate::workflow::waiting_for_human_approval(
                        &super::_ports::to_evidence(reflected.record.clone(), Vec::new()),
                        self.config
                            .approver(instrukt_core::config::ApprovalGate::Prd)?,
                        self.config
                            .approver(instrukt_core::config::ApprovalGate::Task)?,
                    );
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
            Remote::Client { down: None, .. } => match &self.lookup_failed {
                Some(err) => GitHubRead::NotRead(PendingReason::Failed(err.to_string())),
                None => GitHubRead::Read,
            },
        }
    }

    /// The pickup listing: every open todo issue with the head of its
    /// revision chain, read once per command. Its result replaces the
    /// project's listing copies with the todos that are not local items.
    /// Empty when GitHub cannot be read; the copies then stay as they were.
    pub fn pickup_listing(&mut self) -> Vec<(TodoRecord, IssueRef)> {
        if let Some(records) = &self.listed {
            return records.clone();
        }
        let result = match &self.remote {
            Remote::Client { gh, down: None } => Some(_reflect::remote_records(gh, true)),
            _ => None,
        };
        let records = match result {
            Some(Ok(remote)) => {
                for (id, error) in remote.incompatible {
                    self.incompatible.insert(id);
                    self.lookup_failure(error);
                }
                remote.records
            }
            Some(Err(err)) => {
                self.lookup_failure(err);
                self.listed = Some(Vec::new());
                return Vec::new();
            }
            None => {
                self.listed = Some(Vec::new());
                return Vec::new();
            }
        };
        for (record, issue) in &records {
            let Some(index) = self.find_id(&record.id) else {
                continue;
            };
            let Some(base) = self.bases.get(&record.id).cloned() else {
                continue;
            };
            if self.uncommitted(index) {
                continue;
            }
            let mut accepted = base.clone();
            if !_reflect::accept_listing(&mut accepted, record, *issue, self.now) {
                continue;
            }
            match self.store.swap(&base, &accepted) {
                Ok(Swapped::Applied(row) | Swapped::Changed(Some(row))) => {
                    self.take_row(index, *row);
                }
                Ok(Swapped::Changed(None)) => {}
                Err(error) => {
                    tracing::warn!(event="local_pickup_copy_unsaved",todo=%record.slug,error=%error);
                }
            }
        }
        let local: Vec<TodoId> = self.items.iter().map(|it| it.record.id.clone()).collect();
        let copies: Vec<ListingCopy> = records
            .iter()
            .filter(|(record, _)| !local.contains(&record.id))
            .map(|(record, issue)| ListingCopy {
                issue_number: issue.number,
                record: record.without_contents(),
                copied_at: self.now,
            })
            .collect();
        match self.store.replace_copies(&copies) {
            Ok(()) => self.copies = copies,
            Err(err) => tracing::warn!(
                event = "work_listing_copies_unsaved",
                project = %self.project.slug,
                error = %err,
                "cannot keep the pickup listing's copies"
            ),
        }
        self.listed = Some(records.clone());
        records
    }

    /// The live listing copy of `slug`; when there is none, the pickup
    /// listing is read once to find it (a todo this computer has never
    /// seen).
    pub fn known_copy(&mut self, slug: &TodoSlug) -> Option<ListingCopy> {
        let find = |copies: &[ListingCopy]| {
            copies
                .iter()
                .find(|c| &c.record.slug == slug && c.record.is_live())
                .cloned()
        };
        if let Some(copy) = find(&self.copies) {
            return Some(copy);
        }
        if self.listed.is_none() {
            self.pickup_listing();
            return find(&self.copies);
        }
        None
    }

    /// Reads the one todo on issue `number` from GitHub, with its verified
    /// contents, and replaces its listing copy; the todo does not become a
    /// local item. `None` when the issue holds no committed record.
    pub fn refresh_copy(&mut self, number: u64) -> Result<Option<LocalItem>, WorkError> {
        let result = match &self.remote {
            Remote::Client { gh, down: None } => _reflect::read_one(gh, number),
            _ => return Ok(None),
        };
        let item = match result {
            Ok(item) => item,
            Err(err) => return Err(self.github_failure(err)),
        };
        match &item {
            Some(item) => {
                let copy = ListingCopy {
                    issue_number: number,
                    record: item.record.without_contents(),
                    copied_at: self.now,
                };
                self.store.save_copy(&copy)?;
                self.copies.retain(|c| c.issue_number != number);
                self.copies.push(copy);
            }
            None => {
                self.store.drop_copy(number)?;
                self.copies.retain(|c| c.issue_number != number);
            }
        }
        Ok(item)
    }

    /// Every record this computer holds for the project: its local items',
    /// then the listing copies'. Reads nothing from GitHub.
    pub fn known_records(&self) -> Vec<TodoRecord> {
        let mut records: Vec<TodoRecord> = self.items.iter().map(|it| it.record.clone()).collect();
        for copy in &self.copies {
            if !records.iter().any(|r| r.id == copy.record.id) {
                records.push(copy.record.clone());
            }
        }
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
            // Withdrawn work past its recovery window is gone: GitHub keeps
            // the closed issue as history, not as recoverable work.
            Ok(remote) => {
                for (id, error) in remote.incompatible {
                    self.incompatible.insert(id);
                    self.lookup_failure(error);
                }
                Some(
                    remote
                        .records
                        .into_iter()
                        .filter(|(record, _)| !past_recovery(record, self.now))
                        .collect(),
                )
            }
            Err(err) => {
                self.lookup_failure(err);
                None
            }
        }
    }

    /// Records a failed listing of reflected work: what is not local can
    /// no longer be called absent this command.
    fn lookup_failure(&mut self, err: GitHubError) {
        if !err.is_reach() {
            self.lookup_failed = Some(err.clone());
        }
        self.note(err);
    }

    /// A record of reflected work read directly from GitHub, with its
    /// artifact contents, stored nowhere.
    /// The entry `address` names: a plain slug means the live item (local,
    /// else reflected); `<slug>@<prefix>` the exact entry, retained ones
    /// included. A plain slug with only retained entries is refused naming
    /// their addresses; a lookup GitHub could not answer is reported as
    /// undetermined, never as "no todo".
    pub fn locate(&mut self, address: &TodoAddress) -> Result<Located, WorkError> {
        let slug = &address.slug;
        if address.identity.is_none() {
            if let Some(index) = self.find(slug, false) {
                return Ok(Located::Local(index));
            }
            if let Some(copy) = self.known_copy(slug) {
                return self.located_copy(copy);
            }
            return Err(retained_only(&self.items, slug).unwrap_or_else(|| self.absent(slug)));
        }
        if let Some(index) = exact_local(&self.items, address)? {
            return Ok(Located::Local(index));
        }
        let copied: Vec<ListingCopy> = self
            .copies
            .iter()
            .filter(|c| address.names(&c.record))
            .cloned()
            .collect();
        if let [copy] = copied.as_slice() {
            return self.located_copy(copy.clone());
        }
        match self.remote_matching(&format!("`{address}`"), |r| address.names(r))? {
            Some(item) => Ok(Located::Remote {
                item: Box::new(item),
                refreshed: true,
                copied_at: self.now,
            }),
            None => Err(self.absent(slug)),
        }
    }

    /// The todo of listing copy `copy`, refreshed from GitHub when it can
    /// be read, else as last known.
    fn located_copy(&mut self, copy: ListingCopy) -> Result<Located, WorkError> {
        let stale = |copy: ListingCopy| Located::Remote {
            item: Box::new(LocalItem {
                record: copy.record,
                reflection: Default::default(),
            }),
            refreshed: false,
            copied_at: copy.copied_at,
        };
        if !self.refreshed() {
            return Ok(stale(copy));
        }
        match self.refresh_copy(copy.issue_number) {
            Ok(Some(item)) => Ok(Located::Remote {
                item: Box::new(item),
                refreshed: true,
                copied_at: self.now,
            }),
            Ok(None) => Err(self.absent(&copy.record.slug)),
            Err(WorkError::GitHub(err)) if err.is_reach() => Ok(stale(copy)),
            Err(err) => Err(err),
        }
    }

    /// The error for a todo neither this computer nor GitHub has: unknown
    /// when GitHub answered, undetermined (with the cause) when it could
    /// not.
    pub fn absent(&self, slug: &TodoSlug) -> WorkError {
        match self.github_read() {
            GitHubRead::Read => WorkError::UnknownTodo {
                project: self.project.slug.clone(),
                todo: slug.clone(),
            },
            GitHubRead::NotRead(_) => self.undetermined(&format!("`{slug}`")),
        }
    }

    /// The refusal when GitHub could not be read to find `what`, which
    /// this computer does not have: undetermined, with the cause.
    pub fn undetermined(&self, what: &str) -> WorkError {
        let cause = match self.github_read() {
            GitHubRead::NotRead(reason) => describe(&reason),
            GitHubRead::Read => "GitHub's answer was not read".to_owned(),
        };
        WorkError::refused(format!(
            "{what} is not on this computer, and whether it exists cannot be determined: {cause}"
        ))
    }

    /// Every reflected record not on this computer that `wanted` accepts;
    /// a GitHub that could not be read refuses as undetermined, naming
    /// `what` was looked for.
    pub fn remote_candidates(
        &mut self,
        what: &str,
        wanted: impl Fn(&TodoRecord) -> bool,
    ) -> Result<Vec<(TodoRecord, IssueRef)>, WorkError> {
        let Some(records) = self.remote_all() else {
            return Err(self.undetermined(what));
        };
        let local: Vec<TodoId> = self.items.iter().map(|it| it.record.id.clone()).collect();
        Ok(records
            .into_iter()
            .filter(|(record, _)| wanted(record) && !local.contains(&record.id))
            .collect())
    }

    /// The one reflected record `wanted` accepts, read with its verified
    /// contents and stored nowhere; `None` when GitHub confirms it holds
    /// none. Several are refused, naming each entry's exact address; a
    /// GitHub that could not be read refuses as undetermined.
    pub fn remote_matching(
        &mut self,
        what: &str,
        wanted: impl Fn(&TodoRecord) -> bool,
    ) -> Result<Option<LocalItem>, WorkError> {
        let mut found = self.remote_candidates(what, wanted)?;
        if found.len() > 1 {
            return Err(several(what, found.iter().map(|(record, _)| record)));
        }
        match found.pop() {
            Some((record, issue)) => self.localize_found(record, issue).map(Some),
            None => Ok(None),
        }
    }

    /// Reads a reflected record found on GitHub with its verified contents.
    pub fn localize_found(
        &mut self,
        record: TodoRecord,
        issue: IssueRef,
    ) -> Result<LocalItem, WorkError> {
        let result = match &self.remote {
            Remote::Client { gh, down: None } => _reflect::localize(gh, record, issue),
            _ => return Err(self.undetermined(&format!("`{}`", record.slug))),
        };
        result.map_err(|err| self.github_failure(err))
    }

    /// Restores from GitHub the content of each version of artifact `name`
    /// in `record`, a todo on `issue`, whose digest `digests` names and
    /// whose content `record` lacks, verified against that digest. Returns
    /// why each version still lacks it: only GitHub's answer for the
    /// artifact establishes that it does not serve a version, and a request
    /// that failed, or a GitHub that cannot be read, leaves it unread.
    pub fn restore_contents(
        &mut self,
        issue: Option<IssueRef>,
        record: &mut TodoRecord,
        name: &str,
        digests: &[String],
    ) -> std::collections::BTreeMap<String, Unrestored> {
        let mut missing = std::collections::BTreeMap::new();
        let unread = match &self.remote {
            Remote::Client { down: None, .. } => None,
            Remote::Client {
                down: Some(err), ..
            } => Some(reason_of(err)),
            Remote::Unavailable(reason) => Some(reason.clone()),
        };
        let slug = record.slug.clone();
        let Some(artifact) = record
            .artifacts
            .iter_mut()
            .find(|a| a.name.as_str() == name)
        else {
            return missing;
        };
        let address = artifact.address;
        let mut served: Option<Result<std::collections::BTreeMap<u64, String>, Unrestored>> =
            unread.map(|reason| Err(Unrestored::Unread(reason)));
        let wanted = artifact
            .versions
            .iter_mut()
            .filter(|v| v.content.is_none() && digests.iter().any(|d| d == v.digest.as_str()));
        for version in wanted {
            let digest = version.digest.as_str().to_owned();
            if served.is_none() {
                // GitHub's record holds no copy of a version it carries no
                // provider address for; there is nothing to ask for.
                let (Some(issue), Some(address), Some(_)) = (issue, address, version.remote) else {
                    missing.insert(digest, Unrestored::NotServed);
                    continue;
                };
                let Remote::Client { gh, down: None } = &self.remote else {
                    // Unreachable: a GitHub that cannot be read set `served`.
                    continue;
                };
                let read = _reflect::served_versions(gh, issue.number, address);
                served = Some(read.map_err(|err| {
                    let reason = reason_of(&err);
                    self.note(err);
                    Unrestored::Unread(reason)
                }));
            }
            let body = match (&served, version.remote) {
                (Some(Ok(bodies)), Some(number)) => bodies.get(&number),
                (Some(Err(why)), _) => {
                    missing.insert(digest, why.clone());
                    continue;
                }
                _ => None,
            };
            match body {
                Some(body) if super::_model::digest_of(body) == version.digest => {
                    version.content = Some(body.clone());
                }
                _ => {
                    tracing::warn!(
                        event = "work_artifact_unverifiable",
                        todo = %slug,
                        artifact = name,
                        digest = %digest,
                        "GitHub does not serve this artifact version's recorded content"
                    );
                    missing.insert(digest, Unrestored::NotServed);
                }
            }
        }
        missing
    }

    /// Commits content restored from GitHub onto item `index`'s row, with
    /// `restored` the item's record holding it, while the row is unchanged.
    /// A row another command changed meanwhile keeps its own record; the
    /// verified content then serves only the read that restored it.
    pub fn keep_restored(&mut self, index: usize, restored: &TodoRecord) -> Result<(), WorkError> {
        let base = self.items[index].clone();
        let next = _store::carry(
            &LocalItem {
                record: restored.clone(),
                reflection: base.reflection.clone(),
            },
            &base,
        );
        if next.record == base.record {
            return Ok(());
        }
        if let Swapped::Applied(row) = self.store.swap(&base, &next)? {
            let inbound = std::mem::take(&mut self.items[index].reflection.inbound);
            self.take_row(index, *row);
            self.items[index].reflection.inbound = inbound;
        }
        Ok(())
    }

    /// Makes a reflected record found by [`Session::locate`] a local item.
    pub fn adopt(&mut self, item: LocalItem) -> Result<usize, WorkError> {
        // Saving drops its listing copy (`listing_copies`). A todo another
        // command made a local item meanwhile is taken as stored.
        let row = self.store.adopt(&item)?;
        if let Some(issue) = item.reflection.issue {
            self.copies.retain(|c| c.issue_number != issue.number);
        }
        self.bases.insert(row.record.id.clone(), row.clone());
        self.items.push(row);
        Ok(self.items.len() - 1)
    }

    /// The reflected entry `slug` names, not on this computer: its live
    /// entry, else its only retained one. Several retained entries are
    /// refused naming each exact address; `None` only when GitHub confirms
    /// it holds none.
    pub fn remote_item(&mut self, slug: &TodoSlug) -> Result<Option<LocalItem>, WorkError> {
        let what = format!("`{slug}`");
        let found = self.remote_candidates(&what, |record| &record.slug == slug)?;
        let live: Vec<_> = found.iter().filter(|(r, _)| r.is_live()).cloned().collect();
        let mut chosen = if live.is_empty() { found } else { live };
        if chosen.len() > 1 {
            return Err(several(&what, chosen.iter().map(|(record, _)| record)));
        }
        match chosen.pop() {
            Some((record, issue)) => self.localize_found(record, issue).map(Some),
            None => Ok(None),
        }
    }

    /// Replaces item `index` with GitHub's committed record, under the
    /// reflection lock: the head it reads is settled, never one another
    /// reflection posted and has not recorded.
    pub fn refresh(&mut self, index: usize) -> Result<(), WorkError> {
        let _reflecting = ReflectionLock::acquire(self.work.home())?;
        let id = self.items[index].record.id.clone();
        if let Some(live) = self.store.item(&id)? {
            self.take_row(index, live);
        }
        let base = self.items[index].clone();
        let mut refreshed = base.clone();
        let result = match &self.remote {
            Remote::Client { gh, down: None } => _reflect::refresh(gh, &mut refreshed),
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
        match self.store.swap(&base, &refreshed)? {
            Swapped::Applied(row) => self.take_row(index, *row),
            Swapped::Changed(_) => {
                return Err(WorkError::refused(format!(
                    "`{}` changed while it was refreshed; nothing was replaced. Run the command again: it reads the todo's current record",
                    base.record.slug
                )));
            }
        }
        tracing::info!(
            event = "work_refreshed",
            project = %self.project.slug,
            todo = %self.items[index].record.slug,
            revision = self.items[index].record.revision,
            "local item replaced by GitHub's committed record"
        );
        Ok(())
    }

    /// A record this computer holds by identity: a local item's, else a
    /// listing copy's (the pickup listing is read once when it has
    /// neither).
    pub fn record_by_id(&mut self, id: &TodoId) -> Option<TodoRecord> {
        let find = |s: &Self| s.known_records().into_iter().find(|r| &r.id == id);
        if let Some(record) = find(self) {
            return Some(record);
        }
        if self.listed.is_none() {
            self.pickup_listing();
            return find(self);
        }
        None
    }

    /// The record of todo `id` as GitHub holds it now, for judging a
    /// relationship the command records (a prerequisite edge or a group):
    /// each todo the judgement consults is refreshed once per command, a
    /// local item pulled through and a listing copy read again. What
    /// GitHub cannot answer is judged as this computer holds it.
    pub fn record_current(&mut self, id: &TodoId) -> Option<TodoRecord> {
        if !self.current.contains(id) {
            self.current.push(id.clone());
            if let Some(index) = self.find_id(id) {
                if let Err(err) = self.pull_through(index) {
                    tracing::debug!(event = "work_relationship_unrefreshed", error = %err, "judged as held here");
                }
            } else if let Some(copy) = self.copies.iter().find(|c| &c.record.id == id).cloned()
                && self.refreshed()
                && let Err(err) = self.refresh_copy(copy.issue_number)
            {
                if let WorkError::GitHub(error @ GitHubError::RecordSchema { .. }) = &err {
                    self.incompatible.insert(id.clone());
                    self.lookup_failure(error.clone());
                }
                tracing::debug!(event = "work_relationship_unrefreshed", error = %err, "judged as last known");
            }
        }
        self.record_by_id(id)
    }

    /// The live todo `slug` names, a local item or a listing copy.
    pub fn live_ref(&mut self, slug: &TodoSlug) -> Option<TodoRecord> {
        if let Some(index) = self.find(slug, false) {
            return Some(self.items[index].record.clone());
        }
        self.known_copy(slug).map(|copy| copy.record)
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

    /// A GitHub failure of a read that has no local answer.
    fn github_failure(&mut self, err: GitHubError) -> WorkError {
        if err.is_reach() {
            self.note(err.clone());
        }
        WorkError::GitHub(err)
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

/// Where `item`'s reflection stands; `unread` is why GitHub cannot be
/// reflected to this command, `None` when it can.
pub(crate) fn status_of(item: &LocalItem, unread: Option<PendingReason>) -> ReflectionStatus {
    let reflection = &item.reflection;
    if reflection.conflicted {
        return ReflectionStatus::Conflicted;
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
