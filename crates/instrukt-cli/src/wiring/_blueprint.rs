//! Blueprint's implementation (`instrukt_blueprint`). Owned by the
//! blueprint builder in wave 3. Returns `None` while `modules.blueprint` is
//! off (or, until built, always).

use std::sync::Arc;

use instrukt_core::interfaces::GraphGeneration;

use crate::context::Ctx;

/// Graph generation that repository's hook runs start for a revision of
/// main.
pub(super) fn graph_generation(_ctx: &Ctx) -> anyhow::Result<Option<Arc<dyn GraphGeneration>>> {
    Ok(None)
}
