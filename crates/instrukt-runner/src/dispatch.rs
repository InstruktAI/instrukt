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

use std::path::Path;
use std::time::Duration;

use instrukt_core::config::{Configuration, Env};
use instrukt_core::home::Home;
use instrukt_core::ids::{RoleName, TodoSlug};
use instrukt_core::index::{GlobalIndex, IndexEntry};
use jiff::Timestamp;
use jiff::tz::TimeZone;

use crate::agents::{agent_order, global_order};
use crate::anomaly;
use crate::interfaces::{
    ClaimOutcome, Claimant, Lane, PickupCandidate, ProjectRef, RunnerWiring, SessionBinding,
    TodoKind, WorkPorts, WorkflowAssignments,
};
use crate::launch;
use crate::process::Invocation;
use crate::sessions::{Sessions, SessionsError, StartRequest};
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
    /// The `instrukt` binary, for work's command contract.
    pub executable: &'a Path,
}

/// What dispatch did with one todo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Dispatched {
    Started { todo: TodoSlug },
    ClaimedElsewhere { todo: TodoSlug },
    ClaimPending { todo: TodoSlug },
    Exhausted { todo: TodoSlug },
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
    match work.reflection.sync_reflection(project) {
        Ok(report) => {
            tracing::debug!(event = "reflection_synced", project = %project.slug, pending = report.pending.len(), conflicted = report.conflicted.len())
        }
        Err(err) => {
            tracing::warn!(event = "reflection_failed", project = %project.slug, error = %err)
        }
    }
    match work.retention.purge_expired(project) {
        Ok(purged) if !purged.is_empty() => {
            tracing::info!(event = "retained_records_purged", project = %project.slug, count = purged.len())
        }
        Ok(_) => {}
        Err(err) => {
            tracing::warn!(event = "retention_purge_failed", project = %project.slug, error = %err)
        }
    }
    let kinds = autostart_kinds(config);
    if kinds.is_empty() {
        return Vec::new();
    }
    let candidates = match work.pickup.pickup_candidates(project, &kinds) {
        Ok(candidates) => candidates,
        Err(err) => {
            tracing::warn!(event = "pickup_unreadable", project = %project.slug, error = %err);
            return Vec::new();
        }
    };
    let index = match GlobalIndex::load(cx.home) {
        Ok(index) => index.unwrap_or_default(),
        Err(err) => {
            tracing::warn!(event = "index_unreadable", error = %err);
            return Vec::new();
        }
    };
    candidates
        .iter()
        .filter_map(|candidate| {
            dispatch_one(
                cx,
                config,
                work,
                workflow.as_ref(),
                &index,
                project,
                candidate,
            )
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
        tracing::warn!(event = "no_procedure_for_role", project = %project.slug, todo = %todo, role = %role);
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
    let prepared = match launch::prepare(procedure, Some(todo.as_str()), None) {
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
            release_claim(cx, project, todo);
            anomaly::notify(config, cx.env, &format!("{}/{todo}", project.slug), None);
            return Some(Dispatched::Exhausted { todo: todo.clone() });
        }
        Err(err) => {
            tracing::warn!(event = "dispatch_start_failed", project = %project.slug, todo = %todo, error = %err);
            return None;
        }
    };
    let binding = SessionBinding {
        session: started.session.clone(),
        worktree: started.worktree,
    };
    let bind_role = prepared.role.unwrap_or(role);
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
    tracing::info!(event = "todo_dispatched", project = %project.slug, todo = %todo, session = %started.session);
    Some(Dispatched::Started { todo: todo.clone() })
}

/// The procedure declaring `role`, preferring the selected flavor's domain.
pub fn procedure_for_role<'i>(
    index: &'i GlobalIndex,
    config: &Configuration,
    role: &RoleName,
) -> Option<&'i IndexEntry> {
    let domain = config.workflow_domain().ok();
    let declaring = || {
        index.snippets.iter().filter(|entry| {
            entry.taxonomy == "procedure" && entry.role.as_deref() == Some(role.as_str())
        })
    };
    declaring()
        .find(|entry| domain.as_ref().is_some_and(|d| entry.domain == d.as_str()))
        .or_else(|| declaring().next())
}

/// Releases the runner's claim with `instrukt todo update <slug> --unclaim`.
fn release_claim(cx: &DispatchContext<'_>, project: &ProjectRef, todo: &TodoSlug) {
    let released = Invocation::program(
        &cx.executable.display().to_string(),
        &["todo", "update", todo.as_str(), "--unclaim"],
    )
    .cwd(&project.root)
    .run(Duration::from_secs(120));
    match released {
        Ok(done) if done.success => {
            tracing::info!(event = "runner_claim_released", project = %project.slug, todo = %todo);
        }
        Ok(done) => {
            tracing::warn!(event = "runner_claim_release_refused", project = %project.slug, todo = %todo, stderr = %done.stderr.trim());
        }
        Err(err) => {
            tracing::warn!(event = "runner_claim_release_failed", project = %project.slug, todo = %todo, error = %err);
        }
    }
}
