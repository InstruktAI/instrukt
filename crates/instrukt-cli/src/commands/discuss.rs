//! Open and close the calling native thread's conversation link.
use std::process::ExitCode;

use crate::{context::Ctx, wiring};
use instrukt_core::{config::Configuration, ids::SessionRef};
use instrukt_runner::{
    links::{self, Discussion, Operation},
    sessions::SessionsError,
};

#[derive(Debug, clap::Args)]
pub struct DiscussArgs {
    #[command(subcommand)]
    command: Command,
}
#[derive(Debug, clap::Subcommand)]
enum Command {
    /// Join this thread to one peer; completed turns then cross automatically.
    Open {
        thread: String,
        #[arg(long, allow_hyphen_values = true)]
        message: Option<String>,
    },
    /// End this thread's discussion for both members, optionally sending a final contribution.
    Close {
        #[arg(long, allow_hyphen_values = true)]
        message: Option<String>,
    },
}
pub fn run(args: DiscussArgs, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    let runtime = Configuration::load_global(&ctx.home)?
        .runner_runtime()?
        .ok_or(SessionsError::NoRuntime)?;
    let adapter = instrukt_runner::runtime::adapter(runtime);
    let env = wiring::environment(ctx)?;
    let target;
    let operation = match &args.command {
        Command::Open { thread, message } => {
            target = if thread.contains(':') {
                SessionRef::parse(thread)?
            } else {
                SessionRef::parse(&format!("{}:{thread}", runtime.name()))?
            };
            Operation::Open {
                target: &target,
                message: message.as_deref(),
            }
        }
        Command::Close { message } => Operation::Close {
            message: message.as_deref(),
        },
    };
    let result = links::manage(
        &ctx.home,
        instrukt_todos::work::recorded_assignment,
        &Discussion {
            adapter: adapter.as_ref(),
            env: &env,
            operation,
            now: ctx.clock.now(),
        },
    )?;
    ctx.out.line(&result)?;
    Ok(ExitCode::SUCCESS)
}
