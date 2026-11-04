//! Evaluation command: `instrukt eval report`
//! (`local/projects/instrukt/spec/feature/work/eval-report`).

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context as _, bail};
use instrukt_core::config::{self, Env, Module};
use instrukt_core::git;
use instrukt_core::ids::ProjectSlug;
use instrukt_core::publication::{EvalFingerprint, PublishedRecord};
use instrukt_todos::eval;
use instrukt_todos::work::{GitHubRead, TodoRecord, Work, describe};
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
    let work = Work::new(ctx.home.clone(), Env::from_process(), ctx.clock.shared());
    let project = work.project_at(&root)?;
    // Every project publication knows, the current one first.
    let mut checkouts = vec![(project.slug.clone(), root.clone())];
    for published in PublishedRecord::load_all(&ctx.home)
        .context("cannot read which projects this home publishes")?
    {
        if published.project != project.slug {
            checkouts.push((published.project, published.checkout));
        }
    }
    let read: Vec<(ProjectSlug, Vec<TodoRecord>, Vec<String>)> = checkouts
        .into_iter()
        .filter_map(|(slug, checkout)| {
            let (records, limitations) = project_record(ctx, &work, &slug, &checkout)?;
            Some((slug, records, limitations))
        })
        .collect();
    let projects: Vec<eval::ProjectInput<'_>> = read
        .iter()
        .map(|(slug, records, limitations)| eval::ProjectInput {
            project: slug.clone(),
            records,
            limitations: limitations.clone(),
        })
        .collect();
    let fingerprints = EvalFingerprint::recorded(&ctx.home)
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

/// The records of project `slug`, checked out at `checkout`, and what
/// limits them. A project that tracks work is read through work's normal
/// read. Otherwise its record on this computer is what remains: a project
/// whose checkout or configuration is not available is listed with that
/// limitation, and one that switched tracking off keeps its recorded
/// history. `None` only for a project that tracks no work and never did
/// here, which has nothing to evaluate.
fn project_record(
    ctx: &Ctx,
    work: &Work,
    slug: &ProjectSlug,
    checkout: &Path,
) -> Option<(Vec<TodoRecord>, Vec<String>)> {
    // The record on this computer alone, limited by `why`.
    let stored = |why: String| -> (Vec<TodoRecord>, Vec<String>) {
        match work.stored_work(slug) {
            Ok(records) => (
                records,
                vec![format!(
                    "{why} Only its record on this computer is shown; work reflected only to GitHub is not read."
                )],
            ),
            Err(err) => (
                Vec::new(),
                vec![
                    why,
                    format!("Its record on this computer cannot be read either: {err}."),
                ],
            ),
        }
    };
    if !checkout.is_dir() {
        return Some(stored(format!(
            "Its checkout at {} is not available.",
            checkout.display()
        )));
    }
    if !checkout.join("instrukt.yaml").is_file() {
        return Some(stored(format!(
            "Its checkout at {} has no project configuration, so whether it tracks work cannot be read.",
            checkout.display()
        )));
    }
    match config::module_enabled(&ctx.home, Some(checkout), Module::Todos) {
        Ok(true) => {}
        Ok(false) => {
            let (records, limitations) =
                stored("Work tracking is switched off in this project now.".to_owned());
            return (!records.is_empty() || limitations.len() > 1)
                .then_some((records, limitations));
        }
        Err(err) => {
            return Some(stored(format!("Its configuration cannot be read: {err}.")));
        }
    }
    let recorded = work
        .project_at(checkout)
        .and_then(|project| work.recorded_work(&project));
    Some(match recorded {
        Ok(recorded) => {
            let mut limitations = Vec::new();
            if let GitHubRead::NotRead(reason) = recorded.github {
                limitations.push(format!(
                    "Reflected work on GitHub could not be read ({}); work recorded only on other computers is missing.",
                    describe(&reason)
                ));
            }
            if !recorded.conflicted.is_empty() {
                let slugs: Vec<String> = recorded
                    .conflicted
                    .iter()
                    .map(|slug| format!("`{slug}`"))
                    .collect();
                limitations.push(format!(
                    "{} conflict{} with a newer GitHub revision, so evidence recorded elsewhere may be missing; refresh with `instrukt todo refresh <slug>`.",
                    slugs.join(", "),
                    if slugs.len() == 1 { "s" } else { "" }
                ));
            }
            (recorded.records, limitations)
        }
        Err(err) => stored(format!("Its work record cannot be read: {err}.")),
    })
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
        std::fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
    }
    instrukt_core::fsx::atomic_write(path, html.as_bytes())
        .with_context(|| format!("cannot write the report to {}", path.display()))
}
