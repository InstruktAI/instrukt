//! Internal ingress process; no installation side effects.
use crate::context::Ctx;
use instrukt_core::config::Env;
use std::process::ExitCode;

#[derive(Debug, clap::Args)]
pub struct WebhooksArgs {
    #[command(subcommand)]
    command: WebhooksCommand,
}

#[derive(Debug, clap::Subcommand)]
enum WebhooksCommand {
    /// Serve declared signed native/GitHub endpoints on ingress.port at 127.0.0.1.
    /// Requires this computer's stable ID to equal global ingress.computer.
    Serve,
}

pub fn run(args: WebhooksArgs, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    match args.command {
        WebhooksCommand::Serve => {
            instrukt_runner::webhooks::serve(&ctx.home, &Env::from_process(), ctx.clock.shared())?
        }
    }
    Ok(ExitCode::SUCCESS)
}
