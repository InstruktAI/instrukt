//! Work's implementations of the interfaces other contexts declare: the
//! runner's work interfaces, workflow's evidence read and core's todo
//! existence.

use instrukt_core::config::Configuration;
use instrukt_core::github;
use instrukt_core::ids::{ProjectSlug, RoleName, SessionRef, TodoSlug};
use instrukt_core::interfaces::{InterfaceError, TodoExistence};
use instrukt_runner::interfaces::{
    AutomaticHolds, ClaimOutcome, Claimant, Claims, Firing, GitHubReach, HoldCause, Pickup,
    PickupCandidate, ProjectRef, Reflection, ReflectionReport, RetentionPurge, SessionBinding,
    SessionBindings, TaskFirings, TaskOutcome, TaskTiming, TodoKind, WorkInterfaceError,
};
use jiff::Timestamp;

use super::_envelope::ShownStatus;
use super::_error::WorkError;
use super::_github::{GitHub, GitHubError};
use super::_model::{Binding, Claim, TodoId, TodoRecord};
use super::_reflect;
use super::_session::{self, GitHubRead, PendingReason, ReflectionStatus, Session};
use super::_store;
use super::Work;
use crate::evidence::{
    Actor, ArtifactHistory, ArtifactVersion, ClaimRecord, HoldRecord, HoldSource, Prerequisite,
    RoleBinding, StampFact, StampRecord, TaskResult, TodoEvidence, TodoStatus, WorkEvidence,
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
            .filter(|it| !it.reflection.conflicted && !s.incompatible.contains(&it.record.id))
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
            if s.incompatible.contains(&copy.record.id) {
                continue;
            }
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
        ReflectionStatus::Conflicted { .. } => ClaimOutcome::ClaimedElsewhere,
        ReflectionStatus::Pending(_) => ClaimOutcome::NotReflected,
    }
}

fn checked_replacement(
    record: &super::_model::TodoRecord,
    machine: &instrukt_core::machine::MachineId,
    key: &str,
    stopped: &SessionRef,
) -> Result<usize, WorkError> {
    let index = record
        .bindings
        .iter()
        .position(|binding| binding.attempt.semantic_key == key)
        .ok_or_else(|| {
            WorkError::refused("the replacement key no longer identifies this assignment")
        })?;
    let old = &record.bindings[index];
    let claim = record
        .claim
        .as_ref()
        .ok_or_else(|| WorkError::refused("the replacement has no current execution claim"))?;
    if old
        .session
        .as_ref()
        .is_some_and(|session| session != stopped)
        || old.attempt.released_at.is_some()
        || &claim.machine != machine
        || claim
            .session
            .as_ref()
            .is_some_and(|session| Some(session) != old.session.as_ref())
    {
        return Err(WorkError::refused(
            "the replacement does not match the current todo binding and claim",
        ));
    }
    if let Some(path) = old.worktree.as_ref() {
        if let Some(expected) = old.attempt.branch.as_ref() {
            let branch = instrukt_core::git::run(path, ["branch", "--show-current"])
                .map_err(|e| WorkError::refused(e.to_string()))?;
            if branch.trim() != expected {
                return Err(WorkError::refused(
                    "retained phase worktree no longer carries its captured branch",
                ));
            }
        }
        if let Some(base) = old.attempt.base_head.as_ref().map(|base| base.as_str()) {
            instrukt_core::git::run(path, ["merge-base", "--is-ancestor", base, "HEAD"]).map_err(
                |e| {
                    WorkError::refused(format!(
                        "retained phase worktree differs from its captured base: {e}"
                    ))
                },
            )?;
        }
    }
    Ok(index)
}

/// Whether item `index`, as the state database holds it after this
/// command's flush, admits a native start or follow-up on this computer:
/// reflected and not conflicted, still active, unheld and claimed by
/// `machine` with no session, and holding the assignment `holds` accepts.
/// The session's own copy is never the evidence: another command, a
/// refresh or a reflection can have replaced the row between the commit and
/// the flush. Why it does not, otherwise.
fn admitted(
    s: &Session<'_>,
    index: usize,
    machine: &instrukt_core::machine::MachineId,
    holds: impl Fn(&Binding) -> bool,
) -> Result<(), &'static str> {
    let Some((live, withdrawing)) = s.confirmed(index).map_err(|_| "cannot be read back")? else {
        return Err("is gone from this computer's record");
    };
    if withdrawing {
        return Err("belongs to a todo whose withdrawal is releasing its working resources");
    }
    if live.reflection.conflicted {
        return Err("met a newer GitHub revision");
    }
    if live.reflection.pending {
        return Err("remains pending GitHub reflection");
    }
    let record = &live.record;
    if record.status != TodoStatus::Active
        || record.hold.is_some()
        || !record
            .claim
            .as_ref()
            .is_some_and(|c| &c.machine == machine && c.session.is_none())
    {
        return Err("no longer stands on this computer's unassigned execution claim");
    }
    if !record.bindings.iter().any(holds) {
        return Err("is no longer recorded");
    }
    Ok(())
}

pub(super) fn matches_driver_role(bound: &RoleName, requested: &RoleName) -> bool {
    bound == requested
        || (requested.as_str() == "todo-orchestrator" && bound.as_str() == "work-orchestrator")
}

fn replacement_todo(s: &Session<'_>, key: &str) -> Result<TodoSlug, WorkError> {
    let mut owners = s.items.iter().filter(|item| {
        item.record
            .bindings
            .iter()
            .any(|binding| binding.attempt.semantic_key == key)
    });
    let todo = owners
        .next()
        .map(|item| item.record.slug.clone())
        .ok_or_else(|| WorkError::refused("the replacement key has no local todo assignment"))?;
    if owners.next().is_some() {
        return Err(WorkError::refused(
            "the replacement key identifies multiple todo assignments",
        ));
    }
    Ok(todo)
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
        let attempted: bool = state
            .conn()
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM runtime_bindings WHERE semantic_key=?1)",
                [key],
                |row| row.get(0),
            )
            .map_err(instrukt_core::state_db::StateDbError::sqlite(state.path()))
            .map_err(WorkError::from)?;
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
        let mut s = Session::open(self, project)?;
        let index = s.enter(todo)?;
        // A start or follow-up on a todo whose working resources are being
        // released would run in a worktree that is going.
        s.refuse_withdrawing(index)?;
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
                    matches_driver_role(&b.role, role)
                        && b.attempt.assignment == AssignmentKind::Driver
                        && b.attempt.lane == Some(*lane)
                        && b.session.is_some()
                        && b.worktree.as_ref().is_some_and(|tree| tree.is_dir())
                        && b.attempt.failed_at.is_none()
                })
                .cloned()
        {
            if retained.attempt.runtime != Some(s.config.runner_runtime().map_err(WorkError::from)?)
            {
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
                let key = &retained.attempt.semantic_key;
                admitted(&s, index, &machine, |b| {
                    &b.attempt.semantic_key == key
                        && b.attempt.released_at.is_none()
                        && b.session == retained.session
                })
                .map_err(|why| {
                    WorkError::refused(format!(
                        "the resumed assignment {why}; no runtime follow-up was attempted"
                    ))
                })?;
            } else if s.unjudged(index) {
                return Err(super::_session::unjudged_refusal(todo).into());
            }
            return Ok(DispatchReservation::Existing(SessionBinding {
                title: retained
                    .attempt
                    .native_placement
                    .as_ref()
                    .map(|p| p.title.clone()),
                key: retained.attempt.semantic_key,
                session: retained
                    .session
                    .ok_or_else(|| WorkError::refused("retained phase has no native session"))?,
                worktree: retained.worktree,
            }));
        }
        let placement = (|| -> Result<_, WorkError> {
            if *lane == instrukt_runner::interfaces::Lane::TaskRun || record.kind == TodoKind::Task
            {
                return Ok(None);
            }
            let branch = format!("{}/work", record.slug);
            if let Some(pending) = record
                .bindings
                .iter()
                .rev()
                .find(|b| b.role == *role && b.attempt.released_at.is_none())
            {
                return Ok(Some(instrukt_runner::runtime::WorktreeRequest {
                    branch: pending
                        .attempt
                        .branch
                        .clone()
                        .ok_or_else(|| WorkError::refused("retained todo branch is missing"))?,
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
                    "todo branch `{branch}` already exists; no replacement worktree is created"
                )));
            }
            let (from, base) = super::_attempts::phase_base(&project.root, &branch)?;
            Ok(Some(instrukt_runner::runtime::WorktreeRequest {
                branch,
                base: base.to_string(),
                from,
            }))
        })()?;
        let selected_runtime = s.config.runner_runtime().map_err(WorkError::from)?;
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
        let is_legacy_driver_pending = |b: &Binding| -> bool {
            role.as_str() == "todo-orchestrator"
                && matches!(
                    b.role.as_str(),
                    "prepare-orchestrator" | "work-orchestrator"
                )
                && b.attempt.assignment == AssignmentKind::Driver
        };
        if let Some(pending) = record.bindings.iter().find(|b| {
            b.attempt.released_at.is_none() && b.session.is_none() && !is_legacy_driver_pending(b)
        }) {
            if &pending.role != role || pending.attempt.occurrence != occurrence {
                return Err(WorkError::refused(format!(
                    "unresolved assignment `{}` must be reconciled first",
                    pending.attempt.semantic_key
                ))
                .into());
            }
            if s.unjudged(index) {
                return Err(super::_session::unjudged_refusal(todo).into());
            }
            return Ok(DispatchReservation::Start {
                key: pending.attempt.semantic_key.clone(),
                worktree: placement,
            });
        }
        if let Some(active) = record.bindings.iter().rev().find(|b| {
            &b.role == role
                && b.attempt.released_at.is_none()
                && (occurrence.is_none() || Some(&b.attempt.semantic_key) == task_key.as_ref())
        }) && let Some(session) = &active.session
        {
            if s.unjudged(index) {
                return Err(super::_session::unjudged_refusal(todo).into());
            }
            return Ok(DispatchReservation::Existing(SessionBinding {
                title: active
                    .attempt
                    .native_placement
                    .as_ref()
                    .map(|p| p.title.clone()),
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
        let now = s.now;
        s.commit("dispatch reserved", |s| {
            let record = &mut s.items[index].record;
            // Retire any legacy prepare-orchestrator driver binding (and any
            // unbound legacy work-orchestrator reservation) when the unified
            // todo-orchestrator takes over on `<slug>/work`.
            if role.as_str() == "todo-orchestrator" {
                for b in &mut record.bindings {
                    if b.attempt.assignment == AssignmentKind::Driver
                        && b.attempt.released_at.is_none()
                        && (b.role.as_str() == "prepare-orchestrator"
                            || (b.role.as_str() == "work-orchestrator" && b.session.is_none()))
                    {
                        b.attempt.released_at = Some(now);
                    }
                }
            }
            record.bindings.push(binding);
            Ok(())
        })?;
        s.flush()?;
        admitted(&s, index, &machine, |b| {
            b.attempt.semantic_key == key && b.attempt.released_at.is_none() && b.session.is_none()
        })
        .map_err(|why| {
            WorkError::refused(format!(
                "the dispatch reservation {why}; no runtime start was attempted"
            ))
        })?;
        Ok(DispatchReservation::Start {
            key,
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
            .find(|b| matches_driver_role(&b.role, role))
            .map(|b| b.binding))
    }

    fn check_dispatch_replacement(
        &self,
        project: &ProjectRef,
        key: &str,
        stopped: &SessionRef,
    ) -> Result<(), WorkInterfaceError> {
        let mut s = Session::open(self, project)?;
        let machine = s.machine()?;
        let todo = replacement_todo(&s, key)?;
        let index = s.enter_judged(&todo)?;
        s.refuse_withdrawing(index)?;
        checked_replacement(&s.items[index].record, &machine, key, stopped)?;
        Ok(())
    }

    fn replace_dispatch(
        &self,
        project: &ProjectRef,
        binding: &SessionBinding,
        stopped: &SessionRef,
        accept_runtime: &mut dyn FnMut(
            &instrukt_core::state_db::rusqlite::Transaction<'_>,
        ) -> Result<(), instrukt_runner::state::StateError>,
    ) -> Result<(), WorkInterfaceError> {
        let mut s = Session::open(self, project)?;
        let machine = s.machine()?;
        let todo = replacement_todo(&s, &binding.key)?;
        let index = s.enter(&todo)?;
        let record = &s.items[index].record;
        let old_index = checked_replacement(record, &machine, &binding.key, stopped)?;
        let todo_id = record.id.clone();
        let role = record.bindings[old_index].role.clone();
        s.commit_with(
            "dispatch replaced",
            |s| {
                let record = &mut s.items[index].record;
                let old_index = checked_replacement(record, &machine, &binding.key, stopped)?;
                if stopped.runtime != binding.session.runtime {
                    return Err(WorkError::refused(
                        "the replacement is in a different runtime",
                    ));
                }
                let old = &mut record.bindings[old_index];
                let claim = record.claim.as_mut().ok_or_else(|| {
                    WorkError::refused("the replacement has no current execution claim")
                })?;
                old.session = Some(binding.session.clone());
                old.worktree.clone_from(&binding.worktree);
                claim.session = Some(binding.session.clone());
                Ok(())
            },
            |tx| {
                instrukt_runner::executions::replace_session(
                    tx,
                    &project.slug,
                    todo_id.as_str(),
                    &role,
                    stopped,
                    &binding.session,
                )
                .map_err(|error| WorkError::refused(error.to_string()))?;
                accept_runtime(tx).map_err(|error| WorkError::refused(error.to_string()))
            },
        )?;
        Ok(())
    }

    fn reflect_dispatch_replacement(
        &self,
        project: &ProjectRef,
        key: &str,
    ) -> Result<(), WorkInterfaceError> {
        let mut s = Session::open(self, project)?;
        let todo = replacement_todo(&s, key)?;
        let index = s.enter(&todo)?;
        s.flush()?;
        match s.status(index) {
            ReflectionStatus::Reflected => Ok(()),
            ReflectionStatus::Pending(reason) => {
                Err(WorkError::refused(super::_session::describe(&reason)).into())
            }
            ReflectionStatus::Conflicted { .. } => Err(WorkError::refused(
                "todo reflection has a conflicting revision. Run `instrukt todo refresh`",
            )
            .into()),
        }
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
            if binding.attempt.runtime.is_some_and(|runtime|runtime.name()!=dispatch.binding.session.runtime.as_str()) || !matches_driver_role(&binding.role, &dispatch.role)
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
        if let Some(cause) = &s.board_setup_failure {
            return Err(WorkError::refused(format!("cockpit setup pending: {cause}")).into());
        }
        let reach = match s.github_read() {
            GitHubRead::Read | GitHubRead::Unjudged => GitHubReach::Reachable,
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
                ReflectionStatus::Conflicted { .. } => conflicted.push(item.record.slug.clone()),
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

/// Core's todo-existence question for one project, answered by work without
/// taking the state writer lock or creating state files.
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
        let (items, copies, provider) =
            _store::read_existence_state(self.work.home(), &self.project.slug)
                .map_err(|e| failed(e.to_string()))?;
        if _session::find_in(&items, slug, true)
            .is_some_and(|i| items[i].record.status != TodoStatus::Removed)
        {
            return Ok(true);
        }
        if copies
            .iter()
            .any(|c| &c.record.slug == slug && c.record.is_live())
        {
            return Ok(true);
        }
        let config = Configuration::load(self.work.home(), Some(&self.project.root))
            .map_err(|e| failed(e.to_string()))?;
        let api_url = github::api_url(self.work.home()).map_err(|e| failed(e.to_string()))?;
        let repository = match github::code_host(&config, &api_url, &self.project.root)
            .map_err(|e| failed(e.to_string()))?
        {
            github::CodeHost::GitHub { repository } => repository,
            github::CodeHost::Elsewhere { reason } => {
                return Err(failed(WorkError::CodeNotOnGitHub { reason }.to_string()));
            }
        };
        let now = self.work.now();
        // Anchored before the capability lock is taken (see `Session::open`).
        let clock = super::_apps::Clock::starting(now);
        let unreadable = |reason: PendingReason| {
            failed(format!(
                "cannot tell whether todo `{slug}` exists: it is not on this computer and GitHub cannot be read ({})",
                _session::describe(&reason)
            ))
        };
        if let Some(until) = provider.deferred_until
            && until > now
        {
            return Err(unreadable(PendingReason::RateLimited {
                until: Some(until),
            }));
        }
        self.work
            .github
            .begin_pass()
            .map_err(|e| failed(WorkError::GitHub(GitHubError::from(e)).to_string()))?;
        let gh = GitHub::for_app(
            self.work.github.clone(),
            &api_url,
            &repository,
            None,
            github::Operation::Record,
            clock,
        )
        .map_err(|e| unreadable(_session::reason_of(&e)))?;
        let remote =
            _reflect::remote_records(&gh, true).map_err(|e| unreadable(_session::reason_of(&e)))?;
        if remote
            .records
            .iter()
            .any(|(r, _)| &r.slug == slug && r.is_live())
        {
            return Ok(true);
        }
        if let Some((_, error)) = remote.incompatible.first() {
            return Err(unreadable(PendingReason::Failed(error.to_string())));
        }
        Ok(false)
    }
}

/// The record's prerequisites with structural holders expanded to the
/// todos whose milestones satisfy them.
pub(super) fn expand(s: &mut Session<'_>, record: &TodoRecord) -> Vec<Prerequisite> {
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

/// Whether the todo waits on a person (`github-record`, the `Needs you`
/// marker): it shows as held, or a current approval gate waits for a
/// human. Work decides this once; the board marker and workflow's
/// assignment relay it. `awaiting_human_approval` answers whether an
/// approval gate waits for a human under the configured `prd` and `task`
/// approvers; it is asked only when the shown status leaves that open.
pub(crate) fn waits_on_person<E>(
    shown: ShownStatus,
    awaiting_human_approval: impl FnOnce() -> Result<bool, E>,
) -> Result<bool, E> {
    if shown == ShownStatus::Held {
        return Ok(true);
    }
    awaiting_human_approval()
}

pub(crate) fn to_evidence(record: TodoRecord, prerequisites: Vec<Prerequisite>) -> TodoEvidence {
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
                        title: b.attempt.native_placement.as_ref().map(|p| p.title.clone()),
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
    }
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
    use std::convert::Infallible;

    use instrukt_core::config::Approver;
    use instrukt_runner::interfaces::TodoKind;

    use super::{ShownStatus, waits_on_person};
    use crate::evidence::{
        PreparationReview, ReviewVerdict, StampFact, TodoEvidence, TodoStatus, artifact_names,
    };
    use crate::workflow::_testing::{digest, todo};
    use crate::workflow::waiting_for_human_approval;

    fn waits(evidence: &TodoEvidence, approver: Approver) -> bool {
        let shown = ShownStatus::of(evidence.status, evidence.hold.is_some());
        let Ok(waits) = waits_on_person(shown, || {
            Ok::<_, Infallible>(waiting_for_human_approval(evidence, approver, approver))
        });
        waits
    }

    #[test]
    fn a_hold_waits_on_a_person_while_the_todo_shows_as_held() {
        let held = todo(TodoKind::Change).held().build();
        assert!(waits(&held, Approver::Human));
        assert!(waits(&held, Approver::Peer));

        let delivered = todo(TodoKind::Change)
            .held()
            .status(TodoStatus::Delivered)
            .build();
        assert!(!waits(&delivered, Approver::Human));
        assert!(!waits(&delivered, Approver::Peer));
    }

    #[test]
    fn a_todo_shown_as_held_waits_without_asking_about_approval() {
        assert_eq!(waits_on_person(ShownStatus::Held, || Err(())), Ok(true));
        assert_eq!(waits_on_person(ShownStatus::Active, || Err(())), Err(()));
    }

    #[test]
    fn an_unapproved_reviewed_prd_waits_on_a_person_only_under_a_human_approver() {
        let reviewed = todo(TodoKind::Feature)
            .artifact(artifact_names::PRD, "prd-v1")
            .artifact(artifact_names::PRD_REVIEW_FINDINGS, "ok")
            .stamped(StampFact::PreparationVerdict {
                phase: PreparationReview::PrdReview,
                verdict: ReviewVerdict::Approve,
                target: digest("prd-v1"),
                report: digest("ok"),
            })
            .build();
        assert!(waits(&reviewed, Approver::Human));
        assert!(!waits(&reviewed, Approver::Peer));
        assert!(!waits(&todo(TodoKind::Feature).build(), Approver::Human));
    }
}
