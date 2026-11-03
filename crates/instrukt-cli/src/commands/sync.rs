//! Publication command: `instrukt sync [--force]`.

use std::process::ExitCode;

use instrukt_core::publication::{self, Refusal, SyncOutcome, SyncRequest};

use crate::context::Ctx;
use crate::wiring::core_wiring;

#[derive(Debug, clap::Args)]
pub struct SyncArgs {
    /// Republish even when main equals the commit last published.
    #[arg(long)]
    force: bool,
}

/// Publishes the project the caller stands in from its canonical main.
///
/// Prints one line naming the outcome; after a publication whose replaced
/// copy could not be removed, a second line naming that leftover. A
/// refusal exits non-zero: validation findings are printed one per line,
/// then the refusal is reported on stderr.
pub fn run(args: SyncArgs, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    let wiring = core_wiring(ctx);
    let outcome = publication::sync(&SyncRequest {
        home: &ctx.home,
        cwd: &ctx.cwd,
        force: args.force,
        wiring: &wiring,
    })?;
    match outcome {
        SyncOutcome::Published {
            project,
            commit,
            leftover,
        } => {
            ctx.out.line(&format!("published {project} at {commit}"))?;
            if let Some(leftover) = leftover {
                ctx.out.line(&format!(
                    "leftover {} could not be removed; a later sync removes it",
                    leftover.display()
                ))?;
            }
            Ok(ExitCode::SUCCESS)
        }
        SyncOutcome::Unchanged { project, commit } => {
            ctx.out
                .line(&format!("{project} is already published at {commit}"))?;
            Ok(ExitCode::SUCCESS)
        }
        SyncOutcome::Refused {
            project,
            commit,
            refusal,
        } => {
            if let Refusal::Findings(findings) = &refusal {
                for finding in findings {
                    ctx.out.line(&finding.to_string())?;
                }
            }
            anyhow::bail!("{project} at {commit} is not published: {refusal}")
        }
    }
}
