//! Read producer-generated declarations without enabling events or transport.

use std::process::ExitCode;

use instrukt_runner::events::EventCatalog;

use crate::context::Ctx;

#[derive(Debug, clap::Args)]
pub struct EventsArgs {
    #[command(subcommand)]
    command: EventsCommand,
}

#[derive(Debug, clap::Subcommand)]
enum EventsCommand {
    /// Reserve or bind a deliberate native standing assignment without runtime I/O.
    Bind(BindArgs),
    /// List declared event types and how to inspect their schema. No event
    /// infrastructure or enabled module is required.
    List {
        /// Select the first segment of a type name (instrukt, runner, workflow).
        #[arg(long)]
        domain: Option<String>,
    },
    /// Show a producer's complete declaration as JSON, including origins,
    /// data kinds, related references and any lifecycle state.
    Schema {
        /// The exact declared event type, as shown by `instrukt events list`.
        r#type: String,
    },
}

#[derive(Debug, clap::Args)]
struct BindArgs {
    procedure: instrukt_core::ids::SnippetId,
    #[arg(long)]
    reserve: bool,
    #[arg(long)]
    reservation: Option<String>,
    #[arg(long)]
    session: Option<instrukt_core::ids::SessionRef>,
    #[arg(long)]
    evidence: Option<String>,
    #[arg(long)]
    not_started: bool,
    #[arg(long)]
    reason: Option<String>,
    #[arg(long)]
    args: Option<String>,
    #[arg(long)]
    brief: Option<String>,
    #[arg(long)]
    json: bool,
}
fn input(source: &str) -> anyhow::Result<String> {
    use std::io::Read as _;
    if source == "-" {
        let mut text = String::new();
        std::io::stdin().read_to_string(&mut text)?;
        Ok(text)
    } else {
        Ok(std::fs::read_to_string(source)?)
    }
}
pub fn run(args: EventsArgs, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    let catalog = EventCatalog::builtin()?;
    match args.command {
        EventsCommand::Bind(args) => {
            let config = instrukt_core::config::Configuration::load_at(&ctx.home, &ctx.cwd)?;
            let env = crate::wiring::environment(ctx)?;
            let invocation = instrukt_runner::keyed::RuntimeInvocation::acquire(
                &ctx.home,
                std::time::Duration::from_secs(120),
            )?;
            if args.reserve {
                if args.reservation.is_some()
                    || args.session.is_some()
                    || args.evidence.is_some()
                    || args.not_started
                {
                    anyhow::bail!("--reserve takes no binding or resolution fields");
                }
                let entry = instrukt_runner::launch::procedure(&ctx.home, &args.procedure)?;
                let brief = args.brief.as_deref().map(input).transpose()?;
                let result = instrukt_runner::standing::reserve_manual(
                    &invocation,
                    &instrukt_runner::standing::ManualEntry {
                        config: &config,
                        env: &env,
                        entry: &entry,
                        args: args.args.as_deref(),
                        brief: brief.as_deref(),
                    },
                )?;
                ctx.out.json(&result)?;
            } else {
                let token = args
                    .reservation
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("--reservation is required"))?;
                let evidence = args
                    .evidence
                    .as_deref()
                    .map(input)
                    .transpose()?
                    .map(|text| {
                        serde_json::from_str::<instrukt_runner::standing::ManualEvidence>(&text)
                    })
                    .transpose()?;
                if args.not_started {
                    if args.session.is_some() || evidence.is_some() {
                        anyhow::bail!("--not-started takes only --reason");
                    }
                } else if evidence
                    .as_ref()
                    .is_none_or(|e| Some(&e.session) != args.session.as_ref())
                {
                    anyhow::bail!("--session and matching --evidence are required");
                }
                instrukt_runner::standing::bind_manual(
                    &invocation,
                    &instrukt_runner::standing::ManualBinding {
                        config: &config,
                        env: &env,
                        procedure: args.procedure.as_str(),
                        token,
                        evidence: evidence.as_ref(),
                        reason: args.reason.as_deref(),
                        now: ctx.clock.now(),
                    },
                )?;
                ctx.out.json(&serde_json::json!({"reservation":token,"session":args.session.map(|s|s.to_string())}))?;
            }
        }

        EventsCommand::List { domain } => {
            for declaration in catalog.list(domain.as_deref()) {
                let subject = declaration
                    .subject_kind
                    .map_or("any reference", |kind| kind.as_str());
                ctx.out.line(&format!(
                    "{} v{} — {} (subject: {}, delivery: {:?})",
                    declaration.name,
                    declaration.version,
                    declaration.description,
                    subject,
                    declaration.delivery_kind
                ))?;
                ctx.out
                    .line(&format!("  instrukt events schema {}", declaration.name))?;
            }
        }
        EventsCommand::Schema { r#type } => {
            let declaration = catalog.get(&r#type).ok_or_else(|| anyhow::anyhow!("unknown event type `{type}`; use `instrukt events list` to discover declared types"))?;
            ctx.out.line(&serde_json::to_string_pretty(declaration)?)?;
        }
    }
    Ok(ExitCode::SUCCESS)
}
