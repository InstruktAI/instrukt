//! Docs commands: `instrukt docs validate|schema|fold|index|get|render|plugin`.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::Context as _;
use instrukt_core::docs::{
    self, DomainRequest, GetRequest, IndexRequest, Taxonomy, ValidationRequest,
};
use instrukt_core::git;
use instrukt_core::ids::{ProjectSlug, TodoSlug};
use instrukt_core::publication::{self, CompanyName, PlannedFile, RepositoryView};

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
    /// Render the repository's root `AGENTS.md` from its working tree.
    ///
    /// The root `AGENTS.md` is rendered from the repository's entry index —
    /// `docs/project/index.yaml` in a project, `docs/index.yaml` in a company
    /// home — and what the working tree holds, nothing else: no home,
    /// publication or configuration is read, so a fresh clone and the binary
    /// are enough. Writes the rendering when it differs, removes each
    /// `AGENTS.md` instrukt rendered that is no longer rendered (any below
    /// the root, and the root's where no entry index exists), and prints one
    /// line per file changed, then each finding and each notice. A finding
    /// leaves its file as it is and fails the run. Stages nothing.
    ///
    /// With `--check`, writes nothing: prints the root file when it differs
    /// from its rendering or is missing, each rendering no longer rendered
    /// and each finding, and exits non-zero when it prints any.
    Render(RenderArgs),
    /// Render the company's Claude plugin from the repository's working tree.
    ///
    /// The plugin `plugins/<key>/` is rendered from the company index
    /// `docs/index.yaml`, the documents it selects, the key and the name
    /// given here and what the working tree holds, nothing else: no home,
    /// publication or configuration is read, so a fresh clone and the
    /// binary are enough. Writes each file of the plugin whose content
    /// differs, removes each file under `plugins/<key>/` that the rendering
    /// does not hold, sets the plugin's entry in
    /// `.claude-plugin/marketplace.json`, then prints the plugin's version
    /// and one line per file changed. The version is kept when the
    /// rendering equals what the folder holds and its last number is raised
    /// otherwise. A finding leaves every file as it is and fails the run.
    /// Stages nothing.
    ///
    /// With `--check`, writes nothing: prints each file that differs from
    /// the rendering or is missing, each file a rendering would remove and
    /// each finding, and exits non-zero when it prints any.
    ///
    /// With `--validate`, writes nothing and compares no file: prints each
    /// finding and exits non-zero only when there is one, so a branch whose
    /// plugin is behind its documents passes.
    Plugin(PluginArgs),
}

#[derive(Debug, clap::Args)]
struct PluginArgs {
    /// The company's key: the name of the plugin and of its skill, and the
    /// plugin's folder under `plugins/`.
    #[arg(long, value_name = "KEY")]
    key: ProjectSlug,
    /// The name people write the company by, as one line: the skill's title
    /// and the plugin's author.
    #[arg(long, value_name = "NAME")]
    name: CompanyName,
    /// Write nothing; report each file a rendering would change and each
    /// finding.
    #[arg(long, conflicts_with = "validate")]
    check: bool,
    /// Write nothing and compare no file; report each finding.
    #[arg(long)]
    validate: bool,
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
    /// Skip checking whether pending-doc slugs name existing todos; pending
    /// slugs are still checked for form, alongside marker structure, workflow
    /// roles and all other structural rules.
    #[arg(long)]
    no_todo_lookup: bool,
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
        DocsCommand::Plugin(args) => plugin(&args, ctx),
    }
}

/// Renders the repository's committed entrypoints from its working tree,
/// or with `--check` reports what that would change.
fn render(args: &RenderArgs, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    let root = git::worktree_root(&ctx.cwd)?.unwrap_or_else(|| ctx.cwd.clone());
    let plan = publication::render_repository(RepositoryView::WorkingTree(&root))?;
    let mut failed = !plan.findings.is_empty();
    if args.check {
        for line in check_lines(&plan.writes, &plan.removals, "instrukt docs render") {
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

/// Renders the company's plugin from the working tree, or with `--check`
/// reports what that would change, or with `--validate` reports only what
/// it finds.
fn plugin(args: &PluginArgs, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    let root = git::worktree_root(&ctx.cwd)?.unwrap_or_else(|| ctx.cwd.clone());
    let plan = publication::plan_plugin(&root, &args.key, &args.name)?;
    let mut failed = !plan.findings.is_empty();
    if args.check {
        for line in check_lines(&plan.writes, &plan.removals, "instrukt docs plugin") {
            ctx.out.line(&line)?;
            failed = true;
        }
    } else if !args.validate && !failed {
        if let Some(version) = plan.version {
            ctx.out
                .line(&format!("{}: version {version}", plan.folder.display()))?;
        }
        let removed: Vec<&Path> = plan.removals.iter().map(PathBuf::as_path).collect();
        for path in publication::write_plugin(&root, &plan)? {
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
    Ok(if !failed {
        ExitCode::SUCCESS
    } else if args.check || args.validate {
        ExitCode::FAILURE
    } else {
        crate::output::could_not(
            "the findings above left the plugin and its catalog as they were; fix them and run `instrukt docs plugin` again",
        )
    })
}

/// One line per file whose content differs from its rendering or is
/// missing, and per file no longer rendered, each naming `command`, which
/// writes or removes it.
fn check_lines(writes: &[PlannedFile], removals: &[PathBuf], command: &str) -> Vec<String> {
    let mut lines = Vec::new();
    for file in writes.iter().filter(|file| file.differs()) {
        let what = if file.current.is_some() {
            format!("differs from its rendering; `{command}` writes it")
        } else {
            format!("missing; `{command}` writes it")
        };
        lines.push(format!("{}: {what}", file.path.display()));
    }
    for path in removals {
        lines.push(format!(
            "{}: no longer rendered; `{command}` removes it",
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
    let mut wiring = core_wiring(ctx)?;
    if args.no_todo_lookup {
        wiring.todo_existence = None;
    }
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
