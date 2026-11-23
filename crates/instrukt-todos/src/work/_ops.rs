//! The `instrukt todo` record operations (`todos`, `dependencies-and-holds`,
//! `stamps`, `approvals`, `github-record` "Artifacts and stamps"). Each
//! opens a session, validates the whole resulting record before it
//! commits, and flushes pending reflection.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use instrukt_core::config::Configuration;
use instrukt_core::ids::{CommitSha, TodoSlug};
use instrukt_core::machine;
use instrukt_core::publication::EvalFingerprint;
use instrukt_core::{git, github};
use instrukt_runner::events::{Event, EventLog};
use instrukt_runner::interfaces::{AgentPreference, CatchUp, ProjectRef, TaskSchedule, TodoKind};

use super::_diff::{DiffLine, diff_lines};
use super::_error::WorkError;
use super::_model::{
    Artifact, Delivery, ReleasedHold, Removal, SCHEMA, TodoAddress, TodoId, TodoRecord, TodoRef,
    Version, digest_of, valid_artifact_name,
};
use super::_session::{
    GitHubRead, Located, ReflectionStatus, Session, Unrestored, addressed, exact_local, find_in,
    past_recovery, status_of,
};
use super::_stamps::{self, StampInputs, StampRequest};
use super::_store::{LocalItem, Store};
use super::Work;
use crate::evidence::{
    Actor, ArtifactName, DependencyDigest, HoldRecord, HoldSource, StampFact, StampRecord,
    TodoStatus, Upstream, artifact_names, upstream_of,
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
    /// Whether the todo is a local item of this computer.
    pub local: bool,
    /// A local item's reflection; `None` for work read from GitHub.
    pub reflection: Option<ReflectionStatus>,
    pub issue: Option<u64>,
    pub pull: Option<u64>,
    pub prerequisites: Vec<PrerequisiteView>,
    /// Prerequisites expanded through structural holders, matching
    /// `WorkEvidence::evidence` so `todo show` derives the same ledger as
    /// `todo work`.
    pub expanded_prerequisites: Vec<crate::evidence::Prerequisite>,
    /// A structural holder's children.
    pub children: Vec<TodoRef>,
    /// Whether the todo was refreshed from GitHub for this view; when not,
    /// it is shown as this computer last knew it, and why.
    pub refresh: GitHubRead,
    /// For a todo that is not a local item: when its copy was read.
    pub copied_at: Option<jiff::Timestamp>,
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
    /// Whether GitHub's reflected work was read.
    pub github: GitHubRead,
    /// Local items whose newer GitHub revision met a local change: their
    /// record here may lack evidence recorded elsewhere.
    pub conflicted: Vec<TodoSlug>,
}

/// `instrukt todo list`: read from the state database alone.
#[derive(Debug, Clone)]
pub struct Listing {
    pub entries: Vec<ListEntry>,
}

#[derive(Debug, Clone)]
pub struct ListEntry {
    pub record: TodoRecord,
    pub local: bool,
    pub reflection: Option<ReflectionStatus>,
    /// Every prerequisite delivered.
    pub ready: bool,
    /// Where the todo is shown from: `None` for a todo this computer holds
    /// a claim on (the local record); otherwise its last known copy.
    pub last_known: Option<LastKnown>,
}

/// A todo this computer holds no claim on, as last known here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LastKnown {
    /// The computer its claim names; `None` when unclaimed.
    pub runs_on: Option<instrukt_core::machine::MachineId>,
    /// When the copy was read; `None` for a local item, whose last read
    /// from GitHub this computer does not record.
    pub copied_at: Option<jiff::Timestamp>,
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
        ensure_slug_free(&mut s, &request.slug, None)?;
        let actor = s.actor()?;
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
        let id = TodoId::new(machine::random_hex(16).map_err(|source| WorkError::Io {
            path: "/dev/urandom".into(),
            source,
        })?);
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
            hold: None,
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
        s.flush()?;
        Ok(Outcome {
            value: s.items[index].record.clone(),
            reflection: s.status(index),
            docs_changed: std::mem::take(&mut s.docs_changed),
        })
    }

    /// The todo `address` names: a plain slug is the live item;
    /// `<slug>@<identity-prefix>` an exact entry, retained ones included.
    pub fn show(&self, project: &ProjectRef, address: &TodoAddress) -> Result<TodoView, WorkError> {
        let mut s = Session::open(self, project)?;
        match s.locate(address)? {
            Located::Local(index) => {
                match s.pull_through(index) {
                    Ok(()) | Err(WorkError::Conflicted { .. }) => {}
                    Err(err) => return Err(err),
                }
                let item = s.items[index].clone();
                let view = view(&mut s, item.record, true);
                Ok(TodoView {
                    reflection: Some(s.status(index)),
                    issue: item.reflection.issue.map(|i| i.number),
                    pull: item.reflection.pull,
                    refresh: s.github_read(),
                    ..view
                })
            }
            Located::Remote {
                item,
                refreshed,
                copied_at,
            } => {
                let issue = item.reflection.issue.map(|i| i.number);
                let view = view(&mut s, item.record, false);
                let refresh = if refreshed {
                    GitHubRead::Read
                } else {
                    match s.github_read() {
                        GitHubRead::Read => GitHubRead::NotRead(
                            super::_session::PendingReason::Failed("not read".to_owned()),
                        ),
                        not_read => not_read,
                    }
                };
                Ok(TodoView {
                    issue,
                    refresh,
                    copied_at: Some(copied_at),
                    ..view
                })
            }
        }
    }

    /// Every record of the project's work that is available, for
    /// evaluation (`eval-report`, "Evidence source"), read as `show` reads
    /// it: this computer's items, retained ones included, each pulled
    /// through from GitHub first so evidence recorded elsewhere is in it,
    /// and reflected work GitHub holds that is not local. Whether GitHub
    /// answered, and which items conflict with a newer GitHub revision, are
    /// reported alongside.
    pub fn recorded_work(&self, project: &ProjectRef) -> Result<RecordedWork, WorkError> {
        let mut s = Session::open(self, project)?;
        s.pull_all()?;
        let conflicted: Vec<TodoSlug> = (0..s.items.len())
            .filter(|&index| s.status(index) == ReflectionStatus::Conflicted)
            .map(|index| s.items[index].record.slug.clone())
            .collect();
        let mut records: Vec<TodoRecord> = s.items.iter().map(|it| it.record.clone()).collect();
        let local: BTreeSet<TodoId> = records.iter().map(|r| r.id.clone()).collect();
        for (record, _) in s.remote_all().unwrap_or_default() {
            if !local.contains(&record.id) {
                records.push(record);
            }
        }
        Ok(RecordedWork {
            records,
            github: s.github_read(),
            conflicted,
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
    /// no listing ever reads GitHub (`github-record`, "Interest-driven
    /// refresh"). The todos this computer holds claims on are shown from
    /// the local record; every other todo as last known, labelled with the
    /// computer its claim names and the copy's age.
    pub fn list(&self, project: &ProjectRef, selection: Selection) -> Result<Listing, WorkError> {
        // A local read: no GitHub request, and no lock on a current schema;
        // a database behind this build's schema migrates once under the
        // writer lock (`state-database`, "Writing"). Withdrawn work past its
        // recovery window is left out; a writing command purges it.
        let store = self.local_store_checked(project)?;
        let (items, copies, provider) = store.existence()?;
        let now = self.now();
        let items: Vec<LocalItem> = items
            .into_iter()
            .filter(|it| !past_recovery(&it.record, now))
            .collect();
        let unread = super::_session::unavailable(self, &provider, now);
        let machine = machine::machine_id(&self.home)?;
        let wanted = match selection {
            Selection::Active => vec![TodoStatus::Active, TodoStatus::StructuralHolder],
            Selection::Iceboxed => vec![TodoStatus::Iceboxed],
            Selection::Removed => vec![TodoStatus::Removed],
            Selection::Delivered => vec![TodoStatus::Delivered],
        };
        let runs_on = |record: &TodoRecord| record.claim.as_ref().map(|c| c.machine.clone());
        let mut entries: Vec<ListEntry> = items
            .iter()
            .filter(|it| wanted.contains(&it.record.status))
            .map(|it| {
                let here = runs_on(&it.record).as_ref() == Some(&machine);
                ListEntry {
                    record: it.record.clone(),
                    local: true,
                    reflection: Some(status_of(it, unread.clone())),
                    ready: false,
                    last_known: (!here).then(|| LastKnown {
                        runs_on: runs_on(&it.record),
                        copied_at: it.reflection.refreshed_at,
                    }),
                }
            })
            .collect();
        let local_ids: BTreeSet<TodoId> = items.iter().map(|it| it.record.id.clone()).collect();
        for copy in &copies {
            if !local_ids.contains(&copy.record.id) && wanted.contains(&copy.record.status) {
                entries.push(ListEntry {
                    last_known: Some(LastKnown {
                        runs_on: runs_on(&copy.record),
                        copied_at: Some(copy.copied_at),
                    }),
                    record: copy.record.clone(),
                    local: false,
                    reflection: None,
                    ready: false,
                });
            }
        }
        let delivered: BTreeSet<TodoId> = items
            .iter()
            .map(|it| &it.record)
            .chain(copies.iter().map(|copy| &copy.record))
            .chain(entries.iter().map(|e| &e.record))
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
        s.flush()?;
        Ok(Outcome {
            value: UpdateReport {
                record: s.items[index].record.clone(),
                warnings,
            },
            reflection: s.status(index),
            docs_changed: std::mem::take(&mut s.docs_changed),
        })
    }

    /// `instrukt todo refresh`: replaces the local item `address` names
    /// with GitHub's committed record. It is the door a conflicted todo
    /// names, retained work included: a withdrawn todo's reactivation
    /// refuses until it is refreshed, and its door gives its exact address.
    pub fn refresh(
        &self,
        project: &ProjectRef,
        address: &TodoAddress,
    ) -> Result<Outcome<TodoRecord>, WorkError> {
        let mut s = Session::open(self, project)?;
        let index = addressed(&s.items, address)?.ok_or_else(|| unknown(&s, &address.slug))?;
        s.refresh(index)?;
        s.flush()?;
        Ok(Outcome {
            value: s.items[index].record.clone(),
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

    pub fn remove(
        &self,
        project: &ProjectRef,
        slug: &TodoSlug,
    ) -> Result<Outcome<TodoRecord>, WorkError> {
        let mut s = Session::open(self, project)?;
        let index = s.enter(slug)?;
        writable(&s.items[index].record)?;
        let now = s.now;
        // One writer transaction, with no network request: the live row is
        // judged first, so a refused withdrawal releases nothing; every
        // working resource is checked before any is released, a worktree
        // holding uncommitted work or locked refusing the withdrawal; then
        // they are released, and the withdrawal commits naming only what
        // was. A resource git keeps after all refuses the withdrawal whole.
        s.commit_admitted("removed", index, |record| {
            writable(record)?;
            let released = plan_release(&project.root, record)?.perform(&project.root, slug)?;
            record.removal = Some(Removal {
                at: now,
                from: record.status,
                branch: released.branch,
                worktrees: released.worktrees,
            });
            record.status = TodoStatus::Removed;
            record.claim = None;
            Ok(())
        })?;
        s.flush()?;
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
        // Withdrawn here and withdrawn on another computer (still inside
        // its window) are candidates alike: one is restored, several are
        // refused naming each.
        // A full identity found here names one todo for good.
        let exact = matches!(removed.as_slice(), [i] if s.items[*i].record.id.as_str() == key);
        let what = format!("withdrawn todo `{key}`");
        let mut remote = if exact {
            Vec::new()
        } else {
            s.remote_candidates(&what, wanted)?
        };
        let index = match (removed.as_slice(), remote.len()) {
            ([index], 0) => *index,
            ([], 1) => match remote.pop() {
                Some((record, issue)) => {
                    let item = s.localize_found(record, issue)?;
                    s.adopt(item)?
                }
                None => return Err(no_withdrawn(key)),
            },
            ([], 0) => return Err(no_withdrawn(key)),
            (local, _) => {
                let records: Vec<TodoRecord> = local
                    .iter()
                    .map(|i| s.items[*i].record.clone())
                    .chain(remote.into_iter().map(|(record, _)| record))
                    .collect();
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
        s.pull_through(index)?;
        s.commit("reactivated", |s| {
            let record = &mut s.items[index].record;
            let from = record
                .removal
                .take()
                .map_or(TodoStatus::Active, |removal| removal.from);
            record.status = from;
            Ok(())
        })?;
        s.flush()?;
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
        s.flush()?;
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
        if let Some((binding, _)) = completion
            && !matches!(request, StampRequest::DriftReviewed { .. })
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
        let resolved = _stamps::resolve(
            request,
            &record,
            &StampInputs {
                cwd,
                base_branch: &base,
                session: session.as_ref(),
                now: s.now,
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
            role: role_of(&record, &actor),
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
            s.flush()?;
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
        } else {
            current_drift_replay(&record, &stamp.fact, &stamp.actor).cloned()
        };
        if let Some(prior) = replay_prior {
            if let Some((binding, _)) = completion
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
                s.flush()?;
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
            if let Some((binding, _)) = completion {
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
        s.flush()?;
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
        s.flush()?;
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
    /// "Artifacts and stamps"), from one snapshot of the local items and the
    /// listing copies, with no GitHub request and, on a current schema, no
    /// lock (an older one migrates once under the writer lock). It resolves
    /// as [`Session::locate`] does: a plain slug is the live local item, else
    /// the live listing copy; `<slug>@<identity-prefix>` an exact local
    /// entry, retained ones included, else the one listing copy it names.
    /// `None` when neither answers the address.
    fn local_record(
        &self,
        project: &ProjectRef,
        address: &TodoAddress,
    ) -> Result<Option<Held>, WorkError> {
        let store = self.local_store_checked(project)?;
        let (items, copies, _) = store.existence()?;
        let now = self.now();
        let items: Vec<LocalItem> = items
            .into_iter()
            .filter(|it| !past_recovery(&it.record, now))
            .collect();
        let found = match address.identity {
            None => find_in(&items, &address.slug, false),
            Some(_) => exact_local(&items, address)?,
        };
        if let Some(index) = found {
            return Ok(Some(Held::Item(items[index].record.clone())));
        }
        let mut copied =
            copies
                .into_iter()
                .map(|copy| copy.record)
                .filter(|r| match address.identity {
                    None => r.slug == address.slug && r.is_live(),
                    Some(_) => address.names(r),
                });
        Ok(match (address.identity.is_some(), copied.next()) {
            (true, Some(record)) => copied.next().is_none().then_some(record),
            (_, found) => found,
        }
        .map(Held::Copy))
    }

    /// The record `address` names for a read of artifact `name`, with why
    /// each version `wanted` asks for still has no content. A record this
    /// computer holds answers alone when it settles the read
    /// ([`Held::answers`]). Otherwise the todo is located and refreshed as [`Work::show`] does;
    /// each version still without content is then asked of GitHub and
    /// verified against its digest, and content restored onto a local item
    /// is committed to its row before the read returns it.
    fn artifact_record(
        &self,
        project: &ProjectRef,
        address: &TodoAddress,
        name: &str,
        wanted: &Wanted<'_>,
    ) -> Result<(TodoRecord, BTreeMap<String, Unrestored>), WorkError> {
        let lacking = |record: &TodoRecord| -> Vec<String> {
            record
                .artifact(name)
                .map_or_else(Vec::new, |artifact| wanted.lacking(artifact))
        };
        if let Some(held) = self.local_record(project, address)?
            && held.answers(name, wanted)
        {
            return Ok((held.into_record(), BTreeMap::new()));
        }
        let mut s = Session::open(self, project)?;
        let (mut record, issue, local) = match s.locate(address)? {
            Located::Local(index) => {
                match s.pull_through(index) {
                    Ok(()) | Err(WorkError::Conflicted { .. }) => {}
                    Err(err) => return Err(err),
                }
                let item = &s.items[index];
                (item.record.clone(), item.reflection.issue, Some(index))
            }
            Located::Remote { item, .. } => (item.record, item.reflection.issue, None),
        };
        let wanted = lacking(&record);
        let missing = s.restore_contents(issue, &mut record, name, &wanted);
        if let Some(index) = local {
            s.keep_restored(index, &record, name)?;
        }
        Ok((record, missing))
    }

    /// A todo's artifacts and their retained versions: from the record this
    /// computer holds ([`Work::local_record`]), else as [`Work::show`] finds
    /// the todo.
    pub fn list_artifacts(
        &self,
        project: &ProjectRef,
        address: &TodoAddress,
    ) -> Result<Vec<Artifact>, WorkError> {
        if let Some(held) = self.local_record(project, address)? {
            return Ok(held.into_record().artifacts);
        }
        Ok(self.show(project, address)?.record.artifacts)
    }

    /// The content of the current version of artifact `name`.
    pub fn read_artifact(
        &self,
        project: &ProjectRef,
        address: &TodoAddress,
        name: &str,
    ) -> Result<String, WorkError> {
        let (record, missing) = self.artifact_record(project, address, name, &Wanted::Current)?;
        let version = record
            .artifact(name)
            .and_then(|a| a.versions.last())
            .ok_or_else(|| no_artifact(&address.slug, name))?;
        let digest = version.digest.as_str();
        version
            .content
            .clone()
            .ok_or_else(|| content_missing(name, digest, missing.get(digest)))
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
        let (record, missing) =
            self.artifact_record(project, address, name, &Wanted::Digests(&[old, new]))?;
        let slug = &address.slug;
        let artifact = record
            .artifact(name)
            .ok_or_else(|| no_artifact(slug, name))?;
        let content = |digest: &str| -> Result<String, WorkError> {
            let version = Wanted::version(artifact, digest).ok_or_else(|| {
                WorkError::refused(format!(
                    "`{name}` of `{slug}` retains no version {digest}; `instrukt todo artifacts list {slug}` shows its versions"
                ))
            })?;
            version
                .content
                .clone()
                .ok_or_else(|| content_missing(name, digest, missing.get(digest)))
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
        s.flush()?;
        Ok(Outcome {
            value: s.items[index].record.clone(),
            reflection: s.status(index),
            docs_changed: std::mem::take(&mut s.docs_changed),
        })
    }
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
        let reason = reason.trim();
        if reason.is_empty() {
            return Err(WorkError::refused(
                "a hold records why the work must wait; the reason is blank",
            ));
        }
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
        record.hold = Some(HoldRecord {
            source: HoldSource::Manual,
            reason: reason.to_owned(),
            by: actor.clone(),
            at: now,
        });
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

/// The working resources a withdrawn todo owns, checked and ready to be
/// released (`todos`, "Parking, withdrawal and delivery").
struct Release {
    /// The todo's worktrees.
    worktrees: Vec<std::path::PathBuf>,
    /// Its work branch, when it exists.
    branch: Option<String>,
    /// Its empty workspace directory, when present.
    workspace: Option<std::path::PathBuf>,
}

/// What a [`Release`] released, kept as recovery evidence: the worktrees
/// removed, and the branch deleted with the commit it pointed at.
struct Released {
    worktrees: Vec<std::path::PathBuf>,
    branch: Option<super::_model::ReleasedBranch>,
}

/// A worktree of the project as git lists it.
struct Worktree {
    path: std::path::PathBuf,
    /// The branch it has checked out.
    branch: Option<String>,
    /// Why it is locked, when it is (`""` for no reason given).
    locked: Option<String>,
}

/// The project's worktrees, the primary one first.
fn worktrees(root: &Path) -> Result<Vec<Worktree>, git::GitError> {
    let listed = git::run(root, ["worktree", "list", "--porcelain"])?;
    let mut found: Vec<Worktree> = Vec::new();
    for line in listed.lines() {
        if let Some(path) = line.strip_prefix("worktree ") {
            found.push(Worktree {
                path: std::path::PathBuf::from(path),
                branch: None,
                locked: None,
            });
        } else if let Some(last) = found.last_mut() {
            if let Some(branch) = line.strip_prefix("branch refs/heads/") {
                last.branch = Some(branch.to_owned());
            } else if line == "locked" {
                last.locked = Some(String::new());
            } else if let Some(reason) = line.strip_prefix("locked ") {
                last.locked = Some(reason.to_owned());
            }
        }
    }
    Ok(found)
}

/// Checks the working resources a withdrawn todo owns: each worktree its
/// session bindings returned that is a worktree of this project, then its
/// work branch. Only git's own worktree list and the recorded branch are
/// acted on; a path that is not one of the project's worktrees is left
/// alone. A worktree with uncommitted work or locked, a workspace holding
/// files or a work branch checked out in the project refuses the
/// withdrawal. Nothing is released yet.
fn plan_release(root: &Path, record: &TodoRecord) -> Result<Release, WorkError> {
    let listed = worktrees(root)?;
    let main = listed.first().map(|w| w.path.clone());
    // The todo's working copies: worktrees its sessions returned, and any
    // other worktree with its work branch checked out.
    let mut owned: Vec<&Worktree> = Vec::new();
    for worktree in &listed {
        let bound = record
            .bindings
            .iter()
            .any(|b| b.worktree.as_ref() == Some(&worktree.path));
        let on_branch = worktree.branch.is_some() && worktree.branch == record.branch;
        if (bound || on_branch)
            && main.as_ref() != Some(&worktree.path)
            && !owned.iter().any(|o| o.path == worktree.path)
        {
            owned.push(worktree);
        }
    }
    // Check everything before releasing anything: work that would be lost,
    // or a worktree git keeps by its lock, refuses the withdrawal and names
    // where it is.
    for worktree in &owned {
        if let Some(reason) = &worktree.locked {
            let why = if reason.is_empty() {
                String::new()
            } else {
                format!(" ({reason})")
            };
            return Err(WorkError::refused(format!(
                "the worktree {} is locked{why}; unlock it with `git worktree unlock`, then withdraw again",
                worktree.path.display()
            )));
        }
        let changes = git::run(&worktree.path, ["status", "--porcelain"])?;
        if !changes.trim().is_empty() {
            return Err(WorkError::refused(format!(
                "the worktree {} holds uncommitted changes; commit or discard them, then withdraw again",
                worktree.path.display()
            )));
        }
    }
    let workspace = root.join("workspace").join(record.slug.as_str());
    let workspace_present = match std::fs::symlink_metadata(&workspace) {
        Ok(meta) if meta.file_type().is_symlink() => {
            return Err(WorkError::Symlink { path: workspace });
        }
        Ok(meta) if meta.is_dir() => true,
        Ok(_) => false,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => false,
        Err(source) => {
            return Err(WorkError::Io {
                path: workspace,
                source,
            });
        }
    };
    if workspace_present {
        let mut entries = std::fs::read_dir(&workspace).map_err(|source| WorkError::Io {
            path: workspace.clone(),
            source,
        })?;
        if entries.next().is_some() {
            return Err(WorkError::refused(format!(
                "the workspace {} still holds files; keep or remove them, then withdraw again",
                workspace.display()
            )));
        }
    }
    let branch = match &record.branch {
        Some(name) if branch_head(root, name)?.is_some() => {
            let current = match git::run(root, ["symbolic-ref", "--quiet", "--short", "HEAD"]) {
                Ok(current) => Some(current.trim().to_owned()),
                Err(git::GitError::Failed { .. }) => None,
                Err(err) => return Err(err.into()),
            };
            if current.as_deref() == Some(name.as_str()) {
                return Err(WorkError::refused(format!(
                    "the work branch `{name}` is checked out in the project; switch away from it, then withdraw again"
                )));
            }
            Some(name.clone())
        }
        _ => None,
    };
    Ok(Release {
        worktrees: owned.iter().map(|w| w.path.clone()).collect(),
        branch,
        workspace: workspace_present.then_some(workspace),
    })
}

/// The commit work branch `name` points at, `None` when there is no such
/// branch.
fn branch_head(root: &Path, name: &str) -> Result<Option<CommitSha>, git::GitError> {
    let reference = format!("refs/heads/{name}");
    match git::run(
        root,
        ["rev-parse", "--verify", "--quiet", reference.as_str()],
    ) {
        Ok(sha) => Ok(CommitSha::parse(sha.trim()).ok()),
        Err(git::GitError::Failed { .. }) => Ok(None),
        Err(err) => Err(err),
    }
}

impl Release {
    /// Releases what [`plan_release`] checked: the workspace, the
    /// worktrees, then the work branch. Git's own refusals protect what
    /// changed since the check: a worktree that gained uncommitted work or a
    /// lock is kept, as is a workspace that gained files, and a branch that
    /// moved on keeps its newer commits ([`delete_branch`]). Anything kept
    /// refuses the withdrawal of `slug`, naming it and what was already
    /// released; nothing is released after it.
    fn perform(self, root: &Path, slug: &TodoSlug) -> Result<Released, WorkError> {
        let mut done = Released {
            worktrees: Vec::new(),
            branch: None,
        };
        let mut already: Vec<String> = Vec::new();
        let kept = |why: String, already: &[String]| {
            let released = if already.is_empty() {
                String::new()
            } else {
                format!(" (already released: {})", already.join(", "))
            };
            WorkError::refused(format!(
                "`{slug}` is not withdrawn: {why}{released}; make its work safe, then withdraw again"
            ))
        };
        if let Some(workspace) = &self.workspace {
            if let Err(source) = instrukt_core::fsx::remove_dir(workspace) {
                return Err(kept(
                    format!(
                        "its workspace {} was not removed ({source})",
                        workspace.display()
                    ),
                    &already,
                ));
            }
            already.push(format!("its workspace {}", workspace.display()));
        }
        for path in self.worktrees {
            let target = path.to_string_lossy().into_owned();
            let why = match git::run(root, ["worktree", "remove", target.as_str()]) {
                Ok(_) => {
                    already.push(format!("its worktree {}", path.display()));
                    done.worktrees.push(path);
                    continue;
                }
                Err(git::GitError::Failed { stderr, .. }) => format!(
                    "git kept its worktree {} ({})",
                    path.display(),
                    stderr.trim()
                ),
                Err(err) => format!("its worktree {} was not removed ({err})", path.display()),
            };
            return Err(kept(why, &already));
        }
        if let Some(name) = &self.branch {
            done.branch = delete_branch(root, name).map_err(|why| kept(why, &already))?;
        }
        Ok(done)
    }
}

/// Deletes work branch `name` once no worktree has it checked out, with
/// git's compare-and-delete against the head it read: a branch that moves
/// on in between is kept with its newer commits. Returns the branch and
/// the commit it pointed at when deleted, `None` when there is no such
/// branch; otherwise why it is kept.
fn delete_branch(root: &Path, name: &str) -> Result<Option<super::_model::ReleasedBranch>, String> {
    let kept = |why: String| format!("its work branch `{name}` is kept: {why}");
    let listed = worktrees(root).map_err(|err| kept(err.to_string()))?;
    if let Some(user) = listed.iter().find(|w| w.branch.as_deref() == Some(name)) {
        return Err(kept(format!(
            "the worktree {} has it checked out",
            user.path.display()
        )));
    }
    let head = match branch_head(root, name) {
        Ok(Some(head)) => head,
        Ok(None) => return Ok(None),
        Err(err) => return Err(kept(err.to_string())),
    };
    let reference = format!("refs/heads/{name}");
    tracing::info!(
        event = "work_branch_releasing",
        branch = name,
        head = %head,
        "deleting a withdrawn todo's work branch at its last commit"
    );
    match git::run(
        root,
        ["update-ref", "-d", reference.as_str(), head.as_str()],
    ) {
        Ok(_) => {}
        Err(git::GitError::Failed { stderr, .. }) => {
            return Err(kept(format!(
                "it moved on from {head} while it was released ({})",
                stderr.trim()
            )));
        }
        Err(err) => return Err(kept(err.to_string())),
    }
    // Git's own branch deletion also drops the branch's configuration; a
    // branch without any has none to drop.
    let section = format!("branch.{name}");
    if let Err(err) = git::run(root, ["config", "--remove-section", section.as_str()]) {
        tracing::debug!(event = "work_branch_config_kept", branch = name, error = %err, "no branch configuration removed");
    }
    Ok(Some(super::_model::ReleasedBranch {
        name: name.to_owned(),
        head,
    }))
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
fn ancestors(s: &mut Session<'_>, id: &TodoId) -> Vec<TodoId> {
    let mut found = Vec::new();
    let mut current = s.record_current(id).and_then(|r| r.group);
    while let Some(parent) = current {
        if found.contains(&parent.id) {
            break;
        }
        found.push(parent.id.clone());
        current = s.record_current(&parent.id).and_then(|r| r.group);
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
        if s.record_current(&prerequisite.id)
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

/// A cycle of waiting through `record`, if the project graph has one: the
/// slugs from `record` back to itself ([`super::_graph::first_cycle`]). Every todo
/// the waiting from `record` reaches is refreshed first, once per command,
/// so the graph is judged as GitHub holds it now.
fn cycle_through(s: &mut Session<'_>, record: &TodoRecord) -> Option<Vec<TodoSlug>> {
    loop {
        let graph = wait_graph(s);
        let unrefreshed: Vec<TodoId> = super::_graph::reached(&graph, &record.id)
            .into_iter()
            .filter(|id| !s.consulted(id))
            .collect();
        if unrefreshed.is_empty() {
            return super::_graph::first_cycle(&graph, std::slice::from_ref(&record.id));
        }
        for id in &unrefreshed {
            s.record_current(id);
        }
    }
}

/// The waiting graph as the session holds it: every local item, and the
/// listing copy of every todo that is not one.
fn wait_graph(s: &Session<'_>) -> Vec<super::_graph::WaitNode> {
    let mut graph: Vec<super::_graph::WaitNode> = s
        .items
        .iter()
        .map(|it| super::_graph::WaitNode::of(&it.record, true))
        .collect();
    for copy in &s.copies {
        if !graph.iter().any(|n| n.id == copy.record.id) {
            graph.push(super::_graph::WaitNode::of(&copy.record, false));
        }
    }
    graph
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

/// A record this computer holds for a todo ([`Work::local_record`]).
enum Held {
    /// A local item's: the local record it reads.
    Item(TodoRecord),
    /// A listing copy's: the last-known reflected record, without artifact
    /// contents.
    Copy(TodoRecord),
}

impl Held {
    /// Whether this record settles a read of artifact `name` for `wanted`
    /// on its own. A local item's does when it lacks none of the content
    /// asked for, an artifact or version it does not carry answering that
    /// there is none. A listing copy's is only last known: something
    /// written after it was read is missing from it, so it settles a read
    /// only by holding every version asked for with its content.
    fn answers(&self, name: &str, wanted: &Wanted<'_>) -> bool {
        match self {
            Held::Item(record) => record
                .artifact(name)
                .is_none_or(|artifact| wanted.lacking(artifact).is_empty()),
            Held::Copy(record) => record
                .artifact(name)
                .is_some_and(|artifact| wanted.holds(artifact)),
        }
    }

    fn into_record(self) -> TodoRecord {
        match self {
            Held::Item(record) | Held::Copy(record) => record,
        }
    }
}

/// What a read of an artifact's content asks for.
enum Wanted<'d> {
    /// Its current version: the last one written.
    Current,
    /// The versions with these digests. Writing appends a version even when
    /// its content repeats an earlier one, so a digest can name several;
    /// any of them holding its content answers.
    Digests(&'d [&'d str]),
}

impl Wanted<'_> {
    /// The digests of the versions asked for whose content `artifact`
    /// lacks: the current version's when it is held without content; a
    /// digest whose versions all lack it. A digest no version carries lacks
    /// nothing: there is no content to restore.
    fn lacking(&self, artifact: &Artifact) -> Vec<String> {
        match self {
            Wanted::Current => artifact
                .versions
                .last()
                .filter(|v| v.content.is_none())
                .map(|v| v.digest.as_str().to_owned())
                .into_iter()
                .collect(),
            Wanted::Digests(digests) => digests
                .iter()
                .filter(|digest| {
                    Wanted::version(artifact, digest).is_some_and(|v| v.content.is_none())
                })
                .map(|digest| (*digest).to_owned())
                .collect(),
        }
    }

    /// Whether `artifact` holds every version asked for with its content.
    fn holds(&self, artifact: &Artifact) -> bool {
        match self {
            Wanted::Current => artifact
                .versions
                .last()
                .is_some_and(|v| v.content.is_some()),
            Wanted::Digests(digests) => digests.iter().all(|digest| {
                Wanted::version(artifact, digest).is_some_and(|v| v.content.is_some())
            }),
        }
    }

    /// The version of `artifact` with `digest` a read takes: one holding
    /// its content, else the first.
    fn version<'a>(artifact: &'a Artifact, digest: &str) -> Option<&'a Version> {
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
}

/// The refusal for an artifact version whose content this computer does
/// not hold, saying why restoring it failed ([`Unrestored`]).
fn content_missing(name: &str, digest: &str, why: Option<&Unrestored>) -> WorkError {
    let held = format!("this computer does not hold the content of `{name}` version {digest}");
    WorkError::refused(match why {
        Some(Unrestored::NotServed) => format!("{held}, and GitHub no longer serves it"),
        Some(Unrestored::NotReflected) => {
            format!("{held}, and it records no GitHub copy of it to restore it from")
        }
        Some(Unrestored::Unread(reason)) => format!(
            "{held}, and GitHub could not be read for it: {}",
            super::_session::describe(reason)
        ),
        None => format!("{held}, and GitHub was not asked for it"),
    })
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

fn view(s: &mut Session<'_>, record: TodoRecord, local: bool) -> TodoView {
    let prerequisites = record
        .after
        .iter()
        .map(|r| PrerequisiteView {
            slug: r.slug.clone(),
            status: s.record_by_id(&r.id).map(|p| p.status),
        })
        .collect();
    let expanded_prerequisites = super::_ports::expand(s, &record);
    let mut children: Vec<TodoRef> = s
        .items
        .iter()
        .map(|it| &it.record)
        .filter(|r| r.group.as_ref().is_some_and(|g| g.id == record.id) && r.is_live())
        .map(TodoRecord::reference)
        .collect();
    if record.status == TodoStatus::StructuralHolder {
        for copy in &s.copies {
            let remote = &copy.record;
            if remote.group.as_ref().is_some_and(|g| g.id == record.id)
                && remote.is_live()
                && !children.iter().any(|c| c.id == remote.id)
            {
                children.push(remote.reference());
            }
        }
    }
    TodoView {
        record_status: super::_envelope::record_status(&record),
        record,
        local,
        reflection: None,
        issue: None,
        pull: None,
        prerequisites,
        expanded_prerequisites,
        children,
        refresh: GitHubRead::Read,
        copied_at: None,
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
    fn a_held_current_version_lacks_nothing_though_an_earlier_repeat_of_it_does() {
        let artifact = repeated("same", &[false, true]);
        assert!(Wanted::Current.lacking(&artifact).is_empty());
        assert_eq!(
            Wanted::Current.lacking(&repeated("same", &[true, false])),
            vec![digest_of("same").as_str().to_owned()]
        );
    }

    #[test]
    fn a_listing_copy_settles_no_read_of_content_it_does_not_hold() {
        let mut record = record("depth", 1, &[]);
        let copy = Held::Copy(record.clone());
        assert!(!copy.answers("log", &Wanted::Current));
        assert!(Held::Item(record.clone()).answers("log", &Wanted::Current));
        record.artifacts.push(repeated("same", &[false]));
        let unknown = Wanted::Digests(&["sha256:other"]);
        assert!(!Held::Copy(record.clone()).answers("log", &unknown));
        assert!(Held::Item(record).answers("log", &unknown));
    }

    #[test]
    fn a_digest_any_of_whose_versions_holds_its_content_is_answered_by_that_version() {
        let artifact = repeated("same", &[false, true]);
        let digest = digest_of("same");
        assert!(
            Wanted::Digests(&[digest.as_str()])
                .lacking(&artifact)
                .is_empty()
        );
        let taken = Wanted::version(&artifact, digest.as_str()).unwrap();
        assert_eq!(
            (taken.sequence, taken.content.as_deref()),
            (2, Some("same"))
        );
    }

    fn entry(record: TodoRecord, ready: bool) -> ListEntry {
        ListEntry {
            record,
            local: true,
            reflection: None,
            ready,
            last_known: None,
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
