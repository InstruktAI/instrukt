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
use instrukt_runner::install::{Scheduler, WakeUp};
use instrukt_runner::runner::{self, RunnerContext, TickOutcome};
use instrukt_runner::sessions::Sessions;

use jiff::tz::TimeZone;

use crate::context::Ctx;
use crate::wiring;

/// The interval of `instrukt run`'s wake-ups, the same minute the OS
/// scheduler's wake-up uses.
const RUN_INTERVAL: Duration = Duration::from_secs(60);

#[derive(Debug, clap::Args)]
pub struct RunArgs {
    /// Run as instrukt's owned runner service: wait for a run started by
    /// hand instead of failing. The service definition passes it.
    #[arg(long, hide = true)]
    service: bool,
}

/// Runs the runner until stopped: a wake-up every minute, holding the
/// runner lock.
pub fn run_run(args: RunArgs, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    require_runner_modules(ctx, "run")?;
    let env = crate::wiring::environment(ctx)?;
    let tz = TimeZone::system();
    let wiring = wiring::runner_wiring(ctx)?;
    let reports = instrukt_runner::review::ProposalReports::default();
    let cx = RunnerContext {
        home: &ctx.home,
        env: &env,
        tz: &tz,
        wiring: &wiring,
        reports: &reports,
    };
    let clock = ctx.clock.clone();
    runner::run(&cx, move || clock.now(), RUN_INTERVAL, args.service)?;
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
    let reports = instrukt_runner::review::ProposalReports::default();
    let cx = RunnerContext {
        home: &ctx.home,
        env: &env,
        tz: &tz,
        wiring: &wiring,
        reports: &reports,
    };
    let clock = ctx.clock.clone();
    match runner::tick(&cx, &move || clock.now())? {
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
            let status = with_installation(ctx, true, None, |request| {
                instrukt_runner::installation::ticks_status(request, ctx.clock.now())
            })?;
            if args.json {
                ctx.out.json(&status)?;
            } else {
                ctx.out.line(&format!("registration: {}", status.mode))?;
                match status.last_wakeup {
                    None => ctx.out.line("no wake-up ever recorded")?,
                    Some(last) => ctx.out.line(&format!("last wake-up: {last}"))?,
                }
                if status.stale {
                    ctx.out.line(
                        "warning: no wake-up for more than five minutes; check `instrukt install`",
                    )?;
                }
                for finding in status.registration.findings {
                    ctx.out.line(&format!("unmet: {finding}"))?;
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
    }
}

#[derive(Debug, clap::Args)]
#[command(
    after_help = "Scheduler locations: SYSTEMD_UNIT_PATH selects the native systemd search path (the running manager must include the first explicit absolute directory). INSTRUKT_LAUNCH_AGENTS_DIR selects an absolute plist directory, default ~/Library/LaunchAgents; alternate directories are loaded explicitly and are not login-scanned automatically. INSTRUKT_CRONTAB_FILE selects an absolute file-only table, never account registration; unset uses the executing user's native crontab."
)]
pub struct InstallArgs {
    /// Compare desired and actual infrastructure without changing packages, services or data.
    #[arg(long)]
    check: bool,
    /// Limit convergence to an enabled module's dependencies. Full demand still protects shared resources.
    #[arg(long, value_parser = ["todos", "events", "history", "blueprint", "conversations"])]
    module: Option<String>,
    /// Render per-resource results and findings as JSON.
    #[arg(long)]
    json: bool,
}

/// Converge infrastructure demanded by adopted projects; modules off withdraw
/// only unneeded owned units. Resolve reported prerequisites and rerun this command.
pub fn run_install(args: InstallArgs, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    let module = args.module.as_deref().and_then(Module::from_key);
    let conversations = args.module.as_deref() == Some("conversations");
    let report = with_installation(ctx, args.check, module, |request| {
        instrukt_runner::installation::converge(&instrukt_runner::installation::Request {
            conversations,
            ..request.clone()
        })
    })?;
    let healthy = report.healthy();
    if args.json {
        ctx.out.json(&report)?;
    } else {
        for notice in &report.notices {
            ctx.out.line(&format!("notice: {notice}"))?;
        }
        if report.resources.is_empty() && report.findings.is_empty() {
            ctx.out
                .line("no enabled adopted project demands background infrastructure")?;
        }
        for resource in report.resources {
            ctx.out.line(&format!(
                "{}: {} | {} | projects: {} | {}",
                resource.disposition,
                resource.name,
                resource
                    .paths
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", "),
                resource.projects.join(", "),
                resource.detail
            ))?;
        }
        for finding in report.findings {
            ctx.out.line(&format!("unmet: {finding}"))?;
        }
    }
    Ok(if healthy {
        ExitCode::SUCCESS
    } else if args.check {
        ExitCode::FAILURE
    } else {
        crate::output::could_not(if args.json {
            "the installation is incomplete; resolve each entry of the output's `findings` and run `instrukt install` again"
        } else {
            "the installation is incomplete; resolve each `unmet:` line above and run `instrukt install` again"
        })
    })
}

pub fn with_installation<T, E>(
    ctx: &Ctx,
    check: bool,
    module: Option<Module>,
    action: impl FnOnce(&instrukt_runner::installation::Request<'_>) -> Result<T, E>,
) -> anyhow::Result<T>
where
    E: Into<anyhow::Error>,
{
    let path_env = std::env::var("PATH").unwrap_or_default();
    let executable = executable()?;
    let company = crate::wiring::company();
    // An inspection reports what it can read: a machine layer that cannot
    // be read leaves it without the caller's identity, and the inspection
    // names what it could not read.
    let env = match crate::wiring::environment(ctx) {
        Ok(env) => env,
        Err(error) if check => {
            tracing::debug!(event = "inspection_without_caller", %error);
            instrukt_core::config::Env::from_process()
        }
        Err(error) => return Err(error),
    };
    action(&instrukt_runner::installation::Request {
        home: &ctx.home,
        cwd: &ctx.cwd,
        env: &env,
        scheduler: Scheduler::detect(Some(&path_env)),
        wake: WakeUp {
            home: &ctx.home,
            locations: instrukt_runner::install::SchedulerLocations::resolve(&ctx.home, &env)?,
            executable: &executable,
            path_env: &path_env,
        },
        check,
        module,
        conversations: false,
        github: ctx.github.as_ref(),
        now: ctx.clock.now(),
        company: Some(company.as_ref()),
    })
    .map_err(Into::into)
}

/// `sessions`, `run` and `tick` refuse only when both `modules.todos` and
/// `modules.events` are off: in the caller's effective configuration and
/// in every published project's.
fn require_runner_modules(ctx: &Ctx, command: &str) -> anyhow::Result<()> {
    let config = Configuration::load_at(&ctx.home, &ctx.cwd)?;
    if command == "run" && instrukt_runner::history::enabled(&ctx.home)? {
        return Ok(());
    }
    // Conversations are `run`'s alone: a company home's declared connector
    // admits it; no tick hosts a connector. The sessions conversations start
    // are resolved through `sessions`.
    if matches!(command, "run" | "sessions")
        && crate::wiring::company()
            .demand(&instrukt_core::config::Admitted::load(&ctx.home)?.generation)
    {
        return Ok(());
    }
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
    if command == "run" {
        bail!(
            "`instrukt run` serves the todos, events and history modules and conversations, and all are off; switch on `modules.todos`, `modules.events` or `modules.history`, or declare a connector in a company home"
        );
    }
    if command == "sessions" {
        bail!(
            "`instrukt sessions` serves the todos and events modules and the sessions conversations start, and all are off; switch on `modules.todos` or `modules.events`, or declare a connector in a company home"
        );
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
