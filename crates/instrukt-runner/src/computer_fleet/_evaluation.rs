//! Hosting the evaluation kind's answering
//! (`project/instrukt/spec/domain/runner`, "Fleet query and membership"):
//! wherever a cluster connection is configured, whether or not history is
//! on, `instrukt run` answers on a responder of its own and keeps this
//! computer's membership entry. The answer is work's, obtained through
//! [`EvaluationAnswers`] and returned as work gave it; answering writes no
//! report.

use std::sync::Arc;
use std::time::{Duration, Instant};

use instrukt_core::config::{Current, Env};
use instrukt_core::home::Home;
use instrukt_core::machine::{self, MachineId};
use jiff::Timestamp;
use serde_json::{Map, Value};

use super::_members;
use super::_query::{EVALUATION, Unopened, open};
use super::_responder::{Answering, Responder, ResponderSpec};
use crate::interfaces::{EvaluationAnswers, RunnerWiring};
use crate::transport::{Endpoint, TransportError};

/// The row the evaluation responder and the tick's fleet step record their
/// failure under in `runner_workers`.
pub(crate) const WORKER: &str = "computer-fleet";

/// How often a running responder writes its membership entry again.
const MEMBERSHIP_INTERVAL: Duration = Duration::from_secs(30);

/// The operator clock the runner reads.
pub(crate) type Clock = Arc<dyn Fn() -> Timestamp + Send + Sync>;

/// The evaluation kind's answering on this computer.
pub(super) struct Evaluating {
    answers: Arc<dyn EvaluationAnswers>,
    computer: MachineId,
    clock: Clock,
    /// When the membership entry was last written by this responder.
    kept: Option<Instant>,
}

impl Evaluating {
    pub(super) fn new(
        answers: Arc<dyn EvaluationAnswers>,
        computer: MachineId,
        clock: Clock,
    ) -> Self {
        Self {
            answers,
            computer,
            clock,
            kept: None,
        }
    }
}

impl Answering for Evaluating {
    fn answer(&self, payload: Map<String, Value>) -> Result<String, serde_json::Error> {
        serde_json::to_string(&self.answers.answer(Value::Object(payload)))
    }

    fn tend(&mut self, endpoint: &Endpoint) -> Result<(), TransportError> {
        if self
            .kept
            .is_none_or(|kept| kept.elapsed() >= MEMBERSHIP_INTERVAL)
        {
            _members::write(endpoint, &self.computer, (self.clock)())?;
            self.kept = Some(Instant::now());
        }
        Ok(())
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
    /// configured connection that cannot be used is recorded and retried on
    /// the next wake; with none configured the row is cleared.
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
        let fleet = match open(admitted.global(), home) {
            Ok(fleet) => fleet,
            Err(Unopened::NotConfigured) => {
                crate::state::worker_state(home, WORKER, None);
                return;
            }
            Err(Unopened::Unusable(error)) => {
                tracing::warn!(event = "computer_fleet_unusable", worker = WORKER, %error);
                crate::state::worker_state(home, WORKER, Some(&error.to_string()));
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
                    Evaluating::new(Arc::clone(answers), computer, Arc::clone(clock)),
                )
            });
        match started {
            Ok(responder) => {
                self.responder = Some(responder);
                self.settings = settings;
            }
            Err(error) => {
                tracing::error!(event = "fleet_responder_unstarted", worker = WORKER, %error);
                crate::state::worker_state(home, WORKER, Some(&error.to_string()));
            }
        }
    }
}
