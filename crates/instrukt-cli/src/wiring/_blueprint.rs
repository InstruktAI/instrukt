//! Blueprint's implementation (`instrukt_blueprint`). Returns `None` while
//! `modules.blueprint` is off.

use std::sync::Arc;

use anyhow::Context as _;
use instrukt_blueprint::Generation;
use instrukt_core::config::{self, Module};
use instrukt_core::git;
use instrukt_core::interfaces::GraphGeneration;

use crate::context::Ctx;

/// Graph generation that repository's hook runs start for a revision of
/// main: queues it and starts the detached worker, which runs as this
/// executable.
pub(super) fn graph_generation(ctx: &Ctx) -> anyhow::Result<Option<Arc<dyn GraphGeneration>>> {
    let root = git::worktree_root(&ctx.cwd)?;
    if !config::module_enabled(&ctx.home, root.as_deref(), Module::Blueprint)
        .context("cannot read the blueprint module switch")?
    {
        return Ok(None);
    }
    let exe = std::env::current_exe().context("cannot locate the instrukt executable")?;
    Ok(Some(Arc::new(Generation::new(ctx.home.clone(), exe))))
}
