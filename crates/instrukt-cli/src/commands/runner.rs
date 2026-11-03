//! Runner commands: `run`, `tick`, `ticks status`, `sessions …`, `install`.
//!
//! This module is the runner's composition root: it reads the clock, the
//! machine's time zone and the process environment, and hands them to
//! `instrukt_runner`.

use std::io::Read as _;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use anyhow::{Context as _, bail};
use instrukt_core::config::{Configuration, Env, Module};
use instrukt_core::ids::{SessionRef, SnippetId, TodoSlug};
use instrukt_runner::agents::{agent_order, global_order};
use instrukt_runner::install::{self, Scheduler, WakeUp};
use instrukt_runner::interfaces::{ProjectRef, SessionBinding};
use instrukt_runner::runner::{self, RunnerContext, TickOutcome};
use instrukt_runner::sessions::{Observation, Sessions, StartRequest};
use instrukt_runner::state::RunnerState;
use instrukt_runner::{launch, ticks};
use jiff::Timestamp;
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
    let env = Env::from_process();
    let tz = TimeZone::system();
    let executable = executable()?;
    let wiring = wiring::runner_wiring(ctx)?;
    let cx = RunnerContext {
        home: &ctx.home,
        env: &env,
        tz: &tz,
        executable: &executable,
        wiring: &wiring,
    };
    runner::run(&cx, Timestamp::now, RUN_INTERVAL)?;
    Ok(ExitCode::SUCCESS)
}

#[derive(Debug, clap::Args)]
pub struct TickArgs {}

/// One wake-up, as the OS scheduler invokes it every minute. Exits at once
/// while `instrukt run` holds the runner lock.
pub fn run_tick(_args: TickArgs, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    require_runner_modules(ctx, "tick")?;
    let env = Env::from_process();
    let tz = TimeZone::system();
    let executable = executable()?;
    let wiring = wiring::runner_wiring(ctx)?;
    let cx = RunnerContext {
        home: &ctx.home,
        env: &env,
        tz: &tz,
        executable: &executable,
        wiring: &wiring,
    };
    match runner::tick(&cx, Timestamp::now())? {
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
            let status = ticks::status(ticks::last_wakeup(&state)?, Timestamp::now());
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
    /// Start a session through the runtime mapping; prints its id.
    Run(SessionsRunArgs),
    /// Send text to a session; prints its id.
    Send(SendArgs),
    /// Print a session's status: running, needs-input, ended ok, ended
    /// failed or gone.
    Alive(IdArgs),
    /// Print a session's conversation.
    Conversation(IdArgs),
    /// Link two sessions of one runtime, or remove their link.
    Link(LinkArgs),
    /// End a session.
    End(IdArgs),
}

#[derive(Debug, clap::Args)]
struct SessionsRunArgs {
    /// The procedure the session starts into.
    #[arg(long)]
    procedure: Option<SnippetId>,
    /// The procedure's arguments, as one text.
    #[arg(long, allow_hyphen_values = true)]
    args: Option<String>,
    /// Additional briefing, from a file or `-` for standard input.
    #[arg(long)]
    brief: Option<String>,
    /// A worktree the runtime creates or reattaches.
    #[arg(long)]
    worktree: Option<String>,
    /// Record the session as this todo's binding for the procedure's role.
    #[arg(long)]
    todo: Option<TodoSlug>,
    /// Render as JSON.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, clap::Args)]
struct SendArgs {
    /// The session, `<runtime>:<id>`.
    session: SessionRef,
    /// The text, as one argument.
    #[arg(allow_hyphen_values = true)]
    text: String,
    #[arg(long)]
    json: bool,
}

#[derive(Debug, clap::Args)]
struct IdArgs {
    /// The session, `<runtime>:<id>`.
    session: SessionRef,
    #[arg(long)]
    json: bool,
}

#[derive(Debug, clap::Args)]
struct LinkArgs {
    a: SessionRef,
    b: SessionRef,
    /// Remove the link instead.
    #[arg(long)]
    remove: bool,
    #[arg(long)]
    json: bool,
}

pub fn run_sessions(args: SessionsArgs, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    require_runner_modules(ctx, "sessions")?;
    let env = Env::from_process();
    let config = Configuration::load_at(&ctx.home, &ctx.cwd)?;
    let sessions = Sessions::new(&config, &env);
    match args.command {
        SessionsCommand::Run(args) => sessions_run(args, ctx, &config, &sessions),
        SessionsCommand::Send(args) => {
            sessions.send(&args.session, &args.text)?;
            print_session(ctx, &args.session, args.json)?;
            Ok(ExitCode::SUCCESS)
        }
        SessionsCommand::Alive(args) => match sessions.alive(&args.session)? {
            Observation::State(state) => {
                if args.json {
                    ctx.out
                        .json(&json!({"session": args.session.to_string(), "status": state}))?;
                } else {
                    ctx.out.line(state.as_str())?;
                }
                Ok(ExitCode::SUCCESS)
            }
            Observation::Unobservable(reason) => {
                if args.json {
                    ctx.out.json(&json!({"session": args.session.to_string(), "status": "unobservable", "reason": reason}))?;
                } else {
                    ctx.out.line("unobservable")?;
                    tracing::warn!(event = "session_unobservable", session = %args.session, reason);
                }
                Ok(ExitCode::FAILURE)
            }
        },
        SessionsCommand::Conversation(args) => {
            let conversation = sessions.conversation(&args.session)?;
            if args.json {
                ctx.out.json(
                    &json!({"session": args.session.to_string(), "conversation": conversation}),
                )?;
            } else {
                ctx.out.text(&conversation)?;
            }
            Ok(ExitCode::SUCCESS)
        }
        SessionsCommand::Link(args) => {
            sessions.link(&args.a, &args.b, args.remove)?;
            if args.json {
                ctx.out.json(&json!({"a": args.a.to_string(), "b": args.b.to_string(), "linked": !args.remove}))?;
            } else if args.remove {
                ctx.out
                    .line(&format!("unlinked {} and {}", args.a, args.b))?;
            } else {
                ctx.out.line(&format!("linked {} and {}", args.a, args.b))?;
            }
            Ok(ExitCode::SUCCESS)
        }
        SessionsCommand::End(args) => {
            sessions.end(&args.session)?;
            print_session(ctx, &args.session, args.json)?;
            Ok(ExitCode::SUCCESS)
        }
    }
}

fn sessions_run(
    args: SessionsRunArgs,
    ctx: &mut Ctx,
    config: &Configuration,
    sessions: &Sessions<'_>,
) -> anyhow::Result<ExitCode> {
    let brief = args.brief.as_deref().map(read_brief).transpose()?;
    let (message, dispatch, role) = match &args.procedure {
        Some(id) => {
            let entry = launch::procedure(&ctx.home, id)?;
            let prepared = launch::prepare(&entry, args.args.as_deref(), brief.as_deref())?;
            (prepared.message, prepared.dispatch, prepared.role)
        }
        None => {
            if args.args.is_some() {
                bail!("--args needs --procedure; a setup-only run takes no arguments");
            }
            if args.todo.is_some() {
                bail!("--todo needs --procedure; a setup-only run binds nothing");
            }
            let (Some(_), Some(brief)) = (&args.worktree, brief) else {
                bail!(
                    "--procedure is required, except for a setup-only run with --worktree and --brief"
                );
            };
            (brief, None, None)
        }
    };
    let binding = match &args.todo {
        None => None,
        Some(todo) => {
            let Some(role) = role else {
                bail!(
                    "the procedure declares no `role`, so --todo has no role to bind the session under"
                );
            };
            let project = project_ref(config)?;
            let Some(work) = wiring::work_ports(ctx)? else {
                bail!("--todo records a work binding, which needs `modules.todos` on");
            };
            Some((todo.clone(), role, project, work))
        }
    };
    let (global, thinking) = global_order(config);
    let order = agent_order(None, dispatch.as_ref(), &global, thinking);
    let started = sessions.start(
        &order,
        &StartRequest {
            message: &message,
            worktree: args.worktree.as_deref(),
            cwd: Some(&ctx.cwd),
        },
    )?;
    if let Some((todo, role, project, work)) = binding {
        work.bindings
            .bind_session(
                &project,
                &todo,
                &role,
                &SessionBinding {
                    session: started.session.clone(),
                    worktree: started.worktree.clone(),
                },
            )
            .with_context(|| {
                format!(
                    "session {} started but its binding to `{todo}` was not recorded",
                    started.session
                )
            })?;
    }
    if args.json {
        ctx.out.json(&json!({
            "session": started.session.to_string(),
            "worktree": started.worktree.as_ref().map(|p| p.display().to_string()),
        }))?;
    } else {
        ctx.out.line(&started.session.to_string())?;
        if let Some(path) = &started.worktree {
            ctx.out.line(&path.display().to_string())?;
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn print_session(ctx: &mut Ctx, session: &SessionRef, json: bool) -> anyhow::Result<()> {
    if json {
        ctx.out.json(&json!({"session": session.to_string()}))
    } else {
        ctx.out.line(&session.to_string())
    }
}

/// The briefing from `source`: a file, or standard input for `-`.
fn read_brief(source: &str) -> anyhow::Result<String> {
    if source == "-" {
        let mut text = String::new();
        std::io::stdin()
            .read_to_string(&mut text)
            .context("cannot read the brief from standard input")?;
        Ok(text)
    } else {
        std::fs::read_to_string(source).with_context(|| format!("cannot read the brief {source}"))
    }
}

/// The project the caller stands in.
fn project_ref(config: &Configuration) -> anyhow::Result<ProjectRef> {
    let (Some(root), Some(slug)) = (config.project_root(), config.project()?) else {
        bail!(
            "--todo needs a project: run it inside a repository whose instrukt.yaml declares `project`"
        );
    };
    Ok(ProjectRef {
        slug,
        root: root.to_path_buf(),
    })
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
    let config = Configuration::load_at(&ctx.home, &ctx.cwd)?;
    if config.module_enabled(Module::Todos)? || config.module_enabled(Module::Events)? {
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
