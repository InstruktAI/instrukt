//! Wiring of optional modules into the interfaces core and the runner
//! declare. Only the wiring files decide which module implementation
//! answers; each module's file wires it only when its switch is on.
//!
//! This file composes; it is settled for wave 3. Each builder fills only
//! their own file under `wiring/`:
//!
//! | File | Builder | Supplies |
//! | --- | --- | --- |
//! | `wiring/_work.rs` | work | core's `TodoExistence`; the runner's [`WorkPorts`]; [`WorkEvidence`] for workflow |
//! | `wiring/_workflow.rs` | workflow | core's `FlavorRoles`; the runner's `WorkflowAssignments` |
//! | `wiring/_blueprint.rs` | blueprint | core's `GraphGeneration` |
//! | `wiring/_runner.rs` | runner | the [`EventLog`] work's finalize-ready stamp appends to |

mod _blueprint;
mod _runner;
mod _work;
mod _workflow;

use std::sync::Arc;

use instrukt_core::interfaces::CoreWiring;
use instrukt_runner::events::EventLog;
use instrukt_runner::interfaces::{RunnerWiring, WorkPorts};
use instrukt_todos::evidence::WorkEvidence;
use instrukt_todos::workflow::WorkflowSchema;

use crate::context::Ctx;

/// Caller context is resolved once at the composition root. No runtime is
/// inferred from native variables, and no runtime command is executed here.
pub fn caller_lookup(
    ctx: &Ctx,
) -> anyhow::Result<Option<Arc<dyn instrukt_core::interfaces::CallerIdentityLookup>>> {
    let config = instrukt_core::config::Configuration::load_global(&ctx.home)?;
    match config.runner_runtime() {
        Ok(Some(runtime)) => {
            let adapter: Arc<dyn instrukt_runner::runtime::RuntimeAdapter> =
                Arc::from(instrukt_runner::runtime::adapter(runtime));
            let caller: Arc<dyn instrukt_core::interfaces::CallerIdentityLookup> = adapter;
            Ok(Some(caller))
        }
        Ok(None) => Ok(None),
        Err(error) => {
            tracing::debug!(event="caller_lookup_unavailable",error=%error);
            Ok(None)
        }
    }
}
pub fn environment(ctx: &Ctx) -> anyhow::Result<instrukt_core::config::Env> {
    let env = instrukt_core::config::Env::from_process();
    Ok(match caller_lookup(ctx)? {
        Some(lookup) => env.with_caller(lookup.as_ref()),
        None => env,
    })
}

/// The optional-module implementations for core contexts in this
/// invocation. A module that is off leaves its slot empty. The flavor
/// schema is always wired: authored workflow files are validated whether or
/// not `todos` is on.
pub fn core_wiring(ctx: &Ctx) -> anyhow::Result<CoreWiring> {
    Ok(CoreWiring {
        caller_identity: caller_lookup(ctx)?,
        todo_existence: _work::todo_existence(ctx)?,
        flavor_roles: _workflow::flavor_roles(ctx)?,
        flavor_schema: Some(Arc::new(WorkflowSchema)),
        graph_generation: _blueprint::graph_generation(ctx)?,
    })
}

/// The optional-module implementations publication may apply: those of
/// `todos` wired whatever the caller's working tree switches, because
/// publication decides from the committed configuration it publishes.
pub fn publication_wiring(ctx: &Ctx) -> anyhow::Result<CoreWiring> {
    Ok(CoreWiring {
        caller_identity: caller_lookup(ctx)?,
        todo_existence: _work::available_todo_existence(ctx)?,
        flavor_roles: Some(_workflow::available_flavor_roles(ctx)),
        flavor_schema: Some(Arc::new(WorkflowSchema)),
        graph_generation: _blueprint::graph_generation(ctx)?,
    })
}

/// The work and workflow implementations the runner dispatches todos
/// through. The runner decides per project whether `modules.todos` is on;
/// an empty slot means the implementation is not available in this build.
#[allow(dead_code, reason = "consumed by the runner commands in wave 3")]
pub fn runner_wiring(ctx: &Ctx) -> anyhow::Result<RunnerWiring> {
    let evidence = _work::evidence(ctx)?;
    Ok(RunnerWiring {
        work: _work::ports(ctx)?,
        workflow: _workflow::assignments(ctx, evidence)?,
    })
}

/// Work's command contract, for `instrukt todo prepare|work`'s deliberate
/// pickup claim (workflow) and for work's own commands.
#[allow(dead_code, reason = "consumed by the todo commands in wave 3")]
pub fn work_ports(ctx: &Ctx) -> anyhow::Result<Option<WorkPorts>> {
    _work::ports(ctx)
}

/// Work's evidence read contract, for workflow's fact provider.
#[allow(dead_code, reason = "consumed by the todo commands in wave 3")]
pub fn work_evidence(ctx: &Ctx) -> anyhow::Result<Option<Arc<dyn WorkEvidence>>> {
    _work::evidence(ctx)
}

/// The runner's event log, for work's finalize-ready stamp.
#[allow(
    dead_code,
    reason = "consumed by `todo stamp --finalize-ready` in wave 3"
)]
pub fn event_log(ctx: &Ctx) -> anyhow::Result<Option<Arc<dyn EventLog>>> {
    _runner::event_log(ctx)
}
