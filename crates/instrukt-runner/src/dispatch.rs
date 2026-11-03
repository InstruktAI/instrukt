//! Todo dispatch (`work-dispatch`, "Execution contract", "Todo pickup" and
//! "Agent selection and intent checks"; `task-timing`, "Task and
//! schedule"), against the interfaces work and workflow implement.
//!
//! Per project with `todos` on, each tick: flush and pull reflection,
//! purge expired retained records, then — for the kinds
//! `runner.autostart` enables — take work's pickup candidates in order and
//! ask workflow for each one's assignment. A lane is dispatched into the
//! procedure declaring the lane flow's orchestrator role; a due task into
//! the procedure declaring `task-runner`. Before a session starts the
//! runner records and reflects its claim; an unreflected claim stays local
//! and pending and nothing starts. An exhausted agent order releases the
//! runner's claim through work's command contract and is an anomaly.

use std::collections::BTreeMap;

use instrukt_core::config::{Configuration, Env};
use instrukt_core::home::Home;
use instrukt_core::ids::{ProjectSlug, RoleName, SessionRef, TodoSlug};
use instrukt_core::index::{GlobalIndex, IndexEntry};
use jiff::Timestamp;
use jiff::tz::TimeZone;

use crate::agents::{agent_order, global_order};
use crate::anomaly;
use crate::executions::{Execution, Executions};
use crate::interfaces::{
    ClaimOutcome, Claimant, HoldCause, Lane, PickupCandidate, ProjectRef, RunnerWiring,
    SessionBinding, TaskOutcome, TodoKind, WorkPorts, WorkflowAssignments,
};
use crate::launch;
use crate::sessions::{Observation, SessionState, Sessions, SessionsError, StartRequest};
use crate::state::{self, RunnerState};
use crate::timing::{self, Decision};

/// The role of the procedure that runs a due task.
pub const TASK_RUNNER_ROLE: &str = "task-runner";

/// What dispatch needs from the invocation.
pub struct DispatchContext<'a> {
    pub home: &'a Home,
    pub env: &'a Env,
    pub now: Timestamp,
    pub tz: &'a TimeZone,
    pub previous_wake: Option<Timestamp>,
    pub state: &'a RunnerState,
}

/// What dispatch did with one todo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Dispatched {
    Started {
        todo: TodoSlug,
    },
    ClaimedElsewhere {
        todo: TodoSlug,
    },
    ClaimPending {
        todo: TodoSlug,
    },
    Exhausted {
        todo: TodoSlug,
    },
    /// The start's result is unknown; the todo is held.
    StartUncertain {
        todo: TodoSlug,
    },
}

/// The kinds `runner.autostart` picks up automatically in `config`.
pub fn autostart_kinds(config: &Configuration) -> Vec<TodoKind> {
    TodoKind::ALL
        .into_iter()
        .filter(|kind| {
            config
                .runner_autostart(kind.as_str())
                .unwrap_or_else(|err| {
                    tracing::warn!(event = "autostart_unreadable", error = %err);
                    false
                })
        })
        .collect()
}

/// Dispatches `project`'s eligible todos.
pub fn dispatch_project(
    cx: &DispatchContext<'_>,
    config: &Configuration,
    wiring: &RunnerWiring,
    project: &ProjectRef,
) -> Vec<Dispatched> {
    let (Some(work), Some(workflow)) = (&wiring.work, &wiring.workflow) else {
        tracing::debug!(event = "dispatch_unavailable", project = %project.slug);
        return Vec::new();
    };
    match work.retention.purge_expired(project) {
        Ok(purged) if !purged.is_empty() => {
            tracing::info!(event = "retained_records_purged", project = %project.slug, count = purged.len())
        }
        Ok(_) => {}
        Err(err) => {
            tracing::warn!(event = "retention_purge_failed", project = %project.slug, error = %err)
        }
    }
    // Pickup runs before the reflection pass: a claim this runner left
    // pending is offered once more (`claimed_here`) and reflected by
    // claiming it again, then dispatched (ruling D). Flushing first would
    // reflect it without a session to hold it.
    let dispatched = pickup(cx, config, work, workflow.as_ref(), project);
    match work.reflection.sync_reflection(project) {
        Ok(report) => {
            tracing::debug!(event = "reflection_synced", project = %project.slug, pending = report.pending.len(), conflicted = report.conflicted.len())
        }
        Err(err) => {
            tracing::warn!(event = "reflection_failed", project = %project.slug, error = %err)
        }
    }
    dispatched
}

fn pickup(
    cx: &DispatchContext<'_>,
    config: &Configuration,
    work: &WorkPorts,
    workflow: &dyn WorkflowAssignments,
    project: &ProjectRef,
) -> Vec<Dispatched> {
    let offered = match work.pickup.pickup_candidates(project, &TodoKind::ALL) {
        Ok(candidates) => candidates,
        Err(err) => {
            tracing::warn!(event = "pickup_unreadable", project = %project.slug, error = %err);
            return Vec::new();
        }
    };
    if let Err(err) = Executions::new(cx.state).observe(config, cx.env, workflow, project, &offered)
    {
        tracing::warn!(event = "executions_unobservable", project = %project.slug, error = %err);
    }
    for candidate in &offered {
        hold_failed_firing(cx, work, project, candidate);
    }
    notify_task_failures(cx, config, project);
    let kinds = autostart_kinds(config);
    if kinds.is_empty() {
        return Vec::new();
    }
    let index = match GlobalIndex::load(cx.home) {
        Ok(index) => index.unwrap_or_default(),
        Err(err) => {
            tracing::warn!(event = "index_unreadable", error = %err);
            return Vec::new();
        }
    };
    offered
        .iter()
        .filter(|candidate| kinds.contains(&candidate.kind))
        .filter_map(|candidate| {
            dispatch_one(cx, config, work, workflow, &index, project, candidate)
        })
        .collect()
}

fn dispatch_one(
    cx: &DispatchContext<'_>,
    config: &Configuration,
    work: &WorkPorts,
    workflow: &dyn WorkflowAssignments,
    index: &GlobalIndex,
    project: &ProjectRef,
    candidate: &PickupCandidate,
) -> Option<Dispatched> {
    let todo = &candidate.todo;
    let assignment = match workflow.assignment(project, todo) {
        Ok(Some(assignment)) => assignment,
        Ok(None) => return None,
        Err(err) => {
            tracing::warn!(event = "assignment_unreadable", project = %project.slug, todo = %todo, error = %err);
            return None;
        }
    };
    let (role, occurrence) = match assignment.lane {
        Lane::TaskRun => {
            let timing = candidate.timing.as_ref()?;
            match timing::decide(timing, cx.previous_wake, cx.now, cx.tz) {
                Decision::Fire {
                    occurrence,
                    catch_up,
                } => {
                    tracing::info!(event = "task_due", project = %project.slug, todo = %todo, occurrence = %occurrence, catch_up);
                    (RoleName::parse(TASK_RUNNER_ROLE).ok()?, Some(occurrence))
                }
                Decision::NotDue | Decision::InFlight => return None,
            }
        }
        Lane::Prepare | Lane::Work => (assignment.orchestrator?, None),
    };
    let Some(procedure) = procedure_for_role(index, config, &role) else {
        tracing::warn!(event = "no_procedure_for_role", project = %project.slug, todo = %todo, role = %role, "no procedure of the selected flavor's domain declares this role");
        return None;
    };
    match work.claims.record_claim(project, todo, &Claimant::Runner) {
        Ok(ClaimOutcome::Claimed | ClaimOutcome::AlreadyHeld) => {}
        Ok(ClaimOutcome::ClaimedElsewhere) => {
            tracing::info!(event = "todo_claimed_elsewhere", project = %project.slug, todo = %todo);
            return Some(Dispatched::ClaimedElsewhere { todo: todo.clone() });
        }
        Ok(ClaimOutcome::NotReflected) => {
            tracing::info!(event = "todo_claim_pending", project = %project.slug, todo = %todo);
            return Some(Dispatched::ClaimPending { todo: todo.clone() });
        }
        Err(err) => {
            tracing::warn!(event = "todo_claim_failed", project = %project.slug, todo = %todo, error = %err);
            return None;
        }
    }
    // A task's session learns which scheduled occurrence it executes, so
    // its start and outcome stamps can carry that occurrence.
    let brief = occurrence.map(|occurrence| format!("occurrence: {occurrence}"));
    let prepared = match launch::prepare(procedure, Some(todo.as_str()), brief.as_deref()) {
        Ok(prepared) => prepared,
        Err(err) => {
            tracing::warn!(event = "launch_refused", project = %project.slug, todo = %todo, error = %err);
            return None;
        }
    };
    let (global, thinking) = global_order(config);
    let order = agent_order(
        candidate.agent.as_ref(),
        prepared.dispatch.as_ref(),
        &global,
        thinking,
    );
    let sessions = Sessions::new(config, cx.env);
    let started = sessions.start(
        &order,
        &StartRequest {
            message: &prepared.message,
            worktree: None,
            cwd: Some(&project.root),
        },
    );
    let started = match started {
        Ok(started) => started,
        Err(SessionsError::OrderExhausted { tried }) => {
            tracing::warn!(event = "dispatch_order_exhausted", project = %project.slug, todo = %todo, tried = ?tried);
            match work.claims.release_claim(project, todo, &Claimant::Runner) {
                Ok(()) => {
                    tracing::info!(event = "runner_claim_released", project = %project.slug, todo = %todo);
                }
                Err(err) => {
                    tracing::warn!(event = "runner_claim_release_failed", project = %project.slug, todo = %todo, error = %err);
                }
            }
            anomaly::notify(config, cx.env, &format!("{}/{todo}", project.slug), None);
            return Some(Dispatched::Exhausted { todo: todo.clone() });
        }
        Err(err) if err.start_certainly_failed() => {
            tracing::warn!(event = "dispatch_start_failed", project = %project.slug, todo = %todo, error = %err);
            return None;
        }
        Err(err) => {
            // The runtime may have created a session the runner cannot
            // name: hold the todo so no second session starts, and report
            // it; the developer releases the hold once settled.
            tracing::warn!(event = "dispatch_start_uncertain", project = %project.slug, todo = %todo, error = %err);
            let cause = HoldCause::Anomaly {
                reason: format!("a session start's result is unknown: {err}"),
            };
            if let Err(err) = work.holds.set_automatic_hold(project, todo, &cause) {
                tracing::warn!(event = "dispatch_uncertain_hold_failed", project = %project.slug, todo = %todo, error = %err);
            }
            anomaly::notify(config, cx.env, &format!("{}/{todo}", project.slug), None);
            return Some(Dispatched::StartUncertain { todo: todo.clone() });
        }
    };
    let binding = SessionBinding {
        session: started.session.clone(),
        worktree: started.worktree,
    };
    let bind_role = prepared.role.clone().unwrap_or(role);
    if let Err(err) = work
        .bindings
        .bind_session(project, todo, &bind_role, &binding)
    {
        tracing::warn!(event = "session_binding_failed", project = %project.slug, todo = %todo, session = %started.session, error = %err);
    }
    if let Some(occurrence) = occurrence
        && let Err(err) =
            work.firings
                .record_firing_started(project, todo, occurrence, &started.session)
    {
        tracing::warn!(event = "task_firing_unrecorded", project = %project.slug, todo = %todo, error = %err);
    }
    if let Err(err) = Executions::new(cx.state).record(Execution {
        project: project.slug.clone(),
        todo: todo.clone(),
        role: bind_role,
        session: started.session.clone(),
        state: assignment.state.clone(),
        recovery_sent: false,
        anomaly_raised: false,
    }) {
        tracing::warn!(event = "execution_unrecorded", project = %project.slug, todo = %todo, error = %err);
    }
    tracing::info!(event = "todo_dispatched", project = %project.slug, todo = %todo, session = %started.session);
    Some(Dispatched::Started { todo: todo.clone() })
}

/// The procedure declaring `role` in the selected flavor's domain
/// (`workflow.domain`); a declaration in another domain does not count.
pub fn procedure_for_role<'i>(
    index: &'i GlobalIndex,
    config: &Configuration,
    role: &RoleName,
) -> Option<&'i IndexEntry> {
    let domain = config.workflow_domain().ok()?;
    index.snippets.iter().find(|entry| {
        entry.taxonomy == "procedure"
            && entry.domain == domain.as_str()
            && entry.role.as_deref() == Some(role.as_str())
    })
}

/// A task failure the runner holds and reports: its occurrence's
/// `error` outcome, the session that ran it, and whether the developer was
/// notified yet.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct TaskFailure {
    project: ProjectSlug,
    todo: TodoSlug,
    session: Option<SessionRef>,
    notified: bool,
}

/// A task whose last firing recorded `error` is a blocker needing the
/// developer's judgment (`task-timing`, UC-TT4): the todo is held at once
/// with the task-error reason, so no further occurrence runs on its own,
/// and the failure is remembered for notification.
fn hold_failed_firing(
    cx: &DispatchContext<'_>,
    work: &WorkPorts,
    project: &ProjectRef,
    candidate: &PickupCandidate,
) {
    let Some(firing) = candidate
        .timing
        .as_ref()
        .and_then(|t| t.last_fired.as_ref())
    else {
        return;
    };
    let Some(TaskOutcome::Error { reason }) = &firing.outcome else {
        return;
    };
    let todo = &candidate.todo;
    let key = format!("{}/{todo}/{}", project.slug, firing.occurrence);
    let path = cx.state.task_failures();
    let mut failures: BTreeMap<String, TaskFailure> = match state::read_json(&path) {
        Ok(failures) => failures.unwrap_or_default(),
        Err(err) => {
            tracing::warn!(event = "task_failures_unreadable", error = %err);
            return;
        }
    };
    if failures.contains_key(&key) {
        return;
    }
    let cause = HoldCause::TaskFailed {
        reason: reason
            .clone()
            .unwrap_or_else(|| "no reason recorded".to_owned()),
    };
    if let Err(err) = work.holds.set_automatic_hold(project, todo, &cause) {
        tracing::warn!(event = "task_failure_hold_failed", project = %project.slug, todo = %todo, error = %err);
        return;
    }
    let session = RoleName::parse(TASK_RUNNER_ROLE)
        .ok()
        .and_then(|role| match work.bindings.session_binding(project, todo, &role) {
            Ok(binding) => binding.map(|b| b.session),
            Err(err) => {
                tracing::warn!(event = "task_binding_unreadable", project = %project.slug, todo = %todo, error = %err);
                None
            }
        });
    tracing::warn!(event = "task_failed_held", project = %project.slug, todo = %todo, occurrence = %firing.occurrence);
    failures.insert(
        key,
        TaskFailure {
            project: project.slug.clone(),
            todo: todo.clone(),
            session,
            notified: false,
        },
    );
    if let Err(err) = state::write_json(&path, &failures) {
        tracing::warn!(event = "task_failures_unwritable", error = %err);
    }
}

/// Notifies each of `project`'s held task failures once its conversation
/// waits for nobody (`anomaly-notification`, "Routing contract"): a
/// session still `running` or `needs-input` holds the failure in its own
/// thread, and one that cannot be observed is retried next tick. The
/// notification references the session that ran it.
fn notify_task_failures(cx: &DispatchContext<'_>, config: &Configuration, project: &ProjectRef) {
    let path = cx.state.task_failures();
    let mut failures: BTreeMap<String, TaskFailure> = match state::read_json(&path) {
        Ok(failures) => failures.unwrap_or_default(),
        Err(err) => {
            tracing::warn!(event = "task_failures_unreadable", error = %err);
            return;
        }
    };
    let sessions = Sessions::new(config, cx.env);
    let mut changed = false;
    for failure in failures.values_mut() {
        if failure.notified || failure.project != project.slug {
            continue;
        }
        let reference = match &failure.session {
            None => format!("{}/{}", project.slug, failure.todo),
            Some(session) => match sessions.alive(session) {
                Ok(Observation::State(SessionState::Running | SessionState::NeedsInput)) => {
                    tracing::debug!(event = "task_failure_in_thread", todo = %failure.todo, session = %session);
                    continue;
                }
                Ok(Observation::Unobservable(reason)) => {
                    tracing::warn!(event = "task_failure_unobservable", todo = %failure.todo, session = %session, reason);
                    continue;
                }
                Err(err) => {
                    tracing::warn!(event = "task_failure_unobservable", todo = %failure.todo, session = %session, error = %err);
                    continue;
                }
                Ok(Observation::State(_)) => session.to_string(),
            },
        };
        anomaly::notify(config, cx.env, &reference, None);
        failure.notified = true;
        changed = true;
    }
    if changed && let Err(err) = state::write_json(&path, &failures) {
        tracing::warn!(event = "task_failures_unwritable", error = %err);
    }
}
