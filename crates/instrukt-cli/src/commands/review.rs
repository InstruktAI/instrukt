//! Review evidence commands: `instrukt review record|exempt|show`
//! (`project/instrukt/spec/feature/work/github-apps`, Review evidence).

use std::io::Read as _;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context as _, bail};
use instrukt_core::config::Configuration;
use instrukt_core::git;
use instrukt_core::ids::CommitSha;
use instrukt_todos::work::review::{self, ReviewProject, Verdict};

use crate::context::Ctx;

#[derive(Debug, clap::Args)]
pub struct ReviewArgs {
    #[command(subcommand)]
    command: ReviewCommand,
}

#[derive(Debug, clap::Subcommand)]
enum ReviewCommand {
    /// Record the calling session's verdict of the commits after the base
    /// up to the head, with the complete report.
    Record(RecordArgs),
    /// Record that a commit the canonical branch does not contain owes no
    /// independent review, with its basis.
    Exempt(ExemptArgs),
    /// What this computer holds of a commit's review evidence.
    Show(ShowArgs),
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
        ReviewCommand::Record(args) => record(args, ctx),
        ReviewCommand::Exempt(args) => exempt(args, ctx),
        ReviewCommand::Show(args) => show(&args, ctx),
    }
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
    let recorded = review::record(&ctx.home, &project, &request, &|| clock.now())?;
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
    if evidence.settlements.is_empty() && evidence.exemption.is_none() {
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
