//! The runner's per-tick GitHub reflection pass (`github-record`,
//! `work-dispatch` "Execution contract"): the single path that acquires
//! `state/reflection.lock`, reads inbound human controls (`hold` label
//! actions and `/instrukt` commands) before pickup, and publishes work
//! branches and flushes pending reflection after pickup.
//!
//! One [`ReflectionSession`] spans a single tick: `begin` resets the
//! per-pass GitHub App cache once (`UC-GA1`), and any command-wide failure
//! (`Remote::Unavailable` or `Remote::Client { down: Some(_) }`) recorded
//! during `pull_inbound` stays on the session into `flush_pass` so zero
//! further GitHub requests are sent on that tick (`github-apps`,
//! "Installation credentials").

use instrukt_core::config::BoardRef;
use instrukt_core::git;
use instrukt_core::github;
use instrukt_core::publication::EvalFingerprint;
use instrukt_runner::interfaces::{GitHubReach, ProjectRef, ReflectionReport};
use jiff::Timestamp;

use super::_apps::Clock;
use super::_error::WorkError;
use super::_github::{GitHub, GitHubError};
use super::_inbound;
use super::_reflect;
use super::_session::{PendingReason, ReflectionStatus, Session, describe, status_of};
use super::_store::{self, IssueRef, ProviderState, ReflectionLock};
use super::Work;
use crate::evidence::{
    Actor, ApprovalGate, ApprovedBy, ModelProvenance, StampFact, StampRecord, TodoStatus,
    artifact_names,
};

enum Remote {
    Client {
        gh: Box<GitHub>,
        /// Set when GitHub stopped answering during this pass.
        down: Option<GitHubError>,
    },
    Unavailable(PendingReason),
}

/// Runner-only GitHub reflection pass for one project on one tick.
pub(super) struct ReflectionSession<'w> {
    session: Session<'w>,
    api_url: String,
    repository: String,
    clock: Clock,
    remote: Option<Remote>,
    /// Why this pass's flush could not take the reflection lock:
    /// another process reflected for longer than the bounded wait.
    reflection_busy: Option<String>,
    /// A cockpit setup failure retained independently of item reflection.
    board_setup_failure: Option<String>,
}

impl<'w> ReflectionSession<'w> {
    /// Opens a reflection pass for `project`, resetting the per-pass GitHub
    /// App credential cache once and deferring client construction until the
    /// first GitHub need. Takes no lock and performs no GitHub I/O.
    pub(super) fn begin(work: &'w Work, project: &ProjectRef) -> Result<Self, WorkError> {
        work.github.begin_pass().map_err(GitHubError::from)?;
        let (session, api_url, repository) = Session::open_parts(work, project, false)?;
        // Anchored here, before any capability lock wait, so a wait for it
        // is part of every GitHub answer's elapsed time.
        let clock = Clock::starting(session.now);
        Ok(Self {
            session,
            api_url,
            repository,
            clock,
            remote: None,
            reflection_busy: None,
            board_setup_failure: None,
        })
    }

    /// Reads people's inbound controls (`hold` label actions, `/instrukt`
    /// commands, and output-label drift) from the issues of live local todos
    /// before `pickup`, under `reflection.lock`, and releases the lock before
    /// returning. Carries any command-wide GitHub failure on `self.remote`
    /// into the subsequent [`Self::flush_pass`].
    pub(super) fn pull_inbound(&mut self) -> Result<(), WorkError> {
        self.session.reload()?;
        let live: Vec<usize> = (0..self.session.items.len())
            .filter(|&index| {
                let item = &self.session.items[index];
                item.reflection.issue.is_some()
                    && !matches!(
                        item.record.status,
                        TodoStatus::Delivered | TodoStatus::Removed
                    )
            })
            .collect();
        if live.is_empty() {
            return Ok(());
        }
        let lock = match ReflectionLock::acquire(self.session.home()) {
            Ok(lock) => lock,
            Err(WorkError::Fs(busy @ instrukt_core::fsx::FsError::LockTimeout { .. })) => {
                tracing::warn!(
                    event = "work_reflection_busy",
                    project = %self.session.project.slug,
                    error = %busy,
                    "another reflection holds the reflection lock; skipping inbound pull for this tick"
                );
                return Ok(());
            }
            Err(error) => return Err(error),
        };
        self.ensure_remote()?;
        for index in live {
            if self.down_reason().is_some() {
                break;
            }
            self.pull_through(&lock, index)?;
        }
        drop(lock);
        Ok(())
    }

    /// Reloads local items (picking up `pickup`'s writes), publishes work
    /// branches outside `reflection.lock`, then flushes pending reflection
    /// under `reflection.lock`, returning the pass report.
    pub(super) fn finish(mut self) -> Result<ReflectionReport, WorkError> {
        self.flush_pass()?;
        if let Some(cause) = &self.board_setup_failure {
            return Err(WorkError::refused(format!(
                "cockpit setup pending: {cause}"
            )));
        }
        let reach = match self.down_reason() {
            None => GitHubReach::Reachable,
            Some(PendingReason::RateLimited { until }) => GitHubReach::RateLimited { until },
            Some(_) => GitHubReach::Unreachable,
        };
        let pending = self
            .session
            .items
            .iter()
            .filter(|item| matches!(status_of(item), ReflectionStatus::Pending(_)))
            .map(|item| item.record.slug.clone())
            .collect();
        Ok(ReflectionReport { reach, pending })
    }

    /// Initializes `self.remote` on first GitHub need, preserving any client
    /// or command-wide failure already established on this pass.
    fn ensure_remote(&mut self) -> Result<(), WorkError> {
        if self.remote.is_some() {
            return Ok(());
        }
        let provider = self.session.store.provider()?;
        let remote = match unavailable(self.session.work, &provider, self.session.now) {
            Some(reason) => Remote::Unavailable(reason),
            None => match GitHub::for_app(
                self.session.work.github.clone(),
                &self.api_url,
                &self.repository,
                None,
                github::Operation::Record,
                self.clock,
            ) {
                Ok(gh) => Remote::Client {
                    gh: Box::new(gh),
                    down: None,
                },
                Err(err) => Remote::Unavailable(reason_of(&err)),
            },
        };
        self.remote = Some(remote);
        Ok(())
    }

    fn down_reason(&self) -> Option<PendingReason> {
        match self.remote.as_ref() {
            Some(Remote::Unavailable(reason)) => Some(reason.clone()),
            Some(Remote::Client {
                down: Some(err), ..
            }) => Some(reason_of(err)),
            _ => None,
        }
    }

    /// Reads item `index`'s issue for people's controls and applies them
    /// (`github-record`, Reading GitHub): `hold` label actions and
    /// `/instrukt` commands, each once, and output labels someone changed
    /// are reflected again. GET-only on GitHub; queued command replies are
    /// posted during [`Self::flush_pass`].
    fn pull_through(&mut self, _lock: &ReflectionLock, index: usize) -> Result<(), WorkError> {
        let id = self.session.items[index].record.id.clone();
        if self.session.uncommitted(index) {
            return Ok(());
        }
        if let Some(live) = self.session.store.item(&id)? {
            self.session.take_row(index, live);
        }
        let mut read = self.session.items[index].reflection.clone();
        let result = match self.remote.as_ref() {
            Some(Remote::Client { gh, down: None }) => _reflect::read_issue(gh, &mut read),
            _ => return Ok(()),
        };
        let fresh = match result {
            Err(err) => {
                self.note(err);
                return Ok(());
            }
            Ok(Some(fresh)) => fresh,
            Ok(None) => {
                if read.issue.is_none() && self.session.items[index].reflection.issue.is_some() {
                    // The issue is gone: the next flush reflects the item
                    // again.
                    let row = self
                        .session
                        .store
                        .amend_reflection(&id, |r| *r = read.clone())?;
                    self.session.learned(row);
                    return Ok(());
                }
                if read.issue.is_some() {
                    // Read, and unchanged since its last read.
                    let now = self.session.now;
                    self.session.amend(index, |r| r.refreshed_at = Some(now))?;
                }
                return Ok(());
            }
        };
        let fresh_etag = read.etag.take();
        let commands: Vec<u64> = read
            .inbound
            .iter()
            .filter_map(|comment| comment["id"].as_u64())
            .collect();
        self.session.items[index].reflection.inbound = std::mem::take(&mut read.inbound);
        // While the todo's withdrawal is open, what the issue asks of it
        // waits: once withdrawal commits, the runner stops reading its issue;
        // if a refusal removes the intent or the todo is later reactivated,
        // the dropped ETag ensures the next reflection read finds anything
        // still outstanding (`state-database`, `withdrawal_intents`).
        if self.session.store.withdrawal_intent(&id)?.is_some() {
            self.session.items[index].reflection.inbound.clear();
            return Ok(());
        }
        let events = self.answer_label_actions(index)?;
        self.answer_commands(index)?;
        self.restore_output_labels(index, &fresh.issue)?;
        let seen = &self.session.items[index].reflection.commands_seen;
        let comments: Vec<(Timestamp, bool)> = fresh
            .comments
            .iter()
            .map(|(comment, at)| (*at, commands.contains(comment) && !seen.contains(comment)))
            .collect();
        let held = self.session.items[index].reflection.clone();
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
            project = %self.session.project.slug,
            todo = %self.session.items[index].record.slug,
            comments = fresh.comments.len(),
            events = events.as_ref().map_or(0, Vec::len),
            outstanding,
            "people's controls read from the todo's issue"
        );
        let now = self.session.now;
        self.session.amend(index, |r| {
            r.refreshed_at = Some(now);
            r.comments_since.clone_from(&comments_since);
            r.events_page = events_page;
            r.events_last = events_last;
            r.etag.clone_from(&etag);
        })
    }

    /// Answers the approvers' `/instrukt` commands the last pull found on
    /// item `index`'s issue (`github-record`, "Writer and convergence").
    /// Queued replies are posted during [`Self::flush_pass`].
    fn answer_commands(&mut self, index: usize) -> Result<(), WorkError> {
        let commands = std::mem::take(&mut self.session.items[index].reflection.inbound);
        if self.session.items[index].reflection.issue.is_none() {
            return Ok(());
        }
        let approvers = self
            .session
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
            if let Some(prior) = self.session.items[index]
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
                let result = match self.remote.as_ref() {
                    Some(Remote::Client { gh, down: None }) => gh.can_write(&login),
                    _ => break,
                };
                match result {
                    Ok(allowed) => allowed,
                    Err(error) => {
                        tracing::warn!(event="approval_actor_unverified",login,error=%error);
                        self.session
                            .amend(index, |reflection| reflection.etag = None)?;
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
                            let todo = self.session.items[index].record.id.clone();
                            self.session.store.forget_etag(&todo)?;
                            return Err(err);
                        }
                    },
                }
            };
            if !self.answered(index, id, comment["created_at"].as_str(), &answer)? {
                break;
            }
        }
        Ok(())
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
        let id = self.session.items[index].record.id.clone();
        if !self.session.store.retain_reply(
            &self.session.items[index].record,
            comment,
            commanded_at,
            answer,
        )? {
            return Ok(false);
        }
        let rows =
            std::iter::once(&mut self.session.items[index]).chain(self.session.bases.get_mut(&id));
        for item in rows {
            if !item.reflection.commands_seen.contains(&comment) {
                item.reflection.commands_seen.push(comment);
            }
        }
        Ok(true)
    }

    /// Posts the pending command replies of `todo` on `issue`; the
    /// reflection lock is the proof that no other holder posts them.
    fn post_replies(
        &mut self,
        issue: IssueRef,
        todo: &str,
        _proof: &ReflectionLock,
    ) -> Result<(), WorkError> {
        for (source, commanded_at, text) in self.session.store.pending_replies(todo)? {
            let result = match self.remote.as_ref() {
                Some(Remote::Client { gh, down: None }) => {
                    _reflect::reply(gh, issue, source, commanded_at.as_deref(), &text)
                }
                _ => break,
            };
            match result {
                Ok(()) => self.session.store.reply_sent(todo, source)?,
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
        let expected = super::_envelope::issue(&self.session.items[index].record).labels;
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
            self.session
                .amend(index, |reflection| reflection.pending = true)?;
        }
        Ok(())
    }

    /// Applies the attributable `hold` label actions read from item
    /// `index`'s issue once each, in their source order.
    fn answer_label_actions(
        &mut self,
        index: usize,
    ) -> Result<Option<Vec<super::_positions::EventRead>>, WorkError> {
        let Some(issue) = self.session.items[index].reflection.issue else {
            return Ok(Some(Vec::new()));
        };
        let result = match self.remote.as_ref() {
            Some(Remote::Client { gh, down: None }) => {
                _reflect::read_events(gh, issue, &self.session.items[index].reflection)
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
    /// `issue`; `false` while it stays outstanding.
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
            || self.session.items[index]
                .record
                .label_actions
                .iter()
                .any(|a| a.issue == issue.number && a.id == id)
            || self.session.store.label_seen(issue.number, id)?
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
        let permission = match self.remote.as_ref() {
            Some(Remote::Client { gh, down: None }) => gh.can_write(login),
            _ => return Ok(false),
        };
        let allowed = match permission {
            Ok(allowed) => allowed,
            Err(error) => {
                tracing::warn!(event="github_hold_actor_unverified",login,error=%error);
                self.note(error);
                return Ok(false);
            }
        };
        let admitted =
            allowed && self.session.items[index].record.status != TodoStatus::StructuralHolder;
        if !admitted {
            return self.session.store.ignore_label(
                &self.session.items[index].record,
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
        self.session.commit("github hold label", |s| {
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
        let record = &self.session.items[index].record;
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
        let now = self.session.now;
        let actor = Actor::GitHub {
            login: login.to_owned(),
        };
        let fingerprint =
            EvalFingerprint::current(self.session.work.home())?.map(|f| f.fingerprint);
        let todo_id = record.id.to_string();
        let sequence = record.sequence + 1;
        let notice = serde_json::json!({"decision":fact,"gate":gate_name,"actor":actor,"evidence":fact,"at":now.to_string(),"source_at":source_at,"arrived_at":now.to_string(),"source_comment":comment_id}).to_string();
        self.session.commit_with(
            "inbound command",
            |s| {
                s.items[index].reflection.commands_seen.push(comment_id);
                let record = &mut s.items[index].record;
                let sequence = record.next_sequence();
                record.stamps.push(StampRecord {
                    source_comment: Some(comment_id),
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
            },
            |tx| {
                tx.execute(
                    "INSERT INTO approval_attention(todo_id,decision_sequence,content,state) VALUES (?1,?2,?3,'pending') ON CONFLICT(todo_id,decision_sequence) DO UPDATE SET content=excluded.content WHERE approval_attention.state='pending' AND approval_attention.input_id IS NULL",
                    instrukt_core::state_db::rusqlite::params![todo_id, sequence, notice],
                )
                .map_err(instrukt_core::state_db::StateDbError::sqlite(
                    std::path::Path::new(tx.path().unwrap_or("")),
                ))?;
                _store::retain_reply(
                    tx,
                    std::path::Path::new(tx.path().unwrap_or("")),
                    &todo_id,
                    comment_id,
                    source_at,
                    &answer,
                )?;
                Ok(())
            },
        )?;
        tracing::info!(
            event = "work_inbound_command",
            project = %self.session.project.slug,
            todo = %self.session.items[index].record.slug,
            login,
            "an approver's command was recorded"
        );
        Ok(answer)
    }

    /// Publishes work branches outside `reflection.lock`, then flushes
    /// pending reflection of every stored item of the project under
    /// `reflection.lock`. Persists command-wide reach/unavailable/rate-limit
    /// failure causes onto every pending/owed item in `reflections.failure`
    /// without issuing further GitHub requests once down.
    fn flush_pass(&mut self) -> Result<(), WorkError> {
        self.session.reload()?;
        // Publication runs outside the reflection lock so a git push never
        // holds it.
        self.publish()?;
        let lock = match ReflectionLock::acquire(self.session.home()) {
            Ok(lock) => lock,
            Err(WorkError::Fs(busy @ instrukt_core::fsx::FsError::LockTimeout { .. })) => {
                tracing::warn!(
                    event = "work_reflection_busy",
                    project = %self.session.project.slug,
                    error = %busy,
                    "another reflection holds the reflection lock; this flush reflects nothing and leaves what is unconfirmed to a later flush"
                );
                self.reflection_busy = Some(busy.to_string());
                for row in self.session.store.items()? {
                    self.session.learned(Some(row));
                }
                return Ok(());
            }
            Err(error) => return Err(error),
        };
        self.ensure_remote()?;
        let (board, board_error) = if self.down_reason().is_some() {
            (None, None)
        } else {
            let board_result = (|| {
                // A board the developer names in the project layer leads; otherwise
                // the one this computer recorded. Setup reconciles either with
                // GitHub, finding an existing board before creating one.
                let configured = self.session.config.board()?;
                let recorded = self.session.store.recorded_board()?;
                let known = configured
                    .clone()
                    .or_else(|| recorded.as_deref().and_then(BoardRef::parse));
                let Some(Remote::Client { gh, down: None }) = self.remote.as_ref() else {
                    return Ok(None);
                };
                let account = gh.for_role(None, github::Operation::Board);
                let board = super::_board::setup(&account, gh, known.as_ref())?;
                // The reference is the machine's record: reflection never writes
                // the committed project layer.
                if configured.as_ref() != Some(&board)
                    && recorded.as_deref() != Some(&board.to_string())
                {
                    self.session.store.record_board(&board.to_string())?;
                }
                super::_board::configure(&account, &board)?;
                Ok(Some(board))
            })();
            match board_result {
                Ok(board) => (board, None),
                Err(WorkError::GitHub(gh_error)) if gh_error.is_reach() => {
                    self.note(gh_error);
                    (None, None)
                }
                Err(error) => {
                    tracing::warn!(event="work_board_setup_pending", error=%error);
                    (None, Some(error.to_string()))
                }
            }
        };
        self.board_setup_failure = board_error.clone();
        for loaded in self.session.store.items()? {
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
                if let Some(reason) = self.down_reason() {
                    reflected.reflection.failure = Some(describe(&reason));
                } else {
                    // A todo shown as held already needs you: its record is not
                    // projected and no approver is resolved for it.
                    let needs_you = super::_ports::waits_on_person(
                        super::_envelope::record_status(&reflected.record),
                        || {
                            Ok::<_, instrukt_core::config::ConfigError>(
                                crate::workflow::waiting_for_human_approval(
                                    &super::_ports::to_evidence(
                                        reflected.record.clone(),
                                        Vec::new(),
                                    ),
                                    self.session
                                        .config
                                        .approver(instrukt_core::config::ApprovalGate::Prd)?,
                                    self.session
                                        .config
                                        .approver(instrukt_core::config::ApprovalGate::Task)?,
                                ),
                            )
                        },
                    )?;
                    let result = match self.remote.as_ref() {
                        Some(Remote::Client { gh, down: None }) => Some(_reflect::flush(
                            gh,
                            &mut reflected,
                            &self.session.project.root,
                            &self.session.base,
                            board.as_ref(),
                            needs_you,
                        )),
                        _ => None,
                    };
                    if let Some(Err(err)) = result {
                        if err.is_reach() {
                            let reason = reason_of(&err);
                            self.note(err);
                            reflected.reflection.failure = Some(describe(&reason));
                        } else {
                            tracing::warn!(
                                event = "work_reflection_pending",
                                project = %self.session.project.slug,
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
            }
            // A row this flush left as it was may still be news to the
            // session: another process can have reflected it already.
            let row = if _store::stored(&reflected) != _store::stored(&loaded) {
                self.session.store.commit_reflection(&loaded, &reflected)?
            } else {
                Some(loaded)
            };
            self.session.learned(row);
        }
        drop(lock);
        Ok(())
    }

    /// Publishes the work branch of each repository todo still owed it
    /// (`github-record`, Issues and visible work), outside `reflection.lock`.
    fn publish(&mut self) -> Result<(), WorkError> {
        let remote = github::tracked_remote(&self.session.project.root, &self.session.base)?;
        for index in 0..self.session.items.len() {
            let item = &self.session.items[index];
            let id = item.record.id.clone();
            let owed = super::_envelope::is_repository_work(item.record.kind)
                && item.record.delivery.is_none()
                && item.record.status != TodoStatus::Removed
                && self.session.store.withdrawal_intent(&id)?.is_none();
            let Some(branch) = item.record.branch.clone().filter(|_| owed) else {
                if item.reflection.publication_outstanding.is_some() {
                    self.session
                        .amend(index, |r| r.publication_outstanding = None)?;
                }
                continue;
            };
            let head = match git::run(
                &self.session.project.root,
                [
                    "rev-parse",
                    "--verify",
                    "--quiet",
                    &format!("refs/heads/{branch}^{{commit}}"),
                ],
            ) {
                Ok(head) => instrukt_core::ids::CommitSha::parse(head.trim())
                    .map_err(|e| WorkError::refused(e.to_string()))?,
                Err(git::GitError::Failed { .. }) => continue,
                Err(error) => return Err(error.into()),
            };
            if item.reflection.published_head.as_ref() == Some(&head)
                && item.reflection.publication_outstanding.is_none()
            {
                continue;
            }
            let Some(remote) = remote.as_deref() else {
                let why = format!(
                    "the default branch `{}` tracks no remote, so branch `{branch}` is not published",
                    self.session.base
                );
                if item.reflection.publication_outstanding.as_deref() != Some(why.as_str()) {
                    self.session
                        .amend(index, |r| r.publication_outstanding = Some(why.clone()))?;
                }
                continue;
            };
            match super::_publication::push(&self.session.project.root, remote, &branch) {
                super::_publication::Pushed::Published => {
                    tracing::info!(
                        event = "work_branch_published",
                        project = %self.session.project.slug,
                        todo = %self.session.items[index].record.slug,
                        branch,
                        head = %head,
                        "the work branch is on the remote"
                    );
                    self.session.amend(index, |r| {
                        r.published_head = Some(head.clone());
                        r.publication_outstanding = None;
                    })?;
                }
                super::_publication::Pushed::Diverged => {
                    let why = format!(
                        "branch `{branch}` on `{remote}` moved where this computer's branch cannot follow it, so its commits are not published; the branch on `{remote}` is left as it is"
                    );
                    if self.session.items[index]
                        .reflection
                        .publication_outstanding
                        .as_deref()
                        == Some(why.as_str())
                    {
                        tracing::debug!(event = "work_branch_diverged", project = %self.session.project.slug, todo = %self.session.items[index].record.slug, branch, "the remote work branch is still moved");
                        continue;
                    }
                    tracing::warn!(
                        event = "work_branch_diverged",
                        project = %self.session.project.slug,
                        todo = %self.session.items[index].record.slug,
                        branch,
                        "the remote work branch moved; publication is left to the developer"
                    );
                    self.session
                        .amend(index, |r| r.publication_outstanding = Some(why.clone()))?;
                }
                super::_publication::Pushed::Unreached(reason) => {
                    let why = format!(
                        "branch `{branch}` could not reach `{remote}` ({reason}); the next tick publishes it"
                    );
                    if self.session.items[index]
                        .reflection
                        .publication_outstanding
                        .as_deref()
                        == Some(why.as_str())
                    {
                        tracing::debug!(event = "work_branch_unpublished", project = %self.session.project.slug, todo = %self.session.items[index].record.slug, branch, reason, "the work branch still cannot reach its remote");
                        continue;
                    }
                    tracing::warn!(
                        event = "work_branch_unpublished",
                        project = %self.session.project.slug,
                        todo = %self.session.items[index].record.slug,
                        branch,
                        reason,
                        "the work branch could not reach its remote; the next tick tries again"
                    );
                    self.session
                        .amend(index, |r| r.publication_outstanding = Some(why.clone()))?;
                }
            }
        }
        Ok(())
    }

    /// Records that GitHub stopped answering; a rate limit is remembered
    /// across ticks until the provider permits requests again.
    fn note(&mut self, err: GitHubError) {
        if let GitHubError::RateLimited { until: Some(until) }
        | GitHubError::App(github::AppError::RateLimited { until: Some(until) }) = &err
        {
            let saved = self.session.store.provider().and_then(|mut state| {
                state.deferred_until = Some(*until);
                self.session.store.save_provider(&state)
            });
            if let Err(save) = saved {
                tracing::warn!(event = "work_rate_limit_unsaved", error = %save, "cannot remember GitHub's rate limit");
            }
        }
        if err.is_reach() {
            tracing::warn!(
                event = "work_github_unavailable",
                project = %self.session.project.slug,
                error = %err,
                "GitHub is not answering; the local record continues"
            );
            if let Some(Remote::Client { down, .. }) = self.remote.as_mut() {
                *down = Some(err);
            }
        } else {
            tracing::warn!(
                event = "work_github_read_failed",
                project = %self.session.project.slug,
                error = %err,
                "a GitHub read failed"
            );
        }
    }
}

/// Why GitHub cannot be reached for `work` at `now`, read from this
/// computer alone: a rate limit GitHub set, or App credentials that cannot
/// be used. `None` when a client can be built.
fn unavailable(work: &Work, provider: &ProviderState, now: Timestamp) -> Option<PendingReason> {
    match provider.deferred_until {
        Some(until) if until > now => Some(PendingReason::RateLimited { until: Some(until) }),
        _ => work
            .github
            .validate()
            .err()
            .map(|err| reason_of(&GitHubError::from(err))),
    }
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
