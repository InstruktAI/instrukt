//! Runner commands: `run`, `tick`, `ticks status`, `sessions …`, `install`.
//!
//! This module is the runner's composition root: it reads the clock, the
//! machine's time zone and the process environment, and hands them to
//! `instrukt_runner`.

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use anyhow::{Context as _, bail};
use instrukt_core::config::{Configuration, Module};
use instrukt_core::ids::SessionRef;
use instrukt_runner::install::{self, Scheduler, WakeUp};
use instrukt_runner::keyed;
use instrukt_runner::runner::{self, RunnerContext, TickOutcome};
use instrukt_runner::sessions::Sessions;

use instrukt_runner::state::RunnerState;
use instrukt_runner::ticks;
use jiff::tz::TimeZone;
use serde_json::json;

use crate::context::Ctx;
use crate::wiring;

/// The interval of `instrukt run`'s wake-ups, the same minute the OS
/// scheduler's wake-up uses.
const RUN_INTERVAL: Duration = Duration::from_secs(60);

#[derive(Debug, clap::Args)]
pub struct RunArgs {}

/// Runs the runner until stopped: a wake-up every minute, holding the
/// runner lock.
pub fn run_run(_args: RunArgs, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    require_runner_modules(ctx, "run")?;
    let env = crate::wiring::environment(ctx)?;
    let tz = TimeZone::system();
    let wiring = wiring::runner_wiring(ctx)?;
    let cx = RunnerContext {
        home: &ctx.home,
        env: &env,
        tz: &tz,
        wiring: &wiring,
    };
    let clock = ctx.clock;
    runner::run(&cx, move || clock.now(), RUN_INTERVAL)?;
    Ok(ExitCode::SUCCESS)
}

#[derive(Debug, clap::Args)]
pub struct TickArgs {}

/// One wake-up, as the OS scheduler invokes it every minute. Exits at once
/// while `instrukt run` holds the runner lock.
pub fn run_tick(_args: TickArgs, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    require_runner_modules(ctx, "tick")?;
    let env = crate::wiring::environment(ctx)?;
    let tz = TimeZone::system();
    let wiring = wiring::runner_wiring(ctx)?;
    let cx = RunnerContext {
        home: &ctx.home,
        env: &env,
        tz: &tz,
        wiring: &wiring,
    };
    match runner::tick(&cx, ctx.clock.now())? {
        TickOutcome::Done => {}
        TickOutcome::Busy => ctx
            .out
            .line("the runner is busy; this wake-up was skipped")?,
    }
    Ok(ExitCode::SUCCESS)
}

#[derive(Debug, clap::Args)]
pub struct TicksArgs {
    #[command(subcommand)]
    command: TicksCommand,
}

#[derive(Debug, clap::Subcommand)]
enum TicksCommand {
    /// Show the last wake-up; warns when none arrived for five minutes.
    Status(JsonArgs),
}

#[derive(Debug, clap::Args)]
struct JsonArgs {
    /// Render as JSON.
    #[arg(long)]
    json: bool,
}

pub fn run_ticks(args: TicksArgs, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    match args.command {
        TicksCommand::Status(args) => {
            let state = RunnerState::new(&ctx.home);
            let status = ticks::status(ticks::last_wakeup(&state)?, ctx.clock.now());
            if args.json {
                ctx.out.json(&status)?;
            } else {
                match status.last_wakeup {
                    None => ctx.out.line("no wake-up ever recorded")?,
                    Some(last) => {
                        ctx.out.line(&format!("last wake-up: {last}"))?;
                        if status.stale {
                            ctx.out.line(
                                "warning: no wake-up for more than five minutes; check `instrukt install`",
                            )?;
                        }
                    }
                }
            }
            Ok(ExitCode::SUCCESS)
        }
    }
}

#[derive(Debug, clap::Args)]
pub struct SessionsArgs {
    #[command(subcommand)]
    command: SessionsCommand,
}

#[derive(Debug, clap::Subcommand)]
enum SessionsCommand {
    /// List this project's active and settled runtime threads, newest activity first.
    List(ListArgs),
    /// Resolve a standing agent's start whose result never arrived: bind
    /// the session the runtime did create, or declare that none was.
    ResolveStart(ResolveStartArgs),
}

#[derive(Debug, clap::Args)]
#[command(group(clap::ArgGroup::new("resolution").required(true).args(["session", "none"])))]
struct ResolveStartArgs {
    /// Durable start key, or the home-standing shorthand `<procedure>@<project>`.
    key: String,
    /// The session the runtime created, `<runtime>:<id>`.
    #[arg(long)]
    session: Option<SessionRef>,
    /// Confirm no thread was created and no creation remains in flight; retry the same key.
    #[arg(long)]
    none: bool,
    #[arg(long)]
    json: bool,
}

#[derive(Debug, clap::Args)]
struct ListArgs {
    /// Settled-thread window (default 3d), e.g. 24h or 30m. Active threads always show.
    #[arg(long,value_parser=parse_since,conflicts_with="all")]
    since: Option<Duration>,
    /// Include all settled threads as well as all active threads.
    #[arg(long)]
    all: bool,
    #[arg(long)]
    json: bool,
}

fn parse_since(text: &str) -> Result<Duration, String> {
    let at = text
        .find(|c: char| !c.is_ascii_digit())
        .ok_or_else(|| "duration needs a unit: s, m, h, d or w".to_owned())?;
    let (number, unit) = text.split_at(at);
    let number = number
        .parse::<u64>()
        .map_err(|_| "duration needs a nonnegative integer".to_owned())?;
    let scale = match unit {
        "s" => 1,
        "m" => 60,
        "h" => 3600,
        "d" => 86400,
        "w" => 604800,
        _ => return Err("duration unit must be s, m, h, d or w".into()),
    };
    number
        .checked_mul(scale)
        .map(Duration::from_secs)
        .ok_or_else(|| "duration is too large".into())
}

pub fn run_sessions(args: SessionsArgs, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    require_runner_modules(ctx, "sessions")?;
    let env = crate::wiring::environment(ctx)?;
    let config = Configuration::load_at(&ctx.home, &ctx.cwd)?;
    let home = ctx.home.clone();
    let sessions = Sessions::new(&home, &config, &env);
    match args.command {
        SessionsCommand::List(args) => {
            let since = if args.all {
                None
            } else {
                Some(args.since.unwrap_or(Duration::from_secs(3 * 86400)))
            };
            let mut threads = sessions.list(&ctx.cwd, since, ctx.clock.now())?;
            if let Some((_, capabilities)) = crate::authorization::caller(ctx)?
                && capabilities.class == instrukt_core::capabilities::RoleClass::Focused
                && capabilities.scope(instrukt_core::capabilities::Action::SessionObserve)
                    == Some(instrukt_core::capabilities::Scope::Own)
            {
                let caller = config.caller(&env)?;
                threads.retain(|thread| Some(&thread.session) == caller.as_ref());
            }
            if args.json {
                ctx.out.json(&threads)?;
            } else {
                for thread in threads {
                    ctx.out.line(&format!(
                        "{} | {} | {} | {}",
                        thread.session,
                        thread.title.as_deref().unwrap_or("untitled"),
                        thread.state,
                        thread
                            .last_activity
                            .map(|t| t.to_string())
                            .unwrap_or_else(|| "unknown".into())
                    ))?;
                }
            }
            Ok(ExitCode::SUCCESS)
        }
        SessionsCommand::ResolveStart(args) => {
            sessions.runner_runtime()?;
            let key = match args.key.rsplit_once('@') {
                Some((procedure, project)) if !args.key.starts_with("instrukt:") => {
                    keyed::standing_key(procedure, project)
                }
                _ => args.key.clone(),
            };
            let invocation =
                keyed::RuntimeInvocation::acquire(&ctx.home, Duration::from_secs(120))?;
            keyed::resolve(
                &invocation,
                sessions.adapter()?.as_ref(),
                &key,
                args.session.as_ref(),
                ctx.clock.now(),
            )?;
            if args.json {
                ctx.out.json(&json!({
                    "key": key,
                    "session": args.session.as_ref().map(ToString::to_string),
                }))?;
            } else {
                match &args.session {
                    Some(session) => ctx
                        .out
                        .line(&format!("{} is bound to {session}", args.key))?,
                    None => ctx.out.line(&format!(
                        "{} has no session; the next wake-up starts one",
                        args.key
                    ))?,
                }
            }
            Ok(ExitCode::SUCCESS)
        }
    }
}

#[derive(Debug, clap::Args)]
pub struct InstallArgs {}

/// Registers `instrukt tick` every minute with the OS scheduler.
pub fn run_install(_args: InstallArgs, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    let config = Configuration::load_global(&ctx.home)?;
    if !config.ticks_enabled()? {
        ctx.out
            .line("the wake-up is off (`ticks: off`); nothing was installed")?;
        return Ok(ExitCode::SUCCESS);
    }
    let path_env = std::env::var("PATH").unwrap_or_default();
    let config_home = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from);
    let executable = executable()?;
    let scheduler = Scheduler::detect(Some(&path_env));
    let installed = install::install(
        scheduler,
        &WakeUp {
            user_home: ctx.home.user_home(),
            config_home: config_home.as_deref(),
            executable: &executable,
            path_env: &path_env,
        },
    )?;
    match &installed.file {
        Some(file) => ctx.out.line(&format!(
            "installed: `instrukt tick` every minute through {} ({})",
            installed.scheduler.name(),
            file.display()
        ))?,
        None => ctx.out.line(&format!(
            "installed: `instrukt tick` every minute through {}",
            installed.scheduler.name()
        ))?,
    }
    Ok(ExitCode::SUCCESS)
}

/// `sessions`, `run` and `tick` refuse only when both `modules.todos` and
/// `modules.events` are off: in the caller's effective configuration and
/// in every published project's.
fn require_runner_modules(ctx: &Ctx, command: &str) -> anyhow::Result<()> {
    if instrukt_runner::links::status(&ctx.home)?
        .iter()
        .any(|link| link.active)
    {
        return Ok(());
    }
    let config = Configuration::load_at(&ctx.home, &ctx.cwd)?;
    if config.module_enabled(Module::Todos)? || config.module_enabled(Module::Events)? {
        return Ok(());
    }
    let global = Configuration::load_global(&ctx.home)?;
    if global.module_enabled(Module::Todos)? || global.module_enabled(Module::Events)? {
        return Ok(());
    }
    if !runner::considered_projects(&ctx.home)?.is_empty() {
        return Ok(());
    }
    bail!(
        "`instrukt {command}` serves the todos and events modules, and both are off; switch on `modules.todos` or `modules.events`"
    )
}

/// This `instrukt` binary.
fn executable() -> anyhow::Result<PathBuf> {
    let exe = std::env::current_exe().context("cannot locate the instrukt binary")?;
    std::fs::canonicalize(&exe)
        .with_context(|| format!("cannot resolve the instrukt binary {}", exe.display()))
}
