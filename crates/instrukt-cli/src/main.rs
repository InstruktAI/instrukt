//! `instrukt` — the single binary of every module.
//!
//! This file is the composition root: it parses the command line, builds
//! the invocation context and hands each command to its context's command
//! module under `commands/`.

mod clock;
mod commands;
mod context;
mod output;
mod wiring;

use std::process::ExitCode;

use clap::{Parser, Subcommand};

use crate::commands::{
    agents, blueprint, config, docs, eval, events, repository, runner, secrets, specs, sync, todo,
    webhooks,
};
use crate::context::Ctx;

/// Context engineering for coding agents.
#[derive(Debug, Parser)]
#[command(name = "instrukt", version, propagate_version = true)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Configured profiles and expiring profile/model availability.
    Agents(agents::AgentsArgs),
    /// Store or generate protected machine-local secrets.
    Secrets(secrets::SecretsArgs),
    /// Read, edit, initialize and validate `instrukt.yaml`.
    Config(config::ConfigArgs),
    /// Validate, list and read documentation snippets.
    Docs(docs::DocsArgs),
    /// Publish the current project's committed docs into the home.
    Sync(sync::SyncArgs),
    /// Adopt instrukt in the current git repository.
    Init(repository::InitArgs),
    /// Install, remove, inspect and run instrukt's git hook sections.
    Hooks(repository::HooksArgs),
    /// Attributed commits in an authoring worktree.
    Repo(repository::RepoArgs),
    /// Use-case coverage, reservation and trace.
    Specs(specs::SpecsArgs),
    /// Structural graph queries (blueprint module).
    Blueprint(blueprint::BlueprintArgs),
    /// The work record and workflow (todos module).
    Todo(todo::TodoArgs),
    /// Evaluation reports from the work record.
    Eval(eval::EvalArgs),
    /// Discover producer-declared event types and their schemas.
    Events(events::EventsArgs),
    /// Internal signed loopback webhook receiver (installation is separate).
    Webhooks(webhooks::WebhooksArgs),
    /// Run the runner until stopped.
    Run(runner::RunArgs),
    /// One runner wake-up, as the OS scheduler invokes it.
    Tick(runner::TickArgs),
    /// Scheduled tick status.
    Ticks(runner::TicksArgs),
    /// Inspect keyed runtime threads and resolve uncertain unattended starts.
    Sessions(runner::SessionsArgs),
    /// Register the machine's runner wake-up with the OS scheduler.
    Install(runner::InstallArgs),
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    if let Err(err) = init_tracing(&cli.command) {
        return report(&err);
    }
    let mut ctx = match Ctx::from_process() {
        Ok(ctx) => ctx,
        Err(err) => return report(&err),
    };
    let outcome = match cli.command {
        Command::Agents(args) => agents::run(args, &mut ctx),
        Command::Secrets(args) => secrets::run(args, &mut ctx),
        Command::Config(args) => config::run(args, &mut ctx),
        Command::Docs(args) => docs::run(args, &mut ctx),
        Command::Sync(args) => sync::run(args, &mut ctx),
        Command::Init(args) => repository::run_init(args, &mut ctx),
        Command::Hooks(args) => repository::run_hooks(args, &mut ctx),
        Command::Repo(args) => repository::run_repo(args, &mut ctx),
        Command::Specs(args) => specs::run(args, &mut ctx),
        Command::Blueprint(args) => blueprint::run(args, &mut ctx),
        Command::Todo(args) => todo::run(args, &mut ctx),
        Command::Eval(args) => eval::run(args, &mut ctx),
        Command::Events(args) => events::run(args, &mut ctx),
        Command::Webhooks(args) => webhooks::run(args, &mut ctx),
        Command::Run(args) => runner::run_run(args, &mut ctx),
        Command::Tick(args) => runner::run_tick(args, &mut ctx),
        Command::Ticks(args) => runner::run_ticks(args, &mut ctx),
        Command::Sessions(args) => runner::run_sessions(args, &mut ctx),
        Command::Install(args) => runner::run_install(args, &mut ctx),
    };
    match outcome.and_then(|code| ctx.out.flush().map(|()| code)) {
        Ok(code) => code,
        Err(err) => report(&err),
    }
}

/// Records an error, prints it to stderr and returns the generic failure status.
fn report(err: &anyhow::Error) -> ExitCode {
    tracing::error!(event = "command_failed", error = %format!("{err:#}"));
    eprintln!("instrukt: {err:#}");
    ExitCode::FAILURE
}

/// Initialise the contract writer before constructing the invocation context.
fn init_tracing(command: &Command) -> anyhow::Result<()> {
    let source = match command {
        Command::Agents(_) => "agents",
        Command::Secrets(_) => "secrets",
        Command::Config(_) => "config",
        Command::Docs(_) => "docs",
        Command::Sync(_) => "sync",
        Command::Init(_) => "init",
        Command::Hooks(_) => "hooks",
        Command::Repo(_) => "repo",
        Command::Specs(_) => "specs",
        Command::Blueprint(_) => "blueprint",
        Command::Todo(_) => "todo",
        Command::Eval(_) => "eval",
        Command::Events(_) => "events",
        Command::Webhooks(_) => "webhooks",
        Command::Run(_) => "run",
        Command::Tick(_) => "tick",
        Command::Ticks(_) => "ticks",
        Command::Sessions(_) => "sessions",
        Command::Install(_) => "install",
    };
    instrukt_ai_logging::Builder::new("instrukt")
        .source(source)
        .app_prefix("instrukt_cli")
        .app_prefix("instrukt_core")
        .app_prefix("instrukt_blueprint")
        .app_prefix("instrukt_runner")
        .app_prefix("instrukt_todos")
        .init()?;
    Ok(())
}
