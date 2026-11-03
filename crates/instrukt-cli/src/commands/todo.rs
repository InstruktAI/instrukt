//! Todo commands: the work record's surfaces (`instrukt todo create|list|
//! show|update|refresh|freeze|unfreeze|remove|reactivate|deliver|stamp|
//! artifacts`), with workflow's `prepare|work` flattened in from
//! [`todo_flow`].
//!
//! Owned by the work builder in wave 3 (`docs/local/spec/feature/work/`).
//! The workflow builder owns `todo_flow.rs`; the `Flow` arm below is wired
//! once and stays. The one-line abouts below appear in the `todo --help`
//! snapshot: keep them, so the two builders never edit the same snapshot.
//! The `modules.todos` refusal that names its door covers the whole
//! namespace and belongs in [`run`], before dispatch.

use std::process::ExitCode;

use crate::commands::{not_built, todo_flow};
use crate::context::Ctx;

#[derive(Debug, clap::Args)]
pub struct TodoArgs {
    #[command(subcommand)]
    command: TodoCommand,
}

/// Arguments of a subcommand whose surface is not built yet.
#[derive(Debug, clap::Args)]
struct Pending {
    #[arg(trailing_var_arg = true, allow_hyphen_values = true, hide = true)]
    rest: Vec<String>,
}

#[derive(Debug, clap::Subcommand)]
enum TodoCommand {
    /// Record a new todo with its kind and intent.
    Create(Pending),
    /// The project's work: active by default, or iceboxed, removed or delivered.
    List(Pending),
    /// A todo's record, evidence, holds, claim and session bindings.
    Show(Pending),
    /// Change a todo's metadata, relationships, hold or claim.
    Update(Pending),
    /// Refresh a conflicted todo from GitHub.
    Refresh(Pending),
    /// Park a todo in the icebox.
    Freeze(Pending),
    /// Return a parked todo to active consideration.
    Unfreeze(Pending),
    /// Withdraw a todo, retained for recovery.
    Remove(Pending),
    /// Restore withdrawn work.
    Reactivate(Pending),
    /// Record the confirmed delivered commit.
    Deliver(Pending),
    /// Record an attributed fact bound to evidence.
    Stamp(Pending),
    /// List, read, write and diff a todo's artifacts.
    Artifacts(Pending),
    #[command(flatten)]
    Flow(todo_flow::FlowCommand),
}

pub fn run(args: TodoArgs, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    match args.command {
        TodoCommand::Create(_) => not_built("todo create"),
        TodoCommand::List(_) => not_built("todo list"),
        TodoCommand::Show(_) => not_built("todo show"),
        TodoCommand::Update(_) => not_built("todo update"),
        TodoCommand::Refresh(_) => not_built("todo refresh"),
        TodoCommand::Freeze(_) => not_built("todo freeze"),
        TodoCommand::Unfreeze(_) => not_built("todo unfreeze"),
        TodoCommand::Remove(_) => not_built("todo remove"),
        TodoCommand::Reactivate(_) => not_built("todo reactivate"),
        TodoCommand::Deliver(_) => not_built("todo deliver"),
        TodoCommand::Stamp(_) => not_built("todo stamp"),
        TodoCommand::Artifacts(_) => not_built("todo artifacts"),
        TodoCommand::Flow(command) => todo_flow::run(command, ctx),
    }
}
