//! Recovery of dispatched executions (`session-continuity`, "Five-state
//! status contract"): a step is done only when the work record carries its
//! evidence. The runner keeps each execution it dispatched in the state
//! database's `executions` table while its session holds the todo's
//! execution claim: the execution ends when work offers the todo for
//! pickup again, when no lane is owed, or when workflow's assignment names
//! another holder or none. While the execution lasts, each tick observes
//! it:
//!
//! - the workflow state moved on since the last look: the session is
//!   making progress; its recovery starts afresh;
//! - the todo waits on a person (a hold, or an approval gate that waits for
//!   a human): the next evidence is not the session's to produce, so this
//!   observation sends no recovery input and reports no stopped execution;
//!   a `Gone` session with an eligible keyed assignment still takes the
//!   succession below, which is decided first;
//! - the session is `running` or `needs-input`: it is working or waiting
//!   in its own thread;
//! - its status cannot be observed: retried next tick, never an anomaly;
//! - a present thread ended without progress: send one recovery message; a
//!   repeated ending without evidence is an anomaly;
//! - native terminal presence with an eligible keyed assignment: the common
//!   binding owner starts a successor and moves the work claim atomically.

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

/// What a tick read about an execution whose session still holds its
/// todo's execution claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Look {
    /// The workflow state moved on since the last look.
    pub progressed: bool,
    /// The todo waits on a person, as workflow's assignment relays it.
    pub waits_on_person: bool,
    /// The recovery input sent for the current state ended without being
    /// confirmed.
    pub unconfirmed: bool,
    /// The session as the runtime reports it.
    pub observed: Observation,
}

/// The step for `execution` after the tick read `look`: progress first;
/// then a todo waiting on a person is left alone, whatever its thread
/// shows or an earlier recovery input became; then an unconfirmed
/// recovery input is reported; then the session's state decides.
pub fn step(execution: &Execution, look: &Look) -> Step {
    if look.progressed {
        return Step::Progressed;
    }
    if look.waits_on_person {
        return Step::Wait;
    }
    if look.unconfirmed && !execution.anomaly_raised {
        return Step::RaiseAnomaly;
    }
    match &look.observed {
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

/// Move observation to the confirmed successor in the replacement transaction.
pub fn replace_session(
    tx: &instrukt_core::state_db::rusqlite::Transaction<'_>,
    project: &instrukt_core::ids::ProjectSlug,
    todo_id: &str,
    role: &instrukt_core::ids::RoleName,
    stopped: &instrukt_core::ids::SessionRef,
    successor: &instrukt_core::ids::SessionRef,
) -> Result<(), StateError> {
    tx.execute(
        "UPDATE executions SET session=?5,recovery_sent=0,anomaly_raised=0
         WHERE project=?1 AND todo_id=?2 AND role=?3 AND session=?4",
        instrukt_core::state_db::rusqlite::params![
            project.as_str(),
            todo_id,
            role.as_str(),
            stopped.to_string(),
            successor.to_string()
        ],
    )
    .map_err(crate::state::sql(&crate::state::db_path(tx)))?;
    Ok(())
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

    pub(crate) fn stopped(&self) -> Result<Vec<Execution>, StateError> {
        Ok(self
            .load()?
            .into_iter()
            .filter(|execution| execution.anomaly_raised)
            .collect())
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
        bindings: &dyn crate::interfaces::SessionBindings,
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
            let assignment = match workflow.assignment(project, &execution.todo) {
                Ok(Some(assignment)) => assignment,
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
            if assignment.holder.as_ref() != Some(&execution.session) {
                tracing::debug!(event = "execution_released", todo = %execution.todo, session = %execution.session);
                continue;
            }
            let current = assignment.state;
            let progressed = current != execution.state;
            let observed = if progressed {
                Observation::Unobservable(String::new())
            } else {
                sessions
                    .alive(&execution.session)
                    .unwrap_or_else(|err| Observation::Unobservable(err.to_string()))
            };
            let key=self.state.read(|conn,path| {
                use instrukt_core::state_db::rusqlite::OptionalExtension;
                conn.query_row("SELECT semantic_key FROM session_bindings WHERE session=?1 AND released_at IS NULL",[execution.session.to_string()],|row|row.get::<_,String>(0)).optional().map_err(state::sql(path))
            })?.flatten();
            // Besides a confirmed gone thread, an idle one in another runtime
            // project than its checkout's own is succeeded.
            let succeed = observed == Observation::State(SessionState::Gone)
                || (matches!(
                    observed,
                    Observation::State(SessionState::InputReady | SessionState::EndedFailed)
                ) && key.as_ref().is_some_and(|key| {
                    sessions.adapter().is_ok_and(|adapter| {
                        crate::keyed::request(self.state.home(), adapter.as_ref(), key)
                            .and_then(|request| {
                                let thread = adapter.read_state(&execution.session)?;
                                crate::keyed::idle_misplaced(
                                    adapter.as_ref(),
                                    &request.checkout,
                                    &thread,
                                    Some(key),
                                )
                            })
                            .unwrap_or_else(|error| {
                                tracing::warn!(event="execution_placement_unverified",%key,%error);
                                false
                            })
                    })
                }));
            if succeed && let Some(key) = key {
                match sessions.adapter().and_then(|adapter| {
                    let mut request =
                        crate::keyed::request(self.state.home(), adapter.as_ref(), &key)?;
                    request.title = if key.starts_with("instrukt:task:") {
                        format!("task · {}", execution.todo)
                    } else {
                        format!("{} · {}", execution.role, execution.todo)
                    };
                    crate::keyed::succeed(
                        adapter.as_ref(),
                        &crate::runtime::FollowupAssignment {
                            bindings: Some(bindings),
                            original_request: None,
                            invocation: cx.invocation,
                            request: &request,
                            session: &execution.session,
                            worktree_path: None,
                            project: Some(project.slug.as_str()),
                            now: cx.now,
                        },
                    )
                }) {
                    Ok(thread) => {
                        execution.session = thread.session;
                        execution.recovery_sent = false;
                        execution.anomaly_raised = false;
                    }
                    Err(error) => {
                        tracing::warn!(event="execution_successor_pending",%key,%error);
                    }
                }
                kept.push(execution);
                continue;
            }
            let recovery_id = format!("recovery:{}:{}", execution.session, execution.state);
            // The recovery input's outcome is read only where it can decide
            // the step: progress and a person wait come first, and an
            // execution already reported is not reported again.
            let unconfirmed = !progressed
                && !assignment.waits_on_person
                && !execution.anomaly_raised
                && matches!(
                    crate::inputs::outcome(self.state, &execution.session, &recovery_id)?,
                    crate::inputs::InputOutcome::Unconfirmed
                        | crate::inputs::InputOutcome::SetAside
                );
            let look = Look {
                progressed,
                waits_on_person: assignment.waits_on_person,
                unconfirmed,
                observed,
            };
            let next = step(&execution, &look);
            match next {
                Step::Wait | Step::Reported => {}
                Step::Progressed => {
                    execution.state = current;
                    execution.recovery_sent = false;
                    execution.anomaly_raised = false;
                }
                Step::Recover => match sessions.send(
                    cx.invocation,
                    Some(bindings),
                    &execution.session,
                    &crate::inputs::Entry {
                        id: &recovery_id,
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
                    Ok(
                        crate::inputs::InputOutcome::Unconfirmed
                        | crate::inputs::InputOutcome::SetAside,
                    ) => {
                        execution.recovery_sent = true;
                        execution.anomaly_raised = true;
                        tracing::warn!(event="execution_incomplete", todo=%execution.todo, session=%execution.session);
                        anomaly::notify(config, env, &execution.session.to_string(), None);
                    }
                    Err(err @ SessionsError::Gone(_)) => {
                        tracing::warn!(event = "execution_recovery_undeliverable", todo = %execution.todo, session = %execution.session, error = %err);
                        execution.recovery_sent = true;
                        execution.anomaly_raised = true;
                        anomaly::notify(config, env, &execution.session.to_string(), None);
                    }
                    Err(err) => {
                        if look.observed == Observation::State(SessionState::Gone) {
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
                    execution.recovery_sent = true;
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

    /// A look with no progress, no person wait and no unconfirmed input.
    fn seen(observed: Observation) -> Look {
        Look {
            progressed: false,
            waits_on_person: false,
            unconfirmed: false,
            observed,
        }
    }

    #[test]
    fn an_ended_session_without_progress_is_sent_once_then_reported_once() {
        let ended = seen(Observation::State(SessionState::InputReady));
        assert_eq!(step(&execution(false, false), &ended), Step::Recover);
        assert_eq!(step(&execution(true, false), &ended), Step::RaiseAnomaly);
        assert_eq!(step(&execution(true, true), &ended), Step::Reported);
        let gone = seen(Observation::State(SessionState::Gone));
        assert_eq!(step(&execution(false, false), &gone), Step::Recover);
    }

    #[test]
    fn working_waiting_unobservable_or_progressing_sessions_are_left_alone() {
        for observed in [
            Observation::State(SessionState::Running),
            Observation::State(SessionState::NeedsInput),
            Observation::Unobservable("timed out".into()),
        ] {
            assert_eq!(step(&execution(true, false), &seen(observed)), Step::Wait);
        }
        let progressed = Look {
            progressed: true,
            ..seen(Observation::State(SessionState::EndedFailed))
        };
        assert_eq!(step(&execution(true, true), &progressed), Step::Progressed);
    }

    #[test]
    fn an_ended_session_whose_todo_waits_on_a_person_is_left_alone() {
        let waiting = Look {
            waits_on_person: true,
            ..seen(Observation::State(SessionState::InputReady))
        };
        assert_eq!(step(&execution(false, false), &waiting), Step::Wait);
        assert_eq!(step(&execution(true, false), &waiting), Step::Wait);
    }

    #[test]
    fn an_unconfirmed_recovery_input_is_reported_only_outside_a_person_wait() {
        let unconfirmed = Look {
            unconfirmed: true,
            ..seen(Observation::State(SessionState::Running))
        };
        assert_eq!(
            step(&execution(true, false), &unconfirmed),
            Step::RaiseAnomaly
        );
        let waiting = Look {
            waits_on_person: true,
            ..unconfirmed
        };
        assert_eq!(step(&execution(true, false), &waiting), Step::Wait);
    }
}
