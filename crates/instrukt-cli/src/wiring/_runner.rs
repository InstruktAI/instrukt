//! The runner's implementations work consumes (`instrukt_runner`). Owned
//! by the runner builder in wave 3.

use std::sync::Arc;

use instrukt_runner::events::EventLog;

use crate::context::Ctx;

/// The event log in `state/runner/` that work's finalize-ready stamp
/// appends `workflow.finalized` to. `None` until the runner builds it.
pub(super) fn event_log(_ctx: &Ctx) -> anyhow::Result<Option<Arc<dyn EventLog>>> {
    Ok(None)
}
