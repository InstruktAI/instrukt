//! Command composition for the runner's retained history.
use crate::context::Ctx;
use instrukt_runner::history;
use std::{path::PathBuf, process::ExitCode, time::Duration};

#[derive(Debug, clap::Args)]
pub struct HistoryArgs {
    #[command(subcommand)]
    command: Command,
}
#[derive(Debug, clap::Subcommand)]
enum Command {
    /// Catch up from T3's durable local state.
    Extract {
        #[arg(long)]
        json: bool,
    },
    /// Import the legacy mirror store once; subsequent runs preserve that completed import.
    ImportTeleclaude {
        #[arg(long)]
        source: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    /// Search locally and on explicitly participating cluster computer ids.
    Search {
        query: String,
        #[arg(long,default_value_t=50,value_parser=clap::value_parser!(u16).range(1..=1000))]
        limit: u16,
        #[arg(long = "computer")]
        computers: Vec<String>,
        /// Total remote reply deadline in seconds.
        #[arg(long,default_value_t=10,value_parser=clap::value_parser!(u16).range(1..=120))]
        timeout: u16,
        #[arg(long)]
        json: bool,
    },
    /// Read a retained source document by conversation id or legacy alias.
    Show {
        source: String,
        conversation: String,
    },
}
pub fn run(args: HistoryArgs, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    let env = crate::wiring::environment(ctx)?;
    match args.command {
        Command::Extract { json } => {
            let result = history::extract(&ctx.home, &env)?;
            let computer = instrukt_core::machine::machine_id(&ctx.home)?;
            if json {
                ctx.out.json(&serde_json::json!({"computer":computer,"loaded":result.loaded,"position":result.position}))?;
            } else {
                ctx.out.line(&format!(
                    "{}: loaded {} conversations; source position {}",
                    computer, result.loaded, result.position
                ))?;
            }
        }
        Command::ImportTeleclaude { source, json } => {
            let result = history::import_teleclaude(&ctx.home, source.as_deref())?;
            if json {
                ctx.out.json(&result)?;
            } else {
                ctx.out
                    .line(&format!("imported {} conversations", result.loaded))?;
            }
        }
        Command::Search {
            query,
            limit,
            computers,
            timeout,
            json,
        } => {
            let result = history::search(
                &ctx.home,
                &env,
                &history::Search {
                    query: &query,
                    limit: usize::from(limit),
                    computers: &computers,
                    timeout: Duration::from_secs(u64::from(timeout)),
                },
            )?;
            if json {
                ctx.out.json(&result)?;
            } else {
                for found in &result.matches {
                    ctx.out.line(&format!(
                        "{} (origin {}) | {} | {} | {}\n{}",
                        found.store_computer,
                        found.computer,
                        found.source,
                        found.conversation,
                        found.title,
                        found.excerpt
                    ))?;
                }
                for failure in &result.failures {
                    ctx.out.line(&format!(
                        "incomplete: {} | {} | {}",
                        failure.computer, failure.operation, failure.detail
                    ))?;
                }
            }
            if !result.failures.is_empty() {
                return Ok(ExitCode::FAILURE);
            }
        }
        Command::Show {
            source,
            conversation,
        } => ctx
            .out
            .json(&history::show(&ctx.home, &source, &conversation)?)?,
    }
    Ok(ExitCode::SUCCESS)
}
