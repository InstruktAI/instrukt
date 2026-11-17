//! `instrukt mcp list|list-tools|call`: an agent reaches the MCP servers its
//! scope declares, each call carrying the login the rules select.
use std::process::ExitCode;

use crate::{authorization, context::Ctx, wiring};
use instrukt_core::config::Configuration;
use instrukt_core::ids::ProjectSlug;
use instrukt_runner::mcp::{self, Caller, Origin, Scrub, StoredLogins, Target, View, Visible};

#[derive(Debug, clap::Args)]
pub struct McpArgs {
    /// The local operator, outside any session: see what that company's
    /// sessions see.
    #[arg(long, global = true, value_name = "NAME")]
    company: Option<String>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, clap::Subcommand)]
enum Command {
    /// Name each server this caller sees, with its purpose and where it is
    /// declared; contacts no server and reads no secret.
    List {
        #[arg(long)]
        json: bool,
    },
    /// Show one server's tools: each tool's name, description and input shape.
    ListTools {
        name: String,
        #[arg(long)]
        json: bool,
    },
    /// Call one tool with a JSON object of arguments and print its result as
    /// JSON.
    Call {
        name: String,
        tool: String,
        #[arg(long = "json", value_name = "ARGUMENTS", allow_hyphen_values = true)]
        arguments: String,
    },
}

pub fn run(args: McpArgs, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    let asked = args
        .company
        .as_deref()
        .map(ProjectSlug::parse)
        .transpose()?;
    let caller = caller(ctx)?;
    let mut view = mcp::view(&ctx.home, &caller, asked.as_ref())?;
    match args.command {
        Command::List { json } => list(ctx, &view, json),
        Command::ListTools { name, json } => {
            let visible = view.server(&name)?.clone();
            speak(ctx, &caller, &mut view)?;
            let (scrub, login) = login(ctx, &view, &visible)?;
            let started = std::time::Instant::now();
            let tools = mcp::list_tools(&target(&visible, login.as_deref()));
            completed(&visible, "tools/list", None, started, tools.is_ok());
            let tools = tools.map_err(|error| scrubbed(&scrub, &error))?;
            if json {
                ctx.out.json(&scrub.json(serde_json::to_value(&tools)?))?;
            } else if tools.is_empty() {
                ctx.out
                    .line(&format!("MCP server `{name}` offers no tools."))?;
            } else {
                for tool in &tools {
                    let line = match &tool.description {
                        Some(description) => format!("{}: {description}", tool.name),
                        None => tool.name.clone(),
                    };
                    ctx.out.line(&scrub.text(&line))?;
                    ctx.out
                        .line(&scrub.text(&format!("  input: {}", tool.input_schema)))?;
                }
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::Call {
            name,
            tool,
            arguments,
        } => {
            let arguments: serde_json::Value = serde_json::from_str(&arguments)
                .ok()
                .filter(serde_json::Value::is_object)
                .ok_or_else(|| {
                    anyhow::anyhow!("--json takes the tool's arguments as one JSON object")
                })?;
            let visible = view.server(&name)?.clone();
            speak(ctx, &caller, &mut view)?;
            let (scrub, login) = login(ctx, &view, &visible)?;
            let started = std::time::Instant::now();
            let result = mcp::call(&target(&visible, login.as_deref()), &tool, arguments);
            completed(&visible, "tools/call", Some(&tool), started, result.is_ok());
            let result = result.map_err(|error| scrubbed(&scrub, &error))?;
            ctx.out.json(&scrub.json(result))?;
            Ok(ExitCode::SUCCESS)
        }
    }
}

/// The caller the selected runtime's caller identity names, with the
/// company its recorded assignment works for.
fn caller(ctx: &Ctx) -> anyhow::Result<Caller> {
    let config = Configuration::load_global(&ctx.home)?;
    let session = config.caller(&wiring::environment(ctx)?)?;
    let company = authorization::caller(ctx)?.and_then(|(assignment, _)| assignment.company);
    Ok(Caller {
        session,
        company,
        working_in: working_in(ctx),
    })
}

/// The company home the working directory lies in, if any.
fn working_in(ctx: &Ctx) -> Option<ProjectSlug> {
    let companies = ctx.home.companies_dir().canonicalize().ok()?;
    let here = ctx.cwd.canonicalize().ok()?;
    let name = here.strip_prefix(&companies).ok()?.components().next()?;
    ProjectSlug::parse(name.as_os_str().to_str()?).ok()
}

/// Establishes the speaker of the calling session's current turn, for a
/// session working for its own company.
fn speak(ctx: &Ctx, caller: &Caller, view: &mut View) -> anyhow::Result<()> {
    let (Some(session), Some(_)) = (&caller.session, &caller.company) else {
        return Ok(());
    };
    let Some(runtime) = Configuration::load_global(&ctx.home)?.runner_runtime()? else {
        return Ok(());
    };
    let adapter = instrukt_runner::runtime::adapter(runtime);
    view.speaker = mcp::speaker(&ctx.home, session, adapter.as_ref())?;
    Ok(())
}

/// One completion record per catalog or tool operation, naming no value.
fn completed(
    visible: &Visible,
    operation: &str,
    tool: Option<&str>,
    started: std::time::Instant,
    succeeded: bool,
) {
    tracing::debug!(
        event = "mcp_operation_completed",
        server = visible.server.name.as_str(),
        operation,
        tool = tool.unwrap_or_default(),
        succeeded,
        duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
    );
}

fn list(ctx: &mut Ctx, view: &View, json: bool) -> anyhow::Result<ExitCode> {
    let declared = |visible: &Visible| match &visible.origin {
        Origin::Machine => "this computer".to_owned(),
        Origin::Company(company) => format!("company {company}"),
    };
    if json {
        let servers: Vec<_> = view
            .servers
            .values()
            .map(|visible| {
                let mut entry = serde_json::json!({
                    "name": visible.server.name.as_str(),
                    "description": visible.server.description,
                    "declared": match &visible.origin {
                        Origin::Machine => "machine",
                        Origin::Company(_) => "company",
                    },
                });
                if let Origin::Company(company) = &visible.origin {
                    entry["company"] = company.as_str().into();
                }
                entry
            })
            .collect();
        ctx.out.json(&servers)?;
    } else if view.servers.is_empty() {
        ctx.out.line("No MCP server is declared for this caller.")?;
    } else {
        for visible in view.servers.values() {
            ctx.out.line(&format!(
                "{}: {} (declared for {})",
                visible.server.name,
                visible.server.description,
                declared(visible)
            ))?;
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// The login a request to `visible` carries, and the scrub that removes it
/// from everything the server returns.
fn login(ctx: &Ctx, view: &View, visible: &Visible) -> anyhow::Result<(Scrub, Option<String>)> {
    let login = view.login(visible, &StoredLogins::new(&ctx.home))?;
    let value = login.as_ref().map(|login| login.value.clone());
    Ok((Scrub::new(login), value))
}

fn target<'a>(visible: &'a Visible, authorization: Option<&'a str>) -> Target<'a> {
    Target {
        name: visible.server.name.as_str(),
        url: &visible.server.url,
        authorization,
    }
}

/// A failure with every resolved login value removed before it is printed
/// or logged.
fn scrubbed(scrub: &Scrub, error: &mcp::McpError) -> anyhow::Error {
    anyhow::anyhow!(scrub.text(&error.to_string()))
}
