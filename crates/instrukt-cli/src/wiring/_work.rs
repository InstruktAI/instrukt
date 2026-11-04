//! Work's implementations (`instrukt_todos::work`). Core's todo-existence
//! question is answered only while `modules.todos` is on for the caller's
//! project; the runner's ports and the evidence read are always wired, and
//! the runner gates each project by its own effective switch.

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
    available_todo_existence(ctx)
}

/// Core's todo-existence question whatever the caller's working-tree
/// switch: publication applies it only when the committed configuration
/// it publishes switches `todos` on.
pub(super) fn available_todo_existence(
    ctx: &Ctx,
) -> anyhow::Result<Option<Arc<dyn TodoExistence>>> {
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

/// The runner-declared work interfaces. Wired whatever the caller's own
/// switch: a scheduled tick runs from `$HOME`, and the runner considers
/// each project by its own effective `modules.todos` (`work-dispatch`,
/// "Execution contract"). The `todo` namespace keeps its own refusal.
pub(super) fn ports(ctx: &Ctx) -> anyhow::Result<Option<WorkPorts>> {
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

/// Work's evidence read contract for workflow, wired as the ports are: the
/// runner's workflow reads it per project.
pub(super) fn evidence(ctx: &Ctx) -> anyhow::Result<Option<Arc<dyn WorkEvidence>>> {
    Ok(Some(Arc::new(work(ctx))))
}

/// The work record of this invocation: the process environment supplies
/// credentials and the caller's session, the system clock the time.
pub(crate) fn work(ctx: &Ctx) -> Work {
    Work::new(ctx.home.clone(), Env::from_process(), ctx.clock.shared())
}

/// Whether `modules.todos` is on where the caller stands.
fn enabled(ctx: &Ctx) -> anyhow::Result<bool> {
    let root = git::worktree_root(&ctx.cwd)?;
    config::module_enabled(&ctx.home, root.as_deref(), Module::Todos)
        .context("cannot read the todos module switch")
}
