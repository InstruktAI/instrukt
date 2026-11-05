//! Publication command: `instrukt sync [--force]`.

use std::path::PathBuf;
use std::process::ExitCode;

use instrukt_core::publication::{self, Refusal, SyncError, SyncOutcome, SyncReport, SyncRequest};

use crate::context::Ctx;
use crate::wiring::publication_wiring;

#[derive(Debug, clap::Args)]
pub struct SyncArgs {
    /// Republish even when main equals the commit last published.
    #[arg(long)]
    force: bool,
}

/// Publishes the project the caller stands in from its canonical main.
///
/// Prints one line naming the outcome, then one line per editor schema or
/// modeline a publishing sync changed in the primary worktree, then one
/// line per leftover copy of an earlier publication that could not be
/// removed (a later sync removes it; it does not fail this one). A refusal or a failure exits non-zero:
/// validation findings are printed one per line, the leftovers likewise,
/// then the refusal or failure is reported on stderr.
pub fn run(args: SyncArgs, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    let wiring = publication_wiring(ctx)?;
    let synced = publication::sync(&SyncRequest {
        home: &ctx.home,
        cwd: &ctx.cwd,
        force: args.force,
        wiring: &wiring,
        now: ctx.clock.now(),
    });
    let SyncReport {
        outcome,
        leftovers,
        editor,
    } = match synced {
        Ok(report) => report,
        Err(SyncError { error, leftovers }) => {
            report_leftovers(ctx, &leftovers)?;
            return Err((*error).into());
        }
    };
    let refusal = match outcome {
        SyncOutcome::Published { project, commit } => {
            ctx.out.line(&format!("published {project} at {commit}"))?;
            None
        }
        SyncOutcome::Unchanged { project, commit } => {
            ctx.out
                .line(&format!("{project} is already published at {commit}"))?;
            None
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
            Some(format!("{project} at {commit} is not published: {refusal}"))
        }
    };
    for line in crate::commands::repository::editor_lines(&editor) {
        ctx.out.line(&line)?;
    }
    report_leftovers(ctx, &leftovers)?;
    match refusal {
        Some(message) => anyhow::bail!(message),
        None => Ok(ExitCode::SUCCESS),
    }
}

/// One line per leftover a sync could not remove.
fn report_leftovers(ctx: &mut Ctx, leftovers: &[PathBuf]) -> anyhow::Result<()> {
    for leftover in leftovers {
        ctx.out.line(&format!(
            "leftover {} could not be removed; a later sync removes it",
            leftover.display()
        ))?;
    }
    Ok(())
}
