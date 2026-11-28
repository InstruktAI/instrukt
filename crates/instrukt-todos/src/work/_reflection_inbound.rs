//! Inbound GitHub control ingestion for [`super::_reflection_session::ReflectionSession`]:
//! reading issue comments and events under `reflection.lock`, applying
//! attributable `hold` label actions and `/instrukt` approver commands once
//! each, flagging drifted output labels, and posting queued command replies.

use instrukt_core::publication::EvalFingerprint;
use jiff::Timestamp;

use super::_error::WorkError;
use super::_inbound;
use super::_reflect;
use super::_reflection_session::{ReflectionSession, Remote};
use super::_store::{self, IssueRef, ReflectionLock};
use crate::evidence::{
    Actor, ApprovalGate, ApprovedBy, ModelProvenance, StampFact, StampRecord, TodoStatus,
    artifact_names,
};

impl ReflectionSession<'_> {
    /// Reads item `index`'s issue for people's controls and applies them
    /// (`github-record`, Reading GitHub): `hold` label actions and
    /// `/instrukt` commands, each once, and output labels someone changed
    /// are reflected again. GET-only on GitHub; queued command replies are
    /// posted during [`Self::flush_pass`].
    pub(super) fn pull_through(
        &mut self,
        _lock: &ReflectionLock,
        index: usize,
    ) -> Result<(), WorkError> {
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

    /// Posts the pending command replies of `todo` on `issue`; the
    /// reflection lock is the proof that no other holder posts them.
    pub(super) fn post_replies(
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
}
