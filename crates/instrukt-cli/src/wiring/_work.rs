//! Work's implementations (`instrukt_todos::work`). Each function returns
//! `None` while `modules.todos` is off for the invocation's project (the
//! global layer outside a project).

use std::sync::Arc;

use anyhow::Context as _;
use instrukt_core::config::{self, Env, Module};
use instrukt_core::git;
use instrukt_core::interfaces::TodoExistence;
use instrukt_runner::interfaces::WorkPorts;
use instrukt_todos::evidence::WorkEvidence;
use instrukt_todos::work::{Work, WorkError, WorkExistence};

use crate::context::Ctx;

/// Core's todo-existence question (docs pending slugs, repository
/// `Todo:` trailers), for the project the caller stands in.
pub(super) fn todo_existence(ctx: &Ctx) -> anyhow::Result<Option<Arc<dyn TodoExistence>>> {
    if !enabled(ctx)? {
        return Ok(None);
    }
    let Some(root) = git::worktree_root(&ctx.cwd)? else {
        return Ok(None);
    };
    let work = Arc::new(work(ctx));
    let project = match work.project_at(&root) {
        Ok(project) => project,
        // No project slug to ask about: the core context's no-answer case.
        Err(WorkError::NoProject) => return Ok(None),
        Err(err) => return Err(err.into()),
    };
    Ok(Some(Arc::new(WorkExistence::new(work, project))))
}

/// The runner-declared work interfaces.
pub(super) fn ports(ctx: &Ctx) -> anyhow::Result<Option<WorkPorts>> {
    if !enabled(ctx)? {
        return Ok(None);
    }
    let work = Arc::new(work(ctx));
    Ok(Some(WorkPorts {
        pickup: work.clone(),
        claims: work.clone(),
        bindings: work.clone(),
        reflection: work.clone(),
        firings: work.clone(),
        holds: work.clone(),
        retention: work,
    }))
}

/// Work's evidence read contract for workflow.
pub(super) fn evidence(ctx: &Ctx) -> anyhow::Result<Option<Arc<dyn WorkEvidence>>> {
    if !enabled(ctx)? {
        return Ok(None);
    }
    Ok(Some(Arc::new(work(ctx))))
}

/// The work record of this invocation: the process environment supplies
/// credentials and the caller's session, the system clock the time.
pub(crate) fn work(ctx: &Ctx) -> Work {
    Work::new(
        ctx.home.clone(),
        Env::from_process(),
        Arc::new(jiff::Timestamp::now),
    )
}

/// Whether `modules.todos` is on where the caller stands.
fn enabled(ctx: &Ctx) -> anyhow::Result<bool> {
    let root = git::worktree_root(&ctx.cwd)?;
    config::module_enabled(&ctx.home, root.as_deref(), Module::Todos)
        .context("cannot read the todos module switch")
}
