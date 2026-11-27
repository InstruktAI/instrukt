//! Review evidence commands: `instrukt review record|exempt|show`
//! (`project/instrukt/spec/feature/work/github-apps`, Review evidence), and
//! `instrukt review start`, the ask for a requested review
//! (`project/instrukt/spec/feature/runner/requested-review`).

use std::ffi::OsString;
use std::io::Read as _;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context as _, bail};
use instrukt_core::config::Configuration;
use instrukt_core::git;
use instrukt_core::ids::CommitSha;
use instrukt_runner::requested_review::{self, Reached};
use instrukt_todos::work::review::{self, ReviewProject, Verdict};

use crate::context::Ctx;

#[derive(Debug, clap::Args)]
pub struct ReviewArgs {
    #[command(subcommand)]
    command: ReviewCommand,
}

#[derive(Debug, clap::Subcommand)]
enum ReviewCommand {
    /// Ask for the independent review of an exact committed worktree head
    /// by its coordinates alone; instrukt renders the reviewer's assignment
    /// and starts or sends to the head's reviewer itself.
    Start(StartArgs),
    /// Record the calling session's verdict of the commits after the base
    /// up to the head, with the complete report.
    Record(RecordArgs),
    /// Record that a commit the canonical branch does not contain owes no
    /// independent review, with its basis.
    Exempt(ExemptArgs),
    /// What this computer holds of a commit's review evidence.
    Show(ShowArgs),
    /// Hand the project's review evidence that stands on this computer off
    /// into one file.
    Export(ExportArgs),
    /// Take in an export written on another computer, or on this one.
    Import(ImportArgs),
}

/// A requested review's coordinates: each names a place or a commit, and
/// none takes text of its own.
#[derive(Debug, clap::Args)]
struct StartArgs {
    /// The absolute path of the worktree that holds the candidate: the
    /// project's registered checkout or a linked worktree of it.
    #[arg(long)]
    worktree: OsString,
    /// The branch checked out in the worktree whose head is the candidate.
    #[arg(long)]
    branch: OsString,
    /// The full id of the candidate commit, the branch's head.
    #[arg(long)]
    head: OsString,
    /// The full id of the pinned base the review starts from.
    #[arg(long)]
    base: OsString,
    /// The absolute path, outside the worktree, the reviewer writes its
    /// complete report to.
    #[arg(long)]
    report: OsString,
    /// The absolute path of the earlier review's report, on re-review.
    #[arg(long)]
    prior_report: Option<OsString>,
    /// The absolute path of the author's dispositions of the earlier
    /// findings, to dispute them on an unchanged head.
    #[arg(long)]
    disposition: Option<OsString>,
    /// A source the reviewer reads beside the candidate: a snippet id, or
    /// `repo@commit:path`. Repeatable.
    #[arg(long = "source")]
    sources: Vec<OsString>,
    /// Print the reply as JSON.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, clap::Args)]
struct ExportArgs {
    /// The file to write.
    #[arg(long)]
    to: PathBuf,
}

#[derive(Debug, clap::Args)]
struct ImportArgs {
    /// The export file.
    file: PathBuf,
    /// Which verdict stands for a commit of which the file and this
    /// computer hold two different ones.
    #[arg(long, value_parser = statement)]
    standing: Option<review::Statement>,
}

fn statement(text: &str) -> Result<review::Statement, String> {
    match text {
        "imported" => Ok(review::Statement::Imported),
        "held" => Ok(review::Statement::Held),
        _ => Err(format!("`{text}` is neither `imported` nor `held`")),
    }
}

#[derive(Debug, clap::Args)]
struct RecordArgs {
    /// The full id of the reviewed head commit.
    #[arg(long)]
    head: String,
    /// The full id of the commit the review started from: the pinned base
    /// or one of its ancestors.
    #[arg(long)]
    base: String,
    /// The verdict.
    #[arg(long, value_parser = verdict)]
    verdict: Verdict,
    /// The complete review report: a file, or `-` for standard input.
    #[arg(long)]
    report: PathBuf,
}

#[derive(Debug, clap::Args)]
struct ExemptArgs {
    /// The full id of the exempted commit.
    #[arg(long)]
    commit: String,
    /// Why the commit owes no independent review.
    #[arg(long)]
    basis: String,
}

#[derive(Debug, clap::Args)]
struct ShowArgs {
    /// The full id of the commit.
    sha: String,
}

fn verdict(text: &str) -> Result<Verdict, String> {
    Verdict::parse(text).ok_or_else(|| format!("`{text}` is neither `approve` nor `needs_work`"))
}

pub fn run(args: ReviewArgs, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    match args.command {
        ReviewCommand::Start(args) => start(args, ctx),
        ReviewCommand::Record(args) => record(args, ctx),
        ReviewCommand::Exempt(args) => exempt(args, ctx),
        ReviewCommand::Show(args) => show(&args, ctx),
        ReviewCommand::Export(args) => export(&args, ctx),
        ReviewCommand::Import(args) => import(&args, ctx),
    }
}

fn export(args: &ExportArgs, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    let (project, _) = standing_project(ctx)?;
    let exported = review::export(&ctx.home, &project.slug, &args.to, ctx.clock.now())?;
    ctx.out.line(&format!(
        "exported {} verdicts, {} exemptions and {} derived approvals to {} ({}); they are handed off from this computer",
        exported.verdicts,
        exported.exemptions,
        exported.derived,
        args.to.display(),
        exported.digest
    ))?;
    Ok(ExitCode::SUCCESS)
}

fn import(args: &ImportArgs, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    let (project, _) = standing_project(ctx)?;
    let imported = review::import(
        &ctx.home,
        &project.slug,
        &args.file,
        args.standing,
        ctx.clock.now(),
    )?;
    ctx.out.line(&format!(
        "imported: {} verdicts now stand here, {} were already current, {} pairs were settled by the statement",
        imported.standing, imported.unchanged, imported.set_aside
    ))?;
    Ok(ExitCode::SUCCESS)
}

/// The project the caller stands in, with its configuration.
fn standing_project(ctx: &Ctx) -> anyhow::Result<(ReviewProject, Configuration)> {
    let Some(root) = git::worktree_root(&ctx.cwd)? else {
        bail!("review evidence belongs to a project; run this inside its git repository");
    };
    let config = Configuration::load(&ctx.home, Some(&root))?;
    let Some(slug) = config.project()? else {
        bail!(
            "{} is not an adopted project; review evidence belongs to one",
            root.display()
        );
    };
    Ok((review::project(slug, &root), config))
}

fn caller(
    ctx: &Ctx,
    config: &Configuration,
) -> anyhow::Result<Option<instrukt_core::ids::SessionRef>> {
    Ok(config.caller(&crate::wiring::environment(ctx)?)?)
}

fn start(args: StartArgs, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    let (project, config) = standing_project(ctx)?;
    let coordinates = requested_review::Coordinates::parse(&requested_review::RawCoordinates {
        worktree: args.worktree,
        branch: args.branch,
        head: args.head,
        base: args.base,
        report: args.report,
        prior_report: args.prior_report,
        disposition: args.disposition,
        sources: args.sources,
    })?;
    let verdicts = crate::wiring::head_verdicts(ctx);
    let asker = caller(ctx, &config)?;
    let project = instrukt_runner::interfaces::ProjectRef {
        slug: project.slug,
        root: project.root,
    };
    let reply = requested_review::ask(
        &requested_review::AskContext {
            home: &ctx.home,
            config: &config,
            project: &project,
            verdicts: verdicts.as_ref(),
            asker,
            now: ctx.clock.now(),
        },
        coordinates,
    )?;
    if args.json {
        ctx.out.json(&reply)?;
        return Ok(ExitCode::SUCCESS);
    }
    let reached = match (reply.reached, reply.repeated) {
        (_, true) => "already reached by this same outstanding ask",
        (Reached::Started, false) => "started by this ask",
        (Reached::Sent, false) => "sent this ask",
    };
    ctx.out.line(&format!(
        "requested {} of {} from pinned base {}",
        reply.kind, reply.head, reply.base
    ))?;
    ctx.out.line(&format!(
        "reviewer: session {}, {reached}, start key {}",
        reply.reviewer, reply.start_key
    ))?;
    ctx.out
        .line(&format!("report: {}", reply.report.display()))?;
    ctx.out.line(&format!(
        "outcome: in the report, and in `{}` once the reviewer records its verdict",
        reply.show
    ))?;
    Ok(ExitCode::SUCCESS)
}

fn record(args: RecordArgs, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    let (project, config) = standing_project(ctx)?;
    let report = if args.report.as_os_str() == "-" {
        let mut text = String::new();
        std::io::stdin()
            .read_to_string(&mut text)
            .context("cannot read the report from standard input")?;
        text
    } else {
        std::fs::read_to_string(&args.report)
            .with_context(|| format!("cannot read the report {}", args.report.display()))?
    };
    let request = review::RecordRequest {
        head: args.head,
        base: args.base,
        verdict: args.verdict,
        report,
        caller: caller(ctx, &config)?,
    };
    let clock = ctx.clock.clone();
    let env = crate::wiring::environment(ctx)?;
    let recorded = review::record(&ctx.home, &env, &project, &request, &|| clock.now())?;
    ctx.out.line(&format!(
        "recorded {} version {}: {} {} {}",
        recorded.head,
        recorded.version,
        request.verdict.as_str(),
        recorded.name,
        recorded.moment
    ))?;
    Ok(ExitCode::SUCCESS)
}

fn exempt(args: ExemptArgs, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    let (project, config) = standing_project(ctx)?;
    let request = review::ExemptRequest {
        commit: args.commit,
        basis: args.basis,
        caller: caller(ctx, &config)?,
    };
    let clock = ctx.clock.clone();
    let exemption = review::exempt(&ctx.home, &project, &request, &|| clock.now())?;
    ctx.out.line(&format!(
        "exempted {}: {}",
        exemption.commit, exemption.basis
    ))?;
    Ok(ExitCode::SUCCESS)
}

fn show(args: &ShowArgs, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    let (project, _) = standing_project(ctx)?;
    let commit = CommitSha::parse(&args.sha)
        .map_err(|_| anyhow::anyhow!("`{}` is not a full commit id", args.sha))?;
    let evidence = review::evidence(&ctx.home, &project.slug, &commit)?;
    if evidence.settlements.is_empty()
        && evidence.exemption.is_none()
        && evidence.derived.is_none()
        && evidence.recognised.is_none()
        && evidence.deliveries.is_empty()
    {
        ctx.out.line(&format!(
            "{commit}: this computer holds no review evidence of it"
        ))?;
        return Ok(ExitCode::SUCCESS);
    }
    for settlement in &evidence.settlements {
        ctx.out.line(&format!(
            "head verdict ({}{}, since {})",
            settlement.standing.as_str(),
            settlement
                .cause
                .as_ref()
                .map(|cause| format!(" by {cause}"))
                .unwrap_or_default(),
            settlement.changed_at
        ))?;
        for version in &settlement.versions {
            ctx.out.line(&format!(
                "  version {}: {} {} moment {}, recorded {} on {} by {}{}",
                version.number,
                version.verdict.as_str(),
                version.name,
                version.moment,
                version.recorded_at,
                version.recorded_on,
                version.reviewer,
                version
                    .entered_by
                    .as_ref()
                    .map(|s| format!(", entered by {s}"))
                    .unwrap_or_default()
            ))?;
            ctx.out.line(&format!(
                "    pinned base {}{}{}",
                version.base,
                if version.base_on_canonical {
                    ""
                } else {
                    " (not on the canonical branch)"
                },
                version
                    .request
                    .map(|n| format!(", answers request {n}"))
                    .unwrap_or_default()
            ))?;
            if !version.unattributed.is_empty() {
                ctx.out.line(&format!(
                    "    commits naming no authoring session: {}",
                    version
                        .unattributed
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(", ")
                ))?;
            }
            ctx.out.text(&indent(&version.report))?;
        }
    }
    if let Some(exemption) = &evidence.exemption {
        ctx.out.line(&format!(
            "exemption: {} (declared by {}{}, recorded {} on {})",
            exemption.basis,
            exemption
                .declared_by
                .as_ref()
                .map_or_else(|| "no resolved session".to_owned(), ToString::to_string),
            if exemption.names_session {
                ""
            } else {
                "; the commit names no authoring session"
            },
            exemption.recorded_at,
            exemption.recorded_on
        ))?;
    }
    if let Some(derived) = &evidence.derived {
        ctx.out.line(&format!(
            "derived approval: {} moment {}, against canonical head {}, {} {}{}",
            derived.name,
            derived.moment,
            derived.canonical,
            if derived.imported {
                "brought by an import"
            } else {
                "recorded by this computer's tick"
            },
            derived.recorded_at,
            derived
                .withdrawn
                .as_ref()
                .map(|(at, uncovered)| format!("; withdrawn {at}, {uncovered} found uncovered"))
                .unwrap_or_default()
        ))?;
        ctx.out.text(&indent(&derived.coverage))?;
    }
    if let Some(recognised) = &evidence.recognised {
        ctx.out.line(&format!(
            "recognised as already published: {} {} moment {} (recognised {}{})",
            recognised.published.verdict.as_str(),
            recognised.published.name,
            recognised.published.moment,
            recognised.recognised_at,
            if recognised.body.is_some() {
                ""
            } else {
                "; no review carrying it was read, so none is deliverable from this computer"
            }
        ))?;
    }
    for delivery in &evidence.deliveries {
        let destination = match &delivery.destination {
            review::Destination::Status => "review status".to_owned(),
            review::Destination::Pull(number) => format!("review on pull request #{number}"),
        };
        ctx.out.line(&format!(
            "publication: {destination} of {} {}: {}{}{}",
            delivery.name,
            delivery.moment,
            delivery.state.as_str(),
            delivery
                .link
                .as_ref()
                .map(|link| format!(" ({link})"))
                .unwrap_or_default(),
            delivery
                .cause
                .as_ref()
                .map(|cause| format!(": {cause}"))
                .unwrap_or_default()
        ))?;
    }
    Ok(ExitCode::SUCCESS)
}

fn indent(report: &str) -> String {
    let mut text: String = report
        .lines()
        .map(|line| format!("    | {line}\n"))
        .collect();
    if text.is_empty() {
        text.push('\n');
    }
    text
}
