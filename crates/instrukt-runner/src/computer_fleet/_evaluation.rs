//! Hosting the evaluation kind's answering
//! (`project/instrukt/spec/domain/runner`, "Fleet query and membership"):
//! wherever a cluster connection is configured, whether or not history is
//! on, `instrukt run` answers on a responder of its own and `instrukt tick`
//! in a step before its wake, and each keeps this computer's membership
//! entry. The answer is work's, obtained through
//! [`EvaluationAnswers`] and returned as work gave it; answering writes no
//! report.

use std::sync::Arc;
use std::time::{Duration, Instant};

use instrukt_core::config::{Configuration, Current, Env};
use instrukt_core::home::Home;
use instrukt_core::machine::{self, MachineId};
use jiff::Timestamp;
use serde_json::{Map, Value};

use super::_failure::{Clock, FailureFile, FailureRecorder, WORKER};
use super::_members;
use super::_query::{EVALUATION, Unopened, open};
use super::_responder::{Answering, PollError, Responder, ResponderSpec, answer_request};
use crate::interfaces::{EvaluationAnswers, RunnerWiring};
use crate::transport::{Endpoint, EventScope, Reachability, TransportError};

/// How often a running responder writes its membership entry again.
const MEMBERSHIP_INTERVAL: Duration = Duration::from_secs(30);

/// The evaluation kind's answering on this computer.
pub(super) struct Evaluating {
    answers: Arc<dyn EvaluationAnswers>,
    /// The membership entry a responder keeps from its poll loop; a tick
    /// writes its entry itself.
    membership: Option<Membership>,
}

/// A responder's membership entry: this computer, the clock its last-seen
/// time is read from, and when it was last written.
struct Membership {
    computer: MachineId,
    clock: Clock,
    kept: Option<Instant>,
}

impl Answering for Evaluating {
    fn answer(&self, payload: Map<String, Value>) -> Result<String, serde_json::Error> {
        serde_json::to_string(&self.answers.answer(Value::Object(payload))?)
    }

    fn answered(&self, elapsed: Duration) {
        tracing::debug!(
            event = "eval_request_answered",
            duration_ms = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
        );
    }

    fn tend(&mut self, endpoint: &Endpoint) -> Result<(), TransportError> {
        if let Some(membership) = &mut self.membership
            && membership
                .kept
                .is_none_or(|kept| kept.elapsed() >= MEMBERSHIP_INTERVAL)
        {
            _members::write(endpoint, &membership.computer, (membership.clock)())?;
            membership.kept = Some(Instant::now());
        }
        Ok(())
    }
}

/// `instrukt tick`'s fleet step, before the rest of its wake: where a
/// cluster connection is configured and work's answering is wired, it sets
/// this computer's membership entry and answers every unexpired evaluation
/// request waiting for it when the step starts; later arrivals wait for the
/// next tick. A failure is recorded in the computer fleet's failure file and
/// ends the step, and the tick goes on; a successful step removes the file,
/// as does having no cluster connection configured.
pub(crate) fn tick_step(
    home: &Home,
    config: &Configuration,
    wiring: &RunnerWiring,
    now: Timestamp,
) {
    let Some(answers) = &wiring.evaluation else {
        return;
    };
    let mut failure = FailureFile::new(home, Arc::new(move || now));
    let fleet = match open(config, home, Reachability::Unrecorded) {
        Ok(fleet) => fleet,
        Err(Unopened::NotConfigured) => {
            failure.record(None);
            return;
        }
        Err(Unopened::Unusable(error)) => {
            tracing::warn!(event = "computer_fleet_unusable", worker = WORKER, %error);
            failure.record(Some(&error.to_string()));
            return;
        }
    };
    let computer = match machine::machine_id(home) {
        Ok(computer) => computer,
        Err(error) => {
            tracing::error!(event = "fleet_step_failed", worker = WORKER, %error);
            failure.record(Some(&error.to_string()));
            return;
        }
    };
    let endpoint = fleet.transport().endpoint(EventScope::Cluster);
    let evaluating = Evaluating {
        answers: Arc::clone(answers),
        membership: None,
    };
    let step = || -> Result<(), TransportError> {
        _members::write(endpoint, &computer, now)?;
        let queue = EVALUATION.queue(&computer);
        let waiting: usize = endpoint.command(redis::cmd("LLEN").arg(&queue))?;
        for _ in 0..waiting {
            let Some(body) = endpoint.command::<Option<String>>(redis::cmd("LPOP").arg(&queue))?
            else {
                break;
            };
            match answer_request(EVALUATION, endpoint, &evaluating, &body) {
                Ok(()) | Err(PollError::Off) => {}
                Err(PollError::Transport(error)) => return Err(error),
                Err(PollError::Invalid(reason)) => {
                    tracing::error!(event = "fleet_request_unanswered", worker = WORKER, %reason);
                }
            }
        }
        Ok(())
    };
    match step() {
        Ok(()) => {
            failure.record(None);
        }
        Err(error) => {
            tracing::warn!(event = "fleet_step_failed", worker = WORKER, %error);
            failure.record(Some(&error.to_string()));
        }
    }
}

/// `instrukt run`'s evaluation responder, started and restarted on its
/// wakes.
#[derive(Default)]
pub(crate) struct EvaluationHost {
    responder: Option<Responder>,
    /// The cluster transport settings the responder was started on.
    settings: Option<serde_yaml_ng::Value>,
}

impl EvaluationHost {
    /// Starts the responder where a cluster connection is configured and
    /// work's answering is wired, independently of `modules.history`; starts
    /// it again when the cluster settings change or its thread ended. A
    /// configured connection that cannot be used is recorded in the failure
    /// file and retried on the next wake; with none configured the file is
    /// removed.
    pub(crate) fn refresh(
        &mut self,
        home: &Home,
        env: &Env,
        current: &Current,
        wiring: &RunnerWiring,
        clock: &Clock,
    ) {
        let Some(answers) = &wiring.evaluation else {
            self.responder = None;
            return;
        };
        let admitted = current.get();
        let settings = admitted.global().get(Some("redis"), env).ok();
        if self.responder.is_some() && self.settings != settings {
            tracing::info!(
                event = "fleet_responder_restarted",
                worker = WORKER,
                cause = "transport settings changed"
            );
            self.responder = None;
        }
        if self.responder.as_ref().is_some_and(Responder::ended) {
            tracing::warn!(event = "fleet_responder_restarted", worker = WORKER);
            self.responder = None;
        }
        if self.responder.is_some() {
            return;
        }
        // The host's own record, apart from the responder's: what it knows
        // it recorded starts unknown on each wake.
        let mut failure = FailureFile::new(home, Arc::clone(clock));
        let fleet = match open(admitted.global(), home, Reachability::Unrecorded) {
            Ok(fleet) => fleet,
            Err(Unopened::NotConfigured) => {
                failure.record(None);
                return;
            }
            Err(Unopened::Unusable(error)) => {
                tracing::warn!(event = "computer_fleet_unusable", worker = WORKER, %error);
                failure.record(Some(&error.to_string()));
                return;
            }
        };
        let started = machine::machine_id(home)
            .map_err(super::StartError::from)
            .and_then(|computer| {
                Responder::start(
                    home,
                    ResponderSpec {
                        kind: EVALUATION,
                        worker: WORKER,
                        ready: false,
                    },
                    fleet.transport(),
                    Evaluating {
                        answers: Arc::clone(answers),
                        membership: Some(Membership {
                            computer,
                            clock: Arc::clone(clock),
                            kept: None,
                        }),
                    },
                    FailureFile::new(home, Arc::clone(clock)),
                )
            });
        match started {
            Ok(responder) => {
                self.responder = Some(responder);
                self.settings = settings;
            }
            Err(error) => {
                tracing::error!(event = "fleet_responder_unstarted", worker = WORKER, %error);
                failure.record(Some(&error.to_string()));
            }
        }
    }
}
