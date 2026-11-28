//! `instrukt company reply` and `instrukt company status`: explicit frontstage
//! reply outbox and optional curated active-turn progress updates for company
//! conversation sessions.
use std::io::Read as _;
use std::process::ExitCode;

use crate::{context::Ctx, wiring};
use instrukt_company::conversations::{queue_explicit_reply, set_turn_status};
use instrukt_core::config::Configuration;

#[derive(Debug, clap::Args)]
pub struct CompanyArgs {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, clap::Subcommand)]
enum Command {
    /// Queue an explicit reply to a company conversation through the durable
    /// explicit reply outbox.
    Reply {
        /// Provider conversation ID to reply to. Optional when the connector
        /// uses `session_scope: conversation` (defaults to the bound
        /// conversation); required under `session_scope: company`.
        #[arg(long, value_name = "CONVERSATION")]
        conversation: Option<String>,
        /// Reply body. Pass `-` to read the reply from stdin.
        #[arg(long, value_name = "TEXT", allow_hyphen_values = true)]
        text: String,
        /// Emit the queued reply summary as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Publish a short curated progress update for the active addressed turn.
    Status {
        /// Provider conversation ID. Optional when the connector uses
        /// `session_scope: conversation`; required under `session_scope: company`
        /// and must match the currently active addressed turn's conversation.
        #[arg(long, value_name = "CONVERSATION")]
        conversation: Option<String>,
        /// Single-line curated status text (at most 256 UTF-8 bytes).
        #[arg(long, value_name = "TEXT", allow_hyphen_values = true)]
        text: String,
        /// Emit the updated status summary as JSON.
        #[arg(long)]
        json: bool,
    },
}

pub fn run(args: CompanyArgs, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    let env = wiring::environment(ctx)?;
    let (runtime_name, session) = match Configuration::load_global(&ctx.home.for_inspection()) {
        Ok(config) => {
            let runtime = config.runner_runtime()?;
            let session = config.caller(&env)?;
            (runtime.name().to_owned(), session)
        }
        Err(load_err) => {
            let caller = crate::runtimes::DECLARATIONS
                .iter()
                .find_map(|decl| {
                    crate::runtimes::adapter(decl.name, &env)
                        .ok()
                        .and_then(|adapter| adapter.lookup_caller(&env))
                })
                .map(|c| c.session);
            match caller {
                Some(session) => (session.runtime.as_str().to_owned(), Some(session)),
                None => return Err(load_err.into()),
            }
        }
    };
    let adapter = crate::runtimes::adapter(&runtime_name, &env)?;
    match args.command {
        Command::Reply {
            conversation,
            text,
            json,
        } => {
            let Some(session) = session else {
                anyhow::bail!(
                    "`instrukt company reply` requires a recognized active company conversation session"
                );
            };
            let body = if text == "-" {
                let mut buf = String::new();
                std::io::stdin().read_to_string(&mut buf)?;
                buf
            } else {
                text
            };
            let queued = queue_explicit_reply(
                &ctx.home,
                &session,
                adapter.as_ref(),
                conversation.as_deref(),
                &body,
                ctx.clock.now(),
            )?;
            if json {
                ctx.out.json(&queued)?;
            } else {
                ctx.out.line(&format!(
                    "Queued reply for {} ({}, {} piece(s), turn {}).",
                    queued.conversation, queued.connector, queued.pieces, queued.turn
                ))?;
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::Status {
            conversation,
            text,
            json,
        } => {
            let Some(session) = session else {
                anyhow::bail!(
                    "`instrukt company status` requires a recognized active company conversation session"
                );
            };
            let summary = set_turn_status(
                &ctx.home,
                &session,
                adapter.as_ref(),
                conversation.as_deref(),
                &text,
                ctx.clock.now(),
            )?;
            if json {
                ctx.out.json(&summary)?;
            } else {
                ctx.out.line(&format!(
                    "Updated status for {} ({}): {}",
                    summary.conversation, summary.connector, summary.text
                ))?;
            }
            Ok(ExitCode::SUCCESS)
        }
    }
}
