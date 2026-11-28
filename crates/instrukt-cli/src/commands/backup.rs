//! Encrypted offsite home backup, generation listing, and guarded restore
//! (`project/instrukt/spec/feature/runner/home-backup`).

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Subcommand};
use instrukt_core::machine::MachineId;
use instrukt_runner::backup::{IdentityInput, RestoreOptions};

use crate::context::Ctx;

#[derive(Debug, Args)]
pub struct BackupArgs {
    #[command(subcommand)]
    pub command: BackupCommand,
}

#[derive(Debug, Subcommand)]
pub enum BackupCommand {
    /// Snapshot irreplaceable home state, encrypt client-side, publish and prune offsite generations.
    Run,
    /// List offsite backup generations for this machine, newest first.
    List,
    /// Download, decrypt, verify every restored SQLite database and restore a backup generation.
    Restore(RestoreArgs),
}

#[derive(Debug, Args)]
pub struct RestoreArgs {
    /// Explicit remote object key to restore (permits recovering an explicit or unvalidated generation).
    #[arg(long, value_name = "KEY")]
    pub from: Option<String>,
    /// Select the newest validated generation from another computer's machine ID.
    #[arg(long, value_name = "ID", value_parser = parse_machine_id)]
    pub machine: Option<MachineId>,
    /// Extract the verified backup into an alternate directory outside the live instrukt home.
    #[arg(long, value_name = "DIR")]
    pub into: Option<PathBuf>,
    /// Path to an age private identity file, or `-` to read from stdin (prompts with echo off on a terminal when omitted).
    #[arg(long, value_name = "PATH")]
    pub identity: Option<String>,
    /// Confirm overwriting destination files without an interactive prompt.
    #[arg(long)]
    pub yes: bool,
}

fn parse_machine_id(raw: &str) -> Result<MachineId, String> {
    MachineId::parse(raw).ok_or_else(|| {
        format!(
            "`{raw}` is not a valid machine id (expected 16 to 64 lowercase hexadecimal characters)"
        )
    })
}

pub fn run(args: BackupArgs, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    match args.command {
        BackupCommand::Run => {
            let env = crate::wiring::environment(ctx)?;
            // `instrukt_runner::backup::run` validates global backup admission
            // before acquiring `state/backup.lock` or writing any files.
            let writing_home = ctx.home.for_writing();
            let report = instrukt_runner::backup::run(&writing_home, &env, ctx.clock.now())?;
            ctx.out.line(&format!(
                "published {} ({} bytes; pruned: {}, orphans deleted: {}, uploads aborted: {})",
                report.generation,
                report.size_bytes,
                report.pruned_generations.len(),
                report.deleted_orphans.len(),
                report.aborted_uploads.len(),
            ))?;
            Ok(ExitCode::SUCCESS)
        }
        BackupCommand::List => {
            let generations = instrukt_runner::backup::list(&ctx.home)?;
            if generations.is_empty() {
                ctx.out
                    .line("no backup generations found for this machine")?;
            } else {
                for generation in generations {
                    let status = if generation.validated {
                        "validated"
                    } else {
                        "unvalidated"
                    };
                    let ts = generation
                        .timestamp
                        .or(generation.last_modified)
                        .map(|t| t.to_string())
                        .unwrap_or_else(|| "unknown".into());
                    ctx.out.line(&format!(
                        "{} | {} | {} bytes | {}",
                        generation.key, status, generation.size_bytes, ts
                    ))?;
                }
            }
            Ok(ExitCode::SUCCESS)
        }
        BackupCommand::Restore(restore_args) => {
            let identity = match restore_args.identity.as_deref() {
                Some("-") => IdentityInput::Stdin,
                Some(path) => IdentityInput::Path(PathBuf::from(path)),
                None => IdentityInput::Prompt,
            };
            let options = RestoreOptions {
                from: restore_args.from,
                machine: restore_args.machine,
                into: restore_args.into,
                identity,
                yes: restore_args.yes,
            };
            let report =
                instrukt_runner::backup::restore_with_stores(&ctx.home, &options, |home| {
                    let mut stores = instrukt_todos::work::review::stores(home);
                    stores.extend(instrukt_company::proposals::stores(home)?);
                    Ok(stores)
                })?;
            ctx.out.line(&format!(
                "restored {} into {} ({} files, {} SQLite databases verified)",
                report.generation,
                report.target.display(),
                report.restored_files,
                report.verified_databases,
            ))?;
            Ok(ExitCode::SUCCESS)
        }
    }
}
