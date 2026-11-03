//! `instrukt` — the single binary of every module.
//!
//! This file is the composition root: it parses the command line, builds
//! the invocation context and hands each command to its context's command
//! module under `commands/`.

mod commands;
mod context;
mod output;
mod wiring;

use std::io::IsTerminal;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

use crate::commands::{blueprint, config, docs, eval, repository, runner, specs, sync, todo};
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
    /// Attributed commits and commit-review stamps.
    Repo(repository::RepoArgs),
    /// Use-case coverage, reservation and trace.
    Specs(specs::SpecsArgs),
    /// Structural graph queries (blueprint module).
    Blueprint(blueprint::BlueprintArgs),
    /// The work record and workflow (todos module).
    Todo(todo::TodoArgs),
    /// Evaluation reports from the work record.
    Eval(eval::EvalArgs),
    /// Run the runner until stopped.
    Run(runner::RunArgs),
    /// One runner wake-up, as the OS scheduler invokes it.
    Tick(runner::TickArgs),
    /// Scheduled tick status.
    Ticks(runner::TicksArgs),
    /// The sessions facade over the configured runtime.
    Sessions(runner::SessionsArgs),
    /// Register the machine's runner wake-up with the OS scheduler.
    Install(runner::InstallArgs),
}

fn main() -> ExitCode {
    init_tracing();
    let cli = Cli::parse();
    let mut ctx = match Ctx::from_process() {
        Ok(ctx) => ctx,
        Err(err) => return report(&err),
    };
    let outcome = match cli.command {
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

/// Prints an error to stderr and returns the generic failure status.
fn report(err: &anyhow::Error) -> ExitCode {
    eprintln!("instrukt: {err:#}");
    ExitCode::FAILURE
}

/// Diagnostics go to stderr at INFO, the production level
/// (`sdlc/policy/observability`), or at the directives `INSTRUKT_LOG`
/// supplies. Invalid directives are reported, never silently replaced.
fn init_tracing() {
    let filter = match std::env::var("INSTRUKT_LOG") {
        Err(std::env::VarError::NotPresent) => tracing_subscriber::EnvFilter::new("info"),
        Err(std::env::VarError::NotUnicode(_)) => {
            eprintln!("instrukt: INSTRUKT_LOG is not valid UTF-8; logging at info");
            tracing_subscriber::EnvFilter::new("info")
        }
        Ok(directives) => match tracing_subscriber::EnvFilter::try_new(&directives) {
            Ok(filter) => filter,
            Err(err) => {
                eprintln!(
                    "instrukt: INSTRUKT_LOG={directives:?} is invalid ({err}); logging at info"
                );
                tracing_subscriber::EnvFilter::new("info")
            }
        },
    };
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(std::io::stderr().is_terminal())
        .with_writer(std::io::stderr)
        .init();
}
