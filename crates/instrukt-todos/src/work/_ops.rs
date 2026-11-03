//! The `instrukt todo` record operations (`todos`, `dependencies-and-holds`,
//! `stamps`, `approvals`, `github-record` "Artifacts and stamps"). Each
//! opens a session, validates the whole resulting record before it
//! commits, and flushes pending reflection.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use instrukt_core::git;
use instrukt_core::ids::{CommitSha, TodoSlug};
use instrukt_core::machine;
use instrukt_core::publication::EvalFingerprint;
use instrukt_runner::events::{Event, EventLog};
use instrukt_runner::interfaces::{AgentPreference, CatchUp, ProjectRef, TaskSchedule, TodoKind};

use super::_diff::{DiffLine, diff_lines};
use super::_error::WorkError;
use super::_model::{
    Artifact, Delivery, ReleasedHold, Removal, SCHEMA, TodoId, TodoRecord, TodoRef, Version,
    digest_of, valid_artifact_name,
};
use super::_session::{GitHubRead, PullFor, ReflectionStatus, Session};
use super::_stamps::{self, StampInputs, StampRequest};
use super::Work;
use crate::evidence::{
    Actor, ArtifactName, DependencyDigest, HoldRecord, HoldSource, PreparationReview,
    ReviewVerdict, StampFact, StampRecord, TodoStatus, Upstream, artifact_names, upstream_of,
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
    /// Whether the todo is a local item of this computer.
    pub local: bool,
    /// A local item's reflection; `None` for work read from GitHub.
    pub reflection: Option<ReflectionStatus>,
    pub issue: Option<u64>,
    pub pull: Option<u64>,
    pub prerequisites: Vec<PrerequisiteView>,
    /// A structural holder's children.
    pub children: Vec<TodoRef>,
}

/// One prerequisite with the status this computer knows of it.
#[derive(Debug, Clone)]
pub struct PrerequisiteView {
    pub slug: TodoSlug,
    pub status: Option<TodoStatus>,
}

/// `instrukt todo list`.
#[derive(Debug, Clone)]
pub struct Listing {
    pub entries: Vec<ListEntry>,
    pub github: GitHubRead,
}

#[derive(Debug, Clone)]
pub struct ListEntry {
    pub record: TodoRecord,
    pub local: bool,
    pub reflection: Option<ReflectionStatus>,
    /// Every prerequisite delivered.
    pub ready: bool,
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
        if !request.after.is_empty() || request.group.is_some() {
            // A relationship is judged against the current graph.
            s.pull_all()?;
        }
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
        })
    }

    pub fn show(&self, project: &ProjectRef, slug: &TodoSlug) -> Result<TodoView, WorkError> {
        let mut s = Session::open(self, project)?;
        if let Some(index) = s.find(slug, true) {
            match s.pull_through(index, PullFor::Read) {
                Ok(()) | Err(WorkError::Conflicted { .. }) => {}
                Err(err) => return Err(err),
            }
            let item = s.items[index].clone();
            let view = view(&mut s, item.record, true);
            return Ok(TodoView {
                reflection: Some(s.status(index)),
                issue: item.reflection.issue.map(|i| i.number),
                pull: item.reflection.pull,
                ..view
            });
        }
        let item = s.remote_item(slug)?.ok_or_else(|| unknown(&s, slug))?;
        let issue = item.reflection.issue.map(|i| i.number);
        let view = view(&mut s, item.record, false);
        Ok(TodoView { issue, ..view })
    }

    pub fn list(&self, project: &ProjectRef, selection: Selection) -> Result<Listing, WorkError> {
        let mut s = Session::open(self, project)?;
        s.pull_all()?;
        let wanted = match selection {
            Selection::Active => vec![TodoStatus::Active, TodoStatus::StructuralHolder],
            Selection::Iceboxed => vec![TodoStatus::Iceboxed],
            Selection::Removed => vec![TodoStatus::Removed],
            Selection::Delivered => vec![TodoStatus::Delivered],
        };
        let mut entries: Vec<ListEntry> = s
            .items
            .iter()
            .enumerate()
            .filter(|(_, it)| wanted.contains(&it.record.status))
            .map(|(index, it)| ListEntry {
                record: it.record.clone(),
                local: true,
                reflection: Some(s.status(index)),
                ready: false,
            })
            .collect();
        let local_ids: BTreeSet<TodoId> = s.items.iter().map(|it| it.record.id.clone()).collect();
        let remote = match selection {
            Selection::Active | Selection::Iceboxed => Some(s.remote_live()),
            Selection::Removed | Selection::Delivered => s.remote_all(),
        };
        for (record, _) in remote.unwrap_or_default() {
            if !local_ids.contains(&record.id) && wanted.contains(&record.status) {
                entries.push(ListEntry {
                    record,
                    local: false,
                    reflection: None,
                    ready: false,
                });
            }
        }
        let delivered: BTreeSet<TodoId> = s
            .items
            .iter()
            .map(|it| &it.record)
            .chain(entries.iter().map(|e| &e.record))
            .filter(|r| r.status == TodoStatus::Delivered)
            .map(|r| r.id.clone())
            .collect();
        for entry in &mut entries {
            entry.ready = entry.record.after.iter().all(|r| delivered.contains(&r.id));
        }
        Ok(Listing {
            entries: order(entries),
            github: s.github_read(),
        })
    }

    pub fn update(
        &self,
        project: &ProjectRef,
        slug: &TodoSlug,
        request: UpdateRequest,
    ) -> Result<Outcome<UpdateReport>, WorkError> {
        let mut s = Session::open(self, project)?;
        let index = s.enter(slug)?;
        let actor = s.actor()?;
        if request.after.is_some() || request.group.is_some() {
            // A relationship is judged against the current graph.
            s.pull_all()?;
        }
        let warnings = s.commit("updated", |s| apply_update(s, index, &request, &actor))?;
        s.flush()?;
        Ok(Outcome {
            value: UpdateReport {
                record: s.items[index].record.clone(),
                warnings,
            },
            reflection: s.status(index),
        })
    }

    pub fn refresh(
        &self,
        project: &ProjectRef,
        slug: &TodoSlug,
    ) -> Result<Outcome<TodoRecord>, WorkError> {
        let mut s = Session::open(self, project)?;
        let index = s.find(slug, false).ok_or_else(|| unknown(&s, slug))?;
        s.refresh(index)?;
        s.flush()?;
        Ok(Outcome {
            value: s.items[index].record.clone(),
            reflection: s.status(index),
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
        self.transition(project, slug, "removed", |record, now| {
            writable(record)?;
            record.removal = Some(Removal {
                at: now,
                from: record.status,
            });
            record.status = TodoStatus::Removed;
            record.claim = None;
            Ok(())
        })
    }

    /// Restores withdrawn work by identity or an unambiguous slug.
    pub fn reactivate(
        &self,
        project: &ProjectRef,
        key: &str,
    ) -> Result<Outcome<TodoRecord>, WorkError> {
        let mut s = Session::open(self, project)?;
        let removed: Vec<usize> = s
            .items
            .iter()
            .enumerate()
            .filter(|(_, it)| {
                it.record.status == TodoStatus::Removed
                    && (it.record.id.as_str() == key || it.record.slug.as_str() == key)
            })
            .map(|(i, _)| i)
            .collect();
        let index = match removed.as_slice() {
            [index] => *index,
            [] => {
                return Err(WorkError::refused(format!(
                    "no withdrawn todo `{key}` within its recovery window on this computer"
                )));
            }
            many => {
                let ids: Vec<String> = many
                    .iter()
                    .map(|i| s.items[*i].record.id.to_string())
                    .collect();
                return Err(WorkError::refused(format!(
                    "`{key}` names several withdrawn todos; restore one by identity: {}",
                    ids.join(", ")
                )));
            }
        };
        let slug = s.items[index].record.slug.clone();
        if s.live_ref(&slug).is_some() {
            return Err(WorkError::refused(format!(
                "a live todo is named `{slug}`; restoring would make the name ambiguous — rename one first"
            )));
        }
        s.pull_through(index, PullFor::Change)?;
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
        })
    }

    /// Records one stamp form. `--finalize-ready` appends
    /// `workflow.finalized` to `events` first, idempotently.
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
        let session = s.caller_session();
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
        let stamp = StampRecord {
            fact: resolved.fact,
            at: s.now,
            role: role_of(&record, &actor),
            actor: actor.clone(),
            model: None,
            eval_fingerprint: EvalFingerprint::current(self.home())?.map(|f| f.fingerprint),
            human_involved: None,
            sequence: 0,
        };
        if let StampFact::FinalizeReady { commit } = &stamp.fact {
            let log = events.ok_or_else(|| {
                WorkError::refused(
                    "the finalize-ready handoff appends `workflow.finalized` to the runner's event log, which this build does not provide",
                )
            })?;
            let event = Event::WorkflowFinalized {
                project: project.slug.clone(),
                todo: slug.clone(),
                revision: commit.clone(),
            };
            let appended = log.append(&event)?;
            tracing::info!(
                event = "work_finalize_handoff",
                project = %project.slug,
                todo = %slug,
                commit = %commit,
                appended = ?appended,
                "workflow.finalized handed to the event log"
            );
        }
        if resolved.already_recorded {
            // The runner stamped this run's start; nothing new to record.
            s.flush()?;
            return Ok(Outcome {
                value: stamp,
                reflection: s.status(index),
            });
        }
        let now = s.now;
        s.commit("stamped", |s| {
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
            record.stamps.push(stamp);
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
        let after = record.after.clone();
        let mut grounding = super::_grounding::ground(
            self.home(),
            &project.root,
            &project.slug,
            &s.base,
            slug,
            content,
        )?;
        for prerequisite in &after {
            let found = s.record_by_id(&prerequisite.id);
            for upstream in upstream_of(name) {
                grounding.dependencies.push(DependencyDigest {
                    todo: prerequisite.slug.clone(),
                    artifact: ArtifactName::new(*upstream),
                    digest: found
                        .as_ref()
                        .and_then(|r| r.current_digest(upstream).cloned()),
                });
            }
        }
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
        })
    }

    /// The current version of artifact `name`.
    pub fn read_artifact(
        &self,
        project: &ProjectRef,
        slug: &TodoSlug,
        name: &str,
    ) -> Result<Version, WorkError> {
        let view = self.show(project, slug)?;
        view.record
            .artifact(name)
            .and_then(|a| a.versions.last())
            .cloned()
            .ok_or_else(|| no_artifact(slug, name))
    }

    /// The line diff between two retained versions of artifact `name`.
    pub fn diff_artifact(
        &self,
        project: &ProjectRef,
        slug: &TodoSlug,
        name: &str,
        old: &str,
        new: &str,
    ) -> Result<Vec<DiffLine>, WorkError> {
        let view = self.show(project, slug)?;
        let artifact = view
            .record
            .artifact(name)
            .ok_or_else(|| no_artifact(slug, name))?;
        let content = |digest: &str| -> Result<String, WorkError> {
            let version = artifact
                .versions
                .iter()
                .find(|v| v.digest.as_str() == digest)
                .ok_or_else(|| {
                    WorkError::refused(format!(
                        "`{name}` of `{slug}` retains no version {digest}; `instrukt todo artifacts list {slug}` shows its versions"
                    ))
                })?;
            version.content.clone().ok_or_else(|| {
                WorkError::refused(format!(
                    "this computer does not hold the content of `{name}` version {digest}, and GitHub no longer serves it"
                ))
            })
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
        s.items[index].record.slug = new_slug.clone();
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
        let caller = s.caller_session();
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
                        &b.session == caller
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
    match holder.status {
        TodoStatus::StructuralHolder => {}
        TodoStatus::Active | TodoStatus::Iceboxed if !prepared(holder) => {
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
    let reference = holder.reference();
    s.items[index].record.group = Some(reference);
    Ok(())
}

/// Whether preparation completed: an approved plan review or task
/// approval is recorded.
fn prepared(record: &TodoRecord) -> bool {
    record.stamps.iter().any(|s| {
        matches!(
            s.fact,
            StampFact::PreparationVerdict {
                phase: PreparationReview::PlanReview,
                verdict: ReviewVerdict::Approve,
                ..
            } | StampFact::Approval {
                gate: crate::evidence::ApprovalGate::Task,
                ..
            }
        )
    })
}

/// The structural holders above `id`, nearest first.
fn ancestors(s: &mut Session<'_>, id: &TodoId) -> Vec<TodoId> {
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

/// What a todo waits on: its prerequisites, and for a structural holder
/// its children (a holder's milestone is every child's).
fn waits_on(s: &mut Session<'_>, id: &TodoId) -> Vec<TodoRef> {
    let Some(record) = s.record_by_id(id) else {
        return Vec::new();
    };
    let mut next = record.after.clone();
    if record.status == TodoStatus::StructuralHolder {
        let mut children: Vec<TodoRef> = s
            .items
            .iter()
            .map(|it| &it.record)
            .filter(|r| r.group.as_ref().is_some_and(|g| &g.id == id))
            .map(TodoRecord::reference)
            .collect();
        for (remote, _) in s.remote_live() {
            if remote.group.as_ref().is_some_and(|g| &g.id == id)
                && !children.iter().any(|c| c.id == remote.id)
            {
                children.push(remote.reference());
            }
        }
        next.extend(children);
    }
    next
}

/// A cycle of waiting through `record`, if the project graph has one: the
/// slugs from `record` back to itself. A holder waits on its children, so
/// reaching one of `record`'s holders closes the cycle through its group.
fn cycle_through(s: &mut Session<'_>, record: &TodoRecord) -> Option<Vec<TodoSlug>> {
    let above = ancestors(s, &record.id);
    let mut stack: Vec<(TodoRef, Vec<TodoSlug>)> = waits_on(s, &record.id)
        .into_iter()
        .map(|next| {
            let path = vec![record.slug.clone(), next.slug.clone()];
            (next, path)
        })
        .collect();
    let mut seen: BTreeSet<TodoId> = BTreeSet::new();
    while let Some((current, path)) = stack.pop() {
        if current.id == record.id {
            return Some(path);
        }
        if above.contains(&current.id) {
            // The holder waits on its children down to `record`.
            let mut path = path;
            path.push(record.slug.clone());
            return Some(path);
        }
        if !seen.insert(current.id.clone()) {
            continue;
        }
        for next in waits_on(s, &current.id) {
            let mut longer = path.clone();
            longer.push(next.slug.clone());
            stack.push((next, longer));
        }
    }
    None
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
        .find(|b| &b.session == session)
        .map(|b| b.role.clone())
}

fn unknown(s: &Session<'_>, slug: &TodoSlug) -> WorkError {
    WorkError::UnknownTodo {
        project: s.project.slug.clone(),
        todo: slug.clone(),
    }
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
    let mut children: Vec<TodoRef> = s
        .items
        .iter()
        .map(|it| &it.record)
        .filter(|r| r.group.as_ref().is_some_and(|g| g.id == record.id) && r.is_live())
        .map(TodoRecord::reference)
        .collect();
    if record.status == TodoStatus::StructuralHolder {
        for (remote, _) in s.remote_live() {
            if remote.group.as_ref().is_some_and(|g| g.id == record.id)
                && !children.iter().any(|c| c.id == remote.id)
            {
                children.push(remote.reference());
            }
        }
    }
    TodoView {
        record,
        local,
        reflection: None,
        issue: None,
        pull: None,
        prerequisites,
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

    fn entry(record: TodoRecord, ready: bool) -> ListEntry {
        ListEntry {
            record,
            local: true,
            reflection: None,
            ready,
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
        });
        assert!(matches!(
            change_kind(&mut started, TodoKind::Bug, true),
            Err(WorkError::Refused(_))
        ));
    }
}
