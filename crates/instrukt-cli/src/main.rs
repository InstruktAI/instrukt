//! `instrukt` — the single binary of every module.
//!
//! This file is the composition root: it parses the command line, builds
//! the invocation context and hands each command to its context's command
//! module under `commands/`.

mod authorization;
mod clock;
mod commands;
mod context;
mod output;
mod wiring;

use std::process::ExitCode;

use clap::{CommandFactory, FromArgMatches, Parser, Subcommand};

use crate::commands::{
    agents, blueprint, config, discuss, docs, eval, events, history, repository, runner, secrets,
    specs, sync, todo, webhooks,
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
    #[command(hide = true)]
    DevSeed,
    /// Open or close this native thread's conversation link.
    Discuss(discuss::DiscussArgs),
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
    /// Extract, import and search retained conversations across computers.
    History(history::HistoryArgs),
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
    let matches = match Cli::command().try_get_matches() {
        Ok(matches) => matches,
        Err(error) if error.kind() == clap::error::ErrorKind::DisplayHelp => {
            let result = (|| -> anyhow::Result<Option<clap::Error>> {
                let ctx = Ctx::from_process()?;
                let Some((assignment, caps)) = authorization::caller(&ctx)? else {
                    return Ok(None);
                };
                let command = authorization::help(Cli::command(), &mut Vec::new(), &caps)
                    .after_help(format!("Role: {} · class: {}. Scope comes from assignment {}. Object claims, phase and writer constraints apply.\nUsual capabilities: {}",assignment.role,caps.class.name(),assignment.key,caps.describe()));
                Ok(command.try_get_matches().err())
            })();
            return match result {
                Ok(Some(role_help)) => role_help.exit(),
                Ok(None) => error.exit(),
                Err(error) => report(&error),
            };
        }
        Err(error) => error.exit(),
    };
    let cli = match Cli::from_arg_matches(&matches) {
        Ok(cli) => cli,
        Err(error) => error.exit(),
    };
    if let Err(err) = init_tracing(&cli.command) {
        return report(&err);
    }
    let mut ctx = match Ctx::from_process() {
        Ok(ctx) => ctx,
        Err(err) => return report(&err),
    };
    if let Err(error) = authorization::check(&ctx, &matches) {
        return report(&error);
    }
    let outcome = match cli.command {
        Command::DevSeed => seed_development(&mut ctx),
        Command::Discuss(args) => discuss::run(args, &mut ctx),
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
        Command::History(args) => history::run(args, &mut ctx),
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
        Command::DevSeed => "dev-seed",
        Command::Discuss(_) => "discuss",
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
        Command::History(_) => "history",
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

fn seed_development(ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    use instrukt_core::{config, publication};
    if !ctx.home.is_development() {
        anyhow::bail!("dev-seed requires the repository development wrapper");
    }
    config::init(&ctx.home, &config::Layer::Global, None)?;
    config::Agents::load(&ctx.home)?;
    let report = publication::sync(&publication::SyncRequest {
        home: &ctx.home,
        cwd: &ctx.cwd,
        force: false,
        wiring: &instrukt_core::interfaces::CoreWiring::default(),
        now: ctx.clock.now(),
    })?;
    if let publication::SyncOutcome::Refused { refusal, .. } = report.outcome {
        anyhow::bail!("development publication refused: {refusal}");
    }
    instrukt_core::fsx::atomic_write(&ctx.home.root().join(".seeded-v1"), b"1\n")?;
    Ok(ExitCode::SUCCESS)
}
