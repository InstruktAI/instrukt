//! Work's implementations (`instrukt_todos::work`). Owned by the work
//! builder in wave 3. Each function returns `None` while `modules.todos`
//! is off for the invocation's project (or, until built, always).

use std::sync::Arc;

use instrukt_core::interfaces::TodoExistence;
use instrukt_runner::interfaces::WorkPorts;
use instrukt_todos::evidence::WorkEvidence;

use crate::context::Ctx;

/// Core's todo-existence question (docs pending slugs, repository
/// `Todo:` trailers).
pub(super) fn todo_existence(_ctx: &Ctx) -> anyhow::Result<Option<Arc<dyn TodoExistence>>> {
    Ok(None)
}

/// The runner-declared work interfaces.
pub(super) fn ports(_ctx: &Ctx) -> anyhow::Result<Option<WorkPorts>> {
    Ok(None)
}

/// Work's evidence read contract for workflow.
pub(super) fn evidence(_ctx: &Ctx) -> anyhow::Result<Option<Arc<dyn WorkEvidence>>> {
    Ok(None)
}
