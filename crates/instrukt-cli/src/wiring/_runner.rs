//! The runner's implementations work consumes (`instrukt_runner`). Owned
//! by the runner builder in wave 3.

use std::sync::Arc;

use instrukt_runner::event_log::DbEventLog;
use instrukt_runner::events::EventLog;
use instrukt_runner::state::RunnerState;

use crate::context::Ctx;

/// The event log in the state database that work's finalize-ready stamp
/// appends `workflow.finalized` to, in the stamp's own transaction. Always wired: the event lands in the
/// log whatever `modules.events` says; the switch governs only whether a
/// standing agent reacts to it.
pub(super) fn event_log(ctx: &Ctx) -> anyhow::Result<Option<Arc<dyn EventLog>>> {
    Ok(Some(Arc::new(DbEventLog::new(
        RunnerState::new(&ctx.home),
        ctx.clock.now(),
    ))))
}
