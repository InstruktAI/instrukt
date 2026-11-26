//! `instrukt company reply`: queue an explicit reply from an active company
//! conversation session into the durable `conversation_answers` outbox.
use std::io::Read as _;
use std::process::ExitCode;

use crate::{context::Ctx, wiring};
use instrukt_company::conversations::queue_reply;
use instrukt_core::config::Configuration;

#[derive(Debug, clap::Args)]
pub struct CompanyArgs {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, clap::Subcommand)]
enum Command {
    /// Queue an explicit reply to a company conversation through the durable
    /// answer outbox.
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
}

pub fn run(args: CompanyArgs, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    match args.command {
        Command::Reply {
            conversation,
            text,
            json,
        } => {
            let env = wiring::environment(ctx)?;
            let config = Configuration::load_global(&ctx.home.for_inspection())?;
            let Some(session) = config.caller(&env)? else {
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
            let queued = queue_reply(&ctx.home, &session, conversation.as_deref(), &body)?;
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
    }
}
