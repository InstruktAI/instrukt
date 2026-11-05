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

pub fn run(args: EventsArgs, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    let catalog = EventCatalog::builtin()?;
    match args.command {
        EventsCommand::List { domain } => {
            for declaration in catalog.list(domain.as_deref()) {
                let subject = declaration
                    .subject_kind
                    .map_or("any reference", |kind| kind.as_str());
                ctx.out.line(&format!(
                    "{} v{} — {} (subject: {})",
                    declaration.name, declaration.version, declaration.description, subject
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
