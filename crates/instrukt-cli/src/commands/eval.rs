//! Evaluation command: `instrukt eval report`
//! (`project/instrukt/spec/feature/work/eval-report`).

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context as _, bail};
use instrukt_core::git;
use instrukt_todos::eval;
use instrukt_todos::work::Work;
use jiff::Timestamp;

use crate::context::Ctx;

#[derive(Debug, clap::Args)]
pub struct EvalArgs {
    #[command(subcommand)]
    command: EvalCommand,
}

#[derive(Debug, clap::Subcommand)]
enum EvalCommand {
    /// Write an HTML report of delivery outcomes from the work record, by
    /// project and evaluation window, with the model a separate dimension.
    Report(ReportArgs),
}

#[derive(Debug, clap::Args)]
struct ReportArgs {
    /// The start of the history range: a date (`2026-10-01`, from midnight
    /// UTC) or an RFC 3339 date-time.
    #[arg(long, value_parser = since)]
    since: Option<Timestamp>,
    /// Where to write the HTML; defaults to the project's
    /// `scratchpad/eval/index.html`.
    #[arg(long)]
    out: Option<PathBuf>,
}

/// `--since`: a date from midnight UTC, or an RFC 3339 date-time.
fn since(text: &str) -> Result<Timestamp, String> {
    if let Ok(at) = text.parse::<Timestamp>() {
        return Ok(at);
    }
    text.parse::<jiff::civil::Date>()
        .ok()
        .and_then(|date| date.to_zoned(jiff::tz::TimeZone::UTC).ok())
        .map(|zoned| zoned.timestamp())
        .ok_or_else(|| format!("`{text}` is neither a date (YYYY-MM-DD) nor an RFC 3339 date-time"))
}

pub fn run(args: EvalArgs, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    match args.command {
        EvalCommand::Report(args) => report(args, ctx),
    }
}

fn report(args: ReportArgs, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    let Some(root) = git::worktree_root(&ctx.cwd)? else {
        bail!(
            "an evaluation report reads a project's work record; run this inside its git repository"
        );
    };
    let work = Work::new(
        ctx.home.clone(),
        crate::wiring::environment(ctx)?,
        ctx.clock.shared(),
    );
    let project = work.project_at(&root)?;
    // Every project publication knows, the current one first.
    let reading = eval::read(&ctx.home, &work, Some((project.slug.clone(), root.clone())));
    reading
        .published
        .context("cannot read which projects this home publishes")?;
    let projects: Vec<eval::ProjectInput<'_>> = reading
        .projects
        .iter()
        .map(|read| eval::ProjectInput {
            project: read.project.clone(),
            records: &read.records,
            limitations: read
                .limitations
                .iter()
                .map(eval::Limitation::local)
                .collect(),
        })
        .collect();
    let fingerprints = reading
        .fingerprints
        .context("cannot read the recorded eval fingerprints")?;
    let report = eval::report(eval::Inputs {
        projects: &projects,
        fingerprints: &fingerprints,
        since: args.since,
        now: ctx.clock.now(),
    });
    let out = match args.out {
        Some(path) if path.is_absolute() => path,
        Some(path) => ctx.cwd.join(path),
        None => root.join("scratchpad/eval/index.html"),
    };
    write(&out, &eval::render_html(&report))?;
    tracing::info!(
        event = "eval_report_written",
        project = %project.slug,
        path = %out.display(),
        windows = report.projects.iter().map(|p| p.windows.len()).sum::<usize>(),
        "evaluation report written"
    );
    if report.is_empty() {
        ctx.out
            .line("no recorded work: there is nothing to compare yet")?;
    }
    for limitation in &report.limitations {
        ctx.out.line(&format!("limitation: {limitation}"))?;
    }
    for project in &report.projects {
        ctx.out.line(&format!(
            "{}: {} evaluation window{}",
            project.project,
            project.windows.len(),
            if project.windows.len() == 1 { "" } else { "s" }
        ))?;
        for limitation in &project.limitations {
            ctx.out
                .line(&format!("{}: limitation: {limitation}", project.project))?;
        }
    }
    ctx.out.line(&format!("report: {}", out.display()))?;
    Ok(ExitCode::SUCCESS)
}

/// Writes the report to `path`, never through a link.
fn write(path: &Path, html: &str) -> anyhow::Result<()> {
    if std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
        bail!(
            "{} is a symbolic link; the report is written only to a file of its own",
            path.display()
        );
    }
    if let Some(parent) = path.parent() {
        instrukt_core::fsx::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
    }
    instrukt_core::fsx::atomic_write(path, html.as_bytes())
        .with_context(|| format!("cannot write the report to {}", path.display()))
}
