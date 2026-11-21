//! `instrukt` — the single binary of every module.
//!
//! This file is the composition root: it parses the command line, builds
//! the invocation context and hands each command to its context's command
//! module under `commands/`.
#![cfg_attr(
    test,
    allow(
        clippy::disallowed_methods,
        reason = "unit tests build their scratch trees directly"
    )
)]

mod authorization;
mod clock;
mod commands;
mod connectors;
mod context;
mod output;
mod wiring;

use std::io::Write as _;
use std::process::ExitCode;
use std::sync::OnceLock;

use clap::{CommandFactory, FromArgMatches, Parser, Subcommand};

use crate::commands::{
    agents, blueprint, bugs, config, discuss, docs, eval, events, history, mcp, repository, runner,
    secrets, specs, sync, todo, webhooks,
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
    /// Read computer/repository setup, adoption and publication before changing instrukt.
    Status(commands::status::StatusArgs),
    /// Park, inspect and remove known-issue dispositions for the log bug hunter.
    Bugs(bugs::BugsArgs),
    /// Open or close this native thread's conversation link.
    Discuss(discuss::DiscussArgs),
    /// Configured profiles and expiring profile/model availability.
    Agents(agents::AgentsArgs),
    /// Reach the MCP servers declared for this caller: list them, list one's
    /// tools, call a tool.
    Mcp(mcp::McpArgs),
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
    /// Inspect active and settled native runtime threads.
    Sessions(runner::SessionsArgs),
    /// Converge/check module infrastructure across adopted projects; repair with the same command.
    Install(runner::InstallArgs),
}

/// The log source of this run, once tracing writes to it: the panic hook
/// names its file.
enum LogDestination {
    File(&'static str),
    Stderr,
}

static LOG_DESTINATION: OnceLock<LogDestination> = OnceLock::new();

fn main() -> ExitCode {
    install_panic_hook();
    // The connector kinds this binary supports reach configuration before
    // anything reads it.
    instrukt_core::config::install_connector_declarations(&connectors::DECLARATIONS);
    if let Err(error) = instrukt_core::fsx::fence(&instrukt_core::config::Env::from_process()) {
        eprintln!("instrukt: {error}");
        return ExitCode::from(instrukt_core::fsx::FENCE_EXIT);
    }
    let code = run();
    // A write refused at the fence ends the binary with the fence's own
    // status, whatever the refused operation's caller made of it.
    if instrukt_core::fsx::fence_refused() {
        return ExitCode::from(instrukt_core::fsx::FENCE_EXIT);
    }
    code
}

fn run() -> ExitCode {
    let matches = match Cli::command().try_get_matches() {
        Ok(matches) => matches,
        Err(error) if error.kind() == clap::error::ErrorKind::DisplayHelp => {
            let result = (|| -> anyhow::Result<Option<clap::Error>> {
                let ctx = Ctx::inspection()?;
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
    // A machine configuration that cannot be loaded stops every command;
    // `config validate --global` names the file's own problem.
    if let Some(code) = unloadable_machine_layer(&matches) {
        return code;
    }
    if let Command::Status(args) = &cli.command {
        return match Ctx::inspection().and_then(|mut ctx| {
            let caller_error = match authorization::check(&ctx, &matches) {
                Ok(()) => None,
                Err(error) if error.is::<authorization::PermissionRefused>() => return Err(error),
                Err(error) => Some(error.to_string()),
            };
            commands::status::run(args, &mut ctx, caller_error)
        }) {
            Ok(code) => code,
            Err(error) => report(&error),
        };
    }
    let mut activated = if matches!(&cli.command,Command::Webhooks(args) if args.managed()) {
        let mut ctx = match Ctx::inspection() {
            Ok(ctx) => ctx,
            Err(error) => return report(&error),
        };
        if let Err(error) = authorization::check(&ctx, &matches) {
            return report(&error);
        }
        ctx.receiver = match instrukt_runner::webhooks::prepare_activation(&ctx.home) {
            Ok(receiver) => Some(receiver),
            Err(error) => return report(&error.into()),
        };
        Some(ctx)
    } else {
        None
    };
    if !authorization::strict_inspection(&matches)
        && !authorization::untraced(&matches)
        && let Err(err) = init_tracing(&cli.command)
    {
        return report(&err);
    }
    let deferred_patch = matches
        .subcommand()
        .is_some_and(|(name, args)| name == "config" && args.subcommand_name() == Some("patch"));
    let mut ctx = match if let Some(ctx) = activated.take() {
        Ok(ctx)
    } else if deferred_patch
        || authorization::strict_inspection(&matches)
        || authorization::untraced(&matches)
    {
        Ctx::inspection()
    } else if authorization::read_only(&matches) || authorization::repository_local(&matches) {
        Ctx::configuration_read()
    } else {
        Ctx::from_process()
    } {
        Ok(ctx) => ctx,
        Err(err) => return report(&err),
    };
    if authorization::read_only(&matches)
        && !authorization::repository_local(&matches)
        && !matches!(&cli.command, Command::Install(_))
    {
        let notice_result = (|| -> anyhow::Result<()> {
            let mut layers = vec![instrukt_core::config::Layer::Global];
            // A read naming its owning project reads that project's layer,
            // wherever the caller stands; one this computer does not record is
            // refused by the command itself.
            let root = match authorization::named_project(&matches) {
                Some(named) => {
                    instrukt_core::publication::PublishedRecord::load(&ctx.home, &named)?
                        .map(|record| record.checkout)
                }
                None => instrukt_core::git::worktree_root(&ctx.cwd)?,
            };
            if let Some(root) = root {
                layers.push(instrukt_core::config::Layer::Project(root));
            }
            if matches.subcommand().is_some_and(|(name, args)| {
                name == "agents" || (name == "config" && args.subcommand_name() == Some("show"))
            }) && let Some(notice) = instrukt_core::config::Agents::inspection_notice(&ctx.home)?
            {
                eprintln!("instrukt: {notice}");
            }
            for layer in layers {
                if let Some(notice) =
                    instrukt_core::config::inspect_layer(&ctx.home, &layer)?.migration
                {
                    eprintln!("instrukt: {notice}");
                }
            }
            Ok(())
        })();
        // An inspection reports a configuration it cannot read itself; a
        // notice about converting it does not stand in its way.
        match notice_result {
            Ok(()) => {}
            Err(error) if authorization::strict_inspection(&matches) => {
                tracing::debug!(event = "inspection_notice_unavailable", %error);
            }
            Err(error) => return report(&error),
        }
    }
    // As for `instrukt status`: an inspection whose caller cannot be
    // determined still runs, and a refused permission still refuses it.
    match authorization::check(&ctx, &matches) {
        Ok(()) => {}
        Err(error)
            if authorization::strict_inspection(&matches)
                && !error.is::<authorization::PermissionRefused>() =>
        {
            tracing::debug!(event = "inspection_caller_undetermined", %error);
        }
        Err(error) => return report(&error),
    }
    if deferred_patch {
        ctx.home = ctx.home.for_writing();
        if let Err(error) = instrukt_core::config::Agents::migrate_shape(&ctx.home) {
            return report(&error.into());
        }
    }
    let outcome = match cli.command {
        Command::Bugs(args) => bugs::run(args, &mut ctx),
        Command::Status(args) => commands::status::run(&args, &mut ctx, None),
        Command::DevSeed => seed_development(&mut ctx),
        Command::Discuss(args) => discuss::run(args, &mut ctx),
        Command::Mcp(args) => mcp::run(args, &mut ctx),
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

/// Records an error, prints it to stderr as one line and returns the generic
/// failure status. A machine configuration that cannot be loaded, wherever a
/// command met it, is named as such.
fn report(err: &anyhow::Error) -> ExitCode {
    let line = err
        .chain()
        .find_map(|cause| cause.downcast_ref::<instrukt_core::config::ConfigError>())
        .and_then(machine_layer_line)
        .unwrap_or_else(|| output::error_line(err));
    tracing::error!(event = "command_failed", error = %line);
    eprintln!("instrukt: {line}");
    ExitCode::FAILURE
}

/// The line for `error` when it says the machine layer cannot be loaded.
fn machine_layer_line(error: &instrukt_core::config::ConfigError) -> Option<String> {
    let (path, reason) = error.unloadable_reason()?;
    let home = instrukt_core::home::Home::from_env().ok()?;
    (path == home.global_config()).then(|| {
        format!(
            "the machine configuration {} cannot be read: {reason}; run `instrukt config validate --global` to see what to fix",
            path.display()
        )
    })
}

/// The failure status when the machine layer cannot be loaded, after its one
/// line: the file's own problem for `config validate --global`, which is
/// how it is found; for every other command, the line naming that door.
fn unloadable_machine_layer(matches: &clap::ArgMatches) -> Option<ExitCode> {
    let home = match instrukt_core::home::Home::from_env() {
        Ok(home) => home,
        Err(error) => return Some(report(&error.into())),
    };
    let error = instrukt_core::config::unloadable_machine_layer(&home)?;
    let validating = matches.subcommand().is_some_and(|(name, config)| {
        name == "config"
            && config
                .subcommand()
                .is_some_and(|(name, args)| name == "validate" && args.get_flag("global"))
    });
    let line = if validating {
        output::error_line(&error.into())
    } else {
        machine_layer_line(&error).unwrap_or_else(|| error.to_string())
    };
    tracing::error!(event = "command_failed", error = %line);
    eprintln!("instrukt: {line}");
    Some(ExitCode::FAILURE)
}

/// A panic ends the command with one line naming the log, never the panic's
/// message, location or backtrace; those go to the log.
fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let location = info.location().map_or_else(String::new, |location| {
            format!(
                "{}:{}:{}",
                location.file(),
                location.line(),
                location.column()
            )
        });
        let payload = info.payload();
        let message = payload
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
            .unwrap_or("a panic without a message");
        if matches!(LOG_DESTINATION.get(), Some(LogDestination::Stderr)) {
            let _ = writeln!(std::io::stderr(), "instrukt stopped unexpectedly");
            return;
        }
        tracing::error!(event = "panic", %location, message);
        let log = LOG_DESTINATION
            .get()
            .and_then(|destination| match destination {
                LogDestination::File(source) => {
                    instrukt_ai_logging::resolve_log_file("instrukt", Some(source)).ok()
                }
                LogDestination::Stderr => None,
            })
            .or_else(|| instrukt_ai_logging::resolve_log_dir("instrukt").ok())
            .map_or_else(
                || "instrukt's log".to_owned(),
                |path| path.display().to_string(),
            );
        // A hook that panics aborts the process: a write that fails is left.
        let _ = writeln!(
            std::io::stderr(),
            "instrukt stopped unexpectedly; the details are in {log}"
        );
    }));
}

/// Initialise the contract writer before constructing the invocation context.
fn init_tracing(command: &Command) -> anyhow::Result<()> {
    let source = match command {
        Command::DevSeed => "dev-seed",
        Command::Status(_) => "status",
        Command::Bugs(_) => "bugs",
        Command::Discuss(_) => "discuss",
        Command::Mcp(_) => "mcp",
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
    // The logger creates its directory and file itself: under a fence, both
    // are admitted where the library names them, before it does.
    if instrukt_core::fsx::fence_active() {
        instrukt_core::fsx::admit(&instrukt_ai_logging::resolve_log_dir("instrukt")?)?;
        instrukt_core::fsx::admit(&instrukt_ai_logging::resolve_log_file(
            "instrukt",
            Some(source),
        )?)?;
    }
    const APP_TARGETS: &[&str] = &[
        "instrukt",
        "instrukt_cli",
        "instrukt_core",
        "instrukt_blueprint",
        "instrukt_runner",
        "instrukt_todos",
        "instrukt_company",
        "instrukt_connector",
        "instrukt_connector_slack",
    ];
    let builder = || {
        APP_TARGETS.iter().fold(
            instrukt_ai_logging::Builder::new("instrukt").source(source),
            |builder, prefix| builder.app_prefix(*prefix),
        )
    };
    let mut file = builder();
    if matches!(command, Command::Mcp(_)) {
        // The HTTP client's records carry what an MCP server sent back, a
        // reflected login among it, before instrukt can remove it: for this
        // command no third-party record below a warning is written.
        file = file.env_lookup(|name| match name {
            "INSTRUKT_THIRD_PARTY_LOG_LEVEL" => Some("WARNING".into()),
            "INSTRUKT_THIRD_PARTY_LOGGERS" => None,
            other => std::env::var_os(other),
        });
    }
    let destination = match file.init() {
        Ok(_) => LogDestination::File(source),
        Err(error) => {
            if builder()
                .env_lookup(|_| None)
                .init_writer(std::io::stderr())
                .is_ok()
            {
                tracing::warn!(event = "file_logging_unavailable", %error, "logging to stderr");
            } else {
                let _ = writeln!(
                    std::io::stderr(),
                    "instrukt: cannot initialise diagnostic logging"
                );
            }
            LogDestination::Stderr
        }
    };
    let _ = LOG_DESTINATION.set(destination);
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
        wiring: &instrukt_core::interfaces::CoreWiring::default(),
        now: ctx.clock.now(),
    })?;
    if let publication::SyncOutcome::Refused { refusal, .. } = report.outcome {
        anyhow::bail!("development publication refused: {refusal}");
    }
    instrukt_core::fsx::atomic_write(&ctx.home.root().join(".seeded-v1"), b"1\n")?;
    Ok(ExitCode::SUCCESS)
}
