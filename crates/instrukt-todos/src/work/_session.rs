//! One command's hold on a project's work record: the writer lock, the
//! loaded local items, the GitHub client and its reach, and the shared
//! steps every operation composes — entering an item, pulling it through,
//! committing a change and flushing pending reflection.

use instrukt_core::config::Configuration;
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
use super::_reflect::{self, Pulled};
use super::_store::{self, IssueRef, ListingCopy, LocalItem, Store};
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
    remote: Remote,
    /// The last known copies of the project's todos that are not local
    /// items (`listing_copies`).
    pub copies: Vec<ListingCopy>,
    /// The pickup listing read this command: every open todo issue with
    /// the head of its revision chain. `None` until read.
    listed: Option<Vec<(TodoRecord, IssueRef)>>,
    /// The todos refreshed for this command's relationship checks.
    current: Vec<TodoId>,
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
    /// Takes the writer lock and loads the project's local record. Purges
    /// withdrawn work whose recovery window has expired.
    pub fn open(work: &'w Work, project: &ProjectRef) -> Result<Self, WorkError> {
        // Opening the store takes the writer lock, held for the whole
        // command.
        let store = Store::open(work.home(), &project.slug)?;
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
        let items = store.items()?;
        let copies = store.copies()?;
        let now = work.now();
        let remote = match store.provider()?.deferred_until {
            Some(until) if until > now => {
                Remote::Unavailable(PendingReason::RateLimited { until: Some(until) })
            }
            _ => match GitHub::for_app(
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
        let mut session = Self {
            work,
            project: project.clone(),
            config,
            store,
            items,
            remote,
            copies,
            listed: None,
            current: Vec::new(),
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

    /// The item `slug` addresses, refreshed from GitHub before the command
    /// acts on it (`github-record`, "Interest-driven refresh"): a local item
    /// is pulled through; a todo known only as a listing copy is read from
    /// GitHub and becomes a local item. A conflicted item refuses.
    pub fn enter(&mut self, slug: &TodoSlug) -> Result<usize, WorkError> {
        let index = match self.find(slug, false) {
            Some(index) => index,
            None => self.localize_slug(slug)?,
        };
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
    pub fn pull_through(&mut self, index: usize) -> Result<(), WorkError> {
        let id = self.items[index].record.id.clone();
        if !self.current.contains(&id) {
            self.current.push(id);
        }
        let before = self.items[index].clone();
        let result = match &self.remote {
            Remote::Client { gh, down: None } => Some(_reflect::pull(gh, &mut self.items[index])),
            _ => None,
        };
        if matches!(&result, Some(Ok(_))) && self.items[index].reflection.issue.is_some() {
            self.items[index].reflection.refreshed_at = Some(self.now);
        }
        match result {
            Some(Ok(Pulled::Adopted)) => tracing::info!(
                event = "work_pulled",
                project = %self.project.slug,
                todo = %self.items[index].record.slug,
                revision = self.items[index].record.revision,
                "a newer remote revision entered the local record"
            ),
            Some(Err(err @ GitHubError::RecordSchema { .. })) => {
                self.incompatible
                    .insert(self.items[index].record.id.clone());
                self.lookup_failure(err.clone());
                return Err(err.into());
            }
            Some(Err(err)) => self.note(err),
            _ => {}
        }
        if self.items[index] != before {
            self.store.save(&self.items[index])?;
        }
        if !self.items[index].reflection.conflicted {
            self.answer_label_actions(index)?;
            self.answer_commands(index)?;
            self.restore_output_labels(index)?;
        }
        if self.items[index].reflection.conflicted {
            return Err(WorkError::Conflicted {
                todo: self.items[index].record.slug.clone(),
            });
        }
        Ok(())
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
                if !self.items[index].reflection.commands_seen.contains(&id) {
                    self.items[index].reflection.commands_seen.push(id);
                }
                self.store
                    .save_with_reply(&self.items[index], id, &answer)?;
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
                        self.items[index].reflection.etag = None;
                        self.store.save(&self.items[index])?;
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
            if !self.items[index].reflection.commands_seen.contains(&id) {
                self.items[index].reflection.commands_seen.push(id);
            }
            self.store
                .save_with_reply(&self.items[index], id, &answer)?;
        }
        self.flush_command_replies(index)
    }

    fn flush_command_replies(&mut self, index: usize) -> Result<(), WorkError> {
        let Some(issue) = self.items[index].reflection.issue else {
            return Ok(());
        };
        let todo = self.items[index].record.id.to_string();
        for (source, text) in self.store.pending_replies(&todo)? {
            let result = match &self.remote {
                Remote::Client { gh, down: None } => _reflect::reply(gh, issue, source, &text),
                _ => break,
            };
            match result {
                Ok(()) => self.store.reply_sent(&todo, source)?,
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
            self.items[index].reflection.pending = true;
            self.store.save(&self.items[index])?;
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
    pub fn commit_with<T>(
        &mut self,
        what: &'static str,
        change: impl FnOnce(&mut Self) -> Result<T, WorkError>,
        write_related: impl FnOnce(&Transaction<'_>) -> Result<(), WorkError>,
    ) -> Result<T, WorkError> {
        self.schema_ready()?;
        let before: Vec<LocalItem> = self.items.clone();
        let value = match change(self) {
            Ok(value) => value,
            Err(err) => {
                self.restore(before);
                return Err(err);
            }
        };
        if let Err(error) = self.schema_ready() {
            self.restore(before);
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
            Err(err) => {
                self.restore(before);
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
    /// rewrote. Nothing commits unless everything succeeded: documentation
    /// moves only once every row is written, and a failing transfer writes
    /// nothing of its own.
    fn write_change(
        &mut self,
        changed: &[usize],
        write_related: impl FnOnce(&Transaction<'_>) -> Result<(), WorkError>,
        transfer: Option<&(TodoSlug, TodoSlug)>,
    ) -> Result<Vec<std::path::PathBuf>, WorkError> {
        let path = self.store.path().to_path_buf();
        let tx = self.store.transaction()?;
        for &index in changed {
            _store::save_in(&tx, &path, &self.items[index])?;
        }
        write_related(&tx)?;
        let docs = match transfer {
            Some((parent, child)) => {
                super::_ops::transfer_documentation(&self.project, parent, child)?
            }
            None => Vec::new(),
        };
        tx.commit()
            .map_err(instrukt_core::state_db::StateDbError::sqlite(&path))?;
        Ok(docs)
    }

    /// Appends `event` to the runner's event log in a transaction of its
    /// own.
    pub fn log_event(&mut self, log: &dyn EventLog, event: &Event) -> Result<(), WorkError> {
        let path = self.store.path().to_path_buf();
        let tx = self.store.transaction()?;
        let appended = log.append(&tx, event)?;
        tx.commit()
            .map_err(instrukt_core::state_db::StateDbError::sqlite(&path))?;
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
    /// drops the change's pending documentation transfer.
    fn restore(&mut self, before: Vec<LocalItem>) {
        self.items = before;
        self.pending_transfer = None;
    }

    /// Flushes pending reflection of every local item under the lock.
    pub fn flush(&mut self) -> Result<(), WorkError> {
        let board_result = (|| {
            let configured = self.config.board()?;
            let gh = match &self.remote {
                Remote::Client { gh, down: None } => gh,
                Remote::Client {
                    down: Some(error), ..
                } => return Err(error.clone().into()),
                Remote::Unavailable(reason) => return Err(WorkError::refused(describe(reason))),
            };
            let account = gh.for_role(None, github::Operation::Board)?;
            let board = super::_board::setup(&account, configured.as_ref())?;
            if configured.as_ref() != Some(&board) {
                let layer = instrukt_core::config::Layer::Project(self.project.root.clone());
                let revision = instrukt_core::config::revision(self.home(), &layer)?;
                let patch =
                    serde_yaml_ng::from_str(&format!("board: {}/{}", board.owner, board.number))
                        .map_err(|e| WorkError::refused(e.to_string()))?;
                instrukt_core::config::patch(self.home(), &layer, &revision, &patch)?;
                self.config = Configuration::load(self.home(), Some(&self.project.root))?;
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
        for index in 0..self.items.len() {
            // A known incompatible remote must stop before acknowledgements.
            if let Remote::Client { gh, down: None } = &self.remote
                && self.items[index].reflection.issue.is_some()
            {
                match _reflect::check_compatible(gh, &self.items[index]) {
                    Err(error @ GitHubError::RecordSchema { .. }) => {
                        self.items[index].reflection.failure = Some(error.to_string());
                        self.store.save(&self.items[index])?;
                        continue;
                    }
                    Err(error) => self.note(error),
                    Ok(_) => {}
                }
            }
            if !self.items[index].reflection.pending {
                self.items[index].reflection.failure = None;
            }
            let before_setup = self.items[index].clone();
            if let Some(error) = &board_error {
                self.items[index].reflection.board_failure = Some(error.clone());
            }
            if self.items[index] != before_setup {
                self.store.save(&self.items[index])?;
            }
            self.flush_command_replies(index)?;
            let owed = self.items[index].reflection.pending
                || _reflect::pull_request_owed(&self.items[index])
                || (board.is_some() && self.items[index].reflection.issue.is_some());
            if !owed || self.items[index].reflection.conflicted {
                continue;
            }
            let before = self.items[index].clone();
            let needs_you = super::_envelope::record_status(&self.items[index].record) == "held"
                || crate::workflow::waiting_for_human_approval(
                    &super::_ports::to_evidence(self.items[index].record.clone(), Vec::new()),
                    self.config
                        .approver(instrukt_core::config::ApprovalGate::Prd)?,
                    self.config
                        .approver(instrukt_core::config::ApprovalGate::Task)?,
                );
            let result = match &self.remote {
                Remote::Client { gh, down: None } => Some(_reflect::flush(
                    gh,
                    &mut self.items[index],
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
                        todo = %self.items[index].record.slug,
                        error = %err,
                        "reflection failed; the local record keeps the change"
                    );
                    self.items[index].reflection.failure = Some(err.to_string());
                }
            }
            if let Some(error) = &board_error {
                self.items[index].reflection.board_failure = Some(error.clone());
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
        if let Some(error) = &reflection.failure {
            return ReflectionStatus::Pending(PendingReason::Failed(error.clone()));
        }
        // Outstanding pull-request creation, reviews or closing are
        // reflection still owed.
        let pull_owed = _reflect::pull_request_owed(&self.items[index]);
        if let Some(error) = &reflection.board_failure {
            return ReflectionStatus::Pending(PendingReason::Failed(error.clone()));
        }
        if !reflection.pending && !pull_owed {
            return ReflectionStatus::Reflected;
        }
        ReflectionStatus::Pending(match &self.remote {
            Remote::Unavailable(reason) => reason.clone(),
            Remote::Client {
                down: Some(err), ..
            } => reason_of(err),
            Remote::Client { down: None, .. } if reflection.pending => PendingReason::Failed(
                reflection
                    .failure
                    .clone()
                    .unwrap_or_else(|| "not yet reflected".to_owned()),
            ),
            Remote::Client { down: None, .. } => PendingReason::Failed(
                reflection
                    .pull_outstanding
                    .clone()
                    .unwrap_or_else(|| "the pull request is not yet reflected".to_owned()),
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
            if let Some(index) = self.find_id(&record.id) {
                let before = self.items[index].clone();
                if _reflect::accept_listing(&mut self.items[index], record, *issue, self.now)
                    && let Err(error) = self.store.save(&self.items[index])
                {
                    self.items[index] = before;
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
            let retained: Vec<String> = self
                .items
                .iter()
                .map(|it| &it.record)
                .filter(|r| &r.slug == slug)
                .map(|r| format!("`{}` ({})", TodoAddress::exact(r), status_word(r.status)))
                .collect();
            if retained.is_empty() {
                return Err(self.absent(slug));
            }
            return Err(WorkError::refused(format!(
                "no live todo is named `{slug}`; address a retained entry exactly: {}",
                retained.join(", ")
            )));
        }
        let local: Vec<usize> = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, it)| address.names(&it.record))
            .map(|(i, _)| i)
            .collect();
        match local.as_slice() {
            [index] => return Ok(Located::Local(*index)),
            [] => {}
            many => {
                let ids: Vec<String> = many
                    .iter()
                    .map(|i| self.items[*i].record.id.to_string())
                    .collect();
                return Err(WorkError::refused(format!(
                    "`{address}` names several entries; give more of the identity: {}",
                    ids.join(", ")
                )));
            }
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

    /// Makes a reflected record found by [`Session::locate`] a local item.
    pub fn adopt(&mut self, item: LocalItem) -> Result<usize, WorkError> {
        // Saving drops its listing copy (`listing_copies`).
        self.store.save(&item)?;
        if let Some(issue) = item.reflection.issue {
            self.copies.retain(|c| c.issue_number != issue.number);
        }
        self.items.push(item);
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

    /// Deletes the item from the local record and this session.
    fn delete(&mut self, index: usize) -> Result<(), WorkError> {
        let item = self.items.remove(index);
        self.store.delete(&item.record.id)
    }

    fn purge(&mut self) -> Result<(), WorkError> {
        let now = self.now;
        let mut index = 0;
        while index < self.items.len() {
            if past_recovery(&self.items[index].record, now) {
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

/// Whether `record` is withdrawn work whose recovery window has passed at
/// `now`.
fn past_recovery(record: &TodoRecord, now: Timestamp) -> bool {
    matches!(
        (&record.status, &record.removal),
        (TodoStatus::Removed, Some(removal))
            if removal.at.checked_add(RECOVERY_WINDOW).is_ok_and(|end| end <= now)
    )
}

fn reason_of(err: &GitHubError) -> PendingReason {
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
