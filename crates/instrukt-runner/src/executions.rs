//! Recovery of dispatched executions (`session-continuity`, "Five-state
//! status contract"): a step is done only when the work record carries its
//! evidence. The runner keeps each execution it dispatched in the state
//! database's `executions` table until its todo is released again (the
//! claim given back, so work offers it once more) or no lane is owed. While
//! the execution lasts, each tick observes it:
//!
//! - the workflow state moved on since the last look: the session is
//!   making progress; its recovery starts afresh;
//! - the session is `running` or `needs-input`: it is working or waiting
//!   in its own thread;
//! - its status cannot be observed: retried next tick, never an anomaly;
//! - it `ended` or is `gone` with no progress: the runner sends that same
//!   session one recovery message; if it ends again, or is gone, still
//!   without progress, the execution is an anomaly, reported once.

use instrukt_core::config::Configuration;
use instrukt_core::ids::{ProjectSlug, RoleName, SessionRef, TodoSlug};
use serde::{Deserialize, Serialize};

use crate::anomaly;
use crate::interfaces::{PickupCandidate, ProjectRef, WorkflowAssignments};
use crate::sessions::{Observation, SessionState, Sessions, SessionsError};
use crate::state::{self, RunnerState, StateError};
use instrukt_core::state_db::rusqlite::params;

/// The message a stalled execution's session receives once.
pub const RECOVERY_MESSAGE: &str = "instrukt: the work this session was started for has no recorded completion. Finish it and record its evidence, or record why it cannot be completed.";

/// One dispatched execution the runner watches.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Execution {
    pub project: ProjectSlug,
    pub todo: TodoSlug,
    pub role: RoleName,
    pub session: SessionRef,
    /// The workflow state at the last look.
    pub state: String,
    #[serde(default)]
    pub recovery_sent: bool,
    #[serde(default)]
    pub anomaly_raised: bool,
}

/// What a tick does for an execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Nothing now: the session works, waits in its thread, or cannot be
    /// observed (retried next tick).
    Wait,
    /// The session moved the work on; recovery starts afresh.
    Progressed,
    /// Send the one recovery message.
    Recover,
    /// Report the anomaly.
    RaiseAnomaly,
    /// Already reported; nothing more.
    Reported,
}

/// The step for an execution whose session was observed as `observed`.
pub fn step(execution: &Execution, progressed: bool, observed: &Observation) -> Step {
    if progressed {
        return Step::Progressed;
    }
    match observed {
        Observation::Unobservable(_)
        | Observation::State(
            SessionState::Running | SessionState::NeedsInput | SessionState::Unknown,
        ) => Step::Wait,
        Observation::State(
            SessionState::InputReady | SessionState::EndedFailed | SessionState::Gone,
        ) => match (execution.recovery_sent, execution.anomaly_raised) {
            (false, _) => Step::Recover,
            (true, false) => Step::RaiseAnomaly,
            (true, true) => Step::Reported,
        },
    }
}

/// The runner's watched executions.
pub struct Executions<'a> {
    state: &'a RunnerState,
}

impl<'a> Executions<'a> {
    pub fn new(state: &'a RunnerState) -> Self {
        Self { state }
    }

    /// Every watched execution whose todo is still in the work record.
    fn load(&self) -> Result<Vec<Execution>, StateError> {
        let loaded = self.state.read(|conn, path| {
            let sql = state::sql(path);
            let mut statement = conn
                .prepare(
                    "SELECT e.project, t.slug, e.role, e.session, e.workflow_state,
                        e.recovery_sent, e.anomaly_raised
                     FROM executions e JOIN todos t ON t.id = e.todo_id
                     ORDER BY e.rowid",
                )
                .map_err(&sql)?;
            let rows = statement
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, bool>(5)?,
                        row.get::<_, bool>(6)?,
                    ))
                })
                .map_err(&sql)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(&sql)?;
            rows.into_iter()
                .map(
                    |(
                        project,
                        todo,
                        role,
                        session,
                        workflow_state,
                        recovery_sent,
                        anomaly_raised,
                    )| {
                        Ok(Execution {
                            project: ProjectSlug::parse(&project)
                                .map_err(|e| state::invalid(path, e))?,
                            todo: TodoSlug::parse(&todo).map_err(|e| state::invalid(path, e))?,
                            role: RoleName::parse(&role).map_err(|e| state::invalid(path, e))?,
                            session: SessionRef::parse(&session)
                                .map_err(|e| state::invalid(path, e))?,
                            state: workflow_state,
                            recovery_sent,
                            anomaly_raised,
                        })
                    },
                )
                .collect::<Result<Vec<_>, StateError>>()
        })?;
        Ok(loaded.unwrap_or_default())
    }

    /// Replaces every watched execution with `executions`, in one
    /// transaction.
    fn save(&self, executions: &[Execution]) -> Result<(), StateError> {
        self.state.write(|tx| {
            let path = state::db_path(tx);
            tx.execute("DELETE FROM executions", [])
                .map_err(state::sql(&path))?;
            for execution in executions {
                insert(tx, execution)?;
            }
            Ok(())
        })
    }

    /// Starts watching an execution, replacing any for the same todo.
    pub fn record(&self, execution: Execution) -> Result<(), StateError> {
        self.state.write(|tx| {
            let path = state::db_path(tx);
            let Some(id) = state::todo_id(tx, execution.project.as_str(), execution.todo.as_str())?
            else {
                return Err(state::invalid(
                    &path,
                    format!(
                        "no local todo `{}` to watch an execution of",
                        execution.todo
                    ),
                ));
            };
            tx.execute(
                "DELETE FROM executions WHERE project = ?1 AND todo_id = ?2",
                params![execution.project.as_str(), id],
            )
            .map_err(state::sql(&path))?;
            insert(tx, &execution)
        })
    }

    /// Observes `project`'s executions. `offered` are the todos work offers
    /// for pickup: an execution whose todo is among them was released.
    pub fn observe(
        &self,
        cx: &crate::dispatch::DispatchContext<'_>,
        config: &Configuration,
        workflow: &dyn WorkflowAssignments,
        project: &ProjectRef,
        offered: &[PickupCandidate],
    ) -> Result<(), StateError> {
        let mut executions = self.load()?;
        let env = cx.env;
        let sessions = Sessions::new(self.state.home(), config, env);
        let mut kept = Vec::with_capacity(executions.len());
        for mut execution in executions.drain(..) {
            if execution.project != project.slug {
                kept.push(execution);
                continue;
            }
            if offered.iter().any(|c| c.todo == execution.todo) {
                tracing::debug!(event = "execution_released", todo = %execution.todo, session = %execution.session);
                continue;
            }
            let current = match workflow.assignment(project, &execution.todo) {
                Ok(Some(assignment)) => assignment.state,
                Ok(None) => {
                    tracing::debug!(event = "execution_finished", todo = %execution.todo, session = %execution.session);
                    continue;
                }
                Err(err) => {
                    tracing::warn!(event = "execution_unassessable", todo = %execution.todo, error = %err);
                    kept.push(execution);
                    continue;
                }
            };
            let progressed = current != execution.state;
            let observed = if progressed {
                Observation::Unobservable(String::new())
            } else {
                sessions
                    .alive(&execution.session)
                    .unwrap_or_else(|err| Observation::Unobservable(err.to_string()))
            };
            match step(&execution, progressed, &observed) {
                Step::Wait | Step::Reported => {}
                Step::Progressed => {
                    execution.state = current;
                    execution.recovery_sent = false;
                    execution.anomaly_raised = false;
                }
                Step::Recover => match sessions.send(
                    cx.invocation,
                    &execution.session,
                    &crate::inputs::Entry {
                        id: &format!("recovery:{}:{}", execution.session, execution.state),
                        kind: crate::inputs::InputKind::Recovery,
                        assignment: None,
                        source_at: Some(&cx.now.to_string()),
                        arrived_at: cx.now,
                        intent: crate::inputs::Intent::Asap,
                        content: RECOVERY_MESSAGE,
                    },
                ) {
                    Ok(crate::inputs::InputOutcome::Pending) => {
                        tracing::debug!(event="execution_recovery_pending",session=%execution.session);
                    }
                    Ok(
                        crate::inputs::InputOutcome::Accepted
                        | crate::inputs::InputOutcome::Entered,
                    ) => {
                        execution.recovery_sent = true;
                        tracing::info!(event = "execution_recovery_sent", todo = %execution.todo, session = %execution.session);
                    }
                    Err(err @ SessionsError::Gone(_)) => {
                        tracing::warn!(event = "execution_recovery_undeliverable", todo = %execution.todo, session = %execution.session, error = %err);
                        execution.recovery_sent = true;
                        execution.anomaly_raised = true;
                        anomaly::notify(config, env, &execution.session.to_string(), None);
                    }
                    Err(err) => {
                        if observed == Observation::State(SessionState::Gone) {
                            execution.recovery_sent = true;
                            execution.anomaly_raised = true;
                            tracing::warn!(event = "execution_recovery_undeliverable", todo = %execution.todo, session = %execution.session, error = %err);
                            anomaly::notify(config, env, &execution.session.to_string(), None);
                        } else {
                            tracing::warn!(event = "execution_recovery_pending", todo = %execution.todo, session = %execution.session, error = %err);
                        }
                    }
                },
                Step::RaiseAnomaly => {
                    execution.anomaly_raised = true;
                    tracing::warn!(event = "execution_incomplete", todo = %execution.todo, session = %execution.session);
                    anomaly::notify(config, env, &execution.session.to_string(), None);
                }
            }
            kept.push(execution);
        }
        self.save(&kept)
    }
}

/// Writes `execution`'s row; one whose todo left the work record is
/// released and not kept.
fn insert(
    tx: &instrukt_core::state_db::rusqlite::Connection,
    execution: &Execution,
) -> Result<(), StateError> {
    let path = state::db_path(tx);
    let Some(id) = state::todo_id(tx, execution.project.as_str(), execution.todo.as_str())? else {
        return Ok(());
    };
    tx.execute(
        "INSERT OR REPLACE INTO executions (project, todo_id, role, session, workflow_state,
            recovery_sent, anomaly_raised)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            execution.project.as_str(),
            id,
            execution.role.as_str(),
            execution.session.to_string(),
            execution.state,
            execution.recovery_sent,
            execution.anomaly_raised,
        ],
    )
    .map_err(state::sql(&path))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn execution(recovery_sent: bool, anomaly_raised: bool) -> Execution {
        Execution {
            project: ProjectSlug::parse("demo").unwrap(),
            todo: TodoSlug::parse("weekly").unwrap(),
            role: RoleName::parse("task-runner").unwrap(),
            session: SessionRef::parse("rt:s1").unwrap(),
            state: "running".into(),
            recovery_sent,
            anomaly_raised,
        }
    }

    #[test]
    fn an_ended_session_without_progress_is_sent_once_then_reported_once() {
        let ended = Observation::State(SessionState::InputReady);
        assert_eq!(step(&execution(false, false), false, &ended), Step::Recover);
        assert_eq!(
            step(&execution(true, false), false, &ended),
            Step::RaiseAnomaly
        );
        assert_eq!(step(&execution(true, true), false, &ended), Step::Reported);
        let gone = Observation::State(SessionState::Gone);
        assert_eq!(step(&execution(false, false), false, &gone), Step::Recover);
    }

    #[test]
    fn working_waiting_unobservable_or_progressing_sessions_are_left_alone() {
        for observed in [
            Observation::State(SessionState::Running),
            Observation::State(SessionState::NeedsInput),
            Observation::Unobservable("timed out".into()),
        ] {
            assert_eq!(step(&execution(true, false), false, &observed), Step::Wait);
        }
        let ended = Observation::State(SessionState::EndedFailed);
        assert_eq!(step(&execution(true, true), true, &ended), Step::Progressed);
    }
}
