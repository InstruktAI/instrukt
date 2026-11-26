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
    let inspection = ctx.home.for_inspection();
    let config = instrukt_core::config::Configuration::load_global(&inspection)?;
    match config.runner_runtime() {
        Ok(runtime) => {
            let adapter: Arc<dyn instrukt_runner::runtime::RuntimeAdapter> =
                Arc::from(instrukt_runner::runtime::adapter(runtime));
            let caller: Arc<dyn instrukt_core::interfaces::CallerIdentityLookup> = adapter;
            Ok(Some(caller))
        }
        Err(error) => {
            tracing::debug!(event="caller_lookup_unavailable",error=%error);
            Ok(None)
        }
    }
}
/// The selected runtime's adapter, asked which agent runtimes are configured,
/// as [`caller_lookup`] selects it: an unset `runner.runtime` selects the
/// compiled adapter. `None` only when the machine layer names no compiled
/// adapter.
pub fn configured_runtimes(
    ctx: &Ctx,
) -> anyhow::Result<Option<Arc<dyn instrukt_core::interfaces::ConfiguredAgentRuntimes>>> {
    let config = instrukt_core::config::Configuration::load_global(&ctx.home)?;
    match config.runner_runtime() {
        Ok(runtime) => Ok(Some(Arc::new(ConfiguredRuntimes(Arc::from(
            instrukt_runner::runtime::adapter(runtime),
        ))))),
        Err(error) => {
            tracing::debug!(event = "configured_runtimes_unavailable", error = %error);
            Ok(None)
        }
    }
}

/// The selected runtime's adapter, answering publication's question.
struct ConfiguredRuntimes(Arc<dyn instrukt_runner::runtime::RuntimeAdapter>);

impl instrukt_core::interfaces::ConfiguredAgentRuntimes for ConfiguredRuntimes {
    fn configured(
        &self,
        home: &instrukt_core::home::Home,
        env: &instrukt_core::config::Env,
    ) -> instrukt_core::interfaces::Answer {
        match self.0.configured_agent_runtimes(home, env) {
            Ok(instances) => instrukt_core::interfaces::Answer::Reported(instances),
            Err(error) => instrukt_core::interfaces::Answer::Unreachable {
                runtime: self.0.runtime().name().to_owned(),
                cause: error.to_string(),
            },
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
        github_actors: Some(ctx.github.clone()),
        session_role: Some(Arc::new(RecordedRole(ctx.home.clone()))),
        todo_existence: _work::todo_existence(ctx)?,
        flavor_roles: _workflow::flavor_roles(ctx)?,
        flavor_schema: Some(Arc::new(WorkflowSchema)),
        graph_generation: _blueprint::graph_generation(ctx)?,
        configured_runtimes: None,
    })
}

/// The optional-module implementations publication may apply: those of
/// `todos` wired whatever the caller's working tree switches, because
/// publication decides from the committed configuration it publishes.
pub fn publication_wiring(ctx: &Ctx) -> anyhow::Result<CoreWiring> {
    Ok(CoreWiring {
        caller_identity: caller_lookup(ctx)?,
        github_actors: Some(ctx.github.clone()),
        session_role: Some(Arc::new(RecordedRole(ctx.home.clone()))),
        todo_existence: _work::available_todo_existence(ctx)?,
        flavor_roles: Some(_workflow::available_flavor_roles(ctx)),
        flavor_schema: Some(Arc::new(WorkflowSchema)),
        graph_generation: _blueprint::graph_generation(ctx)?,
        configured_runtimes: configured_runtimes(ctx)?,
    })
}

/// The work and workflow implementations the runner dispatches todos
/// through. The runner decides per project whether `modules.todos` is on;
/// an empty slot means the implementation is not available in this build.
#[allow(dead_code, reason = "consumed by the runner commands in wave 3")]
pub fn runner_wiring(ctx: &Ctx) -> anyhow::Result<RunnerWiring> {
    let evidence = _work::evidence(ctx)?;
    Ok(RunnerWiring {
        github_credentials: Some(ctx.github.clone()),
        review: Some(_work::review(ctx)?),
        work: _work::ports(ctx)?,
        workflow: _workflow::assignments(ctx, evidence.clone())?,
        standings: _workflow::standings(ctx, evidence)?,
        company: Some(company()),
    })
}

/// The company the runner hosts, built with the binary's connector
/// registry.
pub fn company() -> std::sync::Arc<dyn instrukt_runner::interfaces::CompanyPorts> {
    std::sync::Arc::new(instrukt_company::conversations::Ports::new(
        crate::connectors::builders(),
    ))
}

/// Work's command contract, for `instrukt todo work`'s deliberate
/// pickup claim (workflow) and for work's own commands.
#[allow(dead_code, reason = "consumed by the todo commands in wave 3")]
pub fn work_ports(ctx: &Ctx) -> anyhow::Result<Option<WorkPorts>> {
    _work::ports(ctx)
}

/// What fleet attention reads of the work record, for `instrukt events
/// raise`.
pub fn fleet_record(
    ctx: &Ctx,
) -> anyhow::Result<Arc<dyn instrukt_runner::interfaces::FleetRecord>> {
    Ok(Arc::new(_work::work(ctx)?))
}

/// Work's answers for `instrukt review start`
/// (`project/instrukt/spec/feature/runner/requested-review`).
pub fn head_verdicts(ctx: &Ctx) -> Arc<dyn instrukt_runner::interfaces::HeadVerdicts> {
    _work::head_verdicts(ctx)
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

pub(crate) struct RecordedRole(pub(crate) instrukt_core::home::Home);
impl instrukt_core::interfaces::SessionRoleLookup for RecordedRole {
    fn role(
        &self,
        session: &instrukt_core::ids::SessionRef,
    ) -> Result<Option<instrukt_core::ids::RoleName>, instrukt_core::interfaces::InterfaceError>
    {
        instrukt_runner::assignments::resolve(
            &self.0,
            session,
            instrukt_todos::work::recorded_assignment,
            instrukt_company::conversations::conversation_assignment,
        )
        .map(|a| a.map(|a| a.role))
        .map_err(|e| instrukt_core::interfaces::InterfaceError {
            module: "assignments",
            message: e.to_string(),
        })
    }
}
