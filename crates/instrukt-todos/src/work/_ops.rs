//! The `instrukt todo` record operations (`todos`, `dependencies-and-holds`,
//! `stamps`, `approvals`, `github-record` "Artifacts and stamps"). Each
//! opens a local session, validates the whole resulting record before it
//! commits, and leaves reflection pending for the runner.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use instrukt_core::config::Configuration;
use instrukt_core::ids::{CommitSha, SessionRef, TodoSlug};
use instrukt_core::machine;
use instrukt_core::publication::EvalFingerprint;
use instrukt_core::{git, github};
use instrukt_runner::events::{Event, EventLog};
use instrukt_runner::interfaces::{AgentPreference, CatchUp, ProjectRef, TaskSchedule, TodoKind};

use super::_diff::{DiffLine, diff_lines};
use super::_error::WorkError;
use super::_model::{
    Artifact, Delivery, ReleasedHold, SCHEMA, TodoAddress, TodoId, TodoRecord, TodoRef, Version,
    digest_of, valid_artifact_name,
};
use super::_session::{ReflectionStatus, Session, exact_local, find_in, past_recovery, status_of};
use super::_stamps::{self, StampInputs, StampRequest};
use super::_store::{BranchRelease, Completed, LocalItem, Store, WithdrawalIntent};
use super::_withdrawal::{WithdrawalLock, plan_release, release};
use super::Work;
use crate::evidence::{
    Actor, ArtifactName, DependencyDigest, HoldRecord, HoldSource, StampFact, StampRecord,
    TodoStatus, Upstream, VerificationDisposition, artifact_names, upstream_of,
};

/// `instrukt todo create`.
#[derive(Debug, Clone)]
pub struct CreateRequest {
    pub slug: TodoSlug,
    pub content: String,
    pub kind: TodoKind,
    pub icebox: bool,
    pub after: Vec<TodoSlug>,
    pub group: Option<TodoSlug>,
    pub schedule: Option<TaskSchedule>,
    pub catch_up: Option<CatchUp>,
    pub agent: Option<AgentPreference>,
    pub human_involved: Option<bool>,
    /// A manual dispatch hold committed with the todo, for this reason.
    pub hold: Option<String>,
}

/// `instrukt todo update`: every given change, applied together.
#[derive(Debug, Clone, Default)]
pub struct UpdateRequest {
    pub slug: Option<TodoSlug>,
    pub kind: Option<TodoKind>,
    /// `Some(None)` clears the schedule.
    pub schedule: Option<Option<TaskSchedule>>,
    /// `Some(None)` leaves the group.
    pub group: Option<Option<TodoSlug>>,
    /// Replaces the whole prerequisite set; empty clears it.
    pub after: Option<Vec<TodoSlug>>,
    pub hold: Option<String>,
    pub release: bool,
    pub unclaim: bool,
    pub agent: Option<AgentPreference>,
    /// Permits permanent orphan-artifact handling during a kind migration.
    pub force: bool,
}

/// A listing selection; the active set is the default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Selection {
    #[default]
    Active,
    Iceboxed,
    Removed,
    Delivered,
}

/// A change's result and where its reflection stands.
#[derive(Debug, Clone)]
pub struct Outcome<T> {
    pub value: T,
    pub reflection: ReflectionStatus,
    /// Documentation files the change rewrote in the working tree (a
    /// group parent's pending documentation moving to its first child),
    /// repository-relative, for the caller to commit.
    pub docs_changed: Vec<std::path::PathBuf>,
}

/// What `instrukt todo update` changed, with the warnings it raised.
#[derive(Debug, Clone)]
pub struct UpdateReport {
    pub record: TodoRecord,
    pub warnings: Vec<String>,
}

/// A todo as `instrukt todo show` presents it.
#[derive(Debug, Clone)]
pub struct TodoView {
    pub record: TodoRecord,
    /// The record status — active, iceboxed, held, delivered or withdrawn —
    /// exactly as reflection writes it to GitHub.
    pub record_status: &'static str,
    /// Where the todo's reflection stands.
    pub reflection: ReflectionStatus,
    pub issue: Option<u64>,
    pub pull: Option<u64>,
    pub prerequisites: Vec<PrerequisiteView>,
    /// Prerequisites expanded through structural holders, matching
    /// `WorkEvidence::evidence` so `todo show` derives the same ledger as
    /// `todo work`.
    pub expanded_prerequisites: Vec<crate::evidence::Prerequisite>,
    /// A structural holder's children.
    pub children: Vec<TodoRef>,
}

/// One prerequisite with the status this computer knows of it.
#[derive(Debug, Clone)]
pub struct PrerequisiteView {
    pub slug: TodoSlug,
    pub status: Option<TodoStatus>,
}

/// The project's work records available to evaluation.
#[derive(Debug, Clone)]
pub struct RecordedWork {
    pub records: Vec<TodoRecord>,
}

/// `instrukt todo list`: read from the state database alone.
#[derive(Debug, Clone)]
pub struct Listing {
    pub entries: Vec<ListEntry>,
}

#[derive(Debug, Clone)]
pub struct ListEntry {
    pub record: TodoRecord,
    pub reflection: ReflectionStatus,
    /// Every prerequisite delivered.
    pub ready: bool,
    /// The computer the todo's claim names when that is not this one.
    pub claimed_on: Option<instrukt_core::machine::MachineId>,
}

impl Work {
    pub fn create(
        &self,
        project: &ProjectRef,
        request: CreateRequest,
    ) -> Result<Outcome<TodoRecord>, WorkError> {
        let mut s = Session::open(self, project)?;
        if request.content.trim().is_empty() {
            return Err(WorkError::refused(
                "a todo records substantive intent; the content is empty",
            ));
        }
        if request.kind != TodoKind::Task
            && (request.schedule.is_some() || request.agent.is_some() || request.catch_up.is_some())
        {
            return Err(WorkError::refused(
                "a schedule, catch-up policy and agent preference belong to a task; this todo is not one",
            ));
        }
        if request.icebox && request.hold.is_some() {
            return Err(WorkError::refused(
                "a held todo is converged work waiting in the lifecycle, and the icebox parks work that has not converged; file it with one of them",
            ));
        }
        ensure_slug_free(&mut s, &request.slug, None)?;
        let actor = s.actor()?;
        let hold = request
            .hold
            .as_deref()
            .map(|reason| manual_hold(reason, &actor, s.now))
            .transpose()?;
        let after = resolve_refs(&mut s, &request.after)?;
        if request.after.contains(&request.slug) {
            return Err(WorkError::refused("a todo cannot depend on itself"));
        }
        let group = match &request.group {
            Some(parent) => Some(
                s.live_ref(parent)
                    .map(|r| r.reference())
                    .ok_or_else(|| unknown(&s, parent))?,
            ),
            None => None,
        };
        let hex = machine::random_hex(16).map_err(|source| WorkError::Io {
            path: "/dev/urandom".into(),
            source,
        })?;
        let id = TodoId::parse(&hex)
            .ok_or_else(|| WorkError::refused("a fresh todo identity was not 32 hex digits"))?;
        let now = s.now;
        let content = request.content.clone();
        let record = TodoRecord {
            schema: SCHEMA,
            id,
            project: project.slug.clone(),
            slug: request.slug.clone(),
            prior_slugs: Vec::new(),
            kind: request.kind,
            status: if request.icebox {
                TodoStatus::Iceboxed
            } else {
                TodoStatus::Active
            },
            created: now,
            created_by: actor.clone(),
            human_involved: request.human_involved,
            revision: 0,
            sequence: 1,
            artifacts: vec![Artifact {
                name: ArtifactName::new(artifact_names::INPUT),
                address: None,
                versions: vec![Version {
                    digest: digest_of(&content),
                    writer: actor.clone(),
                    created: now,
                    derived_from: Vec::new(),
                    sequence: 1,
                    grounding: Default::default(),
                    remote: None,
                    content: Some(content),
                }],
            }],
            stamps: Vec::new(),
            claim: None,
            bindings: Vec::new(),
            hold,
            hold_history: Vec::new(),
            label_actions: Vec::new(),
            after,
            group: None,
            schedule: request.schedule.clone(),
            catch_up: request.catch_up.unwrap_or_default(),
            agent: request.agent.clone(),
            branch: None,
            delivery: None,
            removal: None,
        };
        // The todo created is the command's target, whatever todo its
        // checks entered first: a refusal of a change to its group or a
        // prerequisite sends the caller back to creating it.
        s.set_target(&record.id);
        let index = s.commit("created", |s| {
            s.items.push(super::_store::LocalItem {
                record,
                reflection: Default::default(),
            });
            let index = s.items.len() - 1;
            if let Some(parent) = &group {
                join_group(s, index, parent, &actor)?;
            }
            check_prerequisites(s, index)?;
            Ok(index)
        })?;
        Ok(Outcome {
            value: s.items[index].record.clone(),
            reflection: s.status(index),
            docs_changed: std::mem::take(&mut s.docs_changed),
        })
    }

    /// The todo `address` names: a plain slug is the live item;
    /// `<slug>@<identity-prefix>` an exact entry, retained ones included.
    /// Reads only this computer's local record without contacting GitHub
    /// or acquiring locks on a current-schema database.
    pub fn show(&self, project: &ProjectRef, address: &TodoAddress) -> Result<TodoView, WorkError> {
        let s = Session::open_read(self, project)?;
        let index = s.locate(address)?;
        Ok(view(&s, index))
    }

    /// Every record of the project's work this computer holds, retained
    /// ones included, for evaluation (`eval-report`, "Evidence source"):
    /// read from the local record alone, with no GitHub request.
    pub fn recorded_work(&self, project: &ProjectRef) -> Result<RecordedWork, WorkError> {
        let store = self.local_store_checked(project)?;
        Ok(RecordedWork {
            records: store.items()?.into_iter().map(|it| it.record).collect(),
        })
    }

    /// The records this computer holds for `project`, read from its local
    /// record alone: for a project whose checkout or configuration is not
    /// available, or that no longer tracks work, its recorded history.
    pub fn stored_work(
        &self,
        project: &instrukt_core::ids::ProjectSlug,
    ) -> Result<Vec<TodoRecord>, WorkError> {
        Ok(super::_store::read_items(&self.home, project)?
            .into_iter()
            .map(|item| item.record)
            .collect())
    }

    /// The project's todos as this computer's state database holds them:
    /// no listing ever reads GitHub.
    pub fn list(&self, project: &ProjectRef, selection: Selection) -> Result<Listing, WorkError> {
        // A local read: no GitHub request, and no lock on a current schema;
        // a database behind this build's schema migrates once under the
        // writer lock (`state-database`, "Writing"). Withdrawn work past its
        // recovery window is left out; a writing command purges it.
        let store = self.local_store_checked(project)?;
        let items = store.items()?;
        let now = self.now();
        let items: Vec<LocalItem> = items
            .into_iter()
            .filter(|it| !past_recovery(&it.record, now))
            .collect();
        let machine = machine::machine_id(&self.home)?;
        let wanted = match selection {
            Selection::Active => vec![TodoStatus::Active, TodoStatus::StructuralHolder],
            Selection::Iceboxed => vec![TodoStatus::Iceboxed],
            Selection::Removed => vec![TodoStatus::Removed],
            Selection::Delivered => vec![TodoStatus::Delivered],
        };
        let mut entries: Vec<ListEntry> = items
            .iter()
            .filter(|it| wanted.contains(&it.record.status))
            .map(|it| ListEntry {
                record: it.record.clone(),
                reflection: status_of(it),
                ready: false,
                claimed_on: it
                    .record
                    .claim
                    .as_ref()
                    .map(|c| c.machine.clone())
                    .filter(|on| on != &machine),
            })
            .collect();
        let delivered: BTreeSet<TodoId> = items
            .iter()
            .map(|it| &it.record)
            .filter(|r| r.status == TodoStatus::Delivered)
            .map(|r| r.id.clone())
            .collect();
        for entry in &mut entries {
            entry.ready = entry.record.after.iter().all(|r| delivered.contains(&r.id));
        }
        Ok(Listing {
            entries: order(entries),
        })
    }

    pub fn update(
        &self,
        project: &ProjectRef,
        slug: &TodoSlug,
        request: UpdateRequest,
        events: Option<&dyn EventLog>,
    ) -> Result<Outcome<UpdateReport>, WorkError> {
        let mut s = Session::open(self, project)?;
        let index = s.enter(slug)?;
        let actor = s.actor()?;
        let warnings =
            s.commit_with_events("updated", events.map(|log| (log, [].as_slice())), |s| {
                apply_update(s, index, &request, &actor)
            })?;
        Ok(Outcome {
            value: UpdateReport {
                record: s.items[index].record.clone(),
                warnings,
            },
            reflection: s.status(index),
            docs_changed: std::mem::take(&mut s.docs_changed),
        })
    }

    pub fn freeze(
        &self,
        project: &ProjectRef,
        slug: &TodoSlug,
    ) -> Result<Outcome<TodoRecord>, WorkError> {
        self.transition(project, slug, "frozen", |record, _| {
            if record.status != TodoStatus::Active {
                return Err(WorkError::refused(format!(
                    "only active work is parked; `{}` is {}",
                    record.slug,
                    status_word(record.status)
                )));
            }
            record.status = TodoStatus::Iceboxed;
            Ok(())
        })
    }

    pub fn unfreeze(
        &self,
        project: &ProjectRef,
        slug: &TodoSlug,
    ) -> Result<Outcome<TodoRecord>, WorkError> {
        self.transition(project, slug, "unfrozen", |record, _| {
            if record.status != TodoStatus::Iceboxed {
                return Err(WorkError::refused(format!(
                    "`{}` is not in the icebox",
                    record.slug
                )));
            }
            record.status = TodoStatus::Active;
            Ok(())
        })
    }

    /// `instrukt todo remove`: withdraws the live todo `slug` and releases
    /// its working resources (`todos`, "Parking, withdrawal and delivery").
    ///
    /// 1. The entry reads only the local record without contacting GitHub.
    /// 2. Under the withdrawal lock, the todo's worktrees, workspace and
    ///    work branch are checked; work that would be lost refuses with
    ///    nothing done. Then one writer transaction records the intent
    ///    (`withdrawal_intents`) while the live todo under `slug` is still
    ///    the one entered, unchanged. The todo's own row is not written: it
    ///    stays live with its claim as it was, or unclaimed when it had
    ///    none, and nothing withdrawn reaches GitHub before its resources
    ///    are released. While the intent is open no other command changes
    ///    the record and no runner reflection read replaces it.
    /// 3. The resources are released outside the writer lock, the intent
    ///    recording each outcome that must be recognized afterwards
    ///    ([`release`]). A refusal git or the filesystem positively shows —
    ///    a resource that must be kept, or an attempted release proven to
    ///    have changed nothing or taken back — closes the intent and leaves
    ///    the todo as it was while the intent records nothing released and
    ///    nothing under way (a worktree the withdrawal moved or is moving, a partial
    ///    marker) and the command attempted no other release. An
    ///    observation or effect whose outcome was not established keeps the
    ///    intent even when it records nothing released, and so does a branch
    ///    update that left only its marker, whether or not the marker was
    ///    taken back. A kept intent names what is recorded released and
    ///    under way, for the withdrawal run again to complete.
    /// 4. One writer transaction commits the withdrawn record, with
    ///    `removal` naming exactly what was released, and closes the intent.
    /// 5. The withdrawal lock is released; the runner's next reflection pass
    ///    reflects the withdrawal to GitHub.
    ///
    /// A withdrawal interrupted with its intent recorded is completed by
    /// running it again, which continues from the intent after recognizing
    /// what the interrupted run did.
    pub fn remove(
        &self,
        project: &ProjectRef,
        slug: &TodoSlug,
    ) -> Result<Outcome<TodoRecord>, WorkError> {
        let mut s = Session::open(self, project)?;
        let store = Store::new(&self.home, &project.slug);
        // A withdrawal already admitted resumes on the record it was
        // admitted on.
        let resuming = match s.find(slug, false) {
            Some(index)
                if store
                    .withdrawal_intent(&s.items[index].record.id)?
                    .is_some() =>
            {
                Some(index)
            }
            _ => None,
        };
        let index = match resuming {
            Some(index) => index,
            None => {
                let index = s.enter(slug)?;
                writable(&s.items[index].record)?;
                index
            }
        };
        let base = s.base(index).ok_or_else(|| {
            WorkError::refused(format!("`{slug}` has no stored record to withdraw"))
        })?;
        let id = base.record.id.clone();
        // The withdrawal acts from the project's primary checkout: it holds
        // the project's workspace and HEAD, and is never one of the linked
        // worktrees the release moves away, which the command may run in.
        let checkout = instrukt_core::repository::primary_worktree(&project.root)
            .map_err(|err| WorkError::refused(err.to_string()))?;
        let withdrawing = WithdrawalLock::acquire(&self.home)?;
        let (intent, fresh) = match store.withdrawal_intent(&id)? {
            None if resuming.is_some() => {
                return Err(WorkError::refused(format!(
                    "`{slug}`'s withdrawal ended while this command waited for it; nothing was done. See where it stands with `instrukt todo show {slug}`"
                )));
            }
            Some(intent) => {
                tracing::info!(
                    event = "work_withdrawal_resumed",
                    project = %project.slug,
                    todo = %slug,
                    started = %intent.started_at,
                    "continuing an interrupted withdrawal from its recorded intent"
                );
                (intent, false)
            }
            None => {
                let checked = plan_release(&checkout, &base.record)?;
                let intent = WithdrawalIntent {
                    todo: id.clone(),
                    branch: checked.branch,
                    branch_head: checked.head,
                    worktrees: checked.worktrees,
                    workspace_path: checked.workspace_path,
                    workspace: checked.workspace,
                    branch_state: BranchRelease::Pending,
                    started_at: s.now,
                };
                if let Err(refusal) = store.open_withdrawal(&base, slug, &intent)? {
                    return Err(s.refusal(index, refusal));
                }
                (intent, true)
            }
        };
        let released = release(&checkout, slug, &store, intent, fresh)?;
        match store.complete_withdrawal(&id, s.now, released.branch, released.worktrees)? {
            Completed::Withdrawn(row) => s.take_written(index, *row, "removed"),
            Completed::NotLive(row) => {
                let status = status_word(row.record.status);
                return Err(WorkError::refused(format!(
                    "`{slug}` became {status} while its working resources were released; its record is left as it is"
                )));
            }
            Completed::Gone => {
                return Err(WorkError::refused(format!(
                    "`{slug}` left this computer's record while its working resources were released"
                )));
            }
        }
        drop(withdrawing);
        Ok(Outcome {
            value: s.items[index].record.clone(),
            reflection: s.status(index),
            docs_changed: std::mem::take(&mut s.docs_changed),
        })
    }

    /// Restores withdrawn work by identity or an unambiguous slug.
    pub fn reactivate(
        &self,
        project: &ProjectRef,
        key: &str,
    ) -> Result<Outcome<TodoRecord>, WorkError> {
        let mut s = Session::open(self, project)?;
        // An identity, a slug, or `<slug>@<identity-prefix>`.
        let address: Option<TodoAddress> = key.parse().ok();
        let wanted = |record: &TodoRecord| {
            record.status == TodoStatus::Removed
                && (record.id.as_str() == key || address.as_ref().is_some_and(|a| a.names(record)))
        };
        let removed: Vec<usize> = s
            .items
            .iter()
            .enumerate()
            .filter(|(_, it)| wanted(&it.record))
            .map(|(i, _)| i)
            .collect();
        // One withdrawn todo is restored; several are refused naming each.
        let index = match removed.as_slice() {
            [index] => *index,
            [] => return Err(no_withdrawn(key)),
            several => {
                let records: Vec<TodoRecord> =
                    several.iter().map(|i| s.items[*i].record.clone()).collect();
                return Err(super::_session::several(
                    &format!("`{key}`"),
                    records.iter(),
                ));
            }
        };
        let slug = s.items[index].record.slug.clone();
        if s.live_ref(&slug).is_some() {
            return Err(WorkError::refused(format!(
                "a live todo is named `{slug}`; restoring would make the name ambiguous — rename one first"
            )));
        }
        s.commit("reactivated", |s| {
            let record = &mut s.items[index].record;
            let from = record
                .removal
                .take()
                .map_or(TodoStatus::Active, |removal| removal.from);
            record.status = from;
            Ok(())
        })?;
        Ok(Outcome {
            value: s.items[index].record.clone(),
            reflection: s.status(index),
            docs_changed: std::mem::take(&mut s.docs_changed),
        })
    }

    pub fn deliver(
        &self,
        project: &ProjectRef,
        slug: &TodoSlug,
        commit: &CommitSha,
    ) -> Result<Outcome<TodoRecord>, WorkError> {
        let mut s = Session::open(self, project)?;
        let index = s.enter(slug)?;
        if s.items[index].record.status != TodoStatus::Active {
            return Err(WorkError::refused(format!(
                "only active work is delivered; `{slug}` is {}",
                status_word(s.items[index].record.status)
            )));
        }
        let object = format!("{commit}^{{commit}}");
        match git::run(&project.root, ["cat-file", "-e", object.as_str()]) {
            Ok(_) => {}
            Err(git::GitError::Failed { .. }) => {
                return Err(WorkError::refused(format!(
                    "{commit} is not a commit of this repository; delivery names the actual delivered commit"
                )));
            }
            Err(err) => return Err(err.into()),
        }
        let actor = s.actor()?;
        let now = s.now;
        s.commit("delivered", |s| {
            let record = &mut s.items[index].record;
            record.delivery = Some(Delivery {
                commit: commit.clone(),
                at: now,
                by: actor,
            });
            record.status = TodoStatus::Delivered;
            Ok(())
        })?;
        Ok(Outcome {
            value: s.items[index].record.clone(),
            reflection: s.status(index),
            docs_changed: std::mem::take(&mut s.docs_changed),
        })
    }

    /// Record a stamp and any integration event in one transaction; replay retains the original fact.
    pub fn stamp(
        &self,
        project: &ProjectRef,
        slug: &TodoSlug,
        request: &StampRequest,
        cwd: &Path,
        events: Option<&dyn EventLog>,
    ) -> Result<Outcome<StampRecord>, WorkError> {
        let mut s = Session::open(self, project)?;
        let index = s.enter(slug)?;
        let record = s.items[index].record.clone();
        if record.status != TodoStatus::Active {
            return Err(WorkError::refused(format!(
                "stamps record facts of active work; `{slug}` is {}",
                status_word(record.status)
            )));
        }
        let session = s.caller_session()?;
        let completion =
            super::_attempts::completion_binding(&record, session.as_ref(), cwd, request)?;
        if let Some(binding) = completion
            && super::_attempts::completion(request)
                .is_some_and(|c| c.repeat == super::_attempts::Repeat::Retained)
            && record.bindings[binding].attempt.released_at.is_some()
            && let Some(sequence) = record.bindings[binding].attempt.completion_sequence
            && let Some(stamp) = record
                .stamps
                .iter()
                .find(|stamp| stamp.sequence == sequence)
        {
            return Ok(Outcome {
                value: stamp.clone(),
                reflection: s.status(index),
                docs_changed: Vec::new(),
            });
        }
        // A verification disposition: workflow admits the caller under the
        // binding it acts as; a released verifier attempt replays exactly;
        // the orchestrator's record-only write needs a free, clean workspace.
        let mut disposition_role = None;
        if let StampRequest::VerificationDisposition {
            disposition,
            follow_up,
        } = request
        {
            let (role, released) =
                admit_disposition(&record, session.as_ref(), cwd, completion, *disposition)?;
            if let Some(binding) = released {
                let value =
                    released_disposition(&record, binding, *disposition, follow_up.as_ref())?;
                return Ok(Outcome {
                    value,
                    reflection: s.status(index),
                    docs_changed: Vec::new(),
                });
            }
            disposition_role = Some(role);
        }
        if matches!(request, StampRequest::FinalizeReady) {
            let prd_approver = s
                .config
                .approver(instrukt_core::config::ApprovalGate::Prd)?;
            super::_attempts::readiness(
                &record,
                session.as_ref(),
                cwd,
                &project.root,
                prd_approver,
            )?;
        }
        if matches!(request, StampRequest::TaskFiringStarted { .. }) {
            let task_approver = s
                .config
                .approver(instrukt_core::config::ApprovalGate::Task)?;
            super::_attempts::require_task_approval(&record, task_approver)?;
        }
        let base = s.base.clone();
        // A hotfix deferral's follow-up resolves among the project's open
        // todos, which only the work session sees; resolution judges it.
        let follow_up = match request {
            StampRequest::VerificationDisposition {
                follow_up: Some(slug),
                ..
            } => s.live_ref(slug).map(|todo| todo.reference()),
            _ => None,
        };
        let resolved = _stamps::resolve(
            request,
            &record,
            &StampInputs {
                cwd,
                base_branch: &base,
                session: session.as_ref(),
                now: s.now,
                follow_up: follow_up.as_ref(),
            },
        )?;
        let actor = match resolved.actor {
            Some(actor) => actor,
            None => s.actor()?,
        };
        let (model, model_provenance) = s.model_of(&actor);
        let mut fact = resolved.fact;
        if let StampFact::DriftReviewed {
            artifact,
            grounding,
            ..
        } = &mut fact
        {
            // The judgment holds only while what it assessed is unchanged:
            // record the grounding current now.
            let latest_version = record
                .artifact(artifact.as_str())
                .and_then(|a| a.versions.last());
            let content = latest_version
                .and_then(|v| v.content.clone())
                .unwrap_or_default();
            let prior_owned = prior_owned_for_version(&record, artifact);
            *grounding = grounding_now(&mut s, index, artifact.as_str(), &content, &prior_owned)?;
        }
        let stamp = StampRecord {
            source_comment: None,
            fact,
            at: s.now,
            // A disposition carries the role of the binding admission
            // matched, which stickiness reads (`delivery-cycle`).
            role: disposition_role.or_else(|| role_of(&record, &actor)),
            actor: actor.clone(),
            model,
            eval_fingerprint: EvalFingerprint::current(self.home())?.map(|f| f.fingerprint),
            human_involved: None,
            sequence: 0,
            model_provenance,
        };
        // The finalize-ready handoff commits with its `workflow.finalized`
        // event, in one transaction.
        let handoff = match &stamp.fact {
            StampFact::FinalizeReady { commit } => {
                let log = events.ok_or_else(|| {
                    WorkError::refused(
                        "the finalize-ready handoff appends `workflow.finalized` to the runner's event log, which this build does not provide",
                    )
                })?;
                tracing::info!(
                    event = "work_finalize_handoff",
                    project = %project.slug,
                    todo = %slug,
                    commit = %commit,
                    "workflow.finalized handed to the event log"
                );
                Some((
                    log,
                    vec![Event::WorkflowFinalized {
                        project: project.slug.clone(),
                        todo: slug.clone(),
                        revision: commit.clone(),
                    }],
                ))
            }
            _ => None,
        };
        if resolved.already_recorded {
            // The runner stamped this run's start; nothing new to record.
            if let Some((log, events)) = &handoff {
                for event in events {
                    s.log_event(*log, event)?;
                }
            }
            let retained = record
                .stamps
                .iter()
                .rev()
                .find(|prior| prior.fact == stamp.fact)
                .cloned()
                .ok_or_else(|| WorkError::refused("replayed fact has no retained stamp"))?;
            return Ok(Outcome {
                value: retained,
                reflection: s.status(index),
                docs_changed: std::mem::take(&mut s.docs_changed),
            });
        }
        let replay_prior = if matches!(stamp.fact, StampFact::DiagnosisSettled { .. }) {
            record
                .stamps
                .iter()
                .rev()
                .find(|prior| prior.fact == stamp.fact)
                .cloned()
        } else if matches!(stamp.fact, StampFact::VerificationDisposition { .. }) {
            // Only the orchestrator's record-only write replays here; a
            // verifier's released attempt replayed above.
            completion
                .is_none()
                .then(|| record_only_replay(&record, &stamp.fact, &stamp.actor))
                .flatten()
                .cloned()
        } else {
            current_drift_replay(&record, &stamp.fact, &stamp.actor).cloned()
        };
        if let Some(prior) = replay_prior {
            if let Some(binding) = completion
                && record.bindings[binding].attempt.released_at.is_none()
            {
                let now = s.now;
                let completed_head = super::_stamps::head(cwd)?;
                let sequence = prior.sequence;
                s.commit("reconcile completed read-only attempt", |s| {
                    let b = &mut s.items[index].record.bindings[binding];
                    b.attempt.completion_sequence = Some(sequence);
                    b.attempt.completed_head = Some(completed_head.clone());
                    b.attempt.released_at = Some(now);
                    Ok(())
                })?;
            }
            return Ok(Outcome {
                value: prior,
                reflection: s.status(index),
                docs_changed: std::mem::take(&mut s.docs_changed),
            });
        }
        let now = s.now;
        let events = handoff
            .as_ref()
            .map(|(log, events)| (*log, events.as_slice()));
        let decision = matches!(
            stamp.fact,
            StampFact::Approval { .. } | StampFact::ChangesRequested { .. }
        )
        .then(|| serde_json::to_string(&stamp))
        .transpose()
        .map_err(|e| WorkError::refused(e.to_string()))?;
        let todo_id = record.id.to_string();
        let decision_sequence = record.sequence + 1;
        s.commit_with("stamped", |s| {
            let record = &mut s.items[index].record;
            if let StampFact::DiagnosisUnresolved { reason, .. } = &stamp.fact {
                if let Some(previous) = record.hold.take() {
                    record.hold_history.push(ReleasedHold {
                        hold: previous,
                        released: now,
                        by: actor.clone(),
                    });
                }
                record.hold = Some(HoldRecord {
                    source: HoldSource::Automatic,
                    reason: format!("diagnosis unresolved: {reason}"),
                    by: actor.clone(),
                    at: now,
                });
            }
            if resolved.branch.is_some() {
                record.branch = resolved.branch;
            }
            // Ruling H: a claim covers one occurrence's run; that run's
            // outcome (success or error) releases it, so the task returns
            // to pickup. Only the execution holding the claim releases it.
            if resolved.releases_claim {
                record.claim = None;
            }
            let mut stamp = stamp;
            stamp.sequence = record.next_sequence();
            if let Some(binding) = completion {
                let b = &mut record.bindings[binding];
                if b.attempt.released_at.is_none() {
                    b.attempt.completion_sequence = Some(stamp.sequence);
                    b.attempt.completed_head = if b.worktree.is_some() {
                        Some(super::_stamps::head(cwd)?)
                    } else {
                        None
                    };
                    b.attempt.released_at = Some(now);
                }
            }
            record.stamps.push(stamp);
            Ok(())
        },|tx|{
            if let Some((log,events))=events {for event in events {log.append(tx,event)?;}}
            if let Some(content)=decision {
                tx.execute("INSERT INTO approval_attention(todo_id,decision_sequence,content,state) VALUES (?1,?2,?3,'pending') ON CONFLICT DO NOTHING",instrukt_core::state_db::rusqlite::params![todo_id,decision_sequence,content]).map_err(instrukt_core::state_db::StateDbError::sqlite(std::path::Path::new(tx.path().unwrap_or(""))))?;
            }
            Ok(())
        })?;
        Ok(Outcome {
            value: s.items[index]
                .record
                .stamps
                .last()
                .cloned()
                .ok_or_else(|| WorkError::refused("the stamp was not recorded"))?,
            reflection: s.status(index),
            docs_changed: std::mem::take(&mut s.docs_changed),
        })
    }

    /// `instrukt todo artifacts write`: appends a version.
    pub fn write_artifact(
        &self,
        project: &ProjectRef,
        slug: &TodoSlug,
        name: &str,
        content: &str,
    ) -> Result<Outcome<Version>, WorkError> {
        if !valid_artifact_name(name) {
            return Err(WorkError::refused(format!(
                "`{name}` is not an artifact name: lowercase letters, digits, `_` and `-`, starting with a letter"
            )));
        }
        if content.trim().is_empty() {
            return Err(WorkError::refused("an artifact version needs content"));
        }
        let mut s = Session::open(self, project)?;
        let index = s.enter(slug)?;
        let record = &s.items[index].record;
        writable(record)?;
        if record.status == TodoStatus::StructuralHolder {
            return Err(WorkError::refused(format!(
                "`{slug}` is a structural holder; its artifacts are frozen evidence of the split"
            )));
        }
        // Provenance: the upstream artifacts as they stand now.
        let derived_from = upstream_of(name)
            .iter()
            .map(|upstream| Upstream {
                artifact: ArtifactName::new(*upstream),
                digest: record.current_digest(upstream).cloned(),
            })
            .collect();
        let grounding = grounding_now(&mut s, index, name, content, &[])?;
        let version = Version {
            digest: digest_of(content),
            writer: s.actor()?,
            created: s.now,
            derived_from,
            sequence: 0,
            grounding,
            remote: None,
            content: Some(content.to_owned()),
        };
        s.commit("artifact written", |s| {
            let record = &mut s.items[index].record;
            let mut version = version;
            version.sequence = record.next_sequence();
            match record
                .artifacts
                .iter_mut()
                .find(|a| a.name.as_str() == name)
            {
                Some(artifact) => artifact.versions.push(version),
                None => record.artifacts.push(Artifact {
                    name: ArtifactName::new(name),
                    address: None,
                    versions: vec![version],
                }),
            }
            Ok(())
        })?;
        let written = s.items[index]
            .record
            .artifact(name)
            .and_then(|a| a.versions.last())
            .cloned()
            .ok_or_else(|| WorkError::refused("the artifact version was not recorded"))?;
        Ok(Outcome {
            value: written,
            reflection: s.status(index),
            docs_changed: std::mem::take(&mut s.docs_changed),
        })
    }

    /// The project's local store, for a read of the local record alone: its
    /// configuration loaded and its code checked to be on GitHub, with no
    /// lock taken and no GitHub request.
    fn local_store_checked(&self, project: &ProjectRef) -> Result<Store, WorkError> {
        let config = Configuration::load(&self.home, Some(&project.root))?;
        let api_url = github::api_url(&self.home)?;
        if let github::CodeHost::Elsewhere { reason } =
            github::code_host(&config, &api_url, &project.root)?
        {
            return Err(WorkError::CodeNotOnGitHub { reason });
        }
        Ok(Store::new(&self.home, &project.slug))
    }

    /// The record `address` names as this computer holds it (`github-record`,
    /// "Artifacts and stamps"), with no GitHub request and, on a current
    /// schema, no lock (an older one migrates once under the writer lock).
    /// A plain slug is the live local item; `<slug>@<identity-prefix>` an
    /// exact local entry, retained ones included. A todo this computer's
    /// record does not hold is unknown.
    fn local_record(
        &self,
        project: &ProjectRef,
        address: &TodoAddress,
    ) -> Result<TodoRecord, WorkError> {
        let store = self.local_store_checked(project)?;
        let (items, _) = store.existence()?;
        let now = self.now();
        let items: Vec<LocalItem> = items
            .into_iter()
            .filter(|it| !past_recovery(&it.record, now))
            .collect();
        let found = match address.identity {
            None => find_in(&items, &address.slug, false),
            Some(_) => exact_local(&items, address)?,
        };
        match found {
            Some(index) => Ok(items[index].record.clone()),
            None => Err(WorkError::UnknownTodo {
                project: project.slug.clone(),
                todo: address.slug.clone(),
            }),
        }
    }

    /// A todo's artifacts and their retained versions, from the record this
    /// computer holds ([`Work::local_record`]).
    pub fn list_artifacts(
        &self,
        project: &ProjectRef,
        address: &TodoAddress,
    ) -> Result<Vec<Artifact>, WorkError> {
        Ok(self.local_record(project, address)?.artifacts)
    }

    /// The content of the current version of artifact `name`.
    pub fn read_artifact(
        &self,
        project: &ProjectRef,
        address: &TodoAddress,
        name: &str,
    ) -> Result<String, WorkError> {
        let record = self.local_record(project, address)?;
        let version = record
            .artifact(name)
            .and_then(|a| a.versions.last())
            .ok_or_else(|| no_artifact(&address.slug, name))?;
        version
            .content
            .clone()
            .ok_or_else(|| content_missing(name, version.digest.as_str()))
    }

    /// The line diff between two retained versions of artifact `name`.
    pub fn diff_artifact(
        &self,
        project: &ProjectRef,
        address: &TodoAddress,
        name: &str,
        old: &str,
        new: &str,
    ) -> Result<Vec<DiffLine>, WorkError> {
        let record = self.local_record(project, address)?;
        let slug = &address.slug;
        let artifact = record
            .artifact(name)
            .ok_or_else(|| no_artifact(slug, name))?;
        let content = |digest: &str| -> Result<String, WorkError> {
            let version = version_of(artifact, digest).ok_or_else(|| {
                WorkError::refused(format!(
                    "`{name}` of `{slug}` retains no version {digest}; `instrukt todo artifacts list {slug}` shows its versions"
                ))
            })?;
            version
                .content
                .clone()
                .ok_or_else(|| content_missing(name, digest))
        };
        Ok(diff_lines(&content(old)?, &content(new)?))
    }

    fn transition(
        &self,
        project: &ProjectRef,
        slug: &TodoSlug,
        what: &'static str,
        change: impl FnOnce(&mut TodoRecord, jiff::Timestamp) -> Result<(), WorkError>,
    ) -> Result<Outcome<TodoRecord>, WorkError> {
        let mut s = Session::open(self, project)?;
        let index = s.enter(slug)?;
        let now = s.now;
        s.commit(what, |s| change(&mut s.items[index].record, now))?;
        Ok(Outcome {
            value: s.items[index].record.clone(),
            reflection: s.status(index),
            docs_changed: std::mem::take(&mut s.docs_changed),
        })
    }
}

/// The manual dispatch hold `actor` places at `at` for `reason`, as both
/// `instrukt todo create --hold` and `instrukt todo update --hold` record it;
/// a reason that is blank once trimmed is refused.
fn manual_hold(reason: &str, actor: &Actor, at: jiff::Timestamp) -> Result<HoldRecord, WorkError> {
    let reason = reason.trim();
    if reason.is_empty() {
        return Err(WorkError::refused(
            "a hold records why the work must wait; the reason is blank",
        ));
    }
    Ok(HoldRecord {
        source: HoldSource::Manual,
        reason: reason.to_owned(),
        by: actor.clone(),
        at,
    })
}

/// The record change `instrukt todo update` asks for; returns its
/// warnings.
fn apply_update(
    s: &mut Session<'_>,
    index: usize,
    request: &UpdateRequest,
    actor: &Actor,
) -> Result<Vec<String>, WorkError> {
    let mut warnings = Vec::new();
    let now = s.now;
    writable(&s.items[index].record)?;
    if let Some(new_slug) = &request.slug {
        if s.items[index].record.implementation_started() {
            return Err(WorkError::refused(
                "implementation has started; the todo keeps its name",
            ));
        }
        let id = s.items[index].record.id.clone();
        ensure_slug_free(s, new_slug, Some(&id))?;
        let record = &mut s.items[index].record;
        if &record.slug != new_slug && !record.prior_slugs.contains(&record.slug) {
            record.prior_slugs.push(record.slug.clone());
        }
        record.slug = new_slug.clone();
        for item in &mut s.items {
            for reference in item
                .record
                .after
                .iter_mut()
                .chain(item.record.group.iter_mut())
            {
                if reference.id == id {
                    reference.slug = new_slug.clone();
                }
            }
        }
    }
    if let Some(kind) = request.kind {
        change_kind(&mut s.items[index].record, kind, request.force)?;
    }
    let record = &mut s.items[index].record;
    if let Some(schedule) = &request.schedule {
        task_field(record, "a schedule")?;
        record.schedule = schedule.clone();
    }
    if let Some(agent) = &request.agent {
        task_field(record, "an agent preference")?;
        record.agent = Some(agent.clone());
    }
    if let Some(group) = &request.group {
        match group {
            Some(parent) => {
                let target = s
                    .live_ref(parent)
                    .map(|r| r.reference())
                    .ok_or_else(|| unknown(s, parent))?;
                join_group(s, index, &target, actor)?;
            }
            None => s.items[index].record.group = None,
        }
    }
    if let Some(after) = &request.after {
        if after.contains(&s.items[index].record.slug) {
            return Err(WorkError::refused("a todo cannot depend on itself"));
        }
        let refs = resolve_refs(s, after)?;
        s.items[index].record.after = refs;
    }
    if request.after.is_some() || request.group.is_some() {
        warnings.extend(check_prerequisites(s, index)?);
    }
    let record = &mut s.items[index].record;
    if request.release {
        let hold = record
            .hold
            .take()
            .ok_or_else(|| WorkError::refused(format!("`{}` is not on hold", record.slug)))?;
        record.hold_history.push(ReleasedHold {
            hold,
            released: now,
            by: actor.clone(),
        });
    }
    if let Some(reason) = &request.hold {
        let hold = manual_hold(reason, actor, now)?;
        if record.status == TodoStatus::StructuralHolder {
            return Err(WorkError::refused(format!(
                "`{}` is a structural holder; each child carries its own hold",
                record.slug
            )));
        }
        if record.hold.is_some() {
            return Err(WorkError::refused(format!(
                "`{}` is already on hold; release it with `instrukt todo update {} --release` first",
                record.slug, record.slug
            )));
        }
        record.hold = Some(hold);
    }
    if request.unclaim {
        let caller = s.caller_session()?;
        let machine = s.machine()?;
        let record = &mut s.items[index].record;
        let claim = record.claim.as_ref().ok_or_else(|| {
            WorkError::refused(format!("`{}` holds no execution claim", record.slug))
        })?;
        // A task's claim covers one run: only that run's session releases
        // it — the claiming session, or the one bound as its task runner.
        let is_task = record.kind == TodoKind::Task;
        let holder = claim.machine == machine
            && caller.as_ref().is_some_and(|caller| {
                claim.session.as_ref() == Some(caller)
                    || record.bindings.iter().any(|b| {
                        b.session.as_ref() == Some(caller)
                            && (!is_task
                                || b.role.as_str() == instrukt_runner::dispatch::TASK_RUNNER_ROLE)
                    })
            });
        if !holder {
            return Err(WorkError::refused(format!(
                "`{}` is claimed elsewhere; only the session holding the claim, or one bound to the todo on the claiming computer, releases it",
                record.slug
            )));
        }
        for binding in &mut record.bindings {
            if binding.attempt.assignment == instrukt_runner::interfaces::AssignmentKind::Driver
                && binding.session.as_ref() == caller.as_ref()
                && binding.attempt.released_at.is_none()
            {
                binding.attempt.released_at = Some(now);
            }
        }
        record.claim = None;
    }
    Ok(warnings)
}

fn change_kind(record: &mut TodoRecord, kind: TodoKind, force: bool) -> Result<(), WorkError> {
    if record.kind == kind {
        return Ok(());
    }
    if record.implementation_started() {
        return Err(WorkError::refused(
            "implementation has started; the todo keeps its kind",
        ));
    }
    let leaving_task = record.kind == TodoKind::Task;
    let entering_task = kind == TodoKind::Task;
    let orphans: Vec<String> = record
        .artifacts
        .iter()
        .map(|a| a.name.as_str())
        .filter(|name| {
            (leaving_task && *name == artifact_names::TASK_PLAN)
                || (entering_task && REPOSITORY_ARTIFACTS.contains(name))
        })
        .map(str::to_owned)
        .chain(
            (leaving_task && (record.schedule.is_some() || record.agent.is_some()))
                .then(|| "the task schedule and agent preference".to_owned()),
        )
        .collect();
    if !orphans.is_empty() && !force {
        return Err(WorkError::refused(format!(
            "a {} keeps no {}; pass `--force` to change the kind and leave them as orphaned history",
            kind,
            orphans.join(", ")
        )));
    }
    if leaving_task {
        record.schedule = None;
        record.agent = None;
    }
    record.kind = kind;
    Ok(())
}

/// Evidence only repository work produces.
const REPOSITORY_ARTIFACTS: [&str; 6] = [
    artifact_names::PRD,
    artifact_names::ARCHITECTURE,
    artifact_names::DIAGNOSIS,
    artifact_names::IMPLEMENTATION_PLAN,
    artifact_names::QUALITY_CHECKLIST,
    artifact_names::DEFERRALS,
];

fn task_field(record: &TodoRecord, what: &str) -> Result<(), WorkError> {
    if record.kind == TodoKind::Task {
        Ok(())
    } else {
        Err(WorkError::refused(format!(
            "{what} belongs to a task; `{}` is a {}",
            record.slug, record.kind
        )))
    }
}

/// Groups item `index` under `parent`, converting the parent into a
/// structural holder on its first child.
fn join_group(
    s: &mut Session<'_>,
    index: usize,
    parent: &TodoRef,
    actor: &Actor,
) -> Result<(), WorkError> {
    let child = s.items[index].record.reference();
    if parent.id == child.id {
        return Err(WorkError::refused("a todo cannot be its own group"));
    }
    if ancestors(s, &parent.id).contains(&child.id) {
        return Err(WorkError::refused(format!(
            "`{}` is grouped under `{}`; grouping them the other way round makes a loop",
            parent.slug, child.slug
        )));
    }
    let p = s.enter_ref(parent)?;
    let now = s.now;
    let holder = &mut s.items[p].record;
    let convertible = matches!(holder.status, TodoStatus::Active | TodoStatus::Iceboxed);
    if convertible && holder.implementation_started() {
        return Err(WorkError::refused(format!(
            "`{}` cannot become a group: implementation has already started",
            parent.slug
        )));
    }
    let holder = &mut s.items[p].record;
    match holder.status {
        TodoStatus::StructuralHolder => {}
        TodoStatus::Active | TodoStatus::Iceboxed => {
            // Its pending documentation moves to its first child once the
            // whole change is checked and saved (see `Session::commit`).
            s.pending_transfer = Some((holder.slug.clone(), child.slug.clone()));
            let holder = &mut s.items[p].record;
            holder.status = TodoStatus::StructuralHolder;
            holder.claim = None;
            if let Some(hold) = holder.hold.take() {
                holder.hold_history.push(ReleasedHold {
                    hold,
                    released: now,
                    by: actor.clone(),
                });
            }
        }
        _ => {
            return Err(WorkError::refused(format!(
                "`{}` cannot become a group: only work still before prepared, or an existing holder, holds children",
                holder.slug
            )));
        }
    }
    let reference = s.items[p].record.reference();
    s.items[index].record.group = Some(reference);
    Ok(())
}

/// Moves a new structural holder's pending documentation to its first
/// child (`dependencies-and-holds`, "Structural holders"), the last step of
/// a validated change: the files are rewritten in the working tree and
/// listed for the caller to commit. A docs refusal refuses the conversion.
pub(super) fn transfer_documentation(
    project: &ProjectRef,
    parent: &TodoSlug,
    child: &TodoSlug,
) -> Result<Vec<std::path::PathBuf>, WorkError> {
    match instrukt_core::docs::transfer_pending(&project.root, parent, child) {
        Ok(outcome) => {
            let mut changed = outcome.written;
            changed.extend(
                outcome
                    .renamed
                    .into_iter()
                    .flat_map(|(from, to)| [from, to]),
            );
            Ok(changed)
        }
        Err(instrukt_core::docs::DocsError::TransferRefused { path, reason }) => {
            Err(WorkError::refused(format!(
                "`{parent}` cannot become a group: its pending documentation cannot move to `{child}` ({}: {reason})",
                path.display()
            )))
        }
        Err(err) => Err(err.into()),
    }
}

/// Pure selection of previously-owned snippet IDs applicable to `artifact`'s
/// current version: the current version's recorded owned IDs plus any owned IDs
/// carried by a drift judgment recorded *after* that version (ignoring older
/// judgments that predate an artifact rewrite).
fn prior_owned_for_version(
    record: &TodoRecord,
    artifact: &ArtifactName,
) -> Vec<instrukt_core::ids::SnippetId> {
    let mut prior_owned = Vec::new();
    let latest_version = record
        .artifact(artifact.as_str())
        .and_then(|a| a.versions.last());
    let version_seq = latest_version.map_or(0, |v| v.sequence);
    if let Some(version) = latest_version {
        for snippet in &version.grounding.owned_snippets {
            if !prior_owned.contains(&snippet.id) {
                prior_owned.push(snippet.id.clone());
            }
        }
    }
    if let Some(prior_grounding) = record.stamps.iter().rev().find_map(|s| match &s.fact {
        StampFact::DriftReviewed {
            artifact: prior_artifact,
            grounding,
            ..
        } if prior_artifact == artifact && s.sequence > version_seq => Some(grounding),
        _ => None,
    }) {
        for snippet in &prior_grounding.owned_snippets {
            if !prior_owned.contains(&snippet.id) {
                prior_owned.push(snippet.id.clone());
            }
        }
    }
    prior_owned
}

/// Pure replay check for `StampFact::DriftReviewed`: only the latest drift
/// judgment for the artifact, matching fact and actor, with neither the
/// artifact itself nor any upstream artifact rewritten after that stamp,
/// counts as an idempotent replay.
fn current_drift_replay<'a>(
    record: &'a TodoRecord,
    fact: &StampFact,
    actor: &Actor,
) -> Option<&'a StampRecord> {
    let StampFact::DriftReviewed { artifact, .. } = fact else {
        return None;
    };
    let latest = record.stamps.iter().rev().find(|prior| {
        matches!(
            &prior.fact,
            StampFact::DriftReviewed { artifact: prior_artifact, .. }
                if prior_artifact == artifact
        )
    })?;
    if &latest.fact != fact || &latest.actor != actor {
        return None;
    }
    let rewritten_after = |name: &str| {
        record
            .artifact(name)
            .and_then(|a| a.versions.last())
            .is_some_and(|v| v.sequence > latest.sequence)
    };
    if rewritten_after(artifact.as_str())
        || upstream_of(artifact.as_str())
            .iter()
            .any(|u| rewritten_after(u))
    {
        return None;
    }
    Some(latest)
}

/// The grounding of artifact `name` of item `index` with `content`, as it
/// stands now (`artifact-currency`, "Drift evidence"): canonical head,
/// declared paths and snippets, owned snippets (retaining deleted `prior_owned`
/// IDs as absent), and each prerequisite's artifacts of the kinds `name` derives
/// from.
fn grounding_now(
    s: &mut Session<'_>,
    index: usize,
    name: &str,
    content: &str,
    prior_owned: &[instrukt_core::ids::SnippetId],
) -> Result<crate::evidence::Grounding, WorkError> {
    let (slug, after, bindings) = {
        let record = &s.items[index].record;
        (
            record.slug.clone(),
            record.after.clone(),
            record.bindings.clone(),
        )
    };
    let project_root = instrukt_core::repository::primary_worktree(&s.project.root)
        .map_err(|err| WorkError::refused(err.to_string()))?;
    let candidates = super::_grounding::candidate_worktrees(
        bindings
            .iter()
            .map(|b| (b.attempt.lane, b.worktree.as_deref())),
    );
    let owned_root = super::_grounding::owned_snippet_root(&project_root, &candidates)?;
    let mut grounding = super::_grounding::ground(
        s.home(),
        &project_root,
        &owned_root,
        &s.project.slug,
        &s.base,
        &slug,
        content,
        prior_owned,
    )?;
    for prerequisite in &after {
        let found = s.record_by_id(&prerequisite.id);
        for upstream in upstream_of(name) {
            grounding.dependencies.push(DependencyDigest {
                id: Some(prerequisite.id.clone()),
                todo: prerequisite.slug.clone(),
                artifact: ArtifactName::new(*upstream),
                digest: found
                    .as_ref()
                    .and_then(|r| r.current_digest(upstream).cloned()),
            });
        }
    }
    Ok(grounding)
}

/// The structural holders above `id`, nearest first.
fn ancestors(s: &Session<'_>, id: &TodoId) -> Vec<TodoId> {
    let mut found = Vec::new();
    let mut current = s.record_by_id(id).and_then(|r| r.group);
    while let Some(parent) = current {
        if found.contains(&parent.id) {
            break;
        }
        found.push(parent.id.clone());
        current = s.record_by_id(&parent.id).and_then(|r| r.group);
    }
    found
}

/// Checks item `index`'s prerequisites against the project graph: no
/// holder above it, no cycle. Returns a warning per iceboxed prerequisite.
fn check_prerequisites(s: &mut Session<'_>, index: usize) -> Result<Vec<String>, WorkError> {
    let record = s.items[index].record.clone();
    if let Some(cycle) = cycle_through(s, &record) {
        let names: Vec<String> = cycle.iter().map(ToString::to_string).collect();
        return Err(WorkError::refused(format!(
            "this change makes `{}` wait on itself: {}",
            record.slug,
            names.join(" → ")
        )));
    }
    let mut warnings = Vec::new();
    for prerequisite in &record.after {
        if s.record_by_id(&prerequisite.id)
            .is_some_and(|r| r.status == TodoStatus::Iceboxed)
        {
            warnings.push(format!(
                "prerequisite `{}` is parked in the icebox; `{}` stays blocked until it is delivered or the edge is removed",
                prerequisite.slug, record.slug
            ));
        }
    }
    Ok(warnings)
}

/// A cycle of waiting through `record`, if the project graph as this
/// computer's record holds it has one: the slugs from `record` back to
/// itself ([`super::_graph::first_cycle`]). The transaction that commits
/// the change judges the graph it holds again.
fn cycle_through(s: &Session<'_>, record: &TodoRecord) -> Option<Vec<TodoSlug>> {
    super::_graph::first_cycle(&wait_graph(s), std::slice::from_ref(&record.id))
}

/// The waiting graph as the session holds it: every local item.
fn wait_graph(s: &Session<'_>) -> Vec<super::_graph::WaitNode> {
    s.items
        .iter()
        .map(|it| super::_graph::WaitNode::of(&it.record, true))
        .collect()
}

fn resolve_refs(s: &mut Session<'_>, slugs: &[TodoSlug]) -> Result<Vec<TodoRef>, WorkError> {
    let mut refs: Vec<TodoRef> = Vec::new();
    for slug in slugs {
        let record = s.live_ref(slug).ok_or_else(|| unknown(s, slug))?;
        if !refs.iter().any(|r| r.id == record.id) {
            refs.push(record.reference());
        }
    }
    Ok(refs)
}

/// The version of `artifact` with `digest`. Writing appends a version even
/// when its content repeats an earlier one, so a digest can name several;
/// the first of them holding its content answers, else the first.
fn version_of<'a>(artifact: &'a Artifact, digest: &str) -> Option<&'a super::_model::Version> {
    let mut matching = artifact
        .versions
        .iter()
        .filter(|v| v.digest.as_str() == digest);
    let first = matching.next()?;
    Some(
        std::iter::once(first)
            .chain(matching)
            .find(|v| v.content.is_some())
            .unwrap_or(first),
    )
}

/// The refusal for an artifact version whose content this computer does
/// not hold.
fn content_missing(name: &str, digest: &str) -> WorkError {
    WorkError::refused(format!(
        "this computer does not hold the content of `{name}` version {digest}"
    ))
}

fn ensure_slug_free(
    s: &mut Session<'_>,
    slug: &TodoSlug,
    except: Option<&TodoId>,
) -> Result<(), WorkError> {
    match s.live_ref(slug) {
        Some(record) if Some(&record.id) != except => Err(WorkError::refused(format!(
            "a live todo is already named `{slug}`"
        ))),
        _ => Ok(()),
    }
}

fn writable(record: &TodoRecord) -> Result<(), WorkError> {
    match record.status {
        TodoStatus::Delivered => Err(WorkError::refused(format!(
            "`{}` is delivered; delivered work is read-only",
            record.slug
        ))),
        TodoStatus::Removed => Err(WorkError::refused(format!(
            "`{}` is withdrawn; restore it with `instrukt todo reactivate {}` first",
            record.slug, record.slug
        ))),
        _ => Ok(()),
    }
}

/// The binding a verification disposition is recorded under, admitted by
/// workflow's rule: the caller's bound verifier attempt (`completion`), or
/// on the record-only path its live driver binding, over a free and clean
/// shared workspace. Returns that binding's role and, for a verifier
/// attempt already released, its index for exact replay.
fn admit_disposition(
    record: &TodoRecord,
    caller: Option<&SessionRef>,
    cwd: &Path,
    completion: Option<usize>,
    disposition: VerificationDisposition,
) -> Result<(instrukt_core::ids::RoleName, Option<usize>), WorkError> {
    const NEITHER: &str = "a verification disposition is recorded by an independent bound verifier or the claim-holding orchestrator";
    const DOORS: &str = "a pass, fail or blocked comes from a new independent `verifier` attempt, and a Tier 0 skip or hotfix deferral from the claim-holding orchestrator";
    let caller = caller.ok_or_else(|| {
        WorkError::refused(format!("{NEITHER}; no agent session is active: {DOORS}"))
    })?;
    let (binding, driver) = match completion {
        Some(binding) => (binding, None),
        None => {
            let driver = super::_attempts::live_driver(record, caller).ok_or_else(|| {
                WorkError::refused(format!(
                    "{NEITHER}, and this session holds neither a verifier attempt nor the todo's live driver binding: {DOORS}"
                ))
            })?;
            (driver, Some(driver))
        }
    };
    let role = record.bindings[binding].role.clone();
    let commit = _stamps::head(cwd)?;
    crate::workflow::verification_admission(record, caller, &role, disposition, &commit)
        .map_err(WorkError::refused)?;
    if let Some(driver) = driver {
        super::_attempts::record_only_workspace(record, driver, cwd)?;
        return Ok((role, None));
    }
    let released = record.bindings[binding].attempt.released_at.is_some();
    Ok((role, released.then_some(binding)))
}

/// A released verifier attempt's exact replay: its retained disposition,
/// when the request names the same disposition and no follow-up and the
/// `verification-findings` report is unchanged; otherwise refused, since
/// a verifier attempt records one disposition (`stamps`, "Verification
/// dispositions").
fn released_disposition(
    record: &TodoRecord,
    binding: usize,
    disposition: VerificationDisposition,
    follow_up: Option<&TodoSlug>,
) -> Result<StampRecord, WorkError> {
    let retained = record.bindings[binding]
        .attempt
        .completion_sequence
        .and_then(|sequence| record.stamps.iter().find(|s| s.sequence == sequence));
    let Some(stamp) = retained else {
        return Err(WorkError::refused(
            "this released verifier attempt recorded no disposition; a new verifier attempt records one",
        ));
    };
    let StampFact::VerificationDisposition {
        disposition: recorded,
        report,
        ..
    } = &stamp.fact
    else {
        return Err(WorkError::refused(
            "this released verifier attempt recorded no disposition; a new verifier attempt records one",
        ));
    };
    let current = record.current_digest(artifact_names::VERIFICATION_FINDINGS);
    if *recorded == disposition && follow_up.is_none() && current == Some(report) {
        return Ok(stamp.clone());
    }
    Err(WorkError::refused(format!(
        "this verifier attempt recorded `{recorded}` for findings `{report}`; a different disposition or a rewritten report needs a new verifier attempt"
    )))
}

/// The orchestrator's earlier record-only disposition this one replays:
/// the latest disposition recorded, when the same actor recorded the same
/// disposition for the same commit, report and follow-up identity.
fn record_only_replay<'r>(
    record: &'r TodoRecord,
    fact: &StampFact,
    actor: &Actor,
) -> Option<&'r StampRecord> {
    let StampFact::VerificationDisposition {
        disposition,
        commit,
        report,
        follow_up,
    } = fact
    else {
        return None;
    };
    let latest = record
        .stamps
        .iter()
        .rev()
        .find(|s| matches!(s.fact, StampFact::VerificationDisposition { .. }))?;
    let StampFact::VerificationDisposition {
        disposition: d,
        commit: c,
        report: r,
        follow_up: f,
    } = &latest.fact
    else {
        return None;
    };
    let same_follow_up = f.as_ref().map(|t| &t.id) == follow_up.as_ref().map(|t| &t.id);
    (d == disposition && c == commit && r == report && same_follow_up && &latest.actor == actor)
        .then_some(latest)
}

fn role_of(record: &TodoRecord, actor: &Actor) -> Option<instrukt_core::ids::RoleName> {
    let Actor::Session(session) = actor else {
        return None;
    };
    record
        .bindings
        .iter()
        .find(|b| b.session.as_ref() == Some(session))
        .map(|b| b.role.clone())
}

fn no_withdrawn(key: &str) -> WorkError {
    WorkError::refused(format!(
        "no withdrawn todo `{key}` within its recovery window"
    ))
}

fn unknown(s: &Session<'_>, slug: &TodoSlug) -> WorkError {
    s.absent(slug)
}

fn no_artifact(slug: &TodoSlug, name: &str) -> WorkError {
    WorkError::refused(format!(
        "`{slug}` has no artifact `{name}`; `instrukt todo artifacts list {slug}` shows its artifacts"
    ))
}

fn status_word(status: TodoStatus) -> &'static str {
    super::_envelope::status_name(status)
}

fn view(s: &Session<'_>, index: usize) -> TodoView {
    let item = &s.items[index];
    let record = item.record.clone();
    let prerequisites = record
        .after
        .iter()
        .map(|r| PrerequisiteView {
            slug: r.slug.clone(),
            status: s.record_by_id(&r.id).map(|p| p.status),
        })
        .collect();
    let expanded_prerequisites = super::_ports::expand(s, &record);
    let children: Vec<TodoRef> = s
        .items
        .iter()
        .map(|it| &it.record)
        .filter(|r| r.group.as_ref().is_some_and(|g| g.id == record.id) && r.is_live())
        .map(TodoRecord::reference)
        .collect();
    TodoView {
        record_status: super::_envelope::record_status(&record).as_str(),
        reflection: s.status(index),
        issue: item.reflection.issue.map(|i| i.number),
        pull: item.reflection.pull,
        record,
        prerequisites,
        expanded_prerequisites,
        children,
    }
}

/// Readiness and dependency order: ready work first, each todo after its
/// prerequisites, then oldest first.
fn order(entries: Vec<ListEntry>) -> Vec<ListEntry> {
    let ids: BTreeMap<TodoId, usize> = entries
        .iter()
        .enumerate()
        .map(|(i, e)| (e.record.id.clone(), i))
        .collect();
    let mut remaining: Vec<usize> = (0..entries.len()).collect();
    remaining.sort_by_key(|&i| (!entries[i].ready, entries[i].record.created));
    let mut placed: BTreeSet<usize> = BTreeSet::new();
    let mut ordered = Vec::with_capacity(entries.len());
    while !remaining.is_empty() {
        let next = remaining
            .iter()
            .position(|&i| {
                entries[i]
                    .record
                    .after
                    .iter()
                    .filter_map(|r| ids.get(&r.id))
                    .all(|p| placed.contains(p))
            })
            .unwrap_or(0);
        let index = remaining.remove(next);
        placed.insert(index);
        ordered.push(index);
    }
    let mut slots: Vec<Option<ListEntry>> = entries.into_iter().map(Some).collect();
    ordered
        .into_iter()
        .filter_map(|i| slots.get_mut(i).and_then(Option::take))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evidence::ContentDigest;
    use instrukt_core::ids::ProjectSlug;

    fn record(slug: &str, n: u8, after: &[&TodoRecord]) -> TodoRecord {
        TodoRecord {
            schema: SCHEMA,
            id: TodoId::parse(&format!("{n:02x}").repeat(16)).unwrap(),
            project: ProjectSlug::parse("demo").unwrap(),
            slug: TodoSlug::parse(slug).unwrap(),
            prior_slugs: Vec::new(),
            kind: TodoKind::Change,
            status: TodoStatus::Active,
            created: jiff::Timestamp::from_second(i64::from(n)).unwrap(),
            created_by: Actor::Person { name: "dev".into() },
            human_involved: None,
            revision: 1,
            sequence: 0,
            artifacts: Vec::new(),
            stamps: Vec::new(),
            claim: None,
            bindings: Vec::new(),
            hold: None,
            hold_history: Vec::new(),
            label_actions: Vec::new(),
            after: after.iter().map(|r| r.reference()).collect(),
            group: None,
            schedule: None,
            catch_up: CatchUp::Once,
            agent: None,
            branch: None,
            delivery: None,
            removal: None,
        }
    }

    /// An artifact whose versions repeat `text`, holding content only where
    /// `held` says so, oldest first.
    fn repeated(text: &str, held: &[bool]) -> Artifact {
        Artifact {
            name: ArtifactName::new("log"),
            address: None,
            versions: held
                .iter()
                .enumerate()
                .map(|(i, held)| Version {
                    digest: digest_of(text),
                    writer: Actor::Person { name: "dev".into() },
                    created: jiff::Timestamp::UNIX_EPOCH,
                    derived_from: Vec::new(),
                    sequence: i as u64 + 1,
                    grounding: Default::default(),
                    remote: None,
                    content: held.then(|| text.to_owned()),
                })
                .collect(),
        }
    }

    #[test]
    fn a_digest_any_of_whose_versions_holds_its_content_is_answered_by_that_version() {
        let artifact = repeated("same", &[false, true]);
        let digest = digest_of("same");
        // The version that holds the content, whole: not the earlier repeat.
        assert_eq!(
            version_of(&artifact, digest.as_str()),
            Some(&artifact.versions[1])
        );
    }

    fn entry(record: TodoRecord, ready: bool) -> ListEntry {
        ListEntry {
            record,
            reflection: ReflectionStatus::Reflected,
            ready,
            claimed_on: None,
        }
    }

    #[test]
    fn listing_puts_ready_work_first_and_dependents_after_their_prerequisites() {
        let base = record("base", 3, &[]);
        let dependent = record("dependent", 1, &[&base]);
        let other = record("other", 2, &[]);
        let ordered = order(vec![
            entry(dependent.clone(), false),
            entry(other.clone(), true),
            entry(base.clone(), true),
        ]);
        let slugs: Vec<&str> = ordered.iter().map(|e| e.record.slug.as_str()).collect();
        assert_eq!(
            slugs,
            [
                other.slug.as_str(),
                base.slug.as_str(),
                dependent.slug.as_str()
            ]
        );
    }

    #[test]
    fn leaving_the_task_kind_with_a_task_plan_needs_force() {
        let mut task = record("nightly", 1, &[]);
        task.kind = TodoKind::Task;
        task.artifacts.push(Artifact {
            name: ArtifactName::new(artifact_names::TASK_PLAN),
            address: None,
            versions: Vec::new(),
        });
        assert!(matches!(
            change_kind(&mut task.clone(), TodoKind::Change, false),
            Err(WorkError::Refused(_))
        ));
        change_kind(&mut task, TodoKind::Change, true).unwrap();
        assert_eq!(task.kind, TodoKind::Change);
    }

    #[test]
    fn a_started_implementation_keeps_its_kind() {
        let mut started = record("started", 1, &[]);
        started.stamps.push(StampRecord {
            source_comment: None,
            fact: StampFact::BuildStarted {
                base: CommitSha::parse(&"a".repeat(40)).unwrap(),
                plan: ContentDigest::new("sha256:x"),
            },
            at: jiff::Timestamp::UNIX_EPOCH,
            actor: Actor::Person { name: "dev".into() },
            model: None,
            eval_fingerprint: None,
            role: None,
            human_involved: None,
            sequence: 0,
            model_provenance: Default::default(),
        });
        assert!(matches!(
            change_kind(&mut started, TodoKind::Bug, true),
            Err(WorkError::Refused(_))
        ));
    }

    fn drift_stamp(
        sequence: u64,
        artifact: &str,
        class: crate::evidence::DriftClass,
        actor: &Actor,
    ) -> StampRecord {
        StampRecord {
            source_comment: None,
            fact: StampFact::DriftReviewed {
                artifact: ArtifactName::new(artifact),
                class,
                reason: "assessed".into(),
                examined: ContentDigest::of("plan-v1"),
                grounding: Default::default(),
            },
            at: jiff::Timestamp::UNIX_EPOCH,
            actor: actor.clone(),
            model: None,
            eval_fingerprint: None,
            role: None,
            human_involved: None,
            sequence,
            model_provenance: Default::default(),
        }
    }

    fn artifact_with_seq(name: &str, sequence: u64) -> Artifact {
        Artifact {
            name: ArtifactName::new(name),
            address: None,
            versions: vec![Version {
                digest: ContentDigest::of(name),
                writer: Actor::Person { name: "dev".into() },
                created: jiff::Timestamp::UNIX_EPOCH,
                derived_from: Vec::new(),
                sequence,
                grounding: Default::default(),
                remote: None,
                content: None,
            }],
        }
    }

    #[test]
    fn current_drift_replay_returns_matching_latest_stamp_when_unrewritten() {
        let mut rec = record("demo", 1, &[]);
        let actor = Actor::Person { name: "dev".into() };
        rec.artifacts
            .push(artifact_with_seq(artifact_names::INPUT, 1));
        rec.artifacts
            .push(artifact_with_seq(artifact_names::IMPLEMENTATION_PLAN, 2));
        let stamp = drift_stamp(
            3,
            artifact_names::IMPLEMENTATION_PLAN,
            crate::evidence::DriftClass::Cosmetic,
            &actor,
        );
        rec.stamps.push(stamp.clone());
        assert_eq!(
            current_drift_replay(&rec, &stamp.fact, &actor).map(|s| s.sequence),
            Some(3)
        );
    }

    #[test]
    fn current_drift_replay_returns_none_after_upstream_artifact_rewrite() {
        let mut rec = record("demo", 1, &[]);
        let actor = Actor::Person { name: "dev".into() };
        rec.artifacts
            .push(artifact_with_seq(artifact_names::INPUT, 5));
        rec.artifacts
            .push(artifact_with_seq(artifact_names::IMPLEMENTATION_PLAN, 2));
        let stamp = drift_stamp(
            3,
            artifact_names::IMPLEMENTATION_PLAN,
            crate::evidence::DriftClass::Cosmetic,
            &actor,
        );
        rec.stamps.push(stamp.clone());
        assert!(current_drift_replay(&rec, &stamp.fact, &actor).is_none());
    }

    #[test]
    fn current_drift_replay_returns_none_after_target_artifact_rewrite() {
        let mut rec = record("demo", 1, &[]);
        let actor = Actor::Person { name: "dev".into() };
        rec.artifacts
            .push(artifact_with_seq(artifact_names::IMPLEMENTATION_PLAN, 5));
        let stamp = drift_stamp(
            3,
            artifact_names::IMPLEMENTATION_PLAN,
            crate::evidence::DriftClass::Cosmetic,
            &actor,
        );
        rec.stamps.push(stamp.clone());
        assert!(current_drift_replay(&rec, &stamp.fact, &actor).is_none());
    }

    #[test]
    fn current_drift_replay_returns_none_when_a_later_contrary_judgment_exists() {
        let mut rec = record("demo", 1, &[]);
        let actor = Actor::Person { name: "dev".into() };
        let cosmetic = drift_stamp(
            3,
            artifact_names::IMPLEMENTATION_PLAN,
            crate::evidence::DriftClass::Cosmetic,
            &actor,
        );
        let substantive = drift_stamp(
            4,
            artifact_names::IMPLEMENTATION_PLAN,
            crate::evidence::DriftClass::Substantive,
            &actor,
        );
        rec.stamps.push(cosmetic.clone());
        rec.stamps.push(substantive);
        assert!(current_drift_replay(&rec, &cosmetic.fact, &actor).is_none());
    }

    #[test]
    fn prior_owned_for_version_ignores_judgments_predating_artifact_rewrite() {
        let mut rec = record("demo", 1, &[]);
        let actor = Actor::Person { name: "dev".into() };
        let old_owned = instrukt_core::ids::SnippetId::parse("project/demo/policy/old").unwrap();
        let cur_owned = instrukt_core::ids::SnippetId::parse("project/demo/policy/cur").unwrap();
        rec.stamps.push(StampRecord {
            source_comment: None,
            fact: StampFact::DriftReviewed {
                artifact: ArtifactName::new(artifact_names::IMPLEMENTATION_PLAN),
                class: crate::evidence::DriftClass::Cosmetic,
                reason: "prior version".into(),
                examined: ContentDigest::of("plan-v1"),
                grounding: crate::evidence::Grounding {
                    owned_snippets: vec![crate::evidence::SnippetDigest {
                        id: old_owned,
                        digest: Some(ContentDigest::of("parent")),
                    }],
                    ..Default::default()
                },
            },
            at: jiff::Timestamp::UNIX_EPOCH,
            actor,
            model: None,
            eval_fingerprint: None,
            role: None,
            human_involved: None,
            sequence: 3,
            model_provenance: Default::default(),
        });
        rec.artifacts.push(Artifact {
            name: ArtifactName::new(artifact_names::IMPLEMENTATION_PLAN),
            address: None,
            versions: vec![Version {
                digest: ContentDigest::of("plan-v2"),
                writer: Actor::Person { name: "dev".into() },
                created: jiff::Timestamp::UNIX_EPOCH,
                derived_from: Vec::new(),
                sequence: 5,
                grounding: crate::evidence::Grounding {
                    owned_snippets: vec![crate::evidence::SnippetDigest {
                        id: cur_owned.clone(),
                        digest: Some(ContentDigest::of("cur")),
                    }],
                    ..Default::default()
                },
                remote: None,
                content: None,
            }],
        });
        assert_eq!(
            prior_owned_for_version(
                &rec,
                &ArtifactName::new(artifact_names::IMPLEMENTATION_PLAN)
            ),
            vec![cur_owned]
        );
    }
}
