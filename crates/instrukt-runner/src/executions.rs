//! Recovery of dispatched executions (`session-continuity`, "Five-state
//! status contract"): a step is done only when the work record carries its
//! evidence. The runner keeps each execution it dispatched in
//! `state/runner/executions.json` until its todo is released again (the
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

use instrukt_core::config::{Configuration, Env};
use instrukt_core::ids::{ProjectSlug, RoleName, SessionRef, TodoSlug};
use serde::{Deserialize, Serialize};

use crate::anomaly;
use crate::interfaces::{PickupCandidate, ProjectRef, WorkflowAssignments};
use crate::sessions::{Observation, SessionState, Sessions};
use crate::state::{self, RunnerState, StateError};

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
        | Observation::State(SessionState::Running | SessionState::NeedsInput) => Step::Wait,
        Observation::State(
            SessionState::EndedOk | SessionState::EndedFailed | SessionState::Gone,
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

    fn load(&self) -> Result<Vec<Execution>, StateError> {
        Ok(state::read_json(&self.state.executions())?.unwrap_or_default())
    }

    fn save(&self, executions: &[Execution]) -> Result<(), StateError> {
        state::write_json(&self.state.executions(), executions)
    }

    /// Starts watching an execution, replacing any for the same todo.
    pub fn record(&self, execution: Execution) -> Result<(), StateError> {
        let mut executions = self.load()?;
        executions.retain(|e| !(e.project == execution.project && e.todo == execution.todo));
        executions.push(execution);
        self.save(&executions)
    }

    /// Observes `project`'s executions. `offered` are the todos work offers
    /// for pickup: an execution whose todo is among them was released.
    pub fn observe(
        &self,
        config: &Configuration,
        env: &Env,
        workflow: &dyn WorkflowAssignments,
        project: &ProjectRef,
        offered: &[PickupCandidate],
    ) -> Result<(), StateError> {
        let mut executions = self.load()?;
        let sessions = Sessions::new(config, env);
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
                Step::Recover => {
                    execution.recovery_sent = true;
                    match sessions.send(&execution.session, RECOVERY_MESSAGE) {
                        Ok(()) => {
                            tracing::info!(event = "execution_recovery_sent", todo = %execution.todo, session = %execution.session);
                        }
                        Err(err) => {
                            tracing::warn!(event = "execution_recovery_undeliverable", todo = %execution.todo, session = %execution.session, error = %err);
                            execution.anomaly_raised = true;
                            anomaly::notify(config, env, &execution.session.to_string(), None);
                        }
                    }
                }
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
        let ended = Observation::State(SessionState::EndedOk);
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
