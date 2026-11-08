//! Runtime facts and scoped availability; never edits provider configuration.
use crate::context::Ctx;
use instrukt_runner::agents::{self, Availability, AvailabilityStatus};
use std::process::ExitCode;

#[derive(Debug, clap::Args)]
pub struct AgentsArgs {
    #[command(subcommand)]
    command: AgentCommand,
}
#[derive(Debug, clap::Subcommand)]
enum AgentCommand {
    /// Configured profiles and native model options.
    List,
    /// Show normalized local agent/model availability; expired marks lapse.
    Availability,
    /// Mark an agent profile or one model available, unavailable or degraded.
    Status {
        /// Configured profile name.
        profile: instrukt_core::ids::ProfileName,
        /// New capacity status. Degraded capacity is skipped for automated dispatch.
        #[arg(value_enum)]
        status: Status,
        /// Restrict the mark to this model instead of the entire profile.
        #[arg(long)]
        model: Option<String>,
        /// Absolute RFC3339 expiry; required for non-available model marks.
        #[arg(long)]
        until: Option<jiff::Timestamp>,
        /// Cause of the mark; required when unavailable.
        #[arg(long)]
        reason: Option<String>,
    },
}
#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum Status {
    Available,
    Unavailable,
    Degraded,
}
pub fn run(args: AgentsArgs, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    let output = match args.command {
        AgentCommand::List => {
            serde_json::to_string_pretty(&instrukt_core::config::Agents::load(&ctx.home)?)?
        }
        AgentCommand::Availability => {
            serde_json::to_string_pretty(&agents::availability(&ctx.home, ctx.clock.now())?)?
        }
        AgentCommand::Status {
            profile,
            status,
            model,
            until,
            reason,
        } => {
            let mark = Availability {
                profile,
                model,
                status: match status {
                    Status::Available => AvailabilityStatus::Available,
                    Status::Unavailable => AvailabilityStatus::Unavailable,
                    Status::Degraded => AvailabilityStatus::Degraded,
                },
                until,
                reason,
            };
            agents::mark_availability(&ctx.home, &mark, ctx.clock.now())?;
            serde_json::to_string_pretty(&agents::availability(&ctx.home, ctx.clock.now())?)?
        }
    };
    ctx.out.line(&output)?;
    Ok(ExitCode::SUCCESS)
}
