use crate::sessions::SessionsError;
use crate::state::{self, RunnerState, StateError};
use instrukt_core::config::Runtime;
use instrukt_core::state_db::rusqlite::params;
use jiff::Timestamp;
pub(crate) fn note(
    state: &RunnerState,
    runtime: Runtime,
    key: &str,
    now: Timestamp,
    outcome: Result<(), &SessionsError>,
) {
    let result=state.write::<_,StateError>(|tx| {
  let worker=format!("runtime-start:{}:{key}",runtime.name());
  match outcome {
   Ok(())=>tx.execute("DELETE FROM runner_workers WHERE worker=?1",[worker]),
   Err(error)=>tx.execute("INSERT INTO runner_workers(worker,failure,since) VALUES(?1,?2,?3) ON CONFLICT(worker) DO UPDATE SET failure=excluded.failure",params![worker,error.to_string(),now.to_string()])
  }.map_err(state::sql(&state::db_path(tx)))?;Ok(())
 });
    if let Err(error) = result {
        tracing::warn!(event="runtime_start_status_failed",%error);
    }
}
