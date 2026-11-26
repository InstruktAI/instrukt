//! Work's implementations (`instrukt_todos::work`). Core's todo-existence
//! question is answered only while `modules.todos` is on for the caller's
//! project; the runner's ports and the evidence read are always wired, and
//! the runner gates each project by its own effective switch.

use std::sync::Arc;

use anyhow::Context as _;
use instrukt_core::config::{self, Module};
use instrukt_core::git;
use instrukt_core::interfaces::{CommitExemption, InterfaceError, TodoExistence};
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
    let work = Arc::new(work(ctx)?);
    let project = match work.project_at(&root) {
        Ok(project) => project,
        // No project slug to ask about: the core context's no-answer case.
        Err(WorkError::NoProject) => return Ok(None),
        Err(err) => return Err(err.into()),
    };
    Ok(Some(Arc::new(WorkExistence::new(work, project))))
}

/// Work's admission of a commit's exemption, for the repository's commit
/// command: wired whenever the caller stands in an adopted project, whatever
/// its todos switch, as `instrukt review exempt` is.
pub(super) fn commit_exemption(ctx: &Ctx) -> anyhow::Result<Option<Arc<dyn CommitExemption>>> {
    let Some(root) = git::worktree_root(&ctx.cwd)? else {
        return Ok(None);
    };
    let Some(slug) = config::Configuration::load(&ctx.home, Some(&root))?.project()? else {
        return Ok(None);
    };
    Ok(Some(Arc::new(ReviewExemption {
        home: ctx.home.clone(),
        project: instrukt_todos::work::review::project(slug, &root),
        clock: ctx.clock.clone(),
    })))
}

/// The commit command's exemption, admitted and recorded by
/// `instrukt_todos::work::review::exempt` exactly as `instrukt review
/// exempt` records it.
struct ReviewExemption {
    home: instrukt_core::home::Home,
    project: instrukt_todos::work::review::ReviewProject,
    clock: crate::clock::Clock,
}

impl CommitExemption for ReviewExemption {
    fn exempt(
        &self,
        commit: &instrukt_core::ids::CommitSha,
        basis: &str,
        caller: Option<&instrukt_core::ids::SessionRef>,
    ) -> Result<(), InterfaceError> {
        let request = instrukt_todos::work::review::ExemptRequest {
            commit: commit.as_str().to_owned(),
            basis: basis.to_owned(),
            caller: caller.cloned(),
        };
        instrukt_todos::work::review::exempt(&self.home, &self.project, &request, &|| {
            self.clock.now()
        })
        .map(|_| ())
        .map_err(|error| InterfaceError {
            module: "todos",
            message: error.to_string(),
        })
    }
}

/// The runner-declared work interfaces. Wired whatever the caller's own
/// switch: a scheduled tick runs from `$HOME`, and the runner considers
/// each project by its own effective `modules.todos` (`work-dispatch`,
/// "Execution contract"). The `todo` namespace keeps its own refusal.
pub(super) fn ports(ctx: &Ctx) -> anyhow::Result<Option<WorkPorts>> {
    let work = Arc::new(work(ctx)?);
    Ok(Some(WorkPorts {
        pickup: work.clone(),
        claims: work.clone(),
        bindings: work.clone(),
        reflection: work.clone(),
        firings: work.clone(),
        holds: work.clone(),
        retention: work.clone(),
        reclamation: work.clone(),
        fleet: work.clone(),
        open_sessions: work,
    }))
}

/// Work's answers for automatic review, wired whether or not a project's
/// todos module is on.
pub(super) fn review(
    ctx: &Ctx,
) -> anyhow::Result<Arc<dyn instrukt_runner::interfaces::AutomaticReview>> {
    Ok(Arc::new(instrukt_todos::work::review::ReviewWork::new(
        ctx.home.clone(),
        ctx.github.clone(),
        evidence(ctx)?,
    )))
}

/// Work's answers a requested review is admitted on: a head's verdict
/// standing, its pinned base and the history retention cutoff.
pub(super) fn head_verdicts(ctx: &Ctx) -> Arc<dyn instrukt_runner::interfaces::HeadVerdicts> {
    Arc::new(instrukt_todos::work::review::HeadVerdictWork::new(
        ctx.home.clone(),
    ))
}

/// Work's evidence read contract for workflow, wired as the ports are: the
/// runner's workflow reads it per project.
pub(super) fn evidence(ctx: &Ctx) -> anyhow::Result<Option<Arc<dyn WorkEvidence>>> {
    Ok(Some(Arc::new(work(ctx)?)))
}

/// The work record of this invocation: the process environment supplies
/// credentials and the caller's session, the system clock the time.
pub(crate) fn work(ctx: &Ctx) -> anyhow::Result<Work> {
    Ok(Work::new(
        ctx.home.clone(),
        super::environment(ctx)?,
        ctx.clock.shared(),
    )
    .with_github(ctx.github.clone()))
}

/// Whether `modules.todos` is on where the caller stands.
fn enabled(ctx: &Ctx) -> anyhow::Result<bool> {
    let root = git::worktree_root(&ctx.cwd)?;
    config::module_enabled(&ctx.home, root.as_deref(), Module::Todos)
        .context("cannot read the todos module switch")
}
