//! Work's implementations of the interfaces other contexts declare: the
//! runner's work interfaces, workflow's evidence read and core's todo
//! existence.

use instrukt_core::ids::{ProjectSlug, RoleName, SessionRef, TodoSlug};
use instrukt_core::interfaces::{InterfaceError, TodoExistence};
use instrukt_runner::interfaces::{
    AutomaticHolds, ClaimOutcome, Claimant, Claims, Firing, GitHubReach, HoldCause, Pickup,
    PickupCandidate, ProjectRef, Reflection, ReflectionReport, RetentionPurge, SessionBinding,
    SessionBindings, TaskFirings, TaskOutcome, TaskTiming, TodoKind, WorkInterfaceError,
};
use jiff::Timestamp;

use super::_error::WorkError;
use super::_model::{Binding, Claim, TodoId, TodoRecord};
use super::_session::{GitHubRead, PendingReason, PullFor, ReflectionStatus, Session};
use super::Work;
use crate::evidence::{
    Actor, ArtifactHistory, ArtifactVersion, ClaimRecord, HoldRecord, HoldSource, Prerequisite,
    RoleBinding, StampFact, StampRecord, TaskResult, TodoEvidence, TodoStatus, WorkEvidence,
    artifact_names,
};

/// The actor automatic holds and runner claims are attributed to: the
/// runner acts outside any agent session.
const RUNNER: &str = "instrukt runner";

impl Pickup for Work {
    fn pickup_candidates(
        &self,
        project: &ProjectRef,
        kinds: &[TodoKind],
    ) -> Result<Vec<PickupCandidate>, WorkInterfaceError> {
        let mut s = Session::open(self, project)?;
        s.pull_all()?;
        let machine = s.machine()?;
        // The runner's own claims on this computer that no session took up
        // yet are offered again (ruling D): one still awaiting reflection is
        // reflected on the next claim, and one another command already
        // reflected is dispatched rather than stranded.
        let pending_here: Vec<TodoId> = s
            .items
            .iter()
            .filter(|it| {
                it.record.bindings.is_empty()
                    && it
                        .record
                        .claim
                        .as_ref()
                        .is_some_and(|c| c.machine == machine && c.session.is_none())
            })
            .map(|it| it.record.id.clone())
            .collect();
        let mut records: Vec<TodoRecord> = s
            .items
            .iter()
            .filter(|it| !it.reflection.conflicted)
            .map(|it| it.record.clone())
            .collect();
        for (record, _) in s.remote_live() {
            if !records.iter().any(|r| r.id == record.id) {
                records.push(record);
            }
        }
        records.retain(|r| {
            r.status == TodoStatus::Active
                && r.hold.is_none()
                && (r.claim.is_none() || pending_here.contains(&r.id))
                && kinds.contains(&r.kind)
        });
        records.sort_by_key(|r| r.created);
        let mut ordered: Vec<TodoRecord> = Vec::with_capacity(records.len());
        while !records.is_empty() {
            let next = records
                .iter()
                .position(|r| {
                    r.after
                        .iter()
                        .all(|p| !records.iter().any(|other| other.id == p.id))
                })
                .unwrap_or(0);
            ordered.push(records.remove(next));
        }
        Ok(ordered
            .into_iter()
            .map(|record| PickupCandidate {
                claimed_here: pending_here.contains(&record.id),
                timing: timing(&record),
                agent: record.agent.clone(),
                kind: record.kind,
                todo: record.slug,
            })
            .collect())
    }
}

impl Claims for Work {
    fn record_claim(
        &self,
        project: &ProjectRef,
        todo: &TodoSlug,
        claimant: &Claimant,
    ) -> Result<ClaimOutcome, WorkInterfaceError> {
        let mut s = Session::open(self, project)?;
        let index = match s.enter(todo) {
            Ok(index) => index,
            Err(WorkError::Conflicted { .. }) => return Ok(ClaimOutcome::ClaimedElsewhere),
            Err(err) => return Err(err.into()),
        };
        let machine = s.machine()?;
        let session = match claimant {
            Claimant::Runner => None,
            Claimant::Session(session) => Some(session.clone()),
        };
        let record = &s.items[index].record;
        if let Some(claim) = &record.claim {
            if claim.machine != machine {
                return Ok(ClaimOutcome::ClaimedElsewhere);
            }
            let holds = match &session {
                None => claim.session.is_none(),
                Some(session) => {
                    claim.session.as_ref() == Some(session)
                        || record.bindings.iter().any(|b| &b.session == session)
                }
            };
            if !holds {
                return Ok(ClaimOutcome::ClaimedElsewhere);
            }
            if !s.items[index].reflection.pending {
                return Ok(ClaimOutcome::AlreadyHeld);
            }
            // A claim recorded while GitHub was unreachable: reflect it now.
            s.flush()?;
            return Ok(match claim_outcome(s.status(index)) {
                ClaimOutcome::Claimed => ClaimOutcome::AlreadyHeld,
                other => other,
            });
        }
        if record.status != TodoStatus::Active {
            return Err(WorkError::refused(format!(
                "only active work is claimed; `{todo}` is not active"
            ))
            .into());
        }
        if session.is_none() && record.hold.is_some() {
            return Err(WorkError::refused(format!(
                "`{todo}` is on hold; automatic dispatch does not claim it"
            ))
            .into());
        }
        let now = s.now;
        s.commit("claimed", |s| {
            s.items[index].record.claim = Some(Claim {
                machine,
                session,
                at: now,
            });
            Ok(())
        })?;
        s.flush()?;
        Ok(claim_outcome(s.status(index)))
    }

    fn release_claim(
        &self,
        project: &ProjectRef,
        todo: &TodoSlug,
        claimant: &Claimant,
    ) -> Result<(), WorkInterfaceError> {
        let mut s = Session::open(self, project)?;
        let index = s.enter(todo)?;
        let machine = s.machine()?;
        let record = &s.items[index].record;
        let Some(claim) = &record.claim else {
            return Ok(());
        };
        // The holders `work-dispatch` names: on the claiming computer, the
        // runner for its own claim, or the session that claimed or is bound
        // to the todo (as `todo update --unclaim`).
        let holds = claim.machine == machine
            && match claimant {
                Claimant::Runner => claim.session.is_none(),
                Claimant::Session(session) => {
                    claim.session.as_ref() == Some(session)
                        || record.bindings.iter().any(|b| &b.session == session)
                }
            };
        if !holds {
            return Err(WorkError::refused(format!(
                "`{todo}` is claimed elsewhere; only its holder releases the claim"
            ))
            .into());
        }
        s.commit("released", |s| {
            s.items[index].record.claim = None;
            Ok(())
        })?;
        s.flush()?;
        Ok(())
    }
}

fn claim_outcome(status: ReflectionStatus) -> ClaimOutcome {
    match status {
        ReflectionStatus::Reflected => ClaimOutcome::Claimed,
        ReflectionStatus::Conflicted => ClaimOutcome::ClaimedElsewhere,
        ReflectionStatus::Pending(_) => ClaimOutcome::NotReflected,
    }
}

impl SessionBindings for Work {
    fn session_binding(
        &self,
        project: &ProjectRef,
        todo: &TodoSlug,
        role: &RoleName,
    ) -> Result<Option<SessionBinding>, WorkInterfaceError> {
        let evidence =
            self.evidence(project, todo)?
                .ok_or_else(|| WorkInterfaceError::UnknownTodo {
                    project: project.slug.clone(),
                    todo: todo.clone(),
                })?;
        Ok(evidence
            .bindings
            .into_iter()
            .find(|b| &b.role == role)
            .map(|b| b.binding))
    }

    fn bind_session(
        &self,
        project: &ProjectRef,
        todo: &TodoSlug,
        role: &RoleName,
        binding: &SessionBinding,
    ) -> Result<(), WorkInterfaceError> {
        let mut s = Session::open(self, project)?;
        let index = s.enter(todo)?;
        s.commit("session bound", |s| {
            let record = &mut s.items[index].record;
            record.bindings.retain(|b| &b.role != role);
            record.bindings.push(Binding {
                role: role.clone(),
                session: binding.session.clone(),
                worktree: binding.worktree.clone(),
            });
            Ok(())
        })?;
        s.flush()?;
        Ok(())
    }
}

impl Reflection for Work {
    fn sync_reflection(
        &self,
        project: &ProjectRef,
    ) -> Result<ReflectionReport, WorkInterfaceError> {
        let mut s = Session::open(self, project)?;
        s.pull_all()?;
        s.flush()?;
        let reach = match s.github_read() {
            GitHubRead::Read => GitHubReach::Reachable,
            GitHubRead::NotRead(PendingReason::RateLimited { until }) => {
                GitHubReach::RateLimited { until }
            }
            GitHubRead::NotRead(_) => GitHubReach::Unreachable,
        };
        let mut pending = Vec::new();
        let mut conflicted = Vec::new();
        for (index, item) in s.items.iter().enumerate() {
            match s.status(index) {
                ReflectionStatus::Pending(_) => pending.push(item.record.slug.clone()),
                ReflectionStatus::Conflicted => conflicted.push(item.record.slug.clone()),
                ReflectionStatus::Reflected => {}
            }
        }
        Ok(ReflectionReport {
            reach,
            pending,
            conflicted,
        })
    }
}

impl TaskFirings for Work {
    fn record_firing_started(
        &self,
        project: &ProjectRef,
        todo: &TodoSlug,
        occurrence: Timestamp,
        session: &SessionRef,
    ) -> Result<(), WorkInterfaceError> {
        let mut s = Session::open(self, project)?;
        let index = s.enter(todo)?;
        if s.items[index].record.kind != TodoKind::Task {
            return Err(WorkError::refused(format!("`{todo}` is not a task")).into());
        }
        let now = s.now;
        let actor = Actor::Session(session.clone());
        let fingerprint = instrukt_core::publication::EvalFingerprint::current(self.home())
            .map_err(WorkError::from)?
            .map(|f| f.fingerprint);
        let (model, model_provenance) = s.model_of(&actor);
        s.commit("task firing started", |s| {
            let record = &mut s.items[index].record;
            let role = record
                .bindings
                .iter()
                .find(|b| &b.session == session)
                .map(|b| b.role.clone());
            let sequence = record.next_sequence();
            record.stamps.push(StampRecord {
                fact: StampFact::TaskFiringStarted { occurrence },
                at: now,
                actor,
                model,
                eval_fingerprint: fingerprint,
                role,
                human_involved: None,
                sequence,
                model_provenance,
            });
            Ok(())
        })?;
        s.flush()?;
        Ok(())
    }
}

impl AutomaticHolds for Work {
    fn set_automatic_hold(
        &self,
        project: &ProjectRef,
        todo: &TodoSlug,
        cause: &HoldCause,
    ) -> Result<(), WorkInterfaceError> {
        let mut s = Session::open(self, project)?;
        let index = s.enter(todo)?;
        if s.items[index].record.hold.is_some() {
            tracing::debug!(
                event = "work_hold_kept",
                todo = %todo,
                "the todo is already on hold; the automatic hold adds nothing"
            );
            return Ok(());
        }
        if s.items[index].record.status == TodoStatus::StructuralHolder {
            return Err(WorkError::refused(format!(
                "`{todo}` is a structural holder; it carries no dispatch hold"
            ))
            .into());
        }
        let reason = match cause {
            HoldCause::TaskFailed { reason } => format!("task failed: {reason}"),
            HoldCause::Anomaly { reason } => format!("anomaly: {reason}"),
        };
        let now = s.now;
        s.commit("automatic hold", |s| {
            s.items[index].record.hold = Some(HoldRecord {
                source: HoldSource::Automatic,
                reason,
                by: Actor::Person {
                    name: RUNNER.to_owned(),
                },
                at: now,
            });
            Ok(())
        })?;
        s.flush()?;
        Ok(())
    }
}

impl RetentionPurge for Work {
    fn purge_expired(&self, project: &ProjectRef) -> Result<Vec<TodoSlug>, WorkInterfaceError> {
        let s = Session::open(self, project)?;
        Ok(s.purged.clone())
    }
}

impl WorkEvidence for Work {
    fn evidence(
        &self,
        project: &ProjectRef,
        todo: &TodoSlug,
    ) -> Result<Option<TodoEvidence>, WorkInterfaceError> {
        let mut s = Session::open(self, project)?;
        let record = match s.find(todo, true) {
            Some(index) => {
                s.pull_through(index, PullFor::Read)?;
                s.items[index].record.clone()
            }
            None => match s.remote_item(todo)? {
                Some(item) => item.record,
                None => return Ok(None),
            },
        };
        let prerequisites = expand(&mut s, &record);
        Ok(Some(to_evidence(record, prerequisites)))
    }

    fn evidence_by_id(
        &self,
        project: &ProjectRef,
        id: &TodoId,
    ) -> Result<Option<TodoEvidence>, WorkInterfaceError> {
        let mut s = Session::open(self, project)?;
        let record = match s.find_id(id) {
            Some(index) => {
                s.pull_through(index, PullFor::Read)?;
                s.items[index].record.clone()
            }
            None => match s.remote_matching(&format!("todo {id}"), |r| &r.id == id)? {
                Some(item) => item.record,
                None => return Ok(None),
            },
        };
        let prerequisites = expand(&mut s, &record);
        Ok(Some(to_evidence(record, prerequisites)))
    }
}

/// Core's todo-existence question for one project, answered by work.
pub struct WorkExistence {
    work: std::sync::Arc<Work>,
    project: ProjectRef,
}

impl WorkExistence {
    pub fn new(work: std::sync::Arc<Work>, project: ProjectRef) -> Self {
        Self { work, project }
    }
}

impl TodoExistence for WorkExistence {
    fn todo_exists(&self, project: &ProjectSlug, slug: &TodoSlug) -> Result<bool, InterfaceError> {
        let failed = |message: String| InterfaceError {
            module: "todos",
            message,
        };
        if project != &self.project.slug {
            return Err(failed(format!(
                "work answers for {} here, not for {project}",
                self.project.slug
            )));
        }
        let mut s = Session::open(&self.work, &self.project).map_err(|e| failed(e.to_string()))?;
        if s.find(slug, true)
            .is_some_and(|i| s.items[i].record.status != TodoStatus::Removed)
        {
            return Ok(true);
        }
        let found = s.live_ref(slug).is_some();
        match s.github_read() {
            GitHubRead::Read => Ok(found),
            GitHubRead::NotRead(reason) => Err(failed(format!(
                "cannot tell whether todo `{slug}` exists: it is not on this computer and GitHub cannot be read ({})",
                super::_session::describe(&reason)
            ))),
        }
    }
}

/// The record's prerequisites with structural holders expanded to the
/// todos whose milestones satisfy them.
fn expand(s: &mut Session<'_>, record: &TodoRecord) -> Vec<Prerequisite> {
    let mut out = Vec::new();
    let mut seen = Vec::new();
    let mut stack: Vec<_> = record.after.iter().rev().cloned().collect();
    while let Some(reference) = stack.pop() {
        if seen.contains(&reference.id) {
            continue;
        }
        seen.push(reference.id.clone());
        let Some(prerequisite) = s.record_by_id(&reference.id) else {
            out.push(Prerequisite::Todo(reference.slug));
            continue;
        };
        if prerequisite.status != TodoStatus::StructuralHolder {
            out.push(Prerequisite::Todo(prerequisite.slug));
            continue;
        }
        let mut children: Vec<TodoRecord> = s
            .items
            .iter()
            .map(|it| it.record.clone())
            .filter(|r| r.group.as_ref().is_some_and(|g| g.id == prerequisite.id))
            .collect();
        for (remote, _) in s.remote_live() {
            if remote
                .group
                .as_ref()
                .is_some_and(|g| g.id == prerequisite.id)
                && !children.iter().any(|c| c.id == remote.id)
            {
                children.push(remote);
            }
        }
        if children.is_empty() {
            out.push(Prerequisite::EmptyHolder(prerequisite.slug));
        }
        stack.extend(children.into_iter().rev().map(|c| c.reference()));
    }
    out
}

fn to_evidence(record: TodoRecord, prerequisites: Vec<Prerequisite>) -> TodoEvidence {
    let implementation_plan_seeded = seeded(&record);
    TodoEvidence {
        timing: timing(&record),
        id: record.id,
        project: record.project,
        slug: record.slug,
        kind: record.kind,
        status: record.status,
        artifacts: record
            .artifacts
            .into_iter()
            .map(|a| ArtifactHistory {
                name: a.name,
                versions: a
                    .versions
                    .into_iter()
                    .map(|v| ArtifactVersion {
                        digest: v.digest,
                        writer: v.writer,
                        created: v.created,
                        derived_from: v.derived_from,
                        sequence: v.sequence,
                        grounding: v.grounding,
                    })
                    .collect(),
            })
            .collect(),
        stamps: record.stamps,
        claim: record.claim.map(|c| ClaimRecord {
            session: c.session,
            at: c.at,
        }),
        bindings: record
            .bindings
            .into_iter()
            .map(|b| RoleBinding {
                role: b.role,
                binding: SessionBinding {
                    session: b.session,
                    worktree: b.worktree,
                },
            })
            .collect(),
        hold: record.hold,
        prerequisites,
        delivery: record.delivery.map(|d| d.commit),
        agent: record.agent,
        implementation_plan_seeded,
    }
}

/// Whether the implementation plan was seeded: its first version was
/// written by the todo's filer, before any plan review (`preparation`,
/// "seeded derivation": a plan present at first entry with no prior
/// review). No spec names how a seed is recorded; this reads it from the
/// record rather than adding a surface.
fn seeded(record: &TodoRecord) -> bool {
    let Some(first) = record
        .artifact(artifact_names::IMPLEMENTATION_PLAN)
        .and_then(|a| a.versions.first())
    else {
        return false;
    };
    let reviewed_before = record.stamps.iter().any(|s| {
        s.at <= first.created
            && matches!(
                s.fact,
                StampFact::PreparationVerdict {
                    phase: crate::evidence::PreparationReview::PlanReview,
                    ..
                }
            )
    });
    first.writer == record.created_by && !reviewed_before
}

/// A task's schedule and its last firing.
fn timing(record: &TodoRecord) -> Option<TaskTiming> {
    let schedule = record.schedule.clone()?;
    let last_fired = record
        .stamps
        .iter()
        .rev()
        .find_map(|stamp| match &stamp.fact {
            StampFact::TaskFiringStarted { occurrence } => Some(Firing {
                occurrence: *occurrence,
                started: stamp.at,
                outcome: record.stamps.iter().rev().find_map(|s| match &s.fact {
                    StampFact::TaskOutcome {
                        occurrence: o,
                        result,
                        reason,
                    } if o == occurrence => Some(match result {
                        TaskResult::Success => TaskOutcome::Success,
                        TaskResult::Error => TaskOutcome::Error {
                            reason: reason.clone(),
                        },
                    }),
                    _ => None,
                }),
            }),
            _ => None,
        });
    Some(TaskTiming {
        schedule,
        catch_up: record.catch_up,
        last_fired,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evidence::{ArtifactName, PreparationReview, ReviewVerdict};
    use crate::work::_model::{Artifact, TodoId, Version, digest_of};
    use instrukt_core::ids::ProjectSlug;
    use instrukt_runner::interfaces::CatchUp;

    fn filer() -> Actor {
        Actor::Person {
            name: "filer".into(),
        }
    }

    fn at(second: i64) -> Timestamp {
        Timestamp::from_second(1_800_000_000 + second).unwrap()
    }

    fn with_plan(writer: Actor, written: i64) -> TodoRecord {
        TodoRecord {
            schema: 1,
            id: TodoId::parse(&"ef".repeat(16)).unwrap(),
            project: ProjectSlug::parse("demo").unwrap(),
            slug: TodoSlug::parse("depth").unwrap(),
            kind: TodoKind::Change,
            status: TodoStatus::Active,
            created: at(0),
            created_by: filer(),
            human_involved: None,
            revision: 1,
            sequence: 0,
            artifacts: vec![Artifact {
                name: ArtifactName::new(artifact_names::IMPLEMENTATION_PLAN),
                address: None,
                versions: vec![Version {
                    digest: digest_of("plan"),
                    writer,
                    created: at(written),
                    derived_from: Vec::new(),
                    sequence: 0,
                    grounding: Default::default(),
                    remote: None,
                    content: None,
                }],
            }],
            stamps: Vec::new(),
            claim: None,
            bindings: Vec::new(),
            hold: None,
            hold_history: Vec::new(),
            after: Vec::new(),
            group: None,
            schedule: None,
            catch_up: CatchUp::Once,
            agent: None,
            branch: None,
            delivery: None,
            removal: None,
        }
    }

    #[test]
    fn a_plan_the_filer_supplied_before_any_review_is_seeded() {
        assert!(seeded(&with_plan(filer(), 1)));
    }

    #[test]
    fn a_plan_drafted_by_preparation_is_not_seeded() {
        let drafter = Actor::Person {
            name: "planner".into(),
        };
        assert!(!seeded(&with_plan(drafter, 1)));
    }

    #[test]
    fn a_plan_first_written_after_a_plan_review_is_not_seeded() {
        let mut record = with_plan(filer(), 5);
        record.stamps.push(StampRecord {
            fact: StampFact::PreparationVerdict {
                phase: PreparationReview::PlanReview,
                verdict: ReviewVerdict::NeedsWork,
                target: digest_of("older"),
                report: digest_of("report"),
            },
            at: at(2),
            actor: filer(),
            model: None,
            eval_fingerprint: None,
            role: None,
            human_involved: None,
            sequence: 0,
            model_provenance: Default::default(),
        });
        assert!(!seeded(&record));
    }
}
