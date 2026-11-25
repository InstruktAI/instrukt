//! `instrukt mcp list|list-tools|call|clients|discover|check-ingress|issuer|sign-out`:
//! an agent reaches the MCP servers its scope declares, each call carrying the
//! login the rules select.
use std::process::ExitCode;

use crate::{authorization, context::Ctx, wiring};
use instrukt_core::config::{Configuration, SpeakerLogin};
use instrukt_core::ids::ProjectSlug;
use instrukt_runner::mcp::{self, Caller, McpError, Origin, Scrub, Target, View, Visible};
use jiff::Timestamp;

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
    /// List global reusable host-level OAuth clients (`mcp_clients.<name>`) with
    /// their non-secret client IDs and stable shared return addresses.
    Clients {
        #[arg(long)]
        json: bool,
    },
    /// Probe a remote MCP server's OAuth/OIDC/EMA metadata live without writing
    /// any configuration or state.
    Discover {
        url: String,
        #[arg(long)]
        json: bool,
    },
    /// Verify local `ingress.url` and cluster Redis reachability through the real
    /// sign-in return path without inspecting any other computer's configuration.
    CheckIngress {
        #[arg(long)]
        json: bool,
    },
    /// Manage a company's built-in lightweight OIDC issuer (`sign_in.mode: issuer`).
    Issuer {
        #[command(subcommand)]
        command: IssuerCommand,
    },
    /// Remove a speaker's signed-in login(s) and cancel open sign-ins.
    SignOut {
        /// Server name; omit to sign out of every server of the company.
        name: Option<String>,
        /// Local operator only (outside any session): email address to sign out.
        #[arg(long)]
        email: Option<String>,
    },
}

#[derive(Debug, clap::Subcommand)]
enum IssuerCommand {
    /// Local operator only: initialize or rotate a company's ES256 OIDC issuer key
    /// and publish its public discovery/JWKS metadata to cluster Redis.
    Init {
        #[arg(long)]
        json: bool,
    },
    /// Local operator only: idempotently republish derived public discovery/JWKS
    /// metadata from `company_issuers` to cluster Redis without rotating keys.
    Publish {
        #[arg(long)]
        json: bool,
    },
    /// Show a company's built-in OIDC issuer URL and JWKS URL.
    Show {
        #[arg(long)]
        json: bool,
    },
}

pub fn run(args: McpArgs, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    let asked = args
        .company
        .as_deref()
        .map(ProjectSlug::parse)
        .transpose()?;
    let caller = caller(ctx)?;

    match args.command {
        Command::Clients { json } => {
            let is_session = require_operator_or_admin_dm(ctx, &caller, asked.as_ref())?;
            // The local operator's listing materializes registrations: it
            // writes. An admin's direct message stays a read.
            if !is_session {
                ctx.enter_writing()?;
            }
            let clients = if let Some(ref company) = asked {
                mcp::list_company_clients(&ctx.home, company, is_session, Timestamp::now())?
            } else {
                mcp::list_host_clients(&ctx.home, is_session, Timestamp::now())?
            };
            if json {
                ctx.out.json(&clients)?;
            } else if clients.is_empty() {
                if let Some(ref company) = asked {
                    ctx.out.line(&format!(
                        "No company-declared MCP OAuth client (`sign_in.client`) for company `{company}`."
                    ))?;
                } else {
                    ctx.out
                        .line("No host-level MCP OAuth client is declared.")?;
                }
            } else {
                for client in &clients {
                    let desc = client
                        .description
                        .as_deref()
                        .map(|d| format!(" — {d}"))
                        .unwrap_or_default();
                    let status = if client.initialized {
                        let id = client.client_id.as_deref().unwrap_or("(unset)");
                        let ret = client
                            .return_address
                            .as_deref()
                            .unwrap_or("(set ingress.url to form return address)");
                        let iss = if client.issuer.is_empty() {
                            "(pins on first sign-in)"
                        } else {
                            client.issuer.as_str()
                        };
                        format!("issuer={iss} client_id={id} return_address={ret}")
                    } else if let Some(ref company) = asked {
                        format!(
                            "(not initialized yet; local operator must run `instrukt mcp clients --company {company}`)"
                        )
                    } else {
                        format!(
                            "issuer={} (not initialized yet; local operator must run `instrukt mcp clients`)",
                            client.issuer
                        )
                    };
                    ctx.out.line(&format!("{}: {status}{desc}", client.name))?;
                }
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::Discover { url, json } => {
            let _ = require_operator_or_admin_dm(ctx, &caller, asked.as_ref())?;
            let report = mcp::discover(&url)?;
            if json {
                ctx.out.json(&report)?;
            } else {
                ctx.out.line(&format!("url: {}", report.url))?;
                if let Some(ref resource) = report.resource {
                    ctx.out.line(&format!("resource: {resource}"))?;
                }
                ctx.out.line(&format!(
                    "authorization_servers: {}",
                    if report.authorization_servers.is_empty() {
                        "none".to_owned()
                    } else {
                        report.authorization_servers.join(", ")
                    }
                ))?;
                ctx.out.line(&format!(
                    "scopes_supported: {}",
                    if report.scopes_supported.is_empty() {
                        "none".to_owned()
                    } else {
                        report.scopes_supported.join(", ")
                    }
                ))?;
                ctx.out.line(&format!(
                    "registration_supported: {}",
                    report.registration_supported
                ))?;
                ctx.out.line(&format!(
                    "pkce_s256_supported: {}",
                    report.pkce_s256_supported
                ))?;
                ctx.out.line(&format!(
                    "iss_parameter_supported: {}",
                    report.iss_parameter_supported
                ))?;
                ctx.out
                    .line(&format!("openid_provider: {}", report.openid_provider))?;
                ctx.out.line(&format!(
                    "ema_jwt_bearer_supported: {}",
                    report.ema_jwt_bearer_supported
                ))?;
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::CheckIngress { json } => {
            let _ = require_operator_or_admin_dm(ctx, &caller, asked.as_ref())?;
            let report = mcp::check_ingress(&ctx.home)?;
            if json {
                ctx.out.json(&report)?;
            } else {
                ctx.out.line(&format!(
                    "ingress reachable: {} (cluster Redis ok, sign-in return probe verified)",
                    report.ingress_url
                ))?;
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::Issuer { command } => match command {
            IssuerCommand::Init { json } => {
                if caller.session.is_some() {
                    anyhow::bail!(
                        "issuer init refused: `instrukt mcp issuer init` is for the local operator outside any session"
                    );
                }
                let company = asked.ok_or_else(|| {
                    anyhow::anyhow!("`instrukt mcp issuer init` requires `--company <name>`")
                })?;
                let info = mcp::init_company_issuer(&ctx.home, &company, Timestamp::now())?;
                if json {
                    ctx.out.json(&info)?;
                } else {
                    ctx.out.line(&format!("company: {}", info.company))?;
                    ctx.out.line(&format!("issuer_url: {}", info.issuer_url))?;
                    ctx.out.line(&format!("jwks_url: {}", info.jwks_url))?;
                }
                Ok(ExitCode::SUCCESS)
            }
            IssuerCommand::Publish { json } => {
                if caller.session.is_some() {
                    anyhow::bail!(
                        "issuer publish refused: `instrukt mcp issuer publish` is for the local operator outside any session"
                    );
                }
                let published = mcp::republish_company_issuers(&ctx.home, asked.as_ref())?;
                if json {
                    ctx.out.json(&published)?;
                } else if published.is_empty() {
                    ctx.out.line("No enabled company OIDC issuer to publish.")?;
                } else {
                    for info in &published {
                        ctx.out.line(&format!(
                            "{}: issuer_url={} jwks_url={}",
                            info.company, info.issuer_url, info.jwks_url
                        ))?;
                    }
                }
                Ok(ExitCode::SUCCESS)
            }
            IssuerCommand::Show { json } => {
                let is_session = require_operator_or_admin_dm(ctx, &caller, asked.as_ref())?;
                let company = if is_session {
                    caller.company.clone().ok_or_else(|| {
                        anyhow::anyhow!("company-agent session has no company binding")
                    })?
                } else {
                    asked.ok_or_else(|| {
                        anyhow::anyhow!("`instrukt mcp issuer show` requires `--company <name>`")
                    })?
                };
                let info = mcp::show_company_issuer(&ctx.home, &company)?;
                if json {
                    ctx.out.json(&info)?;
                } else {
                    ctx.out.line(&format!("company: {}", info.company))?;
                    ctx.out.line(&format!("issuer_url: {}", info.issuer_url))?;
                    ctx.out.line(&format!("jwks_url: {}", info.jwks_url))?;
                    ctx.out.line(&format!("enabled: {}", info.enabled))?;
                }
                Ok(ExitCode::SUCCESS)
            }
        },
        Command::SignOut { name, email } => {
            let server = name.as_deref().map(ProjectSlug::parse).transpose()?;
            if caller.session.is_some() {
                if email.is_some() {
                    anyhow::bail!(
                        "`--email` is for the local operator outside any session; a session signs out only its active speaker"
                    );
                }
                let mut view = mcp::view(&ctx.home, &caller, asked.as_ref())?;
                speak(ctx, &caller, &mut view)?;
                let company = view.company.ok_or_else(|| {
                    anyhow::anyhow!("sign-out is refused in a session with no company")
                })?;
                let speaker = view.speaker.ok_or_else(|| {
                    anyhow::anyhow!("sign-out is refused in a session with no speaker")
                })?;
                let addr = speaker.email.ok_or_else(|| {
                    anyhow::anyhow!("speaker has no recorded email address to sign out")
                })?;
                mcp::sign_out(
                    &ctx.home,
                    &company,
                    &addr,
                    server.as_ref(),
                    Timestamp::now(),
                )?;
            } else {
                let company = asked.ok_or_else(|| {
                    anyhow::anyhow!(
                        "`instrukt mcp sign-out` outside a session requires `--company <name>` and `--email <address>`"
                    )
                })?;
                let addr = email.ok_or_else(|| {
                    anyhow::anyhow!(
                        "`instrukt mcp sign-out` outside a session requires `--email <address>`"
                    )
                })?;
                mcp::sign_out(
                    &ctx.home,
                    &company,
                    &addr,
                    server.as_ref(),
                    Timestamp::now(),
                )?;
            }
            ctx.out.line("Signed out.")?;
            Ok(ExitCode::SUCCESS)
        }
        Command::List { json } => {
            let view = mcp::view(&ctx.home, &caller, asked.as_ref())?;
            list(ctx, &view, json)
        }
        Command::ListTools { name, json } => {
            let mut view = mcp::view(&ctx.home, &caller, asked.as_ref())?;
            let visible = view.server(&name)?.clone();
            speak(ctx, &caller, &mut view)?;
            let started = std::time::Instant::now();
            let (scrub, tools) = with_sign_in_retry(ctx, &view, &visible, |auth| {
                mcp::list_tools(&target(&visible, auth))
            })?;
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
            let mut view = mcp::view(&ctx.home, &caller, asked.as_ref())?;
            let visible = view.server(&name)?.clone();
            speak(ctx, &caller, &mut view)?;
            let started = std::time::Instant::now();
            let (scrub, result) = with_sign_in_retry(ctx, &view, &visible, |auth| {
                mcp::call(&target(&visible, auth), &tool, arguments.clone())
            })?;
            completed(&visible, "tools/call", Some(&tool), started, result.is_ok());
            let result = result.map_err(|error| scrubbed(&scrub, &error))?;
            ctx.out.json(&scrub.json(result))?;
            Ok(ExitCode::SUCCESS)
        }
    }
}

/// Execute an MCP network operation, renewing a `speaker_login: sign-in` Bearer
/// token at most once if the server answers HTTP 401.
fn with_sign_in_retry<T>(
    ctx: &Ctx,
    view: &View,
    visible: &Visible,
    mut op: impl FnMut(Option<&str>) -> Result<T, McpError>,
) -> anyhow::Result<(Scrub, Result<T, McpError>)> {
    let (scrub, login) = view.resolve_call_login(&ctx.home, visible, false, Timestamp::now())?;
    let outcome = op(login.as_deref());
    if let Err(McpError::Unauthorized { .. }) = &outcome
        && visible.server.speaker_login == SpeakerLogin::SignIn
        && view.speaker.is_some()
    {
        let (scrub2, login2) =
            view.resolve_call_login(&ctx.home, visible, true, Timestamp::now())?;
        let retried = op(login2.as_deref());
        return Ok((scrub2, retried));
    }
    Ok((scrub, outcome))
}

/// For management read/probe verbs (`clients`, `discover`, `check-ingress`,
/// `issuer show`), allow the local operator outside any session, or a verified
/// workspace administrator in a company-agent direct message. Returns `true`
/// when invoked from a company-agent session.
fn require_operator_or_admin_dm(
    ctx: &Ctx,
    caller: &Caller,
    asked: Option<&ProjectSlug>,
) -> anyhow::Result<bool> {
    let Some(session) = &caller.session else {
        return Ok(false);
    };
    let Some(own) = &caller.company else {
        if let Some(company) = &caller.working_in {
            return Err(McpError::Unbound {
                company: company.to_string(),
            }
            .into());
        }
        return Ok(false);
    };
    if let Some(asked) = asked
        && asked != own
    {
        return Err(McpError::OtherCompany {
            asked: asked.to_string(),
            own: own.to_string(),
        }
        .into());
    }
    let runtime = Configuration::load_global(&ctx.home.for_inspection())?.runner_runtime()?;
    let adapter = instrukt_runner::runtime::adapter(runtime);
    let speaker = instrukt_company::conversations::speaker(&ctx.home, session, adapter.as_ref())?
        .ok_or_else(|| {
            anyhow::anyhow!(
                "refused: this command requires a verified workspace administrator speaking in a direct message, or the local operator outside any session"
            )
        })?;
    if !speaker.is_admin || !speaker.is_direct {
        anyhow::bail!(
            "refused: this command requires a verified workspace administrator speaking in a direct message, or the local operator outside any session"
        );
    }
    Ok(true)
}

/// The caller the selected runtime's caller identity names, with the
/// company its recorded assignment works for.
fn caller(ctx: &Ctx) -> anyhow::Result<Caller> {
    let inspection = ctx.home.for_inspection();
    let config = Configuration::load_global(&inspection)?;
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
    let runtime = Configuration::load_global(&ctx.home.for_inspection())?.runner_runtime()?;
    let adapter = instrukt_runner::runtime::adapter(runtime);
    view.speaker = instrukt_company::conversations::speaker(&ctx.home, session, adapter.as_ref())?;
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
        let mut servers = Vec::with_capacity(view.servers.len());
        for visible in view.servers.values() {
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
                if let Some(hint) =
                    mcp::server_registration_hint(&ctx.home, company, &visible.server)?
                {
                    match hint {
                        mcp::ServerRegistrationHint::Initialized {
                            client_id,
                            return_address,
                        } => {
                            entry["client_id"] = client_id.into();
                            if let Some(addr) = return_address {
                                entry["return_address"] = addr.into();
                            }
                        }
                        mcp::ServerRegistrationHint::Uninitialized { message } => {
                            entry["registration_status"] = message.into();
                        }
                    }
                }
            }
            servers.push(entry);
        }
        ctx.out.json(&servers)?;
    } else if view.servers.is_empty() {
        ctx.out.line("No MCP server is declared for this caller.")?;
    } else {
        for visible in view.servers.values() {
            let extra = match &visible.origin {
                Origin::Company(company) => {
                    match mcp::server_registration_hint(&ctx.home, company, &visible.server)? {
                        Some(mcp::ServerRegistrationHint::Initialized {
                            client_id,
                            return_address: Some(addr),
                        }) => format!(" [client_id={client_id}, return={addr}]"),
                        Some(mcp::ServerRegistrationHint::Initialized {
                            client_id,
                            return_address: None,
                        }) => format!(" [client_id={client_id}]"),
                        Some(mcp::ServerRegistrationHint::Uninitialized { message }) => {
                            format!(" [{message}]")
                        }
                        None => String::new(),
                    }
                }
                Origin::Machine => String::new(),
            };
            ctx.out.line(&format!(
                "{}: {} (declared for {}){extra}",
                visible.server.name,
                visible.server.description,
                declared(visible)
            ))?;
        }
    }
    Ok(ExitCode::SUCCESS)
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
