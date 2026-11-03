//! Docs commands: `instrukt docs validate|fold` (and, later, `index|get`).

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::Context as _;
use instrukt_core::docs::{self, ValidationRequest};
use instrukt_core::git;
use instrukt_core::ids::TodoSlug;

use crate::context::Ctx;
use crate::wiring::core_wiring;

#[derive(Debug, clap::Args)]
pub struct DocsArgs {
    #[command(subcommand)]
    command: DocsCommand,
}

#[derive(Debug, clap::Subcommand)]
enum DocsCommand {
    /// Check the repository's documentation against the snippet contract.
    ///
    /// Reads the working tree and changes nothing. Prints one line per
    /// finding (`<file>: <code>[: <value>]`) and exits non-zero while any
    /// finding remains.
    Validate(ValidateArgs),
    /// Apply one delivery's pending documentation to the working tree.
    ///
    /// Clears the todo from `delivered_by`, replaces each live snippet with
    /// its companion (removing an empty companion together with its
    /// snippet), unwraps its planned additions and applies its planned
    /// replacements. Other todos' material is kept.
    Fold(FoldArgs),
}

#[derive(Debug, clap::Args)]
struct ValidateArgs {
    /// Files or directories to validate, with the cross-snippet checks they
    /// take part in; the whole `docs/` when omitted.
    paths: Vec<PathBuf>,
    /// Print the findings as a JSON list.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, clap::Args)]
struct FoldArgs {
    /// The delivering todo's slug.
    slug: TodoSlug,
}

pub fn run(args: DocsArgs, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    match args.command {
        DocsCommand::Validate(args) => validate(args, ctx),
        DocsCommand::Fold(args) => fold(args, ctx),
    }
}

fn repo_root(cwd: &Path) -> anyhow::Result<PathBuf> {
    git::worktree_root(cwd)?
        .with_context(|| format!("{} is not inside a git repository", cwd.display()))
}

fn validate(args: ValidateArgs, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    let root = repo_root(&ctx.cwd)?;
    let paths: Vec<PathBuf> = args.paths.iter().map(|p| ctx.cwd.join(p)).collect();
    let wiring = core_wiring(ctx);
    let findings = docs::validate(&ValidationRequest {
        repo_root: &root,
        home: &ctx.home,
        paths: &paths,
        wiring: &wiring,
    })?;
    if args.json {
        ctx.out.json(&findings)?;
    } else {
        for finding in &findings {
            ctx.out.line(&finding.to_string())?;
        }
    }
    Ok(if findings.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

fn fold(args: FoldArgs, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    let root = repo_root(&ctx.cwd)?;
    docs::fold(&root, &args.slug)?;
    Ok(ExitCode::SUCCESS)
}
