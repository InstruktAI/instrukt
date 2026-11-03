//! Blueprint commands: `instrukt blueprint affected|path|query|explain|status`.
//!
//! `affected` and `path` answer from the project's latest graph and name its
//! SHA. `query`, `explain` and `status` need the engine adapter and
//! generation, which this build does not have yet.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context as _, bail};
use instrukt_blueprint::{GraphStore, LatestGraph, Link, Node, PathAnswer};
use instrukt_core::config::{self, Module};
use instrukt_core::git;

use crate::commands::not_built;
use crate::context::Ctx;

/// The answer when there is no graph to answer from (spec: "unavailable").
const UNAVAILABLE: &str =
    "blueprint unavailable: no graph of this project to answer from; explore by hand";

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
}

pub fn run(args: BlueprintArgs, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    let root = git::worktree_root(&ctx.cwd)?;
    let enabled = config::module_enabled(&ctx.home, root.as_deref(), Module::Blueprint)
        .context("cannot read the blueprint module switch")?;
    if !enabled {
        bail!(
            "the blueprint module is off; switch it on with `modules.blueprint: true` in instrukt.yaml"
        );
    }
    match args.command {
        BlueprintCommand::Affected { files } => affected(ctx, root.as_deref(), &files),
        BlueprintCommand::Path { from, to } => path(ctx, root.as_deref(), &from, &to),
        BlueprintCommand::Query { .. } => not_built("blueprint query"),
        BlueprintCommand::Explain { .. } => not_built("blueprint explain"),
        BlueprintCommand::Status => not_built("blueprint status"),
    }
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

/// The project's latest graph with the repository root it answers for;
/// `None` when there is none to answer from: outside a repository, without
/// a project slug, with no graph yet, or with an unreadable one.
fn latest_graph(ctx: &Ctx, root: Option<&Path>) -> anyhow::Result<Option<(PathBuf, LatestGraph)>> {
    let Some(root) = root else {
        tracing::debug!(
            event = "blueprint_no_repository",
            cwd = %ctx.cwd.display(),
            "no graph to answer from outside a repository"
        );
        return Ok(None);
    };
    let Some(project) = config::project_slug(root).context("cannot read the project slug")? else {
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

fn dependent(node: &Node) -> String {
    match node.location {
        Some(location) => format!("{}:{} {}", node.source_file, location.start, node.name()),
        None => format!("{} {}", node.source_file, node.name()),
    }
}

fn step(link: &Link) -> String {
    format!("{} -[{}]-> {}", link.source, link.relation, link.target)
}
