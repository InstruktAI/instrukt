//! Blueprint's `instrukt specs trace <spec_id> <UC-id>` and
//! `instrukt specs trace --file <path> [--file <path> …]`
//! (`docs/project/spec/feature/blueprint/use-case-trace.md`).
//!
//! The subcommand's one-line about stays on `SpecsCommand::Trace` in
//! `specs.rs`; this file owns its arguments and answers.

use std::path::Path;
use std::process::ExitCode;

use anyhow::{Context as _, bail};
use instrukt_core::git;
use instrukt_core::ids::{SnippetId, UseCaseId, UseCaseRef};
use instrukt_core::specs::Coverage;

use crate::commands::blueprint::{UNAVAILABLE, dependent, latest_graph, require_module};
use crate::context::Ctx;

#[derive(Debug, clap::Args)]
pub struct TraceArgs {
    /// The spec id of the use case to trace.
    #[arg(required_unless_present = "file", conflicts_with = "file")]
    spec_id: Option<String>,
    /// The use-case id (`UC-…`) within that spec.
    #[arg(required_unless_present = "file")]
    use_case: Option<String>,
    /// Trace files instead: the use cases at stake when they change, by
    /// spec folder. Relative to the repository root; repeat for several.
    #[arg(long = "file", value_name = "PATH")]
    file: Vec<String>,
}

pub fn run(args: TraceArgs, ctx: &mut Ctx) -> anyhow::Result<ExitCode> {
    let root = git::worktree_root(&ctx.cwd)?;
    require_module(ctx, root.as_deref())?;
    let Some(root) = root else {
        bail!("not inside a git repository; `specs trace` traces a project's use cases");
    };
    let coverage = Coverage::measure(&root, &ctx.home)
        .context("cannot read the project's use cases and bindings")?;
    match (args.spec_id, args.use_case) {
        _ if !args.file.is_empty() => trace_files(ctx, &root, &coverage, &args.file),
        (Some(spec), Some(use_case)) => trace_use_case(ctx, &root, &coverage, &spec, &use_case),
        _ => bail!("give a spec id and a use-case id, or --file <path>"),
    }
}

fn trace_use_case(
    ctx: &mut Ctx,
    root: &Path,
    coverage: &Coverage,
    spec: &str,
    use_case: &str,
) -> anyhow::Result<ExitCode> {
    let reference = UseCaseRef {
        spec: SnippetId::parse(spec)
            .map_err(|err| anyhow::anyhow!("unknown spec id `{spec}`: {err}"))?,
        use_case: UseCaseId::parse(use_case)
            .map_err(|err| anyhow::anyhow!("unknown use-case id `{use_case}`: {err}"))?,
    };
    if coverage.inventory().use_case(&reference).is_none() {
        if !coverage
            .inventory()
            .use_cases()
            .iter()
            .any(|known| known.reference().spec == reference.spec)
        {
            bail!(
                "unknown spec id `{spec}`: no feature spec of this project defines use cases under it"
            );
        }
        bail!("unknown use case `{reference}`: the spec defines no {use_case}");
    }
    let latest = latest_graph(ctx, Some(root))?;
    let trace = instrukt_blueprint::trace_use_case(
        latest.as_ref().map(|(_, latest)| &latest.graph),
        coverage.bindings().bindings(),
        &reference,
    );
    match &latest {
        Some((_, latest)) => ctx.out.line(&format!(
            "trace of {reference} from the blueprint graph of {}",
            latest.sha
        ))?,
        None => ctx.out.line(&format!(
            "trace of {reference}: no blueprint graph of this project yet; the binding tests only"
        ))?,
    }
    if trace.tests.is_empty() {
        ctx.out.line(&format!("no test binds {reference}"))?;
        return Ok(ExitCode::SUCCESS);
    }
    ctx.out.line("tests binding it:")?;
    for test in &trace.tests {
        ctx.out.line(&format!("  {test}"))?;
    }
    if let Some(reached) = &trace.reached {
        if reached.is_empty() {
            ctx.out.line("the tests reach no code outside test files")?;
        } else {
            ctx.out.line("code they reach:")?;
            for node in reached {
                ctx.out.line(&format!("  {}", dependent(node)))?;
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn trace_files(
    ctx: &mut Ctx,
    root: &Path,
    coverage: &Coverage,
    files: &[String],
) -> anyhow::Result<ExitCode> {
    let Some((root, latest)) = latest_graph(ctx, Some(root))? else {
        ctx.out.line(UNAVAILABLE)?;
        return Ok(ExitCode::SUCCESS);
    };
    let files: Vec<String> = files
        .iter()
        .map(|file| instrukt_blueprint::repo_relative(&root, file))
        .collect();
    let trace = instrukt_blueprint::trace_files(
        &latest.graph,
        coverage.bindings().bindings(),
        coverage.inventory(),
        &files,
    );
    let mut distinct = files.clone();
    distinct.sort();
    distinct.dedup();
    let traced = match distinct.as_slice() {
        [one] => one.clone(),
        many => format!("{} files", many.len()),
    };
    ctx.out.line(&format!(
        "trace of {traced} from the blueprint graph of {}",
        latest.sha
    ))?;
    if trace.groups.is_empty() {
        ctx.out.line("no use case is at stake")?;
    } else {
        ctx.out.line("use cases at stake, by spec folder:")?;
        for group in &trace.groups {
            ctx.out.line(&format!("  {}", group.folder))?;
            for at_stake in &group.use_cases {
                ctx.out.line(&format!("    {}", at_stake.use_case))?;
                for test in &at_stake.tests {
                    ctx.out.line(&format!("      {test}"))?;
                }
            }
        }
    }
    if !trace.untraced.is_empty() {
        ctx.out.line(
            "files no use-case-binding test reaches (the trace offers no use-case evidence for them):",
        )?;
        for untraced in &trace.untraced {
            if untraced.known {
                ctx.out.line(&format!("  {}", untraced.file))?;
            } else {
                ctx.out
                    .line(&format!("  {} (the graph does not know it)", untraced.file))?;
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}
