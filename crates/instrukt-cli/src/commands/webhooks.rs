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
    Serve {
        /// Take the service-manager-owned named loopback socket instead of binding one.
        #[arg(long)]
        socket_activation: bool,
    },
}

impl WebhooksArgs {
    pub fn managed(&self) -> bool {
        matches!(
            &self.command,
            WebhooksCommand::Serve {
                socket_activation: true
            }
        )
    }
}
pub fn run(args: WebhooksArgs, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    match args.command {
        WebhooksCommand::Serve { socket_activation } => {
            instrukt_runner::webhooks::serve_prepared(instrukt_runner::webhooks::ServeRequest {
                home: &ctx.home,
                env: &Env::from_process(),
                clock: ctx.clock.shared(),
                socket_activation,
                prepared: ctx.receiver.take(),
            })?
        }
    }
    Ok(ExitCode::SUCCESS)
}
