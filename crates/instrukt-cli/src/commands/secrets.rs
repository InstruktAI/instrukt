//! Secret writes accept material on stdin or from a protected input file.
use crate::context::Ctx;
use std::{
    io::{IsTerminal, Read},
    path::PathBuf,
    process::ExitCode,
};

#[derive(Debug, clap::Args)]
pub struct SecretsArgs {
    #[command(subcommand)]
    command: SecretCommand,
}
#[derive(Debug, clap::Subcommand)]
enum SecretCommand {
    /// Store a literal value from stdin, or --from-file. Never echoes it.
    Set {
        name: String,
        #[arg(long)]
        from_file: Option<PathBuf>,
    },
    /// Generate and store a cryptographically random value. Never echoes it.
    Generate { name: String },
}
pub fn run(args: SecretsArgs, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    match args.command {
        SecretCommand::Set { name, from_file } => {
            let mut value = String::new();
            match from_file {
                Some(file) => {
                    value = std::fs::read_to_string(file)
                        .map_err(|_| anyhow::anyhow!("cannot read secret input file"))?
                }
                None => {
                    if std::io::stdin().is_terminal() {
                        anyhow::bail!(
                            "use redirected stdin or --from-file for secret input; terminal input would echo material"
                        );
                    }
                    std::io::stdin()
                        .read_to_string(&mut value)
                        .map_err(|_| anyhow::anyhow!("cannot read secret stdin"))?;
                }
            }
            instrukt_core::config::set_secret(&ctx.home, &name, &value)?;
        }
        SecretCommand::Generate { name } => {
            instrukt_core::config::generate_secret(&ctx.home, &name)?
        }
    }
    Ok(ExitCode::SUCCESS)
}
