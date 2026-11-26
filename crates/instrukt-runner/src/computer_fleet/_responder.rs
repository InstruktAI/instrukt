//! The answering half of the fleet query: a responder thread per query kind
//! in the long-lived runner, and the one answer path every answering place
//! shares.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread::JoinHandle;
use std::time::Duration;

use instrukt_core::home::Home;
use instrukt_core::machine::{self, MachineId};
use serde_json::{Map, Value};

use super::_failure::FailureRecorder;
use super::_query::{Envelope, Kind, REQUEST_LIFETIME};
use crate::transport::{Endpoint, EventScope, ScopedTransport, TransportError};

/// What a kind supplies to the responder that carries its requests.
pub(crate) trait Answering: Send + 'static {
    /// Whether this computer still answers the kind; `Ok(false)` ends the
    /// responder. A reason it cannot be read is logged and asked again.
    fn enabled(&self) -> Result<bool, String> {
        Ok(true)
    }
    /// Whether the kind's payload fields are of a request it answers.
    fn accepts(&self, _payload: &Map<String, Value>) -> bool {
        true
    }
    /// The answer to a request, as it travels.
    fn answer(&self, payload: Map<String, Value>) -> Result<String, serde_json::Error>;
    /// Work beside answering, run from the poll loop on each poll; a failure
    /// takes the responder's transport-failure path.
    fn tend(&mut self, _endpoint: &Endpoint) -> Result<(), TransportError> {
        Ok(())
    }
    /// A request answered and its answer written, `elapsed` after it was
    /// taken up.
    fn answered(&self, _elapsed: Duration) {}
}

/// How one kind's responder runs.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ResponderSpec {
    pub kind: Kind,
    /// Its name where its failure is recorded and in logs, and its thread's
    /// name.
    pub worker: &'static str,
    /// Whether it publishes the kind's short-lived readiness key on each poll.
    pub ready: bool,
}

/// Why one poll of a responder did not succeed.
pub(crate) enum PollError {
    /// The kind is no longer answered here.
    Off,
    /// The cluster store failed: retried after a growing delay, and
    /// recorded while it lasts.
    Transport(TransportError),
    /// A request that cannot be answered, or a reason that cannot be read;
    /// logged, and the responder goes on.
    Invalid(String),
}

impl From<TransportError> for PollError {
    fn from(error: TransportError) -> Self {
        Self::Transport(error)
    }
}

/// Answers one request `body` of `kind` taken from this computer's queue:
/// an expired request, one that cannot be read, or one naming a reply place
/// not of the kind's form is left unanswered. The answer is written with
/// the request's lifetime.
pub(crate) fn answer_request(
    kind: Kind,
    endpoint: &Endpoint,
    answering: &impl Answering,
    body: &str,
) -> Result<(), PollError> {
    let started = std::time::Instant::now();
    let request: Envelope<Map<String, Value>> = serde_json::from_str(body)
        .map_err(|error| PollError::Invalid(format!("unreadable request: {error}")))?;
    if request.expires < jiff::Timestamp::now().as_second() {
        return Ok(());
    }
    if !kind.is_reply_place(&request.reply) || !answering.accepts(&request.payload) {
        return Err(PollError::Invalid("invalid request".into()));
    }
    if !answering.enabled().map_err(PollError::Invalid)? {
        return Err(PollError::Off);
    }
    let answer = answering
        .answer(request.payload)
        .map_err(|error| PollError::Invalid(format!("unencodable answer: {error}")))?;
    endpoint.run(|conn| {
        redis::pipe()
            .atomic()
            .cmd("RPUSH")
            .arg(&request.reply)
            .arg(answer)
            .ignore()
            .cmd("EXPIRE")
            .arg(&request.reply)
            .arg(REQUEST_LIFETIME.as_secs())
            .ignore()
            .query::<()>(conn)
    })?;
    answering.answered(started.elapsed());
    Ok(())
}

/// Why a responder could not start.
#[derive(Debug, thiserror::Error)]
pub(crate) enum StartError {
    #[error(transparent)]
    Machine(#[from] instrukt_core::machine::MachineError),
    #[error("the {worker} thread could not start: {source}")]
    Thread {
        worker: &'static str,
        source: std::io::Error,
    },
}

/// One kind's responder thread in the long-lived runner. Cluster Redis is
/// retried inside the thread after a growing delay, and its failure is
/// recorded, where its host says, while it lasts.
pub(crate) struct Responder {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl Responder {
    pub(crate) fn start(
        home: &Home,
        spec: ResponderSpec,
        transport: &ScopedTransport,
        mut answering: impl Answering,
        mut recorder: impl FailureRecorder,
    ) -> Result<Self, StartError> {
        let computer = machine::machine_id(home)?;
        let transport = transport.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let ResponderSpec {
            kind,
            worker,
            ready,
        } = spec;
        let handle = std::thread::Builder::new()
            .name(worker.into())
            .spawn(move || {
                let endpoint = transport.endpoint(EventScope::Cluster);
                let queue = kind.queue(&computer);
                let mut failing: Option<String> = None;
                // An earlier process's record is cleared by this worker's first
                // success; a later failure records again.
                let mut shown_healthy = false;
                let mut delay = Duration::from_secs(1);
                while !stopping.load(Ordering::Relaxed) {
                    let outcome = poll(kind, endpoint, &queue, &computer, ready, &mut answering);
                    match outcome {
                        Ok(()) => {
                            if let Some(cause) = failing.take() {
                                tracing::info!(event = "fleet_responder_recovered", worker, cause = %cause);
                            }
                            if !shown_healthy {
                                shown_healthy = recorder.record(None);
                            }
                            delay = Duration::from_secs(1);
                        }
                        Err(PollError::Off) => break,
                        Err(PollError::Transport(error)) => {
                            let cause = error.to_string();
                            shown_healthy = false;
                            if failing.as_deref() != Some(cause.as_str()) {
                                tracing::warn!(event = "fleet_responder_failing", worker, error = %error);
                                if recorder.record(Some(&cause)) {
                                    failing = Some(cause);
                                }
                            }
                            if !stopping.load(Ordering::Relaxed) {
                                std::thread::park_timeout(delay);
                            }
                            delay = (delay * 2).min(Duration::from_secs(60));
                        }
                        Err(PollError::Invalid(reason)) => {
                            tracing::error!(event = "fleet_request_unanswered", worker, %reason);
                        }
                    }
                }
                // A worker that ends leaves no record of an earlier failure.
                recorder.record(None);
            })
            .map_err(|source| StartError::Thread { worker, source })?;
        Ok(Self {
            stop,
            handle: Some(handle),
        })
    }

    /// Whether the responder's thread has ended: its kind switched off here,
    /// or unwound.
    pub(crate) fn ended(&self) -> bool {
        self.handle
            .as_ref()
            .is_none_or(std::thread::JoinHandle::is_finished)
    }
}

/// One poll: the kind's other work, readiness, then a short wait for a
/// request, answered when it comes.
fn poll(
    kind: Kind,
    endpoint: &Endpoint,
    queue: &str,
    computer: &MachineId,
    ready: bool,
    answering: &mut impl Answering,
) -> Result<(), PollError> {
    if !answering.enabled().map_err(PollError::Invalid)? {
        return Err(PollError::Off);
    }
    answering.tend(endpoint)?;
    if ready {
        endpoint.command::<()>(
            redis::cmd("SET")
                .arg(kind.ready(computer))
                .arg("1")
                .arg("EX")
                .arg(5),
        )?;
    }
    let Some((_, body)) =
        endpoint.command::<Option<(String, String)>>(redis::cmd("BLPOP").arg(queue).arg(1))?
    else {
        return Ok(());
    };
    answer_request(kind, endpoint, answering, &body)
}

impl Drop for Responder {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = &self.handle {
            handle.thread().unpark();
        }
        if let Some(handle) = self.handle.take()
            && let Err(error) = handle.join()
        {
            tracing::error!(event = "fleet_responder_panicked", error = ?error);
        }
    }
}
