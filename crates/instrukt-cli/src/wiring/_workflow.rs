//! Workflow's implementations (`instrukt_todos::workflow`). Owned by the
//! workflow builder in wave 3.

use std::sync::Arc;

use instrukt_core::interfaces::FlavorRoles;
use instrukt_runner::interfaces::WorkflowAssignments;
use instrukt_todos::evidence::WorkEvidence;
use instrukt_todos::workflow::{SelectedFlavorRoles, Workflow, todos_enabled};

use crate::context::Ctx;

/// Core's flavor-role lookup (docs procedure-role validation), only while
/// `modules.todos` is on for the caller's project: with it off, no role
/// membership is checked.
pub(super) fn flavor_roles(ctx: &Ctx) -> anyhow::Result<Option<Arc<dyn FlavorRoles>>> {
    if !todos_enabled(&ctx.home, &ctx.cwd)? {
        return Ok(None);
    }
    Ok(Some(available_flavor_roles(ctx)))
}

/// Core's flavor-role lookup whatever the caller's working-tree switch:
/// publication applies it only when the committed configuration it
/// publishes switches `todos` on.
pub(super) fn available_flavor_roles(ctx: &Ctx) -> Arc<dyn FlavorRoles> {
    Arc::new(SelectedFlavorRoles::new(ctx.home.clone()))
}

/// The runner-declared workflow interface, reading work's evidence. The
/// runner checks `modules.todos` per project; without work's evidence
/// there is nothing to derive from.
pub(super) fn assignments(
    ctx: &Ctx,
    evidence: Option<Arc<dyn WorkEvidence>>,
) -> anyhow::Result<Option<Arc<dyn WorkflowAssignments>>> {
    let env = super::environment(ctx)?;
    Ok(evidence.map(|evidence| -> Arc<dyn WorkflowAssignments> {
        Arc::new(Workflow::new(ctx.home.clone(), evidence, env))
    }))
}
