//! Workflow's subcommands in the todo namespace: `instrukt todo prepare
//! <slug>` and `instrukt todo work <slug>` (`docs/local/spec/domain/
//! workflow.md`, "Flavor-facing contract"; `preparation`,
//! `delivery-cycle`).
//!
//! Owned by the workflow builder in wave 3; flattened into `todo` by
//! `todo.rs`, which also runs the `modules.todos` refusal before
//! dispatching here. Keep the one-line abouts: they appear in the
//! `todo --help` snapshot.

use std::process::ExitCode;

use crate::commands::not_built;
use crate::context::Ctx;

/// Arguments of a subcommand whose surface is not built yet.
#[derive(Debug, clap::Args)]
pub struct Pending {
    #[arg(trailing_var_arg = true, allow_hyphen_values = true, hide = true)]
    rest: Vec<String>,
}

#[derive(Debug, clap::Subcommand)]
pub enum FlowCommand {
    /// A todo's next preparation obligation (deliberate pickup).
    Prepare(Pending),
    /// A todo's next delivery obligation (deliberate pickup).
    Work(Pending),
}

pub fn run(command: FlowCommand, _ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    match command {
        FlowCommand::Prepare(_) => not_built("todo prepare"),
        FlowCommand::Work(_) => not_built("todo work"),
    }
}
