//! Workflow's implementations (`instrukt_todos::workflow`). Owned by the
//! workflow builder in wave 3. Each function returns `None` while
//! `modules.todos` is off (or, until built, always).

use std::sync::Arc;

use instrukt_core::interfaces::FlavorRoles;
use instrukt_runner::interfaces::WorkflowAssignments;
use instrukt_todos::evidence::WorkEvidence;

use crate::context::Ctx;

/// Core's flavor-role lookup (docs procedure-role validation).
pub(super) fn flavor_roles(_ctx: &Ctx) -> anyhow::Result<Option<Arc<dyn FlavorRoles>>> {
    Ok(None)
}

/// The runner-declared workflow interface, reading work's evidence.
pub(super) fn assignments(
    _ctx: &Ctx,
    _evidence: Option<Arc<dyn WorkEvidence>>,
) -> anyhow::Result<Option<Arc<dyn WorkflowAssignments>>> {
    Ok(None)
}
