//! Blueprint commands: `instrukt blueprint affected|path|query|explain|status`.
//!
//! `affected` and `path` answer from the project's latest graph and name its
//! SHA; `query` and `explain` forward to the engine where it offers them;
//! `status` shows the last generation's outcome. The hidden `worker` is the
//! detached process a generation handoff starts.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use anyhow::{Context as _, bail};
use instrukt_blueprint::{
    Engine, GenerationState, GraphStore, LastGeneration, LatestGraph, Link, Node, PathAnswer,
    Question,
};
use instrukt_core::config::{self, Configuration, Module};
use instrukt_core::git;
use instrukt_core::ids::ProjectSlug;

use crate::context::Ctx;

/// The answer when there is no graph to answer from (spec: "unavailable").
pub(crate) const UNAVAILABLE: &str =
    "blueprint unavailable: no graph of this project to answer from; explore by hand";

/// The refusal when the module is off.
pub(crate) const MODULE_OFF: &str =
    "the blueprint module is off; switch it on with `modules.blueprint: true` in instrukt.yaml";

/// How long a forwarded `query` or `explain` may take.
const QUESTION_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Debug, clap::Args)]
pub struct BlueprintArgs {
    #[command(subcommand)]
    command: BlueprintCommand,
}

#[derive(Debug, clap::Subcommand)]
enum BlueprintCommand {
    /// What depends on the given files, from the latest graph.
    Affected {
        /// Files relative to the repository root (absolute paths inside it
        /// are made relative).
        #[arg(required = true)]
        files: Vec<String>,
    },
    /// The shortest chain of links from A to B in the latest graph.
    Path {
        /// A node id or a file path.
        from: String,
        /// A node id or a file path.
        to: String,
    },
    /// A free question to the engine, where it offers one.
    Query {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// The engine's explanation of a node, where it offers one.
    Explain {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// The last generation's outcome.
    Status,
    /// The detached process a hook run's handoff starts.
    #[command(hide = true)]
    Worker { project: ProjectSlug },
}

pub fn run(args: BlueprintArgs, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    if let BlueprintCommand::Worker { project } = &args.command {
        instrukt_blueprint::run_worker(&ctx.home, project)?;
        return Ok(ExitCode::SUCCESS);
    }
    let root = git::worktree_root(&ctx.cwd)?;
    require_module(ctx, root.as_deref())?;
    match args.command {
        BlueprintCommand::Affected { files } => affected(ctx, root.as_deref(), &files),
        BlueprintCommand::Path { from, to } => path(ctx, root.as_deref(), &from, &to),
        BlueprintCommand::Query { args } => ask(ctx, root.as_deref(), Question::Query, &args),
        BlueprintCommand::Explain { args } => ask(ctx, root.as_deref(), Question::Explain, &args),
        BlueprintCommand::Status => status(ctx, root.as_deref()),
        BlueprintCommand::Worker { .. } => Ok(ExitCode::SUCCESS),
    }
}

/// Refuses when the blueprint module is off.
pub(crate) fn require_module(ctx: &Ctx, root: Option<&Path>) -> anyhow::Result<()> {
    let enabled = config::module_enabled(&ctx.home, root, Module::Blueprint)
        .context("cannot read the blueprint module switch")?;
    if !enabled {
        bail!(MODULE_OFF);
    }
    Ok(())
}

fn affected(ctx: &mut Ctx, root: Option<&Path>, files: &[String]) -> anyhow::Result<ExitCode> {
    let Some((root, latest)) = latest_graph(ctx, root)? else {
        ctx.out.line(UNAVAILABLE)?;
        return Ok(ExitCode::SUCCESS);
    };
    let changed: Vec<String> = files
        .iter()
        .map(|file| instrukt_blueprint::repo_relative(&root, file))
        .collect();
    let answer = instrukt_blueprint::blast_radius(&latest.graph, &changed);
    ctx.out
        .line(&format!("blueprint graph of {}", latest.sha))?;
    let count = match answer.changed_files {
        1 => "1 changed file".to_owned(),
        n => format!("{n} changed files"),
    };
    if answer.dependents.is_empty() {
        ctx.out.line(&format!("no dependents found for {count}"))?;
    } else {
        ctx.out.line(&format!("dependents of {count}:"))?;
        for node in &answer.dependents {
            ctx.out.line(&format!("  {}", dependent(node)))?;
        }
    }
    if !answer.unknown_files.is_empty() {
        ctx.out
            .line("files the graph does not know (read them by hand):")?;
        for file in &answer.unknown_files {
            ctx.out.line(&format!("  {file}"))?;
        }
    }
    if let Some(project) = project(&root)?
        && let Some(last) = last_generation(ctx, &project)
        && last.is_failure()
        && last.sha != latest.sha
    {
        ctx.out.line(&format!(
            "the newer generation of {} {}; this answer is from {}",
            last.sha,
            outcome(&last.state),
            latest.sha
        ))?;
    }
    Ok(ExitCode::SUCCESS)
}

fn path(ctx: &mut Ctx, root: Option<&Path>, from: &str, to: &str) -> anyhow::Result<ExitCode> {
    let Some((_, latest)) = latest_graph(ctx, root)? else {
        ctx.out.line(UNAVAILABLE)?;
        return Ok(ExitCode::SUCCESS);
    };
    let answer = instrukt_blueprint::path(&latest.graph, from, to)
        .with_context(|| format!("in the blueprint graph of {}", latest.sha))?;
    ctx.out
        .line(&format!("blueprint graph of {}", latest.sha))?;
    match answer {
        PathAnswer::Found(chain) if chain.is_empty() => {
            ctx.out
                .line(&format!("{from} and {to} name the same node"))?;
        }
        PathAnswer::Found(chain) => {
            ctx.out.line(&format!("path from {from} to {to}:"))?;
            for link in &chain {
                ctx.out.line(&format!("  {}", step(link)))?;
            }
        }
        PathAnswer::Unreachable => {
            ctx.out
                .line(&format!("{to} is not reachable from {from}"))?;
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// `query` or `explain`: forwarded unchanged to the engine over the latest
/// graph, where the engine offers the question.
fn ask(
    ctx: &mut Ctx,
    root: Option<&Path>,
    question: Question,
    args: &[String],
) -> anyhow::Result<ExitCode> {
    let config =
        Configuration::load(&ctx.home, root).context("cannot read the blueprint engine setting")?;
    let engine = Engine::from_setting(
        config
            .blueprint_engine()
            .context("cannot read the blueprint engine setting")?,
    );
    if !engine.offers(question) {
        ctx.out.line(&format!(
            "the configured engine does not offer `{}`",
            question.name()
        ))?;
        return Ok(ExitCode::SUCCESS);
    }
    let Some((_, latest)) = latest_graph(ctx, root)? else {
        ctx.out.line(UNAVAILABLE)?;
        return Ok(ExitCode::SUCCESS);
    };
    let scratch = tempfile::Builder::new()
        .prefix("instrukt-blueprint-")
        .tempdir()
        .context("cannot make a scratch directory for the engine")?;
    let graph = scratch.path().join("graph.json");
    std::fs::write(&graph, latest.graph.to_json()?)
        .with_context(|| format!("cannot write {}", graph.display()))?;
    let answer = engine
        .ask(question, args, &graph, scratch.path(), QUESTION_TIMEOUT)
        .with_context(|| format!("in the blueprint graph of {}", latest.sha))?
        .unwrap_or_default();
    ctx.out
        .line(&format!("blueprint graph of {}", latest.sha))?;
    ctx.out.text(&answer)?;
    if !answer.is_empty() && !answer.ends_with('\n') {
        ctx.out.line("")?;
    }
    Ok(ExitCode::SUCCESS)
}

/// `status`: the latest graph and the last generation's outcome.
fn status(ctx: &mut Ctx, root: Option<&Path>) -> anyhow::Result<ExitCode> {
    let Some(root) = root else {
        bail!("not inside a git repository; `blueprint status` reports a project's generation");
    };
    let Some(project) = project(root)? else {
        bail!("the project layer of instrukt.yaml declares no project slug");
    };
    match latest_graph(ctx, Some(root))? {
        Some((_, latest)) => ctx
            .out
            .line(&format!("blueprint graph of {}", latest.sha))?,
        None => ctx.out.line("no blueprint graph of this project yet")?,
    }
    let last = instrukt_blueprint::last_generation(&ctx.home, &project)
        .with_context(|| format!("cannot read the last generation of {project}"))?;
    match last {
        Some(last) => ctx.out.line(&format!(
            "last generation: {} {}",
            last.sha,
            outcome(&last.state)
        ))?,
        None => ctx.out.line("no generation has run yet")?,
    }
    Ok(ExitCode::SUCCESS)
}

fn outcome(state: &GenerationState) -> String {
    match state {
        GenerationState::Running => "is running".to_owned(),
        GenerationState::Interrupted => "was interrupted".to_owned(),
        GenerationState::Mapped => "is mapped".to_owned(),
        GenerationState::Failed { error } => format!("failed: {error}"),
    }
}

/// The last generation, for naming a failure beside an answer; a record
/// that cannot be read is logged and leaves the answer as it is.
fn last_generation(ctx: &Ctx, project: &ProjectSlug) -> Option<LastGeneration> {
    match instrukt_blueprint::last_generation(&ctx.home, project) {
        Ok(last) => last,
        Err(err) => {
            tracing::warn!(
                event = "blueprint_generation_record_unreadable",
                project = %project,
                error = %err,
                "the last generation cannot be read; the answer does not name it"
            );
            None
        }
    }
}

fn project(root: &Path) -> anyhow::Result<Option<ProjectSlug>> {
    config::project_slug(root).context("cannot read the project slug")
}

/// The project's latest graph with the repository root it answers for;
/// `None` when there is none to answer from: outside a repository, without
/// a project slug, with no graph yet, or with an unreadable one. A store
/// that cannot be used (not a directory, not readable) is an error.
pub(crate) fn latest_graph(
    ctx: &Ctx,
    root: Option<&Path>,
) -> anyhow::Result<Option<(PathBuf, LatestGraph)>> {
    let Some(root) = root else {
        tracing::debug!(
            event = "blueprint_no_repository",
            cwd = %ctx.cwd.display(),
            "no graph to answer from outside a repository"
        );
        return Ok(None);
    };
    let Some(project) = project(root)? else {
        tracing::warn!(
            event = "blueprint_no_project_slug",
            repo_root = %root.display(),
            "no graph to answer from: the project layer declares no project slug"
        );
        return Ok(None);
    };
    match GraphStore::new(&ctx.home, &project).latest() {
        Ok(Some(latest)) => Ok(Some((root.to_path_buf(), latest))),
        Ok(None) => Ok(None),
        Err(err) if !err.is_unreadable_graph() => {
            Err(anyhow::Error::new(err)
                .context(format!("cannot use the blueprint store of {project}")))
        }
        Err(err) => {
            tracing::warn!(
                event = "blueprint_graph_unreadable",
                project = %project,
                error = %err,
                "the latest graph cannot be read; answering as unavailable"
            );
            Ok(None)
        }
    }
}

/// A node as answers show it: its file, with line when it carries a
/// location, and its label (its id when it carries none).
pub(crate) fn dependent(node: &Node) -> String {
    match node.location {
        Some(location) => format!("{}:{} {}", node.source_file, location.start, node.name()),
        None => format!("{} {}", node.source_file, node.name()),
    }
}

fn step(link: &Link) -> String {
    format!("{} -[{}]-> {}", link.source, link.relation, link.target)
}
