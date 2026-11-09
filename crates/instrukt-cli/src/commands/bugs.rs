//! Project-scoped retained known-issue dispositions, independent of task scans.
use crate::context::Ctx;
use instrukt_core::config::Configuration;
use instrukt_todos::work::suppressions::KnownIssues;
use std::process::ExitCode;
#[derive(Debug, clap::Args)]
pub struct BugsArgs {
    #[command(subcommand)]
    command: Command,
}
#[derive(Debug, clap::Subcommand)]
enum Command {
    /// Retain an exact signal identity and its known-issue reason for this project.
    Suppress {
        signal: String,
        #[arg(long)]
        reason: String,
    },
    /// Read the complete retained set before classifying signals; read failures stop filing.
    Suppressions {
        #[arg(long)]
        json: bool,
    },
    /// Remove a disposition so future signals are eligible for ordinary filing checks.
    Unsuppress { id: String },
}
pub fn run(args: BugsArgs, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    let config = Configuration::load_at(&ctx.home, &ctx.cwd)?;
    let project = config
        .project()?
        .ok_or_else(|| anyhow::anyhow!("known issues require an adopted project"))?;
    let store = KnownIssues::new(&ctx.home, &project);
    match args.command {
        Command::Suppress { signal, reason } => {
            ctx.out.json(&store.retain(&signal, &reason)?)?;
        }
        Command::Suppressions { json } => {
            let records = store.list()?;
            if json {
                ctx.out.json(&records)?;
            } else {
                for record in records {
                    ctx.out.line(&format!(
                        "{} | {} | {}",
                        record.id, record.signal, record.reason
                    ))?;
                }
            }
        }
        Command::Unsuppress { id } => {
            store.remove(&id)?;
            ctx.out.line(&format!("removed suppression {id}"))?;
        }
    }
    Ok(ExitCode::SUCCESS)
}
