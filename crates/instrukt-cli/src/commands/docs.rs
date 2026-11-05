//! Docs commands: `instrukt docs validate|schema|fold|index|get`.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::Context as _;
use instrukt_core::docs::{self, GetRequest, IndexRequest, Taxonomy, ValidationRequest};
use instrukt_core::git;
use instrukt_core::ids::{DomainName, ProjectSlug, TodoSlug};

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
    /// finding (`<file>: <code>[: <value>]`), then one line per notice
    /// (`notice: <file>: <code>[: <value>]`), and exits non-zero while any
    /// finding remains; notices never fail the run.
    Validate(ValidateArgs),
    /// Show the sections the snippets of a folder must and may carry.
    ///
    /// For each type the folder can hold: its required and optional
    /// sections, each with what imposes it (`instrukt root rules`, or the
    /// position's `schema.yaml` and its folder key), then a skeleton to start
    /// a snippet from. Reads the working tree; the folder need not exist.
    Schema(SchemaArgs),
    /// Apply one delivery's pending documentation to the working tree.
    ///
    /// Clears the todo from `delivered_by`, replaces each live snippet with
    /// its companion (removing an empty companion together with its
    /// snippet), unwraps its planned additions and applies its planned
    /// replacements. Other todos' material is kept.
    Fold(FoldArgs),
    /// List published snippet ids with their descriptions (phase 1).
    ///
    /// Shows every published domain and the local snippets of the project
    /// the working directory belongs to. Each filter only narrows. Pending
    /// snippets are flagged `(delivered by: …)`, live snippets with pending
    /// changes `(superseded by: …)`.
    Index(IndexArgs),
    /// Read snippets through the read pipeline (phase 2).
    ///
    /// Prints each snippet's header and body, with configured values
    /// resolved for this session and references made absolute. Names the required reads it did not load; `--closure`
    /// loads them with one procedure instead. Exits non-zero when an id is
    /// unknown or a configured value cannot be resolved.
    Get(GetArgs),
}

#[derive(Debug, clap::Args)]
struct IndexArgs {
    /// Only snippets of these published domains (comma-separated).
    #[arg(long, value_delimiter = ',', value_name = "DOMAIN")]
    domains: Vec<DomainName>,
    /// Only these taxonomy types: principle, concept, policy, procedure,
    /// design, spec, adr (comma-separated).
    #[arg(long, value_delimiter = ',', value_name = "TYPE", value_parser = taxonomy)]
    areas: Vec<Taxonomy>,
    /// Only snippets a domain's or the project's baseline manifest names.
    #[arg(long)]
    baseline_only: bool,
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
    #[arg(required = true, value_name = "ID")]
    ids: Vec<String>,
    /// Read one delivered procedure with every transitive required read and
    /// the domain bundles they name, in one pass.
    #[arg(long, conflicts_with = "target")]
    closure: bool,
    /// Read the documentation as this todo will deliver it.
    #[arg(long, value_name = "SLUG")]
    target: Option<TodoSlug>,
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
struct SchemaArgs {
    /// A folder inside `docs/general/`, `docs/domain/<domain>/` or
    /// `docs/project/`.
    folder: PathBuf,
}

#[derive(Debug, clap::Args)]
struct FoldArgs {
    /// The delivering todo's slug.
    slug: TodoSlug,
}

pub fn run(args: DocsArgs, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    match args.command {
        DocsCommand::Validate(args) => validate(args, ctx),
        DocsCommand::Schema(args) => schema(args, ctx),
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
        baseline_only: args.baseline_only,
        third_party: args.third_party,
        target: args.target.as_ref(),
        project: args.project.as_ref(),
    })?;
    ctx.out.text(&listing)?;
    Ok(ExitCode::SUCCESS)
}

fn get(args: GetArgs, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    let reading = docs::get(&GetRequest {
        home: &ctx.home,
        cwd: &ctx.cwd,
        ids: &args.ids,
        closure: args.closure,
        target: args.target.as_ref(),
    })?;
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
    let docs::Validation { findings, notices } = docs::validate(&ValidationRequest {
        repo_root: &root,
        home: &ctx.home,
        paths: &paths,
        wiring: &wiring,
    })?;
    if args.json {
        ctx.out.json(&findings)?;
        // The JSON list stays the findings'; notices go apart, to stderr.
        ctx.out.flush()?;
        for notice in &notices {
            eprintln!("notice: {notice}");
        }
    } else {
        for finding in &findings {
            ctx.out.line(&finding.to_string())?;
        }
        for notice in &notices {
            ctx.out.line(&format!("notice: {notice}"))?;
        }
    }
    Ok(if findings.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

fn schema(args: SchemaArgs, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    let root = repo_root(&ctx.cwd)?;
    let folder = docs::schema(&root, &ctx.cwd, &args.folder)?;
    if !folder.findings.is_empty() {
        for finding in &folder.findings {
            ctx.out.line(&finding.to_string())?;
        }
        return Ok(ExitCode::FAILURE);
    }
    let mut text = String::new();
    for (index, effective) in folder.types.iter().enumerate() {
        if index > 0 {
            text.push('\n');
        }
        text.push_str(&schema_text(&folder.folder, effective));
    }
    ctx.out.text(&text)?;
    Ok(ExitCode::SUCCESS)
}

/// One type's effective schema at `folder`: its sections with what imposes
/// each, then a skeleton.
fn schema_text(folder: &str, effective: &docs::EffectiveSchema) -> String {
    let taxonomy = effective.taxonomy;
    let width = effective
        .required
        .iter()
        .chain(&effective.optional)
        .map(|s| s.title.chars().count())
        .max()
        .unwrap_or(0);
    let list = |sections: &[docs::SchemaSection]| -> String {
        sections
            .iter()
            .map(|s| format!("  {:<width$}  {}\n", s.title, s.imposed_by))
            .collect()
    };
    let headings: String = effective
        .required
        .iter()
        .map(|s| format!("\n## {}\n", s.title))
        .collect();
    format!(
        "{folder}: {taxonomy}\n\nRequired sections:\n{}\nOptional sections:\n{}\n\
         Skeleton:\n\n---\ndescription: <one line saying what this {taxonomy} is about>\n---\n\n\
         # <Title> — {}\n{headings}",
        list(&effective.required),
        list(&effective.optional),
        taxonomy.title_suffix(),
    )
}

fn fold(args: FoldArgs, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    let root = repo_root(&ctx.cwd)?;
    docs::fold(&root, &args.slug)?;
    Ok(ExitCode::SUCCESS)
}
