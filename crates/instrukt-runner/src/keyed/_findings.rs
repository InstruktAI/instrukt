use crate::sessions::SessionsError;
use crate::state::{self, RunnerState};
use jiff::Timestamp;

pub(crate) fn note(
    state: &RunnerState,
    runtime: &str,
    key: &str,
    now: Timestamp,
    outcome: Result<(), &SessionsError>,
) {
    let worker = format!("runtime-start:{runtime}:{key}");
    let failure = outcome.err().map(ToString::to_string);
    if let Err(error) =
        state::record_worker_failure_at(state.home(), &worker, failure.as_deref(), now)
    {
        tracing::warn!(event = "runtime_start_status_failed", %error);
    }
}
