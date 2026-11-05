//! Docs commands: `instrukt docs validate|fold|index|get`.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::Context as _;
use instrukt_core::docs::{
    self, DomainRequest, GetRequest, IndexRequest, Taxonomy, ValidationRequest,
};
use instrukt_core::git;
use instrukt_core::ids::{ProjectSlug, TodoSlug};

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
    /// finding (`<file>: <code>[: <value>]`), then each notice
    /// (`notice: <file>: <code>[: <value>]`), and exits non-zero while any
    /// finding remains; notices never fail the run.
    Validate(ValidateArgs),
    /// Apply one delivery's pending documentation to the working tree.
    ///
    /// Clears the todo from `delivered_by`, replaces each live snippet with
    /// its companion (removing an empty companion together with its
    /// snippet), unwraps its planned additions and applies its planned
    /// replacements. Other todos' material is kept.
    Fold(FoldArgs),
    /// List published snippet ids with their descriptions (phase 1).
    ///
    /// Shows `general`, every published domain and the snippets of the
    /// project the working directory belongs to. Each filter only narrows.
    /// A listing naming a position opens with its source: its folder in the
    /// checkout it was published from, the published commit and the
    /// remote. Pending snippets are flagged `(delivered by: …)`, live
    /// snippets with pending changes `(superseded by: …)`.
    Index(IndexArgs),
    /// Read snippets through the read pipeline (phase 2).
    ///
    /// Prints each snippet's header (naming its source file) and body, with
    /// configured values resolved for this session. Names the required
    /// reads it did not load; `--closure` loads them with one snippet
    /// instead. `--domain <x>` reads what a domain shows: its index's
    /// snippets in full with their closures, its listed snippets and its
    /// triggered procedures. Exits non-zero when an id or domain is unknown
    /// or a configured value cannot be resolved.
    Get(GetArgs),
}

#[derive(Debug, clap::Args)]
struct IndexArgs {
    /// Only snippets of this published domain (`general` included);
    /// repeatable.
    #[arg(long = "domain", value_name = "DOMAIN")]
    domains: Vec<String>,
    /// Only these taxonomy types: principle, concept, policy, procedure,
    /// design, spec, adr (comma-separated).
    #[arg(long, value_delimiter = ',', value_name = "TYPE", value_parser = taxonomy)]
    areas: Vec<Taxonomy>,
    /// Add the third-party documentation section.
    #[arg(long)]
    third_party: bool,
    /// Only snippets this todo delivers or has pending changes for.
    #[arg(long, value_name = "SLUG")]
    target: Option<TodoSlug>,
    /// Only this project's local snippets.
    #[arg(long, value_name = "SLUG")]
    project: Option<ProjectSlug>,
}

#[derive(Debug, clap::Args)]
struct GetArgs {
    /// Snippet ids, as `instrukt docs index` lists them.
    #[arg(required_unless_present = "domain", value_name = "ID")]
    ids: Vec<String>,
    /// Read one delivered snippet with every transitive required read and
    /// what the domains they name show in full, in one pass.
    #[arg(long, conflicts_with = "target")]
    closure: bool,
    /// Read the documentation as this todo will deliver it.
    #[arg(long, value_name = "SLUG")]
    target: Option<TodoSlug>,
    /// Read what a domain (`general` included) shows, as its index renders
    /// it; takes no ids.
    #[arg(long, value_name = "DOMAIN", conflicts_with_all = ["ids", "closure", "target"])]
    domain: Option<String>,
}

fn taxonomy(value: &str) -> Result<Taxonomy, String> {
    Taxonomy::from_dir(value).ok_or_else(|| {
        let valid: Vec<&str> = Taxonomy::ALL.iter().map(|t| t.dir()).collect();
        format!("expected one of {}", valid.join(", "))
    })
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
        DocsCommand::Index(args) => index(args, ctx),
        DocsCommand::Get(args) => get(args, ctx),
    }
}

fn index(args: IndexArgs, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    let listing = docs::index(&IndexRequest {
        home: &ctx.home,
        cwd: &ctx.cwd,
        domains: &args.domains,
        areas: &args.areas,
        third_party: args.third_party,
        target: args.target.as_ref(),
        project: args.project.as_ref(),
    })?;
    ctx.out.text(&listing)?;
    Ok(ExitCode::SUCCESS)
}

fn get(args: GetArgs, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    let reading = match &args.domain {
        Some(domain) => docs::get_domain(&DomainRequest {
            home: &ctx.home,
            cwd: &ctx.cwd,
            domain,
        })?,
        None => docs::get(&GetRequest {
            home: &ctx.home,
            cwd: &ctx.cwd,
            ids: &args.ids,
            closure: args.closure,
            target: args.target.as_ref(),
        })?,
    };
    ctx.out.text(&reading.text)?;
    Ok(if reading.unresolved {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}

fn repo_root(cwd: &Path) -> anyhow::Result<PathBuf> {
    git::worktree_root(cwd)?
        .with_context(|| format!("{} is not inside a git repository", cwd.display()))
}

fn validate(args: ValidateArgs, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    let root = repo_root(&ctx.cwd)?;
    let paths: Vec<PathBuf> = args.paths.iter().map(|p| ctx.cwd.join(p)).collect();
    let wiring = core_wiring(ctx)?;
    let validation = docs::validate(&ValidationRequest {
        repo_root: &root,
        home: &ctx.home,
        paths: &paths,
        wiring: &wiring,
    })?;
    let findings = &validation.findings;
    if args.json {
        // The findings are the JSON list; notices go to stderr, apart.
        ctx.out.json(findings)?;
        for notice in &validation.notices {
            eprintln!("notice: {notice}");
        }
    } else {
        for finding in findings {
            ctx.out.line(&finding.to_string())?;
        }
        for notice in &validation.notices {
            ctx.out.line(&format!("notice: {notice}"))?;
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
