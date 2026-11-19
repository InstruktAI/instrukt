//! Docs commands: `instrukt docs validate|schema|fold|index|get|render`.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::Context as _;
use instrukt_core::docs::{
    self, DomainRequest, GetRequest, IndexRequest, Taxonomy, ValidationRequest,
};
use instrukt_core::git;
use instrukt_core::ids::{ProjectSlug, TodoSlug};
use instrukt_core::publication::{self, RenderPlan, RepositoryView};

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
    /// Render the repository's `AGENTS.md` files from its working tree.
    ///
    /// Each `AGENTS.md` beside an `AGENTS.master.md` is rendered from what
    /// the working tree holds and nothing else: no home, publication or
    /// configuration is read, so a fresh clone and the binary are enough.
    /// Writes each rendering that differs, removes each rendering whose
    /// master is gone, and prints one line per file changed, then each
    /// finding and each notice. A finding leaves its file as it is and
    /// fails the run. Stages nothing.
    ///
    /// With `--check`, writes nothing: prints each file that differs from
    /// its rendering or is missing, each rendering whose master is gone and
    /// each finding, and exits non-zero when it prints any.
    Render(RenderArgs),
}

#[derive(Debug, clap::Args)]
struct RenderArgs {
    /// Write nothing; report what a render would change and what it finds.
    #[arg(long)]
    check: bool,
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
        DocsCommand::Render(args) => render(&args, ctx),
    }
}

/// Renders the repository's committed entrypoints from its working tree,
/// or with `--check` reports what that would change.
fn render(args: &RenderArgs, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    let root = git::worktree_root(&ctx.cwd)?.unwrap_or_else(|| ctx.cwd.clone());
    let plan = publication::render_repository(RepositoryView::WorkingTree(&root))?;
    let mut failed = !plan.findings.is_empty();
    if args.check {
        for line in check_lines(&plan) {
            ctx.out.line(&line)?;
            failed = true;
        }
    } else {
        let removed: Vec<&Path> = plan.removals.iter().map(PathBuf::as_path).collect();
        for path in publication::write_rendering(&root, &plan)? {
            let what = if removed.contains(&path.as_path()) {
                "removed"
            } else {
                "written"
            };
            ctx.out.line(&format!("{}: {what}", path.display()))?;
        }
    }
    for finding in &plan.findings {
        ctx.out.line(&finding.to_string())?;
    }
    if !args.check {
        for notice in &plan.notices {
            ctx.out.line(&format!("notice: {notice}"))?;
        }
    }
    Ok(if !failed {
        ExitCode::SUCCESS
    } else if args.check {
        ExitCode::FAILURE
    } else {
        crate::output::could_not(
            "the findings above leave their files as they were; fix them and run `instrukt docs render` again",
        )
    })
}

/// One line per entrypoint whose file differs from its rendering or is
/// missing, and per rendering whose master is gone.
fn check_lines(plan: &RenderPlan) -> Vec<String> {
    let mut lines = Vec::new();
    for file in plan.writes.iter().filter(|file| file.differs()) {
        let what = if file.current.is_some() {
            "differs from its rendering; `instrukt docs render` writes it"
        } else {
            "missing; `instrukt docs render` writes it"
        };
        lines.push(format!("{}: {what}", file.path.display()));
    }
    for path in &plan.removals {
        lines.push(format!(
            "{}: its master is gone; `instrukt docs render` removes it",
            path.display()
        ));
    }
    lines
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
            env: &crate::wiring::environment(ctx)?,
            home: &ctx.home,
            cwd: &ctx.cwd,
            domain,
        })?,
        None => docs::get(&GetRequest {
            env: &crate::wiring::environment(ctx)?,
            home: &ctx.home,
            cwd: &ctx.cwd,
            ids: &args.ids,
            closure: args.closure,
            target: args.target.as_ref(),
        })?,
    };
    ctx.out.text(&reading.text)?;
    Ok(if reading.unresolved {
        crate::output::could_not(
            "a configured value in what was asked could not be resolved; the output says where",
        )
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

fn schema(args: SchemaArgs, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    let root = repo_root(&ctx.cwd)?;
    let folder = docs::schema(&root, &ctx.cwd, &args.folder)?;
    if !folder.findings.is_empty() {
        for finding in &folder.findings {
            ctx.out.line(&finding.to_string())?;
        }
        return Ok(crate::output::could_not(
            "the folder's schema cannot be shown until the findings above are fixed",
        ));
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
