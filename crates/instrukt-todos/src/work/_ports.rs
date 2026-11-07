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
use super::_session::{GitHubRead, PendingReason, ReflectionStatus, Session};
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
        // The tick's one pickup listing of the project: its result replaces
        // the listing copies (`github-record`, "Interest-driven refresh").
        let listed = s.pickup_listing();
        let machine = s.machine()?;
        // The runner's own claims on this computer that no session took up
        // yet are offered again (ruling D): one still awaiting reflection is
        // reflected on the next claim, and one another command already
        // reflected is dispatched rather than stranded.
        let pending_here: Vec<TodoId> = s
            .items
            .iter()
            .filter(|it| {
                it.record
                    .claim
                    .as_ref()
                    .is_some_and(|c| c.machine == machine && c.session.is_none())
            })
            .map(|it| it.record.id.clone())
            .collect();
        // A local item is judged as the listing read it when GitHub holds
        // a newer revision than this computer last read and nothing local is
        // pending: another computer may have claimed or changed it. Claiming
        // refreshes it before the claim is committed.
        let mut records: Vec<TodoRecord> = s
            .items
            .iter()
            .filter(|it| !it.reflection.conflicted)
            .map(|it| {
                let newer = listed.iter().find(|(remote, _)| {
                    remote.id == it.record.id
                        && !it.reflection.pending
                        && Some(remote.revision) > it.reflection.remote_revision
                });
                newer.map_or_else(|| it.record.clone(), |(remote, _)| remote.clone())
            })
            .collect();
        for copy in &s.copies {
            if !records.iter().any(|r| r.id == copy.record.id) {
                records.push(copy.record.clone());
            }
        }
        records.retain(|r| {
            r.status == TodoStatus::Active
                && r.hold.is_none()
                && (r.claim.is_none() || pending_here.contains(&r.id))
                && kinds.contains(&r.kind)
        });
        records.retain(|record| {
            let handed_off = record.claim.is_none()
                && record.bindings.iter().rev().find(|binding|binding.attempt.assignment==instrukt_runner::interfaces::AssignmentKind::Driver && binding.attempt.lane==Some(instrukt_runner::interfaces::Lane::Prepare)).is_some_and(|binding|binding.attempt.released_at.is_some())
                && record.stamps.iter().rev().find(|stamp|matches!(stamp.fact,StampFact::PreparationHeadReviewed {..})).is_some_and(|stamp|matches!(&stamp.fact,StampFact::PreparationHeadReviewed {verdict:crate::evidence::ReviewVerdict::Approve,report,..} if record.current_digest("preparation-review-findings")==Some(report)));
            if handed_off && let Err(error)=super::_attempts::preparation_integrated(record,&project.root) {
                tracing::info!(event="preparation_handoff_pending",todo=%record.slug,reason=%error);
                return false;
            }
            true
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
                        || record
                            .bindings
                            .iter()
                            .any(|b| b.session.as_ref() == Some(session))
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
                        || record
                            .bindings
                            .iter()
                            .any(|b| b.session.as_ref() == Some(session))
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

/// The tick's polling of the todos this computer has an interest in,
/// beyond the pickup listing (`github-record`, "Interest-driven refresh"):
/// the todos it holds claims on, so what a human does on GitHub reaches the
/// worker holding the claim, and the prerequisites that claimed work is
/// blocked on. Local items are pulled with conditional requests; a
/// prerequisite that is not a local item refreshes its listing copy.
fn poll_interest(s: &mut Session<'_>) -> Result<(), WorkError> {
    let machine = s.machine()?;
    let claimed: Vec<usize> = (0..s.items.len())
        .filter(|&index| {
            s.items[index]
                .record
                .claim
                .as_ref()
                .is_some_and(|c| c.machine == machine)
        })
        .collect();
    let mut blocking: Vec<TodoId> = Vec::new();
    for &index in &claimed {
        match s.pull_through(index) {
            Ok(()) | Err(WorkError::Conflicted { .. }) => {}
            Err(err) => return Err(err),
        }
        for prerequisite in &s.items[index].record.after {
            if !blocking.contains(&prerequisite.id) {
                blocking.push(prerequisite.id.clone());
            }
        }
    }
    let known = s.known_records();
    blocking.retain(|id| {
        known
            .iter()
            .find(|r| &r.id == id)
            .is_none_or(|r| r.status != TodoStatus::Delivered)
    });
    for id in blocking {
        if let Some(index) = s.find_id(&id) {
            if claimed.contains(&index) {
                continue;
            }
            match s.pull_through(index) {
                Ok(()) | Err(WorkError::Conflicted { .. }) => {}
                Err(err) => return Err(err),
            }
        } else if let Some(copy) = s.copies.iter().find(|c| c.record.id == id).cloned()
            && s.refreshed()
        {
            match s.refresh_copy(copy.issue_number) {
                Ok(_) => {}
                Err(WorkError::GitHub(err)) => tracing::warn!(
                    event = "work_prerequisite_unread",
                    todo = %copy.record.slug,
                    error = %err,
                    "a prerequisite of claimed work could not be refreshed"
                ),
                Err(err) => return Err(err),
            }
        }
    }
    Ok(())
}

fn claim_outcome(status: ReflectionStatus) -> ClaimOutcome {
    match status {
        ReflectionStatus::Reflected => ClaimOutcome::Claimed,
        ReflectionStatus::Conflicted => ClaimOutcome::ClaimedElsewhere,
        ReflectionStatus::Pending(_) => ClaimOutcome::NotReflected,
    }
}

impl SessionBindings for Work {
    fn release_unstarted(
        &self,
        project: &ProjectRef,
        todo: &TodoSlug,
        key: &str,
    ) -> Result<(), WorkInterfaceError> {
        let mut s = Session::open(self, project)?;
        let index = s.enter(todo)?;
        let machine = s.machine()?;
        if !s.items[index]
            .record
            .claim
            .as_ref()
            .is_some_and(|claim| claim.machine == machine && claim.session.is_none())
        {
            return Err(WorkError::refused(
                "unstarted release needs this runner's unassigned claim",
            )
            .into());
        }
        let state = instrukt_core::state_db::StateRead::open(s.home())
            .map_err(WorkError::from)?
            .ok_or_else(|| WorkError::refused("missing reservation database"))?;
        let attempted:bool=state.conn().query_row("SELECT EXISTS(SELECT 1 FROM runtime_starts WHERE semantic_key=?1) OR EXISTS(SELECT 1 FROM runtime_bindings WHERE semantic_key=?1)",[key],|row|row.get(0)).map_err(instrukt_core::state_db::StateDbError::sqlite(state.path())).map_err(WorkError::from)?;
        if attempted {
            return Err(WorkError::refused(
                "runtime start evidence exists; preserve this assignment for reconciliation",
            )
            .into());
        }
        let now = s.now;
        s.commit("selection exhausted before start", |s| {
            let binding = s.items[index]
                .record
                .bindings
                .iter_mut()
                .find(|binding| {
                    binding.attempt.semantic_key == key
                        && binding.session.is_none()
                        && binding.attempt.released_at.is_none()
                })
                .ok_or_else(|| WorkError::refused("no fresh unbound reservation"))?;
            binding.attempt.released_at = Some(now);
            binding.attempt.no_start_reason =
                Some("No eligible configured model; runtime creation was not attempted.".into());
            Ok(())
        })?;
        s.flush()?;
        Ok(())
    }

    fn reserve_dispatch(
        &self,
        project: &ProjectRef,
        todo: &TodoSlug,
        role: &RoleName,
        occurrence: Option<Timestamp>,
        lane: &instrukt_runner::interfaces::Lane,
    ) -> Result<instrukt_runner::interfaces::DispatchReservation, WorkInterfaceError> {
        use instrukt_runner::interfaces::{AssignmentKind, DispatchReservation};
        use instrukt_runner::runtime::CreationPermit;
        let mut s = Session::open(self, project)?;
        let index = s.enter(todo)?;
        let machine = s.machine()?;
        let record = &s.items[index].record;
        if record.status != TodoStatus::Active
            || record.hold.is_some()
            || !record
                .claim
                .as_ref()
                .is_some_and(|c| c.machine == machine && c.session.is_none())
        {
            return Err(WorkError::refused(
                "dispatch needs this computer's reflected, unassigned execution claim",
            )
            .into());
        }
        if occurrence.is_none()
            && let Some(retained) = record
                .bindings
                .iter()
                .rev()
                .find(|b| {
                    b.role == *role
                        && b.attempt.assignment == AssignmentKind::Driver
                        && b.attempt.lane == Some(*lane)
                        && b.session.is_some()
                        && b.worktree.as_ref().is_some_and(|tree| tree.is_dir())
                        && b.attempt.failed_at.is_none()
                })
                .cloned()
        {
            if retained.attempt.runtime != s.config.runner_runtime().map_err(WorkError::from)? {
                return Err(WorkError::refused("retained phase belongs to another runtime").into());
            }
            if retained.attempt.released_at.is_some() {
                s.commit("resume retained driver", |s| {
                    let binding = s.items[index]
                        .record
                        .bindings
                        .iter_mut()
                        .find(|b| b.attempt.semantic_key == retained.attempt.semantic_key)
                        .ok_or_else(|| WorkError::refused("retained phase disappeared"))?;
                    binding.attempt.released_at = None;
                    Ok(())
                })?;
                s.flush()?;
                if s.items[index].reflection.pending || s.items[index].reflection.conflicted {
                    return Err(WorkError::refused("resumed assignment awaits reflection").into());
                }
            }
            return Ok(DispatchReservation::Existing(SessionBinding {
                key: retained.attempt.semantic_key,
                session: retained
                    .session
                    .ok_or_else(|| WorkError::refused("retained phase has no native session"))?,
                worktree: retained.worktree,
            }));
        }
        let placement =
            (|| -> Result<_, WorkError> {
                if *lane == instrukt_runner::interfaces::Lane::TaskRun {
                    return Ok(None);
                }
                let branch = format!(
                    "{}/{}",
                    record.slug,
                    if *lane == instrukt_runner::interfaces::Lane::Prepare {
                        "prepare"
                    } else {
                        "work"
                    }
                );
                if let Some(pending) = record
                    .bindings
                    .iter()
                    .rev()
                    .find(|b| b.role == *role && b.attempt.released_at.is_none())
                {
                    return Ok(Some(instrukt_runner::runtime::WorktreeRequest {
                        branch: pending.attempt.branch.clone().ok_or_else(|| {
                            WorkError::refused("retained phase branch is missing")
                        })?,
                        from: pending
                            .attempt
                            .start_from
                            .clone()
                            .ok_or_else(|| WorkError::refused("retained start base is missing"))?,
                        base: pending
                            .attempt
                            .base_head
                            .as_ref()
                            .ok_or_else(|| WorkError::refused("retained starting head is missing"))?
                            .to_string(),
                    }));
                }
                if instrukt_core::git::succeeds(
                    &project.root,
                    [
                        "show-ref",
                        "--verify",
                        "--quiet",
                        &format!("refs/heads/{branch}"),
                    ],
                )? {
                    return Err(WorkError::refused(format!(
                        "phase branch `{branch}` already exists; no replacement worktree is created"
                    )));
                }
                let (from, base) = super::_attempts::phase_base(
                    &project.root,
                    &branch,
                    *lane == instrukt_runner::interfaces::Lane::Prepare,
                )?;
                if *lane == instrukt_runner::interfaces::Lane::Work {
                    super::_attempts::preparation_integrated(record, &project.root)?;
                }
                Ok(Some(instrukt_runner::runtime::WorktreeRequest {
                    branch,
                    base: base.to_string(),
                    from,
                }))
            })()?;
        let selected_runtime = s
            .config
            .runner_runtime()
            .map_err(WorkError::from)?
            .ok_or_else(|| WorkError::refused("no runtime selected"))?;
        if record.bindings.iter().any(|b| {
            b.role == *role
                && b.attempt.released_at.is_none()
                && b.attempt
                    .runtime
                    .is_some_and(|runtime| runtime != selected_runtime)
        }) {
            return Err(WorkError::refused("retained dispatch belongs to another runtime; reconcile that assignment before retrying").into());
        }
        let task_key = occurrence.map(|at| {
            instrukt_runner::keyed::task_key(project.slug.as_str(), record.id.as_str(), at)
        });
        if let Some(pending) = record
            .bindings
            .iter()
            .find(|b| b.attempt.released_at.is_none() && b.session.is_none())
        {
            if &pending.role != role || pending.attempt.occurrence != occurrence {
                return Err(WorkError::refused(format!(
                    "unresolved assignment `{}` must be reconciled first",
                    pending.attempt.semantic_key
                ))
                .into());
            }
            return Ok(DispatchReservation::Start {
                key: pending.attempt.semantic_key.clone(),
                creation: CreationPermit::Reconcile,
                worktree: placement,
            });
        }
        if let Some(active) = record.bindings.iter().rev().find(|b| {
            &b.role == role
                && b.attempt.released_at.is_none()
                && (occurrence.is_none() || Some(&b.attempt.semantic_key) == task_key.as_ref())
        }) && let Some(session) = &active.session
        {
            return Ok(DispatchReservation::Existing(SessionBinding {
                key: active.attempt.semantic_key.clone(),
                session: session.clone(),
                worktree: active.worktree.clone(),
            }));
        }
        let number = record
            .bindings
            .iter()
            .filter(|b| &b.role == role)
            .map(|b| b.attempt.number)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or_else(|| WorkError::refused("role attempt counter is exhausted"))?;
        let key = task_key.unwrap_or_else(|| {
            instrukt_runner::keyed::todo_key(
                project.slug.as_str(),
                record.id.as_str(),
                role.as_str(),
                number,
            )
        });
        let binding = Binding {
            role: role.clone(),
            session: None,
            worktree: None,
            attempt: super::_model::Attempt {
                number,
                semantic_key: key.clone(),
                occurrence,
                runtime: Some(selected_runtime),
                lane: Some(*lane),
                branch: placement.as_ref().map(|p| p.branch.clone()),
                base_head: placement
                    .as_ref()
                    .map(|p| {
                        instrukt_core::ids::CommitSha::parse(&p.base)
                            .map_err(|e| WorkError::refused(e.to_string()))
                    })
                    .transpose()?,
                start_from: placement.as_ref().map(|p| p.from.clone()),
                assignment: if occurrence.is_some() {
                    AssignmentKind::Task
                } else {
                    AssignmentKind::Driver
                },
                ..Default::default()
            },
        };
        s.commit("dispatch reserved", |s| {
            if let Some(placement) = &placement
                && *lane == instrukt_runner::interfaces::Lane::Work
            {
                s.items[index].record.branch = Some(placement.branch.clone());
            }
            s.items[index].record.bindings.push(binding);
            Ok(())
        })?;
        s.flush()?;
        if s.items[index].reflection.pending || s.items[index].reflection.conflicted {
            return Err(WorkError::refused("the dispatch reservation remains pending GitHub reflection; no runtime start was attempted").into());
        }
        Ok(DispatchReservation::Start {
            key,
            creation: CreationPermit::Initial,
            worktree: placement,
        })
    }

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
            .rev()
            .find(|b| &b.role == role)
            .map(|b| b.binding))
    }

    fn record_dispatch(
        &self,
        project: &ProjectRef,
        todo: &TodoSlug,
        dispatch: &instrukt_runner::interfaces::AcceptedDispatch,
    ) -> Result<(), WorkInterfaceError> {
        let mut s = Session::open(self, project)?;
        let index = s.enter(todo)?;
        let machine = s.machine()?;
        let actor = Actor::Session(dispatch.binding.session.clone());
        let (model, model_provenance) = s.model_of(&actor);
        let now = s.now;
        let fingerprint = instrukt_core::publication::EvalFingerprint::current(self.home())
            .map_err(WorkError::from)?
            .map(|f| f.fingerprint);
        s.commit("dispatch accepted", |s| {
            let record = &mut s.items[index].record;
            let binding = record
                .bindings
                .iter_mut()
                .find(|b| b.attempt.semantic_key == dispatch.key)
                .ok_or_else(|| WorkError::refused("the runtime result has no reserved assignment"))?;
            if binding.attempt.runtime.is_some_and(|runtime|runtime.name()!=dispatch.binding.session.runtime.as_str()) || binding.role != dispatch.role
                || binding.attempt.occurrence != dispatch.occurrence
                || binding.attempt.released_at.is_some()
                || binding.session.as_ref().is_some_and(|id| id != &dispatch.binding.session)
            {
                return Err(WorkError::refused("the reserved assignment was released or bound elsewhere"));
            }
            binding.session = Some(dispatch.binding.session.clone());
            binding.worktree = dispatch.binding.worktree.clone();
            let claim = record.claim.as_mut().ok_or_else(|| {
                WorkError::refused("the execution claim was released during dispatch")
            })?;
            if claim.machine != machine
                || claim.session.as_ref().is_some_and(|id| id != &dispatch.binding.session)
            {
                return Err(WorkError::refused("claimed elsewhere during runtime acceptance"));
            }
            claim.session = Some(dispatch.binding.session.clone());
            if let Some(occurrence) = dispatch.occurrence
                && !record.stamps.iter().any(|s| {
                    matches!(s.fact, StampFact::TaskFiringStarted { occurrence: at } if at == occurrence)
                })
            {
                let sequence = record.next_sequence();
                record.stamps.push(StampRecord {
            source_comment:None,
                    fact: StampFact::TaskFiringStarted { occurrence },
                    at: now,
                    actor,
                    role: Some(dispatch.role.clone()),
                    model,
                    model_provenance,
                    eval_fingerprint: fingerprint,
                    human_involved: None,
                    sequence,
                });
            }
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
        poll_interest(&mut s)?;
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
                .find(|b| b.session.as_ref() == Some(session))
                .map(|b| b.role.clone());
            let sequence = record.next_sequence();
            record.stamps.push(StampRecord {
                source_comment: None,
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
        let held = s.items[index].record.hold.is_some();
        if held {
            tracing::debug!(
                event = "work_hold_kept",
                todo = %todo,
                "the todo is already on hold; the automatic hold adds nothing"
            );
            if matches!(cause, HoldCause::Anomaly { .. }) {
                return Ok(());
            }
        }
        if s.items[index].record.status == TodoStatus::StructuralHolder {
            return Err(WorkError::refused(format!(
                "`{todo}` is a structural holder; it carries no dispatch hold"
            ))
            .into());
        }
        let reason = match cause {
            HoldCause::TaskFailed { reason, .. } => format!("task failed: {reason}"),
            HoldCause::Anomaly { reason } => format!("anomaly: {reason}"),
        };
        let now = s.now;
        let id = s.items[index].record.id.clone();
        s.commit_with(
            "automatic hold",
            |s| {
                if !held {
                    s.items[index].record.hold = Some(HoldRecord {
                        source: HoldSource::Automatic,
                        reason,
                        by: Actor::Person {
                            name: RUNNER.to_owned(),
                        },
                        at: now,
                    });
                }
                Ok(())
            },
            |tx| {
                instrukt_runner::dispatch::record_task_failure(
                    tx,
                    &project.slug,
                    id.as_str(),
                    cause,
                )?;
                Ok(())
            },
        )?;
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
                read_through(&mut s, index)?;
                s.items[index].record.clone()
            }
            None => match s.known_copy(todo) {
                Some(copy) => copy_record(&mut s, copy)?,
                None => match s.remote_item(todo)? {
                    Some(item) => item.record,
                    None => return Ok(None),
                },
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
                read_through(&mut s, index)?;
                s.items[index].record.clone()
            }
            None => match s.copies.iter().find(|c| &c.record.id == id).cloned() {
                Some(copy) => copy_record(&mut s, copy)?,
                None => match s.remote_matching(&format!("todo {id}"), |r| &r.id == id)? {
                    Some(item) => item.record,
                    None => return Ok(None),
                },
            },
        };
        let prerequisites = expand(&mut s, &record);
        Ok(Some(to_evidence(record, prerequisites)))
    }
}

/// Pulls local item `index` for a read: a conflicted item is still read.
fn read_through(s: &mut Session<'_>, index: usize) -> Result<(), WorkError> {
    match s.pull_through(index) {
        Ok(()) | Err(WorkError::Conflicted { .. }) => Ok(()),
        Err(err) => Err(err),
    }
}

/// The record of listing copy `copy`, refreshed from GitHub when it can be
/// read, else as last known.
fn copy_record(
    s: &mut Session<'_>,
    copy: super::_store::ListingCopy,
) -> Result<TodoRecord, WorkError> {
    if !s.refreshed() {
        return Ok(copy.record);
    }
    match s.refresh_copy(copy.issue_number) {
        Ok(Some(item)) => Ok(item.record),
        Ok(None) => Ok(copy.record),
        Err(WorkError::GitHub(err)) if err.is_reach() => Ok(copy.record),
        Err(err) => Err(err),
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
        for copy in &s.copies {
            let remote = &copy.record;
            if remote
                .group
                .as_ref()
                .is_some_and(|g| g.id == prerequisite.id)
                && remote.is_live()
                && !children.iter().any(|c| c.id == remote.id)
            {
                children.push(remote.clone());
            }
        }
        if children.is_empty() {
            out.push(Prerequisite::EmptyHolder(prerequisite.slug));
        }
        stack.extend(children.into_iter().rev().map(|c| c.reference()));
    }
    out
}

pub(crate) fn to_evidence(record: TodoRecord, prerequisites: Vec<Prerequisite>) -> TodoEvidence {
    let implementation_plan_seeded = seeded(&record);
    TodoEvidence {
        branch: record.branch.clone(),
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
            .filter_map(|b| {
                Some(RoleBinding {
                    role: b.role,
                    binding: SessionBinding {
                        key: b.attempt.semantic_key.clone(),
                        session: b.session?,
                        worktree: b.worktree,
                    },
                    attempt: b.attempt,
                })
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
            prior_slugs: Vec::new(),
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
            label_actions: Vec::new(),
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
            source_comment: None,
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
