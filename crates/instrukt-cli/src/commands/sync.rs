//! Publication command: `instrukt sync`.

use std::path::PathBuf;
use std::process::ExitCode;

use instrukt_core::publication::{
    self, EnsuredLink, GlobalDestination, LinkState, Refusal, SyncError, SyncFinding, SyncOutcome,
    SyncReport, SyncRequest,
};

use crate::context::Ctx;
use crate::wiring::publication_wiring;

#[derive(Debug, clap::Args)]
pub struct SyncArgs {}

/// Publishes the project the caller stands in from its canonical main.
///
/// Prints one line naming the outcome, then one `notice:` line per notice
/// of a publishing sync (a domain without `index.yaml`, a root master
/// without `docs/project/index.yaml`), one line per global agent file it
/// wrote or found current with the program that reads it, one per symbolic
/// link it replaced at one of its own destinations, one per finding about
/// an agent file (a settings file left untouched, a reference a global file
/// keeps), one line per editor schema a publishing sync changed
/// in the primary worktree, then one line per leftover copy of an earlier publication that could not be
/// removed (a later sync removes it; it does not fail this one). A refusal or a failure exits non-zero:
/// validation findings are printed one per line, the leftovers likewise,
/// then the refusal or failure is reported on stderr.
pub fn run(_args: SyncArgs, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    let wiring = publication_wiring(ctx)?;
    let synced = publication::sync(&SyncRequest {
        home: &ctx.home,
        cwd: &ctx.cwd,
        wiring: &wiring,
        now: ctx.clock.now(),
    });
    let SyncReport {
        outcome,
        leftovers,
        editor,
        notices,
        global_files,
        links,
        replaced_links,
        findings,
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
            ctx.out.line(&format!(
                "published {project} at {commit}, nothing to change"
            ))?;
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
    for notice in &notices {
        ctx.out.line(&format!("notice: {notice}"))?;
    }
    for line in global_lines(&global_files, &links, &replaced_links, &findings) {
        ctx.out.line(&line)?;
    }
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

/// One line per global agent file a sync wrote or found current, with its
/// runtime and the instances it serves; per link it ensured; per link it
/// replaced at one of its own destinations; and per finding about an agent
/// file.
fn global_lines(
    global_files: &[GlobalDestination],
    links: &[EnsuredLink],
    replaced_links: &[PathBuf],
    findings: &[SyncFinding],
) -> Vec<String> {
    let mut lines = Vec::new();
    for file in global_files {
        let state = if file.written { "written" } else { "current" };
        lines.push(format!(
            "{} ({}: {}): {state}",
            file.path.display(),
            file.runtime,
            file.instances.join(", ")
        ));
    }
    for link in links {
        let state = match link.state {
            LinkState::Made => "made",
            LinkState::Repaired => "repaired",
            LinkState::Current => "already in place",
        };
        lines.push(format!(
            "{}: link to {} ensured: {state}",
            link.path.display(),
            link.target.display()
        ));
    }
    for link in replaced_links {
        lines.push(format!(
            "{}: a symbolic link, replaced; what it pointed to is left as it is",
            link.display()
        ));
    }
    for finding in findings {
        lines.push(match finding {
            SyncFinding::SettingsLeft { path, reason } => {
                format!("{}: left untouched: {reason}", path.display())
            }
            SyncFinding::ReferenceKept { path, reference } => format!(
                "{}: keeps {reference} for an agent program to resolve",
                path.display()
            ),
            SyncFinding::InstructionTarget { target } => {
                format!("notice: {}", instruction_line(target))
            },
            SyncFinding::RuntimeUnreachable { runtime, cause } => format!(
                "notice: runtime `{runtime}` could not be asked which agent runtimes are configured ({cause}); no global agent file was written, and the next sync writes them"
            ),
        });
    }
    lines
}

/// Shared sync/init wording for native target limits and non-delivery.
pub(super) fn instruction_line(target: &instrukt_core::publication::InstructionTarget) -> String {
    let mut details = match target.reason {
        Some(reason) => vec![format!("{}; nothing delivered", reason.name())],
        None => vec![format!("home source: {}", target.home_source.name())],
    };
    if let Some(path) = &target.conflict_path {
        details.push(format!("conflicting entry: {}", path.display()));
    }
    if let Some(path) = &target.unknown_home {
        details.push(format!("unresolved home: {}", path.display()));
    }
    if let Some(agent) = &target.registry_agent {
        details.push(format!("registry agent: {agent}"));
    }
    details.extend(target.assurance.iter().map(|a| a.name().to_owned()));
    format!(
        "instance `{}` ({}): {}",
        target.instance,
        target.runtime,
        details.join("; ")
    )
}
